//! Accessibility tree (AT-SPI): can we find buttons, panels and text fields inside windows?
//!
//! This is how "click an element inside a window" can work without image heuristics. The known
//! Wayland catch: clients don't know where their window is, so screen-relative extents are often
//! wrong or equal to window-relative ones. We record both so we can see whether combining
//! window-relative extents with the compositor's window rectangle is necessary.

use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use anyhow::Context;
use serde_json::{Value, json};
use zbus::{Connection, zvariant::OwnedObjectPath};

use crate::dbus;

const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const COMPONENT: &str = "org.a11y.atspi.Component";
const SCREEN: u32 = 0;
const WINDOW: u32 = 1;
const MAX_NODES_PER_APP: usize = 3000;
const BUDGET: Duration = Duration::from_secs(30);

type Node = (String, OwnedObjectPath);

async fn a11y_bus(enable: bool) -> anyhow::Result<(Connection, bool)> {
    let session = Connection::session().await?;
    let enabled = dbus::get_property(
        &session,
        "org.a11y.Bus",
        "/org/a11y/bus",
        "org.a11y.Status",
        "IsEnabled",
    )
    .await
    .ok()
    .and_then(|v| bool::try_from(v).ok())
    .unwrap_or(false);
    if enable && !enabled {
        let _: () = dbus::call(
            &session,
            "org.a11y.Bus",
            "/org/a11y/bus",
            "org.freedesktop.DBus.Properties",
            "Set",
            &(
                "org.a11y.Status",
                "IsEnabled",
                zbus::zvariant::Value::from(true),
            ),
        )
        .await?;
    }
    let addr: String = dbus::call(
        &session,
        "org.a11y.Bus",
        "/org/a11y/bus",
        "org.a11y.Bus",
        "GetAddress",
        &(),
    )
    .await?;
    let conn = zbus::connection::Builder::address(addr.as_str())?
        .build()
        .await
        .context("connect to a11y bus")?;
    Ok((conn, enabled))
}

async fn children(conn: &Connection, node: &Node) -> anyhow::Result<Vec<Node>> {
    dbus::call(
        conn,
        &node.0,
        node.1.as_str(),
        ACCESSIBLE,
        "GetChildren",
        &(),
    )
    .await
}

async fn extents(conn: &Connection, node: &Node, coords: u32) -> Option<(i32, i32, i32, i32)> {
    dbus::call(
        conn,
        &node.0,
        node.1.as_str(),
        COMPONENT,
        "GetExtents",
        &(coords,),
    )
    .await
    .ok()
}

async fn name_of(conn: &Connection, node: &Node) -> String {
    dbus::get_property(conn, &node.0, node.1.as_str(), ACCESSIBLE, "Name")
        .await
        .ok()
        .and_then(|v| String::try_from(v).ok())
        .unwrap_or_default()
}

/// What hover-to-snap would do: ask the toplevel which element is under a point, descending
/// until we reach a leaf. Timed, because this runs on every mouse move.
async fn hit_test_centre(conn: &Connection, frame: &Node) -> Value {
    let Some((_, _, w, h)) = extents(conn, frame, WINDOW).await else {
        return json!(null);
    };
    let (px, py) = (w / 2, h / 2);
    let start = Instant::now();
    let mut current = frame.clone();
    let mut path = Vec::new();
    for _ in 0..30 {
        let hit: anyhow::Result<Node> = dbus::call(
            conn,
            &current.0,
            current.1.as_str(),
            COMPONENT,
            "GetAccessibleAtPoint",
            &(px, py, WINDOW),
        )
        .await;
        match hit {
            Ok(n) if n.1.as_str() != "/org/a11y/atspi/null" && n != current => {
                let role: String =
                    dbus::call(conn, &n.0, n.1.as_str(), ACCESSIBLE, "GetRoleName", &())
                        .await
                        .unwrap_or_default();
                path.push(
                    json!({ "role": role, "window_extents": extents(conn, &n, WINDOW).await }),
                );
                current = n;
            }
            _ => break,
        }
    }
    json!({ "point_window_coords": [px, py], "ms": start.elapsed().as_secs_f64() * 1000.0, "path": path })
}

async fn walk_app(conn: &Connection, app: &Node, deadline: Instant) -> Value {
    let name = name_of(conn, app).await;

    let mut queue = VecDeque::from([(app.clone(), 0usize)]);
    let (mut nodes, mut with_extents, mut screen_eq_window, mut max_depth) =
        (0usize, 0usize, 0usize, 0usize);
    let mut offscreen = 0usize;
    let mut hit_test = None;
    let mut roles: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples = Vec::new();
    let mut stopped = None;

    while let Some((node, depth)) = queue.pop_front() {
        if nodes >= MAX_NODES_PER_APP {
            stopped = Some("node cap");
            break;
        }
        if Instant::now() > deadline {
            stopped = Some("time budget");
            break;
        }
        nodes += 1;
        max_depth = max_depth.max(depth);
        let role: String = dbus::call(
            conn,
            &node.0,
            node.1.as_str(),
            ACCESSIBLE,
            "GetRoleName",
            &(),
        )
        .await
        .unwrap_or_default();
        *roles.entry(role.clone()).or_default() += 1;

        let screen = extents(conn, &node, SCREEN).await;
        let window = extents(conn, &node, WINDOW).await;
        match screen.or(window) {
            // Toolkits report hidden/unmapped widgets at i32::MIN.
            Some((x, _, _, _)) if x == i32::MIN => offscreen += 1,
            Some((_, _, w, h)) if w > 0 && h > 0 => {
                with_extents += 1;
                if screen == window {
                    screen_eq_window += 1;
                }
                if hit_test.is_none() && matches!(role.as_str(), "frame" | "window" | "dialog") {
                    hit_test = Some(hit_test_centre(conn, &node).await);
                }
                if samples.len() < 12 && depth >= 2 {
                    let label = name_of(conn, &node).await;
                    samples.push(json!({ "role": role, "name": label, "screen": screen, "window": window, "depth": depth }));
                }
            }
            _ => {}
        }
        match children(conn, &node).await {
            Ok(kids) => queue.extend(kids.into_iter().map(|k| (k, depth + 1))),
            Err(_) if depth == 0 => {
                stopped = Some("app did not answer GetChildren");
                break;
            }
            Err(_) => {}
        }
    }

    let mut top_roles: Vec<_> = roles.into_iter().collect();
    top_roles.sort_by_key(|r| std::cmp::Reverse(r.1));
    top_roles.truncate(8);

    json!({
        "app": name,
        "bus_name": app.0,
        "nodes_visited": nodes,
        "nodes_with_extents": with_extents,
        "screen_extents_equal_window_extents": screen_eq_window,
        "offscreen_nodes": offscreen,
        "hit_test_at_window_centre": hit_test,
        "max_depth": max_depth,
        "stopped_early": stopped,
        "top_roles": top_roles,
        "samples": samples,
    })
}

pub async fn probe(enable: bool) -> anyhow::Result<Value> {
    let (conn, was_enabled) = a11y_bus(enable).await?;
    let root: Node = (
        "org.a11y.atspi.Registry".into(),
        OwnedObjectPath::try_from("/org/a11y/atspi/accessible/root")?,
    );
    let apps = children(&conn, &root)
        .await
        .context("registry root children")?;
    let deadline = Instant::now() + BUDGET;
    let mut results = Vec::new();
    for app in &apps {
        results.push(walk_app(&conn, app, deadline).await);
    }
    Ok(json!({
        "a11y_was_enabled": was_enabled,
        "enabled_now": enable || was_enabled,
        "app_count": apps.len(),
        "apps": results,
    }))
}

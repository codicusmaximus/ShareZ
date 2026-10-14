//! GNOME: GNOME Shell's accessibility tree has one panel per window actor ("Wayland window" or
//! "X11 window") carrying its on-screen rectangle. The spike matched these pixel for pixel against
//! X11 window rectangles and the apps' own frame sizes.
//!
//! Known gaps, to be handled by the overlay: the rectangles include GTK 4 client-side shadows,
//! windows on other workspaces and minimised windows are listed too, and the tree order has not
//! yet been confirmed to be the stacking order.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use sharez_core::Rect;
use zbus::{Connection, zvariant::OwnedObjectPath};

use crate::{Error, Result, Window, WindowKind, dbus::call};

const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const COMPONENT: &str = "org.a11y.atspi.Component";
const COORDS_SCREEN: u32 = 0;
const MAX_NODES: usize = 15_000;
const BUDGET: Duration = Duration::from_secs(3);

type Node = (String, OwnedObjectPath);

async fn a11y_bus() -> Result<Connection> {
    let session = Connection::session().await?;
    let addr: String = call(
        &session,
        "org.a11y.Bus",
        "/org/a11y/bus",
        "org.a11y.Bus",
        "GetAddress",
        &(),
    )
    .await?;
    Ok(zbus::connection::Builder::address(addr.as_str())?
        .build()
        .await?)
}

async fn children(conn: &Connection, node: &Node) -> Result<Vec<Node>> {
    call(
        conn,
        &node.0,
        node.1.as_str(),
        ACCESSIBLE,
        "GetChildren",
        &(),
    )
    .await
}

async fn property(conn: &Connection, node: &Node, name: &str) -> Result<String> {
    let v: zbus::zvariant::OwnedValue = call(
        conn,
        &node.0,
        node.1.as_str(),
        "org.freedesktop.DBus.Properties",
        "Get",
        &(ACCESSIBLE, name),
    )
    .await?;
    String::try_from(v).map_err(|e| Error::BadData("AT-SPI", e.to_string()))
}

pub async fn windows() -> Result<Vec<Window>> {
    let conn = a11y_bus().await?;
    let root: Node = (
        "org.a11y.atspi.Registry".into(),
        OwnedObjectPath::try_from("/org/a11y/atspi/accessible/root").expect("valid path"),
    );
    let mut shell = None;
    for app in children(&conn, &root).await? {
        if property(&conn, &app, "Name")
            .await
            .is_ok_and(|n| n == "gnome-shell")
        {
            shell = Some(app);
            break;
        }
    }
    let shell = shell.ok_or(Error::BadData(
        "AT-SPI",
        "GNOME Shell is not on the accessibility bus (is accessibility enabled?)".into(),
    ))?;

    let deadline = Instant::now() + BUDGET;
    let mut queue = VecDeque::from([shell]);
    let mut visited = 0;
    let mut out = Vec::new();
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if visited > MAX_NODES || Instant::now() > deadline {
            return Err(Error::Timeout("GNOME Shell's accessibility tree"));
        }
        let role: String = call(
            &conn,
            &node.0,
            node.1.as_str(),
            ACCESSIBLE,
            "GetRoleName",
            &(),
        )
        .await
        .unwrap_or_default();
        if role == "panel" {
            let name = property(&conn, &node, "Name").await.unwrap_or_default();
            if name == "Wayland window" || name == "X11 window" {
                let (x, y, w, h): (i32, i32, i32, i32) = call(
                    &conn,
                    &node.0,
                    node.1.as_str(),
                    COMPONENT,
                    "GetExtents",
                    &(COORDS_SCREEN,),
                )
                .await?;
                out.push(Window {
                    title: String::new(),
                    app_id: None,
                    pid: None,
                    kind: WindowKind::Normal,
                    frame: Rect::new(x.into(), y.into(), w.into(), h.into()),
                    client: None,
                    shadow_included: name == "Wayland window",
                    minimized: false,
                });
                // A window actor's subtree is the window's own content; nothing more to find.
                continue;
            }
        }
        queue.extend(children(&conn, &node).await.unwrap_or_default());
    }
    Ok(out)
}

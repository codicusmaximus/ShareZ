//! GNOME: window positions and stacking.
//!
//! The ShareZ Helper extension (`gnome-extension/` in the repository) reads them straight from
//! Mutter: exact frames without client-side shadows, titles, and the stacking order. Without it,
//! ShareZ falls back to GNOME Shell's accessibility tree, which has one panel per window actor
//! with its on-screen rectangle, but no titles and no reliable stacking order, and rectangles that
//! include GTK 4 shadows.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use serde::Deserialize;
use sharez_core::Rect;
use zbus::Connection;

use crate::{Error, Result, Window, WindowKind, a11y, dbus::call};

const HELPER: &str = "io.github.codicusmaximus.ShareZ.ShellHelper";
const HELPER_PATH: &str = "/io/github/codicusmaximus/ShareZ/ShellHelper";
const MAX_NODES: usize = 15_000;
const BUDGET: Duration = Duration::from_secs(3);

pub async fn windows() -> Result<Vec<Window>> {
    match from_extension().await {
        Ok(windows) => Ok(windows),
        Err(_) => from_shell_tree().await,
    }
}

#[derive(Deserialize)]
struct HelperWindow {
    title: String,
    app_id: Option<String>,
    pid: i64,
    kind: WindowKind,
    frame: [f64; 4],
    client: [f64; 4],
}

async fn from_extension() -> Result<Vec<Window>> {
    let conn = Connection::session().await?;
    let json: String = call(&conn, HELPER, HELPER_PATH, HELPER, "GetWindows", &()).await?;
    let list: Vec<HelperWindow> =
        serde_json::from_str(&json).map_err(|e| Error::BadData("ShareZ Helper", e.to_string()))?;
    let rect = |[x, y, w, h]: [f64; 4]| Rect::new(x, y, w, h);
    Ok(list
        .into_iter()
        .map(|w| Window {
            title: w.title,
            app_id: w.app_id.filter(|a| !a.is_empty()),
            pid: u32::try_from(w.pid).ok().filter(|&p| p > 0),
            kind: w.kind,
            frame: rect(w.frame),
            client: Some(rect(w.client)),
            shadow_included: false,
            minimized: false,
        })
        .collect())
}

async fn from_shell_tree() -> Result<Vec<Window>> {
    let conn = a11y::bus().await?;
    let mut shell = None;
    for app in a11y::children(&conn, &a11y::registry_root()).await? {
        if a11y::name(&conn, &app).await == "gnome-shell" {
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
        if a11y::role(&conn, &node).await == "panel" {
            let name = a11y::name(&conn, &node).await;
            if name == "Wayland window" || name == "X11 window" {
                // Windows on other workspaces and minimised ones are in the tree but not showing.
                if a11y::showing(&conn, &node).await
                    && let Ok(frame) = a11y::extents(&conn, &node, a11y::COORDS_SCREEN).await
                {
                    out.push(Window {
                        title: String::new(),
                        app_id: None,
                        pid: None,
                        kind: WindowKind::Normal,
                        frame,
                        client: None,
                        shadow_included: name == "Wayland window",
                        minimized: false,
                    });
                }
                // A window actor's subtree is the window's own content; nothing more to find.
                continue;
            }
        }
        queue.extend(a11y::children(&conn, &node).await.unwrap_or_default());
    }
    Ok(out)
}

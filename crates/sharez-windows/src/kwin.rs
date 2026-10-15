//! KDE Plasma: KWin scripts read the window stack (or place a window of ours) and call back into
//! ShareZ over D-Bus.

use std::{path::PathBuf, time::Duration};

use futures_lite::FutureExt;
use serde::Deserialize;
use sharez_core::Rect;
use zbus::Connection;

use crate::{Error, Handle, Result, Window, WindowKind, dbus::call};

const BUS_NAME: &str = "io.github.codicusmaximus.ShareZ.KWinWindows";
const PLUGIN: &str = "sharez-window-list";

const SCRIPT: &str = r#"
const list = workspace.stackingOrder;
const out = [];
for (let i = 0; i < list.length; i++) {
    const w = list[i];
    const g = w.frameGeometry, c = w.clientGeometry;
    out.push({
        title: String(w.caption),
        id: String(w.internalId),
        app_id: String(w.resourceClass),
        pid: w.pid,
        frame: [g.x, g.y, g.width, g.height],
        client: c ? [c.x, c.y, c.width, c.height] : null,
        normal: !!w.normalWindow, dialog: !!w.dialog, dock: !!w.dock,
        desktop: !!w.desktopWindow, minimized: !!w.minimized,
    });
}
callDBus("io.github.codicusmaximus.ShareZ.KWinWindows", "/", "io.github.codicusmaximus.ShareZ.KWinWindows", "Windows", JSON.stringify(out));
"#;

#[derive(Deserialize)]
#[allow(clippy::struct_excessive_bools)] // Mirrors the script's JSON.
struct ScriptWindow {
    title: String,
    app_id: String,
    pid: i64,
    id: String,
    frame: [f64; 4],
    client: Option<[f64; 4]>,
    normal: bool,
    dialog: bool,
    dock: bool,
    desktop: bool,
    minimized: bool,
}

struct Sink(async_channel::Sender<String>);

#[zbus::interface(name = "io.github.codicusmaximus.ShareZ.KWinWindows")]
impl Sink {
    async fn windows(&self, json: String) {
        let _ = self.0.send(json).await;
    }
}

/// KWin reads the script from disk. Under Flatpak only `$XDG_RUNTIME_DIR/app/$FLATPAK_ID` is
/// shared with the host, at the same path.
fn script_path(file: &str) -> PathBuf {
    let runtime =
        std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    match std::env::var_os("FLATPAK_ID") {
        Some(id) => runtime.join("app").join(id),
        None => runtime,
    }
    .join(file)
}

/// Moves a window of ours to `rect`, keeps it above the others on every desktop and out of the
/// task bar and switcher.
pub async fn place(title: &str, rect: Rect) -> Result<bool> {
    let [x, y, w, h] = crate::gnome::px(rect);
    let title = serde_json::to_string(title).map_err(|e| Error::BadData("KWin", e.to_string()))?;
    let script = format!(
        r#"
let placed = false;
for (const w of workspace.stackingOrder) {{
    if (w.caption !== {title}) continue;
    w.frameGeometry = {{x: {x}, y: {y}, width: {w}, height: {h}}};
    w.keepAbove = true;
    w.onAllDesktops = true;
    w.skipTaskbar = true;
    w.skipSwitcher = true;
    w.skipPager = true;
    placed = true;
}}
callDBus("{BUS_NAME}", "/", "{BUS_NAME}", "Windows", JSON.stringify(placed));
"#
    );
    let reply = run(&script, "sharez-kwin-place.js").await?;
    Ok(reply == "true")
}

pub async fn windows() -> Result<Vec<Window>> {
    let json = run(SCRIPT, "sharez-kwin-windows.js").await?;
    let list: Vec<ScriptWindow> =
        serde_json::from_str(&json).map_err(|e| Error::BadData("KWin", e.to_string()))?;
    // KWin lists bottom to top.
    Ok(list.into_iter().rev().map(Window::from).collect())
}

/// Runs a KWin script that calls back with one string, and returns that string.
async fn run(script: &str, file: &str) -> Result<String> {
    let (tx, rx) = async_channel::bounded(1);
    let _server = zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at("/", Sink(tx))?
        .build()
        .await?;

    let path = script_path(file);
    std::fs::write(&path, script)?;
    let result = run_script(&path, &rx).await;
    let _ = std::fs::remove_file(&path);
    result
}

async fn run_script(
    path: &std::path::Path,
    rx: &async_channel::Receiver<String>,
) -> Result<String> {
    let conn = Connection::session().await?;
    let (dest, scripting, iface) = ("org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting");
    let path = path
        .to_str()
        .ok_or_else(|| Error::BadData("KWin", "script path is not UTF-8".into()))?;

    // A previous run that crashed may have left the script loaded.
    let _: Result<bool> = call(&conn, dest, scripting, iface, "unloadScript", &(PLUGIN,)).await;
    let id: i32 = call(&conn, dest, scripting, iface, "loadScript", &(path, PLUGIN)).await?;
    let ran: Result<()> = call(
        &conn,
        dest,
        &format!("/Scripting/Script{id}"),
        "org.kde.kwin.Script",
        "run",
        &(),
    )
    .await;
    let reply = match ran {
        Ok(()) => {
            async { rx.recv().await.map_err(|_| Error::Timeout("KWin")) }
                .or(async {
                    async_io::Timer::after(Duration::from_secs(2)).await;
                    Err(Error::Timeout("KWin"))
                })
                .await
        }
        Err(e) => Err(e),
    };
    let _: Result<bool> = call(&conn, dest, scripting, iface, "unloadScript", &(PLUGIN,)).await;
    reply
}

fn rect([x, y, w, h]: [f64; 4]) -> Rect {
    Rect::new(x, y, w, h)
}

impl From<ScriptWindow> for Window {
    fn from(w: ScriptWindow) -> Self {
        let kind = if w.desktop {
            WindowKind::Desktop
        } else if w.dock {
            WindowKind::Dock
        } else if w.dialog {
            WindowKind::Dialog
        } else if w.normal {
            WindowKind::Normal
        } else {
            WindowKind::Other
        };
        Window {
            title: w.title,
            app_id: Some(w.app_id).filter(|a| !a.is_empty()),
            pid: u32::try_from(w.pid).ok().filter(|&p| p > 0),
            kind,
            frame: rect(w.frame),
            client: w.client.map(rect),
            shadow_included: false,
            minimized: w.minimized,
            handle: Some(w.id).filter(|id| !id.is_empty()).map(Handle::Kwin),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_script_output() {
        // Konsole from the KDE spike run: a 27 px server-side title bar at scale 1.35.
        let json = r#"[{"title":"feasibility : bash — Konsole","app_id":"org.kde.konsole","pid":63229,
            "id":"{0b8f5a52-3a4c-4f0e-9f57-0c9a3c7a1d11}",
            "frame":[380,244,911.11,579.26],"client":[380,271.41,911.11,551.85],
            "normal":true,"dialog":false,"dock":false,"desktop":false,"minimized":false}]"#;
        let list: Vec<ScriptWindow> = serde_json::from_str(json).unwrap();
        let w = Window::from(list.into_iter().next().unwrap());
        assert_eq!(w.kind, WindowKind::Normal);
        assert_eq!(w.pid, Some(63229));
        assert_eq!(
            w.handle,
            Some(Handle::Kwin(
                "{0b8f5a52-3a4c-4f0e-9f57-0c9a3c7a1d11}".into()
            ))
        );
        assert!((w.client.unwrap().y - w.frame.y - 27.41).abs() < 1e-9);
    }
}

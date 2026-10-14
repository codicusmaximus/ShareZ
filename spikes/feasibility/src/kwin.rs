//! KDE Plasma (KWin): window geometry and the native screenshot API.
//!
//! Wayland gives clients no way to list other windows, so on KWin we load a tiny KWin script
//! that reads the window stack and calls back into us over D-Bus. We also probe
//! `org.kde.KWin.ScreenShot2`, which Spectacle uses for dialog-free captures; KWin restricts it
//! to apps whose installed .desktop file declares `X-KDE-DBUS-Restricted-Interfaces`, so from a
//! dev build we expect "denied", and that error is itself the finding.

use std::{collections::HashMap, io::Read, os::fd::AsFd, path::Path, time::Duration};

use anyhow::Context;
use serde_json::{Value, json};
use zbus::{
    Connection,
    zvariant::{Fd, OwnedValue, Value as ZValue},
};

use crate::dbus;

const BUS_NAME: &str = "org.sharez.Spike";
const PLUGIN: &str = "sharez-spike";

const SCRIPT: &str = r#"
const list = (workspace.stackingOrder !== undefined) ? workspace.stackingOrder : workspace.clientList();
const windows = [];
for (let i = 0; i < list.length; i++) {
    const w = list[i];
    const g = w.frameGeometry || w.geometry;
    const b = w.bufferGeometry;
    windows.push({
        caption: String(w.caption),
        resource_class: String(w.resourceClass),
        pid: w.pid,
        frame: [g.x, g.y, g.width, g.height],
        buffer: b ? [b.x, b.y, b.width, b.height] : null,
        normal: !!w.normalWindow, dialog: !!w.dialog, desktop: !!w.desktopWindow,
        dock: !!w.dock, minimized: !!w.minimized, fullscreen: !!w.fullScreen,
        wayland: w.waylandClient !== undefined ? !!w.waylandClient : null,
    });
}
const screens = [];
const outs = workspace.screens || [];
for (let i = 0; i < outs.length; i++) {
    const s = outs[i];
    screens.push({ name: String(s.name), geometry: [s.geometry.x, s.geometry.y, s.geometry.width, s.geometry.height], scale: s.devicePixelRatio });
}
callDBus("org.sharez.Spike", "/Spike", "org.sharez.Spike", "Windows", JSON.stringify({ windows: windows, screens: screens }));
"#;

struct Sink(tokio::sync::mpsc::Sender<String>);

#[zbus::interface(name = "org.sharez.Spike")]
impl Sink {
    async fn windows(&self, json: String) {
        let _ = self.0.send(json).await;
    }
}

async fn window_list(out: &Path) -> anyhow::Result<Value> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let _server = zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at("/Spike", Sink(tx))?
        .build()
        .await
        .context("own org.sharez.Spike on the session bus")?;

    let conn = Connection::session().await?;
    let script_path = std::fs::canonicalize(out)?.join("kwin-spike.js");
    std::fs::write(&script_path, SCRIPT)?;
    let (dest, scripting, iface) = ("org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting");

    let _: bool = dbus::call(&conn, dest, scripting, iface, "unloadScript", &(PLUGIN,))
        .await
        .unwrap_or(false);
    let id: i32 = dbus::call(
        &conn,
        dest,
        scripting,
        iface,
        "loadScript",
        &(script_path.to_str().unwrap(), PLUGIN),
    )
    .await?;

    // KWin 6 exposes each script at /Scripting/Script<id>; KWin 5 at /<id>; `start` runs all.
    let mut how = "Scripting.start";
    for path in [format!("/Scripting/Script{id}"), format!("/{id}")] {
        if dbus::call::<_, ()>(&conn, dest, &path, "org.kde.kwin.Script", "run", &())
            .await
            .is_ok()
        {
            how = if path.starts_with("/Scripting") {
                "kwin6 Script.run"
            } else {
                "kwin5 Script.run"
            };
            break;
        }
    }
    if how == "Scripting.start" {
        dbus::call::<_, ()>(&conn, dest, scripting, iface, "start", &()).await?;
    }

    let result = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await;
    let _: bool = dbus::call(&conn, dest, scripting, iface, "unloadScript", &(PLUGIN,))
        .await
        .unwrap_or(false);
    let json_text = result
        .context("KWin script never called back")?
        .context("channel closed")?;
    let mut v: Value = serde_json::from_str(&json_text)?;
    v["loaded_via"] = json!(how);
    if let Some(ws) = v["windows"].as_array() {
        v["window_count"] = json!(ws.len());
    }
    Ok(v)
}

async fn screenshot2(out: &Path) -> anyhow::Result<Value> {
    let conn = Connection::session().await?;
    let (mut reader, writer) = std::io::pipe()?;
    let read_task = tokio::task::spawn_blocking(move || {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).map(|_| buf)
    });

    let mut opts: HashMap<&str, ZValue> = HashMap::new();
    opts.insert("include-cursor", ZValue::from(false));
    opts.insert("native-resolution", ZValue::from(true));
    let reply: anyhow::Result<HashMap<String, OwnedValue>> = dbus::call(
        &conn,
        "org.kde.KWin",
        "/org/kde/KWin/ScreenShot2",
        "org.kde.KWin.ScreenShot2",
        "CaptureWorkspace",
        &(opts, Fd::from(writer.as_fd())),
    )
    .await;
    drop(writer);

    let meta = reply.context("ScreenShot2.CaptureWorkspace")?;
    let data = read_task.await??;
    let get = |k: &str| meta.get(k).and_then(|v| u32::try_from(v.clone()).ok());
    let raw = out.join("kwin-screenshot2.raw");
    std::fs::write(&raw, &data)?;
    Ok(json!({
        "width": get("width"), "height": get("height"), "stride": get("stride"), "format": get("format"),
        "scale": meta.get("scale").and_then(|v| f64::try_from(v.clone()).ok()),
        "bytes": data.len(),
        "raw_file": raw,
    }))
}

pub async fn probe(out: &Path) -> anyhow::Result<Value> {
    let windows = window_list(out).await.map_err(|e| format!("{e:#}"));
    let shot = screenshot2(out).await.map_err(|e| format!("{e:#}"));
    Ok(json!({
        "window_list": match windows { Ok(v) => v, Err(e) => json!({ "error": e }) },
        "screenshot2": match shot { Ok(v) => v, Err(e) => json!({ "error": e }) },
    }))
}

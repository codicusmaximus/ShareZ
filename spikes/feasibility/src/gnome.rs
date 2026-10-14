//! GNOME Shell: window geometry.
//!
//! Mutter exposes no window positions to ordinary clients. We check the two candidates:
//! `org.gnome.Shell.Introspect.GetWindows` (gated behind a gsettings key / allowlist, and as far
//! as we know reports sizes but not positions), and the spike helper extension in
//! `spikes/feasibility/gnome-extension`, which reads `Meta.Window.get_frame_rect()` directly.
//! The shell's own `org.gnome.Shell.Screenshot` is probed too; it is allowlisted to GNOME's own
//! tools, so "access denied" is the expected answer.

use std::collections::HashMap;

use serde_json::{Value, json};
use zbus::{Connection, zvariant::OwnedValue};

use crate::dbus;

async fn introspect(conn: &Connection) -> anyhow::Result<Value> {
    let windows: HashMap<u64, HashMap<String, OwnedValue>> = dbus::call(
        conn,
        "org.gnome.Shell",
        "/org/gnome/Shell/Introspect",
        "org.gnome.Shell.Introspect",
        "GetWindows",
        &(),
    )
    .await?;
    let sample = windows.values().next().map(|w| {
        w.iter()
            .map(|(k, v)| (k.clone(), format!("{v:?}")))
            .collect::<HashMap<_, _>>()
    });
    Ok(json!({ "window_count": windows.len(), "sample_window": sample }))
}

async fn helper_extension(conn: &Connection) -> anyhow::Result<Value> {
    let text: String = dbus::call(
        conn,
        "org.sharez.SpikeHelper",
        "/org/sharez/SpikeHelper",
        "org.sharez.SpikeHelper",
        "GetWindows",
        &(),
    )
    .await?;
    let mut v: Value = serde_json::from_str(&text)?;
    if let Some(ws) = v["windows"].as_array() {
        v["window_count"] = json!(ws.len());
    }
    Ok(v)
}

async fn shell_screenshot(conn: &Connection) -> anyhow::Result<Value> {
    let path = std::env::temp_dir().join("sharez-gnome-shell-shot.png");
    let (ok, file): (bool, String) = dbus::call(
        conn,
        "org.gnome.Shell.Screenshot",
        "/org/gnome/Shell/Screenshot",
        "org.gnome.Shell.Screenshot",
        "Screenshot",
        &(false, false, path.to_str().unwrap()),
    )
    .await?;
    Ok(json!({ "ok": ok, "file": file }))
}

pub async fn probe() -> anyhow::Result<Value> {
    let conn = Connection::session().await?;
    let wrap =
        |r: anyhow::Result<Value>| r.unwrap_or_else(|e| json!({ "error": format!("{e:#}") }));
    Ok(json!({
        "introspect": wrap(introspect(&conn).await),
        "helper_extension": wrap(helper_extension(&conn).await),
        "shell_screenshot": wrap(shell_screenshot(&conn).await),
    }))
}

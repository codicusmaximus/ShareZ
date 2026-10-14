//! What are we running on? Session type, compositor, portal backends and GStreamer plugins.

use std::process::Command;

use serde_json::{Value, json};

use crate::dbus;

pub fn session_type() -> String {
    std::env::var("XDG_SESSION_TYPE").unwrap_or_default()
}

pub fn desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
}

/// First line of `cmd args`, or None if the command is missing or fails.
pub fn command_line(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().next().map(|l| l.trim().to_owned())
}

pub fn has_gst_element(name: &str) -> bool {
    Command::new("gst-inspect-1.0")
        .args(["--exists", name])
        .status()
        .is_ok_and(|s| s.success())
}

/// Names of running processes whose name contains `needle`.
fn running(needle: &str) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return found;
    };
    for entry in dir.flatten() {
        if let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) {
            let comm = comm.trim();
            if comm.contains(needle) && !found.iter().any(|f| f == comm) {
                found.push(comm.to_owned());
            }
        }
    }
    found
}

pub async fn probe() -> anyhow::Result<Value> {
    let vars = [
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "WAYLAND_DISPLAY",
        "DISPLAY",
    ]
    .map(|k| (k, std::env::var(k).ok()));

    let mut portal = json!({ "backends_running": running("xdg-desktop-por") });
    if let Ok(conn) = zbus::Connection::session().await {
        for iface in [
            "Screenshot",
            "ScreenCast",
            "RemoteDesktop",
            "GlobalShortcuts",
        ] {
            let full = format!("org.freedesktop.portal.{iface}");
            let v = dbus::get_property(
                &conn,
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                &full,
                "version",
            )
            .await
            .ok()
            .and_then(|v| u32::try_from(v).ok());
            portal[iface] = json!(v);
        }
        for prop in ["AvailableSourceTypes", "AvailableCursorModes"] {
            let v = dbus::get_property(
                &conn,
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.ScreenCast",
                prop,
            )
            .await
            .ok()
            .and_then(|v| u32::try_from(v).ok());
            portal[format!("ScreenCast.{prop}")] = json!(v);
        }
    }

    let gst_elements = [
        "pipewiresrc",
        "ximagesrc",
        "pulsesrc",
        "videocrop",
        "x264enc",
        "openh264enc",
        "vah264enc",
        "vaapih264enc",
        "vp8enc",
        "opusenc",
        "matroskamux",
    ]
    .map(|e| (e, has_gst_element(e)));

    Ok(json!({
        "vars": vars.iter().cloned().collect::<std::collections::BTreeMap<_, _>>(),
        "versions": {
            "kwin": command_line("kwin_wayland", &["--version"]).or_else(|| command_line("kwin_x11", &["--version"])),
            "plasma": command_line("plasmashell", &["--version"]),
            "gnome_shell": command_line("gnome-shell", &["--version"]),
            "pipewire": command_line("pipewire", &["--version"]).or_else(|| command_line("pw-cli", &["--version"])),
            "gstreamer": command_line("gst-launch-1.0", &["--version"]),
            "ffprobe": command_line("ffprobe", &["-version"]),
            "os": std::fs::read_to_string("/etc/os-release").ok()
                .and_then(|s| s.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| l[12..].trim_matches('"').to_owned())),
        },
        "portal": portal,
        "gst_elements": gst_elements.iter().cloned().collect::<std::collections::BTreeMap<_, _>>(),
    }))
}

//! xdg-desktop-portal: the one capture path that works on every Wayland desktop.
//!
//! Questions: does a non-interactive screenshot work without a dialog every time, how long does
//! it take (it is our "freeze frame" on Wayland), and does a screencast restore token let us skip
//! the share dialog on the second recording?

use std::{os::fd::OwnedFd, path::Path, time::Instant};

use anyhow::Context;
use ashpd::desktop::{
    PersistMode, Session,
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream},
    screenshot::Screenshot,
};
use serde_json::{Value, json};

fn uri_to_path(uri: &str) -> String {
    let raw = uri.strip_prefix("file://").unwrap_or(uri);
    // Minimal percent-decoding; portal paths are ASCII apart from the occasional %20.
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&raw[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn png_size(path: &str) -> Option<(u32, u32)> {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let reader = dec.read_info().ok()?;
    let info = reader.info();
    Some((info.width, info.height))
}

pub async fn screenshot(out: &Path) -> anyhow::Result<Value> {
    let mut shots = Vec::new();
    // Twice: the first call may show a one-time permission prompt (GNOME), the second shows the
    // steady-state latency the user would feel when pressing the hotkey.
    for attempt in 1..=2 {
        let start = Instant::now();
        let response = Screenshot::request()
            .interactive(false)
            .modal(false)
            .send()
            .await?
            .response()
            .context("portal screenshot")?;
        let ms = start.elapsed().as_millis();
        let src = uri_to_path(&response.uri().to_string());
        let dest = out.join(format!("portal-shot-{attempt}.png"));
        let copied = std::fs::copy(&src, &dest).is_ok();
        shots.push(json!({
            "attempt": attempt,
            "ms": ms,
            "portal_file": src,
            "copied_to": copied.then(|| dest.display().to_string()),
            "pixel_size": png_size(&src),
        }));
        let _ = std::fs::remove_file(&src);
    }
    Ok(json!({ "shots": shots }))
}

pub struct Cast {
    // Dropping the session ends the cast, so it lives as long as the recording does.
    pub _session: Session<Screencast>,
    pub stream: Stream,
    pub fd: OwnedFd,
    pub info: Value,
}

/// Opens a portal screencast of one monitor or window. With a restore token from a previous
/// run, a compliant portal skips the picker dialog.
pub async fn screencast(restore_token: Option<&str>) -> anyhow::Result<Cast> {
    let start = Instant::now();
    let proxy = Screencast::new().await?;
    let session = proxy.create_session(Default::default()).await?;
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Embedded)
                .set_sources(SourceType::Monitor | SourceType::Window)
                .set_multiple(false)
                .set_restore_token(restore_token)
                .set_persist_mode(PersistMode::ExplicitlyRevoked),
        )
        .await?
        .response()
        .context("select_sources")?;
    let response = proxy
        .start(&session, None, Default::default())
        .await?
        .response()
        .context("start screencast (cancelled?)")?;
    let picked_ms = start.elapsed().as_millis();
    let stream = response
        .streams()
        .first()
        .context("portal returned no streams")?
        .to_owned();
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await?;

    let info = json!({
        "used_restore_token": restore_token.is_some(),
        "ms_until_started": picked_ms,
        "new_restore_token": response.restore_token(),
        "node_id": stream.pipe_wire_node_id(),
        "source_type": format!("{:?}", stream.source_type()),
        "position": stream.position(),
        "size": stream.size(),
    });
    Ok(Cast {
        _session: session,
        stream,
        fd,
        info,
    })
}

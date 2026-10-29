//! The xdg-desktop-portal `ScreenCast` session that feeds a recording on Wayland.

use std::{os::fd::OwnedFd, path::PathBuf};

use ashpd::desktop::{
    CreateSessionOptions, PersistMode, ResponseError, Session,
    screencast::{
        CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
        StartCastOptions,
    },
};
use ashpd::enumflags2::BitFlags;
use futures_util::StreamExt;
use gstreamer::glib;
use sharez_core::Rect;

use crate::{Error, Result};

/// A running screencast. Dropping it ends the cast.
#[derive(Debug)]
pub(crate) struct Cast {
    session: Session<Screencast>,
    pub fd: OwnedFd,
    pub node: u32,
    /// The shared monitor in logical coordinates, when the portal says.
    pub area: Option<Rect>,
}

impl Cast {
    /// Resolves when the desktop ends the share (GNOME's "Stop sharing", say).
    pub async fn closed(&self) {
        match self.session.receive_closed().await {
            Ok(mut signals) => {
                signals.next().await;
            }
            // Without the signal, rely on the video source failing instead.
            Err(_) => std::future::pending().await,
        }
    }

    pub async fn close(self) {
        let _ = self.session.close().await;
    }
}

/// Saved so the share dialog only appears on the first recording.
fn token_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state"))
        .join("sharez")
        .join("screencast-restore-token")
}

fn cancelled(e: ashpd::Error) -> Error {
    match e {
        ashpd::Error::Response(ResponseError::Cancelled) => Error::NoStream,
        other => Error::Portal(other),
    }
}

/// Starts sharing one monitor. The first time, the desktop asks which one.
pub(crate) async fn open() -> Result<Cast> {
    match start(std::fs::read_to_string(token_path()).ok()).await {
        // KDE's picker also offers a new, empty virtual screen, and remembers it. Recording that
        // films only the wallpaper, so forget it and ask once more.
        Err(Error::VirtualScreen) => {
            let _ = std::fs::remove_file(token_path());
            start(None).await
        }
        cast => cast,
    }
}

async fn start(token: Option<String>) -> Result<Cast> {
    let proxy = Screencast::new().await?;
    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await?;
    let cursor = if proxy
        .available_cursor_modes()
        .await
        .is_ok_and(|m| m.contains(CursorMode::Embedded))
    {
        CursorMode::Embedded
    } else {
        CursorMode::Hidden
    };
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(cursor)
                .set_sources(BitFlags::from(SourceType::Monitor))
                .set_multiple(false)
                .set_restore_token(token.as_deref().map(str::trim))
                .set_persist_mode(PersistMode::ExplicitlyRevoked),
        )
        .await?
        .response()
        .map_err(cancelled)?;
    let response = proxy
        .start(&session, None, StartCastOptions::default())
        .await?
        .response()
        .map_err(cancelled)?;
    let stream = response.streams().first().ok_or(Error::NoStream)?;
    if stream.source_type() == Some(SourceType::Virtual) {
        let _ = session.close().await;
        return Err(Error::VirtualScreen);
    }
    if let Some(token) = response.restore_token() {
        let path = token_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, token);
    }
    let area = stream.size().map(|(w, h)| {
        let (x, y) = stream.position().unwrap_or_default();
        Rect::new(x.into(), y.into(), w.into(), h.into())
    });
    let node = stream.pipe_wire_node_id();
    let fd = proxy
        .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
        .await?;
    Ok(Cast {
        session,
        fd,
        node,
        area,
    })
}

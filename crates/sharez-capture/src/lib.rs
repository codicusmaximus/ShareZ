//! Freezing the screen: one still image of the whole desktop, taken before the region overlay
//! appears so the user selects from what was on screen when they pressed the hotkey.
//!
//! Wayland desktops only allow this through the xdg-desktop-portal Screenshot portal (no dialog
//! with `interactive=false`; ~0.18 s on KDE, ~0.5 s on GNOME in the feasibility spike). X11 reads
//! the root window directly.

mod portal;
mod x11;

use sharez_core::{DisplayServer, Frame, Session};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("screenshot portal: {0}")]
    Portal(#[from] ashpd::Error),
    /// The desktop refused, usually because the user denied ShareZ screenshots once and the
    /// desktop remembers that.
    #[error("the desktop didn't allow ShareZ to take a screenshot")]
    Denied,
    #[error("the portal returned {0}, which is not a local file")]
    NotAFile(String),
    #[error("reading the screenshot: {0}")]
    Io(#[from] std::io::Error),
    #[error("decoding the screenshot: {0}")]
    Png(#[from] png::DecodingError),
    #[error("X11: {0}")]
    X11(String),
    #[error("unsupported pixel format: {0}")]
    UnsupportedFormat(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// How the screen is captured in this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// xdg-desktop-portal Screenshot, for Wayland.
    Portal,
    /// `GetImage` on the X11 root window. Under XWayland this fails, which is why Wayland
    /// sessions never use it.
    X11,
}

impl Backend {
    #[must_use]
    pub fn for_session(session: &Session) -> Self {
        match session.display {
            DisplayServer::Wayland => Self::Portal,
            DisplayServer::X11 => Self::X11,
        }
    }

    /// Captures every monitor as one image, in buffer pixels.
    pub async fn freeze(self) -> Result<Frame> {
        match self {
            Self::Portal => portal::freeze().await,
            // A single round trip that takes a few milliseconds, so it runs inline.
            Self::X11 => x11::freeze(),
        }
    }
}

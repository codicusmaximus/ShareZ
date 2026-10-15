//! Where the windows are, so the region overlay can snap to them.
//!
//! Wayland gives applications no way to list other windows, so each desktop needs its own route
//! (all confirmed in the feasibility spike):
//!
//! - **KDE Plasma:** a temporary KWin script that reports the window stack back over D-Bus.
//! - **GNOME:** the ShareZ Helper shell extension, or without it GNOME Shell's own accessibility
//!   tree. `org.gnome.Shell.Introspect` is denied to normal apps.
//! - **X11:** EWMH properties on the root window.

mod a11y;
mod dbus;
mod elements;
mod ewmh;
mod gnome;
mod kwin;
mod shot;

pub use a11y::ensure_enabled as ensure_accessibility;
pub use elements::{Elements, Target};
pub use gnome::{enable_helper, helper_features, helper_set_recording};
use serde::Serialize;
use sharez_core::{Desktop, DisplayServer, Rect, Session};
pub use shot::{Shot, capture};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("D-Bus: {0}")]
    DBus(#[from] zbus::Error),
    #[error("{0} did not answer in time")]
    Timeout(&'static str),
    #[error("unexpected data from {0}: {1}")]
    BadData(&'static str, String),
    #[error("X11: {0}")]
    X11(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("decoding the window image: {0}")]
    Png(#[from] png::DecodingError),
    #[error("this desktop can't capture single windows")]
    Unsupported,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    Normal,
    Dialog,
    /// Panels and docks.
    Dock,
    /// The desktop background window.
    Desktop,
    Other,
}

/// How the desktop names a window, to capture it on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Handle {
    /// Mutter's stable sequence number, through the helper extension.
    Gnome(u32),
    /// KWin's internal id, a UUID.
    Kwin(String),
    /// The client window.
    X11(u32),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    /// Empty where the source has no titles (GNOME's accessibility tree).
    pub title: String,
    /// The app identifier (KWin resource class, X11 `WM_CLASS`), if known.
    pub app_id: Option<String>,
    pub pid: Option<u32>,
    pub kind: WindowKind,
    /// The visible window including server-side decorations, in logical pixels.
    /// From GNOME Shell's accessibility tree this still includes client-side shadows; see
    /// [`Window::shadow_included`].
    pub frame: Rect,
    /// The client area without server-side decorations, when the source knows it.
    pub client: Option<Rect>,
    /// True when `frame` may include an invisible client-side shadow that must be inset before
    /// snapping (GNOME reports GTK 4 windows about 25 px larger on each side).
    pub shadow_included: bool,
    pub minimized: bool,
    /// For capturing the window's own pixels, if the source can.
    pub handle: Option<Handle>,
}

/// How windows are listed in this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Kwin,
    GnomeShell,
    Ewmh,
}

impl Source {
    /// `None` on Wayland desktops other than KDE and GNOME, where ShareZ falls back to plain
    /// region selection.
    #[must_use]
    pub fn for_session(session: &Session) -> Option<Self> {
        match (&session.display, &session.desktop) {
            (DisplayServer::X11, _) => Some(Self::Ewmh),
            (DisplayServer::Wayland, Desktop::Kde) => Some(Self::Kwin),
            (DisplayServer::Wayland, Desktop::Gnome) => Some(Self::GnomeShell),
            (DisplayServer::Wayland, Desktop::Other(_)) => None,
        }
    }

    /// Lists windows, topmost first where the source knows the stacking order (everywhere except
    /// GNOME without the helper extension).
    /// Moves one of ShareZ's own windows, found by title, to `rect` (logical) and keeps it above
    /// other windows. Wayland apps can't position their windows themselves. `Ok(false)` if the
    /// window wasn't found yet.
    ///
    /// `ring` is for a window that is only a frame: on X11, which may have no compositor to make
    /// the rest transparent, the window is cut down to a frame that many pixels wide.
    pub async fn place(self, title: &str, rect: Rect, ring: Option<u16>) -> Result<bool> {
        match self {
            Self::Kwin => kwin::place(title, rect).await,
            Self::GnomeShell => gnome::place(title, rect).await,
            Self::Ewmh => ewmh::place(title, rect, ring),
        }
    }

    pub async fn windows(self) -> Result<Vec<Window>> {
        match self {
            Self::Kwin => kwin::windows().await,
            Self::GnomeShell => gnome::windows().await,
            Self::Ewmh => ewmh::windows(),
        }
    }
}

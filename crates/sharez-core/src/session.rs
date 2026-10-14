//! Which display server and desktop ShareZ is running under. This decides the capture and
//! window-listing backends.

use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayServer {
    Wayland,
    X11,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Desktop {
    Gnome,
    Kde,
    /// The first entry of `XDG_CURRENT_DESKTOP`, or empty if unset.
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub display: DisplayServer,
    pub desktop: Desktop,
}

impl Session {
    /// Reads the session from the environment. `None` without any display server.
    #[must_use]
    pub fn detect() -> Option<Self> {
        let var = |k| env::var(k).ok().filter(|v: &String| !v.is_empty());
        Self::from_vars(
            var("XDG_SESSION_TYPE").as_deref(),
            var("WAYLAND_DISPLAY").is_some(),
            var("DISPLAY").is_some(),
            var("XDG_CURRENT_DESKTOP").as_deref(),
        )
    }

    fn from_vars(
        session_type: Option<&str>,
        wayland_display: bool,
        x_display: bool,
        current_desktop: Option<&str>,
    ) -> Option<Self> {
        // Wayland sessions also set DISPLAY for XWayland, so Wayland wins when both are present.
        // An explicit x11 session is respected even if a nested Wayland display is around.
        let display = if session_type == Some("x11") && x_display {
            DisplayServer::X11
        } else if wayland_display {
            DisplayServer::Wayland
        } else if x_display {
            DisplayServer::X11
        } else {
            return None;
        };
        // XDG_CURRENT_DESKTOP is a colon-separated list such as "ubuntu:GNOME".
        let entries: Vec<&str> = current_desktop.unwrap_or_default().split(':').collect();
        let has = |name: &str| entries.iter().any(|e| e.eq_ignore_ascii_case(name));
        let desktop = if has("GNOME") {
            Desktop::Gnome
        } else if has("KDE") {
            Desktop::Kde
        } else {
            Desktop::Other(entries[0].to_owned())
        };
        Some(Self { display, desktop })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_wins_over_xwayland() {
        let s = Session::from_vars(Some("wayland"), true, true, Some("KDE")).unwrap();
        assert_eq!(s.display, DisplayServer::Wayland);
        assert_eq!(s.desktop, Desktop::Kde);
    }

    #[test]
    fn desktop_list_and_fallbacks() {
        let s = Session::from_vars(None, false, true, Some("ubuntu:GNOME")).unwrap();
        assert_eq!(
            s,
            Session {
                display: DisplayServer::X11,
                desktop: Desktop::Gnome
            }
        );
        let s = Session::from_vars(Some("x11"), false, true, Some("XFCE")).unwrap();
        assert_eq!(s.desktop, Desktop::Other("XFCE".into()));
        assert!(Session::from_vars(Some("tty"), false, false, None).is_none());
    }
}

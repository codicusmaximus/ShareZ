//! Whether the desktop lets ShareZ take screenshots.
//!
//! With an app ID, the screenshot portal asks the user once before ShareZ may take screenshots and
//! remembers the answer in the permission store. GNOME only shows that question while ShareZ has
//! focus, so a capture started from a shortcut or the tray, with no ShareZ window open, fails
//! without asking. ShareZ therefore checks the store up front and, if there's no answer yet, asks
//! from its own window.

use gtk::{gio, glib::prelude::ToVariant};

const STORE: &str = "org.freedesktop.impl.portal.PermissionStore";
const STORE_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";

/// What the user has said about ShareZ taking screenshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screenshots {
    Allowed,
    Denied,
    /// Never asked, or the store can't be read.
    Unknown,
}

/// Reads ShareZ's entry in the permission store.
pub async fn screenshots() -> Screenshots {
    let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else {
        return Screenshots::Unknown;
    };
    let reply = bus
        .call_future(
            Some(STORE),
            STORE_PATH,
            STORE,
            "Lookup",
            Some(&("screenshot", "screenshot").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
        )
        .await;
    // Lookup returns (a{sas} permissions, v data).
    let Ok(permissions) = reply.map(|r| r.child_value(0)) else {
        return Screenshots::Unknown;
    };
    let entries: std::collections::HashMap<String, Vec<String>> =
        permissions.get().unwrap_or_default();
    from_entry(entries.get(crate::APP_ID).map(Vec::as_slice))
}

fn from_entry(entry: Option<&[String]>) -> Screenshots {
    match entry.and_then(<[String]>::first).map(String::as_str) {
        Some("yes") => Screenshots::Allowed,
        Some("no") => Screenshots::Denied,
        _ => Screenshots::Unknown,
    }
}

/// Whether ShareZ should check: on Wayland, outside Flatpak (a Flatpak can't read the store).
pub fn applies() -> bool {
    sharez_core::Session::detect().is_some_and(|s| s.display == sharez_core::DisplayServer::Wayland)
        && !ashpd::is_sandboxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_entries() {
        let yes = ["yes".to_owned()];
        let no = ["no".to_owned()];
        assert_eq!(from_entry(Some(&yes)), Screenshots::Allowed);
        assert_eq!(from_entry(Some(&no)), Screenshots::Denied);
        assert_eq!(from_entry(None), Screenshots::Unknown);
        assert_eq!(from_entry(Some(&[])), Screenshots::Unknown);
    }
}

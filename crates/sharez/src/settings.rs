//! The few choices ShareZ remembers between runs, in `~/.config/sharez/settings.ini`, and whether
//! it starts at login.

use std::path::PathBuf;

use gtk::glib;

use crate::hotkeys::{Hotkey, Trigger};

const RECORD: &str = "record";
const SHORTCUTS: &str = "shortcuts";

fn path() -> PathBuf {
    glib::user_config_dir().join("sharez").join("settings.ini")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub desktop_audio: bool,
    pub microphone: bool,
    /// Count 3, 2, 1 over the region before recording starts.
    pub countdown: bool,
    /// How fast the countdown runs: 1 is one number a second.
    pub countdown_speed: f64,
    /// The keys for each shortcut on X11, in the shortcuts spec's form. On Wayland the desktop
    /// keeps them instead.
    pub shortcuts: Vec<(Hotkey, String)>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            desktop_audio: true,
            microphone: false,
            countdown: false,
            countdown_speed: 1.5,
            shortcuts: Hotkey::ALL
                .into_iter()
                .map(|h| (h, h.default_trigger().to_owned()))
                .collect(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let file = glib::KeyFile::new();
        let mut s = Self::default();
        if file
            .load_from_file(path(), glib::KeyFileFlags::NONE)
            .is_ok()
        {
            if let Ok(v) = file.boolean(RECORD, "desktop-audio") {
                s.desktop_audio = v;
            }
            if let Ok(v) = file.boolean(RECORD, "microphone") {
                s.microphone = v;
            }
            if let Ok(v) = file.boolean(RECORD, "countdown") {
                s.countdown = v;
            }
            if let Ok(v) = file.double(RECORD, "countdown-speed") {
                s.countdown_speed = snap_speed(v);
            }
            for (hotkey, keys) in &mut s.shortcuts {
                if let Ok(v) = file.string(SHORTCUTS, hotkey.id())
                    && Trigger::parse(&v).is_some()
                {
                    *keys = v.into();
                }
            }
        }
        s
    }

    pub fn save(&self) {
        let file = glib::KeyFile::new();
        file.set_boolean(RECORD, "desktop-audio", self.desktop_audio);
        file.set_boolean(RECORD, "microphone", self.microphone);
        file.set_boolean(RECORD, "countdown", self.countdown);
        file.set_double(RECORD, "countdown-speed", self.countdown_speed);
        for (hotkey, keys) in &self.shortcuts {
            file.set_string(SHORTCUTS, hotkey.id(), keys);
        }
        let path = path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = file.save_to_file(&path) {
            eprintln!("Could not save {}: {e}", path.display());
        }
    }
}

/// The countdown speeds offered: 0.25× to 2× in quarter steps.
pub const SPEEDS: (f64, f64, f64) = (0.25, 2.0, 0.25);

/// The nearest offered speed.
pub fn snap_speed(v: f64) -> f64 {
    let (min, max, step) = SPEEDS;
    if v.is_finite() {
        ((v / step).round() * step).clamp(min, max)
    } else {
        1.0
    }
}

fn autostart_path() -> PathBuf {
    glib::user_config_dir()
        .join("autostart")
        .join(format!("{}.desktop", crate::APP_ID))
}

/// Whether ShareZ starts, in the background, when the user logs in.
pub fn starts_at_login() -> bool {
    autostart_path().exists()
}

/// Adds or removes the autostart entry. It runs this same executable.
pub fn set_starts_at_login(on: bool) -> Result<(), String> {
    let path = autostart_path();
    if !on {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, autostart_entry(&exe.to_string_lossy())).map_err(|e| e.to_string())
}

fn autostart_entry(exe: &str) -> String {
    // Desktop-entry Exec quoting: the path in double quotes, with ", `, $ and \ escaped.
    let quoted: String = exe
        .chars()
        .flat_map(|c| {
            let escape = matches!(c, '"' | '`' | '$' | '\\');
            escape.then_some('\\').into_iter().chain([c])
        })
        .collect();
    format!(
        "[Desktop Entry]\nType=Application\nName=ShareZ\nComment=Capture and record your screen\n\
         Exec=\"{quoted}\" --background\nIcon=applets-screenshooter\nTerminal=false\n\
         X-GNOME-Autostart-enabled=true\nX-GNOME-UsesNotifications=true\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_snap_to_quarters() {
        assert!((snap_speed(1.4) - 1.5).abs() < f64::EPSILON);
        assert!((snap_speed(0.0) - 0.25).abs() < f64::EPSILON);
        assert!((snap_speed(9.0) - 2.0).abs() < f64::EPSILON);
        assert!((snap_speed(f64::NAN) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn autostart_runs_in_the_background() {
        let entry = autostart_entry("/home/me/.local/bin/sharez");
        assert!(entry.contains("Exec=\"/home/me/.local/bin/sharez\" --background\n"));
    }

    #[test]
    fn autostart_quotes_odd_paths() {
        let entry = autostart_entry("/home/me/my \"apps\"/$x/sharez");
        assert!(entry.contains(r#"Exec="/home/me/my \"apps\"/\$x/sharez" --background"#));
    }
}

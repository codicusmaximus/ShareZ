//! The few choices ShareZ remembers between runs, in `~/.config/sharez/settings.ini`, and whether
//! it starts at login.

use std::path::PathBuf;

use gtk::glib;

use crate::hotkeys::{After, Capture, Hotkey, Trigger};

const RECORD: &str = "record";
const HOTKEYS: &str = "hotkeys";
/// Where 0.1 kept the two fixed shortcuts' keys on X11, by hotkey id.
const OLD_SHORTCUTS: &str = "shortcuts";

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
    /// The user's hotkeys, in the order they are listed.
    pub hotkeys: Vec<Hotkey>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            desktop_audio: true,
            microphone: false,
            countdown: false,
            countdown_speed: 1.5,
            hotkeys: Hotkey::defaults(),
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
            s.hotkeys = load_hotkeys(&file);
        }
        s
    }

    pub fn save(&self) {
        let file = glib::KeyFile::new();
        file.set_boolean(RECORD, "desktop-audio", self.desktop_audio);
        file.set_boolean(RECORD, "microphone", self.microphone);
        file.set_boolean(RECORD, "countdown", self.countdown);
        file.set_double(RECORD, "countdown-speed", self.countdown_speed);
        save_hotkeys(&file, &self.hotkeys);
        let path = path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = file.save_to_file(&path) {
            eprintln!("Could not save {}: {e}", path.display());
        }
    }
}

fn group(id: &str) -> String {
    format!("hotkey {id}")
}

fn save_hotkeys(file: &glib::KeyFile, hotkeys: &[Hotkey]) {
    let ids: Vec<&str> = hotkeys.iter().map(|h| h.id.as_str()).collect();
    file.set_string(HOTKEYS, "order", &ids.join(";"));
    for h in hotkeys {
        let group = group(&h.id);
        file.set_string(&group, "name", &h.name);
        file.set_string(&group, "capture", h.capture.id());
        file.set_string(&group, "after", h.after.id());
        file.set_string(&group, "keys", &h.keys);
    }
}

/// The hotkeys in a settings file. One from before hotkeys could be edited gets the defaults,
/// keeping any keys chosen for the two shortcuts it had.
fn load_hotkeys(file: &glib::KeyFile) -> Vec<Hotkey> {
    let Ok(ids) = file.string(HOTKEYS, "order") else {
        let mut hotkeys = Hotkey::defaults();
        for h in &mut hotkeys {
            if let Ok(v) = file.string(OLD_SHORTCUTS, &h.id)
                && Trigger::parse(&v).is_some()
            {
                h.keys = v.into();
            }
        }
        return hotkeys;
    };
    let mut hotkeys: Vec<Hotkey> = Vec::new();
    for id in ids.split(';').map(str::trim).filter(|id| !id.is_empty()) {
        let group = group(id);
        let (Some(capture), Some(after)) = (
            file.string(&group, "capture")
                .ok()
                .and_then(|c| Capture::from_id(&c)),
            file.string(&group, "after")
                .ok()
                .and_then(|a| After::from_id(&a)),
        ) else {
            continue;
        };
        if hotkeys.iter().any(|h| h.id == id) {
            continue;
        }
        let keys = file
            .string(&group, "keys")
            .ok()
            .filter(|k| Trigger::parse(k).is_some())
            .unwrap_or_default();
        let name = file.string(&group, "name").unwrap_or_default();
        hotkeys.push(Hotkey::new(id, &name, capture, after, &keys));
    }
    hotkeys
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

    fn keyfile(text: &str) -> glib::KeyFile {
        let file = glib::KeyFile::new();
        file.load_from_data(text, glib::KeyFileFlags::NONE).unwrap();
        file
    }

    #[test]
    fn old_settings_keep_their_keys() {
        let file = keyfile("[shortcuts]\ncapture-region=ALT+F1\nrecord-region=junk+\n");
        let hotkeys = load_hotkeys(&file);
        assert_eq!(hotkeys.len(), Hotkey::defaults().len());
        let region = hotkeys.iter().find(|h| h.id == "capture-region").unwrap();
        assert_eq!(region.keys, "ALT+F1");
        let record = hotkeys.iter().find(|h| h.id == "record-region").unwrap();
        assert_eq!(record.keys, "CTRL+SHIFT+Print");
    }

    #[test]
    fn hotkeys_round_trip() {
        let mut hotkeys = Hotkey::defaults();
        hotkeys.swap(0, 1);
        hotkeys[0].name = "Mine; with \"odd\" = text".into();
        hotkeys[1].keys = String::new();
        let file = glib::KeyFile::new();
        save_hotkeys(&file, &hotkeys);
        let back = load_hotkeys(&keyfile(&file.to_data()));
        assert_eq!(back, hotkeys);
    }

    #[test]
    fn broken_hotkeys_are_skipped() {
        let file = keyfile(
            "[hotkeys]\norder=a;b;a;\n[hotkey a]\ncapture=screen\nafter=copy\nkeys=Print\n\
             [hotkey b]\ncapture=teleport\nafter=copy\n",
        );
        let hotkeys = load_hotkeys(&file);
        assert_eq!(hotkeys.len(), 1);
        assert_eq!(hotkeys[0].capture, Capture::Screen);
        assert_eq!(hotkeys[0].after, After::Copy);
    }

    #[test]
    fn deleting_every_hotkey_sticks() {
        let file = keyfile("[hotkeys]\norder=\n");
        assert_eq!(load_hotkeys(&file), Vec::<Hotkey>::new());
    }

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

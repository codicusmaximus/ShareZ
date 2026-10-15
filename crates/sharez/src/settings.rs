//! The few choices ShareZ remembers between runs, in `~/.config/sharez/settings.ini`.

use std::path::PathBuf;

use gtk::glib;

const RECORD: &str = "record";

fn path() -> PathBuf {
    glib::user_config_dir().join("sharez").join("settings.ini")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub desktop_audio: bool,
    pub microphone: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            desktop_audio: true,
            microphone: false,
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
        }
        s
    }

    pub fn save(self) {
        let file = glib::KeyFile::new();
        file.set_boolean(RECORD, "desktop-audio", self.desktop_audio);
        file.set_boolean(RECORD, "microphone", self.microphone);
        let path = path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = file.save_to_file(&path) {
            eprintln!("Could not save {}: {e}", path.display());
        }
    }
}

//! Global shortcuts on X11: passive key grabs on the root window.

use std::{collections::HashMap, thread};

use gtk::{gdk, glib::translate::IntoGlib};
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xproto::{
            ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, GrabMode, Keycode,
            ModMask, Window, WindowClass,
        },
    },
    rust_connection::RustConnection,
};

use crate::hotkeys::Trigger;

/// The modifiers a shortcut is made of. Caps Lock and Num Lock are ignored.
/// X11's modifier bits (`ModMask`): Shift, Lock, Control, Mod1 (Alt), Mod2 (Num Lock), Mod4.
const SHIFT: u16 = 1;
const LOCK: u16 = 1 << 1;
const CONTROL: u16 = 1 << 2;
const ALT: u16 = 1 << 3;
const NUM_LOCK: u16 = 1 << 4;
const SUPER: u16 = 1 << 6;
const RELEVANT: u16 = SHIFT | CONTROL | ALT | SUPER;
const LOCKS: [u16; 4] = [0, LOCK, NUM_LOCK, LOCK | NUM_LOCK];

fn mask(t: &Trigger) -> u16 {
    let m = t.modifiers;
    [
        (m.shift, SHIFT),
        (m.ctrl, CONTROL),
        (m.alt, ALT),
        (m.logo, SUPER),
    ]
    .into_iter()
    .filter_map(|(on, bit)| on.then_some(bit))
    .fold(0, |a, b| a | b)
}

/// The shortcuts while grabbed. Dropping it lets them go.
pub struct Grabs {
    conn: std::sync::Arc<RustConnection>,
    /// Woken with a client message so the listening thread can end.
    wake: Window,
    /// Hotkey id → its keys, for people.
    labels: Vec<(String, String)>,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("X11: {e}")
}

impl Grabs {
    /// Grabs `keys` (hotkey id, spec trigger); the ids of those pressed go to `pressed`. Also
    /// returns a sentence for each key that couldn't be grabbed, such as one another app already
    /// has.
    pub fn start(
        keys: &[(String, String)],
        pressed: async_channel::Sender<String>,
    ) -> Result<(Self, Vec<String>), String> {
        let (conn, screen) = RustConnection::connect(None).map_err(err)?;
        let conn = std::sync::Arc::new(conn);
        let root = conn.setup().roots[screen].root;
        let codes = keycodes(&conn)?;
        let mut grabs: HashMap<(Keycode, u16), String> = HashMap::new();
        let mut labels = Vec::new();
        let mut problems = Vec::new();
        for (hotkey, spec) in keys {
            let Some(trigger) = Trigger::parse(spec) else {
                problems.push(format!("{spec} isn't a shortcut ShareZ understands"));
                continue;
            };
            let Some(code) =
                gdk::Key::from_name(&trigger.key).and_then(|k| codes.get(&k.into_glib()).copied())
            else {
                problems.push(format!("{} isn't on this keyboard", trigger.label()));
                continue;
            };
            let m = mask(&trigger);
            let mut ok = true;
            for lock in LOCKS {
                let grabbed = conn
                    .grab_key(
                        false,
                        root,
                        ModMask::from(m | lock),
                        code,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )
                    .map_err(err)?
                    .check();
                ok &= grabbed.is_ok();
            }
            if ok {
                grabs.insert((code, m), hotkey.clone());
                labels.push((hotkey.clone(), trigger.label()));
            } else {
                problems.push(format!(
                    "{} is already used by another app",
                    trigger.label()
                ));
            }
        }
        let wake = conn.generate_id().map_err(err)?;
        conn.create_window(
            0,
            wake,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new(),
        )
        .map_err(err)?;
        conn.flush().map_err(err)?;
        let listener = conn.clone();
        thread::spawn(move || {
            while let Ok(event) = listener.wait_for_event() {
                match event {
                    Event::KeyPress(k) => {
                        let state = u16::from(k.state) & RELEVANT;
                        if let Some(h) = grabs.get(&(k.detail, state)) {
                            let _ = pressed.send_blocking(h.clone());
                        }
                    }
                    Event::ClientMessage(m) if m.window == wake => break,
                    _ => {}
                }
            }
        });
        Ok((Self { conn, wake, labels }, problems))
    }

    pub fn labels(&self) -> Vec<(String, String)> {
        self.labels.clone()
    }
}

impl Drop for Grabs {
    fn drop(&mut self) {
        let root = self.conn.setup().roots[0].root;
        let _ = self
            .conn
            .ungrab_key(x11rb::protocol::xproto::Grab::ANY, root, ModMask::ANY);
        let message = ClientMessageEvent::new(32, self.wake, x11rb::NONE, [0u32; 5]);
        let _ = self
            .conn
            .send_event(false, self.wake, EventMask::NO_EVENT, message);
        let _ = self.conn.destroy_window(self.wake);
        let _ = self.conn.flush();
    }
}

/// Keysym → the first keycode that types it.
fn keycodes(conn: &RustConnection) -> Result<HashMap<u32, Keycode>, String> {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let map = conn
        .get_keyboard_mapping(min, max - min + 1)
        .map_err(err)?
        .reply()
        .map_err(err)?;
    let per = usize::from(map.keysyms_per_keycode).max(1);
    let mut codes = HashMap::new();
    for (i, syms) in map.keysyms.chunks(per).enumerate() {
        let Ok(code) = Keycode::try_from(usize::from(min) + i) else {
            break;
        };
        for &sym in syms.iter().filter(|&&s| s != 0) {
            codes.entry(sym).or_insert(code);
        }
    }
    Ok(codes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks() {
        let t = Trigger::parse("CTRL+ALT+Print").unwrap();
        assert_eq!(
            mask(&t),
            u16::from(ModMask::CONTROL) | u16::from(ModMask::M1)
        );
        assert_eq!(mask(&Trigger::parse("Print").unwrap()), 0);
    }
}

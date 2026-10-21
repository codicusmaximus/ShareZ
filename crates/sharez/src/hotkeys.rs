//! Global keyboard shortcuts, which work while ShareZ is in the background.
//!
//! Wayland doesn't let apps listen to keys they don't have focus for, so there the desktop's
//! `GlobalShortcuts` portal binds them: ShareZ suggests a key, and GNOME or KDE asks the user to
//! confirm it the first time and lets them change it in their own settings. On X11 ShareZ grabs
//! the keys on the root window itself and keeps them in its settings.
//!
//! Shortcuts are written as in the XDG shortcuts spec (`CTRL+SHIFT+Print`): modifiers, then an
//! xkb key name, joined with `+`.

use std::cell::RefCell;

use ashpd::desktop::{
    CreateSessionOptions, Session,
    global_shortcuts::{
        BindShortcutsOptions, ConfigureShortcutsOptions, GlobalShortcuts, ListShortcutsOptions,
        NewShortcut, Shortcut,
    },
};
use futures_util::StreamExt;
use gtk::{gdk, glib};
use sharez_core::{DisplayServer, Session as Desktop};

/// What a hotkey captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capture {
    /// Pick a region, window or element in the overlay.
    Region,
    /// Everything on screen, straight away.
    Screen,
    /// The region picked last time, straight away (or a new one if there is none yet).
    LastRegion,
    /// Start recording a region, or stop the recording that is running.
    Record,
}

impl Capture {
    pub const ALL: [Capture; 4] = [
        Capture::Region,
        Capture::Screen,
        Capture::LastRegion,
        Capture::Record,
    ];

    /// How the settings file spells it.
    pub fn id(self) -> &'static str {
        match self {
            Capture::Region => "region",
            Capture::Screen => "screen",
            Capture::LastRegion => "last-region",
            Capture::Record => "record",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Capture::Region => "Region",
            Capture::Screen => "Whole Screen",
            Capture::LastRegion => "Last Region",
            Capture::Record => "Record Region",
        }
    }

    /// Whether it makes a screenshot, which an after-capture action then handles.
    pub fn is_screenshot(self) -> bool {
        self != Capture::Record
    }
}

/// What happens to a screenshot once it is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum After {
    SaveAndCopy,
    Copy,
    Save,
    /// Show a popup that asks.
    Ask,
}

impl After {
    pub const ALL: [After; 4] = [After::SaveAndCopy, After::Copy, After::Save, After::Ask];

    pub fn id(self) -> &'static str {
        match self {
            After::SaveAndCopy => "save-and-copy",
            After::Copy => "copy",
            After::Save => "save",
            After::Ask => "ask",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            After::SaveAndCopy => "Save and Copy",
            After::Copy => "Copy Only",
            After::Save => "Save Only",
            After::Ask => "Ask What to Do",
        }
    }

    pub fn copies(self) -> bool {
        matches!(self, After::SaveAndCopy | After::Copy)
    }
}

/// A shortcut the user set up: a key, what it captures and what happens to the capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    /// Stable for the hotkey's life. The portal stores the desktop's key under it, so it never
    /// changes when the hotkey is edited.
    pub id: String,
    pub name: String,
    pub capture: Capture,
    pub after: After,
    /// In the shortcuts spec's form, or empty for none. On X11 these are the keys ShareZ grabs;
    /// on Wayland only a suggestion for the desktop, which keeps the real ones.
    pub keys: String,
}

impl Hotkey {
    pub fn new(id: &str, name: &str, capture: Capture, after: After, keys: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            capture,
            after,
            keys: keys.into(),
        }
    }

    /// The hotkeys a new install starts with. Print and Ctrl+Print are ShareX's; ShareX records
    /// with Shift+Print, but GNOME and KDE use that for their own screenshots. Super+Shift+S is
    /// the key Windows' own snipping tool uses for "pick a region, then decide".
    pub fn defaults() -> Vec<Self> {
        vec![
            Self::new(
                "capture-screen",
                "Capture Screen",
                Capture::Screen,
                After::SaveAndCopy,
                "Print",
            ),
            Self::new(
                "capture-region",
                "Capture Region",
                Capture::Region,
                After::SaveAndCopy,
                "CTRL+Print",
            ),
            Self::new(
                "capture-region-ask",
                "Capture Region and Ask",
                Capture::Region,
                After::Ask,
                "SHIFT+LOGO+s",
            ),
            Self::new(
                "record-region",
                "Record Region",
                Capture::Record,
                After::SaveAndCopy,
                "CTRL+SHIFT+Print",
            ),
        ]
    }

    /// A fresh hotkey with an id no other hotkey has had, so the desktop asks about its key.
    pub fn fresh(existing: &[Hotkey]) -> Self {
        let mut n = chrono::Local::now().timestamp_millis();
        while existing.iter().any(|h| h.id == format!("hotkey-{n}")) {
            n += 1;
        }
        let id = format!("hotkey-{n}");
        Self::new(&id, "New Hotkey", Capture::Region, After::Ask, "")
    }

    /// "Region, then ask what to do", for the list of hotkeys.
    pub fn summary(&self) -> String {
        if self.capture.is_screenshot() {
            format!("{} · {}", self.capture.title(), self.after.title())
        } else {
            self.capture.title().to_owned()
        }
    }

    /// What the desktop shows next to the key in its own settings.
    fn description(&self) -> String {
        if self.name.trim().is_empty() {
            self.summary()
        } else {
            self.name.clone()
        }
    }
}

/// Modifier keys, as in the shortcuts spec.
#[allow(
    clippy::struct_excessive_bools,
    reason = "one per modifier key, as the spec has them"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

/// A parsed shortcut: modifiers and a key name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub modifiers: Modifiers,
    pub key: String,
}

impl Trigger {
    /// Parses `CTRL+SHIFT+Print`. Modifier names are case-insensitive; `NUM` is accepted and
    /// ignored, since Num Lock shouldn't change what a shortcut does.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let key = parts.pop().filter(|k| !k.is_empty())?.to_owned();
        let mut modifiers = Modifiers::default();
        for m in parts {
            match m.to_ascii_uppercase().as_str() {
                "CTRL" | "CONTROL" => modifiers.ctrl = true,
                "ALT" => modifiers.alt = true,
                "SHIFT" => modifiers.shift = true,
                "LOGO" | "SUPER" | "META" => modifiers.logo = true,
                "NUM" => {}
                _ => return None,
            }
        }
        Some(Self { modifiers, key })
    }

    /// From a key press: the key and the modifiers held with it. `None` for a lone modifier.
    pub fn from_press(key: gdk::Key, state: gdk::ModifierType) -> Option<Self> {
        let name = key.to_lower().name()?.to_string();
        let modifier_key = [
            "Shift_L",
            "Shift_R",
            "Control_L",
            "Control_R",
            "Alt_L",
            "Alt_R",
            "Super_L",
            "Super_R",
            "Meta_L",
            "Meta_R",
            "ISO_Level3_Shift",
            "Caps_Lock",
            "Num_Lock",
        ];
        if modifier_key.contains(&name.as_str()) {
            return None;
        }
        Some(Self {
            modifiers: Modifiers {
                ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
                alt: state.contains(gdk::ModifierType::ALT_MASK),
                shift: state.contains(gdk::ModifierType::SHIFT_MASK),
                logo: state.contains(gdk::ModifierType::SUPER_MASK),
            },
            key: name,
        })
    }

    /// Back to the spec's form.
    pub fn to_spec(&self) -> String {
        let m = self.modifiers;
        let mut parts: Vec<&str> = [
            (m.ctrl, "CTRL"),
            (m.alt, "ALT"),
            (m.shift, "SHIFT"),
            (m.logo, "LOGO"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        parts.push(&self.key);
        parts.join("+")
    }

    /// For people: `Ctrl+Shift+Print`.
    pub fn label(&self) -> String {
        let m = self.modifiers;
        let mut parts: Vec<&str> = [
            (m.ctrl, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.logo, "Super"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        parts.push(&self.key);
        parts.join("+")
    }
}

/// A desktop's description of a trigger, for people. GNOME says `Press <Control>Print`, in GTK
/// accelerator syntax; that becomes `Ctrl+Print`. Anything else is shown as it is.
pub fn readable(description: &str) -> String {
    let mut rest = description.strip_prefix("Press ").unwrap_or(description);
    if !rest.starts_with('<') {
        return description.to_owned();
    }
    let mut parts = Vec::new();
    while let Some(tail) = rest.strip_prefix('<') {
        let Some((name, after)) = tail.split_once('>') else {
            return description.to_owned();
        };
        parts.push(match name.to_ascii_lowercase().as_str() {
            "control" | "ctrl" | "primary" => "Ctrl",
            "shift" => "Shift",
            "alt" | "mod1" => "Alt",
            "super" | "mod4" => "Super",
            "meta" => "Meta",
            "hyper" => "Hyper",
            _ => return description.to_owned(),
        });
        rest = after;
    }
    if rest.is_empty() {
        return description.to_owned();
    }
    parts.push(rest);
    parts.join("+")
}

/// A shortcut as a GTK accelerator (`<Control><Shift>Print`), from its readable form.
pub fn gtk_accel(label: &str) -> Option<String> {
    let t = Trigger::parse(label)?;
    let m = t.modifiers;
    let mut accel: String = [
        (m.ctrl, "<Control>"),
        (m.alt, "<Alt>"),
        (m.shift, "<Shift>"),
        (m.logo, "<Super>"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    accel.push_str(&t.key);
    Some(accel)
}

/// The shortcuts while they are bound. Dropping it unbinds them.
pub enum Shortcuts {
    Portal {
        portal: GlobalShortcuts,
        session: Session<GlobalShortcuts>,
        /// What the desktop says each shortcut is, for showing in the window.
        bound: RefCell<Vec<Shortcut>>,
    },
    X11(crate::x11keys::Grabs),
}

impl Shortcuts {
    /// Binds `hotkeys` and sends the id of each one pressed to `pressed`. On X11 ShareZ grabs
    /// their keys; on Wayland the desktop is asked, with their keys as suggestions. Also returns a
    /// sentence for each key that couldn't be bound. `asking` runs when the desktop is about to
    /// ask the user to confirm the keys.
    pub async fn bind(
        desktop: &Desktop,
        hotkeys: &[Hotkey],
        pressed: async_channel::Sender<String>,
        asking: impl FnOnce(),
    ) -> Result<(Self, Vec<String>), String> {
        match desktop.display {
            DisplayServer::X11 => {
                let keys: Vec<(String, String)> = hotkeys
                    .iter()
                    .filter(|h| !h.keys.is_empty())
                    .map(|h| (h.id.clone(), h.keys.clone()))
                    .collect();
                crate::x11keys::Grabs::start(&keys, pressed).map(|(g, p)| (Self::X11(g), p))
            }
            DisplayServer::Wayland => portal(hotkeys, pressed, asking)
                .await
                .map(|s| (s, Vec::new())),
        }
    }

    /// What each hotkey (by id) is bound to, for people. Hotkeys the desktop doesn't mention
    /// have no key.
    pub fn labels(&self) -> Vec<(String, String)> {
        match self {
            Self::Portal { bound, .. } => bound
                .borrow()
                .iter()
                .map(|s| (s.id().to_owned(), readable(s.trigger_description())))
                .filter(|(_, label)| !label.is_empty())
                .collect(),
            Self::X11(grabs) => grabs.labels(),
        }
    }

    /// Whether the desktop, not ShareZ, decides the keys.
    pub fn desktop_owns_keys(&self) -> bool {
        matches!(self, Self::Portal { .. })
    }

    /// The keys as GTK accelerators, for ShareZ's own windows. A desktop's global shortcuts don't
    /// fire while the app that bound them has focus (GNOME passes the key to it instead), so the
    /// window has to handle them itself. X11 grabs work everywhere and need none.
    pub fn window_accels(&self) -> Vec<(String, String)> {
        match self {
            Self::Portal { .. } => self
                .labels()
                .into_iter()
                .filter_map(|(h, label)| Some((h, gtk_accel(&label)?)))
                .collect(),
            Self::X11(_) => Vec::new(),
        }
    }

    /// Whether the desktop can show its own screen for changing the shortcuts.
    pub fn desktop_configures(&self) -> bool {
        matches!(self, Self::Portal { portal, .. } if portal.version() >= 2)
    }

    /// Opens the desktop's screen for changing the shortcuts.
    pub async fn configure(&self) -> Result<(), String> {
        match self {
            Self::Portal {
                portal, session, ..
            } => portal
                .configure_shortcuts(session, None, ConfigureShortcutsOptions::default())
                .await
                .map_err(|e| e.to_string()),
            Self::X11(_) => Ok(()),
        }
    }

    /// Calls `changed` whenever the user changes the shortcuts in the desktop's settings.
    pub fn on_change(self: &std::rc::Rc<Self>, changed: impl Fn() + 'static) {
        if !matches!(self.as_ref(), Self::Portal { .. }) {
            return;
        }
        let this = std::rc::Rc::downgrade(self);
        glib::spawn_future_local(async move {
            // A second proxy on the same connection, since the session's can't be shared.
            let Ok(portal) = GlobalShortcuts::new().await else {
                return;
            };
            let Ok(mut stream) = portal.receive_shortcuts_changed().await else {
                return;
            };
            while let Some(change) = stream.next().await {
                let Some(this) = this.upgrade() else { return };
                // ShareZ has one shortcuts session, so every change is about it.
                if let Self::Portal { bound, .. } = this.as_ref() {
                    *bound.borrow_mut() = change.shortcuts().to_vec();
                    changed();
                }
            }
        });
    }
}

/// How long the desktop gets to answer the calls that show no dialog.
const ANSWER_TIMEOUT: u32 = 15;

/// `call`, or an error if the desktop doesn't answer in time.
pub async fn answered<T>(
    call: impl std::future::Future<Output = ashpd::Result<T>>,
) -> Result<T, String> {
    let call = std::pin::pin!(call);
    let timeout = std::pin::pin!(glib::timeout_future_seconds(ANSWER_TIMEOUT));
    match futures_util::future::select(call, timeout).await {
        futures_util::future::Either::Left((result, _)) => result.map_err(|e| e.to_string()),
        futures_util::future::Either::Right(_) => Err("the desktop didn't answer".into()),
    }
}

async fn portal(
    hotkeys: &[Hotkey],
    pressed: async_channel::Sender<String>,
    asking: impl FnOnce(),
) -> Result<Shortcuts, String> {
    let (portal, session, listed) = answered(async {
        let portal = GlobalShortcuts::new().await?;
        let session = portal
            .create_session(CreateSessionOptions::default())
            .await?;
        let mut activated = portal.receive_activated().await?;
        glib::spawn_future_local(async move {
            while let Some(a) = activated.next().await {
                let _ = pressed.send(a.shortcut_id().to_owned()).await;
            }
        });
        // The desktop remembers what the user chose, so only ask about shortcuts it doesn't know.
        let listed = portal
            .list_shortcuts(&session, ListShortcutsOptions::default())
            .await?
            .response()?
            .shortcuts()
            .to_vec();
        Ok((portal, session, listed))
    })
    .await?;
    let known = |h: &Hotkey| listed.iter().any(|s| s.id() == h.id);
    let bound = if hotkeys.iter().all(known) {
        listed
    } else {
        // All of them, not just the new ones: a session only has the shortcuts it binds.
        let new: Vec<NewShortcut> = hotkeys
            .iter()
            .map(|h| {
                let shortcut = NewShortcut::new(h.id.as_str(), h.description());
                match Trigger::parse(&h.keys) {
                    Some(_) => shortcut.preferred_trigger(h.keys.as_str()),
                    None => shortcut,
                }
            })
            .collect();
        asking();
        // No time limit: the desktop is waiting for the user.
        portal
            .bind_shortcuts(&session, &new, None, BindShortcutsOptions::default())
            .await
            .and_then(|request| request.response())
            .map_err(|e| e.to_string())?
            .shortcuts()
            .to_vec()
    };
    Ok(Shortcuts::Portal {
        portal,
        session,
        bound: RefCell::new(bound),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spec_triggers() {
        let t = Trigger::parse("CTRL+SHIFT+Print").unwrap();
        assert!(t.modifiers.ctrl && t.modifiers.shift && !t.modifiers.alt);
        assert_eq!(t.key, "Print");
        assert_eq!(t.to_spec(), "CTRL+SHIFT+Print");
        assert_eq!(t.label(), "Ctrl+Shift+Print");
    }

    #[test]
    fn modifier_order_is_normalised() {
        let t = Trigger::parse("shift+logo+alt+ctrl+a").unwrap();
        assert_eq!(t.to_spec(), "CTRL+ALT+SHIFT+LOGO+a");
    }

    #[test]
    fn num_lock_is_ignored_and_junk_rejected() {
        assert_eq!(Trigger::parse("NUM+Print").unwrap().to_spec(), "Print");
        assert!(Trigger::parse("HYPER+Print").is_none());
        assert!(Trigger::parse("CTRL+").is_none());
        assert!(Trigger::parse("").is_none());
    }

    #[test]
    fn gnome_descriptions_become_labels() {
        assert_eq!(readable("Press <Control>Print"), "Ctrl+Print");
        assert_eq!(readable("Press <Shift><Control>Print"), "Shift+Ctrl+Print");
        assert_eq!(readable("Meta+Shift+R"), "Meta+Shift+R");
    }

    #[test]
    fn labels_become_window_accels() {
        assert_eq!(gtk_accel("Ctrl+Print").as_deref(), Some("<Control>Print"));
        assert_eq!(
            gtk_accel(&readable("Press <Shift><Control>Print")).as_deref(),
            Some("<Control><Shift>Print")
        );
        assert_eq!(
            gtk_accel("Meta+Shift+R").as_deref(),
            Some("<Shift><Super>R")
        );
        assert_eq!(gtk_accel("not a key+"), None);
    }

    #[test]
    fn defaults_parse_and_ids_are_unique() {
        let defaults = Hotkey::defaults();
        for h in &defaults {
            assert!(Trigger::parse(&h.keys).is_some(), "{}", h.keys);
            assert_eq!(defaults.iter().filter(|o| o.id == h.id).count(), 1);
        }
        let screen = &defaults[0];
        assert_eq!(
            (screen.capture, screen.after),
            (Capture::Screen, After::SaveAndCopy)
        );
        assert_eq!(screen.keys, "Print");
    }

    #[test]
    fn modes_round_trip() {
        for c in Capture::ALL {
            assert_eq!(Capture::from_id(c.id()), Some(c));
        }
        for a in After::ALL {
            assert_eq!(After::from_id(a.id()), Some(a));
        }
        assert!(After::SaveAndCopy.copies() && After::Copy.copies());
        assert!(!After::Ask.copies() && !After::Save.copies());
    }

    #[test]
    fn fresh_ids_are_new() {
        let mut hotkeys = Hotkey::defaults();
        let a = Hotkey::fresh(&hotkeys);
        hotkeys.push(a.clone());
        let b = Hotkey::fresh(&hotkeys);
        assert_ne!(a.id, b.id);
        assert!(hotkeys.iter().all(|h| h.id != b.id));
    }

    #[test]
    fn summaries() {
        let d = Hotkey::defaults();
        assert_eq!(d[2].summary(), "Region · Ask What to Do");
        assert_eq!(d[3].summary(), "Record Region");
    }
}

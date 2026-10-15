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

/// Something a shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hotkey {
    Capture,
    Record,
}

impl Hotkey {
    pub const ALL: [Hotkey; 2] = [Hotkey::Capture, Hotkey::Record];

    /// The id the portal stores it under; also the key in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            Hotkey::Capture => "capture-region",
            Hotkey::Record => "record-region",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Hotkey::Capture => "Capture Region",
            Hotkey::Record => "Record Region",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Hotkey::Capture => "Capture a region of the screen",
            Hotkey::Record => "Start or stop recording a region of the screen",
        }
    }

    /// Ctrl+Print as in ShareX. ShareX records with Shift+Print, but GNOME and KDE use that for
    /// their own screenshots.
    pub fn default_trigger(self) -> &'static str {
        match self {
            Hotkey::Capture => "CTRL+Print",
            Hotkey::Record => "CTRL+SHIFT+Print",
        }
    }

    /// The app action it runs.
    pub fn action(self) -> &'static str {
        match self {
            Hotkey::Capture => "capture",
            Hotkey::Record => "record",
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
    /// Binds the shortcuts and sends each press to `pressed`. `x11` is the keys from the
    /// settings, used on X11 only. Also returns a sentence for each key that couldn't be bound.
    /// `asking` runs when the desktop is about to ask the user to confirm the keys.
    pub async fn bind(
        desktop: &Desktop,
        x11: &[(Hotkey, String)],
        pressed: async_channel::Sender<Hotkey>,
        asking: impl FnOnce(),
    ) -> Result<(Self, Vec<String>), String> {
        match desktop.display {
            DisplayServer::X11 => {
                crate::x11keys::Grabs::start(x11, pressed).map(|(g, p)| (Self::X11(g), p))
            }
            DisplayServer::Wayland => portal(pressed, asking).await.map(|s| (s, Vec::new())),
        }
    }

    /// What each shortcut is bound to, for people. Empty when the desktop doesn't say.
    pub fn labels(&self) -> Vec<(Hotkey, String)> {
        match self {
            Self::Portal { bound, .. } => bound
                .borrow()
                .iter()
                .filter_map(|s| Some((Hotkey::from_id(s.id())?, readable(s.trigger_description()))))
                .collect(),
            Self::X11(grabs) => grabs.labels(),
        }
    }

    /// The keys as GTK accelerators, for ShareZ's own windows. A desktop's global shortcuts don't
    /// fire while the app that bound them has focus (GNOME passes the key to it instead), so the
    /// window has to handle them itself. X11 grabs work everywhere and need none.
    pub fn window_accels(&self) -> Vec<(Hotkey, String)> {
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
    pressed: async_channel::Sender<Hotkey>,
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
                if let Some(hotkey) = Hotkey::from_id(a.shortcut_id()) {
                    let _ = pressed.send(hotkey).await;
                }
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
    let known = |h: Hotkey| listed.iter().any(|s| s.id() == h.id());
    let bound = if Hotkey::ALL.into_iter().all(known) {
        listed
    } else {
        let new: Vec<NewShortcut> = Hotkey::ALL
            .into_iter()
            .map(|h| {
                NewShortcut::new(h.id(), h.description()).preferred_trigger(h.default_trigger())
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
    fn defaults_parse() {
        for h in Hotkey::ALL {
            assert!(Trigger::parse(h.default_trigger()).is_some());
            assert_eq!(Hotkey::from_id(h.id()), Some(h));
        }
    }
}

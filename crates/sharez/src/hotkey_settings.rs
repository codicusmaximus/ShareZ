//! The Hotkeys list in the window, and the dialog that edits one hotkey: its name, what it
//! captures, what happens to the screenshot, and its keys.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use gtk::glib;

use crate::{
    Ui,
    hotkeys::{After, Capture, Hotkey, Trigger, gtk_accel},
};

/// What a hotkey's key shows when it has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    /// The shortcuts are still being bound.
    SettingUp,
    /// The desktop's dialog is waiting for the user.
    Confirming,
    /// There are no global shortcuts on this desktop.
    Unavailable,
    Ready,
}

/// The list of hotkeys in the main window.
pub struct HotkeyList {
    pub group: adw::PreferencesGroup,
    pub add: gtk::Button,
    /// Opens the desktop's own shortcut settings, where it has them.
    pub desktop: gtk::Button,
    rows: RefCell<Vec<adw::ActionRow>>,
}

impl HotkeyList {
    pub fn new() -> Self {
        let group = adw::PreferencesGroup::builder().title("Hotkeys").build();
        let desktop = gtk::Button::builder()
            .icon_name("preferences-desktop-keyboard-shortcuts-symbolic")
            .tooltip_text("Change the Keys in the Desktop's Settings")
            .css_classes(["flat"])
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add Hotkey")
            .css_classes(["flat"])
            .valign(gtk::Align::Center)
            .build();
        let suffix = gtk::Box::builder().spacing(6).build();
        suffix.append(&desktop);
        suffix.append(&add);
        group.set_header_suffix(Some(&suffix));
        Self {
            group,
            add,
            desktop,
            rows: RefCell::default(),
        }
    }

    /// Lists `hotkeys`, each with its key from `keys` (by id) or `state`. Clicking one calls
    /// `edit` with its id.
    pub fn show(
        &self,
        hotkeys: &[Hotkey],
        keys: &[(String, String)],
        state: KeyState,
        edit: &Rc<dyn Fn(String)>,
    ) {
        for row in self.rows.borrow_mut().drain(..) {
            self.group.remove(&row);
        }
        for h in hotkeys {
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .title(title(h))
                .subtitle(h.summary())
                .activatable(true)
                .build();
            let key = keys
                .iter()
                .find(|(id, _)| *id == h.id)
                .map(|(_, k)| k.as_str());
            row.add_suffix(&key_widget(key, state));
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let edit = Rc::clone(edit);
            let id = h.id.clone();
            row.connect_activated(move |_| edit(id.clone()));
            self.group.add(&row);
            self.rows.borrow_mut().push(row);
        }
        if hotkeys.is_empty() {
            let row = adw::ActionRow::builder()
                .title("No hotkeys")
                .subtitle("Add one with the + button")
                .build();
            self.group.add(&row);
            self.rows.borrow_mut().push(row);
        }
    }
}

fn title(h: &Hotkey) -> String {
    if h.name.trim().is_empty() {
        h.capture.title().to_owned()
    } else {
        h.name.clone()
    }
}

/// A key as a keycap, or a dim word when there isn't one.
fn key_widget(key: Option<&str>, state: KeyState) -> gtk::Widget {
    if let Some(accel) = key.and_then(gtk_accel) {
        let label = gtk::ShortcutLabel::new(&accel);
        label.set_valign(gtk::Align::Center);
        return label.upcast();
    }
    let text = match (key, state) {
        // A desktop's description that isn't a plain key, shown as it is.
        (Some(k), _) => k,
        (None, KeyState::SettingUp) => "Setting up…",
        (None, KeyState::Confirming) => "Confirm in the dialog…",
        (None, KeyState::Unavailable) => "Unavailable",
        (None, KeyState::Ready) => "Not set",
    };
    dim(text)
}

fn dim(text: &str) -> gtk::Widget {
    gtk::Label::builder()
        .label(text)
        .css_classes(["dim-label"])
        .valign(gtk::Align::Center)
        .build()
        .upcast()
}

fn capture_subtitle(c: Capture) -> &'static str {
    match c {
        Capture::Region => "Drag a region or click a window",
        Capture::Screen => "Everything on screen, at once",
        Capture::LastRegion => "The region picked last time, at once",
        Capture::Record => "Start or stop recording a region",
    }
}

fn after_subtitle(c: Capture, a: After) -> &'static str {
    if !c.is_screenshot() {
        return if a.for_capture(c).copies() {
            "Saved to Videos/ShareZ and copied as a file"
        } else {
            "Saved to Videos/ShareZ, not copied"
        };
    }
    match a {
        After::SaveAndCopy => "Saved to Pictures/ShareZ and copied",
        After::Copy => "Copied, not saved",
        After::Save => "Saved, not copied",
        After::Ask => "A popup asks what to do",
        After::Annotate => "Opens in the annotator, then asks",
    }
}

/// The other hotkey that already uses `spec`, if any.
fn taken_by<'a>(hotkeys: &'a [Hotkey], id: &str, spec: &str) -> Option<&'a Hotkey> {
    let wanted = Trigger::parse(spec)?;
    hotkeys
        .iter()
        .filter(|h| h.id != id)
        .find(|h| Trigger::parse(&h.keys).is_some_and(|t| t == wanted))
}

/// The dialog that edits one hotkey, and the hotkey as edited so far.
struct Editor {
    dialog: adw::PreferencesDialog,
    name: adw::EntryRow,
    capture: adw::ComboRow,
    after: adw::ComboRow,
    /// What the after-capture row lists, which depends on what the hotkey captures.
    afters: Cell<&'static [After]>,
    keys: adw::ActionRow,
    key_slot: gtk::Box,
    delete: gtk::Button,
    draft: RefCell<Hotkey>,
    /// Waiting for the user to press the new keys.
    recording: Cell<bool>,
    deleted: Cell<bool>,
    /// The key the desktop has for the hotkey. Only the desktop can change that one.
    bound: Option<String>,
    desktop_owns: bool,
}

impl Editor {
    fn new(hotkey: &Hotkey, new: bool, bound: Option<String>, desktop_owns: bool) -> Rc<Self> {
        let name = adw::EntryRow::builder()
            .title("Name")
            .text(&hotkey.name)
            .build();
        let capture = adw::ComboRow::builder()
            .title("Capture")
            .model(&gtk::StringList::new(&Capture::ALL.map(Capture::title)))
            .selected(index(&Capture::ALL, hotkey.capture))
            .build();
        let afters = hotkey.capture.afters();
        let after = adw::ComboRow::builder()
            .title("After Capture")
            .model(&after_model(afters))
            .selected(index(afters, hotkey.after.for_capture(hotkey.capture)))
            .build();
        let what = adw::PreferencesGroup::new();
        what.add(&name);
        what.add(&capture);
        what.add(&after);

        let keys = adw::ActionRow::builder()
            .title("Keys")
            .activatable(true)
            .use_markup(false)
            .build();
        let key_slot = gtk::Box::builder().valign(gtk::Align::Center).build();
        keys.add_suffix(&key_slot);
        let keys_group = adw::PreferencesGroup::new();
        keys_group.add(&keys);

        let delete = gtk::Button::builder()
            .label("Delete Hotkey")
            .css_classes(["pill", "destructive-action"])
            .halign(gtk::Align::Center)
            .build();
        let delete_group = adw::PreferencesGroup::new();
        delete_group.add(&delete);

        let page = adw::PreferencesPage::new();
        page.add(&what);
        page.add(&keys_group);
        page.add(&delete_group);
        let dialog = adw::PreferencesDialog::builder()
            .title(if new { "New Hotkey" } else { "Edit Hotkey" })
            .content_width(440)
            .build();
        dialog.add(&page);
        let editor = Rc::new(Self {
            dialog,
            name,
            capture,
            after,
            afters: Cell::new(afters),
            keys,
            key_slot,
            delete,
            draft: RefCell::new(hotkey.clone()),
            recording: Cell::new(false),
            deleted: Cell::new(false),
            bound,
            desktop_owns,
        });
        editor.sync();
        editor.show_key();
        editor
    }

    /// Copies the rows into the draft, and updates what depends on them.
    fn sync(&self) {
        let capture = Capture::ALL[(self.capture.selected() as usize).min(Capture::ALL.len() - 1)];
        let afters = capture.afters();
        if afters != self.afters.get() {
            // Changing the list runs this again, so nothing may be borrowed here.
            let keep = self.draft.borrow().after.for_capture(capture);
            self.afters.set(afters);
            self.after.set_model(Some(&after_model(afters)));
            self.after.set_selected(index(afters, keep));
        }
        let mut d = self.draft.borrow_mut();
        self.name.text().trim().clone_into(&mut d.name);
        d.capture = capture;
        d.after = afters[(self.after.selected() as usize).min(afters.len() - 1)];
        self.capture.set_subtitle(capture_subtitle(d.capture));
        self.after.set_subtitle(after_subtitle(d.capture, d.after));
    }

    /// Shows the draft's key, and what clicking the row does.
    fn show_key(&self) {
        while let Some(child) = self.key_slot.first_child() {
            self.key_slot.remove(&child);
        }
        if self.recording.get() {
            self.keys
                .set_subtitle("Press the new keys · Esc cancels · Backspace clears");
            self.key_slot.append(&dim("…"));
            return;
        }
        let label = match &self.bound {
            Some(b) => Some(b.clone()),
            None => Trigger::parse(&self.draft.borrow().keys).map(|t| t.label()),
        };
        self.key_slot
            .append(&key_widget(label.as_deref(), KeyState::Ready));
        self.keys
            .set_subtitle(match (&self.bound, self.desktop_owns) {
                (Some(_), _) => "Your desktop keeps this key; click to change it there",
                (None, true) => "Click to suggest keys, which your desktop asks you to confirm",
                (None, false) => "Click to change",
            });
    }

    /// Takes a key press while recording keys. Escape cancels, Backspace clears the keys.
    fn press(&self, hotkeys: &[Hotkey], key: gtk::gdk::Key, state: gtk::gdk::ModifierType) {
        let plain = !state.intersects(
            gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SHIFT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK,
        );
        if plain && key == gtk::gdk::Key::BackSpace {
            self.draft.borrow_mut().keys.clear();
        } else if !(plain && key == gtk::gdk::Key::Escape) {
            // A modifier on its own: wait for the rest.
            let Some(trigger) = Trigger::from_press(key, state) else {
                return;
            };
            let spec = trigger.to_spec();
            let id = self.draft.borrow().id.clone();
            if let Some(other) = taken_by(hotkeys, &id, &spec) {
                self.dialog.add_toast(adw::Toast::new(&format!(
                    "{} already runs {}",
                    trigger.label(),
                    title(other)
                )));
                return;
            }
            self.draft.borrow_mut().keys = spec;
        }
        self.recording.set(false);
        self.show_key();
    }
}

fn after_model(afters: &[After]) -> gtk::StringList {
    let titles: Vec<&str> = afters.iter().map(|a| a.title()).collect();
    gtk::StringList::new(&titles)
}

fn index<T: PartialEq + Copy>(all: &[T], value: T) -> u32 {
    all.iter().position(|v| *v == value).unwrap_or(0) as u32
}

/// Opens the dialog for the hotkey `id`. Changes apply when it closes. `new` is a hotkey just
/// added, which the desktop hasn't heard of yet.
pub fn edit(ui: &Rc<Ui>, id: &str, new: bool) {
    let Some(hotkey) = ui
        .settings
        .borrow()
        .hotkeys
        .iter()
        .find(|h| h.id == id)
        .cloned()
    else {
        return;
    };
    let desktop_owns = crate::desktop_owns_keys();
    let bound: Option<String> = ui
        .shortcuts
        .borrow()
        .as_ref()
        .filter(|s| s.desktop_owns_keys())
        .and_then(|s| s.labels().into_iter().find(|(i, _)| *i == hotkey.id))
        .map(|(_, label)| label);
    if desktop_owns {
        // ShareZ's own window would run a hotkey's keys instead of letting them be recorded.
        ui.clear_window_accels();
    } else {
        // Let go of the keys while editing, or pressing one would run it instead of choosing it.
        ui.shortcuts.replace(None);
    }
    let editor = Editor::new(&hotkey, new, bound, desktop_owns);
    for row in [&editor.capture, &editor.after] {
        let editor = Rc::downgrade(&editor);
        row.connect_selected_notify(move |_| {
            if let Some(e) = editor.upgrade() {
                e.sync();
            }
        });
    }
    let weak = Rc::downgrade(&editor);
    editor.keys.connect_activated(glib::clone!(
        #[weak]
        ui,
        move |_| {
            if let Some(editor) = weak.upgrade() {
                change_keys(&ui, &editor);
            }
        }
    ));
    let presses = gtk::EventControllerKey::new();
    presses.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&editor);
    presses.connect_key_pressed(glib::clone!(
        #[weak]
        ui,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, state| {
            let Some(editor) = weak.upgrade().filter(|e| e.recording.get()) else {
                return glib::Propagation::Proceed;
            };
            editor.press(&ui.settings.borrow().hotkeys, key, state);
            glib::Propagation::Stop
        }
    ));
    editor.dialog.add_controller(presses);
    let weak = Rc::downgrade(&editor);
    editor.delete.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            editor.deleted.set(true);
            editor.dialog.close();
        }
    });
    // The dialog's handler keeps the editor alive until it closes.
    let kept = Rc::clone(&editor);
    editor.dialog.connect_closed(glib::clone!(
        #[weak]
        ui,
        move |_| apply(&ui, &kept, &hotkey, new)
    ));
    editor.dialog.present(Some(&ui.window));
    if !new {
        // Not the name, where a stray key press would rename it.
        editor.capture.grab_focus();
    }
}

/// Starts recording new keys, or sends the user to the desktop, which keeps them on Wayland.
fn change_keys(ui: &Rc<Ui>, editor: &Rc<Editor>) {
    if editor.bound.is_none() {
        editor.recording.set(true);
        editor.show_key();
        return;
    }
    let Some(shortcuts) = ui.shortcuts.borrow().clone() else {
        return;
    };
    let dialog = editor.dialog.clone();
    if !shortcuts.desktop_configures() {
        dialog.add_toast(adw::Toast::new(
            "Change it in your desktop's keyboard shortcut settings",
        ));
        return;
    }
    glib::spawn_future_local(async move {
        if let Err(e) = shortcuts.configure().await {
            dialog.add_toast(adw::Toast::new(&format!(
                "Could not open the shortcut settings: {e}"
            )));
        }
    });
}

/// Saves the edited hotkey (or deletes it), and binds the keys again where that's needed.
fn apply(ui: &Rc<Ui>, editor: &Editor, original: &Hotkey, new: bool) {
    editor.sync();
    let draft = editor.draft.borrow().clone();
    let keys_changed = draft.keys != original.keys;
    let deleted = editor.deleted.get();
    {
        let mut settings = ui.settings.borrow_mut();
        let list = &mut settings.hotkeys;
        if deleted {
            list.retain(|h| h.id != draft.id);
        } else if let Some(h) = list.iter_mut().find(|h| h.id == draft.id) {
            *h = draft;
        }
        settings.save();
    }
    // On X11 the keys were let go while editing. On Wayland the desktop only needs asking about
    // a hotkey it hasn't bound yet.
    let rebind =
        !editor.desktop_owns || (!deleted && editor.bound.is_none() && (new || keys_changed));
    if rebind {
        let ui = ui.clone();
        glib::spawn_future_local(async move { crate::bind_shortcuts(&ui).await });
    } else {
        ui.refresh_hotkeys();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_in_use_are_found_however_they_are_written() {
        let hotkeys = Hotkey::defaults();
        let other = taken_by(&hotkeys, "capture-screen", "ctrl+Print").unwrap();
        assert_eq!(other.id, "capture-region");
        // A hotkey's own key isn't taken from itself.
        assert!(taken_by(&hotkeys, "capture-region", "CTRL+Print").is_none());
        assert!(taken_by(&hotkeys, "capture-region", "ALT+F9").is_none());
    }

    #[test]
    fn recordings_say_what_happens_to_them() {
        assert_eq!(
            after_subtitle(Capture::Record, After::Ask),
            "Saved to Videos/ShareZ and copied as a file"
        );
        assert_eq!(
            after_subtitle(Capture::Record, After::Save),
            "Saved to Videos/ShareZ, not copied"
        );
        assert_ne!(
            after_subtitle(Capture::Region, After::Ask),
            after_subtitle(Capture::Region, After::Copy)
        );
    }
}

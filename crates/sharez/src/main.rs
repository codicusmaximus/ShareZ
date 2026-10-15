//! The ShareZ application. It keeps running in the background once started, reachable from its
//! top-bar or tray icon and its global shortcuts.

mod hotkeys;
mod outline;
mod output;
mod overlay;
mod permissions;
mod settings;
mod tray;
mod x11keys;

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use adw::prelude::*;
use gtk::{gio, glib};
use sharez_core::{Frame, Session};
use sharez_record::{Audio, Format, Options, Recorder, Stop};
use sharez_windows::{Elements, Target};

use hotkeys::{Hotkey, Shortcuts, Trigger};
use overlay::Selection;
use settings::Settings;
use tray::Tray;

pub const APP_ID: &str = "io.github.codicusmaximus.ShareZ";
const FPS: u32 = 30;
const RECORDING_NOTIFICATION: &str = "recording";
const USAGE: &str = "Usage: sharez [--background | --capture | --record | --list-windows]

  --background    Start without opening the window (used when starting at login)
  --capture       Capture a region, in the running ShareZ if there is one
  --record        Start recording a region, or stop the recording that is running
  --list-windows  Print the windows ShareZ can see, as JSON";

/// What this run of `sharez` was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Launch {
    Window,
    Background,
    /// Run an app action, then stay in the background.
    Action(&'static str),
}

fn main() -> glib::ExitCode {
    let launch = match std::env::args().nth(1).as_deref() {
        None => Launch::Window,
        // Diagnostics that need no UI.
        Some("--list-windows") => return list_windows(),
        Some("--background") => Launch::Background,
        Some("--capture") => Launch::Action("capture"),
        Some("--record") => Launch::Action("record"),
        Some("--help" | "-h") => {
            println!("{USAGE}");
            return glib::ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("Unknown option {other}\n\n{USAGE}");
            return glib::ExitCode::FAILURE;
        }
    };
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(startup);
    let first = Cell::new(true);
    app.connect_activate(move |app| {
        // A second `sharez` (from the launcher, say) always shows the window.
        if !first.replace(false) || launch == Launch::Window {
            app.activate_action("show", None);
        }
    });
    if launch != Launch::Window {
        if let Err(e) = app.register(None::<&gio::Cancellable>) {
            eprintln!("Could not start: {e}");
            return glib::ExitCode::FAILURE;
        }
        // ShareZ is already running: hand it the job and leave.
        if app.is_remote() {
            if let Launch::Action(action) = launch {
                app.activate_action(action, None);
            }
            if let Some(bus) = app.dbus_connection() {
                let _ = bus.flush_sync(None::<&gio::Cancellable>);
            }
            // Leave without tearing down a GApplication that never ran, which only warns.
            std::process::exit(0);
        }
        if let Launch::Action(action) = launch {
            let app = app.clone();
            glib::idle_add_local_once(move || app.activate_action(action, None));
        }
    }
    let argv0 = std::env::args().next().unwrap_or_else(|| "sharez".into());
    app.run_with_args(&[argv0])
}

/// Prints the windows ShareZ can see as JSON, to check window snapping on a given desktop.
fn list_windows() -> glib::ExitCode {
    let Some(session) = Session::detect() else {
        eprintln!("No display server found.");
        return glib::ExitCode::FAILURE;
    };
    let Some(source) = sharez_windows::Source::for_session(&session) else {
        eprintln!("Listing windows is not supported on this desktop ({session:?}).");
        return glib::ExitCode::FAILURE;
    };
    match glib::MainContext::default().block_on(source.windows()) {
        Ok(windows) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&windows).expect("windows serialize")
            );
            glib::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Could not list windows with {source:?}: {e}");
            glib::ExitCode::FAILURE
        }
    }
}

/// The main window and what the app is doing.
struct Ui {
    app: adw::Application,
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    /// Asks for the desktop's permission to take screenshots, when it hasn't been given.
    banner: adw::Banner,
    page: adw::StatusPage,
    capture: gtk::Button,
    record: gtk::Button,
    discard: gtk::Button,
    desktop_audio: adw::SwitchRow,
    microphone: adw::SwitchRow,
    shortcut_group: adw::PreferencesGroup,
    shortcut_rows: Vec<(Hotkey, adw::ActionRow)>,
    change_shortcuts: gtk::Button,
    settings: RefCell<Settings>,
    /// Set while recording; sending on it ends the recording.
    stop: RefCell<Option<async_channel::Sender<Stop>>>,
    /// Set while the region picker is open, so a second shortcut press doesn't open another.
    busy: Cell<bool>,
    /// Quit once the running recording has been saved.
    quit_after: Cell<bool>,
    tray: RefCell<Option<Tray>>,
    shortcuts: RefCell<Option<Rc<Shortcuts>>>,
}

/// Sets ShareZ up once per run. The window starts hidden; activation shows it.
fn startup(app: &adw::Application) {
    // Closing the window leaves ShareZ running in the background; Quit ends it.
    std::mem::forget(app.hold());
    let ui = build_window(app);
    add_actions(&ui);
    connect(&ui);
    // Apps only publish their accessibility trees (used to snap to elements) while it is on.
    glib::spawn_future_local(async {
        if let Err(e) = sharez_windows::ensure_accessibility().await {
            eprintln!("Could not enable accessibility: {e}");
        }
    });
    glib::spawn_future_local(start_services(ui));
}

/// What to record besides the picture.
fn audio_group(settings: &Settings) -> (adw::PreferencesGroup, adw::SwitchRow, adw::SwitchRow) {
    let desktop_audio = adw::SwitchRow::builder()
        .title("Desktop Audio")
        .subtitle("What you hear")
        .active(settings.desktop_audio)
        .build();
    let microphone = adw::SwitchRow::builder()
        .title("Microphone")
        .active(settings.microphone)
        .build();
    let audio = adw::PreferencesGroup::builder()
        .title("Recording Audio")
        .build();
    audio.add(&desktop_audio);
    audio.add(&microphone);
    (audio, desktop_audio, microphone)
}

/// The shortcuts and their keys, with a button for the desktop's own settings where it has them.
fn shortcuts_group() -> (
    adw::PreferencesGroup,
    gtk::Button,
    Vec<(Hotkey, adw::ActionRow)>,
) {
    let shortcut_group = adw::PreferencesGroup::builder().title("Shortcuts").build();
    let change_shortcuts = gtk::Button::builder()
        .label("Change…")
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    shortcut_group.set_header_suffix(Some(&change_shortcuts));
    let shortcut_rows: Vec<_> = Hotkey::ALL
        .into_iter()
        .map(|h| {
            // Desktops describe keys like `<Control>Print`, which isn't markup.
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .title(h.title())
                .subtitle("Setting up…")
                .build();
            shortcut_group.add(&row);
            (h, row)
        })
        .collect();

    (shortcut_group, change_shortcuts, shortcut_rows)
}

/// Running in the background, and starting at login.
fn background_group() -> (adw::PreferencesGroup, adw::SwitchRow) {
    let background = adw::PreferencesGroup::builder()
        .title("Background")
        .description("Closing this window keeps ShareZ running, ready for its shortcuts.")
        .build();
    let login = adw::SwitchRow::builder()
        .title("Start at Login")
        .active(settings::starts_at_login())
        .build();
    background.add(&login);

    (background, login)
}

fn build_window(app: &adw::Application) -> Rc<Ui> {
    let pill = |label: &str, class: &str, action: &str| {
        gtk::Button::builder()
            .label(label)
            .css_classes(["pill", class])
            .action_name(action)
            .build()
    };
    let capture = pill("Capture Region", "suggested-action", "app.capture");
    let record = pill("Record Region", "suggested-action", "app.record");
    let discard = pill("Discard", "flat", "app.discard-recording");
    discard.set_visible(false);
    let buttons = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::Center)
        .build();
    buttons.append(&capture);
    buttons.append(&record);
    buttons.append(&discard);

    let settings = Settings::load();
    let (audio, desktop_audio, microphone) = audio_group(&settings);

    let (shortcut_group, change_shortcuts, shortcut_rows) = shortcuts_group();
    let (background, login) = background_group();

    let groups = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .build();
    groups.append(&audio);
    groups.append(&shortcut_group);
    groups.append(&background);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(36)
        .build();
    content.append(&buttons);
    content.append(
        &adw::Clamp::builder()
            .maximum_size(400)
            .child(&groups)
            .build(),
    );

    let page = adw::StatusPage::builder()
        .icon_name("applets-screenshooter-symbolic")
        .title("ShareZ")
        .description("Screen capture for Linux. This is an early development build.")
        .child(&content)
        .build();
    let banner = adw::Banner::builder()
        .title("ShareZ needs your permission to take screenshots")
        .button_label("Allow…")
        .revealed(false)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.add_top_bar(&banner);
    view.set_content(Some(&page));
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&view));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("ShareZ")
        .default_width(520)
        .default_height(760)
        .hide_on_close(true)
        .content(&toasts)
        .build();

    login.connect_active_notify(glib::clone!(
        #[weak]
        toasts,
        move |row| {
            if let Err(e) = settings::set_starts_at_login(row.is_active()) {
                toasts.add_toast(adw::Toast::new(&format!(
                    "Could not change starting at login: {e}"
                )));
            }
        }
    ));

    Rc::new(Ui {
        app: app.clone(),
        window,
        toasts,
        banner,
        page,
        capture,
        record,
        discard,
        desktop_audio,
        microphone,
        shortcut_group,
        shortcut_rows,
        change_shortcuts,
        settings: RefCell::new(settings),
        stop: RefCell::new(None),
        busy: Cell::new(false),
        quit_after: Cell::new(false),
        tray: RefCell::new(None),
        shortcuts: RefCell::new(None),
    })
}

/// The app's actions, which the buttons, the tray menu, the shortcuts, notifications and
/// `sharez --capture` all go through. They hold the app's only strong references to `ui`.
fn add_actions(ui: &Rc<Ui>) {
    fn add(ui: &Rc<Ui>, name: &str, run: impl Fn(&Rc<Ui>) + 'static) {
        let a = gio::SimpleAction::new(name, None);
        ui.app.add_action(&a);
        let ui = ui.clone();
        a.connect_activate(move |_, _| run(&ui));
    }
    add(ui, "capture", |ui| {
        let ui = ui.clone();
        glib::spawn_future_local(async move { capture_screen(&ui).await });
    });
    add(ui, "record", |ui| {
        if ui.stop(Stop::Save) {
            return;
        }
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = record_region(&ui).await {
                ui.set_recording(None);
                if ui.quit_after.get() {
                    ui.app.quit();
                    return;
                }
                ui.window.present();
                let dialog = adw::AlertDialog::new(Some("Recording Failed"), Some(&e));
                dialog.add_response("ok", "OK");
                dialog.present(Some(&ui.window));
            }
        });
    });
    add(ui, "stop-recording", |ui| {
        ui.stop(Stop::Save);
    });
    add(ui, "discard-recording", |ui| {
        ui.stop(Stop::Discard);
    });
    add(ui, "show", |ui| ui.window.present());
    add(ui, "quit", |ui| {
        // Finish the file first, or it would be unplayable.
        if ui.stop(Stop::Save) {
            ui.quit_after.set(true);
        } else {
            ui.app.quit();
        }
    });
    ui.app.set_accels_for_action("app.quit", &["<Control>q"]);
    ui.app
        .set_accels_for_action("window.close", &["<Control>w"]);

    // A notification's "open the file".
    let open = gio::SimpleAction::new("open-file", Some(glib::VariantTy::STRING));
    open.connect_activate(|_, path| {
        if let Some(path) = path.and_then(glib::Variant::get::<String>) {
            output::open(std::path::Path::new(&path));
        }
    });
    ui.app.add_action(&open);
}

fn connect(ui: &Rc<Ui>) {
    let save = glib::clone!(
        #[weak]
        ui,
        move || {
            let mut settings = ui.settings.borrow_mut();
            settings.desktop_audio = ui.desktop_audio.is_active();
            settings.microphone = ui.microphone.is_active();
            settings.save();
        }
    );
    let save2 = save.clone();
    ui.desktop_audio.connect_active_notify(move |_| save());
    ui.microphone.connect_active_notify(move |_| save2());

    ui.banner.connect_button_clicked(glib::clone!(
        #[weak]
        ui,
        move |_| {
            glib::spawn_future_local(async move { allow_screenshots(&ui).await });
        }
    ));
    ui.change_shortcuts.connect_clicked(glib::clone!(
        #[weak]
        ui,
        move |_| {
            let Some(shortcuts) = ui.shortcuts.borrow().clone() else {
                return;
            };
            glib::spawn_future_local(async move {
                if let Err(e) = shortcuts.configure().await {
                    ui.toasts.add_toast(adw::Toast::new(&format!(
                        "Could not open the shortcut settings: {e}"
                    )));
                }
            });
        }
    ));
    if Session::detect().is_some_and(|s| s.display == sharez_core::DisplayServer::X11) {
        for (hotkey, row) in &ui.shortcut_rows {
            let hotkey = *hotkey;
            row.set_activatable(true);
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            row.connect_activated(glib::clone!(
                #[weak]
                ui,
                move |_| change_shortcut(&ui, hotkey)
            ));
        }
    }
}

/// The tray icon and the global shortcuts, which need a portal or two.
async fn start_services(ui: Rc<Ui>) {
    // Portals need to know which app is asking. A Flatpak tells them; a plain install must say.
    if let Ok(id) = ashpd::AppID::try_from(APP_ID)
        && let Err(e) = hotkeys::answered(ashpd::register_host_app(id)).await
    {
        eprintln!("Could not register with the desktop portal: {e}");
    }
    if permissions::applies() {
        let state = permissions::screenshots().await;
        if state != permissions::Screenshots::Allowed {
            ui.ask_for_screenshots(state);
        }
    }
    let Some(session) = Session::detect() else {
        return;
    };
    // The tray and the shortcuts start side by side, so neither waits on the other.
    let tray_ui = ui.clone();
    glib::spawn_future_local(async move {
        let (tx, rx) = async_channel::unbounded::<tray::Action>();
        let tray = Tray::start(&session, tx).await;
        if tray.is_none() {
            eprintln!("No tray: open the ShareZ window by starting ShareZ again.");
        }
        *tray_ui.tray.borrow_mut() = tray;
        while let Ok(action) = rx.recv().await {
            tray_ui.app.activate_action(action, None);
        }
    });
    bind_shortcuts(&ui).await;
}

/// (Re)binds the global shortcuts and shows what they are.
async fn bind_shortcuts(ui: &Rc<Ui>) {
    let Some(session) = Session::detect() else {
        return;
    };
    // Let go of the old keys first, so the same key can be grabbed again on X11.
    ui.shortcuts.replace(None);
    let (tx, rx) = async_channel::unbounded::<Hotkey>();
    let keys = ui.settings.borrow().shortcuts.clone();
    let asking = glib::clone!(
        #[weak]
        ui,
        move || {
            for (_, row) in &ui.shortcut_rows {
                row.set_subtitle("Waiting for you to confirm it in the desktop's dialog…");
            }
        }
    );
    match Shortcuts::bind(&session, &keys, tx, asking).await {
        Ok((shortcuts, problems)) => {
            let shortcuts = Rc::new(shortcuts);
            shortcuts.on_change(glib::clone!(
                #[weak]
                ui,
                move || ui.show_shortcuts(None)
            ));
            ui.change_shortcuts
                .set_visible(shortcuts.desktop_configures());
            ui.shortcuts.replace(Some(shortcuts));
            for problem in &problems {
                eprintln!("Shortcut: {problem}");
                ui.toasts.add_toast(adw::Toast::new(problem));
            }
            ui.show_shortcuts(None);
        }
        Err(e) => {
            eprintln!("No global shortcuts: {e}");
            ui.show_shortcuts(Some(&e));
        }
    }
    let app = ui.app.clone();
    glib::spawn_future_local(async move {
        while let Ok(hotkey) = rx.recv().await {
            app.activate_action(hotkey.action(), None);
        }
    });
}

/// Asks for a new key on X11, where ShareZ keeps the shortcuts itself.
fn change_shortcut(ui: &Rc<Ui>, hotkey: Hotkey) {
    // Let go of the keys while asking, or pressing one would run it instead of choosing it.
    ui.shortcuts.replace(None);
    let dialog = adw::AlertDialog::new(
        Some(&format!("Shortcut for {}", hotkey.title())),
        Some("Press the keys to use, with Ctrl, Alt, Shift or Super if you like."),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("default", "Use Default");
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        ui,
        #[weak]
        dialog,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, state| {
            if key == gtk::gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            let Some(trigger) = Trigger::from_press(key, state) else {
                return glib::Propagation::Proceed;
            };
            ui.set_shortcut(hotkey, trigger.to_spec());
            dialog.close();
            glib::Propagation::Stop
        }
    ));
    dialog.add_controller(keys);
    dialog.connect_response(
        Some("default"),
        glib::clone!(
            #[weak]
            ui,
            move |_, _| ui.set_shortcut(hotkey, hotkey.default_trigger().into())
        ),
    );
    dialog.connect_closed(glib::clone!(
        #[weak]
        ui,
        move |_| {
            glib::spawn_future_local(async move { bind_shortcuts(&ui).await });
        }
    ));
    dialog.present(Some(&ui.window));
}

impl Ui {
    /// Asks a running recording to end. False if nothing is recording.
    fn stop(&self, how: Stop) -> bool {
        match self.stop.borrow().as_ref() {
            Some(tx) => {
                let _ = tx.try_send(how);
                true
            }
            None => false,
        }
    }

    fn set_shortcut(&self, hotkey: Hotkey, spec: String) {
        let mut settings = self.settings.borrow_mut();
        if let Some(entry) = settings.shortcuts.iter_mut().find(|(h, _)| *h == hotkey) {
            entry.1 = spec;
        }
        settings.save();
    }

    /// Shows each shortcut's keys, or why there are none.
    fn show_shortcuts(&self, error: Option<&str>) {
        let labels = self
            .shortcuts
            .borrow()
            .as_ref()
            .map(|s| s.labels())
            .unwrap_or_default();
        for (hotkey, row) in &self.shortcut_rows {
            let label = labels.iter().find(|(h, _)| h == hotkey).map(|(_, l)| l);
            row.set_subtitle(match (label, error) {
                (Some(l), _) if !l.is_empty() => l,
                (_, Some(_)) => "Unavailable",
                _ => "Not set",
            });
        }
        self.shortcut_group.set_description(
            error
                .map(|e| format!("This desktop doesn't offer global shortcuts to apps ({e})."))
                .as_deref(),
        );
    }

    /// Shows the window with the permission banner. GNOME only asks the user while ShareZ has
    /// focus, so the question has to start from a click in this window.
    fn ask_for_screenshots(&self, state: permissions::Screenshots) {
        if state == permissions::Screenshots::Denied {
            self.banner.set_title(
                "Screenshots are turned off for ShareZ. See Troubleshooting in the README to \
                 turn them back on.",
            );
            self.banner.set_button_label(None);
        } else {
            self.banner
                .set_title("ShareZ needs your permission to take screenshots");
            self.banner.set_button_label(Some("Allow…"));
        }
        self.banner.set_revealed(true);
        self.window.present();
    }

    /// The window's toast if it is showing, otherwise a notification.
    fn tell_saved(&self, title: &str, path: PathBuf, shown: bool) {
        if shown {
            self.toasts.add_toast(output::saved_toast(path));
        } else {
            self.app
                .send_notification(None, &output::saved_notification(title, &path));
        }
    }

    /// Switches the window and the tray between their normal and recording states.
    fn set_recording(self: &Rc<Self>, stop: Option<async_channel::Sender<Stop>>) {
        let recording = stop.is_some();
        *self.stop.borrow_mut() = stop;
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.set_recording(recording);
        }
        self.capture.set_sensitive(!recording);
        self.desktop_audio.set_sensitive(!recording);
        self.microphone.set_sensitive(!recording);
        self.discard.set_visible(recording);
        self.record.set_label(if recording {
            "Stop Recording"
        } else {
            "Record Region"
        });
        self.record.remove_css_class(if recording {
            "suggested-action"
        } else {
            "destructive-action"
        });
        self.record.add_css_class(if recording {
            "destructive-action"
        } else {
            "suggested-action"
        });
        if !recording {
            self.page.set_title("ShareZ");
            self.app.withdraw_notification(RECORDING_NOTIFICATION);
            return;
        }
        let started = Instant::now();
        self.page.set_title("Recording 0:00");
        glib::timeout_add_seconds_local(
            1,
            glib::clone!(
                #[weak(rename_to = ui)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    if ui.stop.borrow().is_none() {
                        return glib::ControlFlow::Break;
                    }
                    let secs = started.elapsed().as_secs();
                    ui.page
                        .set_title(&format!("Recording {}:{:02}", secs / 60, secs % 60));
                    glib::ControlFlow::Continue
                }
            ),
        );
        let n = gio::Notification::new("Recording");
        n.set_body(Some("ShareZ is recording a region of your screen."));
        n.add_button("Stop", "app.stop-recording");
        self.app.send_notification(Some(RECORDING_NOTIFICATION), &n);
    }
}

/// A region the user picked.
struct Picked {
    selection: Selection,
    /// The region's pixels, already on the clipboard, when they were asked for.
    image: Option<Result<Frame, String>>,
}

/// Hides ShareZ, freezes the screen, lists windows and lets the user pick a region. With `copy`,
/// the region goes on the clipboard as soon as it is picked. `None` if they cancelled.
async fn select_region(ui: &Ui, copy: bool) -> Result<Option<Picked>, String> {
    let session = Session::detect().ok_or("No display server found.")?;
    ui.window.set_visible(false);
    // Give the compositor a moment to remove our window, or the tray menu that started this,
    // before the screen is captured.
    glib::timeout_future(Duration::from_millis(200)).await;
    let (frame, windows, elements) = futures_util::join!(
        sharez_capture::Backend::for_session(&session).freeze(),
        list_windows_for(&session),
        Elements::snapshot(),
    );
    let frame = match frame {
        Ok(frame) => frame,
        Err(sharez_capture::Error::Denied) => {
            ui.ask_for_screenshots(permissions::screenshots().await);
            return Ok(None);
        }
        Err(e) => return Err(format!("Could not capture the screen: {e}")),
    };
    let elements = elements
        .inspect_err(|e| eprintln!("No accessibility data, so no element snapping: {e}"))
        .ok();
    let targets = match &elements {
        Some(e) => e.targets(windows),
        None => windows.into_iter().map(Target::opaque).collect(),
    };
    let snapshot = overlay::Snapshot {
        frame: frame.clone(),
        targets,
        elements,
    };
    let (tx, rx) = async_channel::bounded(1);
    overlay::show(Some(ui.app.upcast_ref()), snapshot, move |selection| {
        let picked = selection.map(|selection| Picked {
            selection,
            image: copy.then(|| output::copy_image(&frame, selection.pixels)),
        });
        let _ = tx.try_send(picked);
    });
    Ok(rx.recv().await.ok().flatten())
}

/// Takes one screenshot from the focused window, so the desktop can ask the user's permission.
async fn allow_screenshots(ui: &Rc<Ui>) {
    let Some(session) = Session::detect() else {
        return;
    };
    match sharez_capture::Backend::for_session(&session)
        .freeze()
        .await
    {
        Ok(_) => {
            ui.banner.set_revealed(false);
            ui.toasts.add_toast(adw::Toast::new(
                "Screenshots allowed. The shortcuts and tray icon are ready.",
            ));
        }
        Err(sharez_capture::Error::Denied) => {
            ui.ask_for_screenshots(permissions::screenshots().await);
        }
        Err(e) => ui.toasts.add_toast(adw::Toast::new(&format!(
            "Could not take a screenshot: {e}"
        ))),
    }
}

async fn capture_screen(ui: &Rc<Ui>) {
    if ui.stop.borrow().is_some() || ui.busy.replace(true) {
        return;
    }
    let shown = ui.window.is_visible();
    let picked = select_region(ui, true).await;
    ui.busy.set(false);
    if shown {
        ui.window.present();
    }
    let saved = match picked {
        Ok(None | Some(Picked { image: None, .. })) => return,
        Ok(Some(Picked {
            image: Some(image), ..
        })) => image.and_then(|image| output::save_image(&image)),
        Err(e) => Err(e),
    };
    match saved {
        Ok(path) => ui.tell_saved("Screenshot copied", path, shown),
        Err(e) => {
            ui.window.present();
            let dialog = adw::AlertDialog::new(Some("Capture Failed"), Some(&e));
            dialog.add_response("ok", "OK");
            dialog.present(Some(&ui.window));
        }
    }
}

async fn record_region(ui: &Rc<Ui>) -> Result<(), String> {
    let session = Session::detect().ok_or("No display server found.")?;
    let format = Format::detect().map_err(|e| e.to_string())?;
    if ui.busy.replace(true) {
        return Ok(());
    }
    let shown = ui.window.is_visible();
    let picked = select_region(ui, false).await;
    ui.busy.set(false);
    let Some(picked) = picked? else {
        if shown {
            ui.window.present();
        }
        return Ok(());
    };
    let selection = picked.selection;
    let options = Options {
        audio: Audio {
            desktop: ui.desktop_audio.is_active(),
            microphone: ui.microphone.is_active(),
        },
        fps: FPS,
        path: output::new_file(glib::UserDirectory::Videos, format.extension())?,
    };
    let recorder = Recorder::start(
        &session,
        selection.logical,
        selection.screen,
        &options,
        format,
    )
    .await
    .map_err(|e| e.to_string())?;
    let (tx, rx) = async_channel::bounded(1);
    ui.set_recording(Some(tx));
    if shown {
        // Out of the way but one click from Stop. The tray and notification can stop it too.
        ui.window.minimize();
        ui.window.set_visible(true);
    }
    let outline = outline::show(&session, selection.logical).await;
    let result = recorder.run(rx).await;
    drop(outline);
    ui.set_recording(None);
    let saved = result.map_err(|e| e.to_string())?;
    if let Some(path) = &saved {
        output::copy_video(path);
    }
    if ui.quit_after.get() {
        ui.app.quit();
        return Ok(());
    }
    if shown {
        ui.window.present();
    }
    match saved {
        Some(path) => ui.tell_saved("Recording copied", path, shown),
        None if shown => ui.toasts.add_toast(adw::Toast::new("Recording discarded")),
        None => {}
    }
    Ok(())
}

async fn list_windows_for(session: &Session) -> Vec<sharez_windows::Window> {
    let Some(source) = sharez_windows::Source::for_session(session) else {
        return Vec::new();
    };
    source
        .windows()
        .await
        .inspect_err(|e| eprintln!("Could not list windows with {source:?}: {e}"))
        .unwrap_or_default()
}

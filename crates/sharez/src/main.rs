//! The ShareZ application. It keeps running in the background once started, reachable from its
//! top-bar or tray icon and its global shortcuts.

mod annotate;
mod countdown;
mod hotkey_settings;
mod hotkeys;
mod outline;
mod output;
mod overlay;
mod permissions;
mod popup;
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
use sharez_core::{DisplayServer, Frame, PixelRect, Session};
use sharez_record::{Audio, Format, Options, Recorder, Stop};
use sharez_windows::{Elements, Target};

use hotkey_settings::{HotkeyList, KeyState};
use hotkeys::{After, Capture, Hotkey, Shortcuts};
use overlay::Selection;
use settings::Settings;
use tray::Tray;

pub const APP_ID: &str = "io.github.codicusmaximus.ShareZ";
const FPS: u32 = 30;
const RECORDING_NOTIFICATION: &str = "recording";
const USAGE: &str = "Usage: sharez [--background | --capture | --capture-screen | --record |
              --hotkey ID | --annotate FILE | --list-windows]

  --background      Start without opening the window (used when starting at login)
  --capture         Capture a region, in the running ShareZ if there is one
  --capture-screen  Capture the whole screen
  --record          Start recording a region, or stop the recording that is running
  --hotkey ID       Do what the hotkey with this id does (the ids are in
                    ~/.config/sharez/settings.ini), for the desktop's own shortcuts
  --annotate FILE   Open an image in the annotator
  --list-windows    Print the windows ShareZ can see, as JSON";

/// What this run of `sharez` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Launch {
    Window,
    Background,
    /// Run an app action, then stay in the background.
    Action(&'static str),
    /// Run a hotkey's job, by id, then stay in the background.
    Hotkey(String),
    /// Open an image file in the annotator.
    Annotate(String),
}

impl Launch {
    fn run(&self, app: &adw::Application) {
        match self {
            Launch::Action(action) => app.activate_action(action, None),
            Launch::Hotkey(id) => app.activate_action("hotkey", Some(&id.to_variant())),
            Launch::Annotate(path) => app.activate_action("annotate", Some(&path.to_variant())),
            Launch::Window | Launch::Background => {}
        }
    }
}

fn main() -> glib::ExitCode {
    let mut args = std::env::args().skip(1);
    let launch = match args.next().as_deref() {
        None => Launch::Window,
        // Diagnostics that need no UI.
        Some("--list-windows") => return list_windows(),
        Some("--background") => Launch::Background,
        Some("--capture") => Launch::Action("capture"),
        Some("--capture-screen") => Launch::Action("capture-screen"),
        Some("--record") => Launch::Action("record"),
        Some("--hotkey") => {
            let Some(id) = args.next() else {
                eprintln!("--hotkey needs a hotkey id\n\n{USAGE}");
                return glib::ExitCode::FAILURE;
            };
            Launch::Hotkey(id)
        }
        Some("--annotate") => {
            let Some(path) = args.next() else {
                eprintln!("--annotate needs an image file\n\n{USAGE}");
                return glib::ExitCode::FAILURE;
            };
            // The running ShareZ may have another working folder.
            let path = std::path::absolute(&path).unwrap_or_else(|_| path.into());
            Launch::Annotate(path.to_string_lossy().into_owned())
        }
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
    let window = launch == Launch::Window;
    app.connect_activate(move |app| {
        // A second `sharez` (from the launcher, say) always shows the window.
        if !first.replace(false) || window {
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
            launch.run(&app);
            if let Some(bus) = app.dbus_connection() {
                let _ = bus.flush_sync(None::<&gio::Cancellable>);
            }
            // Leave without tearing down a GApplication that never ran, which only warns.
            std::process::exit(0);
        }
        let app = app.clone();
        glib::idle_add_local_once(move || launch.run(&app));
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
    capture_screen: gtk::Button,
    record: gtk::Button,
    discard: gtk::Button,
    desktop_audio: adw::SwitchRow,
    microphone: adw::SwitchRow,
    countdown: adw::ExpanderRow,
    countdown_speed: gtk::Scale,
    hotkeys: HotkeyList,
    /// Where binding the hotkeys' keys has got to, and why it failed.
    key_state: Cell<KeyState>,
    key_error: RefCell<Option<String>>,
    /// The hotkey actions given keys in ShareZ's own windows, to take back later.
    window_accels: RefCell<Vec<String>>,
    /// The last region a screenshot was taken of, for Last Region.
    last_region: Cell<Option<LastRegion>>,
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

/// A region picked earlier, and the size of the screen it was picked on.
#[derive(Debug, Clone, Copy)]
struct LastRegion {
    pixels: PixelRect,
    screen: (u32, u32),
}

/// Whether the desktop, not ShareZ, decides the hotkeys' keys (Wayland).
fn desktop_owns_keys() -> bool {
    Session::detect().is_some_and(|s| s.display == DisplayServer::Wayland)
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

/// The recording options: what to record besides the picture, and the countdown.
struct RecordingRows {
    group: adw::PreferencesGroup,
    desktop_audio: adw::SwitchRow,
    microphone: adw::SwitchRow,
    countdown: adw::ExpanderRow,
    countdown_speed: gtk::Scale,
}

fn recording_group(settings: &Settings) -> RecordingRows {
    let desktop_audio = adw::SwitchRow::builder()
        .title("Desktop Audio")
        .subtitle("What you hear")
        .active(settings.desktop_audio)
        .build();
    let microphone = adw::SwitchRow::builder()
        .title("Microphone")
        .active(settings.microphone)
        .build();
    let (countdown, countdown_speed) = countdown_rows(settings);
    let group = adw::PreferencesGroup::builder().title("Recording").build();
    group.add(&desktop_audio);
    group.add(&microphone);
    group.add(&countdown);
    RecordingRows {
        group,
        desktop_audio,
        microphone,
        countdown,
        countdown_speed,
    }
}

/// A switch for the 3-2-1 countdown, with its speed inside.
fn countdown_rows(settings: &Settings) -> (adw::ExpanderRow, gtk::Scale) {
    let (min, max, step) = settings::SPEEDS;
    let speed = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
    speed.set_value(settings.countdown_speed);
    speed.set_digits(2);
    speed.set_draw_value(true);
    speed.set_value_pos(gtk::PositionType::Right);
    speed.set_round_digits(2);
    speed.set_hexpand(true);
    speed.set_valign(gtk::Align::Center);
    speed.set_width_request(200);
    speed.set_format_value_func(|_, v| format!("{v}×"));
    speed.add_mark(1.0, gtk::PositionType::Bottom, None);
    // The scale moves smoothly; keep it on the quarter steps.
    speed.connect_value_changed(|s| {
        let snapped = settings::snap_speed(s.value());
        if (snapped - s.value()).abs() > f64::EPSILON {
            s.set_value(snapped);
        }
    });
    let speed_row = adw::ActionRow::builder()
        .title("Speed")
        .subtitle("1× is one number a second")
        .build();
    speed_row.add_suffix(&speed);
    let countdown = adw::ExpanderRow::builder()
        .title("Countdown")
        .subtitle("Show 3, 2, 1 over the region before recording starts")
        .show_enable_switch(true)
        .enable_expansion(settings.countdown)
        .build();
    countdown.add_row(&speed_row);
    (countdown, speed)
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

/// The big buttons: Capture Region, Capture Screen, Record Region (Stop Recording while it
/// records), and Discard while recording.
struct Buttons {
    row: gtk::Box,
    capture: gtk::Button,
    capture_screen: gtk::Button,
    record: gtk::Button,
    discard: gtk::Button,
}

fn buttons() -> Buttons {
    let pill = |label: &str, class: &str, action: &str| {
        gtk::Button::builder()
            .label(label)
            .css_classes(["pill", class])
            .action_name(action)
            .build()
    };
    let capture = pill("Capture Region", "suggested-action", "app.capture");
    let capture_screen = pill("Capture Screen", "suggested-action", "app.capture-screen");
    let record = pill("Record Region", "suggested-action", "app.record");
    let discard = pill("Discard", "flat", "app.discard-recording");
    discard.set_visible(false);
    let buttons = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::Center)
        .build();
    buttons.append(&capture);
    buttons.append(&capture_screen);
    buttons.append(&record);
    buttons.append(&discard);
    Buttons {
        row: buttons,
        capture,
        capture_screen,
        record,
        discard,
    }
}

fn build_window(app: &adw::Application) -> Rc<Ui> {
    let buttons = buttons();
    let settings = Settings::load();
    let recording = recording_group(&settings);

    let hotkeys = HotkeyList::new();
    let (background, login) = background_group();

    let groups = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .build();
    groups.append(&hotkeys.group);
    groups.append(&recording.group);
    groups.append(&background);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(36)
        .build();
    content.append(&buttons.row);
    content.append(
        &adw::Clamp::builder()
            .maximum_size(480)
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
        .default_width(600)
        .default_height(820)
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
        capture: buttons.capture,
        capture_screen: buttons.capture_screen,
        record: buttons.record,
        discard: buttons.discard,
        desktop_audio: recording.desktop_audio,
        microphone: recording.microphone,
        countdown: recording.countdown,
        countdown_speed: recording.countdown_speed,
        hotkeys,
        key_state: Cell::new(KeyState::SettingUp),
        key_error: RefCell::new(None),
        window_accels: RefCell::new(Vec::new()),
        last_region: Cell::new(None),
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
        glib::spawn_future_local(async move {
            screenshot(&ui, Capture::Region, After::SaveAndCopy).await;
        });
    });
    add(ui, "capture-screen", |ui| {
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            screenshot(&ui, Capture::Screen, After::SaveAndCopy).await;
        });
    });
    // A hotkey, by id: from its keys, or `sharez --hotkey`.
    let hotkey = gio::SimpleAction::new("hotkey", Some(glib::VariantTy::STRING));
    ui.app.add_action(&hotkey);
    let weak = Rc::downgrade(ui);
    hotkey.connect_activate(move |_, id| {
        if let (Some(ui), Some(id)) = (weak.upgrade(), id.and_then(glib::Variant::str)) {
            run_hotkey(&ui, id);
        }
    });
    // An image file in the annotator: `sharez --annotate`.
    let annotate_file = gio::SimpleAction::new("annotate", Some(glib::VariantTy::STRING));
    ui.app.add_action(&annotate_file);
    let weak = Rc::downgrade(ui);
    annotate_file.connect_activate(move |_, path| {
        let (Some(ui), Some(path)) = (weak.upgrade(), path.and_then(glib::Variant::str)) else {
            return;
        };
        match output::load_image(std::path::Path::new(path)) {
            Ok(image) => annotate(&ui, image, ui.window.is_visible()),
            Err(e) => ui.fail("Could Not Open the Image", &e),
        }
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

    // A notification's "open the file" and "show it in its folder".
    for (name, reveal) in [
        ("open-file", output::Reveal::Open),
        ("show-file", output::Reveal::InFolder),
    ] {
        let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        action.connect_activate(move |_, path| {
            if let Some(path) = path.and_then(glib::Variant::get::<String>) {
                reveal.run(std::path::Path::new(&path));
            }
        });
        ui.app.add_action(&action);
    }
}

fn connect(ui: &Rc<Ui>) {
    let save = glib::clone!(
        #[weak]
        ui,
        move || {
            let mut settings = ui.settings.borrow_mut();
            settings.desktop_audio = ui.desktop_audio.is_active();
            settings.microphone = ui.microphone.is_active();
            settings.countdown = ui.countdown.enables_expansion();
            settings.countdown_speed = settings::snap_speed(ui.countdown_speed.value());
            settings.save();
        }
    );
    let save = Rc::new(save);
    let s = Rc::clone(&save);
    ui.desktop_audio.connect_active_notify(move |_| s());
    let s = Rc::clone(&save);
    ui.microphone.connect_active_notify(move |_| s());
    let s = Rc::clone(&save);
    ui.countdown.connect_enable_expansion_notify(move |_| s());
    ui.countdown_speed.connect_value_changed(move |_| save());

    ui.banner.connect_button_clicked(glib::clone!(
        #[weak]
        ui,
        move |_| {
            glib::spawn_future_local(async move { allow_screenshots(&ui).await });
        }
    ));
    ui.hotkeys.add.connect_clicked(glib::clone!(
        #[weak]
        ui,
        move |_| {
            let hotkey = Hotkey::fresh(&ui.settings.borrow().hotkeys);
            let id = hotkey.id.clone();
            {
                let mut settings = ui.settings.borrow_mut();
                settings.hotkeys.push(hotkey);
                settings.save();
            }
            ui.refresh_hotkeys();
            hotkey_settings::edit(&ui, &id, true);
        }
    ));
    ui.hotkeys.desktop.connect_clicked(glib::clone!(
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
    ui.refresh_hotkeys();
}

/// Does what the hotkey `id` is set up to do.
fn run_hotkey(ui: &Rc<Ui>, id: &str) {
    let Some(hotkey) = ui
        .settings
        .borrow()
        .hotkeys
        .iter()
        .find(|h| h.id == id)
        .cloned()
    else {
        eprintln!("No hotkey {id}");
        return;
    };
    if hotkey.capture == Capture::Record {
        ui.app.activate_action("record", None);
        return;
    }
    let ui = ui.clone();
    glib::spawn_future_local(async move { screenshot(&ui, hotkey.capture, hotkey.after).await });
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
    ui.key_state.set(KeyState::SettingUp);
    ui.refresh_hotkeys();
    let (tx, rx) = async_channel::unbounded::<String>();
    let hotkeys = ui.settings.borrow().hotkeys.clone();
    let asking = glib::clone!(
        #[weak]
        ui,
        move || {
            ui.key_state.set(KeyState::Confirming);
            ui.refresh_hotkeys();
        }
    );
    match Shortcuts::bind(&session, &hotkeys, tx, asking).await {
        Ok((shortcuts, problems)) => {
            let shortcuts = Rc::new(shortcuts);
            shortcuts.on_change(glib::clone!(
                #[weak]
                ui,
                move || ui.refresh_hotkeys()
            ));
            ui.shortcuts.replace(Some(shortcuts));
            for problem in &problems {
                eprintln!("Shortcut: {problem}");
                ui.toasts.add_toast(adw::Toast::new(problem));
            }
            ui.key_state.set(KeyState::Ready);
            ui.key_error.replace(None);
        }
        Err(e) => {
            eprintln!("No global shortcuts: {e}");
            ui.key_state.set(KeyState::Unavailable);
            ui.key_error.replace(Some(e));
        }
    }
    ui.refresh_hotkeys();
    let app = ui.app.clone();
    glib::spawn_future_local(async move {
        while let Ok(id) = rx.recv().await {
            app.activate_action("hotkey", Some(&id.to_variant()));
        }
    });
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

    /// Takes the hotkeys' keys away from ShareZ's own windows.
    fn clear_window_accels(&self) {
        for action in self.window_accels.borrow_mut().drain(..) {
            self.app.set_accels_for_action(&action, &[]);
        }
    }

    /// Lists the hotkeys with their keys, or why there are none, and gives ShareZ's own windows
    /// the keys too.
    fn refresh_hotkeys(self: &Rc<Self>) {
        let shortcuts = self.shortcuts.borrow().clone();
        let (labels, accels) = shortcuts
            .as_ref()
            .map(|s| (s.labels(), s.window_accels()))
            .unwrap_or_default();
        self.clear_window_accels();
        // The desktop still remembers hotkeys that were deleted; only the current ones count.
        let current = |id: &str| self.settings.borrow().hotkeys.iter().any(|h| h.id == id);
        for (id, accel) in accels.into_iter().filter(|(id, _)| current(id)) {
            let action = format!("app.hotkey::{id}");
            self.app.set_accels_for_action(&action, &[accel.as_str()]);
            self.window_accels.borrow_mut().push(action);
        }
        self.hotkeys
            .desktop
            .set_visible(shortcuts.is_some_and(|s| s.desktop_configures()));
        self.hotkeys.group.set_description(
            self.key_error
                .borrow()
                .as_ref()
                .map(|e| format!("This desktop doesn't offer global shortcuts to apps ({e})."))
                .as_deref(),
        );
        let weak = Rc::downgrade(self);
        let edit: Rc<dyn Fn(String)> = Rc::new(move |id| {
            if let Some(ui) = weak.upgrade() {
                hotkey_settings::edit(&ui, &id, false);
            }
        });
        self.hotkeys.show(
            &self.settings.borrow().hotkeys,
            &labels,
            self.key_state.get(),
            &edit,
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
    fn tell_saved(
        &self,
        title: &str,
        path: PathBuf,
        copied: bool,
        shown: bool,
        reveal: output::Reveal,
    ) {
        if shown {
            self.toasts
                .add_toast(output::saved_toast(path, copied, reveal));
        } else {
            self.app
                .send_notification(None, &output::saved_notification(title, &path, reveal));
        }
    }

    /// News of a screenshot that was copied and not saved.
    fn tell_copied(&self, shown: bool) {
        if shown {
            self.toasts.add_toast(adw::Toast::new("Screenshot copied"));
        } else {
            self.app
                .send_notification(None, &output::copied_notification());
        }
    }

    /// Shows the window with what went wrong.
    fn fail(&self, title: &str, e: &str) {
        self.window.present();
        let dialog = adw::AlertDialog::new(Some(title), Some(e));
        dialog.add_response("ok", "OK");
        dialog.present(Some(&self.window));
    }

    /// Switches the window and the tray between their normal and recording states.
    fn set_recording(self: &Rc<Self>, stop: Option<async_channel::Sender<Stop>>) {
        let recording = stop.is_some();
        *self.stop.borrow_mut() = stop;
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.set_recording(recording);
        }
        self.capture.set_visible(!recording);
        self.capture_screen.set_visible(!recording);
        self.desktop_audio.set_sensitive(!recording);
        self.microphone.set_sensitive(!recording);
        self.countdown.set_sensitive(!recording);
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
    /// The region's pixels, when they were asked for, already on the clipboard if that was too.
    image: Option<Result<Frame, String>>,
    /// The size of the frozen screen it was picked on.
    screen: (u32, u32),
}

/// What a region is being picked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Record,
    Screenshot { copy: bool },
}

/// Hides ShareZ's window and gives the compositor a moment to remove it, or the tray menu that
/// started this, before the screen is captured.
async fn get_out_of_the_way(ui: &Ui) {
    ui.window.set_visible(false);
    glib::timeout_future(Duration::from_millis(200)).await;
}

/// A frozen frame, or `None` if the desktop refused (the window then asks for permission).
async fn frozen(
    ui: &Ui,
    frame: Result<Frame, sharez_capture::Error>,
) -> Result<Option<Frame>, String> {
    match frame {
        Ok(frame) => Ok(Some(frame)),
        Err(sharez_capture::Error::Denied) => {
            ui.ask_for_screenshots(permissions::screenshots().await);
            Ok(None)
        }
        Err(e) => Err(format!("Could not capture the screen: {e}")),
    }
}

/// Hides ShareZ and freezes the whole screen.
async fn freeze_screen(ui: &Ui) -> Result<Option<Frame>, String> {
    let session = Session::detect().ok_or("No display server found.")?;
    get_out_of_the_way(ui).await;
    let frame = sharez_capture::Backend::for_session(&session)
        .freeze()
        .await;
    frozen(ui, frame).await
}

/// Hides ShareZ, freezes the screen, lists windows and lets the user pick a region. For a
/// screenshot that is copied, the region goes on the clipboard as soon as it is picked. `None` if
/// they cancelled.
async fn select_region(ui: &Ui, purpose: Purpose) -> Result<Option<Picked>, String> {
    let session = Session::detect().ok_or("No display server found.")?;
    get_out_of_the_way(ui).await;
    let (frame, windows, elements) = futures_util::join!(
        sharez_capture::Backend::for_session(&session).freeze(),
        list_windows_for(&session),
        Elements::snapshot(),
    );
    let Some(frame) = frozen(ui, frame).await? else {
        return Ok(None);
    };
    let elements = elements
        .inspect_err(|e| eprintln!("No accessibility data, so no element snapping: {e}"))
        .ok();
    let targets = match &elements {
        Some(e) => e.targets(windows),
        None => windows.into_iter().map(Target::opaque).collect(),
    };
    let screen = (frame.width, frame.height);
    let snapshot = overlay::Snapshot {
        frame: frame.clone(),
        targets,
        elements,
        // A recording films the screen, so it shows whatever covers the window anyway.
        window_pixels: purpose != Purpose::Record,
    };
    let (tx, rx) = async_channel::bounded(1);
    overlay::show(Some(ui.app.upcast_ref()), snapshot, move |selection| {
        let picked = selection.map(|mut selection| Picked {
            image: match purpose {
                Purpose::Record => None,
                Purpose::Screenshot { copy } => {
                    let image = match selection.window_image.take() {
                        Some(image) => Ok(image),
                        None => output::crop(&frame, selection.pixels),
                    };
                    Some(image.map(|i| if copy { output::copy_frame(i) } else { i }))
                }
            },
            selection,
            screen,
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

/// Puts `image` on the clipboard behind a quick flash, which gives ShareZ the focus that Wayland
/// asks for. For screenshots taken without the overlay.
async fn flash_and_copy(ui: &Ui, image: Frame) -> Frame {
    let (tx, rx) = async_channel::bounded(1);
    overlay::flash(Some(ui.app.upcast_ref()), move || {
        let _ = tx.try_send(output::copy_frame(image));
    });
    rx.recv().await.expect("the flash always finishes")
}

/// Takes a screenshot the way `capture` says, copied already when `copy`. `None` if the user
/// cancelled.
async fn take(ui: &Ui, capture: Capture, copy: bool) -> Result<Option<Frame>, String> {
    let image = match capture {
        Capture::Region | Capture::Record => {
            let Some(picked) = select_region(ui, Purpose::Screenshot { copy }).await? else {
                return Ok(None);
            };
            ui.last_region.set(Some(LastRegion {
                pixels: picked.selection.pixels,
                screen: picked.screen,
            }));
            return picked.image.transpose();
        }
        Capture::Screen => freeze_screen(ui).await?,
        Capture::LastRegion => {
            let Some(last) = ui.last_region.get() else {
                // Nothing picked yet: pick one now, which later presses repeat.
                return Box::pin(take(ui, Capture::Region, copy)).await;
            };
            let Some(frame) = freeze_screen(ui).await? else {
                return Ok(None);
            };
            if (frame.width, frame.height) != last.screen {
                // The screen changed size since; the old region may not fit.
                ui.last_region.set(None);
                return Box::pin(take(ui, Capture::Region, copy)).await;
            }
            Some(output::crop(&frame, last.pixels)?)
        }
    };
    match image {
        Some(image) if copy => Ok(Some(flash_and_copy(ui, image).await)),
        image => Ok(image),
    }
}

/// Takes a screenshot the way `capture` says and does what `after` says with it.
async fn screenshot(ui: &Rc<Ui>, capture: Capture, after: After) {
    if ui.stop.borrow().is_some() || ui.busy.replace(true) {
        return;
    }
    let shown = ui.window.is_visible();
    let taken = take(ui, capture, after.copies()).await;
    ui.busy.set(false);
    if shown {
        ui.window.present();
    }
    let image = match taken {
        Ok(Some(image)) => image,
        Ok(None) => return,
        Err(e) => return ui.fail("Capture Failed", &e),
    };
    match after {
        After::Ask => ask(ui, image, shown),
        After::Annotate => annotate(ui, image, shown),
        After::Copy => ui.tell_copied(shown),
        After::Save | After::SaveAndCopy => {
            keep(ui, &image, None, after.copies(), shown);
        }
    }
}

/// Saves a screenshot, under Pictures/ShareZ or at `path`, and says so. `None` if it couldn't.
fn keep(
    ui: &Ui,
    image: &Frame,
    path: Option<PathBuf>,
    copied: bool,
    shown: bool,
) -> Option<PathBuf> {
    let saved = match path {
        Some(path) => output::save_image_to(path, image),
        None => output::save_image(image),
    };
    match saved {
        Ok(path) => {
            let title = if copied {
                "Screenshot copied"
            } else {
                "Screenshot saved"
            };
            ui.tell_saved(title, path.clone(), copied, shown, output::Reveal::Open);
            Some(path)
        }
        Err(e) => {
            ui.fail("Could Not Save the Screenshot", &e);
            None
        }
    }
}

/// Shows the popup that asks what to do with `image`, and does it.
fn ask(ui: &Rc<Ui>, image: Frame, shown: bool) {
    let texture = output::texture(&image);
    let weak = Rc::downgrade(ui);
    popup::ask(&ui.app, &texture.clone(), move |choice| {
        let Some(ui) = weak.upgrade() else { return };
        // Copy first, while the popup still has focus.
        let copied = choice.copies();
        if copied {
            output::copy_texture(&texture);
        }
        match choice {
            popup::Choice::Copy => ui.tell_copied(shown),
            popup::Choice::Save | popup::Choice::SaveAndCopy => {
                keep(&ui, &image, None, copied, shown);
            }
            popup::Choice::SaveAs(path) => {
                keep(&ui, &image, Some(path), false, shown);
            }
            popup::Choice::SaveAndOpen => {
                if let Some(path) = keep(&ui, &image, None, false, shown) {
                    output::open(&path);
                }
            }
            popup::Choice::Annotate => annotate(&ui, image, shown),
            popup::Choice::Discard => {}
        }
    });
}

/// Opens the annotator on `image`. Finishing brings the popup back with the result, as ShareX
/// carries on with its after-capture tasks; cancelling drops the screenshot.
fn annotate(ui: &Rc<Ui>, image: Frame, shown: bool) {
    let weak = Rc::downgrade(ui);
    annotate::open(&ui.app, image, move |outcome| {
        let Some(ui) = weak.upgrade() else { return };
        match outcome {
            annotate::Outcome::Apply(image) | annotate::Outcome::Continue(image) => {
                ask(&ui, image, shown);
            }
            annotate::Outcome::Cancel => {}
        }
    });
}

async fn record_region(ui: &Rc<Ui>) -> Result<(), String> {
    let session = Session::detect().ok_or("No display server found.")?;
    let format = Format::detect().map_err(|e| e.to_string())?;
    if ui.busy.replace(true) {
        return Ok(());
    }
    let shown = ui.window.is_visible();
    let picked = select_region(ui, Purpose::Record).await;
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
    let recorder = Recorder::prepare(
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
    let countdown = {
        let settings = ui.settings.borrow();
        settings.countdown.then_some(settings.countdown_speed)
    };
    let stopped = match countdown {
        Some(speed) => countdown::run(&session, selection.logical, speed, &rx).await,
        None => false,
    };
    let began = if stopped {
        Ok(false)
    } else {
        recorder.begin().map(|()| true)
    };
    if !matches!(began, Ok(true)) {
        // Stopped during the countdown, or it couldn't start: nothing was recorded.
        recorder.cancel().await;
        drop(outline);
        ui.set_recording(None);
        if shown {
            ui.window.present();
            if began.is_ok() {
                ui.toasts.add_toast(adw::Toast::new("Recording cancelled"));
            }
        }
        return began.map(|_| ()).map_err(|e| e.to_string());
    }
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
        Some(path) => ui.tell_saved(
            "Recording copied",
            path,
            true,
            shown,
            output::Reveal::InFolder,
        ),
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

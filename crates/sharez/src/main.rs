//! The ShareZ application.

mod outline;
mod output;
mod overlay;
mod settings;

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use adw::prelude::*;
use gtk::{gio, glib};
use sharez_core::{Frame, Session};
use sharez_record::{Audio, Format, Options, Recorder, Stop};
use sharez_windows::{Elements, Target};

use overlay::Selection;
use settings::Settings;

const APP_ID: &str = "io.github.codicusmaximus.ShareZ";
const FPS: u32 = 30;
const RECORDING_NOTIFICATION: &str = "recording";

fn main() -> glib::ExitCode {
    // Diagnostics that need no UI. Handled before GApplication, which rejects unknown options.
    if std::env::args().nth(1).as_deref() == Some("--list-windows") {
        return list_windows();
    }
    let app = adw::Application::builder().application_id(APP_ID).build();
    // Apps only publish their accessibility trees (used to snap to elements) while it is on.
    glib::spawn_future_local(async {
        if let Err(e) = sharez_windows::ensure_accessibility().await {
            eprintln!("Could not enable accessibility: {e}");
        }
    });
    app.connect_activate(build_window);
    app.run()
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

/// The main window and what it needs to know while a recording runs.
struct Ui {
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    page: adw::StatusPage,
    capture: gtk::Button,
    record: gtk::Button,
    discard: gtk::Button,
    desktop_audio: adw::SwitchRow,
    microphone: adw::SwitchRow,
    /// Set while recording; sending on it ends the recording.
    stop: RefCell<Option<async_channel::Sender<Stop>>>,
}

fn build_window(app: &adw::Application) {
    // Opening ShareZ again (from the launcher, say) shows the window that is already there.
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }
    let pill = |label: &str, class: &str| {
        gtk::Button::builder()
            .label(label)
            .css_classes(["pill", class])
            .build()
    };
    let capture = pill("Capture Screen", "suggested-action");
    let record = pill("Record Region", "suggested-action");
    let discard = pill("Discard", "flat");
    discard.set_visible(false);
    let buttons = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::Center)
        .build();
    buttons.append(&capture);
    buttons.append(&record);
    buttons.append(&discard);

    let saved = Settings::load();
    let desktop_audio = adw::SwitchRow::builder()
        .title("Desktop Audio")
        .subtitle("What you hear")
        .active(saved.desktop_audio)
        .build();
    let microphone = adw::SwitchRow::builder()
        .title("Microphone")
        .active(saved.microphone)
        .build();
    let audio = adw::PreferencesGroup::builder()
        .title("Recording Audio")
        .build();
    audio.add(&desktop_audio);
    audio.add(&microphone);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(36)
        .build();
    content.append(&buttons);
    content.append(
        &adw::Clamp::builder()
            .maximum_size(360)
            .child(&audio)
            .build(),
    );

    let page = adw::StatusPage::builder()
        .icon_name("applets-screenshooter-symbolic")
        .title("ShareZ")
        .description("Screen capture for Linux. This is an early development build.")
        .child(&content)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&view));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("ShareZ")
        .default_width(520)
        .default_height(640)
        .content(&toasts)
        .build();

    let ui = Rc::new(Ui {
        window,
        toasts,
        page,
        capture,
        record,
        discard,
        desktop_audio,
        microphone,
        stop: RefCell::new(None),
    });
    connect(app, &ui);
    ui.window.present();
}

fn connect(app: &adw::Application, ui: &Rc<Ui>) {
    let save_settings = glib::clone!(
        #[weak]
        ui,
        move || {
            Settings {
                desktop_audio: ui.desktop_audio.is_active(),
                microphone: ui.microphone.is_active(),
            }
            .save();
        }
    );
    let save = save_settings.clone();
    ui.desktop_audio.connect_active_notify(move |_| save());
    ui.microphone
        .connect_active_notify(move |_| save_settings());

    ui.capture.connect_clicked(glib::clone!(
        #[weak]
        ui,
        move |_| {
            glib::spawn_future_local(async move {
                if let Err(e) = capture_screen(&ui).await {
                    ui.window.present();
                    ui.toasts.add_toast(adw::Toast::new(&e));
                }
            });
        }
    ));
    ui.record.connect_clicked(glib::clone!(
        #[weak]
        ui,
        move |_| {
            if ui.stop(Stop::Save) {
                return;
            }
            glib::spawn_future_local(async move {
                if let Err(e) = record_region(&ui).await {
                    ui.set_recording(None);
                    ui.window.present();
                    let dialog = adw::AlertDialog::new(Some("Recording Failed"), Some(&e));
                    dialog.add_response("ok", "OK");
                    dialog.present(Some(&ui.window));
                }
            });
        }
    ));
    ui.discard.connect_clicked(glib::clone!(
        #[weak]
        ui,
        move |_| {
            ui.stop(Stop::Discard);
        }
    ));

    // The notification's Stop button.
    let stop = gio::SimpleAction::new("stop-recording", None);
    stop.connect_activate(glib::clone!(
        #[weak]
        ui,
        move |_, _| {
            ui.stop(Stop::Save);
        }
    ));
    app.add_action(&stop);

    // Closing the window would end the app mid-file and leave it unplayable. This handler also
    // holds the one strong reference that keeps `ui` alive as long as the window.
    ui.window.connect_close_request(glib::clone!(
        #[strong]
        ui,
        move |_| {
            if ui.stop.borrow().is_some() {
                ui.toasts
                    .add_toast(adw::Toast::new("Stop or discard the recording first."));
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    ));
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

    /// Switches the window between its normal and recording states.
    fn set_recording(self: &Rc<Self>, stop: Option<async_channel::Sender<Stop>>) {
        let recording = stop.is_some();
        *self.stop.borrow_mut() = stop;
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
        let app = self.window.application();
        if !recording {
            self.page.set_title("ShareZ");
            if let Some(app) = app {
                app.withdraw_notification(RECORDING_NOTIFICATION);
            }
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
        if let Some(app) = app {
            let n = gio::Notification::new("Recording");
            n.set_body(Some("ShareZ is recording a region of your screen."));
            n.add_button("Stop", "app.stop-recording");
            app.send_notification(Some(RECORDING_NOTIFICATION), &n);
        }
    }
}

/// Hides ShareZ, freezes the screen, lists windows and lets the user pick a region. `None` if
/// they cancelled.
async fn select_region(ui: &Ui) -> Result<Option<(Frame, Selection)>, String> {
    let session = Session::detect().ok_or("No display server found.")?;
    ui.window.set_visible(false);
    // Give the compositor a moment to remove our window before the screen is captured.
    glib::timeout_future(Duration::from_millis(200)).await;
    let (frame, windows, elements) = futures_util::join!(
        sharez_capture::Backend::for_session(&session).freeze(),
        list_windows_for(&session),
        Elements::snapshot(),
    );
    let frame = frame.map_err(|e| format!("Could not capture the screen: {e}"))?;
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
    overlay::show(
        ui.window.application().as_ref(),
        snapshot,
        move |selection| {
            let _ = tx.try_send(selection);
        },
    );
    Ok(rx.recv().await.ok().flatten().map(|s| (frame, s)))
}

async fn capture_screen(ui: &Ui) -> Result<(), String> {
    let selected = select_region(ui).await?;
    ui.window.present();
    if let Some((frame, selection)) = selected {
        let toast = output::deliver(&ui.window, &frame, selection.pixels)?;
        ui.toasts.add_toast(toast);
    }
    Ok(())
}

async fn record_region(ui: &Rc<Ui>) -> Result<(), String> {
    let session = Session::detect().ok_or("No display server found.")?;
    let format = Format::detect().map_err(|e| e.to_string())?;
    let Some((_, selection)) = select_region(ui).await? else {
        ui.window.present();
        return Ok(());
    };
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
    // Out of the way but one click from Stop. The notification has a Stop button too.
    ui.window.minimize();
    ui.window.set_visible(true);
    let outline = outline::show(&session, selection.logical).await;
    let result = recorder.run(rx).await;
    drop(outline);
    ui.set_recording(None);
    ui.window.present();
    match result.map_err(|e| e.to_string())? {
        Some(path) => ui.toasts.add_toast(output::deliver_video(&ui.window, path)),
        None => ui.toasts.add_toast(adw::Toast::new("Recording discarded")),
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

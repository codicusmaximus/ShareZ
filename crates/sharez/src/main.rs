//! The ShareZ application.

mod output;
mod overlay;

use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use sharez_core::Session;
use sharez_windows::{Elements, Target};

const APP_ID: &str = "io.github.codicusmaximus.ShareZ";

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

fn build_window(app: &adw::Application) {
    let toasts = adw::ToastOverlay::new();
    let capture = gtk::Button::builder()
        .label("Capture Screen")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .build();
    let page = adw::StatusPage::builder()
        .icon_name("applets-screenshooter-symbolic")
        .title("ShareZ")
        .description("Screen capture for Linux. This is an early development build.")
        .child(&capture)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    toasts.set_child(Some(&view));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("ShareZ")
        .default_width(480)
        .default_height(420)
        .content(&toasts)
        .build();

    capture.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        toasts,
        move |_| {
            glib::spawn_future_local(async move {
                if let Err(e) = capture_screen(&window, &toasts).await {
                    window.present();
                    toasts.add_toast(adw::Toast::new(&e));
                }
            });
        }
    ));
    window.present();
}

/// Hides ShareZ, freezes the screen, lists windows and opens the region overlay.
async fn capture_screen(
    window: &adw::ApplicationWindow,
    toasts: &adw::ToastOverlay,
) -> Result<(), String> {
    let session = Session::detect().ok_or("No display server found.")?;
    window.set_visible(false);
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
        frame,
        targets,
        elements,
    };
    let frame_copy = snapshot.frame.clone();
    overlay::show(
        window.application().as_ref(),
        snapshot,
        glib::clone!(
            #[weak]
            window,
            #[weak]
            toasts,
            move |region| {
                window.present();
                if let Some(region) = region {
                    match output::deliver(&window, &frame_copy, region) {
                        Ok(toast) => toasts.add_toast(toast),
                        Err(e) => toasts.add_toast(adw::Toast::new(&e)),
                    }
                }
            }
        ),
    );
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

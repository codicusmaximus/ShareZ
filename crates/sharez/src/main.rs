//! The ShareZ application.

mod overlay;

use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use sharez_core::Session;

const APP_ID: &str = "io.github.codicusmaximus.ShareZ";

fn main() -> glib::ExitCode {
    // Diagnostics that need no UI. Handled before GApplication, which rejects unknown options.
    if std::env::args().nth(1).as_deref() == Some("--list-windows") {
        return list_windows();
    }
    let app = adw::Application::builder().application_id(APP_ID).build();
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
                if let Err(e) = capture_screen(&window).await {
                    window.present();
                    toasts.add_toast(adw::Toast::new(&e));
                }
            });
        }
    ));
    window.present();
}

/// Hides ShareZ, freezes the screen and opens the overlay on the frozen frame.
async fn capture_screen(window: &adw::ApplicationWindow) -> Result<(), String> {
    let session = Session::detect().ok_or("No display server found.")?;
    window.set_visible(false);
    // Give the compositor a moment to remove our window before the screen is captured.
    glib::timeout_future(Duration::from_millis(200)).await;
    let frame = sharez_capture::Backend::for_session(&session)
        .freeze()
        .await
        .map_err(|e| format!("Could not capture the screen: {e}"))?;
    let overlay = overlay::show(window.application().as_ref(), &frame);
    overlay.connect_destroy(glib::clone!(
        #[weak]
        window,
        move |_| window.present()
    ));
    Ok(())
}

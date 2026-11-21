//! The icon that keeps ShareZ reachable while it runs in the background.
//!
//! GNOME has no tray of its own, so the ShareZ Helper extension puts an icon in the top bar while
//! ShareZ runs and calls the app's actions from its menu. Everywhere else (KDE, most X11 desktops,
//! GNOME with the AppIndicator extension) ShareZ publishes a StatusNotifierItem.

use gtk::{gio, glib, prelude::*};
use ksni::TrayMethods;
use sharez_core::{Desktop, Session};

/// An app action the menu runs, by name.
pub type Action = &'static str;

pub enum Tray {
    Sni(ksni::Handle<Sni>),
    /// The helper extension's top-bar icon.
    Gnome,
}

impl Tray {
    /// Shows the icon. Its menu sends action names to `actions`. `None` if there is nowhere to
    /// show it. `app` tells the user when GNOME won't load the helper extension.
    pub async fn start(
        session: &Session,
        app: &impl IsA<gio::Application>,
        actions: async_channel::Sender<Action>,
    ) -> Option<Self> {
        if session.desktop == Desktop::Gnome {
            match sharez_windows::enable_helper().await {
                // Give the shell a moment to load it.
                Ok(true) => glib::timeout_future(std::time::Duration::from_millis(500)).await,
                Ok(false) => {}
                Err(sharez_windows::Error::HelperOutOfDate(path)) => {
                    eprintln!("GNOME won't load the ShareZ Helper in {path}: it is out of date.");
                    app.send_notification(Some("helper-out-of-date"), &out_of_date(&path));
                }
                Err(e) => eprintln!("Could not check the ShareZ Helper extension: {e}"),
            }
        }
        if session.desktop == Desktop::Gnome
            && sharez_windows::helper_features()
                .await
                .iter()
                .any(|f| f == "indicator")
        {
            return Some(Self::Gnome);
        }
        let sni = Sni {
            actions,
            recording: false,
        };
        // Started at login, ShareZ can come up before the panel; the icon appears once it does.
        match sni
            // Flatpak doesn't let apps own the name the spec asks for; watchers cope without.
            .disable_dbus_name(ashpd::is_sandboxed())
            .assume_sni_available(true)
            .spawn()
            .await
        {
            Ok(handle) => Some(Self::Sni(handle)),
            Err(e) => {
                eprintln!("No tray icon: {e}");
                None
            }
        }
    }

    /// Switches the icon and menu between recording and not.
    pub fn set_recording(&self, recording: bool) {
        match self {
            Self::Sni(handle) => {
                let handle = handle.clone();
                glib::spawn_future_local(async move {
                    handle.update(|t| t.recording = recording).await;
                });
            }
            Self::Gnome => {
                glib::spawn_future_local(async move {
                    if let Err(e) = sharez_windows::helper_set_recording(recording).await {
                        eprintln!("Could not update the top-bar icon: {e}");
                    }
                });
            }
        }
    }
}

/// Without the helper, the recording outline, copying recordings and window snapping stop
/// working, which is easy to miss; the usual cause is an old copy in the home folder.
fn out_of_date(path: &str) -> gio::Notification {
    let n = gio::Notification::new("ShareZ Helper is out of date");
    let home = glib::home_dir();
    let body = if !path.is_empty() && std::path::Path::new(path).starts_with(&home) {
        format!(
            "GNOME is loading an old copy from {path} instead of the one ShareZ installed. \
             Delete that folder, then log out and back in."
        )
    } else {
        "It doesn't support this GNOME version yet, so the recording outline, copying \
         recordings and window snapping are off. Updating ShareZ may fix this."
            .to_owned()
    };
    n.set_body(Some(&body));
    n
}

pub struct Sni {
    actions: async_channel::Sender<Action>,
    recording: bool,
}

impl Sni {
    fn item(label: &str, action: Action) -> ksni::MenuItem<Self> {
        ksni::menu::StandardItem {
            label: label.into(),
            activate: Box::new(move |t: &mut Self| {
                let _ = t.actions.send_blocking(action);
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for Sni {
    fn id(&self) -> String {
        crate::APP_ID.into()
    }

    fn title(&self) -> String {
        "ShareZ".into()
    }

    fn icon_name(&self) -> String {
        if self.recording {
            "media-record"
        } else {
            "applets-screenshooter"
        }
        .into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: if self.recording {
                "ShareZ is recording"
            } else {
                "ShareZ"
            }
            .into(),
            ..Default::default()
        }
    }

    fn status(&self) -> ksni::Status {
        if self.recording {
            ksni::Status::NeedsAttention
        } else {
            ksni::Status::Active
        }
    }

    fn attention_icon_name(&self) -> String {
        "media-record".into()
    }

    /// A left click opens the window, or stops a recording.
    fn activate(&mut self, _x: i32, _y: i32) {
        let action = if self.recording {
            "stop-recording"
        } else {
            "show"
        };
        let _ = self.actions.send_blocking(action);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let mut items = if self.recording {
            vec![
                Self::item("Stop Recording", "stop-recording"),
                Self::item("Discard Recording", "discard-recording"),
            ]
        } else {
            vec![
                Self::item("Capture Region", "capture"),
                Self::item("Capture Screen", "capture-screen"),
                Self::item("Record Region", "record"),
            ]
        };
        items.extend([
            ksni::MenuItem::Separator,
            Self::item("Open ShareZ", "show"),
            Self::item("Quit", "quit"),
        ]);
        items
    }
}

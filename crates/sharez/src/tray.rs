//! The icon that keeps ShareZ reachable while it runs in the background.
//!
//! GNOME has no tray of its own, so the ShareZ Helper extension puts an icon in the top bar while
//! ShareZ runs and calls the app's actions from its menu. Everywhere else (KDE, most X11 desktops,
//! GNOME with the AppIndicator extension) ShareZ publishes a StatusNotifierItem.

use gtk::glib;
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
    /// show it.
    pub async fn start(session: &Session, actions: async_channel::Sender<Action>) -> Option<Self> {
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

//! The full-screen overlay that shows the frozen frame. Region selection, window snapping and
//! the magnifier are built on top of this in the region screenshot milestone.

use gtk::{gdk, glib, prelude::*};
use sharez_core::Frame;

pub fn show(app: Option<&gtk::Application>, frame: &Frame) -> gtk::Window {
    let texture = gdk::MemoryTexture::new(
        frame.width as i32,
        frame.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(&frame.data[..]),
        frame.stride(),
    );
    let picture = gtk::Picture::builder()
        .paintable(&texture)
        .content_fit(gtk::ContentFit::Fill)
        .cursor(&gdk::Cursor::from_name("crosshair", None).expect("crosshair cursor"))
        .build();
    let overlay = gtk::Window::builder()
        .title("ShareZ Capture")
        .decorated(false)
        .child(&picture)
        .build();
    overlay.set_application(app);

    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        overlay,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            if key == gdk::Key::Escape {
                overlay.destroy();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    ));
    overlay.add_controller(keys);
    overlay.fullscreen();
    overlay.present();
    overlay
}

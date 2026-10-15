//! The outline around the region while it is being recorded.
//!
//! A borderless, transparent window just larger than the region, with a frame drawn in the margin
//! so the outline itself never shows up in the recording. Clicks pass through it. Wayland apps
//! can't position their own windows, so the desktop places it (see `sharez_windows::Source::place`).

use std::time::Duration;

use gtk::{cairo, glib, prelude::*};
use sharez_core::{Rect, Session};

/// How the outline window is found by the code that places it.
const TITLE: &str = "ShareZ Recording Area";
const LINE: f64 = 2.0;
/// Space between the region and the line, so antialiasing never bleeds into the recording.
const GAP: f64 = 2.0;
const PLACE_ATTEMPTS: u32 = 10;
/// The drawn part of the window: a 1 px dark edge and the line.
const RING: u16 = 3;

/// The outline; it disappears when dropped.
#[derive(Debug)]
pub struct Outline(gtk::Window);

impl Drop for Outline {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

/// Shows the outline around `region` (logical). Returns `None` where windows can't be placed,
/// rather than leaving a frame wherever the desktop happens to put it.
pub async fn show(session: &Session, region: Rect) -> Option<Outline> {
    let source = sharez_windows::Source::for_session(session)?;
    let margin = LINE + GAP;
    let outer = region.inset(-margin, -margin, -margin, -margin);

    let area = gtk::DrawingArea::new();
    area.set_draw_func(|_, cr, w, h| {
        cr.set_line_width(LINE);
        // Red on a thin dark edge, visible on light and dark content alike.
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.5);
        cr.rectangle(0.5, 0.5, f64::from(w) - 1.0, f64::from(h) - 1.0);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        cr.set_line_width(LINE);
        cr.set_source_rgb(0.878, 0.106, 0.141);
        let inset = LINE / 2.0 + 0.5;
        cr.rectangle(
            inset,
            inset,
            f64::from(w) - 2.0 * inset,
            f64::from(h) - 2.0 * inset,
        );
        let _ = cr.stroke();
    });
    let window = gtk::Window::builder()
        .title(TITLE)
        .decorated(false)
        .resizable(false)
        .can_focus(false)
        .default_width(outer.width.ceil() as i32)
        .default_height(outer.height.ceil() as i32)
        .css_classes(["sharez-outline"])
        .child(&area)
        .build();
    transparent_background();
    window.connect_map(click_through);
    window.present();
    let outline = Outline(window);

    for _ in 0..PLACE_ATTEMPTS {
        glib::timeout_future(Duration::from_millis(100)).await;
        click_through(&outline.0);
        match source.place(TITLE, outer, Some(RING)).await {
            Ok(true) => return Some(outline),
            Ok(false) => {}
            Err(e) => {
                eprintln!("Can't place the recording outline with {source:?}: {e}");
                return None;
            }
        }
    }
    eprintln!("The recording outline window never appeared to {source:?}.");
    None
}

/// An empty input region: every click goes to the window underneath.
fn click_through(window: &gtk::Window) {
    if let Some(surface) = window.surface() {
        surface.set_input_region(Some(&cairo::Region::create()));
    }
}

fn transparent_background() {
    thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if DONE.replace(true) {
        return;
    }
    let css = gtk::CssProvider::new();
    css.load_from_string("window.sharez-outline { background: none; box-shadow: none; }");
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

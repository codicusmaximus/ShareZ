//! The 3, 2, 1 shown in the middle of the region before a recording starts. The recording begins
//! on the frame where the 1 is gone; there is no 0.

use std::{cell::Cell, rc::Rc, time::Instant};

use futures_util::future::{Either, select};
use gtk::{cairo, glib, prelude::*};
use sharez_core::{Rect, Session};
use sharez_record::Stop;

use crate::outline::{overlay_window, place};

/// How the countdown window is found by the code that places it.
const TITLE: &str = "ShareZ Countdown";
const FROM: u32 = 3;
/// The badge's side, as a share of the region's shorter side, and its limits in logical pixels.
const SIZE: f64 = 0.45;
const MIN_SIDE: f64 = 64.0;
const MAX_SIDE: f64 = 280.0;
/// The last part of each number fades out, so the 1 is fully gone before recording starts.
const FADE: f64 = 0.25;
/// Time for the compositor to take the window off screen before the first frame is recorded.
const SETTLE_MS: u64 = 60;

/// The badge: a square centred in `region`.
fn badge(region: Rect) -> Rect {
    let side = (region.width.min(region.height) * SIZE).clamp(MIN_SIDE, MAX_SIDE);
    Rect::new(
        region.x + (region.width - side) / 2.0,
        region.y + (region.height - side) / 2.0,
        side,
        side,
    )
}

/// Which number shows `elapsed` seconds in at `speed` numbers a second, and how visible it is.
/// `None` once the countdown is over.
fn frame(elapsed: f64, speed: f64) -> Option<(u32, f64)> {
    let beats = elapsed * speed;
    if !(0.0..f64::from(FROM)).contains(&beats) {
        return None;
    }
    let shown = FROM - beats.floor() as u32;
    let into = beats.fract();
    let opacity = if into < 1.0 - FADE {
        1.0
    } else {
        (1.0 - into) / FADE
    };
    Some((shown, opacity))
}

fn draw(cr: &cairo::Context, side: f64, number: u32, opacity: f64) {
    let r = side * 0.18;
    cr.new_sub_path();
    cr.arc(side - r, r, r, -90_f64.to_radians(), 0.0);
    cr.arc(side - r, side - r, r, 0.0, 90_f64.to_radians());
    cr.arc(r, side - r, r, 90_f64.to_radians(), 180_f64.to_radians());
    cr.arc(r, r, r, 180_f64.to_radians(), 270_f64.to_radians());
    cr.close_path();
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.55 * opacity);
    let _ = cr.fill();

    let text = number.to_string();
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(side * 0.72);
    if let Ok(ext) = cr.text_extents(&text) {
        cr.move_to(
            (side - ext.width()) / 2.0 - ext.x_bearing(),
            (side - ext.height()) / 2.0 - ext.y_bearing(),
        );
        cr.set_source_rgba(1.0, 1.0, 1.0, opacity);
        let _ = cr.show_text(&text);
    }
}

/// Counts down over `region` (logical) at `speed` numbers a second. True if the user stopped the
/// recording during it, which ends the countdown there.
pub async fn run(
    session: &Session,
    region: Rect,
    speed: f64,
    stop: &async_channel::Receiver<Stop>,
) -> bool {
    let rect = badge(region);
    let started: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    let area = gtk::DrawingArea::new();
    area.set_draw_func(glib::clone!(
        #[strong]
        started,
        move |_, cr, w, _| {
            // Nothing until it is in place.
            let Some(started) = started.get() else {
                return;
            };
            if let Some((number, opacity)) = frame(started.elapsed().as_secs_f64(), speed) {
                draw(cr, f64::from(w), number, opacity);
            }
        }
    ));
    area.add_tick_callback(|area, _| {
        area.queue_draw();
        glib::ControlFlow::Continue
    });
    let window = overlay_window(TITLE, rect, &area);

    let waited = async {
        // Where the desktop can't place it, it shows wherever the desktop puts it.
        if let Some(source) = sharez_windows::Source::for_session(session) {
            place(source, &window, TITLE, rect, None).await;
        }
        started.set(Some(Instant::now()));
        glib::timeout_future(std::time::Duration::from_secs_f64(f64::from(FROM) / speed)).await;
    };
    let stopped = matches!(
        select(Box::pin(waited), Box::pin(stop.recv())).await,
        Either::Right(_)
    );
    window.destroy();
    glib::timeout_future(std::time::Duration::from_millis(SETTLE_MS)).await;
    stopped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_three_two_one_then_ends() {
        assert_eq!(frame(0.0, 1.0), Some((3, 1.0)));
        assert_eq!(frame(1.2, 1.0).map(|f| f.0), Some(2));
        assert_eq!(frame(2.5, 1.0), Some((1, 1.0)));
        assert_eq!(frame(3.0, 1.0), None);
        // At 2× each number lasts half a second.
        assert_eq!(frame(1.4, 2.0).map(|f| f.0), Some(1));
        assert_eq!(frame(1.5, 2.0), None);
    }

    #[test]
    fn each_number_fades_out_completely() {
        let (_, opacity) = frame(2.875, 1.0).unwrap();
        assert!((opacity - 0.5).abs() < 1e-9);
        assert!(frame(2.9999, 1.0).unwrap().1 < 0.001);
    }

    #[test]
    fn badge_is_centred_and_bounded() {
        let b = badge(Rect::new(100.0, 100.0, 400.0, 200.0));
        assert_eq!(b, Rect::new(255.0, 155.0, 90.0, 90.0));
        let big = badge(Rect::new(0.0, 0.0, 4000.0, 3000.0));
        assert_eq!(big, Rect::new(1860.0, 1360.0, MAX_SIDE, MAX_SIDE));
        let small = badge(Rect::new(0.0, 0.0, 50.0, 50.0));
        assert_eq!(small, Rect::new(-7.0, -7.0, MIN_SIDE, MIN_SIDE));
    }
}

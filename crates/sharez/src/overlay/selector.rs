//! What the mouse and keyboard do in the region overlay, independent of drawing.
//!
//! Modelled on ShareX's region capture in quick-capture mode: hover highlights the window or
//! element under the cursor, a click takes it, a drag draws a region, and releasing the mouse
//! captures straight away. Everything here is in buffer pixels (the frozen frame's own pixels),
//! so a selection always covers whole pixels and matches what the magnifier shows.

use sharez_core::PixelRect;

/// Movement before a press on a hovered window turns into drawing a region (ShareX: 4 px).
const DRAG_THRESHOLD: f64 = 4.0;
/// Regions smaller than this fall back to the hovered window (ShareX: 5×5).
const MIN_SIZE: u32 = 5;
pub const MAGNIFIER_PIXELS_DEFAULT: u32 = 15;
const MAGNIFIER_PIXELS_RANGE: (u32, u32) = (3, 35);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Nothing,
    Redraw,
    /// A region the user drew, or the whole screen.
    Capture(PixelRect),
    /// The hovered window or element, taken with a click or Enter.
    Pick(PixelRect),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
    Space,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    anchor: (f64, f64),
    current: (f64, f64),
    /// The hovered rectangle when the button went down; a click without dragging takes it.
    hover: Option<PixelRect>,
    dragging: bool,
    /// Ctrl is held: the pointer moves the whole region instead of resizing it.
    moving: bool,
}

#[derive(Debug)]
pub struct Selector {
    width: u32,
    height: u32,
    pointer: Option<(f64, f64)>,
    hover: Option<PixelRect>,
    drag: Option<Drag>,
    ignore_right_release: bool,
    magnifier_pixels: u32,
}

impl Selector {
    /// `width` and `height` are the frozen frame's size.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pointer: None,
            hover: None,
            drag: None,
            ignore_right_release: false,
            magnifier_pixels: MAGNIFIER_PIXELS_DEFAULT,
        }
    }

    pub fn bounds(&self) -> PixelRect {
        PixelRect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }

    /// The buffer pixel under the pointer.
    pub fn pointer_pixel(&self) -> Option<(u32, u32)> {
        self.pointer.map(|(x, y)| self.pixel(x, y))
    }

    pub fn magnifier_pixels(&self) -> u32 {
        self.magnifier_pixels
    }

    fn pixel(&self, x: f64, y: f64) -> (u32, u32) {
        let clamp = |v: f64, max: u32| (v.floor().max(0.0) as u32).min(max.saturating_sub(1));
        (clamp(x, self.width), clamp(y, self.height))
    }

    /// The region being drawn, covering both the anchor pixel and the pixel under the pointer.
    pub fn selection(&self) -> Option<PixelRect> {
        let d = self.drag.filter(|d| d.dragging)?;
        let (ax, ay) = self.pixel(d.anchor.0, d.anchor.1);
        let (cx, cy) = self.pixel(d.current.0, d.current.1);
        Some(PixelRect {
            x: ax.min(cx),
            y: ay.min(cy),
            width: ax.abs_diff(cx) + 1,
            height: ay.abs_diff(cy) + 1,
        })
    }

    /// The hovered window or element, hidden while a region is being drawn.
    pub fn hover(&self) -> Option<PixelRect> {
        if self.selection().is_some() {
            None
        } else {
            self.hover
        }
    }

    pub fn set_hover(&mut self, hover: Option<PixelRect>) -> Outcome {
        if self.hover == hover {
            return Outcome::Nothing;
        }
        self.hover = hover;
        Outcome::Redraw
    }

    /// Whether the region is being moved rather than resized.
    pub fn moving(&self) -> bool {
        self.drag.is_some_and(|d| d.dragging && d.moving)
    }

    /// Ctrl went down or up. Only changes what is drawn; the next motion moves or resizes.
    pub fn set_moving(&mut self, moving: bool) -> Outcome {
        let before = self.moving();
        if let Some(d) = &mut self.drag {
            d.moving = moving;
        }
        if self.moving() == before {
            Outcome::Nothing
        } else {
            Outcome::Redraw
        }
    }

    /// `square` (Shift) keeps the region 1:1; `move_frame` (Ctrl) moves the whole region with the
    /// pointer instead, as in ShareX. Letting go of Ctrl resizes again from the moved anchor.
    pub fn motion(&mut self, x: f64, y: f64, square: bool, move_frame: bool) -> Outcome {
        let previous = self.pointer.replace((x, y)).unwrap_or((x, y));
        let (width, height) = (f64::from(self.width), f64::from(self.height));
        let Some(d) = &mut self.drag else {
            return Outcome::Redraw;
        };
        d.moving = move_frame;
        if d.dragging && move_frame {
            // Clamped to the frame, so the region keeps its size at the edges.
            let shift = |delta: f64, a: f64, c: f64, max: f64| {
                let (lo, hi) = (a.min(c), a.max(c));
                delta.clamp((-lo).min(0.0), (max - 1.0 - hi).max(0.0))
            };
            let dx = shift(x - previous.0, d.anchor.0, d.current.0, width);
            let dy = shift(y - previous.1, d.anchor.1, d.current.1, height);
            d.anchor = (d.anchor.0 + dx, d.anchor.1 + dy);
            d.current = (d.current.0 + dx, d.current.1 + dy);
            return Outcome::Redraw;
        }
        if !d.dragging && (x - d.anchor.0).hypot(y - d.anchor.1) >= DRAG_THRESHOLD {
            d.dragging = true;
        }
        d.current = (x, y);
        if square {
            let (dx, dy) = (x - d.anchor.0, y - d.anchor.1);
            let side = dx.abs().max(dy.abs());
            // Clamped to the frame, so the square never runs off the edge.
            let max_x = if dx < 0.0 {
                d.anchor.0
            } else {
                width - d.anchor.0 - 1.0
            };
            let max_y = if dy < 0.0 {
                d.anchor.1
            } else {
                height - d.anchor.1 - 1.0
            };
            let side = side.min(max_x).min(max_y);
            d.current = (
                d.anchor.0 + side.copysign(dx),
                d.anchor.1 + side.copysign(dy),
            );
        }
        Outcome::Redraw
    }

    pub fn press(&mut self, x: f64, y: f64) -> Outcome {
        self.pointer = Some((x, y));
        self.drag = Some(Drag {
            anchor: (x, y),
            current: (x, y),
            hover: self.hover,
            dragging: false,
            moving: false,
        });
        Outcome::Redraw
    }

    pub fn release(&mut self, x: f64, y: f64) -> Outcome {
        self.pointer = Some((x, y));
        let Some(drag) = self.drag else {
            return Outcome::Nothing;
        };
        let selection = self.selection();
        self.drag = None;
        match selection {
            Some(r) if r.width >= MIN_SIZE && r.height >= MIN_SIZE => Outcome::Capture(r),
            _ => match drag.hover {
                Some(h) => Outcome::Pick(h),
                None => Outcome::Redraw,
            },
        }
    }

    /// A right press while drawing abandons the region (and its release is swallowed).
    pub fn right_press(&mut self) -> Outcome {
        if self.drag.take().is_some() {
            self.ignore_right_release = true;
            Outcome::Redraw
        } else {
            Outcome::Nothing
        }
    }

    /// Otherwise right-click cancels the capture.
    pub fn right_release(&mut self) -> Outcome {
        if std::mem::take(&mut self.ignore_right_release) {
            Outcome::Nothing
        } else {
            Outcome::Cancel
        }
    }

    pub fn key(&mut self, key: Key) -> Outcome {
        match key {
            Key::Escape => Outcome::Cancel,
            Key::Enter => self.hover.map_or(Outcome::Nothing, Outcome::Pick),
            Key::Space => Outcome::Capture(self.bounds()),
        }
    }

    /// Scrolling up zooms in (fewer pixels), as in ShareX. The count stays odd so one pixel sits
    /// in the middle.
    pub fn scroll(&mut self, up: bool) -> Outcome {
        let (min, max) = MAGNIFIER_PIXELS_RANGE;
        let n = if up {
            self.magnifier_pixels.saturating_sub(2)
        } else {
            self.magnifier_pixels + 2
        };
        self.magnifier_pixels = n.clamp(min, max);
        Outcome::Redraw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> PixelRect {
        PixelRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn drag_captures_on_release_including_both_end_pixels() {
        let mut s = Selector::new(100, 100);
        s.press(10.2, 10.7);
        s.motion(30.9, 20.1, false, false);
        assert_eq!(s.selection(), Some(rect(10, 10, 21, 11)));
        assert_eq!(
            s.release(30.9, 20.1),
            Outcome::Capture(rect(10, 10, 21, 11))
        );
        assert_eq!(s.selection(), None);
    }

    #[test]
    fn dragging_backwards_works() {
        let mut s = Selector::new(100, 100);
        s.press(50.0, 50.0);
        s.motion(40.0, 45.0, false, false);
        assert_eq!(s.release(40.0, 45.0), Outcome::Capture(rect(40, 45, 11, 6)));
    }

    #[test]
    fn click_takes_the_hovered_window() {
        let mut s = Selector::new(100, 100);
        s.set_hover(Some(rect(5, 5, 50, 40)));
        s.press(20.0, 20.0);
        s.motion(22.0, 21.0, false, false); // Under the drag threshold.
        assert_eq!(s.hover(), Some(rect(5, 5, 50, 40)));
        assert_eq!(s.release(22.0, 21.0), Outcome::Pick(rect(5, 5, 50, 40)));
    }

    #[test]
    fn tiny_drag_falls_back_to_hover_or_nothing() {
        let mut s = Selector::new(100, 100);
        s.press(20.0, 20.0);
        s.motion(23.0, 23.0, false, false); // Past the threshold, but only 4×4 pixels.
        assert_eq!(s.release(23.0, 23.0), Outcome::Redraw);
        s.set_hover(Some(rect(0, 0, 10, 10)));
        s.press(20.0, 20.0);
        s.motion(23.0, 23.0, false, false);
        assert_eq!(s.hover(), None, "hover hides while drawing");
        assert_eq!(s.release(23.0, 23.0), Outcome::Pick(rect(0, 0, 10, 10)));
    }

    #[test]
    fn shift_makes_a_square_clamped_to_the_frame() {
        let mut s = Selector::new(100, 100);
        s.press(10.0, 10.0);
        s.motion(40.0, 20.0, true, false);
        assert_eq!(s.selection(), Some(rect(10, 10, 31, 31)));
        s.motion(200.0, 20.0, true, false);
        assert_eq!(s.selection(), Some(rect(10, 10, 90, 90)));
    }

    #[test]
    fn ctrl_moves_the_whole_region_and_clamps_it_to_the_frame() {
        let mut s = Selector::new(100, 100);
        s.press(10.0, 10.0);
        s.motion(30.0, 20.0, false, false);
        assert!(!s.moving());
        s.motion(40.0, 25.0, false, true);
        assert!(s.moving());
        assert_eq!(
            s.selection(),
            Some(rect(20, 15, 21, 11)),
            "same size, moved"
        );
        s.motion(500.0, 500.0, false, true);
        assert_eq!(
            s.selection(),
            Some(rect(79, 89, 21, 11)),
            "stops at the edge"
        );
        s.motion(-500.0, -500.0, false, true);
        assert_eq!(s.selection(), Some(rect(0, 0, 21, 11)));
        // Letting go of Ctrl resizes from the moved anchor.
        s.motion(5.0, 5.0, false, false);
        assert!(!s.moving());
        assert_eq!(s.selection(), Some(rect(0, 0, 6, 6)));
        assert_eq!(s.release(5.0, 5.0), Outcome::Capture(rect(0, 0, 6, 6)));
    }

    #[test]
    fn ctrl_before_the_drag_starts_does_nothing() {
        let mut s = Selector::new(100, 100);
        s.press(10.0, 10.0);
        s.motion(12.0, 10.0, false, true);
        assert!(!s.moving());
        assert_eq!(s.selection(), None);
        s.motion(30.0, 10.0, false, true);
        assert_eq!(s.selection(), Some(rect(10, 10, 21, 1)));
        assert_eq!(s.set_moving(true), Outcome::Nothing);
        s.motion(30.0, 30.0, false, true);
        assert_eq!(s.selection(), Some(rect(10, 30, 21, 1)));
        assert_eq!(s.set_moving(false), Outcome::Redraw);
        assert!(!s.moving());
    }

    #[test]
    fn right_click_cancels_the_drag_first_then_the_capture() {
        let mut s = Selector::new(100, 100);
        s.press(10.0, 10.0);
        s.motion(50.0, 50.0, false, false);
        assert_eq!(s.right_press(), Outcome::Redraw);
        assert_eq!(s.selection(), None);
        assert_eq!(s.right_release(), Outcome::Nothing);
        assert_eq!(s.right_press(), Outcome::Nothing);
        assert_eq!(s.right_release(), Outcome::Cancel);
    }

    #[test]
    fn keys_and_scroll() {
        let mut s = Selector::new(80, 60);
        assert_eq!(s.key(Key::Enter), Outcome::Nothing);
        s.set_hover(Some(rect(1, 2, 3, 4)));
        assert_eq!(s.key(Key::Enter), Outcome::Pick(rect(1, 2, 3, 4)));
        assert_eq!(s.key(Key::Space), Outcome::Capture(rect(0, 0, 80, 60)));
        assert_eq!(s.key(Key::Escape), Outcome::Cancel);
        s.scroll(true);
        assert_eq!(s.magnifier_pixels(), 13);
        for _ in 0..20 {
            s.scroll(true);
        }
        assert_eq!(s.magnifier_pixels(), 3);
    }

    #[test]
    fn pointer_is_clamped_to_the_frame() {
        let mut s = Selector::new(80, 60);
        s.motion(-3.0, 100.0, false, false);
        assert_eq!(s.pointer_pixel(), Some((0, 59)));
    }
}

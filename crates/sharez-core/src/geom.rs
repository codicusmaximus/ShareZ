//! Logical and buffer coordinates.
//!
//! Compositors and window managers describe the desktop in *logical* pixels, which are scaled:
//! at 135 % a 2256×1504 panel is 1671×1114 logical pixels. Captured images and screencast
//! buffers are in *buffer* pixels. Everything ShareZ reasons about (windows, elements, the
//! selection) is kept in logical pixels and converted with a [`Mapping`] only when touching
//! pixels, because mixing the two is exactly how a crop ends up the wrong size in the wrong place.

use serde::{Deserialize, Serialize};

/// A rectangle in logical pixels. Fractional values are normal under fractional scaling.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[must_use]
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// Whether the point lies inside, with the right and bottom edges excluded.
    #[must_use]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    #[must_use]
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = Rect::new(
            x,
            y,
            self.right().min(other.right()) - x,
            self.bottom().min(other.bottom()) - y,
        );
        (!r.is_empty()).then_some(r)
    }

    /// Shrinks each edge by the given amount, e.g. to remove a client-side shadow.
    #[must_use]
    pub fn inset(&self, left: f64, top: f64, right: f64, bottom: f64) -> Rect {
        Rect::new(
            self.x + left,
            self.y + top,
            self.width - left - right,
            self.height - top - bottom,
        )
    }

    /// Offsets a rectangle given relative to this one (e.g. an element's window-relative
    /// extents) into the same space as `self`.
    #[must_use]
    pub fn translate_child(&self, child: &Rect) -> Rect {
        Rect::new(
            self.x + child.x,
            self.y + child.y,
            child.width,
            child.height,
        )
    }
}

/// A rectangle in buffer pixels, always inside the buffer it was mapped for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    /// Trims the right and bottom edges to even sizes, as 4:2:0 video encoders require.
    /// Returns `None` if nothing is left.
    #[must_use]
    pub fn even(self) -> Option<PixelRect> {
        let r = PixelRect {
            width: self.width & !1,
            height: self.height & !1,
            ..self
        };
        (r.width > 0 && r.height > 0).then_some(r)
    }
}

/// How one buffer (a frozen frame or a screencast stream) maps onto logical space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mapping {
    /// The logical area the buffer shows.
    pub logical: Rect,
    pub buffer_width: u32,
    pub buffer_height: u32,
}

impl Mapping {
    #[must_use]
    pub fn scale_x(&self) -> f64 {
        f64::from(self.buffer_width) / self.logical.width
    }

    #[must_use]
    pub fn scale_y(&self) -> f64 {
        f64::from(self.buffer_height) / self.logical.height
    }

    /// Converts a logical rectangle to the buffer pixels that cover it, clipped to the buffer.
    /// Edges are rounded outwards so a selection never loses a partly covered pixel.
    #[must_use]
    pub fn to_pixels(&self, r: &Rect) -> Option<PixelRect> {
        let r = r.intersection(&self.logical)?;
        let (sx, sy) = (self.scale_x(), self.scale_y());
        let left = ((r.x - self.logical.x) * sx).floor().max(0.0);
        let top = ((r.y - self.logical.y) * sy).floor().max(0.0);
        let right = ((r.right() - self.logical.x) * sx)
            .ceil()
            .min(f64::from(self.buffer_width));
        let bottom = ((r.bottom() - self.logical.y) * sy)
            .ceil()
            .min(f64::from(self.buffer_height));
        let p = PixelRect {
            x: left as u32,
            y: top as u32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        };
        (p.width > 0 && p.height > 0).then_some(p)
    }

    /// Converts a buffer position to logical coordinates.
    #[must_use]
    pub fn to_logical(&self, px: f64, py: f64) -> (f64, f64) {
        (
            self.logical.x + px / self.scale_x(),
            self.logical.y + py / self.scale_y(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The KDE spike run: a 2256×1504 stream for a 1671×1114 logical monitor at 135 %.
    fn kde_laptop() -> Mapping {
        Mapping {
            logical: Rect::new(0.0, 0.0, 1671.0, 1114.0),
            buffer_width: 2256,
            buffer_height: 1504,
        }
    }

    #[test]
    fn fractional_scale_crop_lands_on_the_same_area() {
        // The spike cropped these numbers straight off the buffer and got a 1419×946 region
        // offset towards the top left. Mapped properly it is the centre quarter.
        let p = kde_laptop()
            .to_pixels(&Rect::new(418.0, 279.0, 834.0, 556.0))
            .unwrap();
        assert_eq!(
            p,
            PixelRect {
                x: 564,
                y: 376,
                width: 1127,
                height: 752
            }
        );
        assert_eq!(p.even(), Some(PixelRect { width: 1126, ..p }));
    }

    #[test]
    fn crops_are_clipped_to_the_buffer() {
        let p = kde_laptop()
            .to_pixels(&Rect::new(-50.0, 1000.0, 200.0, 500.0))
            .unwrap();
        assert_eq!((p.x, p.y), (0, 1350));
        assert_eq!((p.width, p.height), (203, 154));
        assert!(
            kde_laptop()
                .to_pixels(&Rect::new(2000.0, 0.0, 10.0, 10.0))
                .is_none()
        );
    }

    #[test]
    fn monitors_offset_in_logical_space() {
        let right_monitor = Mapping {
            logical: Rect::new(1920.0, 0.0, 1280.0, 720.0),
            buffer_width: 2560,
            buffer_height: 1440,
        };
        let p = right_monitor
            .to_pixels(&Rect::new(2020.0, 10.0, 100.0, 50.0))
            .unwrap();
        assert_eq!(
            p,
            PixelRect {
                x: 200,
                y: 20,
                width: 200,
                height: 100
            }
        );
        assert_eq!(right_monitor.to_logical(200.0, 20.0), (2020.0, 10.0));
    }

    #[test]
    fn csd_shadow_inset_and_child_offset() {
        // GNOME reports GTK 4 windows with their 25 px shadow included.
        let buffer = Rect::new(100.0, 100.0, 1380.0, 1042.0);
        let frame = buffer.inset(25.0, 25.0, 25.0, 25.0);
        assert_eq!(frame, Rect::new(125.0, 125.0, 1330.0, 992.0));
        let button = frame.translate_child(&Rect::new(10.0, 8.0, 34.0, 34.0));
        assert_eq!(button, Rect::new(135.0, 133.0, 34.0, 34.0));
        assert!(button.contains(135.0, 133.0) && !button.contains(169.0, 133.0));
    }
}

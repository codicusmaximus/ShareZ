//! A captured still image.

use std::fmt;

use crate::geom::PixelRect;

/// Tightly packed 8-bit RGBA pixels, straight (not premultiplied) alpha.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Frame {
    /// # Panics
    /// If `data` is not exactly `width * height * 4` bytes.
    #[must_use]
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        assert_eq!(
            data.len(),
            width as usize * height as usize * 4,
            "RGBA frame size mismatch"
        );
        Self {
            width,
            height,
            data,
        }
    }

    #[must_use]
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }

    /// Copies out a region. `None` if the region is empty or not fully inside the frame.
    #[must_use]
    pub fn crop(&self, r: PixelRect) -> Option<Frame> {
        if r.width == 0
            || r.height == 0
            || r.x.checked_add(r.width)? > self.width
            || r.y.checked_add(r.height)? > self.height
        {
            return None;
        }
        let row = r.width as usize * 4;
        let mut data = Vec::with_capacity(row * r.height as usize);
        for y in r.y..r.y + r.height {
            let start = y as usize * self.stride() + r.x as usize * 4;
            data.extend_from_slice(&self.data[start..start + row]);
        }
        Some(Frame::new(r.width, r.height, data))
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_copies_rows() {
        let data: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
        let f = Frame::new(4, 3, data);
        let c = f
            .crop(PixelRect {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            })
            .unwrap();
        assert_eq!(c.width, 2);
        assert_eq!(&c.data[..8], &f.data[20..28]);
        assert_eq!(&c.data[8..], &f.data[36..44]);
        assert!(
            f.crop(PixelRect {
                x: 3,
                y: 0,
                width: 2,
                height: 1
            })
            .is_none()
        );
    }
}

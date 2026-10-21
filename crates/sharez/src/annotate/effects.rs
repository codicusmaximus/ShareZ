//! The blur, pixelate and highlight effects, as ShareX's `ImageHelpers` does them, on a part of
//! a straight-alpha RGBA image.

use sharez_core::Frame;

use super::shapes::Color;

/// A box blur, run twice each way (`ImageHelpers.BoxBlur`). `range` is made odd.
pub fn blur(image: &mut Frame, range: i32) {
    if range <= 1 {
        return;
    }
    let range = if range % 2 == 0 { range + 1 } else { range };
    for _ in 0..2 {
        blur_line(image, range, true);
        blur_line(image, range, false);
    }
}

/// One pass of a box blur along rows or columns with a running sum. Transparent black pixels
/// count towards the average but add nothing, as in ShareX.
fn blur_line(image: &mut Frame, range: i32, horizontal: bool) {
    let (w, h) = (image.width as i32, image.height as i32);
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    let half = range / 2;
    let stride = image.stride();
    let index = |line: i32, i: i32| {
        if horizontal {
            line as usize * stride + i as usize * 4
        } else {
            i as usize * stride + line as usize * 4
        }
    };
    let mut out = vec![[0u8; 4]; len as usize];
    for line in 0..lines {
        let mut hits = 0i32;
        let mut sum = [0i32; 4];
        for i in -half..len {
            let old = i - half - 1;
            if old >= 0 {
                let p = &image.data[index(line, old)..index(line, old) + 4];
                for c in 0..4 {
                    sum[c] -= i32::from(p[c]);
                }
                hits -= 1;
            }
            let new = i + half;
            if new < len {
                let p = &image.data[index(line, new)..index(line, new) + 4];
                for c in 0..4 {
                    sum[c] += i32::from(p[c]);
                }
                hits += 1;
            }
            if i >= 0 {
                let hits = hits.max(1);
                out[i as usize] = sum.map(|s| (s / hits) as u8);
            }
        }
        for (i, p) in out.iter().enumerate() {
            let at = index(line, i as i32);
            image.data[at..at + 4].copy_from_slice(p);
        }
    }
}

/// Averages each `size` × `size` block, weighting colours by their alpha
/// (`ImageHelpers.Pixelate`).
pub fn pixelate(image: &mut Frame, size: i32) {
    if size <= 1 {
        return;
    }
    let size = size as u32;
    let (w, h) = (image.width, image.height);
    let stride = image.stride();
    for y in (0..h).step_by(size as usize) {
        for x in (0..w).step_by(size as usize) {
            let (x_end, y_end) = ((x + size).min(w), (y + size).min(h));
            let count = f32::from(u16::try_from((x_end - x) * (y_end - y)).unwrap_or(u16::MAX));
            let (mut r, mut g, mut b, mut a, mut weight) = (0f32, 0f32, 0f32, 0f32, 0f32);
            for yy in y..y_end {
                for xx in x..x_end {
                    let i = yy as usize * stride + xx as usize * 4;
                    let p = &image.data[i..i + 4];
                    let wgt = f32::from(p[3]) / 255.0;
                    r += f32::from(p[0]) * wgt;
                    g += f32::from(p[1]) * wgt;
                    b += f32::from(p[2]) * wgt;
                    a += f32::from(p[3]) * wgt;
                    weight += wgt;
                }
            }
            let avg = if weight > 0.0 {
                [
                    (r / weight) as u8,
                    (g / weight) as u8,
                    (b / weight) as u8,
                    (a / count) as u8,
                ]
            } else {
                [0, 0, 0, 0]
            };
            for yy in y..y_end {
                for xx in x..x_end {
                    let i = yy as usize * stride + xx as usize * 4;
                    image.data[i..i + 4].copy_from_slice(&avg);
                }
            }
        }
    }
}

/// Darkens each channel to at most the highlight's, like a highlighter pen
/// (`ImageHelpers.HighlightImage`).
pub fn highlight(image: &mut Frame, color: Color) {
    for p in image.data.as_chunks_mut::<4>().0 {
        p[0] = p[0].min(color.r);
        p[1] = p[1].min(color.g);
        p[2] = p[2].min(color.b);
    }
}

/// Copies `src` into `dst` with its top left corner at `x`, `y`.
pub fn paste(dst: &mut Frame, src: &Frame, x: u32, y: u32) {
    let row = src.stride();
    let dst_stride = dst.stride();
    for yy in 0..src.height {
        let from = yy as usize * row;
        let to = (y + yy) as usize * dst_stride + x as usize * 4;
        dst.data[to..to + row].copy_from_slice(&src.data[from..from + row]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Frame {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&f(x, y));
            }
        }
        Frame::new(w, h, data)
    }

    #[test]
    fn blur_keeps_flat_colour_and_softens_edges() {
        let mut flat = frame(20, 20, |_, _| [10, 20, 30, 255]);
        let before = flat.clone();
        blur(&mut flat, 5);
        assert_eq!(flat, before);
        let mut edge = frame(20, 1, |x, _| {
            if x < 10 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        blur(&mut edge, 5);
        let mid = edge.data[10 * 4];
        assert!(mid > 0 && mid < 255, "{mid}");
    }

    #[test]
    fn pixelate_averages_blocks() {
        let mut f = frame(4, 2, |x, _| {
            if x % 2 == 0 {
                [0, 0, 0, 255]
            } else {
                [200, 100, 50, 255]
            }
        });
        pixelate(&mut f, 2);
        assert_eq!(&f.data[0..4], &[100, 50, 25, 255]);
        assert_eq!(&f.data[4..8], &[100, 50, 25, 255]);
    }

    #[test]
    fn highlight_caps_channels() {
        let mut f = frame(1, 1, |_, _| [200, 200, 200, 255]);
        highlight(&mut f, Color::rgb(255, 255, 0));
        assert_eq!(f.data, vec![200, 200, 0, 255]);
    }
}

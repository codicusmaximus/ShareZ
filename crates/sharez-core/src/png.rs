//! Decoding the PNGs that desktop services hand us (the screenshot portal, the GNOME helper).

use std::{
    fmt,
    io::{BufRead, Seek},
};

use crate::Frame;

/// Wider or taller than any screen. A header can claim up to 2³¹ pixels a side, and the decoder
/// sizes its output from the header before reading any pixels.
pub const MAX_SIDE: u32 = 32_768;
/// The decoded image's size cap: three 8K screens side by side are about 400 MB.
pub const MAX_BYTES: usize = 1 << 30;

#[derive(Debug)]
pub enum Error {
    Decode(png::DecodingError),
    TooLarge { width: u32, height: u32 },
    Format(png::ColorType),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(e) => e.fmt(f),
            Self::TooLarge { width, height } => write!(f, "a {width}×{height} image is too large"),
            Self::Format(c) => write!(f, "unsupported PNG {c:?}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            _ => None,
        }
    }
}

impl From<png::DecodingError> for Error {
    fn from(e: png::DecodingError) -> Self {
        Self::Decode(e)
    }
}

/// An RGB or RGBA PNG (any bit depth, or a palette) as RGBA.
pub fn decode(input: impl BufRead + Seek) -> Result<Frame, Error> {
    let mut decoder = png::Decoder::new(input);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let (width, height) = reader.info().size();
    let size = reader
        .output_buffer_size()
        .filter(|&n| width <= MAX_SIDE && height <= MAX_SIDE && n <= MAX_BYTES)
        .ok_or(Error::TooLarge { width, height })?;
    let mut buf = Vec::new();
    buf.try_reserve_exact(size)
        .map_err(|_| Error::TooLarge { width, height })?;
    buf.resize(size, 0);
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let data = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        other => return Err(Error::Format(other)),
    };
    Ok(Frame::new(info.width, info.height, data))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn encode(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn decodes_rgb_as_rgba() {
        let png = encode(2, 1, png::ColorType::Rgb, &[1, 2, 3, 4, 5, 6]);
        let frame = decode(Cursor::new(png)).unwrap();
        assert_eq!(frame.data, [1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn refuses_a_huge_header() {
        // A 1×1 image whose header then claims 65535×65535, CRC fixed up.
        let mut png = encode(1, 1, png::ColorType::Rgba, &[0; 4]);
        png[16..24].copy_from_slice(&[0, 0, 0xff, 0xff, 0, 0, 0xff, 0xff]);
        let crc = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());
        assert!(matches!(
            decode(Cursor::new(png)),
            Err(Error::TooLarge {
                width: 65535,
                height: 65535
            })
        ));
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}

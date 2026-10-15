//! A single window's own pixels, as the compositor draws it off screen, so windows in front of it
//! don't get in the way.
//!
//! - **GNOME:** the ShareZ Helper extension paints the window actor to a texture, as GNOME's own
//!   screenshot UI does in window mode.
//! - **KDE Plasma:** KWin's `ScreenShot2.CaptureWindow`. KWin only answers apps whose desktop file
//!   lists the interface under `X-KDE-DBUS-Restricted-Interfaces`.
//! - **X11:** the window's pixmap from the compositing manager, through the Composite extension.
//!   Without a compositor, covered parts of a window aren't kept anywhere.

use std::{collections::HashMap, time::Duration};

use futures_lite::AsyncReadExt;

use sharez_core::{Frame, Mapping, Rect};
use x11rb::{
    connection::Connection,
    protocol::{
        composite::ConnectionExt as _,
        xproto::{ConnectionExt as _, ImageFormat, ImageOrder, Window as XWindow},
    },
};
use zbus::{
    Connection as Bus,
    zvariant::{Fd, OwnedValue, Value},
};

use crate::{Error, Handle, Result, Window, dbus::call_within};

/// Rendering a large window and encoding it can take a moment.
const TIMEOUT: Duration = Duration::from_secs(4);

/// A window's pixels and where they sit on screen.
#[derive(Debug, Clone)]
pub struct Shot {
    pub image: Frame,
    /// What `image` covers, in logical screen coordinates.
    pub area: Rect,
}

impl Shot {
    /// The pixels under `logical`, a rectangle on screen. `None` if it misses the window.
    #[must_use]
    pub fn crop(&self, logical: Rect) -> Option<Frame> {
        let pixels = Mapping {
            logical: self.area,
            buffer_width: self.image.width,
            buffer_height: self.image.height,
        }
        .to_pixels(&logical)?;
        self.image.crop(pixels)
    }
}

/// Captures `window` on its own.
pub async fn capture(window: &Window) -> Result<Shot> {
    match &window.handle {
        Some(Handle::Gnome(id)) => gnome(*id).await,
        Some(Handle::Kwin(id)) => kwin(id, window.frame).await,
        Some(Handle::X11(id)) => x11(*id),
        None => Err(Error::Unsupported),
    }
}

async fn gnome(id: u32) -> Result<Shot> {
    const HELPER: &str = "io.github.codicusmaximus.ShareZ.ShellHelper";
    let conn = Bus::session().await?;
    let (png, (x, y, w, h)): (Vec<u8>, (i32, i32, i32, i32)) = call_within(
        TIMEOUT,
        &conn,
        HELPER,
        "/io/github/codicusmaximus/ShareZ/ShellHelper",
        HELPER,
        "CaptureWindow",
        &(id,),
    )
    .await?;
    Ok(Shot {
        image: decode_png(&png)?,
        area: Rect::new(x.into(), y.into(), w.into(), h.into()),
    })
}

fn decode_png(png: &[u8]) -> Result<Frame> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or_default()];
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
        other => return Err(Error::BadData("ShareZ Helper", format!("PNG {other:?}"))),
    };
    Ok(Frame::new(info.width, info.height, data))
}

/// The `QImage::Format` values KWin sends, as the byte order of one pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// 32-bit ARGB words, which are B, G, R, A in memory on little-endian machines.
    Bgra {
        alpha: bool,
        premultiplied: bool,
    },
    Rgba {
        alpha: bool,
        premultiplied: bool,
    },
}

impl Layout {
    fn from_qt(format: u32) -> Option<Self> {
        let (alpha, premultiplied) = (true, true);
        Some(match format {
            4 => Self::Bgra {
                alpha: false,
                premultiplied: false,
            },
            5 => Self::Bgra {
                alpha,
                premultiplied: false,
            },
            6 => Self::Bgra {
                alpha,
                premultiplied,
            },
            16 => Self::Rgba {
                alpha: false,
                premultiplied: false,
            },
            17 => Self::Rgba {
                alpha,
                premultiplied: false,
            },
            18 => Self::Rgba {
                alpha,
                premultiplied,
            },
            _ => return None,
        })
    }

    /// One pixel as straight RGBA.
    fn rgba(self, px: [u8; 4]) -> [u8; 4] {
        let (Self::Bgra {
            alpha,
            premultiplied,
        }
        | Self::Rgba {
            alpha,
            premultiplied,
        }) = self;
        let [red, green, blue, opacity] = match self {
            Self::Bgra { .. } => [px[2], px[1], px[0], px[3]],
            Self::Rgba { .. } => px,
        };
        if !alpha {
            return [red, green, blue, 255];
        }
        if !premultiplied {
            return [red, green, blue, opacity];
        }
        let [red, green, blue] = [red, green, blue].map(|c| unpremultiply(c, opacity));
        [red, green, blue, opacity]
    }
}

fn unpremultiply(c: u8, a: u8) -> u8 {
    if a == 0 {
        0
    } else {
        u8::try_from((u16::from(c) * 255 + u16::from(a) / 2) / u16::from(a)).unwrap_or(255)
    }
}

/// Rows of `stride` bytes, each starting with `width` 4-byte pixels, as tightly packed RGBA.
fn repack(
    data: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    pixel: impl Fn([u8; 4]) -> [u8; 4],
) -> Result<Frame> {
    let row = width as usize * 4;
    if stride < row || data.len() < stride * height as usize {
        return Err(Error::BadData(
            "window image",
            "fewer bytes than its size".into(),
        ));
    }
    let out = data
        .chunks(stride)
        .take(height as usize)
        .flat_map(|line| {
            line[..row]
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&p| pixel(p))
        })
        .collect();
    Ok(Frame::new(width, height, out))
}

async fn kwin(id: &str, frame: Rect) -> Result<Shot> {
    let (reader, writer) = std::io::pipe()?;
    let options: HashMap<&str, Value> = HashMap::from([
        ("include-decoration", true.into()),
        ("include-shadow", false.into()),
        ("include-cursor", false.into()),
        ("native-resolution", true.into()),
    ]);
    let conn = Bus::session().await?;
    let results: HashMap<String, OwnedValue> = call_within(
        TIMEOUT,
        &conn,
        "org.kde.KWin.ScreenShot2",
        "/org/kde/KWin/ScreenShot2",
        "org.kde.KWin.ScreenShot2",
        "CaptureWindow",
        &(id, options, Fd::from(&writer)),
    )
    .await?;
    // KWin writes the pixels after replying; our end must close for the read to finish.
    drop(writer);
    let number = |key: &str| {
        results
            .get(key)
            .and_then(|v| u32::try_from(&**v).ok())
            .ok_or_else(|| Error::BadData("KWin", format!("no {key} in the screenshot")))
    };
    let (width, height, stride) = (number("width")?, number("height")?, number("stride")?);
    let format = number("format")?;
    let layout = Layout::from_qt(format)
        .ok_or_else(|| Error::BadData("KWin", format!("image format {format}")))?;
    let mut data = Vec::new();
    async_io::Async::new(reader)?.read_to_end(&mut data).await?;
    Ok(Shot {
        image: repack(&data, width, height, stride as usize, |p| layout.rgba(p))?,
        area: frame,
    })
}

fn x11_err(e: impl std::fmt::Display) -> Error {
    Error::X11(e.to_string())
}

fn x11(client: XWindow) -> Result<Shot> {
    let (conn, screen_num) = x11rb::connect(None).map_err(x11_err)?;
    let setup = conn.setup();
    let root = setup.roots[screen_num].root;
    let cm = conn
        .intern_atom(false, format!("_NET_WM_CM_S{screen_num}").as_bytes())
        .map_err(x11_err)?
        .reply()
        .map_err(x11_err)?
        .atom;
    let owner = conn
        .get_selection_owner(cm)
        .map_err(x11_err)?
        .reply()
        .map_err(x11_err)?
        .owner;
    if owner == x11rb::NONE {
        return Err(Error::Unsupported);
    }
    // The window manager's frame around the client: the client's ancestor just below the root.
    let mut frame = client;
    loop {
        let tree = conn
            .query_tree(frame)
            .map_err(x11_err)?
            .reply()
            .map_err(x11_err)?;
        if tree.parent == root || tree.parent == x11rb::NONE {
            break;
        }
        frame = tree.parent;
    }
    conn.composite_query_version(0, 4)
        .map_err(x11_err)?
        .reply()
        .map_err(x11_err)?;
    let pixmap = conn.generate_id().map_err(x11_err)?;
    conn.composite_name_window_pixmap(frame, pixmap)
        .map_err(x11_err)?
        .check()
        .map_err(x11_err)?;
    let result = (|| {
        let geo = conn
            .get_geometry(frame)
            .map_err(x11_err)?
            .reply()
            .map_err(x11_err)?;
        let pos = conn
            .translate_coordinates(frame, root, 0, 0)
            .map_err(x11_err)?
            .reply()
            .map_err(x11_err)?;
        // The pixmap includes the border.
        let border = geo.border_width;
        let (w, h) = (geo.width + 2 * border, geo.height + 2 * border);
        let bpp = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == geo.depth)
            .map(|f| f.bits_per_pixel);
        if bpp != Some(32) {
            return Err(Error::X11(format!(
                "window depth {} at {bpp:?} bits per pixel",
                geo.depth
            )));
        }
        let image = conn
            .get_image(ImageFormat::Z_PIXMAP, pixmap, 0, 0, w, h, !0)
            .map_err(x11_err)?
            .reply()
            .map_err(x11_err)?;
        let lsb = setup.image_byte_order == ImageOrder::LSB_FIRST;
        // Depth 32 windows (ARGB visuals) carry premultiplied alpha; others have none.
        let layout = match (lsb, geo.depth == 32) {
            (true, alpha) => Layout::Bgra {
                alpha,
                premultiplied: alpha,
            },
            (false, alpha) => Layout::Rgba {
                alpha,
                premultiplied: alpha,
            },
        };
        // MSB-first servers store ARGB words as A, R, G, B.
        let pixel = |[b0, b1, b2, b3]: [u8; 4]| {
            if lsb {
                layout.rgba([b0, b1, b2, b3])
            } else {
                layout.rgba([b1, b2, b3, b0])
            }
        };
        Ok(Shot {
            image: repack(&image.data, w.into(), h.into(), usize::from(w) * 4, pixel)?,
            area: Rect::new(
                f64::from(pos.dst_x) - f64::from(border),
                f64::from(pos.dst_y) - f64::from(border),
                w.into(),
                h.into(),
            ),
        })
    })();
    let _ = conn.free_pixmap(pixmap);
    let _ = conn.flush();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_by_the_window_scale() {
        // A 10×10 logical window drawn at 2×.
        let data = (0..20 * 20).flat_map(|i: u32| [(i % 20) as u8, (i / 20) as u8, 0, 255]);
        let shot = Shot {
            image: Frame::new(20, 20, data.collect()),
            area: Rect::new(100.0, 50.0, 10.0, 10.0),
        };
        let c = shot.crop(Rect::new(102.0, 51.0, 3.0, 4.0)).unwrap();
        assert_eq!((c.width, c.height), (6, 8));
        assert_eq!(&c.data[..2], &[4, 2]);
        assert!(shot.crop(Rect::new(0.0, 0.0, 5.0, 5.0)).is_none());
    }

    #[test]
    fn kwin_formats() {
        let argb_pre = Layout::from_qt(6).unwrap();
        assert_eq!(argb_pre.rgba([20, 40, 60, 128]), [120, 80, 40, 128]);
        assert_eq!(
            Layout::from_qt(4).unwrap().rgba([1, 2, 3, 0]),
            [3, 2, 1, 255]
        );
        assert_eq!(
            Layout::from_qt(17).unwrap().rgba([1, 2, 3, 4]),
            [1, 2, 3, 4]
        );
        assert_eq!(Layout::from_qt(99), None);
    }

    #[test]
    fn repacks_padded_rows() {
        let data = [1, 2, 3, 4, 9, 9, 5, 6, 7, 8, 9, 9];
        let f = repack(&data, 1, 2, 6, |p| p).unwrap();
        assert_eq!(f.data, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(repack(&data[..8], 1, 2, 6, |p| p).is_err());
    }
}

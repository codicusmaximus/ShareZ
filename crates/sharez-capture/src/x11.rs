use sharez_core::Frame;
use x11rb::{
    connection::Connection,
    protocol::xproto::{ConnectionExt as _, ImageFormat, ImageOrder},
};

use crate::{Error, Result};

fn x11_err(e: impl std::fmt::Display) -> Error {
    Error::X11(e.to_string())
}

pub fn freeze() -> Result<Frame> {
    let (conn, screen_num) = x11rb::connect(None).map_err(x11_err)?;
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let (w, h) = (screen.width_in_pixels, screen.height_in_pixels);
    let bpp = setup
        .pixmap_formats
        .iter()
        .find(|f| f.depth == screen.root_depth)
        .map(|f| f.bits_per_pixel);
    if bpp != Some(32) {
        return Err(Error::UnsupportedFormat(format!(
            "root depth {} at {bpp:?} bits per pixel",
            screen.root_depth
        )));
    }
    let image = conn
        .get_image(ImageFormat::Z_PIXMAP, screen.root, 0, 0, w, h, !0)
        .map_err(x11_err)?
        .reply()
        .map_err(x11_err)?;
    // 24/32-bit TrueColor is BGRX in memory on LSB-first servers and XRGB on MSB-first ones.
    let lsb = setup.image_byte_order == ImageOrder::LSB_FIRST;
    let data = image
        .data
        .chunks_exact(4)
        .flat_map(|p| {
            if lsb {
                [p[2], p[1], p[0], 255]
            } else {
                [p[1], p[2], p[3], 255]
            }
        })
        .collect();
    Ok(Frame::new(w.into(), h.into(), data))
}

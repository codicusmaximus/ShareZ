//! X11: window rectangles from EWMH and a root-window grab.
//!
//! Questions: do we get usable frame rectangles (including CSD shadow insets via
//! `_GTK_FRAME_EXTENTS`), how fast is a full-screen grab, and what does this look like under
//! XWayland (where only X11 clients are visible)?

use std::{io::BufWriter, path::Path, time::Instant};

use anyhow::Context;
use serde_json::{Value, json};
use x11rb::{
    connection::Connection,
    protocol::{randr::ConnectionExt as _, xproto::*},
    rust_connection::RustConnection,
};

fn atom(conn: &RustConnection, name: &str) -> anyhow::Result<Atom> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

fn prop32(conn: &RustConnection, win: Window, prop: Atom, ty: impl Into<Atom>) -> Vec<u32> {
    conn.get_property(false, win, prop, ty, 0, 1024)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().map(|v| v.collect()))
        .unwrap_or_default()
}

fn prop_string(conn: &RustConnection, win: Window, prop: Atom, ty: impl Into<Atom>) -> String {
    conn.get_property(false, win, prop, ty, 0, 1024)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| {
            String::from_utf8_lossy(&r.value)
                .replace('\0', " ")
                .trim()
                .to_owned()
        })
        .unwrap_or_default()
}

fn atom_name(conn: &RustConnection, a: Atom) -> String {
    conn.get_atom_name(a)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| String::from_utf8_lossy(&r.name).into_owned())
        .unwrap_or_default()
}

pub async fn probe(out: &Path) -> anyhow::Result<Value> {
    let (conn, screen_num) = RustConnection::connect(None).context("connect to $DISPLAY")?;
    let screen = conn.setup().roots[screen_num].clone();
    let root = screen.root;

    let (net_wm_name, utf8) = (atom(&conn, "_NET_WM_NAME")?, atom(&conn, "UTF8_STRING")?);
    let wm_name = prop32(
        &conn,
        root,
        atom(&conn, "_NET_SUPPORTING_WM_CHECK")?,
        AtomEnum::WINDOW,
    )
    .first()
    .map(|&w| prop_string(&conn, w, net_wm_name, utf8));

    let stacking = prop32(
        &conn,
        root,
        atom(&conn, "_NET_CLIENT_LIST_STACKING")?,
        AtomEnum::WINDOW,
    );
    let (frame_ext, gtk_ext) = (
        atom(&conn, "_NET_FRAME_EXTENTS")?,
        atom(&conn, "_GTK_FRAME_EXTENTS")?,
    );
    let (wm_type, wm_state) = (
        atom(&conn, "_NET_WM_WINDOW_TYPE")?,
        atom(&conn, "_NET_WM_STATE")?,
    );

    let mut windows = Vec::new();
    for &w in stacking.iter().rev() {
        let Ok(geo) = conn.get_geometry(w)?.reply() else {
            continue;
        };
        let pos = conn.translate_coordinates(w, root, 0, 0)?.reply()?;
        let children = conn
            .query_tree(w)?
            .reply()
            .map(|t| t.children.len())
            .unwrap_or(0);
        windows.push(json!({
            "id": w,
            "title": Some(prop_string(&conn, w, net_wm_name, utf8))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| prop_string(&conn, w, AtomEnum::WM_NAME.into(), AtomEnum::STRING)),
            "class": prop_string(&conn, w, AtomEnum::WM_CLASS.into(), AtomEnum::STRING),
            "client_rect": [pos.dst_x, pos.dst_y, geo.width, geo.height],
            "net_frame_extents_lrtb": prop32(&conn, w, frame_ext, AtomEnum::CARDINAL),
            "gtk_frame_extents_lrtb": prop32(&conn, w, gtk_ext, AtomEnum::CARDINAL),
            "types": prop32(&conn, w, wm_type, AtomEnum::ATOM).into_iter().map(|a| atom_name(&conn, a)).collect::<Vec<_>>(),
            "states": prop32(&conn, w, wm_state, AtomEnum::ATOM).into_iter().map(|a| atom_name(&conn, a)).collect::<Vec<_>>(),
            "x_child_windows": children,
        }));
    }

    let monitors = conn
        .randr_get_monitors(root, true)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| {
            r.monitors
                .iter()
                .map(|m| json!({ "name": atom_name(&conn, m.name), "rect": [m.x, m.y, m.width, m.height], "primary": m.primary }))
                .collect::<Vec<_>>()
        });

    // Full root grab: this is the "freeze the screen" step on X11. Under XWayland the root
    // window usually has no contents to read, and GetImage fails with BadMatch.
    let (w, h) = (screen.width_in_pixels, screen.height_in_pixels);
    let grab =
        grab_root(&conn, root, w, h, out).unwrap_or_else(|e| json!({ "error": format!("{e:#}") }));

    Ok(json!({
        "xwayland_only": crate::env::session_type() == "wayland",
        "window_manager": wm_name,
        "root_size": [w, h],
        "monitors": monitors,
        "window_count": windows.len(),
        "windows_top_to_bottom": windows,
        "root_grab": grab,
    }))
}

fn grab_root(
    conn: &RustConnection,
    root: Window,
    w: u16,
    h: u16,
    out: &Path,
) -> anyhow::Result<Value> {
    let start = Instant::now();
    let img = conn
        .get_image(ImageFormat::Z_PIXMAP, root, 0, 0, w, h, !0)?
        .reply()
        .context("GetImage on root")?;
    let grab_ms = start.elapsed().as_millis();

    let path = out.join("x11-root.png");
    let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
    let mut nonblack = 0usize;
    for px in img.data.chunks_exact(4) {
        // Depth 24/32 ZPixmap on little-endian servers is BGRX.
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        if px[0] | px[1] | px[2] != 0 {
            nonblack += 1;
        }
    }
    let mut enc = png::Encoder::new(
        BufWriter::new(std::fs::File::create(&path)?),
        w.into(),
        h.into(),
    );
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgb)?;

    Ok(json!({
        "ms": grab_ms,
        "depth": img.depth,
        "nonblack_fraction": nonblack as f64 / (w as f64 * h as f64),
        "file": path,
    }))
}

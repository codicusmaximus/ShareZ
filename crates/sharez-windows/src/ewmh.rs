//! X11: the window manager publishes the stack and frame sizes on the root window (EWMH).

use sharez_core::Rect;
use x11rb::{
    connection::Connection,
    protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, Window as XWindow},
    rust_connection::RustConnection,
};

use crate::{Error, Result, Window, WindowKind};

fn x11_err(e: impl std::fmt::Display) -> Error {
    Error::X11(e.to_string())
}

struct Atoms {
    stacking: Atom,
    net_wm_name: Atom,
    utf8: Atom,
    frame_extents: Atom,
    gtk_frame_extents: Atom,
    wm_type: Atom,
    wm_state: Atom,
    hidden: Atom,
    type_dialog: Atom,
    type_dock: Atom,
    type_desktop: Atom,
    type_normal: Atom,
}

fn atom(conn: &RustConnection, name: &str) -> Result<Atom> {
    Ok(conn
        .intern_atom(false, name.as_bytes())
        .map_err(x11_err)?
        .reply()
        .map_err(x11_err)?
        .atom)
}

fn prop32(conn: &RustConnection, win: XWindow, prop: Atom, ty: impl Into<Atom>) -> Vec<u32> {
    conn.get_property(false, win, prop, ty, 0, 1024)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().map(Iterator::collect))
        .unwrap_or_default()
}

fn prop_string(conn: &RustConnection, win: XWindow, prop: Atom, ty: impl Into<Atom>) -> String {
    conn.get_property(false, win, prop, ty, 0, 1024)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| String::from_utf8_lossy(&r.value).into_owned())
        .unwrap_or_default()
}

#[derive(Default)]
struct Extents {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

/// EWMH orders extents left, right, top, bottom.
fn extents(v: &[u32]) -> Extents {
    match *v {
        [left, right, top, bottom, ..] => Extents {
            left: left.into(),
            right: right.into(),
            top: top.into(),
            bottom: bottom.into(),
        },
        _ => Extents::default(),
    }
}

pub fn windows() -> Result<Vec<Window>> {
    let (conn, screen_num) = x11rb::connect(None).map_err(x11_err)?;
    let root = conn.setup().roots[screen_num].root;
    let atoms = Atoms {
        stacking: atom(&conn, "_NET_CLIENT_LIST_STACKING")?,
        net_wm_name: atom(&conn, "_NET_WM_NAME")?,
        utf8: atom(&conn, "UTF8_STRING")?,
        frame_extents: atom(&conn, "_NET_FRAME_EXTENTS")?,
        gtk_frame_extents: atom(&conn, "_GTK_FRAME_EXTENTS")?,
        wm_type: atom(&conn, "_NET_WM_WINDOW_TYPE")?,
        wm_state: atom(&conn, "_NET_WM_STATE")?,
        hidden: atom(&conn, "_NET_WM_STATE_HIDDEN")?,
        type_dialog: atom(&conn, "_NET_WM_WINDOW_TYPE_DIALOG")?,
        type_dock: atom(&conn, "_NET_WM_WINDOW_TYPE_DOCK")?,
        type_desktop: atom(&conn, "_NET_WM_WINDOW_TYPE_DESKTOP")?,
        type_normal: atom(&conn, "_NET_WM_WINDOW_TYPE_NORMAL")?,
    };

    let mut out = Vec::new();
    // Bottom to top in the property; ShareZ wants topmost first.
    for &w in prop32(&conn, root, atoms.stacking, AtomEnum::WINDOW)
        .iter()
        .rev()
    {
        let Ok(geo) = conn.get_geometry(w).map_err(x11_err)?.reply() else {
            continue; // Closed since we read the list.
        };
        let Ok(pos) = conn
            .translate_coordinates(w, root, 0, 0)
            .map_err(x11_err)?
            .reply()
        else {
            continue;
        };
        let window_rect = Rect::new(
            pos.dst_x.into(),
            pos.dst_y.into(),
            geo.width.into(),
            geo.height.into(),
        );
        // Server-side decorations sit outside the client window; client-side shadows inside it.
        let ssd = extents(&prop32(&conn, w, atoms.frame_extents, AtomEnum::CARDINAL));
        let csd = extents(&prop32(
            &conn,
            w,
            atoms.gtk_frame_extents,
            AtomEnum::CARDINAL,
        ));
        let frame = window_rect.inset(
            csd.left - ssd.left,
            csd.top - ssd.top,
            csd.right - ssd.right,
            csd.bottom - ssd.bottom,
        );
        let client = window_rect.inset(csd.left, csd.top, csd.right, csd.bottom);

        let types = prop32(&conn, w, atoms.wm_type, AtomEnum::ATOM);
        let kind = match types.first() {
            Some(&t) if t == atoms.type_desktop => WindowKind::Desktop,
            Some(&t) if t == atoms.type_dock => WindowKind::Dock,
            Some(&t) if t == atoms.type_dialog => WindowKind::Dialog,
            Some(&t) if t == atoms.type_normal => WindowKind::Normal,
            None => WindowKind::Normal,
            Some(_) => WindowKind::Other,
        };
        let mut title = prop_string(&conn, w, atoms.net_wm_name, atoms.utf8);
        if title.is_empty() {
            title = prop_string(&conn, w, AtomEnum::WM_NAME.into(), AtomEnum::STRING);
        }
        // WM_CLASS is "instance\0class\0"; the class names the app.
        let class = prop_string(&conn, w, AtomEnum::WM_CLASS.into(), AtomEnum::STRING);
        out.push(Window {
            title,
            app_id: class
                .split('\0')
                .nth(1)
                .filter(|c| !c.is_empty())
                .map(str::to_owned),
            pid: None,
            kind,
            frame,
            client: Some(client),
            shadow_included: false,
            minimized: prop32(&conn, w, atoms.wm_state, AtomEnum::ATOM).contains(&atoms.hidden),
        });
    }
    Ok(out)
}

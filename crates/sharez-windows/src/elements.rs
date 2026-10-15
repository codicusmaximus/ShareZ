//! Elements inside windows (buttons, panels, fields), from the accessibility tree.
//!
//! Apps report element positions relative to their own window, because on Wayland they don't know
//! where the window is. So each listed window is matched to the app's accessible toplevel by size,
//! and element positions are offset by the window's client area. On GNOME without the helper
//! extension the same match also tells how much client-side shadow to inset.

use futures_util::future::join_all;
use sharez_core::Rect;
use zbus::Connection;

use crate::{
    Result, Window,
    a11y::{self, COORDS_WINDOW, Node},
};

/// Size differences up to this are rounding (KWin reports fractional logical sizes).
const SIZE_TOLERANCE: f64 = 2.0;
/// The largest client-side shadow to consider when inferring one from sizes.
const MAX_SHADOW: f64 = 64.0;
/// Elements smaller than this are not worth snapping to.
const MIN_ELEMENT: f64 = 5.0;
/// Guard against cycles in buggy accessibility trees.
const MAX_DEPTH: usize = 40;
/// Parts of a control that should snap to the control itself: the text of a button, say.
const CONTROL_PARTS: &[&str] = &["label", "icon", "image", "static", "text"];
const CONTROLS: &[&str] = &[
    "push button",
    "toggle button",
    "check box",
    "radio button",
    "menu item",
    "check menu item",
    "radio menu item",
    "page tab",
    "link",
    "combo box",
    "list item",
    "table cell",
    "tree item",
    "button",
];

#[derive(Debug, Clone)]
struct Toplevel {
    node: Node,
    name: String,
    width: f64,
    height: f64,
}

/// The accessibility trees of all running apps, as needed to look inside windows.
#[derive(Clone)]
pub struct Elements {
    conn: Connection,
    toplevels: Vec<Toplevel>,
}

impl std::fmt::Debug for Elements {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.toplevels.iter().map(|t| (&t.name, t.width, t.height)))
            .finish()
    }
}

/// A window prepared for snapping.
#[derive(Debug, Clone)]
pub struct Target {
    pub window: Window,
    /// What the user sees as the window: the frame, minus any client-side shadow.
    pub visible: Rect,
    /// Where the app's own coordinates start, if its accessible toplevel was found.
    inside: Option<(usize, Rect)>,
}

impl Target {
    /// A window nothing is known about inside.
    #[must_use]
    pub fn opaque(window: Window) -> Self {
        Self {
            visible: window.frame,
            window,
            inside: None,
        }
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= SIZE_TOLERANCE
}

async fn toplevels_of(conn: &Connection, app: Node) -> Vec<Toplevel> {
    let mut out = Vec::new();
    for node in a11y::children(conn, &app).await.unwrap_or_default() {
        // GTK 4 gives `GtkApplicationWindow` the "application" role.
        if !matches!(
            a11y::role(conn, &node).await.as_str(),
            "frame" | "window" | "dialog" | "alert" | "file chooser" | "application"
        ) {
            continue;
        }
        if let Ok(r) = a11y::extents(conn, &node, COORDS_WINDOW).await {
            let name = a11y::name(conn, &node).await;
            out.push(Toplevel {
                node,
                name,
                width: r.width,
                height: r.height,
            });
        }
    }
    out
}

fn best_match(toplevels: &[Toplevel], window: &Window, used: &[bool]) -> Option<(usize, Rect)> {
    let mut best: Option<(usize, Rect, bool)> = None;
    for (i, t) in toplevels.iter().enumerate() {
        if used[i] {
            continue;
        }
        let client = if let Some(c) = window.client {
            (close(c.width, t.width) && close(c.height, t.height)).then_some(c)
        } else if window.shadow_included {
            let (sx, sy) = (
                (window.frame.width - t.width) / 2.0,
                (window.frame.height - t.height) / 2.0,
            );
            ((sx - sy).abs() <= 1.0 && (0.0..=MAX_SHADOW).contains(&sx))
                .then(|| window.frame.inset(sx, sy, sx, sy))
        } else {
            let f = window.frame;
            (close(f.width, t.width) && close(f.height, t.height)).then_some(f)
        };
        let Some(client) = client else { continue };
        let titled = !window.title.is_empty() && window.title == t.name;
        if best.as_ref().is_none_or(|b| titled && !b.2) {
            best = Some((i, client, titled));
        }
    }
    best.map(|(i, client, _)| (i, client))
}

impl Elements {
    /// Reads every app's toplevels. Apps are queried in parallel so one hung app only costs the
    /// D-Bus timeout, not one timeout per app.
    pub async fn snapshot() -> Result<Self> {
        let conn = a11y::bus().await?;
        let apps = a11y::children(&conn, &a11y::registry_root()).await?;
        let toplevels = join_all(apps.into_iter().map(|app| toplevels_of(&conn, app)))
            .await
            .into_iter()
            .flatten()
            .collect();
        Ok(Self { conn, toplevels })
    }

    /// Matches windows (topmost first) to accessible toplevels. Each toplevel is used once, and an
    /// exact title match wins over a size-only match.
    #[must_use]
    pub fn targets(&self, windows: Vec<Window>) -> Vec<Target> {
        let mut used = vec![false; self.toplevels.len()];
        windows
            .into_iter()
            .map(|window| {
                let found = best_match(&self.toplevels, &window, &used);
                let Some((i, client)) = found else {
                    return Target::opaque(window);
                };
                used[i] = true;
                let visible = if window.shadow_included {
                    client
                } else {
                    window.frame
                };
                Target {
                    window,
                    visible,
                    inside: Some((i, client)),
                }
            })
            .collect()
    }

    /// The elements under a point (logical screen coordinates) inside a target, outermost first,
    /// each in logical screen coordinates. Empty when the app exposes nothing there.
    pub async fn elements_at(&self, target: &Target, x: f64, y: f64) -> Vec<Rect> {
        let Some((i, client)) = target.inside else {
            return Vec::new();
        };
        let (px, py) = ((x - client.x).floor() as i32, (y - client.y).floor() as i32);
        let top = &self.toplevels[i].node;
        // Some toolkits answer with the deepest element straight away (GTK 4), others one level at
        // a time, so drill down first and then walk back up to collect the ancestors.
        let mut deepest = top.clone();
        for _ in 0..MAX_DEPTH {
            match a11y::accessible_at(&self.conn, &deepest, px, py).await {
                Ok(next) if !a11y::is_null(&next) && next != deepest => deepest = next,
                _ => break,
            }
        }
        let mut chain = Vec::new();
        let mut node = deepest;
        for _ in 0..MAX_DEPTH {
            if node == *top || a11y::is_null(&node) {
                break;
            }
            let parent = a11y::parent(&self.conn, &node).await;
            chain.push(node);
            match parent {
                Ok(p) => node = p,
                Err(_) => break,
            }
        }
        let mut path: Vec<(String, Rect)> = Vec::new();
        for node in chain.iter().rev() {
            let Ok(r) = a11y::extents(&self.conn, node, COORDS_WINDOW).await else {
                continue;
            };
            let r = client.translate_child(&r);
            let new = path.last().is_none_or(|(_, last)| *last != r);
            if new
                && r.contains(x, y)
                && r.width >= MIN_ELEMENT
                && r.height >= MIN_ELEMENT
                && r != client
            {
                path.push((a11y::role(&self.conn, node).await, r));
            }
        }
        snap_path(path)
    }
}

/// Drops the innermost element when it is only part of a control (a button's label), so the
/// control itself is what gets snapped to.
fn snap_path(mut path: Vec<(String, Rect)>) -> Vec<Rect> {
    if let [.., (parent, _), (inner, _)] = &path[..]
        && CONTROL_PARTS.contains(&inner.as_str())
        && CONTROLS.contains(&parent.as_str())
    {
        path.pop();
    }
    path.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WindowKind;

    fn window(frame: Rect, client: Option<Rect>, shadow: bool, title: &str) -> Window {
        Window {
            title: title.into(),
            app_id: None,
            pid: None,
            kind: WindowKind::Normal,
            frame,
            client,
            shadow_included: shadow,
            minimized: false,
            handle: None,
        }
    }

    fn toplevel(name: &str, width: f64, height: f64) -> Toplevel {
        Toplevel {
            node: (
                ":1.1".into(),
                zbus::zvariant::OwnedObjectPath::try_from("/a").unwrap(),
            ),
            name: name.into(),
            width,
            height,
        }
    }

    fn match_with(toplevels: &[Toplevel], w: &Window) -> Option<(usize, Rect)> {
        best_match(toplevels, w, &vec![false; toplevels.len()])
    }

    #[test]
    fn button_labels_snap_to_the_button() {
        let button = Rect::new(413.0, 106.0, 122.0, 32.0);
        let label = Rect::new(425.0, 110.0, 89.0, 24.0);
        let path = vec![
            ("push button".to_owned(), button),
            ("label".to_owned(), label),
        ];
        assert_eq!(snap_path(path), vec![button]);
        let path = vec![("panel".to_owned(), button), ("label".to_owned(), label)];
        assert_eq!(snap_path(path), vec![button, label]);
    }

    #[test]
    fn gnome_shadow_inferred_from_sizes() {
        // Nautilus in the GNOME spike run: shell 1380×1042, app frame 1330×992.
        let w = window(Rect::new(438.0, 247.0, 1380.0, 1042.0), None, true, "");
        let (_, client) = match_with(&[toplevel("Files", 1330.0, 992.0)], &w).unwrap();
        assert_eq!(client, Rect::new(463.0, 272.0, 1330.0, 992.0));
        assert!(match_with(&[toplevel("Files", 1300.0, 992.0)], &w).is_none());
    }

    #[test]
    fn kwin_client_matches_and_title_wins() {
        // Konsole at 135 %: KWin's client area is fractional.
        let w = window(
            Rect::new(380.0, 244.0, 911.11, 579.26),
            Some(Rect::new(380.0, 271.41, 911.11, 551.85)),
            false,
            "feasibility : bash — Konsole",
        );
        let tops = vec![
            toplevel("other", 911.0, 552.0),
            toplevel("feasibility : bash — Konsole", 911.0, 552.0),
        ];
        let (i, client) = match_with(&tops, &w).unwrap();
        assert_eq!(i, 1);
        assert_eq!(client, w.client.unwrap());
    }
}

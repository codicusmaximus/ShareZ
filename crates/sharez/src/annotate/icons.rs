//! The tool icons. Desktop icon themes have nothing for most of ShareX's tools, so ShareZ draws
//! its own symbolic ones, writes them to its cache folder and adds that to the icon theme, which
//! recolours them to match light and dark styles.

use std::sync::Once;

use gtk::{gdk, glib};

use super::shapes::Tool;

const SVG_HEAD: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" fill-rule="evenodd" d=""##;
const SVG_TAIL: &str = r#""/></svg>"#;

const ICONS: &[(&str, &str)] = &[
    ("select", "M3 1v13l3.2-3.2 2.4 4.9 1.8-.9-2.4-4.8H13z"),
    ("rectangle", "M2 3h12v10H2zM4 5v6h8V5z"),
    (
        "ellipse",
        "M8 3a6 5 0 1 0 0 10A6 5 0 1 0 8 3zm0 2a4 3 0 1 1 0 6 4 3 0 1 1 0-6z",
    ),
    ("freehand", "M11.5 1.5l3 3-9 9h-3v-3zM1 15h14v1H1z"),
    (
        "freehand-arrow",
        "M11.5 1.5l3 3-8 8-3-3zM2.5 10.5l3 3L1 15z",
    ),
    ("line", "M12.6 2L14 3.4 3.4 14 2 12.6z"),
    ("arrow", "M14 2v6l-2.3-2.3L3.4 14 2 12.6l8.3-8.3L8 2z"),
    (
        "text-outline",
        "M2 2h12v4h-4v8H6V6H2zM3.5 3.5v1h4v8h1v-8h4v-1z",
    ),
    ("text-background", "M1 1h14v14H1zM4 4v2h3v6h2V6h3V4z"),
    ("speech-balloon", "M2 2h12v9H7l-3 3v-3H2z"),
    (
        "step",
        "M8 1a7 7 0 1 0 0 14A7 7 0 1 0 8 1zM7 4h2v8H7V6.6l-1.3.7-.5-1.4z",
    ),
    (
        "magnify",
        "M6.5 1a5.5 5.5 0 1 0 3.2 10l3.9 3.9 1.4-1.4-3.9-3.9A5.5 5.5 0 0 0 6.5 1zm0 2a3.5 3.5 0 1 1 0 7 3.5 3.5 0 0 1 0-7z",
    ),
    (
        "smart-eraser",
        "M9 2l5 5-6.5 6.5H4L2 11.5zM9 4.8L5.3 8.5l2.2 2.2 3.7-3.7zM1 14.5h14V16H1z",
    ),
    (
        "blur",
        "M8 1S3 7 3 10a5 5 0 0 0 10 0c0-3-5-9-5-9zm-2 9h1.5a.5.5 0 0 0 .5.5V12a2 2 0 0 1-2-2z",
    ),
    (
        "pixelate",
        "M2 2h4v4H2zM10 2h4v4h-4zM6 6h4v4H6zM2 10h4v4H2zM10 10h4v4h-4z",
    ),
    (
        "highlight",
        "M10.5 1l4.5 4.5-6 6L4.5 7zM3.8 7.7l4.5 4.5L7 13.5H2.5V12zM1 15h14v1H1z",
    ),
    (
        "crop",
        "M4 1h2v9h9v2h-3v3h-2v-3H4zM1 4h3v2H1zM8 4h4v4h-2V6H8z",
    ),
    (
        "tool-options",
        "M1 3h8v2H1zM12 3h3v2h-3zM9 1.5h2v5H9zM1 11h3v2H1zM7 11h8v2H7zM4 9.5h2v5H4z",
    ),
    ("annotate", "M11.5 1.5l3 3-9 9h-3v-3zM8 14h7v2H8z"),
];

pub fn name(id: &str) -> String {
    format!("sharez-{id}-symbolic")
}

pub fn tool(tool: Tool) -> String {
    name(tool.id())
}

/// Writes the icons and adds them to the icon theme, once.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = glib::user_cache_dir().join("sharez").join("icons");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        for (id, path) in ICONS {
            let file = dir.join(format!("{}.svg", name(id)));
            let svg = format!("{SVG_HEAD}{path}{SVG_TAIL}");
            if std::fs::read_to_string(&file).ok().as_deref() != Some(svg.as_str()) {
                let _ = std::fs::write(&file, svg);
            }
        }
        if let Some(display) = gdk::Display::default() {
            gtk::IconTheme::for_display(&display).add_search_path(&dir);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_an_icon() {
        for t in Tool::ALL {
            assert!(ICONS.iter().any(|(id, _)| *id == t.id()), "{}", t.id());
        }
    }
}

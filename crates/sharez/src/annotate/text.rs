//! Laying out the annotator's text with Pango, at 96 DPI so point sizes come out as they do in
//! ShareX on Windows.

use gtk::pango;
use pangocairo::prelude::*;

use super::shapes::{Align, TextStyle};

const DPI: f64 = 96.0;

thread_local! {
    static CONTEXT: pango::Context = {
        let map = pangocairo::FontMap::new();
        let context = map.create_context();
        pangocairo::functions::context_set_resolution(&context, DPI);
        context
    };
}

pub fn font(family: &str, size: i32, bold: bool, italic: bool) -> pango::FontDescription {
    let mut f = pango::FontDescription::new();
    f.set_family(family);
    f.set_size(size.max(1) * pango::SCALE);
    if bold {
        f.set_weight(pango::Weight::Bold);
    }
    if italic {
        f.set_style(pango::Style::Italic);
    }
    f
}

fn style_font(style: &TextStyle) -> pango::FontDescription {
    font(&style.font, style.size, style.bold, style.italic)
}

/// A layout of `text` in `style`, `width` wide when given (wrapping), for `context`.
pub fn layout(
    context: &pango::Context,
    text: &str,
    style: &TextStyle,
    width: Option<f64>,
) -> pango::Layout {
    let layout = pango::Layout::new(context);
    layout.set_font_description(Some(&style_font(style)));
    layout.set_text(text);
    if style.underline {
        let attrs = pango::AttrList::new();
        attrs.insert(pango::AttrInt::new_underline(pango::Underline::Single));
        layout.set_attributes(Some(&attrs));
    }
    if let Some(w) = width {
        layout.set_width((w * f64::from(pango::SCALE)) as i32);
        layout.set_wrap(pango::WrapMode::WordChar);
    }
    layout.set_alignment(match style.align {
        Align::Near => pango::Alignment::Left,
        Align::Center => pango::Alignment::Center,
        Align::Far => pango::Alignment::Right,
    });
    layout
}

/// Sets up a Cairo context to lay text out the same way as `text_size` measures it.
pub fn prepare(cr: &gtk::cairo::Context) -> pango::Context {
    let context = pangocairo::functions::create_context(cr);
    pangocairo::functions::context_set_resolution(&context, DPI);
    context
}

fn size_of(layout: &pango::Layout) -> (f64, f64) {
    let (_, logical) = layout.pixel_extents();
    (f64::from(logical.width()), f64::from(logical.height()))
}

/// How much room `text` takes, unwrapped.
pub fn text_size(text: &str, style: &TextStyle) -> (f64, f64) {
    CONTEXT.with(|c| size_of(&layout(c, text, style, None)))
}

/// How much room a step's label takes, in the bold sans font steps use.
pub fn step_size(text: &str, font_size: i32) -> (f64, f64) {
    CONTEXT.with(|c| size_of(&step_layout(c, text, font_size)))
}

pub fn step_layout(context: &pango::Context, text: &str, font_size: i32) -> pango::Layout {
    let layout = pango::Layout::new(context);
    layout.set_font_description(Some(&font("Sans", font_size, true, false)));
    layout.set_text(text);
    layout
}

/// A one-line label in the given font, for the editor's own overlay text.
pub fn label_layout(
    context: &pango::Context,
    text: &str,
    family: &str,
    size: i32,
) -> pango::Layout {
    let layout = pango::Layout::new(context);
    layout.set_font_description(Some(&font(family, size, false, false)));
    layout.set_text(text);
    layout
}

pub fn layout_size(layout: &pango::Layout) -> (f64, f64) {
    size_of(layout)
}

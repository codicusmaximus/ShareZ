//! The dialog that asks for a text shape's text and font (ShareX's `TextDrawingInputBox`).
//! Enter accepts and Shift+Enter starts a new line, as in ShareX.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::{gdk, glib, pango};

use super::shapes::{Align, Color, TextStyle};

pub fn rgba(c: Color) -> gdk::RGBA {
    gdk::RGBA::new(
        f32::from(c.r) / 255.0,
        f32::from(c.g) / 255.0,
        f32::from(c.b) / 255.0,
        f32::from(c.a) / 255.0,
    )
}

pub fn color(c: &gdk::RGBA) -> Color {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::rgba(
        byte(c.red()),
        byte(c.green()),
        byte(c.blue()),
        byte(c.alpha()),
    )
}

pub fn color_button(c: Color, alpha: bool) -> gtk::ColorDialogButton {
    let dialog = gtk::ColorDialog::new();
    dialog.set_with_alpha(alpha);
    let button = gtk::ColorDialogButton::new(Some(dialog));
    button.set_rgba(&rgba(c));
    button.set_valign(gtk::Align::Center);
    button
}

pub fn choice<T: Copy>(all: &[T], title: fn(T) -> &'static str, selected: u32) -> gtk::DropDown {
    let names: Vec<&str> = all.iter().map(|v| title(*v)).collect();
    let d = gtk::DropDown::from_strings(&names);
    d.set_selected(selected);
    d.set_valign(gtk::Align::Center);
    d
}

type Done = Box<dyn FnOnce(Option<(String, TextStyle)>)>;

/// Asks for text over `parent`. `done` gets the text and style, or `None` if cancelled.
pub fn ask(
    parent: &impl IsA<gtk::Widget>,
    text: &str,
    style: &TextStyle,
    done: impl FnOnce(Option<(String, TextStyle)>) + 'static,
) {
    let dialog = adw::Dialog::builder()
        .title("Text Input")
        .content_width(480)
        .build();

    let font = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    font.set_level(gtk::FontLevel::Family);
    // The button can only show installed families, so one that isn't (ShareX's Arial, say) shows
    // as Sans and is kept unless another font is picked.
    let installed = pangocairo::FontMap::default()
        .list_families()
        .iter()
        .any(|f| f.name().eq_ignore_ascii_case(&style.font));
    let shown = if installed {
        style.font.as_str()
    } else {
        "Sans"
    };
    font.set_font_desc(&pango::FontDescription::from_string(shown));
    let picked = Rc::new(std::cell::Cell::new(false));
    font.connect_font_desc_notify({
        let picked = Rc::clone(&picked);
        move |_| picked.set(true)
    });
    let original = style.font.clone();
    font.set_tooltip_text(Some("Font"));
    let size = gtk::SpinButton::with_range(1.0, 500.0, 1.0);
    size.set_value(f64::from(style.size));
    size.set_tooltip_text(Some("Size"));
    size.set_valign(gtk::Align::Center);
    let text_color = color_button(style.color, true);
    text_color.set_tooltip_text(Some("Text Color"));
    let toggle = |icon: &str, tip: &str, on: bool| {
        gtk::ToggleButton::builder()
            .icon_name(icon)
            .tooltip_text(tip)
            .active(on)
            .build()
    };
    let bold = toggle("format-text-bold-symbolic", "Bold", style.bold);
    let italic = toggle("format-text-italic-symbolic", "Italic", style.italic);
    let underline = toggle(
        "format-text-underline-symbolic",
        "Underline",
        style.underline,
    );
    let styles = gtk::Box::builder()
        .css_classes(["linked"])
        .valign(gtk::Align::Center)
        .build();
    styles.append(&bold);
    styles.append(&italic);
    styles.append(&underline);
    let align = choice(Align::ALL, Align::title, style.align.index());
    align.set_tooltip_text(Some("Horizontal Alignment"));
    let vertical = choice(
        &[Align::Near, Align::Center, Align::Far],
        |a| match a {
            Align::Near => "Top",
            Align::Center => "Middle",
            Align::Far => "Bottom",
        },
        style.align_vertical.index(),
    );
    vertical.set_tooltip_text(Some("Vertical Alignment"));

    let row1 = gtk::Box::builder().spacing(6).build();
    font.set_hexpand(true);
    row1.append(&font);
    row1.append(&size);
    row1.append(&text_color);
    let row2 = gtk::Box::builder().spacing(6).build();
    row2.append(&styles);
    row2.append(&align);
    row2.append(&vertical);

    let input = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    input.buffer().set_text(text);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&input)
        .min_content_height(140)
        .css_classes(["card"])
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(6)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    content.append(&row1);
    content.append(&row2);
    content.append(&scroller);
    content.append(
        &gtk::Label::builder()
            .label("Enter to finish, Shift+Enter for a new line")
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .build(),
    );

    let cancel = gtk::Button::with_label("Cancel");
    let ok = gtk::Button::builder()
        .label("OK")
        .css_classes(["suggested-action"])
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&ok);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));
    dialog.set_child(Some(&view));

    let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let accept = {
        let done = Rc::clone(&done);
        let dialog = dialog.clone();
        let input = input.clone();
        move || {
            let buffer = input.buffer();
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            let family = if picked.get() {
                font.font_desc()
                    .and_then(|f| f.family().map(|f| f.to_string()))
                    .unwrap_or_else(|| original.clone())
            } else {
                original.clone()
            };
            let style = TextStyle {
                font: family,
                size: size.value() as i32,
                color: color(&text_color.rgba()),
                bold: bold.is_active(),
                italic: italic.is_active(),
                underline: underline.is_active(),
                align: Align::from_index(align.selected()),
                align_vertical: Align::from_index(vertical.selected()),
            };
            if let Some(done) = done.borrow_mut().take() {
                done(Some((text, style)));
            }
            dialog.close();
        }
    };
    let accept = Rc::new(accept);
    ok.connect_clicked({
        let accept = Rc::clone(&accept);
        move |_| accept()
    });
    cancel.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let accept = Rc::clone(&accept);
        move |_, key, _, state| {
            let enter = key == gdk::Key::Return || key == gdk::Key::KP_Enter;
            if enter
                && !state
                    .intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK)
            {
                accept();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    input.add_controller(keys);
    dialog.connect_closed(move |_| {
        if let Some(done) = done.borrow_mut().take() {
            done(None);
        }
    });
    dialog.present(Some(parent));
    input.grab_focus();
    let buffer = input.buffer();
    buffer.select_range(&buffer.start_iter(), &buffer.end_iter());
}

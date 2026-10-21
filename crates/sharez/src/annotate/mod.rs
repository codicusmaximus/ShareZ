//! The annotator, a port of ShareX's image editor (`ShareX.ScreenCaptureLib`, the region capture
//! form in its task editor mode): the same tools, keys and toolbar, rewritten for GTK.
//!
//! `editor` holds the state and input rules, `shapes` the shapes, `render` draws them, and this
//! module is the window around them.

// Drawing code reads best with short coordinate names, and the toolbar and drawing functions
// follow ShareX's long ones.
#![allow(clippy::many_single_char_names, clippy::too_many_lines)]

mod editor;
mod effects;
pub mod icons;
mod prefs;
mod render;
mod shapes;
mod text;
mod text_input;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    rc::{Rc, Weak},
};

use adw::prelude::*;
use gtk::{cairo, gdk, gio, glib};
use sharez_core::Frame;

use crate::output;
use editor::{CropButton, Editor, Key, Mods, Order, Request, RightClick, Transform};
use shapes::{ArrowHead, BorderStyle, Category, Kind, Options, Point, Rect, StepType, Tool};

/// How the annotator was closed, as ShareX's task editor results.
#[derive(Debug)]
pub enum Outcome {
    /// Enter or the tick: the image with the annotations.
    Apply(Frame),
    /// Space, the continue button or a right click on nothing: the image without them, though
    /// with any crop or rotation.
    Continue(Frame),
    /// Escape or closing the window: the screenshot is dropped.
    Cancel,
}

/// ShareX's zoom limits.
const ZOOM_MIN: f64 = 0.2;
const ZOOM_MAX: f64 = 6.0;
/// The handles' size on screen.
const NODE_SIZE: f64 = 13.0;

type Done = Box<dyn FnOnce(Outcome)>;

/// An effect's rendered preview, kept until the effect or the canvas changes.
struct Preview {
    rect: Rect,
    kind: Kind,
    canvas: u64,
    at: (u32, u32),
    surface: Option<cairo::ImageSurface>,
}

/// The tool settings on the toolbar, each shown only for the tools that use it.
struct Bar {
    border_color: gtk::ColorDialogButton,
    fill_color: gtk::ColorDialogButton,
    highlight_color: gtk::ColorDialogButton,
    options: gtk::MenuButton,
    border_size: (gtk::Box, gtk::SpinButton),
    corner_radius: (gtk::Box, gtk::SpinButton),
    border_style: (gtk::Box, gtk::DropDown),
    center_points: (gtk::Box, gtk::SpinButton),
    arrow_head: (gtk::Box, gtk::DropDown),
    step_font_size: (gtk::Box, gtk::SpinButton),
    starting_step: (gtk::Box, gtk::SpinButton),
    step_type: (gtk::Box, gtk::DropDown),
    magnify_strength: (gtk::Box, gtk::SpinButton),
    blur_radius: (gtk::Box, gtk::SpinButton),
    pixelate_size: (gtk::Box, gtk::SpinButton),
    shadow: (gtk::Box, gtk::Switch),
    shadow_color: (gtk::Box, gtk::ColorDialogButton),
}

struct Annotator {
    window: adw::Window,
    toasts: adw::ToastOverlay,
    title: adw::WindowTitle,
    area: gtk::DrawingArea,
    scroller: gtk::ScrolledWindow,
    editor: RefCell<Editor>,
    /// 1 is one image pixel to one screen pixel.
    zoom: Cell<f64>,
    canvas: RefCell<(u64, cairo::ImageSurface)>,
    previews: RefCell<HashMap<u64, Preview>>,
    tools: RefCell<Vec<(Tool, gtk::ToggleButton)>>,
    bar: RefCell<Option<Bar>>,
    actions: gio::SimpleActionGroup,
    /// Set while the toolbar is being filled in from the editor, so it doesn't write back.
    syncing: Cell<bool>,
    done: RefCell<Option<Done>>,
    /// Middle-drag panning: the scroll position when the drag began.
    pan: Cell<Option<(f64, f64)>>,
    /// Itself, for callbacks that outlive a borrow of it.
    me: RefCell<Weak<Annotator>>,
}

/// Opens the annotator on `image`. `done` gets the result when it closes.
pub fn open(app: &adw::Application, image: Frame, done: impl FnOnce(Outcome) + 'static) {
    icons::install();
    let (options, tool) = prefs::load();
    let editor = Editor::new(image, options, tool);
    let surface = render::surface(editor.canvas());
    let window = adw::Window::builder()
        .application(app)
        .title("ShareZ Annotator")
        .build();
    let area = gtk::DrawingArea::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .focusable(true)
        .cursor(&gdk::Cursor::from_name("crosshair", None).expect("cursor"))
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&area)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["sharez-annotator-canvas"])
        .build();
    let me = Rc::new(Annotator {
        window: window.clone(),
        toasts: adw::ToastOverlay::new(),
        title: adw::WindowTitle::new("ShareZ Annotator", ""),
        area,
        scroller,
        editor: RefCell::new(editor),
        zoom: Cell::new(1.0),
        canvas: RefCell::new((0, surface)),
        previews: RefCell::new(HashMap::new()),
        tools: RefCell::new(Vec::new()),
        bar: RefCell::new(None),
        actions: gio::SimpleActionGroup::new(),
        syncing: Cell::new(false),
        done: RefCell::new(Some(Box::new(done))),
        pan: Cell::new(None),
        me: RefCell::new(Weak::new()),
    });
    *me.me.borrow_mut() = Rc::downgrade(&me);
    style();
    me.build();
    me.size_window();
    window.present();
    me.area.grab_focus();
    me.refresh();
    // The window keeps the annotator alive until it is destroyed.
    let keep = RefCell::new(Some(Rc::clone(&me)));
    me.window.connect_destroy(move |_| {
        keep.borrow_mut().take();
    });
}

/// The grey around the canvas, as ShareX's editor has.
fn style() {
    let css = gtk::CssProvider::new();
    css.load_from_string(
        ".sharez-annotator-canvas { background-color: alpha(@window_fg_color, 0.12); }",
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn spin(min: f64, max: f64, step: f64) -> gtk::SpinButton {
    let s = gtk::SpinButton::with_range(min, max, step);
    s.set_valign(gtk::Align::Center);
    s
}

/// A labelled line of the tool options popover.
fn line<W: IsA<gtk::Widget>>(label: &str, widget: W) -> (gtk::Box, W) {
    let b = gtk::Box::builder().spacing(12).build();
    b.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    b.append(&widget);
    (b, widget)
}

fn menu_item(menu: &gio::Menu, label: &str, action: &str, accel: Option<&str>) {
    let item = gio::MenuItem::new(Some(label), Some(&format!("annotate.{action}")));
    if let Some(accel) = accel {
        item.set_attribute_value("accel", Some(&accel.to_variant()));
    }
    menu.append_item(&item);
}

impl Annotator {
    fn weak(self: &Rc<Self>) -> Weak<Self> {
        Rc::downgrade(self)
    }

    /// Screen pixels per image pixel, in GTK's logical pixels.
    fn scale(&self) -> f64 {
        self.zoom.get() / f64::from(self.window.scale_factor().max(1))
    }

    fn to_image(&self, x: f64, y: f64) -> Point {
        let k = self.scale();
        Point::new((x / k).floor(), (y / k).floor())
    }

    fn build(self: &Rc<Self>) {
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&self.title));
        let button = |icon: &str, tip: &str, action: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tip)
                .action_name(format!("annotate.{action}"))
                .focus_on_click(false)
                .build()
        };
        header.pack_start(&button("window-close-symbolic", "Cancel (Esc)", "cancel"));
        let apply = gtk::Button::builder()
            .label("Done")
            .tooltip_text("Apply Changes and Continue (Enter)")
            .action_name("annotate.apply")
            .css_classes(["suggested-action"])
            .focus_on_click(false)
            .build();
        header.pack_end(&apply);
        header.pack_end(&button(
            "media-playback-start-symbolic",
            "Continue Without Changes (Space)",
            "continue",
        ));
        header.pack_end(&gtk::Separator::new(gtk::Orientation::Vertical));
        header.pack_end(&button("edit-copy-symbolic", "Copy Image (Ctrl+C)", "copy"));
        header.pack_end(&button(
            "document-save-as-symbolic",
            "Save Image As… (Ctrl+Shift+S)",
            "save-as",
        ));
        header.pack_end(&button(
            "document-save-symbolic",
            "Save Image (Ctrl+S)",
            "save",
        ));

        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.add_top_bar(&self.toolbar());
        self.toasts.set_child(Some(&self.scroller));
        view.set_content(Some(&self.toasts));
        self.window.set_content(Some(&view));
        self.window
            .insert_action_group("annotate", Some(&self.actions));
        self.actions();
        self.input();

        let weak = self.weak();
        self.area.set_draw_func(move |_, cr, _, _| {
            if let Some(me) = weak.upgrade() {
                me.draw(cr);
            }
        });
        let weak = self.weak();
        self.window.connect_close_request(move |_| {
            if let Some(me) = weak.upgrade() {
                me.cancel();
            }
            glib::Propagation::Stop
        });
        let weak = self.weak();
        self.window.connect_scale_factor_notify(move |_| {
            if let Some(me) = weak.upgrade() {
                me.refresh();
            }
        });
    }

    /// The tools, then their settings, then the Edit and Image menus, in ShareX's order.
    fn toolbar(self: &Rc<Self>) -> gtk::ScrolledWindow {
        let bar = gtk::Box::builder()
            .spacing(2)
            .margin_start(6)
            .margin_end(6)
            .margin_top(2)
            .margin_bottom(4)
            .build();
        let mut group: Option<gtk::ToggleButton> = None;
        let mut tools = Vec::new();
        for tool in Tool::ALL {
            let tip = match tool.key() {
                Some(k) => format!("{} ({})", tool.title(), k.to_ascii_uppercase()),
                None => tool.title().to_string(),
            };
            let b = gtk::ToggleButton::builder()
                .icon_name(icons::tool(tool))
                .tooltip_text(tip)
                .css_classes(["flat"])
                .focus_on_click(false)
                .build();
            if let Some(g) = &group {
                b.set_group(Some(g));
            } else {
                group = Some(b.clone());
            }
            let weak = self.weak();
            b.connect_toggled(move |b| {
                let Some(me) = weak.upgrade() else { return };
                if b.is_active() && !me.syncing.get() {
                    me.editor.borrow_mut().set_tool(tool);
                    me.refresh();
                }
            });
            bar.append(&b);
            tools.push((tool, b));
        }
        *self.tools.borrow_mut() = tools;
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));

        let o = self.editor.borrow().options.clone();
        let border_color = text_input::color_button(o.border_color, true);
        border_color.set_tooltip_text(Some("Border Color"));
        let fill_color = text_input::color_button(o.fill_color, true);
        fill_color.set_tooltip_text(Some("Fill Color"));
        let highlight_color = text_input::color_button(o.highlight_color, false);
        highlight_color.set_tooltip_text(Some("Highlight Color"));
        for b in [&border_color, &fill_color, &highlight_color] {
            bar.append(b);
        }
        self.on_color(&border_color, shapes::Options::set_border_color);
        self.on_color(&fill_color, shapes::Options::set_fill_color);
        self.on_color(&highlight_color, |o, _, c| o.highlight_color = c);

        let b = Bar {
            border_color,
            fill_color,
            highlight_color,
            options: gtk::MenuButton::builder()
                .icon_name(icons::name("tool-options"))
                .tooltip_text("Tool Options")
                .css_classes(["flat"])
                .build(),
            magnify_strength: line("Magnify Strength", spin(100.0, 1000.0, 100.0)),
            border_size: line("Border Size", spin(0.0, 20.0, 1.0)),
            corner_radius: line("Corner Radius", spin(0.0, 150.0, 1.0)),
            border_style: line(
                "Border Style",
                text_input::choice(BorderStyle::ALL, BorderStyle::title, 0),
            ),
            blur_radius: line("Blur Radius", spin(3.0, 199.0, 2.0)),
            pixelate_size: line("Pixel Size", spin(2.0, 10000.0, 1.0)),
            center_points: line(
                "Center Points",
                spin(0.0, f64::from(shapes::MAXIMUM_CENTER_POINTS), 1.0),
            ),
            arrow_head: line(
                "Arrow Head Direction",
                text_input::choice(ArrowHead::ALL, ArrowHead::title, 0),
            ),
            step_font_size: line("Font Size", spin(10.0, 300.0, 1.0)),
            starting_step: line("Starting Step Value", spin(0.0, 10000.0, 1.0)),
            step_type: line(
                "Step Type",
                text_input::choice(StepType::ALL, StepType::title, 0),
            ),
            shadow: line(
                "Drop Shadow",
                gtk::Switch::builder().valign(gtk::Align::Center).build(),
            ),
            shadow_color: line(
                "Drop Shadow Color",
                text_input::color_button(o.shadow_color, true),
            ),
        };
        let popover_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .width_request(300)
            .build();
        for row in [
            &b.magnify_strength.0,
            &b.border_size.0,
            &b.corner_radius.0,
            &b.border_style.0,
            &b.blur_radius.0,
            &b.pixelate_size.0,
            &b.center_points.0,
            &b.arrow_head.0,
            &b.step_font_size.0,
            &b.starting_step.0,
            &b.step_type.0,
            &b.shadow.0,
            &b.shadow_color.0,
        ] {
            popover_box.append(row);
        }
        b.options
            .set_popover(Some(&gtk::Popover::builder().child(&popover_box).build()));
        bar.append(&b.options);

        self.on_spin(&b.magnify_strength.1, |o, _, v| o.magnify_strength = v);
        self.on_spin(&b.border_size.1, shapes::Options::set_border_size);
        self.on_spin(&b.corner_radius.1, |o, _, v| o.corner_radius = v);
        self.on_spin(&b.blur_radius.1, |o, _, v| o.blur_radius = v);
        self.on_spin(&b.pixelate_size.1, |o, _, v| o.pixelate_size = v);
        self.on_spin(&b.center_points.1, |o, _, v| o.line_center_points = v);
        self.on_spin(&b.step_font_size.1, |o, _, v| o.step_font_size = v);
        self.on_drop(&b.border_style.1, |o, i| {
            o.border_style = BorderStyle::from_index(i);
        });
        self.on_drop(&b.arrow_head.1, |o, i| {
            o.arrow_head = ArrowHead::from_index(i);
        });
        self.on_drop(&b.step_type.1, |o, i| o.step_type = StepType::from_index(i));
        self.on_color(&b.shadow_color.1, |o, _, c| o.shadow_color = c);
        let weak = self.weak();
        b.starting_step.1.connect_value_changed(move |s| {
            let Some(me) = weak.upgrade() else { return };
            if !me.syncing.get() {
                me.editor.borrow_mut().set_starting_step(s.value() as i32);
                me.refresh();
            }
        });
        let weak = self.weak();
        b.shadow.1.connect_active_notify(move |s| {
            let Some(me) = weak.upgrade() else { return };
            if !me.syncing.get() {
                me.editor.borrow_mut().options.shadow = s.is_active();
                me.editor.borrow_mut().update_current();
                me.refresh();
            }
        });
        *self.bar.borrow_mut() = Some(b);

        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let edit = gio::Menu::new();
        let section = gio::Menu::new();
        menu_item(&section, "Undo", "undo", Some("<Control>z"));
        menu_item(&section, "Redo", "redo", Some("<Control>y"));
        menu_item(&section, "Paste Text", "paste", Some("<Control>v"));
        menu_item(&section, "Duplicate", "duplicate", Some("<Control>d"));
        edit.append_section(None, &section);
        let section = gio::Menu::new();
        menu_item(&section, "Delete", "delete", Some("Delete"));
        menu_item(&section, "Delete All", "delete-all", Some("<Shift>Delete"));
        edit.append_section(None, &section);
        let section = gio::Menu::new();
        menu_item(&section, "Bring to Front", "top", Some("Home"));
        menu_item(&section, "Bring Forward", "up", Some("Page_Up"));
        menu_item(&section, "Send Backward", "down", Some("Page_Down"));
        menu_item(&section, "Send to Back", "bottom", Some("End"));
        edit.append_section(None, &section);
        bar.append(
            &gtk::MenuButton::builder()
                .label("Edit")
                .menu_model(&edit)
                .css_classes(["flat"])
                .build(),
        );
        let image = gio::Menu::new();
        let section = gio::Menu::new();
        menu_item(&section, "Crop Image", "crop", None);
        image.append_section(None, &section);
        let section = gio::Menu::new();
        menu_item(&section, "Rotate 90° Clockwise", "rotate-cw", None);
        menu_item(&section, "Rotate 90° Counter Clockwise", "rotate-ccw", None);
        menu_item(&section, "Rotate 180°", "rotate-180", None);
        image.append_section(None, &section);
        let section = gio::Menu::new();
        menu_item(&section, "Flip Horizontal", "flip-h", None);
        menu_item(&section, "Flip Vertical", "flip-v", None);
        image.append_section(None, &section);
        let section = gio::Menu::new();
        menu_item(&section, "Zoom In", "zoom-in", Some("<Control>plus"));
        menu_item(&section, "Zoom Out", "zoom-out", Some("<Control>minus"));
        menu_item(&section, "Actual Size", "zoom-actual", Some("<Control>0"));
        menu_item(&section, "Zoom to Fit", "zoom-fit", Some("<Control><Alt>0"));
        image.append_section(None, &section);
        bar.append(
            &gtk::MenuButton::builder()
                .label("Image")
                .menu_model(&image)
                .css_classes(["flat"])
                .build(),
        );
        gtk::ScrolledWindow::builder()
            .child(&bar)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .build()
    }

    /// A toolbar setting the selected shape takes too, as `UpdateCurrentShape`.
    fn changed(&self, f: impl FnOnce(&mut Options, Tool)) {
        if self.syncing.get() {
            return;
        }
        {
            let mut e = self.editor.borrow_mut();
            let tool = e.options_tool();
            f(&mut e.options, tool);
            e.update_current();
        }
        self.refresh();
    }

    fn on_color(
        self: &Rc<Self>,
        b: &gtk::ColorDialogButton,
        f: fn(&mut Options, Tool, shapes::Color),
    ) {
        let weak = self.weak();
        b.connect_rgba_notify(move |b| {
            if let Some(me) = weak.upgrade() {
                let c = text_input::color(&b.rgba());
                me.changed(|o, t| f(o, t, c));
            }
        });
    }

    fn on_spin(self: &Rc<Self>, s: &gtk::SpinButton, f: fn(&mut Options, Tool, i32)) {
        let weak = self.weak();
        s.connect_value_changed(move |s| {
            if let Some(me) = weak.upgrade() {
                let v = s.value() as i32;
                me.changed(|o, t| f(o, t, v));
            }
        });
    }

    fn on_drop(self: &Rc<Self>, d: &gtk::DropDown, f: fn(&mut Options, u32)) {
        let weak = self.weak();
        d.connect_selected_notify(move |d| {
            if let Some(me) = weak.upgrade() {
                let i = d.selected();
                me.changed(|o, _| f(o, i));
            }
        });
    }

    fn actions(self: &Rc<Self>) {
        let add = |name: &str, f: fn(&Rc<Annotator>)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.weak();
            action.connect_activate(move |_, _| {
                if let Some(me) = weak.upgrade() {
                    f(&me);
                    me.refresh();
                }
            });
            self.actions.add_action(&action);
        };
        add("apply", |me| me.apply());
        add("continue", |me| me.finish_continue());
        add("cancel", |me| me.cancel());
        add("save", |me| me.save());
        add("save-as", |me| me.save_as());
        add("copy", |me| me.copy());
        add("undo", |me| me.editor.borrow_mut().undo());
        add("redo", |me| me.editor.borrow_mut().redo());
        add("paste", |me| me.paste());
        add("duplicate", |me| me.editor.borrow_mut().duplicate());
        add("delete", |me| me.editor.borrow_mut().delete_current());
        add("delete-all", |me| me.editor.borrow_mut().delete_all());
        add("top", |me| me.editor.borrow_mut().move_current(Order::Top));
        add("up", |me| me.editor.borrow_mut().move_current(Order::Up));
        add("down", |me| {
            me.editor.borrow_mut().move_current(Order::Down);
        });
        add("bottom", |me| {
            me.editor.borrow_mut().move_current(Order::Bottom);
        });
        add("crop", |me| me.editor.borrow_mut().crop_whole());
        add("rotate-cw", |me| {
            me.editor.borrow_mut().transform(Transform::RotateClockwise);
        });
        add("rotate-ccw", |me| {
            me.editor
                .borrow_mut()
                .transform(Transform::RotateCounterClockwise);
        });
        add("rotate-180", |me| {
            me.editor.borrow_mut().transform(Transform::Rotate180);
        });
        add("flip-h", |me| {
            me.editor.borrow_mut().transform(Transform::FlipHorizontal);
        });
        add("flip-v", |me| {
            me.editor.borrow_mut().transform(Transform::FlipVertical);
        });
        add("zoom-in", |me| me.zoom_step(true, None));
        add("zoom-out", |me| me.zoom_step(false, None));
        add("zoom-actual", |me| me.set_zoom(1.0, None));
        add("zoom-fit", Annotator::zoom_fit);
    }

    fn action(&self, name: &str) {
        self.actions.activate_action(name, None);
    }

    // Input.

    fn input(self: &Rc<Self>) {
        let click = gtk::GestureClick::builder().button(0).build();
        let weak = self.weak();
        click.connect_pressed(move |g, n, x, y| {
            let Some(me) = weak.upgrade() else { return };
            me.area.grab_focus();
            let p = me.to_image(x, y);
            if g.current_button() == gdk::BUTTON_PRIMARY {
                let request = me.editor.borrow_mut().press(p);
                me.request(request);
                if n == 2 {
                    let request = me.editor.borrow_mut().double_click(p);
                    me.request(request);
                }
                me.refresh();
            }
        });
        let weak = self.weak();
        click.connect_released(move |g, _, x, y| {
            let Some(me) = weak.upgrade() else { return };
            let p = me.to_image(x, y);
            match g.current_button() {
                gdk::BUTTON_PRIMARY => me.editor.borrow_mut().release(p),
                gdk::BUTTON_SECONDARY => {
                    let r = me.editor.borrow_mut().right_click(p);
                    if r == RightClick::Continue {
                        me.confirm_exit(Annotator::finish_continue);
                    }
                }
                _ => {}
            }
            me.refresh();
        });
        self.area.add_controller(click);

        let motion = gtk::EventControllerMotion::new();
        let weak = self.weak();
        motion.connect_motion(move |_, x, y| {
            let Some(me) = weak.upgrade() else { return };
            let p = me.to_image(x, y);
            me.editor.borrow_mut().motion(p);
            me.update_cursor(p);
            me.area.queue_draw();
        });
        self.area.add_controller(motion);

        // Middle-drag pans, on the scroller, which doesn't move under the pointer.
        let pan = gtk::GestureDrag::builder()
            .button(gdk::BUTTON_MIDDLE)
            .build();
        let weak = self.weak();
        pan.connect_drag_begin(move |_, _, _| {
            if let Some(me) = weak.upgrade() {
                let (h, v) = (me.scroller.hadjustment(), me.scroller.vadjustment());
                me.pan.set(Some((h.value(), v.value())));
            }
        });
        let weak = self.weak();
        pan.connect_drag_update(move |_, dx, dy| {
            let Some(me) = weak.upgrade() else { return };
            if let Some((x, y)) = me.pan.get() {
                me.scroller.hadjustment().set_value(x - dx);
                me.scroller.vadjustment().set_value(y - dy);
            }
        });
        let weak = self.weak();
        pan.connect_drag_end(move |_, _, _| {
            if let Some(me) = weak.upgrade() {
                me.pan.set(None);
            }
        });
        self.scroller.add_controller(pan);

        // Ctrl+wheel zooms at the pointer.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.weak();
        scroll.connect_scroll(move |c, _, dy| {
            let Some(me) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !c
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
                || dy == 0.0
            {
                return glib::Propagation::Proceed;
            }
            let at = c
                .current_event()
                .and_then(|e| e.position())
                .and_then(|(x, y)| {
                    let root = me.window.clone().upcast::<gtk::Widget>();
                    root.compute_point(&me.scroller, &gtk::graphene::Point::new(x as f32, y as f32))
                })
                .map(|p| (f64::from(p.x()), f64::from(p.y())));
            me.zoom_step(dy < 0.0, at);
            glib::Propagation::Stop
        });
        self.scroller.add_controller(scroll);

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.weak();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(me) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            me.key(key, state, true)
        });
        let weak = self.weak();
        keys.connect_key_released(move |_, key, _, state| {
            if let Some(me) = weak.upgrade() {
                me.key(key, state, false);
            }
        });
        keys.connect_modifiers(|_, _| glib::Propagation::Proceed);
        self.window.add_controller(keys);
    }

    /// Whether keys belong to something else: a dialog, a popover or a text field.
    fn keys_elsewhere(&self) -> bool {
        let Some(focus) = gtk::prelude::GtkWindowExt::focus(&self.window) else {
            return false;
        };
        focus.is::<gtk::Text>()
            || focus.is::<gtk::TextView>()
            || focus.ancestor(adw::Dialog::static_type()).is_some()
            || focus.ancestor(gtk::Popover::static_type()).is_some()
    }

    fn key(&self, key: gdk::Key, state: gdk::ModifierType, pressed: bool) -> glib::Propagation {
        let is_ctrl = matches!(key, gdk::Key::Control_L | gdk::Key::Control_R);
        let is_shift = matches!(key, gdk::Key::Shift_L | gdk::Key::Shift_R);
        // The state is from before the event, so a modifier's own key decides whether it's held.
        let held = |mask, own: bool| if own { pressed } else { state.contains(mask) };
        let ctrl = held(gdk::ModifierType::CONTROL_MASK, is_ctrl);
        let shift = held(gdk::ModifierType::SHIFT_MASK, is_shift);
        self.editor.borrow_mut().set_modifiers(ctrl, shift);
        if is_ctrl {
            self.area.queue_draw();
        }
        if !pressed || self.keys_elsewhere() {
            return glib::Propagation::Proceed;
        }
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let mods = Mods { ctrl, shift, alt };
        let lower = key.to_lower();
        let stop = glib::Propagation::Stop;
        match (lower, ctrl, shift, alt) {
            (gdk::Key::Escape, false, false, false) => {
                if !self.editor.borrow_mut().escape() {
                    self.action("cancel");
                }
                self.refresh();
                return stop;
            }
            (gdk::Key::Return | gdk::Key::KP_Enter, false, false, false) => {
                // A crop waiting for its tick is confirmed first.
                if self.editor.borrow().has_crop() {
                    self.editor.borrow_mut().confirm_crop();
                    self.refresh();
                } else {
                    self.action("apply");
                }
                return stop;
            }
            (gdk::Key::space, false, false, false) => {
                self.action("continue");
                return stop;
            }
            (gdk::Key::s, true, false, false) => {
                self.action("save");
                return stop;
            }
            (gdk::Key::s, true, true, false) => {
                self.action("save-as");
                return stop;
            }
            (gdk::Key::c, true, false, false) => {
                self.action("copy");
                return stop;
            }
            (gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add, true, _, false) => {
                self.action("zoom-in");
                return stop;
            }
            (gdk::Key::minus | gdk::Key::KP_Subtract, true, false, false) => {
                self.action("zoom-out");
                return stop;
            }
            (gdk::Key::_0 | gdk::Key::KP_0 | gdk::Key::KP_Insert, true, false, true) => {
                self.action("zoom-fit");
                return stop;
            }
            (gdk::Key::_0 | gdk::Key::KP_0 | gdk::Key::KP_Insert, true, false, false) => {
                self.action("zoom-actual");
                return stop;
            }
            _ => {}
        }
        let k = match lower {
            gdk::Key::Delete | gdk::Key::KP_Delete => Key::Delete,
            gdk::Key::Home => Key::Home,
            gdk::Key::End => Key::End,
            gdk::Key::Page_Up => Key::PageUp,
            gdk::Key::Page_Down => Key::PageDown,
            gdk::Key::Left => Key::Left,
            gdk::Key::Right => Key::Right,
            gdk::Key::Up => Key::Up,
            gdk::Key::Down => Key::Down,
            gdk::Key::KP_1 => Key::Numpad(1),
            gdk::Key::KP_2 => Key::Numpad(2),
            gdk::Key::KP_3 => Key::Numpad(3),
            gdk::Key::KP_4 => Key::Numpad(4),
            gdk::Key::KP_5 => Key::Numpad(5),
            gdk::Key::KP_6 => Key::Numpad(6),
            gdk::Key::KP_7 => Key::Numpad(7),
            gdk::Key::KP_8 => Key::Numpad(8),
            gdk::Key::KP_9 => Key::Numpad(9),
            k => match k.to_unicode() {
                Some(c) if c.is_ascii_lowercase() => Key::Letter(c),
                _ => return glib::Propagation::Proceed,
            },
        };
        let (handled, request) = self.editor.borrow_mut().key(k, mods);
        self.request(request);
        if handled {
            self.refresh();
            stop
        } else {
            glib::Propagation::Proceed
        }
    }

    fn request(&self, request: Option<Request>) {
        match request {
            Some(Request::EditText { id, new }) => self.edit_text(id, new),
            Some(Request::Paste) => self.paste(),
            None => {}
        }
    }

    fn edit_text(&self, id: u64, new: bool) {
        let Some((text, style)) = self
            .editor
            .borrow()
            .shape(id)
            .and_then(|s| s.text().map(|(t, s)| (t.to_string(), s.clone())))
        else {
            return;
        };
        let editor = self.window.clone();
        let me = self.self_rc();
        text_input::ask(&editor, &text, &style, move |answer| {
            if let Some(me) = me.upgrade() {
                me.editor.borrow_mut().finish_text(id, new, answer);
                me.refresh();
                me.area.grab_focus();
            }
        });
    }

    fn self_rc(&self) -> Weak<Self> {
        self.me.borrow().clone()
    }

    /// Ctrl+V: text on the clipboard becomes a text shape.
    fn paste(&self) {
        let clipboard = self.window.clipboard();
        let me = self.self_rc();
        clipboard.read_text_async(None::<&gio::Cancellable>, move |text| {
            let Some(me) = me.upgrade() else { return };
            if let Ok(Some(text)) = text {
                me.editor.borrow_mut().paste_text(&text);
                me.refresh();
            } else {
                me.toasts.add_toast(adw::Toast::new(
                    "Only text can be pasted into the annotator",
                ));
            }
        });
    }

    fn update_cursor(&self, p: Point) {
        let e = self.editor.borrow();
        let name = if e.is_on_control(p) {
            "pointer"
        } else if e.hover().is_some() {
            "move"
        } else {
            "crosshair"
        };
        self.area.set_cursor_from_name(Some(name));
    }

    // Zoom.

    fn set_zoom(&self, zoom: f64, at: Option<(f64, f64)>) {
        let zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let (h, v) = (self.scroller.hadjustment(), self.scroller.vadjustment());
        // Keep the image point under the pointer, or the middle of the view, where it is.
        let (vx, vy) = at.unwrap_or((h.page_size() / 2.0, v.page_size() / 2.0));
        let old = self.scale();
        let content = |adj: &gtk::Adjustment, view: f64, size: f64| {
            // Where the area starts within the scroller, when it is centred and smaller.
            let offset = ((adj.page_size() - size) / 2.0).max(0.0);
            (adj.value() + view - offset) / old
        };
        let (w, h_px) = (f64::from(self.area.width()), f64::from(self.area.height()));
        let ix = content(&h, vx, w);
        let iy = content(&v, vy, h_px);
        self.zoom.set(zoom);
        self.resize_area();
        let new = self.scale();
        let weak_h = h.clone();
        let weak_v = v.clone();
        glib::idle_add_local_once(move || {
            weak_h.set_value(ix * new - vx);
            weak_v.set_value(iy * new - vy);
        });
        self.refresh();
    }

    /// ShareX's zoom steps: a tenth below 100 %, a quarter to 200 %, a half above.
    fn zoom_step(&self, zoom_in: bool, at: Option<(f64, f64)>) {
        let z = self.zoom.get();
        let next = if zoom_in {
            z + if z >= 2.0 {
                0.5
            } else if z >= 1.0 {
                0.25
            } else {
                0.1
            }
        } else {
            z - if z <= 1.0 {
                0.1
            } else if z <= 2.0 {
                0.25
            } else {
                0.5
            }
        };
        self.set_zoom((next * 100.0).round() / 100.0, at);
    }

    fn zoom_fit(self: &Rc<Self>) {
        let e = self.editor.borrow();
        let (w, h) = (f64::from(e.canvas().width), f64::from(e.canvas().height));
        drop(e);
        let (pw, ph) = (
            f64::from(self.scroller.width().max(1)),
            f64::from(self.scroller.height().max(1)),
        );
        let s = f64::from(self.window.scale_factor().max(1));
        self.set_zoom((pw / w).min(ph / h) * s, None);
    }

    fn resize_area(&self) {
        let e = self.editor.borrow();
        let k = self.scale();
        self.area
            .set_content_width((f64::from(e.canvas().width) * k).ceil() as i32);
        self.area
            .set_content_height((f64::from(e.canvas().height) * k).ceil() as i32);
    }

    /// Sizes the window to the image, as ShareX's auto size start mode, zoomed out if it doesn't
    /// fit on the screen.
    fn size_window(self: &Rc<Self>) {
        let (w, h) = {
            let e = self.editor.borrow();
            (f64::from(e.canvas().width), f64::from(e.canvas().height))
        };
        let monitor = gdk::Display::default()
            .and_then(|d| d.monitors().item(0))
            .and_then(|m| m.downcast::<gdk::Monitor>().ok())
            .map(|m| m.geometry());
        let (sw, sh) = monitor.map_or((1600.0, 900.0), |g| {
            (f64::from(g.width()), f64::from(g.height()))
        });
        let s = monitor
            .and_then(|_| gdk::Display::default())
            .and_then(|d| d.monitors().item(0))
            .and_then(|m| m.downcast::<gdk::Monitor>().ok())
            .map_or(1.0, |m| f64::from(m.scale_factor()));
        let chrome = (40.0, 140.0);
        let (max_w, max_h) = (sw * 0.9 - chrome.0, sh * 0.85 - chrome.1);
        let fit = (max_w / (w / s)).min(max_h / (h / s)).min(1.0);
        self.zoom.set(fit.max(ZOOM_MIN));
        let k = self.zoom.get() / s;
        self.window.set_default_size(
            ((w * k + chrome.0).max(800.0)) as i32,
            ((h * k + chrome.1).max(550.0)) as i32,
        );
        self.resize_area();
    }

    // Drawing.

    fn draw(&self, cr: &cairo::Context) {
        let e = self.editor.borrow();
        {
            let mut canvas = self.canvas.borrow_mut();
            if canvas.0 != e.canvas_version() {
                *canvas = (e.canvas_version(), render::surface(e.canvas()));
                self.previews.borrow_mut().clear();
            }
        }
        let canvas = self.canvas.borrow();
        let surface = &canvas.1;
        let k = self.scale();
        let (w, h) = (f64::from(e.canvas().width), f64::from(e.canvas().height));
        cr.save().ok();
        cr.scale(k, k);
        checkerboard(cr, w, h, k);
        cr.set_source_surface(surface, 0.0, 0.0).ok();
        cr.source().set_filter(if k >= 1.0 {
            cairo::Filter::Nearest
        } else {
            cairo::Filter::Good
        });
        cr.paint().ok();

        for s in e.shapes.iter().filter(|s| s.category() == Category::Effect) {
            if e.is_busy(s.id) {
                render::draw_effect_overlay(cr, s);
            } else {
                self.draw_preview(cr, s, &e);
            }
        }
        for s in e
            .shapes
            .iter()
            .filter(|s| s.category() == Category::Drawing)
        {
            render::draw(cr, s, surface);
        }
        for s in e.shapes.iter().filter(|s| s.kind == Kind::Crop) {
            if s.is_valid() {
                render::draw_crop(cr, s.rect(), k);
            }
        }
        if let Some((confirm, cancel)) = e.crop_buttons() {
            let hover = e.button_hover();
            render::draw_button(
                cr,
                confirm,
                "\u{2714}",
                shapes::Color::rgb(34, 139, 34),
                hover == Some(CropButton::Confirm),
            );
            render::draw_button(
                cr,
                cancel,
                "\u{2716}",
                shapes::Color::rgb(227, 45, 45),
                hover == Some(CropButton::Cancel),
            );
        }
        if let Some(s) = e.hover() {
            cr.save().ok();
            cr.translate(0.5, 0.5);
            let mut outline = s.clone();
            outline.set_rect(s.rect().inflate(-0.5));
            render::hover_path(cr, &outline);
            render::region_border(cr, k);
            cr.restore().ok();
        }
        cr.restore().ok();

        // Handles, the same size at any zoom.
        for (_, p) in e.nodes() {
            let (x, y) = ((p.x + 0.5) * k, (p.y + 0.5) * k);
            let r = (NODE_SIZE - 1.0) / 2.0;
            cr.set_line_width(1.0);
            cr.new_sub_path();
            cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.stroke().ok();
            cr.new_sub_path();
            cr.arc(x, y, r - 1.0, 0.0, std::f64::consts::TAU);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.stroke().ok();
        }
        if let Some(s) = e
            .shapes
            .iter()
            .find(|s| s.kind == Kind::Crop && s.is_valid())
        {
            size_label(cr, s.rect(), k);
        }
    }

    /// An effect's result, rendered once and kept until it moves or changes.
    fn draw_preview(&self, cr: &cairo::Context, s: &shapes::Shape, e: &Editor) {
        let mut previews = self.previews.borrow_mut();
        let stale = previews.get(&s.id).is_none_or(|p| {
            p.rect != s.rect() || p.kind != s.kind || p.canvas != e.canvas_version()
        });
        if stale {
            let rendered = render::apply_effect(e.canvas(), s);
            previews.insert(
                s.id,
                Preview {
                    rect: s.rect(),
                    kind: s.kind.clone(),
                    canvas: e.canvas_version(),
                    at: rendered.as_ref().map_or((0, 0), |(x, y, _)| (*x, *y)),
                    surface: rendered.map(|(_, _, f)| render::surface(&f)),
                },
            );
        }
        let Some(p) = previews.get(&s.id) else { return };
        if let Some(surface) = &p.surface {
            cr.save().ok();
            cr.set_source_surface(surface, f64::from(p.at.0), f64::from(p.at.1))
                .ok();
            cr.source().set_filter(cairo::Filter::Nearest);
            cr.paint().ok();
            cr.restore().ok();
        }
    }

    /// Puts the toolbar, title and canvas size in step with the editor, and redraws.
    fn refresh(&self) {
        self.resize_area();
        self.area.queue_draw();
        self.editor.borrow_mut().node_size = NODE_SIZE / self.scale();
        let e = self.editor.borrow();
        let zoom = (self.zoom.get() * 100.0).round();
        self.title.set_subtitle(&format!(
            "{} × {} · {zoom} %",
            e.canvas().width,
            e.canvas().height
        ));
        for (name, on) in [("undo", e.can_undo()), ("redo", e.can_redo())] {
            if let Some(a) = self
                .actions
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                a.set_enabled(on);
            }
        }
        self.syncing.set(true);
        for (tool, b) in self.tools.borrow().iter() {
            b.set_active(*tool == e.tool());
        }
        if let Some(b) = self.bar.borrow().as_ref() {
            sync_bar(b, &e);
        }
        self.syncing.set(false);
    }

    // Finishing.

    fn result(&self) -> Frame {
        let e = self.editor.borrow();
        render::render(e.canvas(), &e.shapes)
    }

    fn finish(&self, outcome: Outcome) {
        let Some(done) = self.done.borrow_mut().take() else {
            return;
        };
        {
            let e = self.editor.borrow();
            let tool = match e.tool() {
                Tool::Select | Tool::Crop => Tool::Rectangle,
                t => t,
            };
            prefs::save(&e.options, tool);
        }
        self.window.set_visible(false);
        done(outcome);
        self.window.destroy();
    }

    fn apply(&self) {
        let image = self.result();
        self.finish(Outcome::Apply(image));
    }

    fn finish_continue(&self) {
        let image = self.editor.borrow().canvas().clone();
        self.finish(Outcome::Continue(image));
    }

    fn cancel(&self) {
        self.confirm_exit(|me| me.finish(Outcome::Cancel));
    }

    /// ShareX's "save changes before closing?", when anything was drawn.
    fn confirm_exit(&self, then: fn(&Annotator)) {
        if !self.editor.borrow().is_modified() {
            then(self);
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some("Save Changes?"),
            Some("The annotations will be lost if you close without saving."),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("discard", "Don't Save");
        dialog.add_response("save", "Save");
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        let me = self.self_rc();
        dialog.connect_response(None, move |_, response| {
            let Some(me) = me.upgrade() else { return };
            match response {
                "save" => {
                    if me.save_to(None).is_some() {
                        then(&me);
                    }
                }
                "discard" => then(&me),
                _ => {}
            }
        });
        dialog.present(Some(&self.window));
    }

    fn save(&self) {
        self.save_to(None);
    }

    fn save_to(&self, path: Option<PathBuf>) -> Option<PathBuf> {
        let image = self.result();
        let saved = match path {
            Some(p) => output::save_image_to(p, &image),
            None => output::save_image(&image),
        };
        match saved {
            Ok(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.toasts
                    .add_toast(adw::Toast::new(&format!("Saved {name}")));
                Some(path)
            }
            Err(e) => {
                self.toasts.add_toast(adw::Toast::new(&e));
                None
            }
        }
    }

    fn save_as(&self) {
        let Ok(suggested) = output::new_file(glib::UserDirectory::Pictures, "png") else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title("Save Image")
            .modal(true)
            .initial_name(
                suggested
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
            .build();
        if let Some(dir) = suggested.parent() {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let me = self.self_rc();
        dialog.save(Some(&self.window), None::<&gio::Cancellable>, move |file| {
            let Some(me) = me.upgrade() else { return };
            if let Some(mut path) = file.ok().and_then(|f| f.path()) {
                if path.extension().is_none() {
                    path.set_extension("png");
                }
                me.save_to(Some(path));
            }
        });
    }

    fn copy(&self) {
        output::copy_frame(self.result());
        self.toasts
            .add_toast(adw::Toast::new("Copied to the clipboard"));
    }
}

/// Fills in the toolbar's settings for the tool, and shows only those it uses, as ShareX's
/// `UpdateMenu`.
fn sync_bar(b: &Bar, e: &Editor) {
    use Tool::{
        Arrow, Blur, Ellipse, Freehand, FreehandArrow, Highlight, Line, Magnify, Pixelate,
        Rectangle, SpeechBalloon, Step, TextBackground, TextOutline,
    };
    let t = e.options_tool();
    let o = &e.options;
    let border = matches!(
        t,
        Rectangle
            | Ellipse
            | Freehand
            | FreehandArrow
            | Line
            | Arrow
            | TextOutline
            | TextBackground
            | SpeechBalloon
            | Step
            | Magnify
    );
    let fill = matches!(
        t,
        Rectangle | Ellipse | TextBackground | SpeechBalloon | Step | Magnify
    );
    let options = border || matches!(t, Blur | Pixelate);
    b.border_color.set_visible(border);
    b.border_color
        .set_rgba(&text_input::rgba(o.border_color(t)));
    b.fill_color.set_visible(fill);
    b.fill_color.set_rgba(&text_input::rgba(o.fill_color(t)));
    b.highlight_color.set_visible(t == Highlight);
    b.highlight_color
        .set_rgba(&text_input::rgba(o.highlight_color));
    b.options.set_visible(options);

    let show = |row: &gtk::Box, on: bool| row.set_visible(on);
    show(&b.border_size.0, border);
    b.border_size.1.set_value(f64::from(o.border_size(t)));
    show(&b.corner_radius.0, matches!(t, Rectangle | TextBackground));
    b.corner_radius.1.set_value(f64::from(o.corner_radius));
    show(
        &b.border_style.0,
        matches!(
            t,
            Rectangle | Ellipse | Freehand | FreehandArrow | Line | Arrow | Magnify
        ),
    );
    b.border_style.1.set_selected(o.border_style.index());
    show(&b.center_points.0, matches!(t, Line | Arrow));
    b.center_points.1.set_value(f64::from(o.line_center_points));
    show(&b.arrow_head.0, matches!(t, Arrow | FreehandArrow));
    b.arrow_head.1.set_selected(o.arrow_head.index());
    for row in [&b.step_font_size.0, &b.starting_step.0, &b.step_type.0] {
        show(row, t == Step);
    }
    b.step_font_size.1.set_value(f64::from(o.step_font_size));
    b.starting_step.1.set_value(f64::from(e.starting_step));
    b.step_type.1.set_selected(o.step_type.index());
    show(&b.magnify_strength.0, t == Magnify);
    b.magnify_strength
        .1
        .set_value(f64::from(o.magnify_strength));
    show(&b.blur_radius.0, t == Blur);
    b.blur_radius.1.set_value(f64::from(o.blur_radius));
    show(&b.pixelate_size.0, t == Pixelate);
    b.pixelate_size.1.set_value(f64::from(o.pixelate_size));
    show(&b.shadow.0, border);
    b.shadow.1.set_active(o.shadow);
    show(&b.shadow_color.0, border);
    b.shadow_color.1.set_rgba(&text_input::rgba(o.shadow_color));
}

/// Grey and white squares under the image, where it is transparent.
fn checkerboard(cr: &cairo::Context, w: f64, h: f64, k: f64) {
    let size = 8.0 / k;
    cr.save().ok();
    cr.rectangle(0.0, 0.0, w, h);
    cr.clip();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.paint().ok();
    cr.set_source_rgb(0.8, 0.8, 0.8);
    let (cols, rows) = ((w / size).ceil() as i32, (h / size).ceil() as i32);
    for y in 0..rows {
        for x in (y % 2..cols).step_by(2) {
            cr.rectangle(f64::from(x) * size, f64::from(y) * size, size, size);
        }
    }
    cr.fill().ok();
    cr.restore().ok();
}

/// The crop region's size, above its corner.
fn size_label(cr: &cairo::Context, r: Rect, k: f64) {
    let label = format!("X: {} Y: {} W: {} H: {}", r.x, r.y, r.width, r.height);
    let context = text::prepare(cr);
    let layout = text::label_layout(&context, &label, "Sans", 9);
    let (w, h) = text::layout_size(&layout);
    let pad = 4.0;
    let (mut x, mut y) = (r.x * k, r.y * k - h - pad * 2.0 - 4.0);
    if y < 0.0 {
        y = r.y * k + 4.0;
        x += 4.0;
    }
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.75);
    cr.rectangle(x, y, w + pad * 2.0, h + pad * 2.0);
    cr.fill().ok();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.move_to(x + pad, y + pad);
    pangocairo::functions::show_layout(cr, &layout);
}

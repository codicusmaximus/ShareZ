//! The full-screen region overlay shown on the frozen frame.

mod selector;
mod view;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gdk, glib, prelude::*};
use sharez_core::{Frame, Mapping, PixelRect, Rect};
use sharez_windows::{Elements, Target, Window, WindowKind};

use selector::{Key, Outcome};
use view::View;

/// Keys pressed this soon after the overlay opens are ignored, so the hotkey that started the
/// capture can't also end it (ShareX does the same).
const INPUT_DELAY: Duration = Duration::from_millis(500);

/// Everything captured before the overlay opens.
pub struct Snapshot {
    pub frame: Frame,
    /// Windows topmost first, in logical screen coordinates.
    pub targets: Vec<Target>,
    pub elements: Option<Elements>,
    /// Take a picked window's own pixels, so windows in front of it don't show.
    pub window_pixels: bool,
}

/// The region the user chose.
#[derive(Debug, Clone)]
pub struct Selection {
    /// In the frozen frame's buffer pixels.
    pub pixels: PixelRect,
    /// In logical screen coordinates.
    pub logical: Rect,
    /// The whole screen the overlay covered, logical.
    pub screen: Rect,
    /// When a window (or something in it) was picked and `window_pixels` asked for it: the
    /// window's own pixels under the selection, without whatever covered it. `None` if the
    /// desktop couldn't give them.
    pub window_image: Option<Frame>,
}

type Done = Box<dyn FnOnce(Option<Selection>)>;

/// Opens the overlay. `done` gets the chosen region, or `None` if cancelled, just before the
/// overlay closes.
pub fn show(
    app: Option<&gtk::Application>,
    snapshot: Snapshot,
    done: impl FnOnce(Option<Selection>) + 'static,
) {
    let Snapshot {
        frame,
        targets,
        elements,
        window_pixels,
    } = snapshot;
    let texture = gdk::MemoryTexture::new(
        frame.width as i32,
        frame.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(&frame.data[..]),
        frame.stride(),
    );
    let view = View::new(texture.upcast());
    let window = gtk::Window::builder()
        .title("ShareZ Capture")
        .decorated(false)
        .child(&view)
        .build();
    window.set_application(app);

    let snapper = Rc::new(Snapper {
        targets,
        elements,
        element: Cell::new(None),
        hovered: Cell::new(None),
        pressed: Cell::new(None),
        busy: Cell::new(false),
        pointer: Cell::new((0.0, 0.0)),
    });
    let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let close = {
        let window = window.clone();
        move |result: Option<Selection>| {
            // While the overlay still has focus, so `done` can set the clipboard on Wayland.
            if let Some(done) = done.borrow_mut().take() {
                done(result);
            }
            window.destroy();
        }
    };
    let finish = {
        let view = view.clone();
        let snapper = Rc::clone(&snapper);
        let closing = Cell::new(false);
        move |outcome: Outcome| {
            if closing.get() {
                return;
            }
            let (pixels, picked) = match outcome {
                Outcome::Capture(pixels) => (pixels, None),
                Outcome::Pick(pixels) => (pixels, snapper.picked().filter(|_| window_pixels)),
                Outcome::Cancel => return close(None),
                Outcome::Nothing | Outcome::Redraw => return,
            };
            let selection = selection(&view, pixels);
            let Some(picked) = picked else {
                return close(Some(selection));
            };
            // Asking the desktop for the window takes a moment; the overlay stays up meanwhile so
            // the clipboard can still be set.
            closing.set(true);
            view.set_cursor_from_name(Some("progress"));
            let close = close.clone();
            glib::spawn_future_local(async move {
                let window_image = match sharez_windows::capture(&picked).await {
                    Ok(shot) => shot.crop(selection.logical),
                    Err(e) => {
                        eprintln!("Using the screenshot as it was, not the window alone: {e}");
                        None
                    }
                };
                close(Some(Selection {
                    window_image,
                    ..selection
                }));
            });
        }
    };
    let finish: Rc<dyn Fn(Outcome)> = Rc::new(finish);
    add_pointer_controllers(&view, &snapper, &finish);
    add_key_controller(&window, &view, &snapper, &finish);

    window.fullscreen();
    window.present();
    view.grab_focus();
}

fn selection(view: &View, pixels: PixelRect) -> Selection {
    let (sx, sy) = view.scale();
    Selection {
        pixels,
        logical: Rect::new(
            f64::from(pixels.x) / sx,
            f64::from(pixels.y) / sy,
            f64::from(pixels.width) / sx,
            f64::from(pixels.height) / sy,
        ),
        screen: Rect::new(0.0, 0.0, f64::from(view.width()), f64::from(view.height())),
        window_image: None,
    }
}

fn add_pointer_controllers(view: &View, snapper: &Rc<Snapper>, finish: &Rc<dyn Fn(Outcome)>) {
    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion(glib::clone!(
        #[weak]
        view,
        #[strong]
        snapper,
        move |c, x, y| {
            let square = c
                .current_event_state()
                .contains(gdk::ModifierType::SHIFT_MASK);
            let (bx, by) = view.to_buffer(x, y);
            view.update(|s| s.motion(bx, by, square));
            snapper.hover(&view, x, y);
        }
    ));
    view.add_controller(motion);

    let drag = gtk::GestureDrag::new();
    drag.set_button(gdk::BUTTON_PRIMARY);
    drag.connect_drag_begin(glib::clone!(
        #[weak]
        view,
        #[strong]
        snapper,
        move |_, x, y| {
            snapper.press();
            let (bx, by) = view.to_buffer(x, y);
            view.update(|s| s.press(bx, by));
        }
    ));
    drag.connect_drag_end(glib::clone!(
        #[weak]
        view,
        #[strong]
        finish,
        move |g, dx, dy| {
            let Some((x, y)) = g.start_point() else {
                return;
            };
            let (bx, by) = view.to_buffer(x + dx, y + dy);
            finish(view.update(|s| s.release(bx, by)));
        }
    ));
    view.add_controller(drag);

    let right = gtk::GestureClick::new();
    right.set_button(gdk::BUTTON_SECONDARY);
    right.connect_pressed(glib::clone!(
        #[weak]
        view,
        move |_, _, _, _| {
            view.update(selector::Selector::right_press);
        }
    ));
    right.connect_released(glib::clone!(
        #[weak]
        view,
        #[strong]
        finish,
        move |_, _, _, _| finish(view.update(selector::Selector::right_release))
    ));
    view.add_controller(right);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll(glib::clone!(
        #[weak]
        view,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, _, dy| {
            view.update(|s| s.scroll(dy < 0.0));
            glib::Propagation::Stop
        }
    ));
    view.add_controller(scroll);
}

fn add_key_controller(
    window: &gtk::Window,
    view: &View,
    snapper: &Rc<Snapper>,
    finish: &Rc<dyn Fn(Outcome)>,
) {
    let opened = Instant::now();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        view,
        #[strong]
        finish,
        #[strong]
        snapper,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            if opened.elapsed() < INPUT_DELAY {
                return glib::Propagation::Stop;
            }
            let key = match key {
                gdk::Key::Escape => Key::Escape,
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    snapper.press();
                    Key::Enter
                }
                gdk::Key::space => Key::Space,
                _ => return glib::Propagation::Proceed,
            };
            finish(view.update(|s| s.key(key)));
            glib::Propagation::Stop
        }
    ));
    window.add_controller(keys);
}

/// Works out what the pointer is over. Windows come from the list taken before the overlay
/// opened; elements are looked up live, one request at a time, always for the latest position.
struct Snapper {
    targets: Vec<Target>,
    elements: Option<Elements>,
    /// The element last found, and the target it belongs to.
    element: Cell<Option<(usize, Rect)>>,
    /// The target the highlight belongs to.
    hovered: Cell<Option<usize>>,
    /// `hovered` when the mouse button or Enter went down, which a pick takes.
    pressed: Cell<Option<usize>>,
    busy: Cell<bool>,
    pointer: Cell<(f64, f64)>,
}

impl Snapper {
    fn target_at(&self, x: f64, y: f64) -> Option<usize> {
        self.targets
            .iter()
            .position(|t| t.window.kind != WindowKind::Desktop && t.visible.contains(x, y))
    }

    fn press(&self) {
        self.pressed.set(self.hovered.get());
    }

    /// The window a pick took.
    fn picked(&self) -> Option<Window> {
        self.pressed
            .get()
            .and_then(|t| self.targets.get(t))
            .map(|t| t.window.clone())
    }

    /// Highlights `r`, part of target `t`.
    fn set_hover(&self, view: &View, t: Option<usize>, r: Option<Rect>) {
        self.hovered.set(t);
        let pixels = r.and_then(|r| {
            let (w, h) = (f64::from(view.width()), f64::from(view.height()));
            let (sx, sy) = view.scale();
            Mapping {
                logical: Rect::new(0.0, 0.0, w, h),
                buffer_width: (w * sx).round() as u32,
                buffer_height: (h * sy).round() as u32,
            }
            .to_pixels(&r)
        });
        view.update(|s| s.set_hover(pixels));
    }

    /// `x`/`y` are logical; the overlay covers the screen from its origin.
    fn hover(self: &Rc<Self>, view: &View, x: f64, y: f64) {
        self.pointer.set((x, y));
        let target = self.target_at(x, y);
        match (target, self.element.get()) {
            (Some(t), Some((et, r))) if t == et && r.contains(x, y) => {}
            (Some(t), _) => {
                self.element.set(None);
                self.set_hover(view, Some(t), Some(self.targets[t].visible));
            }
            (None, _) => {
                self.element.set(None);
                self.set_hover(view, None, None);
            }
        }
        if target.is_some() && self.elements.is_some() && !self.busy.replace(true) {
            let this = Rc::clone(self);
            let view = view.clone();
            glib::spawn_future_local(async move {
                this.look_up_elements(&view).await;
                this.busy.set(false);
            });
        }
    }

    async fn look_up_elements(&self, view: &View) {
        let elements = self.elements.as_ref().expect("checked by caller");
        loop {
            let (x, y) = self.pointer.get();
            let Some(t) = self.target_at(x, y) else {
                return;
            };
            let path = elements.elements_at(&self.targets[t], x, y).await;
            // The pointer may have moved on while the app answered.
            let now = self.pointer.get();
            if now != (x, y) {
                continue;
            }
            if let Some(&inner) = path.last() {
                self.element.set(Some((t, inner)));
                self.set_hover(
                    view,
                    Some(t),
                    inner.intersection(&self.targets[t].visible).or(Some(inner)),
                );
            }
            return;
        }
    }
}

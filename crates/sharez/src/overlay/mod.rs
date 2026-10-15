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
use sharez_windows::{Elements, Target, WindowKind};

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
}

/// The region the user chose.
#[derive(Debug, Clone, Copy)]
pub struct Selection {
    /// In the frozen frame's buffer pixels.
    pub pixels: PixelRect,
    /// In logical screen coordinates.
    pub logical: Rect,
    /// The whole screen the overlay covered, logical.
    pub screen: Rect,
}

type Done = Box<dyn FnOnce(Option<Selection>)>;

/// Opens the overlay. `done` gets the chosen region, or `None` if cancelled.
pub fn show(
    app: Option<&gtk::Application>,
    snapshot: Snapshot,
    done: impl FnOnce(Option<Selection>) + 'static,
) {
    let Snapshot {
        frame,
        targets,
        elements,
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

    let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let finish = {
        let window = window.clone();
        let view = view.clone();
        move |outcome: Outcome| {
            let result = match outcome {
                Outcome::Capture(pixels) => Some(selection(&view, pixels)),
                Outcome::Cancel => None,
                Outcome::Nothing | Outcome::Redraw => return,
            };
            window.destroy();
            if let Some(done) = done.borrow_mut().take() {
                done(result);
            }
        }
    };
    let finish: Rc<dyn Fn(Outcome)> = Rc::new(finish);
    let snapper = Rc::new(Snapper {
        targets,
        elements,
        element: Cell::new(None),
        busy: Cell::new(false),
        pointer: Cell::new((0.0, 0.0)),
    });
    add_pointer_controllers(&view, &snapper, &finish);
    add_key_controller(&window, &view, &finish);

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
        move |_, x, y| {
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

fn add_key_controller(window: &gtk::Window, view: &View, finish: &Rc<dyn Fn(Outcome)>) {
    let opened = Instant::now();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        view,
        #[strong]
        finish,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            if opened.elapsed() < INPUT_DELAY {
                return glib::Propagation::Stop;
            }
            let key = match key {
                gdk::Key::Escape => Key::Escape,
                gdk::Key::Return | gdk::Key::KP_Enter => Key::Enter,
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
    busy: Cell<bool>,
    pointer: Cell<(f64, f64)>,
}

impl Snapper {
    fn target_at(&self, x: f64, y: f64) -> Option<usize> {
        self.targets
            .iter()
            .position(|t| t.window.kind != WindowKind::Desktop && t.visible.contains(x, y))
    }

    fn set_hover(view: &View, r: Option<Rect>) {
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
                Self::set_hover(view, Some(self.targets[t].visible));
            }
            (None, _) => {
                self.element.set(None);
                Self::set_hover(view, None);
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
                Self::set_hover(
                    view,
                    inner.intersection(&self.targets[t].visible).or(Some(inner)),
                );
            }
            return;
        }
    }
}

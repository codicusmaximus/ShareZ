//! The overlay's drawing: the frozen frame, dimming, the selection border and the magnifier.
//! Looks follow ShareX's region capture (dim 20 %, a two-tone dashed border, a 150 px circular
//! magnifier of 15×15 pixels with a grid, and small info panels).

use gtk::{gdk, glib, graphene, gsk, pango, prelude::*, subclass::prelude::*};
use sharez_core::PixelRect;

use super::selector::{Outcome, Selector};

const ACCENT: (f32, f32, f32) = (
    0x3e as f32 / 255.0,
    0x83 as f32 / 255.0,
    0xf2 as f32 / 255.0,
);
const DIM: f32 = 0.2;
const MAGNIFIER_SIZE: f32 = 150.0;
const MAGNIFIER_OFFSET: f32 = 18.0;
const PANEL_GAP: f32 = 10.0;
const DASH: f32 = 5.0;

fn rgba(r: f32, g: f32, b: f32, a: f32) -> gdk::RGBA {
    gdk::RGBA::new(r, g, b, a)
}

fn accent(alpha: f32) -> gdk::RGBA {
    rgba(ACCENT.0, ACCENT.1, ACCENT.2, alpha)
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> graphene::Rect {
    graphene::Rect::new(x, y, w, h)
}

#[derive(Debug)]
struct State {
    texture: gdk::Texture,
    selector: Selector,
}

mod imp {
    use std::cell::RefCell;

    use gtk::{glib, subclass::prelude::*};

    use super::{Painter, State};

    #[derive(Debug, Default)]
    pub struct View {
        pub(super) state: RefCell<Option<State>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for View {
        const NAME: &'static str = "ShareZOverlayView";
        type Type = super::View;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for View {}

    impl WidgetImpl for View {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(state) = self.state.borrow().as_ref() {
                Painter::new(&self.obj(), state, snapshot).paint();
            }
        }
    }
}

glib::wrapper! {
    pub struct View(ObjectSubclass<imp::View>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl View {
    pub fn new(texture: gdk::Texture) -> Self {
        let view: Self = glib::Object::builder().build();
        let selector = Selector::new(texture.width() as u32, texture.height() as u32);
        view.imp().state.replace(Some(State { texture, selector }));
        view.set_cursor_from_name(Some("crosshair"));
        view.set_focusable(true);
        view
    }

    /// Buffer pixels per logical pixel, horizontally and vertically.
    pub fn scale(&self) -> (f64, f64) {
        let state = self.imp().state.borrow();
        let s = state.as_ref().expect("view state");
        (
            f64::from(s.texture.width()) / f64::from(self.width().max(1)),
            f64::from(s.texture.height()) / f64::from(self.height().max(1)),
        )
    }

    /// Runs `f` on the selector with a point converted from widget to buffer coordinates, and
    /// redraws if needed.
    pub fn update(&self, f: impl FnOnce(&mut Selector) -> Outcome) -> Outcome {
        let outcome = f(&mut self
            .imp()
            .state
            .borrow_mut()
            .as_mut()
            .expect("view state")
            .selector);
        if outcome != Outcome::Nothing {
            self.queue_draw();
        }
        outcome
    }

    pub fn to_buffer(&self, x: f64, y: f64) -> (f64, f64) {
        let (sx, sy) = self.scale();
        (x * sx, y * sy)
    }
}

/// One frame's drawing, in logical widget coordinates.
struct Painter<'a> {
    view: &'a View,
    state: &'a State,
    snapshot: &'a gtk::Snapshot,
    width: f32,
    height: f32,
    sx: f32,
    sy: f32,
    /// One device pixel, so lines stay crisp at fractional scales.
    px: f32,
}

impl<'a> Painter<'a> {
    fn new(view: &'a View, state: &'a State, snapshot: &'a gtk::Snapshot) -> Self {
        let (width, height) = (view.width() as f32, view.height() as f32);
        let px = view
            .native()
            .and_then(|n| n.surface())
            .map_or(1.0, |s| 1.0 / s.scale() as f32);
        Self {
            view,
            state,
            snapshot,
            width,
            height,
            sx: state.texture.width() as f32 / width.max(1.0),
            sy: state.texture.height() as f32 / height.max(1.0),
            px,
        }
    }

    fn to_view(&self, r: PixelRect) -> graphene::Rect {
        rect(
            r.x as f32 / self.sx,
            r.y as f32 / self.sy,
            r.width as f32 / self.sx,
            r.height as f32 / self.sy,
        )
    }

    fn paint(&self) {
        let snap = self.snapshot;
        snap.append_texture(
            &self.state.texture,
            &rect(0.0, 0.0, self.width, self.height),
        );
        let selection = self.state.selector.selection();
        let active = selection.or(self.state.selector.hover());
        match active {
            Some(r) => {
                let r = self.to_view(r);
                self.dim_outside(&r);
                self.border(&r);
            }
            None => snap.append_color(
                &rgba(0.0, 0.0, 0.0, DIM),
                &rect(0.0, 0.0, self.width, self.height),
            ),
        }
        if let Some(sel) = selection {
            let r = self.to_view(sel);
            self.centre_cross(&r);
            self.size_label(sel, &r);
        }
        self.magnifier();
    }

    fn dim_outside(&self, r: &graphene::Rect) {
        let dim = rgba(0.0, 0.0, 0.0, DIM);
        let (w, h) = (self.width, self.height);
        let snap = self.snapshot;
        snap.append_color(&dim, &rect(0.0, 0.0, w, r.y()));
        snap.append_color(
            &dim,
            &rect(0.0, r.y() + r.height(), w, h - r.y() - r.height()),
        );
        snap.append_color(&dim, &rect(0.0, r.y(), r.x(), r.height()));
        snap.append_color(
            &dim,
            &rect(r.x() + r.width(), r.y(), w - r.x() - r.width(), r.height()),
        );
    }

    /// A one-device-pixel accent line with a black 5-on 5-off dash on top, just inside `r`.
    fn border(&self, area: &graphene::Rect) {
        let (snap, px) = (self.snapshot, self.px);
        let (x, y, w, h) = (area.x(), area.y(), area.width(), area.height());
        let edges = [
            rect(x, y, w, px),
            rect(x, y + h - px, w, px),
            rect(x, y, px, h),
            rect(x + w - px, y, px, h),
        ];
        for e in &edges {
            snap.append_color(&accent(1.0), e);
        }
        let black = rgba(0.0, 0.0, 0.0, 230.0 / 255.0);
        let (dash, step) = (DASH * px, 2.0 * DASH * px);
        let mut pos = 0.0;
        while pos < w {
            let len = dash.min(w - pos);
            snap.append_color(&black, &rect(x + pos, y, len, px));
            snap.append_color(&black, &rect(x + pos, y + h - px, len, px));
            pos += step;
        }
        pos = 0.0;
        while pos < h {
            let len = dash.min(h - pos);
            snap.append_color(&black, &rect(x, y + pos, px, len));
            snap.append_color(&black, &rect(x + w - px, y + pos, px, len));
            pos += step;
        }
    }

    /// A 21 px plus at the selection's centre with a black shadow.
    fn centre_cross(&self, r: &graphene::Rect) {
        let (cx, cy) = (r.x() + r.width() / 2.0, r.y() + r.height() / 2.0);
        let (arm, t) = (10.0, self.px);
        for (dx, colour) in [(-1.0, rgba(0.0, 0.0, 0.0, 1.0)), (0.0, accent(1.0))] {
            self.snapshot
                .append_color(&colour, &rect(cx - arm + dx, cy + dx, 2.0 * arm + t, t));
            self.snapshot
                .append_color(&colour, &rect(cx + dx, cy - arm + dx, t, 2.0 * arm + t));
        }
    }

    fn layout(&self, text: &str) -> pango::Layout {
        let layout = self.view.create_pango_layout(Some(text));
        let mut font = pango::FontDescription::new();
        font.set_absolute_size(12.0 * f64::from(pango::SCALE));
        font.set_weight(pango::Weight::Semibold);
        layout.set_font_description(Some(&font));
        layout.set_alignment(pango::Alignment::Center);
        layout
    }

    /// Size of a text panel (padding 7×4 around the text).
    fn panel_size(layout: &pango::Layout) -> (f32, f32) {
        let (_, logical) = layout.pixel_extents();
        (logical.width() as f32 + 14.0, logical.height() as f32 + 8.0)
    }

    fn panel(&self, layout: &pango::Layout, x: f32, y: f32) {
        let (w, h) = Self::panel_size(layout);
        let snap = self.snapshot;
        let bounds = gsk::RoundedRect::from_rect(rect(x, y, w, h), 5.0);
        snap.push_rounded_clip(&bounds);
        snap.append_color(&rgba(0.12, 0.12, 0.13, 0.92), bounds.bounds());
        snap.pop();
        let line = rgba(1.0, 1.0, 1.0, 0.18);
        snap.append_border(&bounds, &[self.px; 4], &[line; 4]);
        snap.save();
        snap.translate(&graphene::Point::new(x + 7.0, y + 4.0));
        snap.append_layout(layout, &rgba(1.0, 1.0, 1.0, 1.0));
        snap.restore();
    }

    /// "W × H" below the selection's bottom-left corner, or above it if there's no room.
    fn size_label(&self, sel: PixelRect, r: &graphene::Rect) {
        let layout = self.layout(&format!("{} × {}", sel.width, sel.height));
        let (w, h) = Self::panel_size(&layout);
        let mut y = r.y() + r.height() + 8.0;
        if y + h > self.height {
            y = (r.y() - h - 8.0).max(0.0);
        }
        self.panel(&layout, r.x().clamp(0.0, (self.width - w).max(0.0)), y);
    }

    fn magnifier(&self) {
        let Some((bx, by)) = self.state.selector.pointer_pixel() else {
            return;
        };
        let n = self.state.selector.magnifier_pixels() as f32;
        let size = MAGNIFIER_SIZE;
        let cell = size / n;
        let info = self.layout(&format!("X: {bx} Y: {by}"));
        let (info_w, info_h) = Self::panel_size(&info);
        let total_h = size + 2.0 + PANEL_GAP + info_h;

        // Down and to the right of the cursor, flipping where it would leave the screen.
        let (cx, cy) = ((bx as f32 + 0.5) / self.sx, (by as f32 + 0.5) / self.sy);
        let mut x = cx + MAGNIFIER_OFFSET;
        if x + size + 2.0 > self.width {
            x = cx - MAGNIFIER_OFFSET - size - 2.0;
        }
        let mut y = cy + MAGNIFIER_OFFSET;
        if y + total_h > self.height {
            y = cy - MAGNIFIER_OFFSET - total_h;
        }
        let (x, y) = (
            (x / self.px).round() * self.px,
            (y / self.px).round() * self.px,
        );

        let snap = self.snapshot;
        let outer = rect(x, y, size + 2.0, size + 2.0);
        let inner = rect(x + 1.0, y + 1.0, size, size);
        let clip = gsk::RoundedRect::from_rect(inner, size / 2.0);
        snap.push_rounded_clip(&clip);
        snap.append_color(&rgba(0.0, 0.0, 0.0, 1.0), &inner);
        // The whole frame scaled so one buffer pixel is one cell, with the cursor's pixel centred.
        let (ix, iy) = (
            inner.x() + size / 2.0 - cell / 2.0,
            inner.y() + size / 2.0 - cell / 2.0,
        );
        let frame_w = self.state.texture.width() as f32 * cell;
        let frame_h = self.state.texture.height() as f32 * cell;
        snap.append_scaled_texture(
            &self.state.texture,
            gsk::ScalingFilter::Nearest,
            &rect(
                ix - bx as f32 * cell,
                iy - by as f32 * cell,
                frame_w,
                frame_h,
            ),
        );
        // Grid lines, then the cursor's row and column tinted, then the centre pixel outlined.
        let grid = rgba(0.0, 0.0, 0.0, 75.0 / 255.0);
        let half = (n / 2.0).ceil() + 1.0;
        for k in -(half as i32)..=(half as i32) {
            let off = k as f32 * cell;
            snap.append_color(&grid, &rect(ix + off, inner.y(), self.px, size));
            snap.append_color(&grid, &rect(inner.x(), iy + off, size, self.px));
        }
        snap.append_color(&accent(75.0 / 255.0), &rect(inner.x(), iy, size, cell));
        snap.append_color(&accent(75.0 / 255.0), &rect(ix, inner.y(), cell, size));
        let centre = gsk::RoundedRect::from_rect(rect(ix, iy, cell, cell), 0.0);
        snap.append_border(&centre, &[self.px; 4], &[rgba(0.0, 0.0, 0.0, 1.0); 4]);
        let centre_in = gsk::RoundedRect::from_rect(
            rect(
                ix + self.px,
                iy + self.px,
                cell - 2.0 * self.px,
                cell - 2.0 * self.px,
            ),
            0.0,
        );
        snap.append_border(&centre_in, &[self.px; 4], &[rgba(1.0, 1.0, 1.0, 1.0); 4]);
        snap.pop();
        // A white ring with a black inner line.
        snap.append_border(
            &gsk::RoundedRect::from_rect(outer, size / 2.0 + 1.0),
            &[1.0; 4],
            &[rgba(1.0, 1.0, 1.0, 1.0); 4],
        );
        snap.append_border(&clip, &[self.px; 4], &[rgba(0.0, 0.0, 0.0, 1.0); 4]);

        let info_x = x + (size + 2.0 - info_w) / 2.0;
        self.panel(&info, info_x, y + size + 2.0 + PANEL_GAP);
    }
}

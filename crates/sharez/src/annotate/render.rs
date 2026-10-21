//! Drawing shapes with Cairo, after each ShareX shape's `OnDraw`, and rendering the finished
//! image (`ShapeManager.RenderOutputImage`): effects first, then drawings, in list order.

use std::f64::consts::PI;

use gtk::cairo::{self, Context, ImageSurface};
use sharez_core::Frame;

use super::{
    effects,
    shapes::{ArrowHead, BorderStyle, Category, Color, Kind, Point, Rect, Shape, TextStyle},
    text,
};

/// The image as a Cairo surface.
pub fn surface(image: &Frame) -> ImageSurface {
    let mut s = ImageSurface::create(
        cairo::Format::ARgb32,
        image.width as i32,
        image.height as i32,
    )
    .expect("Cairo surface");
    let stride = s.stride() as usize;
    {
        let mut data = s.data().expect("surface data");
        for y in 0..image.height as usize {
            let src = &image.data[y * image.stride()..(y + 1) * image.stride()];
            let dst = &mut data[y * stride..y * stride + image.width as usize * 4];
            for (p, q) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                let a = u32::from(p[3]);
                let pre = |c: u8| (u32::from(c) * a + 127) / 255;
                let v = (a << 24) | (pre(p[0]) << 16) | (pre(p[1]) << 8) | pre(p[2]);
                q.copy_from_slice(&v.to_ne_bytes());
            }
        }
    }
    s
}

/// The surface's pixels as an image.
pub fn frame(mut s: ImageSurface) -> Frame {
    s.flush();
    let (w, h, stride) = (s.width() as usize, s.height() as usize, s.stride() as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    let data = s.data().expect("surface data");
    for y in 0..h {
        for q in data[y * stride..y * stride + w * 4].chunks_exact(4) {
            let v = u32::from_ne_bytes([q[0], q[1], q[2], q[3]]);
            let a = v >> 24;
            let un = |c: u32| {
                (c * 255 + a / 2)
                    .checked_div(a)
                    .map_or(0, |c| c.min(255) as u8)
            };
            out.extend_from_slice(&[
                un((v >> 16) & 255),
                un((v >> 8) & 255),
                un(v & 255),
                a as u8,
            ]);
        }
    }
    Frame::new(w as u32, h as u32, out)
}

/// The finished image: the canvas with every effect and drawing on it.
pub fn render(canvas: &Frame, shapes: &[Shape]) -> Frame {
    let mut out = canvas.clone();
    for s in shapes.iter().filter(|s| s.category() == Category::Effect) {
        if let Some((x, y, part)) = apply_effect(&out, s) {
            effects::paste(&mut out, &part, x, y);
        }
    }
    let drawings: Vec<&Shape> = shapes
        .iter()
        .filter(|s| s.category() == Category::Drawing)
        .collect();
    if drawings.is_empty() {
        return out;
    }
    let target = surface(&out);
    let source = surface(canvas);
    {
        let cr = Context::new(&target).expect("Cairo context");
        for s in drawings {
            draw(&cr, s, &source);
        }
    }
    frame(target)
}

/// The part of `image` under an effect, with the effect applied, and where it goes.
pub fn apply_effect(image: &Frame, s: &Shape) -> Option<(u32, u32, Frame)> {
    let (x, y, w, h) = s.rect().pixels(image.width, image.height)?;
    let mut part = image.crop(sharez_core::PixelRect {
        x,
        y,
        width: w,
        height: h,
    })?;
    match s.kind {
        Kind::Blur { radius } if radius > 1 => effects::blur(&mut part, radius),
        Kind::Pixelate { size } if size > 1 => effects::pixelate(&mut part, size),
        Kind::Highlight { color } => effects::highlight(&mut part, color),
        _ => return None,
    }
    Some((x, y, part))
}

fn set_color(cr: &Context, c: Color) {
    cr.set_source_rgba(
        f64::from(c.r) / 255.0,
        f64::from(c.g) / 255.0,
        f64::from(c.b) / 255.0,
        f64::from(c.a) / 255.0,
    );
}

/// Sets up a pen: GDI+'s dashes are in pen widths and have flat ends.
fn pen(cr: &Context, width: f64, style: BorderStyle, round: bool) {
    cr.set_line_width(width);
    let dashes: Vec<f64> = style.dashes().iter().map(|d| d * width).collect();
    cr.set_dash(&dashes, 0.0);
    let cap = if round && dashes.is_empty() {
        cairo::LineCap::Round
    } else {
        cairo::LineCap::Butt
    };
    cr.set_line_cap(cap);
    cr.set_line_join(if round {
        cairo::LineJoin::Round
    } else {
        cairo::LineJoin::Miter
    });
}

/// `AddRoundedRectangle`: a capsule when the radius is half the short side or more.
fn rounded_rect(cr: &Context, r: Rect, radius: f64) {
    cr.new_sub_path();
    if radius <= 0.0 {
        cr.rectangle(r.x, r.y, r.width, r.height);
        return;
    }
    let radius = radius.min(r.width.min(r.height) / 2.0);
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    cr.arc(x1 - radius, y0 + radius, radius, -PI / 2.0, 0.0);
    cr.arc(x1 - radius, y1 - radius, radius, 0.0, PI / 2.0);
    cr.arc(x0 + radius, y1 - radius, radius, PI / 2.0, PI);
    cr.arc(x0 + radius, y0 + radius, radius, PI, PI * 1.5);
    cr.close_path();
}

fn ellipse(cr: &Context, r: Rect) {
    if r.width <= 0.0 || r.height <= 0.0 {
        return;
    }
    cr.save().ok();
    cr.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
    cr.scale(r.width / 2.0, r.height / 2.0);
    cr.new_sub_path();
    cr.arc(0.0, 0.0, 1.0, 0.0, 2.0 * PI);
    cr.restore().ok();
}

/// Fills the path with `fill` and strokes it with the border, as GDI+'s `FillPath` then `DrawPath`.
fn paint_path(cr: &Context, border: Color, size: i32, style: BorderStyle, fill: Color) {
    if fill.visible() {
        set_color(cr, fill);
        cr.fill_preserve().ok();
    }
    if size > 0 && border.visible() {
        set_color(cr, border);
        pen(cr, f64::from(size), style, false);
        cr.stroke_preserve().ok();
    }
    cr.new_path();
}

/// Draws a drawing shape. `canvas` is the unedited image, which the magnifier enlarges.
pub fn draw(cr: &Context, s: &Shape, canvas: &ImageSurface) {
    cr.save().ok();
    // GDI+ puts pixel centres on whole coordinates, Cairo between them.
    cr.translate(0.5, 0.5);
    cr.set_antialias(cairo::Antialias::Good);
    match &s.kind {
        Kind::Rectangle { corner_radius } => draw_rectangle(cr, s, s.rect(), *corner_radius),
        Kind::Ellipse => draw_ellipse(cr, s, s.rect()),
        Kind::Freehand { points, arrow, .. } => draw_freehand(cr, s, points, *arrow),
        Kind::Line {
            points,
            curved,
            arrow,
        } => draw_line(cr, s, points, *curved, *arrow),
        Kind::Text {
            text,
            style,
            outline: true,
            ..
        } => draw_outlined_text(cr, s, text, style),
        Kind::Text {
            text,
            style,
            corner_radius,
            ..
        } => {
            draw_rectangle(cr, s, s.rect(), *corner_radius);
            draw_text(cr, s, text, style);
        }
        Kind::SpeechBalloon { text, style, tail } => {
            let r = s.rect();
            if r.width > 10.0 && r.height > 10.0 {
                draw_balloon(cr, s, *tail);
                draw_text(cr, s, text, style);
            }
        }
        Kind::Step { font_size, .. } => draw_step(cr, s, *font_size),
        Kind::Magnify { strength } => {
            cr.restore().ok();
            draw_magnify(cr, s, *strength, canvas);
            return;
        }
        Kind::SmartEraser { color } => {
            cr.set_antialias(cairo::Antialias::None);
            let r = s.rect();
            set_color(cr, *color);
            cr.rectangle(r.x - 0.5, r.y - 0.5, r.width, r.height);
            cr.fill().ok();
        }
        Kind::Blur { .. } | Kind::Pixelate { .. } | Kind::Highlight { .. } | Kind::Crop => {}
    }
    cr.restore().ok();
}

/// A box with its drop shadow: the shadow is the border's shape, or the fill's when there is no
/// border and the fill is opaque.
fn draw_rectangle(cr: &Context, s: &Shape, r: Rect, radius: i32) {
    if radius == 0 {
        cr.set_antialias(cairo::Antialias::None);
    }
    let radius = f64::from(radius);
    let shadow = r.translate(s.shadow_offset.x, s.shadow_offset.y);
    if s.shadow {
        if s.is_border_visible() {
            rounded_rect(cr, shadow, radius);
            paint_path(
                cr,
                s.shadow_color,
                s.border_size,
                s.border_style,
                Color::TRANSPARENT,
            );
        } else if s.fill_color.a == 255 {
            rounded_rect(cr, shadow, radius);
            paint_path(cr, Color::TRANSPARENT, 0, s.border_style, s.shadow_color);
        }
    }
    rounded_rect(cr, r, radius);
    paint_path(
        cr,
        s.border_color,
        s.border_size,
        s.border_style,
        s.fill_color,
    );
    cr.set_antialias(cairo::Antialias::Good);
}

fn draw_ellipse(cr: &Context, s: &Shape, r: Rect) {
    let shadow = r.translate(s.shadow_offset.x, s.shadow_offset.y);
    if s.shadow {
        if s.is_border_visible() {
            ellipse(cr, shadow);
            paint_path(
                cr,
                s.shadow_color,
                s.border_size,
                s.border_style,
                Color::TRANSPARENT,
            );
        } else if s.fill_color.a == 255 {
            ellipse(cr, shadow);
            paint_path(cr, Color::TRANSPARENT, 0, s.border_style, s.shadow_color);
        }
    }
    ellipse(cr, r);
    paint_path(
        cr,
        s.border_color,
        s.border_size,
        s.border_style,
        s.fill_color,
    );
}

/// The way an arrow at the end of `points` points: from a point at least a head's length back.
fn heading(points: &[Point], reach: f64) -> Option<(Point, Point)> {
    let tip = *points.last()?;
    let back = points
        .iter()
        .rev()
        .skip(1)
        .find(|p| p.distance(tip) >= reach)
        .or_else(|| points.iter().rev().skip(1).find(|p| **p != tip))?;
    let d = tip.distance(*back);
    (d > 0.0).then(|| (tip, Point::new((tip.x - back.x) / d, (tip.y - back.y) / d)))
}

/// ShareX's arrow head: a custom line cap 4 pen widths wide and 6 long, with a slightly hollow
/// back, set in 5 widths.
const HEAD_LENGTH: f64 = 6.0;
const HEAD_INSET: f64 = 5.0;

fn arrow_head(cr: &Context, tip: Point, dir: Point, width: f64) {
    let n = Point::new(-dir.y, dir.x);
    let at = |back: f64, side: f64| {
        Point::new(
            tip.x - dir.x * back * width + n.x * side * width,
            tip.y - dir.y * back * width + n.y * side * width,
        )
    };
    let (left, right, control) = (at(HEAD_LENGTH, 2.0), at(HEAD_LENGTH, -2.0), at(4.0, 0.0));
    cr.move_to(tip.x, tip.y);
    cr.line_to(left.x, left.y);
    // A quadratic curve through the hollow at 5 widths back, as a cubic.
    let c1 = Point::new(
        left.x + (control.x - left.x) * 2.0 / 3.0,
        left.y + (control.y - left.y) * 2.0 / 3.0,
    );
    let c2 = Point::new(
        right.x + (control.x - right.x) * 2.0 / 3.0,
        right.y + (control.y - right.y) * 2.0 / 3.0,
    );
    cr.curve_to(c1.x, c1.y, c2.x, c2.y, right.x, right.y);
    cr.close_path();
    cr.fill().ok();
}

/// Which ends get a head: both only when the line is long enough for two.
fn heads(arrow: Option<ArrowHead>, length: f64, width: f64) -> (bool, bool) {
    match arrow {
        None => (false, false),
        Some(ArrowHead::Both) if length > HEAD_LENGTH * width * 2.0 => (true, true),
        Some(ArrowHead::Start) => (true, false),
        Some(_) => (false, true),
    }
}

/// Strokes a polyline or curve with arrow heads, pulling its ends in under the heads.
fn stroke_with_heads(
    cr: &Context,
    path: &[Point],
    color: Color,
    width: f64,
    style: BorderStyle,
    (start, end): (bool, bool),
) {
    let mut path = path.to_vec();
    let reach = HEAD_LENGTH * width;
    let mut tips = Vec::new();
    if end && let Some((tip, dir)) = heading(&path, reach) {
        tips.push((tip, dir));
        let last = path.len() - 1;
        path[last] = tip.offset(-dir.x * HEAD_INSET * width, -dir.y * HEAD_INSET * width);
    }
    if start {
        let reversed: Vec<Point> = path.iter().rev().copied().collect();
        if let Some((tip, dir)) = heading(&reversed, reach) {
            tips.push((tip, dir));
            path[0] = tip.offset(-dir.x * HEAD_INSET * width, -dir.y * HEAD_INSET * width);
        }
    }
    set_color(cr, color);
    pen(cr, width, style, true);
    if let Some(first) = path.first() {
        cr.move_to(first.x, first.y);
        for p in &path[1..] {
            cr.line_to(p.x, p.y);
        }
        cr.stroke().ok();
    }
    for (tip, dir) in tips {
        arrow_head(cr, tip, dir, width);
    }
}

fn draw_freehand(cr: &Context, s: &Shape, points: &[Point], arrow: Option<ArrowHead>) {
    let width = f64::from(s.border_size.max(1));
    let draw = |color: Color, offset: Point| {
        if !color.visible() || points.is_empty() {
            return;
        }
        let points: Vec<Point> = points.iter().map(|p| *p + offset).collect();
        if points.len() == 1 {
            set_color(cr, color);
            cr.arc(points[0].x, points[0].y, width / 2.0, 0.0, 2.0 * PI);
            cr.fill().ok();
            return;
        }
        let length = points[0].distance(points[points.len() - 1]);
        stroke_with_heads(
            cr,
            &points,
            color,
            width,
            s.border_style,
            heads(arrow, length, width),
        );
    };
    if s.shadow {
        draw(s.shadow_color, s.shadow_offset);
    }
    draw(s.border_color, Point::default());
}

/// GDI+'s `DrawCurve`: a cardinal spline with tension 0.5 through the points, flattened.
fn cardinal_spline(points: &[Point]) -> Vec<Point> {
    let n = points.len();
    let t = 0.5 / 3.0;
    let mut out = vec![points[0]];
    for i in 0..n - 1 {
        let p0 = points[i.saturating_sub(1)];
        let (p1, p2) = (points[i], points[i + 1]);
        let p3 = points[(i + 2).min(n - 1)];
        let c1 = Point::new(p1.x + (p2.x - p0.x) * t, p1.y + (p2.y - p0.y) * t);
        let c2 = Point::new(p2.x - (p3.x - p1.x) * t, p2.y - (p3.y - p1.y) * t);
        for k in 1..=16 {
            let u = f64::from(k) / 16.0;
            let v = 1.0 - u;
            out.push(Point::new(
                v * v * v * p1.x
                    + 3.0 * v * v * u * c1.x
                    + 3.0 * v * u * u * c2.x
                    + u * u * u * p2.x,
                v * v * v * p1.y
                    + 3.0 * v * v * u * c1.y
                    + 3.0 * v * u * u * c2.y
                    + u * u * u * p2.y,
            ));
        }
    }
    out
}

fn draw_line(cr: &Context, s: &Shape, points: &[Point], curved: bool, arrow: Option<ArrowHead>) {
    let width = f64::from(s.border_size.max(1));
    let path = if curved && points.len() > 2 {
        cardinal_spline(points)
    } else {
        vec![points[0], points[points.len() - 1]]
    };
    let length = points[0].distance(points[points.len() - 1]);
    let draw = |color: Color, offset: Point| {
        if color.visible() {
            let path: Vec<Point> = path.iter().map(|p| *p + offset).collect();
            stroke_with_heads(
                cr,
                &path,
                color,
                width,
                s.border_style,
                heads(arrow, length, width),
            );
        }
    };
    if s.shadow {
        draw(s.shadow_color, s.shadow_offset);
    }
    draw(s.border_color, Point::default());
}

/// Lays text out in its box, aligned both ways as `DrawString` does, and moves to its corner.
fn place_text(cr: &Context, t: &str, style: &TextStyle, r: Rect) -> gtk::pango::Layout {
    let context = text::prepare(cr);
    let layout = text::layout(&context, t, style, Some(r.width));
    let (_, h) = text::layout_size(&layout);
    let y = match style.align_vertical {
        super::shapes::Align::Near => r.y,
        super::shapes::Align::Center => r.y + (r.height - h) / 2.0,
        super::shapes::Align::Far => r.bottom() - h,
    };
    cr.move_to(r.x, y);
    layout
}

/// Text in its box, with its shadow.
fn draw_text(cr: &Context, s: &Shape, t: &str, style: &TextStyle) {
    let r = s.rect();
    if t.is_empty() || r.width <= 10.0 || r.height <= 10.0 {
        return;
    }
    if s.shadow {
        let layout = place_text(
            cr,
            t,
            style,
            r.translate(s.shadow_offset.x, s.shadow_offset.y),
        );
        set_color(cr, s.shadow_color);
        pangocairo::functions::show_layout(cr, &layout);
    }
    let layout = place_text(cr, t, style, r);
    set_color(cr, style.color);
    pangocairo::functions::show_layout(cr, &layout);
}

/// Text (Outline): the letters stroked with the border, then filled.
fn draw_outlined_text(cr: &Context, s: &Shape, t: &str, style: &TextStyle) {
    let r = s.rect();
    if t.is_empty() || r.width <= 10.0 || r.height <= 10.0 {
        return;
    }
    let border = f64::from(s.border_size);
    let outline = |r: Rect| {
        let layout = place_text(cr, t, style, r);
        pangocairo::functions::layout_path(cr, &layout);
    };
    cr.set_line_join(cairo::LineJoin::Round);
    if s.shadow {
        outline(r.translate(s.shadow_offset.x, s.shadow_offset.y));
        set_color(cr, s.shadow_color);
        if s.is_border_visible() {
            cr.set_line_width(border);
            cr.stroke().ok();
        } else {
            cr.fill().ok();
        }
    }
    outline(r);
    if s.is_border_visible() {
        set_color(cr, s.border_color);
        cr.set_line_width(border);
        cr.stroke_preserve().ok();
    }
    set_color(cr, style.color);
    cr.fill().ok();
}

/// `CreateTailPath`: a triangle from the shape's middle to the tail, `multiplier` of the shape's
/// average side wide at the base.
fn tail(cr: &Context, r: Rect, tail: Point, multiplier: f64) {
    let c = r.center();
    let width = (multiplier * (r.width + r.height) / 2.0)
        .min(r.width)
        .min(r.height);
    let half = width / 2.0;
    let length = c.distance(tail);
    let angle = (tail.y - c.y).atan2(tail.x - c.x);
    cr.save().ok();
    cr.translate(c.x, c.y);
    cr.rotate(angle);
    cr.move_to(0.0, -half);
    cr.line_to(0.0, half);
    cr.line_to(length, 0.0);
    cr.close_path();
    cr.restore().ok();
}

fn draw_balloon(cr: &Context, s: &Shape, tail_at: Point) {
    let draw = |border: Color, size: i32, fill: Color, r: Rect, t: Point| {
        let tail_visible = !r.contains(t);
        if fill.visible() {
            set_color(cr, fill);
            cr.rectangle(r.x, r.y, r.width, r.height);
            cr.fill().ok();
            if tail_visible {
                tail(cr, r, t, 0.3);
                cr.fill().ok();
            }
        }
        if size > 0 && border.visible() {
            set_color(cr, border);
            cr.set_line_width(f64::from(size));
            cr.set_dash(&[], 0.0);
            if tail_visible {
                // The tail's border, outside the box.
                cr.save().ok();
                cr.set_fill_rule(cairo::FillRule::EvenOdd);
                cr.rectangle(-1e5, -1e5, 2e5, 2e5);
                let inner = r.inflate(-1.0);
                cr.rectangle(inner.x, inner.y, inner.width, inner.height);
                cr.clip();
                tail(cr, r, t, 0.3);
                cr.stroke().ok();
                cr.restore().ok();
            }
            // The box's border, inside it and not across the tail.
            cr.save().ok();
            if tail_visible {
                cr.set_fill_rule(cairo::FillRule::EvenOdd);
                cr.rectangle(-1e5, -1e5, 2e5, 2e5);
                tail(cr, r, t, 0.3);
                cr.clip();
            }
            let half = f64::from(size) / 2.0;
            let inset = r.inflate(f64::from(size - 1)).inflate(-half);
            cr.rectangle(
                r.x.max(inset.x),
                r.y.max(inset.y),
                inset.width.min(r.width),
                inset.height.min(r.height),
            );
            cr.stroke().ok();
            cr.restore().ok();
        }
    };
    let r = s.rect();
    if s.shadow {
        let o = s.shadow_offset;
        if s.is_border_visible() {
            draw(
                s.shadow_color,
                s.border_size,
                Color::TRANSPARENT,
                r.translate(o.x, o.y),
                tail_at + o,
            );
        } else if s.fill_color.a == 255 {
            draw(
                Color::TRANSPARENT,
                0,
                s.shadow_color,
                r.translate(o.x, o.y),
                tail_at + o,
            );
        }
    }
    draw(s.border_color, s.border_size, s.fill_color, r, tail_at);
}

fn draw_step(cr: &Context, s: &Shape, font_size: i32) {
    let r = s.rect();
    let o = s.shadow_offset;
    if s.shadow {
        if s.is_border_visible() {
            ellipse(cr, r.translate(o.x, o.y));
            paint_path(
                cr,
                s.shadow_color,
                s.border_size,
                s.border_style,
                Color::TRANSPARENT,
            );
        } else if s.fill_color.a == 255 {
            ellipse(cr, r.translate(o.x, o.y));
            paint_path(cr, Color::TRANSPARENT, 0, s.border_style, s.shadow_color);
        }
    }
    if s.tail_visible()
        && let Some(t) = s.tail()
    {
        let around = r.inflate(f64::from(s.border_size / 2));
        if s.shadow {
            set_color(cr, s.shadow_color);
            tail(cr, around.translate(o.x, o.y), t + o, 1.0);
            cr.fill().ok();
        }
        set_color(
            cr,
            if s.is_border_visible() {
                s.border_color
            } else {
                s.fill_color
            },
        );
        tail(cr, around, t, 1.0);
        cr.fill().ok();
    }
    ellipse(cr, r);
    paint_path(
        cr,
        s.border_color,
        s.border_size,
        s.border_style,
        s.fill_color,
    );
    let label = s.step_text();
    let number = |color: Color, r: Rect| {
        let context = text::prepare(cr);
        let layout = text::step_layout(&context, &label, font_size);
        let (w, h) = text::layout_size(&layout);
        let c = r.center();
        cr.move_to(c.x - w / 2.0, c.y - h / 2.0 + 1.0);
        set_color(cr, color);
        pangocairo::functions::show_layout(cr, &layout);
    };
    if s.shadow {
        number(s.shadow_color, r.translate(o.x, o.y));
    }
    number(s.border_color, r);
}

/// The canvas under the circle, enlarged `strength` percent, then the circle's border.
fn draw_magnify(cr: &Context, s: &Shape, strength: i32, canvas: &ImageSurface) {
    let r = s.rect();
    let zoom = f64::from(strength.max(100)) / 100.0;
    let (w, h) = ((r.width / zoom).trunc(), (r.height / zoom).trunc());
    let src = Point::new(
        r.x + r.width / 2.0 - (w / 2.0).trunc(),
        r.y + r.height / 2.0 - (h / 2.0).trunc(),
    );
    cr.save().ok();
    ellipse(cr, r);
    cr.clip();
    cr.translate(r.x, r.y);
    cr.scale(r.width / w.max(1.0), r.height / h.max(1.0));
    cr.set_source_surface(canvas, -src.x, -src.y).ok();
    let pattern = cr.source();
    pattern.set_filter(cairo::Filter::Nearest);
    cr.paint().ok();
    cr.restore().ok();
    cr.save().ok();
    cr.translate(0.5, 0.5);
    draw_ellipse(cr, s, r);
    cr.restore().ok();
}

/// What an effect shows while it is moved, or before its preview is ready: a dark box with
/// corner marks and the effect's name.
pub fn draw_effect_overlay(cr: &Context, s: &Shape) {
    let r = s.rect();
    let label = match s.kind {
        Kind::Blur { radius } => format!("Blur [{radius}]"),
        Kind::Pixelate { size } => format!("Pixelate [{size}]"),
        Kind::Highlight { .. } => "Highlight".to_string(),
        _ => return,
    };
    cr.save().ok();
    cr.set_source_rgba(0.0, 0.0, 0.0, 150.0 / 255.0);
    cr.rectangle(r.x, r.y, r.width, r.height);
    cr.fill().ok();
    cr.translate(0.5, 0.5);
    corner_lines(cr, r.inflate(1.0), 25.0);
    let context = text::prepare(cr);
    let layout = text::label_layout(&context, &label, "Verdana", 12);
    let (w, h) = text::layout_size(&layout);
    if r.width > w && r.height > h {
        let c = r.center();
        cr.move_to(c.x - w / 2.0, c.y - h / 2.0);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        pangocairo::functions::show_layout(cr, &layout);
    }
    cr.restore().ok();
}

/// `DrawCornerLines`: white marks along each corner.
fn corner_lines(cr: &Context, r: Rect, size: f64) {
    let lx = size.min(r.width / 2.0);
    let ly = size.min(r.height / 2.0);
    let (x0, y0, x1, y1) = (r.x, r.y, r.right() - 1.0, r.bottom() - 1.0);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.set_line_width(1.0);
    for (x, y, dx, dy) in [
        (x0, y0, 1.0, 1.0),
        (x1, y0, -1.0, 1.0),
        (x0, y1, 1.0, -1.0),
        (x1, y1, -1.0, -1.0),
    ] {
        cr.move_to(x + dx * lx, y);
        cr.line_to(x, y);
        cr.line_to(x, y + dy * ly);
    }
    cr.stroke().ok();
}

/// The region border ShareX draws around selections: black, with white dashes over it.
/// `scale` is screen pixels per image pixel, so the line stays one pixel wide.
pub fn region_border(cr: &Context, scale: f64) {
    cr.set_line_width(1.0 / scale);
    cr.set_dash(&[], 0.0);
    cr.set_source_rgb(0.0, 0.0, 0.0);
    cr.stroke_preserve().ok();
    cr.set_dash(&[5.0 / scale, 5.0 / scale], 0.0);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.stroke().ok();
    cr.set_dash(&[], 0.0);
}

/// The outline the shape under the pointer gets.
pub fn hover_path(cr: &Context, s: &Shape) {
    let r = s.rect();
    match &s.kind {
        Kind::Ellipse | Kind::Step { .. } | Kind::Magnify { .. } => ellipse(cr, r),
        Kind::Rectangle { corner_radius } | Kind::Text { corner_radius, .. } => {
            rounded_rect(cr, r, f64::from(*corner_radius));
        }
        _ => cr.rectangle(r.x, r.y, r.width, r.height),
    }
}

/// The crop tool: its region, its size and a cross at its middle.
pub fn draw_crop(cr: &Context, r: Rect, scale: f64) {
    cr.save().ok();
    cr.rectangle(r.x, r.y, r.width, r.height);
    region_border(cr, scale);
    let c = r.center();
    cr.set_source_rgb(0.0, 0.0, 0.0);
    cr.set_line_width(1.0 / scale);
    cr.move_to(c.x - 10.0, c.y);
    cr.line_to(c.x + 10.0, c.y);
    cr.move_to(c.x, c.y - 10.0);
    cr.line_to(c.x, c.y + 10.0);
    cr.stroke().ok();
    cr.restore().ok();
}

/// One of the crop tool's confirm and cancel buttons, which sinks while the pointer is on it.
pub fn draw_button(cr: &Context, r: Rect, label: &str, color: Color, hover: bool) {
    let depth = 3.0;
    let dark = Color::rgb(color.r / 2, color.g / 2, color.b / 2);
    let r = if hover { r.translate(0.0, depth) } else { r };
    cr.save().ok();
    if !hover {
        set_color(cr, dark);
        rounded_rect(cr, r.translate(0.0, depth), 5.0);
        cr.fill().ok();
    }
    set_color(cr, color);
    rounded_rect(cr, r, 5.0);
    cr.fill().ok();
    let context = text::prepare(cr);
    let layout = text::label_layout(&context, label, "Arial", 18);
    let (w, h) = text::layout_size(&layout);
    let c = r.center();
    set_color(cr, dark);
    cr.move_to(c.x - w / 2.0, c.y - h / 2.0 + 4.0);
    pangocairo::functions::show_layout(cr, &layout);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.move_to(c.x - w / 2.0, c.y - h / 2.0 + 2.0);
    pangocairo::functions::show_layout(cr, &layout);
    cr.restore().ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::shapes::{Options, Tool};

    #[test]
    fn surfaces_round_trip() {
        let image = Frame::new(2, 1, vec![255, 0, 0, 255, 10, 20, 30, 128]);
        let back = frame(surface(&image));
        assert_eq!(&back.data[0..4], &[255, 0, 0, 255]);
        let p = &back.data[4..8];
        assert_eq!(p[3], 128);
        assert!((i32::from(p[2]) - 30).abs() <= 2);
    }

    #[test]
    fn rendering_draws_shapes_and_effects() {
        let image = Frame::new(100, 100, vec![255; 100 * 100 * 4]);
        let o = Options::default();
        let mut rect = Shape::new(Tool::Rectangle, &o, 1);
        rect.set_rect(Rect::new(10.0, 10.0, 50.0, 50.0));
        let mut hl = Shape::new(Tool::Highlight, &o, 2);
        hl.set_rect(Rect::new(70.0, 70.0, 20.0, 20.0));
        let out = render(&image, &[rect, hl]);
        let px = |x: usize, y: usize| &out.data[(y * 100 + x) * 4..(y * 100 + x) * 4 + 4];
        // The red border, ShareX's primary colour.
        assert_eq!(px(10, 30), &[242, 60, 60, 255]);
        // Inside it, untouched.
        assert_eq!(px(30, 30), &[255, 255, 255, 255]);
        // Under the highlight, yellow.
        assert_eq!(px(80, 80), &[255, 255, 0, 255]);
    }
}

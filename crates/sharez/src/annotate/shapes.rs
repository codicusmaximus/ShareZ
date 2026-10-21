//! The annotator's shapes, after ShareX's `Shapes/` folder in ShareX.ScreenCaptureLib: what each
//! tool makes, the settings it takes from the toolbar, and how it moves and resizes. Positions are
//! in image pixels.

use std::ops::{Add, Sub};

use super::text;

/// Shapes smaller than this either way are dropped when the mouse is released
/// (`RegionCaptureOptions.DefaultMinimumSize`).
pub const MINIMUM_SIZE: f64 = 5.0;
/// Lines get at least this much room to be clicked on (`LineDrawingShape.MinimumCollisionSize`).
const MINIMUM_COLLISION_SIZE: f64 = 10.0;
/// The most extra points a line can bend at.
pub const MAXIMUM_CENTER_POINTS: i32 = 5;
const STEP_SIZE: f64 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn offset(self, dx: f64, dy: f64) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }

    pub fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }
}

impl Add for Point {
    type Output = Point;
    fn add(self, o: Point) -> Point {
        Point::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Point {
    type Output = Point;
    fn sub(self, o: Point) -> Point {
        Point::new(self.x - o.x, self.y - o.y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The rectangle two corners span, both included (ShareX's `CreateRectangle`).
    pub fn spanning(a: Point, b: Point) -> Self {
        Self::new(
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs() + 1.0,
            (a.y - b.y).abs() + 1.0,
        )
    }

    /// The smallest rectangle around the points.
    pub fn bounding(points: &[Point]) -> Self {
        let Some(first) = points.first() else {
            return Self::default();
        };
        let (mut min, mut max) = (*first, *first);
        for p in points {
            min = Point::new(min.x.min(p.x), min.y.min(p.y));
            max = Point::new(max.x.max(p.x), max.y.max(p.y));
        }
        Self::spanning(min, max)
    }

    pub fn right(self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(self) -> f64 {
        self.y + self.height
    }

    pub fn location(self) -> Point {
        Point::new(self.x, self.y)
    }

    pub fn center(self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    pub fn intersect(self, o: Self) -> Self {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let right = self.right().min(o.right());
        let bottom = self.bottom().min(o.bottom());
        if right <= x || bottom <= y {
            return Self::default();
        }
        Self::new(x, y, right - x, bottom - y)
    }

    pub fn intersects(self, o: Self) -> bool {
        !self.intersect(o).is_empty()
    }

    /// Grown by `d` on every side.
    pub fn inflate(self, d: f64) -> Self {
        Self::new(
            self.x - d,
            self.y - d,
            self.width + d * 2.0,
            self.height + d * 2.0,
        )
    }

    pub fn translate(self, dx: f64, dy: f64) -> Self {
        Self::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    /// The whole pixels the rectangle covers inside a `width` × `height` image, as x, y, width
    /// and height. `None` if none.
    pub fn pixels(self, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
        let r = Self::new(
            self.x.round(),
            self.y.round(),
            self.width.round(),
            self.height.round(),
        )
        .intersect(Self::new(0.0, 0.0, f64::from(width), f64::from(height)));
        (!r.is_empty()).then_some((r.x as u32, r.y as u32, r.width as u32, r.height as u32))
    }
}

/// Straight (not premultiplied) 8-bit RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }

    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    #[cfg(test)]
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    /// ShareX's primary annotation colour.
    pub const PRIMARY: Color = Color::rgb(242, 60, 60);

    pub fn visible(self) -> bool {
        self.a > 0
    }

    /// As `#rrggbbaa`, for the settings file.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#')?;
        if s.len() != 8 || !s.is_ascii() {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
        Some(Self::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?))
    }
}

/// What ShareX calls a shape type, without the region and image tools the editor doesn't have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Select,
    Rectangle,
    Ellipse,
    Freehand,
    FreehandArrow,
    Line,
    Arrow,
    TextOutline,
    TextBackground,
    SpeechBalloon,
    Step,
    Magnify,
    SmartEraser,
    Blur,
    Pixelate,
    Highlight,
    Crop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Drawing,
    Effect,
    Tool,
}

impl Tool {
    /// In ShareX's toolbar order.
    pub const ALL: [Tool; 17] = [
        Tool::Select,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Freehand,
        Tool::FreehandArrow,
        Tool::Line,
        Tool::Arrow,
        Tool::TextOutline,
        Tool::TextBackground,
        Tool::SpeechBalloon,
        Tool::Step,
        Tool::Magnify,
        Tool::SmartEraser,
        Tool::Blur,
        Tool::Pixelate,
        Tool::Highlight,
        Tool::Crop,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Rectangle => "rectangle",
            Tool::Ellipse => "ellipse",
            Tool::Freehand => "freehand",
            Tool::FreehandArrow => "freehand-arrow",
            Tool::Line => "line",
            Tool::Arrow => "arrow",
            Tool::TextOutline => "text-outline",
            Tool::TextBackground => "text-background",
            Tool::SpeechBalloon => "speech-balloon",
            Tool::Step => "step",
            Tool::Magnify => "magnify",
            Tool::SmartEraser => "smart-eraser",
            Tool::Blur => "blur",
            Tool::Pixelate => "pixelate",
            Tool::Highlight => "highlight",
            Tool::Crop => "crop",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Tool::Select => "Select and Move",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Freehand => "Freehand",
            Tool::FreehandArrow => "Freehand Arrow",
            Tool::Line => "Line",
            Tool::Arrow => "Arrow",
            Tool::TextOutline => "Text (Outline)",
            Tool::TextBackground => "Text (Background)",
            Tool::SpeechBalloon => "Speech Balloon",
            Tool::Step => "Step",
            Tool::Magnify => "Magnify",
            Tool::SmartEraser => "Smart Eraser",
            Tool::Blur => "Blur",
            Tool::Pixelate => "Pixelate",
            Tool::Highlight => "Highlight",
            Tool::Crop => "Crop Image",
        }
    }

    /// The letter that picks it, as in ShareX.
    pub fn key(self) -> Option<char> {
        Some(match self {
            Tool::Select => 'm',
            Tool::Rectangle => 'r',
            Tool::Ellipse => 'e',
            Tool::Freehand => 'f',
            Tool::Line => 'l',
            Tool::Arrow => 'a',
            Tool::TextOutline => 'o',
            Tool::TextBackground => 't',
            Tool::SpeechBalloon => 's',
            Tool::Step => 'i',
            Tool::Blur => 'b',
            Tool::Pixelate => 'p',
            Tool::Highlight => 'h',
            Tool::Crop => 'c',
            Tool::FreehandArrow | Tool::Magnify | Tool::SmartEraser => return None,
        })
    }

    /// The number pad key that picks it, as in ShareX.
    pub fn numpad(self) -> Option<u32> {
        Some(match self {
            Tool::Rectangle => 1,
            Tool::Ellipse => 2,
            Tool::Freehand => 3,
            Tool::Line => 4,
            Tool::Arrow => 5,
            Tool::TextOutline => 6,
            Tool::Step => 7,
            Tool::Blur => 8,
            Tool::Pixelate => 9,
            _ => return None,
        })
    }

    pub fn category(self) -> Category {
        match self {
            Tool::Blur | Tool::Pixelate | Tool::Highlight => Category::Effect,
            Tool::Select | Tool::Crop => Category::Tool,
            _ => Category::Drawing,
        }
    }

    /// Whether selecting it keeps it out of the shape list's undo history (ShareX's crop and cut
    /// out tools).
    pub fn is_canvas_tool(self) -> bool {
        self == Tool::Crop
    }
}

macro_rules! choices {
    ($name:ident { $($variant:ident => $id:literal, $title:literal;)* }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub enum $name {
            #[default]
            $($variant,)*
        }

        impl $name {
            pub const ALL: &[$name] = &[$($name::$variant,)*];

            pub fn id(self) -> &'static str {
                match self { $($name::$variant => $id,)* }
            }

            pub fn title(self) -> &'static str {
                match self { $($name::$variant => $title,)* }
            }

            pub fn from_id(id: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|v| v.id() == id)
            }

            pub fn index(self) -> u32 {
                Self::ALL.iter().position(|v| *v == self).unwrap_or(0) as u32
            }

            pub fn from_index(i: u32) -> Self {
                Self::ALL.get(i as usize).copied().unwrap_or_default()
            }
        }
    };
}

choices!(BorderStyle {
    Solid => "solid", "Solid";
    Dash => "dash", "Dash";
    Dot => "dot", "Dot";
    DashDot => "dash-dot", "Dash Dot";
    DashDotDot => "dash-dot-dot", "Dash Dot Dot";
});

choices!(ArrowHead {
    End => "end", "End";
    Start => "start", "Start";
    Both => "both", "Both";
});

choices!(StepType {
    Numbers => "numbers", "Numbers";
    LettersUppercase => "letters-uppercase", "Letters (Uppercase)";
    LettersLowercase => "letters-lowercase", "Letters (Lowercase)";
    RomanUppercase => "roman-uppercase", "Roman Numerals (Uppercase)";
    RomanLowercase => "roman-lowercase", "Roman Numerals (Lowercase)";
});

choices!(Align {
    Near => "near", "Left";
    Center => "center", "Center";
    Far => "far", "Right";
});

impl BorderStyle {
    /// GDI+'s dash pattern, in pen widths. Empty for a solid line.
    pub fn dashes(self) -> &'static [f64] {
        match self {
            BorderStyle::Solid => &[],
            BorderStyle::Dash => &[3.0, 1.0],
            BorderStyle::Dot => &[1.0, 1.0],
            BorderStyle::DashDot => &[3.0, 1.0, 1.0, 1.0],
            BorderStyle::DashDotDot => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// `TextDrawingOptions`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font: String,
    /// In points, at 96 DPI as on Windows.
    pub size: i32,
    pub color: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub align: Align,
    pub align_vertical: Align,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: "Arial".into(),
            size: 18,
            color: Color::WHITE,
            bold: false,
            italic: false,
            underline: false,
            align: Align::Center,
            align_vertical: Align::Center,
        }
    }
}

/// `AnnotationOptions`: what the toolbar is set to, which new shapes start with.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub border_color: Color,
    pub border_size: i32,
    pub border_style: BorderStyle,
    pub fill_color: Color,
    pub corner_radius: i32,
    pub shadow: bool,
    pub shadow_color: Color,
    pub shadow_offset: Point,
    pub line_center_points: i32,
    pub arrow_head: ArrowHead,
    pub text_outline: TextStyle,
    pub text_outline_border_color: Color,
    pub text_outline_border_size: i32,
    pub text: TextStyle,
    pub text_border_color: Color,
    pub text_border_size: i32,
    pub text_fill_color: Color,
    pub step_border_color: Color,
    pub step_border_size: i32,
    pub step_fill_color: Color,
    pub step_font_size: i32,
    pub step_type: StepType,
    pub magnify_strength: i32,
    pub blur_radius: i32,
    pub pixelate_size: i32,
    pub highlight_color: Color,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            border_color: Color::PRIMARY,
            border_size: 4,
            border_style: BorderStyle::Solid,
            fill_color: Color::TRANSPARENT,
            corner_radius: 3,
            shadow: true,
            shadow_color: Color::rgba(0, 0, 0, 125),
            shadow_offset: Point::new(0.0, 1.0),
            line_center_points: 1,
            arrow_head: ArrowHead::End,
            text_outline: TextStyle {
                size: 25,
                bold: true,
                ..TextStyle::default()
            },
            text_outline_border_color: Color::PRIMARY,
            text_outline_border_size: 5,
            text: TextStyle::default(),
            text_border_color: Color::WHITE,
            text_border_size: 0,
            text_fill_color: Color::PRIMARY,
            step_border_color: Color::WHITE,
            step_border_size: 0,
            step_fill_color: Color::PRIMARY,
            step_font_size: 18,
            step_type: StepType::Numbers,
            magnify_strength: 200,
            blur_radius: 35,
            pixelate_size: 15,
            highlight_color: Color::rgb(255, 255, 0),
        }
    }
}

impl Options {
    /// The border colour the toolbar shows for `tool`: text, outlined text and steps keep their
    /// own.
    pub fn border_color(&self, tool: Tool) -> Color {
        match tool {
            Tool::TextBackground | Tool::SpeechBalloon => self.text_border_color,
            Tool::TextOutline => self.text_outline_border_color,
            Tool::Step => self.step_border_color,
            _ => self.border_color,
        }
    }

    pub fn set_border_color(&mut self, tool: Tool, c: Color) {
        *match tool {
            Tool::TextBackground | Tool::SpeechBalloon => &mut self.text_border_color,
            Tool::TextOutline => &mut self.text_outline_border_color,
            Tool::Step => &mut self.step_border_color,
            _ => &mut self.border_color,
        } = c;
    }

    pub fn fill_color(&self, tool: Tool) -> Color {
        match tool {
            Tool::TextBackground | Tool::SpeechBalloon => self.text_fill_color,
            Tool::Step => self.step_fill_color,
            _ => self.fill_color,
        }
    }

    pub fn set_fill_color(&mut self, tool: Tool, c: Color) {
        *match tool {
            Tool::TextBackground | Tool::SpeechBalloon => &mut self.text_fill_color,
            Tool::Step => &mut self.step_fill_color,
            _ => &mut self.fill_color,
        } = c;
    }

    pub fn border_size(&self, tool: Tool) -> i32 {
        match tool {
            Tool::TextBackground | Tool::SpeechBalloon => self.text_border_size,
            Tool::TextOutline => self.text_outline_border_size,
            Tool::Step => self.step_border_size,
            _ => self.border_size,
        }
    }

    pub fn set_border_size(&mut self, tool: Tool, size: i32) {
        *match tool {
            Tool::TextBackground | Tool::SpeechBalloon => &mut self.text_border_size,
            Tool::TextOutline => &mut self.text_outline_border_size,
            Tool::Step => &mut self.step_border_size,
            _ => &mut self.border_size,
        } = size;
    }
}

/// Which of a shape's handles: the eight around it (ShareX's `NodePosition`), the tail of a
/// speech balloon or step, or a point of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    Tail,
    Point(usize),
}

impl Node {
    const AROUND: [Node; 8] = [
        Node::TopLeft,
        Node::Top,
        Node::TopRight,
        Node::Right,
        Node::BottomRight,
        Node::Bottom,
        Node::BottomLeft,
        Node::Left,
    ];

    fn moves_left(self) -> bool {
        matches!(self, Node::TopLeft | Node::Left | Node::BottomLeft)
    }

    fn moves_right(self) -> bool {
        matches!(self, Node::TopRight | Node::Right | Node::BottomRight)
    }

    fn moves_top(self) -> bool {
        matches!(self, Node::TopLeft | Node::Top | Node::TopRight)
    }

    fn moves_bottom(self) -> bool {
        matches!(self, Node::BottomLeft | Node::Bottom | Node::BottomRight)
    }

    fn is_corner(self) -> bool {
        matches!(
            self,
            Node::TopLeft | Node::TopRight | Node::BottomRight | Node::BottomLeft
        )
    }
}

/// What a shape is, with what only that kind has.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Rectangle {
        corner_radius: i32,
    },
    Ellipse,
    Freehand {
        points: Vec<Point>,
        /// Freehand Arrow.
        arrow: Option<ArrowHead>,
        /// Shift was held for the last point, which then follows the mouse in a straight line.
        polygon: bool,
    },
    Line {
        /// Both ends, with the bend points between.
        points: Vec<Point>,
        /// A bend point was dragged, so the line is drawn as a curve through them.
        curved: bool,
        /// Arrow.
        arrow: Option<ArrowHead>,
    },
    Text {
        text: String,
        style: TextStyle,
        /// Text (Outline) rather than Text (Background).
        outline: bool,
        corner_radius: i32,
    },
    SpeechBalloon {
        text: String,
        style: TextStyle,
        tail: Point,
    },
    Step {
        number: i32,
        font_size: i32,
        step_type: StepType,
        tail: Point,
        /// The tail was dragged out, so it is drawn.
        tail_active: bool,
    },
    Magnify {
        strength: i32,
    },
    SmartEraser {
        color: Color,
    },
    Blur {
        radius: i32,
    },
    Pixelate {
        size: i32,
    },
    Highlight {
        color: Color,
    },
    Crop,
}

/// One annotation, as `BaseShape` with `BaseDrawingShape`'s style.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// Stable while the editor is open, so effects can keep their rendering between frames.
    pub id: u64,
    pub kind: Kind,
    start: Point,
    end: Point,
    /// The size when it was made, which Shift keeps the shape's proportions to while resizing.
    pub initial_size: (f64, f64),
    pub border_color: Color,
    pub border_size: i32,
    pub border_style: BorderStyle,
    pub fill_color: Color,
    pub shadow: bool,
    pub shadow_color: Color,
    pub shadow_offset: Point,
}

impl Shape {
    pub fn new(tool: Tool, options: &Options, id: u64) -> Self {
        let kind = match tool {
            Tool::Rectangle => Kind::Rectangle { corner_radius: 0 },
            Tool::Ellipse => Kind::Ellipse,
            Tool::Freehand | Tool::FreehandArrow => Kind::Freehand {
                points: Vec::new(),
                arrow: (tool == Tool::FreehandArrow).then_some(ArrowHead::End),
                polygon: false,
            },
            Tool::Line | Tool::Arrow => Kind::Line {
                points: vec![Point::default(); 2],
                curved: false,
                arrow: (tool == Tool::Arrow).then_some(ArrowHead::End),
            },
            Tool::TextOutline | Tool::TextBackground => Kind::Text {
                text: String::new(),
                style: TextStyle::default(),
                outline: tool == Tool::TextOutline,
                corner_radius: 0,
            },
            Tool::SpeechBalloon => Kind::SpeechBalloon {
                text: String::new(),
                style: TextStyle::default(),
                tail: Point::default(),
            },
            Tool::Step => Kind::Step {
                number: 1,
                font_size: 18,
                step_type: StepType::Numbers,
                tail: Point::default(),
                tail_active: false,
            },
            Tool::Magnify => Kind::Magnify { strength: 200 },
            Tool::SmartEraser => Kind::SmartEraser {
                color: Color::WHITE,
            },
            Tool::Blur => Kind::Blur { radius: 35 },
            Tool::Pixelate => Kind::Pixelate { size: 15 },
            Tool::Highlight => Kind::Highlight {
                color: Color::rgb(255, 255, 0),
            },
            Tool::Crop | Tool::Select => Kind::Crop,
        };
        let mut shape = Self {
            id,
            kind,
            start: Point::default(),
            end: Point::default(),
            initial_size: (0.0, 0.0),
            border_color: Color::PRIMARY,
            border_size: 4,
            border_style: BorderStyle::Solid,
            fill_color: Color::TRANSPARENT,
            shadow: true,
            shadow_color: Color::rgba(0, 0, 0, 125),
            shadow_offset: Point::new(0.0, 1.0),
        };
        if let Kind::Step { .. } = shape.kind {
            shape.set_rect(Rect::new(0.0, 0.0, STEP_SIZE, STEP_SIZE));
        }
        shape.load(options);
        shape
    }

    pub fn tool(&self) -> Tool {
        match &self.kind {
            Kind::Rectangle { .. } => Tool::Rectangle,
            Kind::Ellipse => Tool::Ellipse,
            Kind::Freehand { arrow: None, .. } => Tool::Freehand,
            Kind::Freehand { arrow: Some(_), .. } => Tool::FreehandArrow,
            Kind::Line { arrow: None, .. } => Tool::Line,
            Kind::Line { arrow: Some(_), .. } => Tool::Arrow,
            Kind::Text { outline: true, .. } => Tool::TextOutline,
            Kind::Text { outline: false, .. } => Tool::TextBackground,
            Kind::SpeechBalloon { .. } => Tool::SpeechBalloon,
            Kind::Step { .. } => Tool::Step,
            Kind::Magnify { .. } => Tool::Magnify,
            Kind::SmartEraser { .. } => Tool::SmartEraser,
            Kind::Blur { .. } => Tool::Blur,
            Kind::Pixelate { .. } => Tool::Pixelate,
            Kind::Highlight { .. } => Tool::Highlight,
            Kind::Crop => Tool::Crop,
        }
    }

    pub fn category(&self) -> Category {
        self.tool().category()
    }

    pub fn rect(&self) -> Rect {
        Rect::spanning(self.start, self.end)
    }

    pub fn set_rect(&mut self, r: Rect) {
        self.start = r.location();
        self.end = Point::new(r.x + r.width - 1.0, r.y + r.height - 1.0);
    }

    #[cfg(test)]
    pub fn end(&self) -> Point {
        self.end
    }

    #[cfg(test)]
    pub fn set_start(&mut self, p: Point) {
        self.start = p;
    }

    /// The text it shows, for the text tools.
    pub fn text(&self) -> Option<(&str, &TextStyle)> {
        match &self.kind {
            Kind::Text { text, style, .. } | Kind::SpeechBalloon { text, style, .. } => {
                Some((text, style))
            }
            _ => None,
        }
    }

    pub fn set_text(&mut self, new: String, new_style: TextStyle) {
        if let Kind::Text { text, style, .. } | Kind::SpeechBalloon { text, style, .. } =
            &mut self.kind
        {
            *text = new;
            *style = new_style;
        }
    }

    pub fn is_border_visible(&self) -> bool {
        self.border_size > 0 && self.border_color.visible()
    }

    /// `IsValidShape`: big enough to keep.
    pub fn is_valid(&self) -> bool {
        let r = self.rect();
        match &self.kind {
            Kind::Freehand { points, .. } => !points.is_empty(),
            Kind::Line { .. } => r.width >= MINIMUM_SIZE || r.height >= MINIMUM_SIZE,
            _ => !r.is_empty() && r.width >= MINIMUM_SIZE && r.height >= MINIMUM_SIZE,
        }
    }

    /// Whether a click with `tool` picks it up.
    pub fn is_selectable(&self, tool: Tool) -> bool {
        match self.kind {
            // Freehand drawing over a freehand shape draws another.
            Kind::Freehand { .. } => tool == Tool::Select,
            _ => tool == self.tool() || tool == Tool::Select,
        }
    }

    /// Whether the select tool keeps it selected: drawings and effects, not the crop tool.
    pub fn is_handled_by_select(&self) -> bool {
        self.category() != Category::Tool
    }

    pub fn intersects(&self, p: Point) -> bool {
        self.rect().contains(p)
    }

    /// `OnConfigLoad`: takes the toolbar's settings.
    pub fn load(&mut self, o: &Options) {
        let basic = |s: &mut Self| {
            s.border_color = o.border_color;
            s.border_size = o.border_size;
            s.border_style = o.border_style;
            s.fill_color = o.fill_color;
            s.shadow = o.shadow;
            s.shadow_color = o.shadow_color;
            s.shadow_offset = o.shadow_offset;
        };
        let shadow = |s: &mut Self| {
            s.shadow = o.shadow;
            s.shadow_color = o.shadow_color;
            s.shadow_offset = o.shadow_offset;
        };
        match &mut self.kind {
            Kind::Rectangle { corner_radius } => {
                *corner_radius = o.corner_radius;
                basic(self);
            }
            Kind::Ellipse => basic(self),
            Kind::Freehand { arrow, .. } => {
                if arrow.is_some() {
                    *arrow = Some(o.arrow_head);
                }
                basic(self);
            }
            Kind::Line { arrow, .. } => {
                if arrow.is_some() {
                    *arrow = Some(o.arrow_head);
                }
                basic(self);
                self.set_center_points(o.line_center_points);
            }
            Kind::Text {
                style,
                outline: true,
                ..
            } => {
                *style = o.text_outline.clone();
                self.border_color = o.text_outline_border_color;
                self.border_size = o.text_outline_border_size;
                shadow(self);
            }
            Kind::Text {
                style,
                outline: false,
                corner_radius,
                ..
            } => {
                *style = o.text.clone();
                *corner_radius = o.corner_radius;
                self.border_color = o.text_border_color;
                self.border_size = o.text_border_size;
                self.fill_color = o.text_fill_color;
                shadow(self);
            }
            Kind::SpeechBalloon { style, .. } => {
                *style = o.text.clone();
                self.border_color = o.text_border_color;
                self.border_size = o.text_border_size;
                self.fill_color = o.text_fill_color;
                shadow(self);
            }
            Kind::Step {
                font_size,
                step_type,
                ..
            } => {
                *font_size = o.step_font_size;
                *step_type = o.step_type;
                self.border_color = o.step_border_color;
                self.border_size = o.step_border_size;
                self.fill_color = o.step_fill_color;
                shadow(self);
                self.fit_step();
            }
            Kind::Magnify { strength } => {
                *strength = o.magnify_strength;
                basic(self);
            }
            Kind::Blur { radius } => *radius = o.blur_radius,
            Kind::Pixelate { size } => *size = o.pixelate_size,
            Kind::Highlight { color } => *color = o.highlight_color,
            Kind::SmartEraser { .. } | Kind::Crop => {}
        }
    }

    /// `OnConfigSave`: shows the shape's settings on the toolbar, so new shapes match it.
    pub fn save(&self, o: &mut Options) {
        let basic = |o: &mut Options| {
            o.border_color = self.border_color;
            o.border_size = self.border_size;
            o.border_style = self.border_style;
            o.fill_color = self.fill_color;
            o.shadow = self.shadow;
            o.shadow_color = self.shadow_color;
            o.shadow_offset = self.shadow_offset;
        };
        let shadow = |o: &mut Options| {
            o.shadow = self.shadow;
            o.shadow_color = self.shadow_color;
            o.shadow_offset = self.shadow_offset;
        };
        match &self.kind {
            Kind::Rectangle { corner_radius } => {
                basic(o);
                o.corner_radius = *corner_radius;
            }
            Kind::Ellipse => basic(o),
            Kind::Freehand { arrow, .. } => {
                basic(o);
                if let Some(a) = arrow {
                    o.arrow_head = *a;
                }
            }
            Kind::Line { arrow, points, .. } => {
                basic(o);
                if let Some(a) = arrow {
                    o.arrow_head = *a;
                }
                o.line_center_points = points.len() as i32 - 2;
            }
            Kind::Text {
                style,
                outline: true,
                ..
            } => {
                o.text_outline = style.clone();
                o.text_outline_border_color = self.border_color;
                o.text_outline_border_size = self.border_size;
                shadow(o);
            }
            Kind::Text {
                style,
                corner_radius,
                ..
            } => {
                o.text = style.clone();
                o.text_border_color = self.border_color;
                o.text_border_size = self.border_size;
                o.text_fill_color = self.fill_color;
                o.corner_radius = *corner_radius;
                shadow(o);
            }
            Kind::SpeechBalloon { style, .. } => {
                o.text = style.clone();
                o.text_border_color = self.border_color;
                o.text_border_size = self.border_size;
                o.text_fill_color = self.fill_color;
                shadow(o);
            }
            Kind::Step {
                font_size,
                step_type,
                ..
            } => {
                o.step_border_color = self.border_color;
                o.step_border_size = self.border_size;
                o.step_fill_color = self.fill_color;
                o.step_font_size = *font_size;
                o.step_type = *step_type;
                shadow(o);
            }
            Kind::Magnify { strength } => {
                basic(o);
                o.magnify_strength = *strength;
            }
            Kind::Blur { radius } => o.blur_radius = *radius,
            Kind::Pixelate { size } => o.pixelate_size = *size,
            Kind::Highlight { color } => o.highlight_color = *color,
            Kind::SmartEraser { .. } | Kind::Crop => {}
        }
    }

    /// Gives a line `count` bend points, spread evenly, unless that's how many it has.
    fn set_center_points(&mut self, count: i32) {
        let Kind::Line { points, curved, .. } = &mut self.kind else {
            return;
        };
        let count = count.clamp(0, MAXIMUM_CENTER_POINTS) as usize;
        if points.len() != count + 2 {
            let (first, last) = (points[0], points[points.len() - 1]);
            *points = vec![first; count + 2];
            points[count + 1] = last;
            *curved = false;
        }
        self.refresh();
    }

    /// Puts what follows from the shape's points back in step: a line's bend points and box, a
    /// freehand shape's box, a step's size.
    pub fn refresh(&mut self) {
        match &mut self.kind {
            Kind::Line { points, curved, .. } => {
                let n = points.len();
                if !*curved {
                    let (first, last) = (points[0], points[n - 1]);
                    for (i, p) in points.iter_mut().enumerate().take(n - 1).skip(1) {
                        *p = first.lerp(last, i as f64 / (n - 1) as f64);
                    }
                }
                let mut r = Rect::bounding(points);
                if r.width < MINIMUM_COLLISION_SIZE {
                    r = Rect::new(
                        r.x - MINIMUM_COLLISION_SIZE / 2.0,
                        r.y,
                        r.width + MINIMUM_COLLISION_SIZE,
                        r.height,
                    );
                }
                if r.height < MINIMUM_COLLISION_SIZE {
                    r = Rect::new(
                        r.x,
                        r.y - MINIMUM_COLLISION_SIZE / 2.0,
                        r.width,
                        r.height + MINIMUM_COLLISION_SIZE,
                    );
                }
                self.set_rect(r);
            }
            Kind::Step { .. } => self.fit_step(),
            _ => {}
        }
    }

    /// What a step shows.
    pub fn step_text(&self) -> String {
        let Kind::Step {
            number, step_type, ..
        } = &self.kind
        else {
            return String::new();
        };
        match step_type {
            StepType::Numbers => number.to_string(),
            StepType::LettersUppercase => letters(*number),
            StepType::LettersLowercase => letters(*number).to_lowercase(),
            StepType::RomanUppercase => roman(*number),
            StepType::RomanLowercase => roman(*number).to_lowercase(),
        }
    }

    /// A step is a circle just big enough for its number, around the same centre.
    fn fit_step(&mut self) {
        let Kind::Step { font_size, .. } = self.kind else {
            return;
        };
        let (w, h) = text::step_size(&self.step_text(), font_size);
        let size = w.max(h).round();
        let padding = 3.0;
        let c = self.rect().center();
        self.set_rect(Rect::new(
            c.x - size / 2.0 - padding,
            c.y - size / 2.0 - padding,
            size + padding * 2.0,
            size + padding * 2.0,
        ));
    }

    /// Sets a step's number, from its place among the steps.
    pub fn set_step_number(&mut self, n: i32) {
        if let Kind::Step { number, .. } = &mut self.kind
            && *number != n
        {
            *number = n;
            self.fit_step();
        }
    }

    /// Sizes a text shape to fit its text, `AutoSize`: around its corner, or centred on it.
    pub fn auto_size(&mut self, center: bool) {
        let (w, h) = match self.text() {
            Some((t, style)) if !t.is_empty() => {
                let (w, h) = text::text_size(t, style);
                (w.ceil() + 15.0, h.ceil() + 20.0)
            }
            _ => (100.0, 60.0),
        };
        let r = self.rect();
        let at = if center {
            Point::new(r.x - (w / 2.0).floor(), r.y - (h / 2.0).floor())
        } else {
            r.location()
        };
        self.set_rect(Rect::new(at.x, at.y, w, h));
    }

    /// Places the tail where it starts once the shape is made.
    pub fn place_tail(&mut self) {
        let r = self.rect();
        match &mut self.kind {
            Kind::SpeechBalloon { tail, .. } => *tail = r.location().offset(0.0, r.height + 30.0),
            Kind::Step { tail, .. } => *tail = r.location().offset(r.width + 5.0, r.height + 5.0),
            _ => {}
        }
    }

    pub fn tail(&self) -> Option<Point> {
        match &self.kind {
            Kind::SpeechBalloon { tail, .. } | Kind::Step { tail, .. } => Some(*tail),
            _ => None,
        }
    }

    /// Whether the tail is outside the shape, and so shows.
    pub fn tail_visible(&self) -> bool {
        match &self.kind {
            Kind::SpeechBalloon { tail, .. } => !self.rect().contains(*tail),
            Kind::Step {
                tail, tail_active, ..
            } => *tail_active && !self.rect().contains(*tail),
            _ => false,
        }
    }

    pub fn translate(&mut self, dx: f64, dy: f64) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        self.start = self.start.offset(dx, dy);
        self.end = self.end.offset(dx, dy);
        match &mut self.kind {
            Kind::Freehand { points, .. } | Kind::Line { points, .. } => {
                for p in points {
                    *p = p.offset(dx, dy);
                }
            }
            Kind::SpeechBalloon { tail, .. } | Kind::Step { tail, .. } => {
                *tail = tail.offset(dx, dy);
            }
            _ => {}
        }
    }

    /// Moves it so its corner, or its centre, is at `p` (`MoveAbsolute`).
    pub fn move_to(&mut self, p: Point, center: bool) {
        let r = self.rect();
        let (mut x, mut y) = (p.x, p.y);
        if center {
            x -= r.width / 2.0;
            y -= r.height / 2.0;
        }
        self.translate(x - r.x, y - r.y);
    }

    /// Resizing with the keyboard: Ctrl and the arrow keys grow it from the bottom right, Alt
    /// from the top left.
    pub fn resize_by(&mut self, dx: f64, dy: f64, from_bottom_right: bool) {
        match &mut self.kind {
            Kind::Freehand { .. } | Kind::Step { .. } => self.translate(dx, dy),
            Kind::Line { points, .. } => {
                let i = if from_bottom_right {
                    points.len() - 1
                } else {
                    0
                };
                points[i] = points[i].offset(dx, dy);
                self.refresh();
            }
            _ => {
                let r = self.rect();
                self.set_rect(if from_bottom_right {
                    Rect::new(r.x, r.y, r.width + dx, r.height + dy)
                } else {
                    Rect::new(r.x + dx, r.y + dy, r.width - dx, r.height - dy)
                });
            }
        }
    }

    /// The handles it shows once selected, where they are, and whether each is round. Middle
    /// handles hide when the shape is too small for them, as in `OnNodePositionUpdate`.
    pub fn nodes(&self, node_size: f64) -> Vec<(Node, Point)> {
        let r = self.rect();
        match &self.kind {
            Kind::Freehand { .. } => Vec::new(),
            Kind::Line { points, .. } => {
                let last = points[points.len() - 1];
                let node = |p: Point| {
                    Rect::new(
                        p.x - node_size / 2.0,
                        p.y - node_size / 2.0,
                        node_size,
                        node_size,
                    )
                };
                points
                    .iter()
                    .enumerate()
                    .filter(|(i, p)| *i == points.len() - 1 || !node(**p).intersects(node(last)))
                    .map(|(i, p)| (Node::Point(i), *p))
                    .collect()
            }
            Kind::Step { tail, .. } => vec![(Node::Tail, *tail)],
            _ => {
                let mut nodes = around(r, node_size);
                if let Kind::SpeechBalloon { tail, .. } = &self.kind {
                    nodes.push((Node::Tail, *tail));
                }
                nodes
            }
        }
    }

    /// Where a handle is now.
    pub fn node_position(&self, node: Node) -> Point {
        match node {
            Node::Tail => self.tail().unwrap_or_default(),
            Node::Point(i) => match &self.kind {
                Kind::Line { points, .. } => points.get(i).copied().unwrap_or_default(),
                _ => Point::default(),
            },
            _ => box_node(self.rect(), node),
        }
    }

    /// Dragging a handle to `pos`, from where things were when the drag began
    /// (`OnNodeUpdate`). Shift keeps the shape's first proportions on the corners.
    pub fn drag_node(&mut self, grab: &Grab, pos: Point, proportional: bool) {
        match (&mut self.kind, grab.node) {
            (Kind::Line { points, curved, .. }, Node::Point(i)) => {
                if i > 0 && i < points.len() - 1 {
                    *curved = true;
                }
                if let Some(p) = points.get_mut(i) {
                    *p = pos;
                }
                self.refresh();
            }
            (Kind::SpeechBalloon { tail, .. }, Node::Tail) => *tail = pos,
            (
                Kind::Step {
                    tail, tail_active, ..
                },
                Node::Tail,
            ) => {
                *tail_active = true;
                *tail = pos;
            }
            (_, node) => {
                let (mut start, mut end) = (grab.start, grab.end);
                let dx = pos.x - grab.node_position.x;
                let dy = pos.y - grab.node_position.y;
                if node.moves_left() {
                    start.x += dx;
                } else if node.moves_right() {
                    end.x += dx;
                }
                if node.moves_top() {
                    start.y += dy;
                } else if node.moves_bottom() {
                    end.y += dy;
                }
                self.start = start;
                self.end = end;
                let (iw, ih) = self.initial_size;
                if proportional && node.is_corner() && iw > 0.0 && ih > 0.0 {
                    let r = self.rect();
                    let ratio = (r.width / iw).min(r.height / ih);
                    let (w, h) = ((iw * ratio).round(), (ih * ratio).round());
                    let anchor_x = if node.moves_left() {
                        grab.rect.right() - 1.0
                    } else {
                        grab.rect.x
                    };
                    let anchor_y = if node.moves_top() {
                        grab.rect.bottom() - 1.0
                    } else {
                        grab.rect.y
                    };
                    let mut n = r;
                    if pos.x < anchor_x {
                        n.x = n.right() - w;
                    }
                    n.width = w;
                    if pos.y < anchor_y {
                        n.y = n.bottom() - h;
                    }
                    n.height = h;
                    self.set_rect(n);
                }
            }
        }
    }

    /// Follows the mouse while the shape is being drawn (`OnUpdate` while creating): `velocity`
    /// moves the whole shape instead when Ctrl is held, and Shift keeps lines to 45° and boxes
    /// square.
    pub fn draw_to(&mut self, pos: Point, velocity: Point, ctrl: bool, shift: bool) {
        if let Kind::Freehand {
            points, polygon, ..
        } = &mut self.kind
        {
            if ctrl {
                self.translate(velocity.x, velocity.y);
                return;
            }
            if points.is_empty() || (!shift && points.last() != Some(&pos)) {
                points.push(pos);
            }
            if shift {
                if !*polygon {
                    points.push(pos);
                }
                if let Some(last) = points.last_mut() {
                    *last = pos;
                }
            }
            *polygon = shift;
            let r = Rect::bounding(points);
            self.set_rect(r);
            return;
        }
        if ctrl {
            self.start = self.start + velocity;
        }
        let mut pos = pos;
        if shift || matches!(self.kind, Kind::Magnify { .. }) {
            let (degree, start_degree) = match self.kind {
                Kind::Line { .. } => (45.0, 0.0),
                _ => (90.0, 45.0),
            };
            pos = snap_to_degree(self.start, pos, degree, start_degree);
            pos = Point::new(pos.x.round(), pos.y.round());
        }
        self.end = pos;
        if let Kind::Line { points, .. } = &mut self.kind {
            let n = points.len();
            points[0] = self.start;
            points[n - 1] = self.end;
        }
    }

    /// Keeps the shape inside `bounds` (the crop tool).
    pub fn clamp_to(&mut self, bounds: Rect) {
        let clamp = |p: Point| {
            Point::new(
                p.x.clamp(bounds.x, bounds.right() - 1.0),
                p.y.clamp(bounds.y, bounds.bottom() - 1.0),
            )
        };
        self.start = clamp(self.start);
        self.end = clamp(self.end);
    }
}

/// Where a handle drag began: the handle, and the shape as it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grab {
    pub node: Node,
    pub node_position: Point,
    pub start: Point,
    pub end: Point,
    pub rect: Rect,
}

impl Grab {
    pub fn new(shape: &Shape, node: Node) -> Self {
        let r = shape.rect();
        Self {
            node,
            node_position: shape.node_position(node),
            start: r.location(),
            end: Point::new(r.right() - 1.0, r.bottom() - 1.0),
            rect: r,
        }
    }
}

/// Where one of the eight handles around a box is.
fn box_node(r: Rect, node: Node) -> Point {
    let (x0, xm, x1) = (r.x, r.x + r.width / 2.0, r.x + r.width - 1.0);
    let (y0, ym, y1) = (r.y, r.y + r.height / 2.0, r.y + r.height - 1.0);
    match node {
        Node::TopLeft => Point::new(x0, y0),
        Node::Top => Point::new(xm, y0),
        Node::TopRight => Point::new(x1, y0),
        Node::Right => Point::new(x1, ym),
        Node::BottomRight => Point::new(x1, y1),
        Node::Bottom => Point::new(xm, y1),
        Node::BottomLeft => Point::new(x0, y1),
        Node::Left | Node::Tail | Node::Point(_) => Point::new(x0, ym),
    }
}

/// The eight handles around a box, without the middle ones when the box is too small for them.
fn around(r: Rect, node_size: f64) -> Vec<(Node, Point)> {
    let node = |n: Node| {
        let p = box_node(r, n);
        Rect::new(
            p.x - node_size / 2.0,
            p.y - node_size / 2.0,
            node_size,
            node_size,
        )
    };
    let br = node(Node::BottomRight);
    let mut hidden = Vec::new();
    if node(Node::Right).intersects(br) {
        hidden.extend([Node::Left, Node::Right]);
    }
    if node(Node::Bottom).intersects(br) {
        hidden.extend([Node::Top, Node::Bottom]);
    }
    if node(Node::TopRight).intersects(br) {
        hidden.extend([Node::TopLeft, Node::Top, Node::TopRight]);
    }
    if node(Node::BottomLeft).intersects(br) {
        hidden.extend([Node::TopLeft, Node::Left, Node::BottomLeft]);
    }
    Node::AROUND
        .into_iter()
        .filter(|n| !hidden.contains(n))
        .map(|n| (n, box_node(r, n)))
        .collect()
}

/// `CaptureHelpers.SnapPositionToDegree`: `pos` turned around `origin` to the nearest multiple of
/// `degree` past `start_degree`, at the same distance.
fn snap_to_degree(origin: Point, pos: Point, degree: f64, start_degree: f64) -> Point {
    let angle = (pos.y - origin.y).atan2(pos.x - origin.x).to_degrees();
    let snapped = ((angle - start_degree) / degree).round() * degree + start_degree;
    let distance = origin.distance(pos);
    let r = snapped.to_radians();
    Point::new(origin.x + distance * r.cos(), origin.y + distance * r.sin())
}

/// 1 → A, 26 → Z, 27 → AA.
fn letters(mut n: i32) -> String {
    let mut s = Vec::new();
    while n > 0 {
        n -= 1;
        s.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

fn roman(mut n: i32) -> String {
    const STEPS: [(i32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (value, numeral) in STEPS {
        while n >= value {
            s.push_str(numeral);
            n -= value;
        }
    }
    s
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn rectangles_include_both_corners() {
        let r = Rect::spanning(Point::new(10.0, 20.0), Point::new(5.0, 30.0));
        assert_eq!(r, Rect::new(5.0, 20.0, 6.0, 11.0));
        let mut s = Shape::new(Tool::Rectangle, &Options::default(), 1);
        s.set_rect(r);
        assert_eq!(s.rect(), r);
    }

    #[test]
    fn step_labels() {
        assert_eq!(letters(1), "A");
        assert_eq!(letters(26), "Z");
        assert_eq!(letters(27), "AA");
        assert_eq!(roman(1994), "MCMXCIV");
        assert_eq!(roman(4), "IV");
    }

    #[test]
    fn colors_round_trip_through_hex() {
        let c = Color::rgba(242, 60, 60, 125);
        assert_eq!(Color::from_hex(&c.hex()), Some(c));
        assert_eq!(Color::from_hex("red"), None);
    }

    #[test]
    fn shift_snaps_lines_to_45_degrees_and_boxes_to_squares() {
        let o = Options::default();
        let mut line = Shape::new(Tool::Line, &o, 1);
        line.set_start(Point::new(0.0, 0.0));
        line.draw_to(Point::new(100.0, 10.0), Point::default(), false, true);
        assert_eq!(line.end(), Point::new(100.0, 0.0));
        let mut rect = Shape::new(Tool::Rectangle, &o, 2);
        rect.set_start(Point::new(0.0, 0.0));
        rect.draw_to(Point::new(100.0, 80.0), Point::default(), false, true);
        let r = rect.rect();
        assert_eq!(r.width, r.height);
    }

    #[test]
    fn line_bend_points_follow_the_ends_until_dragged() {
        let o = Options::default();
        let mut line = Shape::new(Tool::Arrow, &o, 1);
        line.set_start(Point::new(0.0, 0.0));
        line.draw_to(Point::new(100.0, 0.0), Point::default(), false, false);
        line.refresh();
        assert_eq!(line.node_position(Node::Point(1)), Point::new(50.0, 0.0));
        let grab = Grab::new(&line, Node::Point(1));
        line.drag_node(&grab, Point::new(50.0, 40.0), false);
        assert!(matches!(line.kind, Kind::Line { curved: true, .. }));
        line.translate(10.0, 0.0);
        assert_eq!(line.node_position(Node::Point(1)), Point::new(60.0, 40.0));
    }

    #[test]
    fn dragging_a_corner_resizes_and_shift_keeps_proportions() {
        let o = Options::default();
        let mut s = Shape::new(Tool::Rectangle, &o, 1);
        s.set_rect(Rect::new(0.0, 0.0, 100.0, 50.0));
        s.initial_size = (100.0, 50.0);
        let grab = Grab::new(&s, Node::BottomRight);
        s.drag_node(&grab, Point::new(199.0, 149.0), false);
        assert_eq!(s.rect(), Rect::new(0.0, 0.0, 200.0, 150.0));
        let grab = Grab::new(&s, Node::BottomRight);
        s.drag_node(&grab, Point::new(199.0, 149.0), true);
        assert_eq!(s.rect(), Rect::new(0.0, 0.0, 200.0, 100.0));
    }

    #[test]
    fn small_shapes_hide_their_middle_handles() {
        let o = Options::default();
        let mut s = Shape::new(Tool::Rectangle, &o, 1);
        s.set_rect(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(s.nodes(13.0).len(), 8);
        s.set_rect(Rect::new(0.0, 0.0, 20.0, 100.0));
        assert_eq!(s.nodes(13.0).len(), 6);
    }

    #[test]
    fn options_follow_the_selected_shape() {
        let mut o = Options::default();
        let mut s = Shape::new(Tool::TextBackground, &o, 1);
        s.fill_color = Color::BLACK;
        s.save(&mut o);
        assert_eq!(o.text_fill_color, Color::BLACK);
        assert_eq!(o.fill_color, Color::TRANSPARENT);
        o.border_size = 9;
        let r = Shape::new(Tool::Rectangle, &o, 2);
        assert_eq!(r.border_size, 9);
    }
}

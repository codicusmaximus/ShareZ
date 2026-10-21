//! What the annotator is doing, after ShareX's `ShapeManager`: which tool is picked, which shape
//! is selected, and what the mouse and keys do to them, with undo. It knows nothing of GTK, so it
//! is tested on its own. Positions are in image pixels.

use sharez_core::{Frame, PixelRect};

use super::shapes::{
    Category, Color, Grab, Kind, Node, Options, Point, Rect, Shape, TextStyle, Tool,
};

/// The crop tool's buttons: their size and gap (`CropTool.buttonSize`, `buttonOffset`).
const BUTTON: (f64, f64) = (80.0, 40.0);
const BUTTON_OFFSET: f64 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Idle,
    /// Drawing a new shape.
    Creating,
    /// Dragging a shape.
    Moving,
    /// Dragging one of the selected shape's handles.
    Resizing(Grab),
}

/// A state to undo to (`ImageEditorMemento`): the shapes, and the canvas when it changed.
#[derive(Debug, Clone)]
struct Memento {
    shapes: Vec<Shape>,
    canvas: Option<Frame>,
}

/// Something the editor needs the window for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Ask for a text shape's text. `new` shapes are removed if the user cancels.
    EditText { id: u64, new: bool },
    /// Paste what is on the clipboard (Ctrl+V).
    Paste,
}

/// What a right click did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightClick {
    /// Cancelled the shape being drawn, or deleted the one under the pointer.
    Handled,
    /// Nothing was under the pointer: ShareX then closes the editor and carries on without the
    /// annotations.
    Continue,
}

/// The crop tool's buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropButton {
    Confirm,
    Cancel,
}

/// A key the editor handles, with the modifiers held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Letter(char),
    Numpad(u32),
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)] // Mirrors ShareX's `ShapeManager`.
pub struct Editor {
    canvas: Frame,
    pub shapes: Vec<Shape>,
    current: Option<u64>,
    tool: Tool,
    pub options: Options,
    /// What the first step shows.
    pub starting_step: i32,
    mode: Mode,
    /// Ctrl held when not drawing: shapes can't be picked up, so new ones can be drawn over them.
    ctrl: bool,
    /// Ctrl held while drawing or resizing: the whole shape follows the mouse.
    corner_moving: bool,
    /// Shift held: keep proportions, or 45° for lines.
    shift: bool,
    pointer: Point,
    hover: Option<u64>,
    button_hover: Option<CropButton>,
    nodes_visible: bool,
    undo: Vec<Memento>,
    redo: Vec<Memento>,
    modified: bool,
    next_id: u64,
    /// How big handles are, in image pixels, so they stay the same size on screen.
    pub node_size: f64,
    /// Goes up whenever the canvas changes, so the window knows to redraw it.
    canvas_version: u64,
}

impl Editor {
    pub fn new(canvas: Frame, options: Options, tool: Tool) -> Self {
        Self {
            canvas,
            shapes: Vec::new(),
            current: None,
            tool,
            options,
            starting_step: 1,
            mode: Mode::Idle,
            ctrl: false,
            corner_moving: false,
            shift: false,
            pointer: Point::default(),
            hover: None,
            button_hover: None,
            nodes_visible: false,
            undo: Vec::new(),
            redo: Vec::new(),
            modified: false,
            next_id: 1,
            node_size: 13.0,
            canvas_version: 0,
        }
    }

    pub fn canvas(&self) -> &Frame {
        &self.canvas
    }

    pub fn canvas_version(&self) -> u64 {
        self.canvas_version
    }

    pub fn canvas_rect(&self) -> Rect {
        Rect::new(
            0.0,
            0.0,
            f64::from(self.canvas.width),
            f64::from(self.canvas.height),
        )
    }

    /// Whether anything was drawn, or the image changed, since it opened.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Picks a tool. The crop tool's region goes, and a selected shape stays selected only if the
    /// new tool can work on it.
    pub fn set_tool(&mut self, tool: Tool) {
        if self.tool == tool {
            return;
        }
        self.tool = tool;
        self.clear_tools();
        if let Some(shape) = self.current() {
            let keep = if tool == Tool::Select {
                shape.is_handled_by_select()
            } else {
                shape.tool() == tool
            };
            if !keep {
                self.deselect();
            }
        }
    }

    /// The tool whose settings the toolbar shows: the selected shape's, with the select tool.
    pub fn options_tool(&self) -> Tool {
        match (self.tool, self.current()) {
            (Tool::Select, Some(s)) => s.tool(),
            (tool, _) => tool,
        }
    }

    pub fn current(&self) -> Option<&Shape> {
        self.current.and_then(|id| self.shape(id))
    }

    fn current_mut(&mut self) -> Option<&mut Shape> {
        let id = self.current?;
        self.shapes.iter_mut().find(|s| s.id == id)
    }

    pub fn shape(&self, id: u64) -> Option<&Shape> {
        self.shapes.iter().find(|s| s.id == id)
    }

    fn shape_mut(&mut self, id: u64) -> Option<&mut Shape> {
        self.shapes.iter_mut().find(|s| s.id == id)
    }

    pub fn hover(&self) -> Option<&Shape> {
        self.hover.and_then(|id| self.shape(id))
    }

    pub fn button_hover(&self) -> Option<CropButton> {
        self.button_hover
    }

    /// Whether a shape is being drawn, dragged or resized, when effects show their outline
    /// instead of their result.
    pub fn is_busy(&self, id: u64) -> bool {
        self.current == Some(id) && self.mode != Mode::Idle
    }

    /// The selected shape's handles, when they show.
    pub fn nodes(&self) -> Vec<(Node, Point)> {
        if !self.nodes_visible || self.ctrl {
            return Vec::new();
        }
        self.current()
            .map(|s| s.nodes(self.node_size))
            .unwrap_or_default()
    }

    /// The crop tool's confirm and cancel buttons, once its region is drawn.
    pub fn crop_buttons(&self) -> Option<(Rect, Rect)> {
        if self.mode == Mode::Creating {
            return None;
        }
        let r = self.shapes.iter().find(|s| s.kind == Kind::Crop)?.rect();
        let view = self.canvas_rect();
        let (w, h) = BUTTON;
        Some(
            if r.bottom() + BUTTON_OFFSET + h > view.bottom()
                && r.width > w * 2.0 + BUTTON_OFFSET * 3.0
                && r.height > h + BUTTON_OFFSET * 2.0
            {
                let y = r.bottom() - BUTTON_OFFSET - h;
                (
                    Rect::new(r.right() - BUTTON_OFFSET * 2.0 - w * 2.0, y, w, h),
                    Rect::new(r.right() - BUTTON_OFFSET - w, y, w, h),
                )
            } else {
                let y = r.bottom() + BUTTON_OFFSET;
                (
                    Rect::new(r.right() - w * 2.0 - BUTTON_OFFSET, y, w, h),
                    Rect::new(r.right() - w, y, w, h),
                )
            },
        )
    }

    fn button_at(&self, p: Point) -> Option<CropButton> {
        let (confirm, cancel) = self.crop_buttons()?;
        if confirm.contains(p) {
            Some(CropButton::Confirm)
        } else if cancel.contains(p) {
            Some(CropButton::Cancel)
        } else {
            None
        }
    }

    fn node_at(&self, p: Point) -> Option<Node> {
        let nodes = self.nodes();
        let hit = |at: Point| {
            let half = self.node_size / 2.0;
            Rect::new(at.x - half, at.y - half, self.node_size, self.node_size).contains(p)
        };
        // The bottom right handle wins where handles overlap.
        nodes
            .iter()
            .find(|(n, at)| *n == Node::BottomRight && hit(*at))
            .or_else(|| nodes.iter().rev().find(|(_, at)| hit(*at)))
            .map(|(n, _)| *n)
    }

    /// Whether the pointer is on a handle or button, which take clicks before shapes do.
    pub fn is_on_control(&self, p: Point) -> bool {
        self.button_at(p).is_some() || self.node_at(p).is_some()
    }

    /// The topmost shape at `p` the current tool can pick up.
    fn intersect(&self, p: Point) -> Option<u64> {
        if self.ctrl {
            return None;
        }
        self.shapes
            .iter()
            .rev()
            .find(|s| s.is_selectable(self.tool) && s.intersects(p))
            .map(|s| s.id)
    }

    fn select(&mut self, id: u64) {
        self.current = Some(id);
        if let Some(s) = self.shape(id).cloned() {
            s.save(&mut self.options);
        }
        self.nodes_visible = self
            .current()
            .is_some_and(|s| !matches!(s.kind, Kind::Freehand { .. }));
    }

    fn deselect(&mut self) {
        self.current = None;
        self.nodes_visible = false;
    }

    /// Takes the toolbar's new settings into the selected shape.
    pub fn update_current(&mut self) {
        let options = self.options.clone();
        if let Some(s) = self.current_mut() {
            s.load(&options);
            if s.category() != Category::Tool {
                self.image_modified();
            }
        }
    }

    // History.

    fn snapshot(&self, canvas: bool) -> Memento {
        Memento {
            shapes: self
                .shapes
                .iter()
                .filter(|s| s.category() != Category::Tool)
                .cloned()
                .collect(),
            canvas: canvas.then(|| self.canvas.clone()),
        }
    }

    fn push(&mut self, m: Memento) {
        self.undo.push(m);
        self.redo.clear();
    }

    /// Remembers the shapes for undo, except while cropping (`CreateShapesMemento`).
    fn shapes_memento(&mut self) {
        if !self.tool.is_canvas_tool() {
            let m = self.snapshot(false);
            self.push(m);
        }
    }

    fn canvas_memento(&mut self) {
        let m = self.snapshot(true);
        self.push(m);
    }

    fn restore(&mut self, m: Memento) {
        if let Some(canvas) = m.canvas {
            self.canvas = canvas;
            self.canvas_version += 1;
        }
        self.shapes = m.shapes;
        self.clear_tools();
        self.deselect();
        self.hover = None;
        self.image_modified();
    }

    pub fn undo(&mut self) {
        if let Some(m) = self.undo.pop() {
            let now = self.snapshot(m.canvas.is_some());
            self.redo.push(now);
            self.restore(m);
        }
    }

    pub fn redo(&mut self) {
        if let Some(m) = self.redo.pop() {
            let now = self.snapshot(m.canvas.is_some());
            self.undo.push(now);
            self.restore(m);
        }
    }

    /// After any change to what the image will look like: steps are renumbered in list order.
    fn image_modified(&mut self) {
        let mut n = self.starting_step;
        for s in &mut self.shapes {
            if let Kind::Step { .. } = s.kind {
                s.set_step_number(n);
                n += 1;
            }
        }
        self.modified = true;
    }

    pub fn set_starting_step(&mut self, n: i32) {
        self.starting_step = n;
        self.image_modified();
    }

    fn add_shape(&mut self, shape: Shape) {
        self.shapes_memento();
        let id = shape.id;
        self.shapes.push(shape);
        self.current = Some(id);
        if let Some(s) = self.shape(id).cloned() {
            s.save(&mut self.options);
        }
    }

    fn new_shape(&mut self, tool: Tool) -> Shape {
        let id = self.next_id;
        self.next_id += 1;
        Shape::new(tool, &self.options, id)
    }

    /// The crop tool's region goes whenever the tool changes (`ClearTools`).
    fn clear_tools(&mut self) {
        let tools: Vec<u64> = self
            .shapes
            .iter()
            .filter(|s| s.category() == Category::Tool)
            .map(|s| s.id)
            .collect();
        if tools.is_empty() {
            return;
        }
        self.shapes.retain(|s| s.category() != Category::Tool);
        if self.current.is_some_and(|id| tools.contains(&id)) {
            self.deselect();
        }
        self.button_hover = None;
    }

    fn remove(&mut self, id: u64, memento: bool) {
        let Some(category) = self.shape(id).map(Shape::category) else {
            return;
        };
        if memento {
            self.shapes_memento();
        }
        self.shapes.retain(|s| s.id != id);
        if self.current == Some(id) {
            self.deselect();
        }
        if self.hover == Some(id) {
            self.hover = None;
        }
        if category != Category::Tool {
            self.image_modified();
        }
    }

    pub fn delete_current(&mut self) {
        if let Some(id) = self.current {
            self.remove(id, true);
        }
    }

    /// Shift+Delete.
    pub fn delete_all(&mut self) {
        if !self.shapes.is_empty() {
            self.shapes_memento();
            self.shapes.clear();
            self.deselect();
            self.hover = None;
            self.image_modified();
        }
    }

    // The mouse.

    /// A left press (`StartRegionSelection`). Handles and the crop buttons come first, then the
    /// shape under the pointer, and otherwise the tool draws a new shape.
    pub fn press(&mut self, p: Point) -> Option<Request> {
        self.pointer = p;
        if let Some(button) = self.button_at(p) {
            match button {
                CropButton::Confirm => self.confirm_crop(),
                CropButton::Cancel => self.cancel_crop(),
            }
            return None;
        }
        if let Some(node) = self.node_at(p)
            && let Some(shape) = self.current()
        {
            let grab = Grab::new(shape, node);
            self.shapes_memento();
            self.mode = Mode::Resizing(grab);
            return None;
        }
        if let Some(id) = self.intersect(p) {
            self.deselect();
            if self.mode != Mode::Moving {
                self.shapes_memento();
            }
            self.mode = Mode::Moving;
            self.select(id);
            return None;
        }
        if self.tool == Tool::Select {
            self.clear_tools();
            self.deselect();
            return None;
        }
        if self.mode == Mode::Creating {
            return None;
        }
        self.clear_tools();
        self.deselect();
        let mut shape = self.new_shape(self.tool);
        let id = shape.id;
        match shape.kind {
            Kind::Step { .. } => {
                // A step lands where it's clicked and follows the mouse until let go.
                let r = shape.rect();
                shape.set_rect(Rect::new(
                    p.x - r.width / 2.0,
                    p.y - r.height / 2.0,
                    r.width,
                    r.height,
                ));
                shape.place_tail();
                self.add_shape(shape);
                self.mode = Mode::Moving;
                self.created(id);
                None
            }
            Kind::Text { .. } | Kind::SpeechBalloon { .. } => {
                shape.set_rect(Rect::new(p.x, p.y, 1.0, 1.0));
                self.add_shape(shape);
                Some(Request::EditText { id, new: true })
            }
            _ => {
                if let Kind::SmartEraser { color } = &mut shape.kind {
                    *color = self.color_at(p).unwrap_or(Color::WHITE);
                }
                shape.set_rect(Rect::new(p.x, p.y, 1.0, 1.0));
                self.add_shape(shape);
                self.mode = Mode::Creating;
                None
            }
        }
    }

    pub fn motion(&mut self, p: Point) {
        let velocity = p - self.pointer;
        self.pointer = p;
        let (corner, shift, bounds) = (self.corner_moving, self.shift, self.canvas_rect());
        match &mut self.mode {
            Mode::Creating => {
                if let Some(s) = self.current_mut() {
                    s.draw_to(p, velocity, corner, shift);
                    if s.kind == Kind::Crop {
                        s.clamp_to(bounds);
                    }
                }
            }
            Mode::Moving => {
                if let Some(s) = self.current_mut() {
                    s.translate(velocity.x, velocity.y);
                    if s.kind == Kind::Crop {
                        s.clamp_to(bounds);
                    }
                }
            }
            Mode::Resizing(grab) => {
                if corner {
                    grab.start = grab.start + velocity;
                    grab.end = grab.end + velocity;
                    grab.node_position = grab.node_position + velocity;
                    grab.rect = grab.rect.translate(velocity.x, velocity.y);
                }
                let grab = *grab;
                if let Some(s) = self.current_mut() {
                    s.drag_node(&grab, p, shift);
                    if s.kind == Kind::Crop {
                        let r = s.rect().intersect(bounds);
                        s.set_rect(r);
                    }
                }
            }
            Mode::Idle => {
                self.button_hover = self.button_at(p);
                self.hover = if self.is_on_control(p) {
                    None
                } else {
                    self.intersect(p)
                        .filter(|id| self.shape(*id).is_some_and(Shape::is_valid))
                };
            }
        }
    }

    /// A left release (`EndRegionSelection`): a new shape is kept if it is big enough, and is
    /// selected.
    pub fn release(&mut self, p: Point) {
        self.motion(p);
        let mode = std::mem::replace(&mut self.mode, Mode::Idle);
        match mode {
            Mode::Resizing(_) => {
                if let Some(s) = self.current_mut() {
                    s.refresh();
                }
                self.image_modified_if_drawing();
            }
            Mode::Creating | Mode::Moving => {
                let Some(id) = self.current else { return };
                if !self.shape(id).is_some_and(Shape::is_valid) {
                    // A click with nothing drawn: no shape, and nothing to undo.
                    self.shapes.retain(|s| s.id != id);
                    self.deselect();
                    if mode == Mode::Creating {
                        self.undo.pop();
                    }
                    return;
                }
                if mode == Mode::Creating {
                    self.created(id);
                } else if let Some(s) = self.shape_mut(id) {
                    s.refresh();
                }
                self.select(id);
                self.image_modified_if_drawing();
            }
            Mode::Idle => {}
        }
    }

    fn image_modified_if_drawing(&mut self) {
        if self
            .current()
            .is_some_and(|s| s.category() != Category::Tool)
        {
            self.image_modified();
        }
    }

    /// `OnCreated`: remembers the shape's size for Shift-resizing.
    fn created(&mut self, id: u64) {
        let Some(s) = self.shape_mut(id) else { return };
        match s.kind {
            Kind::Text { .. } => s.auto_size(true),
            Kind::SpeechBalloon { .. } => {
                s.auto_size(true);
                s.place_tail();
            }
            _ => {}
        }
        s.refresh();
        let r = s.rect();
        s.initial_size = (r.width, r.height);
        let category = s.category();
        if category != Category::Tool {
            self.image_modified();
        }
    }

    /// A double click edits a text shape's text.
    pub fn double_click(&mut self, p: Point) -> Option<Request> {
        if self.mode == Mode::Creating {
            return None;
        }
        let s = self.current()?;
        (s.text().is_some() && s.intersects(p)).then_some(Request::EditText {
            id: s.id,
            new: false,
        })
    }

    /// The text dialog's answer: the text, or `None` if it was cancelled.
    pub fn finish_text(&mut self, id: u64, new: bool, text: Option<(String, TextStyle)>) {
        match text {
            Some((text, style)) => {
                if let Some(s) = self.shape_mut(id) {
                    s.set_text(text, style);
                }
                if new {
                    self.created(id);
                } else if let Some(s) = self.shape_mut(id) {
                    // The box keeps its corner and fits the new text.
                    s.auto_size(false);
                    self.image_modified();
                }
                self.select(id);
            }
            None if new => {
                self.shapes.retain(|s| s.id != id);
                self.deselect();
                self.undo.pop();
            }
            None => {}
        }
    }

    /// A right click: cancels the shape being drawn, or deletes the one under the pointer.
    pub fn right_click(&mut self, p: Point) -> RightClick {
        self.pointer = p;
        if self.mode == Mode::Creating {
            if let Some(id) = self.current {
                self.shapes.retain(|s| s.id != id);
                self.undo.pop();
            }
            self.deselect();
            self.mode = Mode::Idle;
            return RightClick::Handled;
        }
        if self.mode != Mode::Idle {
            return RightClick::Handled;
        }
        match self.intersect(p) {
            Some(id) => {
                self.remove(id, true);
                RightClick::Handled
            }
            None => RightClick::Continue,
        }
    }

    // The keyboard.

    /// Ctrl and Shift going down or up.
    pub fn set_modifiers(&mut self, ctrl: bool, shift: bool) {
        if ctrl && !self.ctrl && !self.corner_moving {
            if matches!(self.mode, Mode::Creating | Mode::Resizing(_)) {
                self.corner_moving = true;
            } else {
                self.ctrl = true;
                self.hover = None;
            }
        } else if !ctrl {
            self.ctrl = false;
            self.corner_moving = false;
        }
        self.shift = shift;
    }

    /// Escape: deselects, then goes back to the select tool. `false` when there was nothing left
    /// to do, and the editor should close.
    pub fn escape(&mut self) -> bool {
        if self.current.is_some() {
            self.clear_tools();
            self.deselect();
        }
        if self.tool != Tool::Select {
            self.set_tool(Tool::Select);
            return true;
        }
        false
    }

    /// The editor's own keys, as in ShareX. `false` if it isn't one of them.
    pub fn key(&mut self, key: Key, mods: Mods) -> (bool, Option<Request>) {
        let none = !mods.ctrl && !mods.shift && !mods.alt;
        let ctrl_only = mods.ctrl && !mods.shift && !mods.alt;
        let creating = self.mode == Mode::Creating;
        match key {
            Key::Delete if mods.shift && !mods.ctrl => {
                self.delete_all();
                return (true, None);
            }
            Key::Delete if none => {
                self.delete_current();
                if creating {
                    self.mode = Mode::Idle;
                }
                return (true, None);
            }
            Key::Left | Key::Right | Key::Up | Key::Down => {
                return (self.arrow(key, mods), None);
            }
            _ => {}
        }
        if creating {
            return (false, None);
        }
        match key {
            Key::Letter(c) if none => {
                if let Some(tool) = Tool::ALL.into_iter().find(|t| t.key() == Some(c)) {
                    self.set_tool(tool);
                    return (true, None);
                }
            }
            Key::Numpad(n) if none => {
                if let Some(tool) = Tool::ALL.into_iter().find(|t| t.numpad() == Some(n)) {
                    self.set_tool(tool);
                    return (true, None);
                }
            }
            Key::Letter('d') if ctrl_only => {
                self.duplicate();
                return (true, None);
            }
            Key::Letter('v') if ctrl_only => return (true, Some(Request::Paste)),
            Key::Letter('z') if ctrl_only => {
                self.undo();
                return (true, None);
            }
            Key::Letter('y') if ctrl_only => {
                self.redo();
                return (true, None);
            }
            Key::Home if none => {
                self.move_current(Order::Top);
                return (true, None);
            }
            Key::End if none => {
                self.move_current(Order::Bottom);
                return (true, None);
            }
            Key::PageUp if none => {
                self.move_current(Order::Up);
                return (true, None);
            }
            Key::PageDown if none => {
                self.move_current(Order::Down);
                return (true, None);
            }
            _ => {}
        }
        (false, None)
    }

    /// The arrow keys move the selected shape a pixel, or ten with Shift; with Ctrl they grow it
    /// from the bottom right, with Alt from the top left.
    fn arrow(&mut self, key: Key, mods: Mods) -> bool {
        if self.mode == Mode::Creating {
            return false;
        }
        let speed = if mods.shift { 10.0 } else { 1.0 };
        let (dx, dy) = match key {
            Key::Left => (-speed, 0.0),
            Key::Right => (speed, 0.0),
            Key::Up => (0.0, -speed),
            _ => (0.0, speed),
        };
        let bounds = self.canvas_rect();
        let Some(s) = self.current_mut() else {
            return false;
        };
        if mods.ctrl {
            s.resize_by(dx, dy, true);
        } else if mods.alt {
            s.resize_by(dx, dy, false);
        } else {
            s.translate(dx, dy);
        }
        if s.kind == Kind::Crop {
            s.clamp_to(bounds);
        }
        self.image_modified_if_drawing();
        true
    }

    /// Ctrl+D: a copy of the selected shape under the pointer.
    pub fn duplicate(&mut self) {
        let Some(shape) = self.current().filter(|s| s.is_handled_by_select()).cloned() else {
            return;
        };
        let mut copy = shape;
        copy.id = self.next_id;
        self.next_id += 1;
        copy.move_to(self.pointer, true);
        copy.refresh();
        let id = copy.id;
        self.add_shape(copy);
        self.select(id);
        self.image_modified();
    }

    /// Pasted text becomes a Text (Background) shape under the pointer.
    pub fn paste_text(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.set_tool(Tool::TextBackground);
        let mut shape = self.new_shape(Tool::TextBackground);
        let style = shape.text().map(|(_, s)| s.clone()).unwrap_or_default();
        shape.set_text(text.to_string(), style);
        shape.set_rect(Rect::new(self.pointer.x, self.pointer.y, 1.0, 1.0));
        let id = shape.id;
        self.add_shape(shape);
        self.created(id);
        self.select(id);
    }

    pub fn move_current(&mut self, order: Order) {
        let Some(id) = self.current else { return };
        let Some(i) = self.shapes.iter().position(|s| s.id == id) else {
            return;
        };
        let last = self.shapes.len() - 1;
        let to = match order {
            Order::Top => last,
            Order::Bottom => 0,
            Order::Up => (i + 1).min(last),
            Order::Down => i.saturating_sub(1),
        };
        let shape = self.shapes.remove(i);
        self.shapes.insert(to, shape);
        self.image_modified();
    }

    /// The canvas colour at `p`, for the smart eraser.
    fn color_at(&self, p: Point) -> Option<Color> {
        let (x, y) = (p.x.floor(), p.y.floor());
        if x < 0.0
            || y < 0.0
            || x >= f64::from(self.canvas.width)
            || y >= f64::from(self.canvas.height)
        {
            return None;
        }
        let i = y as usize * self.canvas.stride() + x as usize * 4;
        let d = &self.canvas.data[i..i + 4];
        Some(Color::rgba(d[0], d[1], d[2], d[3]))
    }

    // The canvas.

    /// Crops the image to the crop tool's region (`CropArea`), moving the shapes with it.
    pub fn confirm_crop(&mut self) {
        let Some(crop) = self.shapes.iter().find(|s| s.kind == Kind::Crop) else {
            return;
        };
        let (id, r) = (crop.id, crop.rect());
        if let Some((x, y, w, h)) = r.pixels(self.canvas.width, self.canvas.height)
            && (w, h) != (self.canvas.width, self.canvas.height)
            && let Some(cropped) = self.canvas.crop(PixelRect {
                x,
                y,
                width: w,
                height: h,
            })
        {
            self.canvas_memento();
            for s in &mut self.shapes {
                s.translate(-f64::from(x), -f64::from(y));
            }
            self.canvas = cropped;
            self.canvas_version += 1;
            self.image_modified();
        }
        self.remove(id, false);
        self.button_hover = None;
    }

    pub fn cancel_crop(&mut self) {
        if let Some(id) = self
            .shapes
            .iter()
            .find(|s| s.kind == Kind::Crop)
            .map(|s| s.id)
        {
            self.remove(id, false);
        }
        self.button_hover = None;
    }

    /// Whether the crop tool's region is out and waiting for a confirm.
    pub fn has_crop(&self) -> bool {
        self.mode == Mode::Idle && self.shapes.iter().any(|s| s.kind == Kind::Crop)
    }

    /// Starts the crop tool over the whole image, as the Image menu's Crop does.
    pub fn crop_whole(&mut self) {
        self.set_tool(Tool::Crop);
        self.clear_tools();
        let mut shape = self.new_shape(Tool::Crop);
        shape.set_rect(self.canvas_rect());
        let id = shape.id;
        self.shapes.push(shape);
        self.created(id);
        self.select(id);
    }

    /// Turns or mirrors the whole image. Shapes stay where they are, as in ShareX.
    pub fn transform(&mut self, t: Transform) {
        self.canvas_memento();
        self.canvas = transform(&self.canvas, t);
        self.canvas_version += 1;
        self.image_modified();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Top,
    Up,
    Down,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    RotateClockwise,
    RotateCounterClockwise,
    Rotate180,
    FlipHorizontal,
    FlipVertical,
}

fn transform(image: &Frame, t: Transform) -> Frame {
    let (w, h) = (image.width as usize, image.height as usize);
    let (nw, nh) = match t {
        Transform::RotateClockwise | Transform::RotateCounterClockwise => (h, w),
        _ => (w, h),
    };
    let mut out = vec![0; image.data.len()];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match t {
                Transform::RotateClockwise => (h - 1 - y, x),
                Transform::RotateCounterClockwise => (y, w - 1 - x),
                Transform::Rotate180 => (w - 1 - x, h - 1 - y),
                Transform::FlipHorizontal => (w - 1 - x, y),
                Transform::FlipVertical => (x, h - 1 - y),
            };
            let from = (y * w + x) * 4;
            let to = (ny * nw + nx) * 4;
            out[to..to + 4].copy_from_slice(&image.data[from..from + 4]);
        }
    }
    Frame::new(nw as u32, nh as u32, out)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        Editor::new(
            Frame::new(200, 100, vec![255; 200 * 100 * 4]),
            Options::default(),
            Tool::Rectangle,
        )
    }

    fn drag(e: &mut Editor, from: (f64, f64), to: (f64, f64)) -> Option<Request> {
        let r = e.press(Point::new(from.0, from.1));
        e.motion(Point::new(
            f64::midpoint(from.0, to.0),
            f64::midpoint(from.1, to.1),
        ));
        e.release(Point::new(to.0, to.1));
        r
    }

    #[test]
    fn drawing_makes_and_selects_a_shape() {
        let mut e = editor();
        drag(&mut e, (10.0, 10.0), (59.0, 39.0));
        assert_eq!(e.shapes.len(), 1);
        assert_eq!(
            e.current().unwrap().rect(),
            Rect::new(10.0, 10.0, 50.0, 30.0)
        );
        assert!(e.is_modified());
        assert_eq!(e.nodes().len(), 8);
    }

    #[test]
    fn a_click_without_a_drag_draws_nothing_and_leaves_nothing_to_undo() {
        let mut e = editor();
        drag(&mut e, (10.0, 10.0), (11.0, 11.0));
        assert_eq!(e.shapes.len(), 0);
        assert!(!e.can_undo());
    }

    #[test]
    fn clicking_a_shape_with_its_tool_moves_it() {
        let mut e = editor();
        drag(&mut e, (10.0, 10.0), (59.0, 39.0));
        drag(&mut e, (30.0, 20.0), (40.0, 25.0));
        assert_eq!(e.shapes.len(), 1);
        assert_eq!(e.shapes[0].rect().location(), Point::new(20.0, 15.0));
    }

    #[test]
    fn ctrl_draws_over_shapes() {
        let mut e = editor();
        drag(&mut e, (10.0, 10.0), (59.0, 39.0));
        e.set_modifiers(true, false);
        drag(&mut e, (30.0, 20.0), (80.0, 60.0));
        assert_eq!(e.shapes.len(), 2);
    }

    #[test]
    fn undo_and_redo() {
        let mut e = editor();
        drag(&mut e, (10.0, 10.0), (59.0, 39.0));
        drag(&mut e, (100.0, 10.0), (150.0, 50.0));
        e.undo();
        assert_eq!(e.shapes.len(), 1);
        e.undo();
        assert_eq!(e.shapes.len(), 0);
        e.redo();
        e.redo();
        assert_eq!(e.shapes.len(), 2);
        // Moving can be undone too.
        drag(&mut e, (30.0, 20.0), (40.0, 20.0));
        e.undo();
        assert_eq!(e.shapes[0].rect().x, 10.0);
    }

    #[test]
    fn tool_keys_and_escape() {
        let mut e = editor();
        assert!(e.key(Key::Letter('a'), Mods::default()).0);
        assert_eq!(e.tool(), Tool::Arrow);
        assert!(e.key(Key::Numpad(7), Mods::default()).0);
        assert_eq!(e.tool(), Tool::Step);
        assert!(e.escape());
        assert_eq!(e.tool(), Tool::Select);
        assert!(!e.escape());
    }

    #[test]
    fn steps_number_themselves_in_order() {
        let mut e = editor();
        e.set_tool(Tool::Step);
        for x in [20.0, 60.0, 100.0] {
            drag(&mut e, (x, 50.0), (x, 50.0));
        }
        let labels: Vec<String> = e.shapes.iter().map(Shape::step_text).collect();
        assert_eq!(labels, ["1", "2", "3"]);
        e.set_tool(Tool::Select);
        e.press(Point::new(20.0, 50.0));
        e.release(Point::new(20.0, 50.0));
        e.delete_current();
        let labels: Vec<String> = e.shapes.iter().map(Shape::step_text).collect();
        assert_eq!(labels, ["1", "2"]);
    }

    #[test]
    fn text_waits_for_its_dialog() {
        let mut e = editor();
        e.set_tool(Tool::TextBackground);
        let Some(Request::EditText { id, new: true }) = e.press(Point::new(100.0, 50.0)) else {
            panic!("no text request");
        };
        e.release(Point::new(100.0, 50.0));
        e.finish_text(id, true, None);
        assert_eq!(e.shapes.len(), 0);
        let Some(Request::EditText { id, .. }) = e.press(Point::new(100.0, 50.0)) else {
            panic!("no text request");
        };
        e.finish_text(id, true, Some(("Hello".into(), TextStyle::default())));
        let s = e.current().unwrap();
        assert!(s.rect().width > 30.0);
        assert!(s.rect().contains(Point::new(100.0, 50.0)));
    }

    #[test]
    fn cropping_moves_shapes_and_can_be_undone() {
        let mut e = editor();
        drag(&mut e, (50.0, 50.0), (69.0, 69.0));
        e.set_tool(Tool::Crop);
        drag(&mut e, (40.0, 40.0), (139.0, 89.0));
        assert!(e.has_crop());
        e.confirm_crop();
        assert_eq!((e.canvas().width, e.canvas().height), (100, 50));
        assert_eq!(e.shapes.len(), 1);
        assert_eq!(e.shapes[0].rect().location(), Point::new(10.0, 10.0));
        e.undo();
        assert_eq!((e.canvas().width, e.canvas().height), (200, 100));
        assert_eq!(e.shapes[0].rect().location(), Point::new(50.0, 50.0));
    }

    #[test]
    fn right_click_deletes_or_continues() {
        let mut e = editor();
        drag(&mut e, (10.0, 10.0), (59.0, 39.0));
        assert_eq!(e.right_click(Point::new(150.0, 80.0)), RightClick::Continue);
        assert_eq!(e.right_click(Point::new(20.0, 20.0)), RightClick::Handled);
        assert_eq!(e.shapes.len(), 0);
    }

    #[test]
    fn rotating_swaps_the_sides() {
        let image = Frame::new(2, 1, vec![1, 1, 1, 1, 2, 2, 2, 2]);
        let r = transform(&image, Transform::RotateClockwise);
        assert_eq!((r.width, r.height), (1, 2));
        assert_eq!(r.data, vec![1, 1, 1, 1, 2, 2, 2, 2]);
        let f = transform(&image, Transform::FlipHorizontal);
        assert_eq!(f.data, vec![2, 2, 2, 2, 1, 1, 1, 1]);
    }

    #[test]
    fn duplicates_land_under_the_pointer() {
        let mut e = editor();
        drag(&mut e, (10.0, 10.0), (29.0, 29.0));
        e.motion(Point::new(100.0, 50.0));
        e.duplicate();
        assert_eq!(e.shapes.len(), 2);
        assert_eq!(
            e.current().unwrap().rect().center(),
            Point::new(100.0, 50.0)
        );
    }
}

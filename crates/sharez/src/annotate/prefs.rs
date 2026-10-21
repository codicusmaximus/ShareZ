//! The annotator's toolbar settings and last tool, kept between runs in
//! `~/.config/sharez/annotate.ini`, as ShareX keeps its `AnnotationOptions`.

use std::path::PathBuf;

use gtk::glib;

use super::shapes::{
    Align, ArrowHead, BorderStyle, Color, Options, Point, StepType, TextStyle, Tool,
};

const GROUP: &str = "annotate";

fn path() -> PathBuf {
    glib::user_config_dir().join("sharez").join("annotate.ini")
}

pub fn load() -> (Options, Tool) {
    let file = glib::KeyFile::new();
    let mut o = Options::default();
    let mut tool = Tool::Rectangle;
    if file
        .load_from_file(path(), glib::KeyFileFlags::NONE)
        .is_ok()
    {
        read(&file, &mut o, &mut tool);
    }
    (o, tool)
}

pub fn save(o: &Options, tool: Tool) {
    let file = glib::KeyFile::new();
    write(&file, o, tool);
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = file.save_to_file(&path) {
        eprintln!("Could not save {}: {e}", path.display());
    }
}

struct Reader<'a>(&'a glib::KeyFile);

impl Reader<'_> {
    fn int(&self, key: &str, v: &mut i32, min: i32, max: i32) {
        if let Ok(n) = self.0.integer(GROUP, key) {
            *v = n.clamp(min, max);
        }
    }

    fn bool(&self, key: &str, v: &mut bool) {
        if let Ok(b) = self.0.boolean(GROUP, key) {
            *v = b;
        }
    }

    fn color(&self, key: &str, v: &mut Color) {
        if let Some(c) = self.string(key).as_deref().and_then(Color::from_hex) {
            *v = c;
        }
    }

    fn string(&self, key: &str) -> Option<String> {
        self.0.string(GROUP, key).ok().map(|s| s.to_string())
    }

    fn text(&self, prefix: &str, t: &mut TextStyle) {
        if let Some(f) = self
            .string(&format!("{prefix}-font"))
            .filter(|f| !f.is_empty())
        {
            t.font = f;
        }
        self.int(&format!("{prefix}-size"), &mut t.size, 1, 500);
        self.color(&format!("{prefix}-color"), &mut t.color);
        self.bool(&format!("{prefix}-bold"), &mut t.bold);
        self.bool(&format!("{prefix}-italic"), &mut t.italic);
        self.bool(&format!("{prefix}-underline"), &mut t.underline);
        if let Some(a) = self
            .string(&format!("{prefix}-align"))
            .and_then(|a| Align::from_id(&a))
        {
            t.align = a;
        }
        if let Some(a) = self
            .string(&format!("{prefix}-align-vertical"))
            .and_then(|a| Align::from_id(&a))
        {
            t.align_vertical = a;
        }
    }
}

fn read(file: &glib::KeyFile, o: &mut Options, tool: &mut Tool) {
    let r = Reader(file);
    if let Some(t) = r.string("tool").and_then(|t| Tool::from_id(&t)) {
        *tool = t;
    }
    r.color("border-color", &mut o.border_color);
    r.int("border-size", &mut o.border_size, 0, 20);
    if let Some(s) = r
        .string("border-style")
        .and_then(|s| BorderStyle::from_id(&s))
    {
        o.border_style = s;
    }
    r.color("fill-color", &mut o.fill_color);
    r.int("corner-radius", &mut o.corner_radius, 0, 150);
    r.bool("shadow", &mut o.shadow);
    r.color("shadow-color", &mut o.shadow_color);
    let (mut sx, mut sy) = (o.shadow_offset.x as i32, o.shadow_offset.y as i32);
    r.int("shadow-offset-x", &mut sx, -100, 100);
    r.int("shadow-offset-y", &mut sy, -100, 100);
    o.shadow_offset = Point::new(f64::from(sx), f64::from(sy));
    r.int("line-center-points", &mut o.line_center_points, 0, 5);
    if let Some(a) = r.string("arrow-head").and_then(|a| ArrowHead::from_id(&a)) {
        o.arrow_head = a;
    }
    r.text("text-outline", &mut o.text_outline);
    r.color(
        "text-outline-border-color",
        &mut o.text_outline_border_color,
    );
    r.int(
        "text-outline-border-size",
        &mut o.text_outline_border_size,
        0,
        20,
    );
    r.text("text", &mut o.text);
    r.color("text-border-color", &mut o.text_border_color);
    r.int("text-border-size", &mut o.text_border_size, 0, 20);
    r.color("text-fill-color", &mut o.text_fill_color);
    r.color("step-border-color", &mut o.step_border_color);
    r.int("step-border-size", &mut o.step_border_size, 0, 20);
    r.color("step-fill-color", &mut o.step_fill_color);
    r.int("step-font-size", &mut o.step_font_size, 10, 300);
    if let Some(t) = r.string("step-type").and_then(|t| StepType::from_id(&t)) {
        o.step_type = t;
    }
    r.int("magnify-strength", &mut o.magnify_strength, 100, 1000);
    r.int("blur-radius", &mut o.blur_radius, 3, 199);
    r.int("pixelate-size", &mut o.pixelate_size, 2, 10000);
    r.color("highlight-color", &mut o.highlight_color);
}

fn write_text(file: &glib::KeyFile, prefix: &str, t: &TextStyle) {
    file.set_string(GROUP, &format!("{prefix}-font"), &t.font);
    file.set_integer(GROUP, &format!("{prefix}-size"), t.size);
    file.set_string(GROUP, &format!("{prefix}-color"), &t.color.hex());
    file.set_boolean(GROUP, &format!("{prefix}-bold"), t.bold);
    file.set_boolean(GROUP, &format!("{prefix}-italic"), t.italic);
    file.set_boolean(GROUP, &format!("{prefix}-underline"), t.underline);
    file.set_string(GROUP, &format!("{prefix}-align"), t.align.id());
    file.set_string(
        GROUP,
        &format!("{prefix}-align-vertical"),
        t.align_vertical.id(),
    );
}

fn write(file: &glib::KeyFile, o: &Options, tool: Tool) {
    let color = |k: &str, c: Color| file.set_string(GROUP, k, &c.hex());
    file.set_string(GROUP, "tool", tool.id());
    color("border-color", o.border_color);
    file.set_integer(GROUP, "border-size", o.border_size);
    file.set_string(GROUP, "border-style", o.border_style.id());
    color("fill-color", o.fill_color);
    file.set_integer(GROUP, "corner-radius", o.corner_radius);
    file.set_boolean(GROUP, "shadow", o.shadow);
    color("shadow-color", o.shadow_color);
    file.set_integer(GROUP, "shadow-offset-x", o.shadow_offset.x as i32);
    file.set_integer(GROUP, "shadow-offset-y", o.shadow_offset.y as i32);
    file.set_integer(GROUP, "line-center-points", o.line_center_points);
    file.set_string(GROUP, "arrow-head", o.arrow_head.id());
    write_text(file, "text-outline", &o.text_outline);
    color("text-outline-border-color", o.text_outline_border_color);
    file.set_integer(
        GROUP,
        "text-outline-border-size",
        o.text_outline_border_size,
    );
    write_text(file, "text", &o.text);
    color("text-border-color", o.text_border_color);
    file.set_integer(GROUP, "text-border-size", o.text_border_size);
    color("text-fill-color", o.text_fill_color);
    color("step-border-color", o.step_border_color);
    file.set_integer(GROUP, "step-border-size", o.step_border_size);
    color("step-fill-color", o.step_fill_color);
    file.set_integer(GROUP, "step-font-size", o.step_font_size);
    file.set_string(GROUP, "step-type", o.step_type.id());
    file.set_integer(GROUP, "magnify-strength", o.magnify_strength);
    file.set_integer(GROUP, "blur-radius", o.blur_radius);
    file.set_integer(GROUP, "pixelate-size", o.pixelate_size);
    color("highlight-color", o.highlight_color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_round_trip() {
        let mut o = Options {
            border_color: Color::rgba(1, 2, 3, 4),
            border_style: BorderStyle::DashDot,
            step_type: StepType::RomanLowercase,
            shadow_offset: Point::new(2.0, -3.0),
            ..Options::default()
        };
        o.text.font = "DejaVu Serif".into();
        o.text.align = Align::Far;
        let file = glib::KeyFile::new();
        write(&file, &o, Tool::Blur);
        let (mut back, mut tool) = (Options::default(), Tool::Rectangle);
        read(&file, &mut back, &mut tool);
        assert_eq!(back, o);
        assert_eq!(tool, Tool::Blur);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let file = glib::KeyFile::new();
        file.set_integer(GROUP, "border-size", 500);
        file.set_string(GROUP, "border-color", "nonsense");
        let (mut o, mut tool) = (Options::default(), Tool::Rectangle);
        read(&file, &mut o, &mut tool);
        assert_eq!(o.border_size, 20);
        assert_eq!(o.border_color, Color::PRIMARY);
    }
}

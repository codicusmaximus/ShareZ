//! The popup a hotkey can show after a screenshot, asking what to do with it.

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use adw::prelude::*;
use gtk::{gdk, gio, glib};

/// Letter keys pressed this soon after the popup opens are ignored, so a held hotkey that ends
/// in a letter can't also pick an answer.
const INPUT_DELAY: Duration = Duration::from_millis(400);

/// What the user chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Copy,
    Save,
    SaveAndCopy,
    /// Saved where the user chose.
    SaveAs(PathBuf),
    /// Save it, then open it in the default image viewer.
    SaveAndOpen,
    Discard,
}

impl Choice {
    /// Whether it puts the image on the clipboard, which has to happen while the popup has
    /// focus on Wayland.
    pub fn copies(&self) -> bool {
        matches!(self, Choice::Copy | Choice::SaveAndCopy)
    }
}

/// An answer the popup lists: its row, and the key that picks it.
struct Answer {
    title: &'static str,
    icon: &'static str,
    /// A GTK accelerator, for the keycap shown on the row.
    accel: &'static str,
    key: gdk::Key,
    choice: fn() -> Option<Choice>,
}

const ANSWERS: [Answer; 5] = [
    Answer {
        title: "Save and Copy",
        icon: "emblem-ok-symbolic",
        accel: "Return",
        key: gdk::Key::Return,
        choice: || Some(Choice::SaveAndCopy),
    },
    Answer {
        title: "Copy to Clipboard",
        icon: "edit-copy-symbolic",
        accel: "c",
        key: gdk::Key::c,
        choice: || Some(Choice::Copy),
    },
    Answer {
        title: "Save",
        icon: "document-save-symbolic",
        accel: "s",
        key: gdk::Key::s,
        choice: || Some(Choice::Save),
    },
    Answer {
        title: "Save As…",
        icon: "document-save-as-symbolic",
        accel: "a",
        key: gdk::Key::a,
        // Asks for a file first.
        choice: || None,
    },
    Answer {
        title: "Save and Open",
        icon: "document-open-symbolic",
        accel: "o",
        key: gdk::Key::o,
        choice: || Some(Choice::SaveAndOpen),
    },
];

type Done = Box<dyn FnOnce(Choice)>;

/// Shows the popup for `image`. `done` gets the answer just before the popup closes, while it
/// still has focus, so `done` can set the clipboard on Wayland. Closing the popup discards the
/// screenshot.
pub fn ask(
    app: &adw::Application,
    image: &gdk::MemoryTexture,
    done: impl FnOnce(Choice) + 'static,
) {
    let answers = list();
    let discard = list();
    let window = adw::Window::builder()
        .application(app)
        .title("ShareZ Screenshot")
        .default_width(440)
        .resizable(false)
        .content(&layout(image, &answers, &discard))
        .build();

    let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let finish = {
        let window = window.clone();
        let done = Rc::clone(&done);
        move |choice: Choice| {
            if let Some(done) = done.borrow_mut().take() {
                done(choice);
            }
            window.destroy();
        }
    };
    let finish: Rc<dyn Fn(Choice)> = Rc::new(finish);
    // Asks where to save, then finishes; cancelling the file dialog goes back to the popup.
    let save_as: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        window,
        #[strong]
        finish,
        move || save_as(&window, &finish)
    ));
    let pick = {
        let finish = Rc::clone(&finish);
        move |answer: &Answer| match (answer.choice)() {
            Some(choice) => finish(choice),
            None => save_as(),
        }
    };
    let pick = Rc::new(pick);

    let mut first = None;
    for (i, answer) in ANSWERS.iter().enumerate() {
        let row = row(answer.title, answer.icon, answer.accel);
        let pick = Rc::clone(&pick);
        row.connect_activated(move |_| pick(&ANSWERS[i]));
        answers.append(&row);
        first.get_or_insert(row);
    }
    let discard_row = row("Discard", "user-trash-symbolic", "Escape");
    discard_row.connect_activated(glib::clone!(
        #[strong]
        finish,
        move |_| finish(Choice::Discard)
    ));
    discard.append(&discard_row);

    window.add_controller(keys(finish, pick));
    // The window's close button discards it too.
    window.connect_close_request(move |_| {
        if let Some(done) = done.borrow_mut().take() {
            done(Choice::Discard);
        }
        glib::Propagation::Proceed
    });

    window.present();
    if let Some(first) = first {
        first.grab_focus();
    }
}

/// The most room the preview takes.
const PREVIEW: (f64, f64) = (404.0, 240.0);

/// The preview's size: the screenshot's shape, scaled down to fit, never up.
fn preview_size(width: i32, height: i32) -> (i32, i32) {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let scale = (PREVIEW.0 / w).min(PREVIEW.1 / h).min(1.0);
    (
        ((w * scale).round() as i32).max(1),
        ((h * scale).round() as i32).max(1),
    )
}

fn list() -> gtk::ListBox {
    gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build()
}

/// The screenshot with its size, above the lists of answers.
fn layout(
    image: &gdk::MemoryTexture,
    answers: &gtk::ListBox,
    discard: &gtk::ListBox,
) -> adw::ToolbarView {
    let picture = gtk::Picture::for_paintable(image);
    picture.set_content_fit(gtk::ContentFit::Fill);
    picture.set_can_shrink(true);
    let (width, height) = preview_size(image.width(), image.height());
    picture.set_size_request(width, height);
    let preview = gtk::Box::builder()
        .css_classes(["card"])
        .overflow(gtk::Overflow::Hidden)
        .halign(gtk::Align::Center)
        .build();
    preview.append(&picture);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(6)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    content.append(&preview);
    content.append(answers);
    content.append(discard);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Screenshot",
        &format!("{} × {}", image.width(), image.height()),
    )));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));
    view
}

/// Each answer's letter picks it, and Escape discards.
fn keys(
    finish: Rc<dyn Fn(Choice)>,
    pick: Rc<impl Fn(&Answer) + 'static>,
) -> gtk::EventControllerKey {
    let opened = Instant::now();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, state| {
        let held = gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK;
        if key == gdk::Key::Escape {
            finish(Choice::Discard);
            return glib::Propagation::Stop;
        }
        // Enter goes to whichever row has focus, as in any list.
        if state.intersects(held) || key == gdk::Key::Return {
            return glib::Propagation::Proceed;
        }
        let key = key.to_lower();
        let Some(answer) = ANSWERS.iter().find(|a| a.key == key) else {
            return glib::Propagation::Proceed;
        };
        if opened.elapsed() >= INPUT_DELAY {
            pick(answer);
        }
        glib::Propagation::Stop
    });
    keys
}

/// A row of the popup, with the key that picks it shown as a keycap.
fn row(title: &str, icon: &str, accel: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(icon));
    let keycap = gtk::ShortcutLabel::new(accel);
    keycap.set_valign(gtk::Align::Center);
    row.add_suffix(&keycap);
    row
}

fn save_as(window: &adw::Window, finish: &Rc<dyn Fn(Choice)>) {
    let Ok(suggested) = crate::output::new_file(glib::UserDirectory::Pictures, "png") else {
        return;
    };
    let dialog = gtk::FileDialog::builder()
        .title("Save Screenshot")
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
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("PNG Image"));
    filter.add_mime_type("image/png");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    let finish = Rc::clone(finish);
    dialog.save(Some(window), None::<&gio::Cancellable>, move |file| {
        if let Some(path) = file.ok().and_then(|f| f.path()) {
            finish(Choice::SaveAs(with_png_extension(path)));
        }
    });
}

/// Adds `.png` when the user typed a name without an extension.
fn with_png_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        path
    } else {
        path.with_extension("png")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_keep_their_shape() {
        assert_eq!(preview_size(1920, 1080), (404, 227));
        assert_eq!(preview_size(1000, 2000), (120, 240));
        assert_eq!(preview_size(50, 20), (50, 20));
    }

    #[test]
    fn answer_keys_are_distinct() {
        for a in &ANSWERS {
            assert_eq!(ANSWERS.iter().filter(|b| b.key == a.key).count(), 1);
            assert_ne!(a.key, gdk::Key::Escape);
        }
    }

    #[test]
    fn only_copying_answers_copy() {
        assert!(Choice::Copy.copies() && Choice::SaveAndCopy.copies());
        assert!(!Choice::Save.copies() && !Choice::SaveAndOpen.copies());
        assert!(!Choice::SaveAs("/tmp/a.png".into()).copies());
    }

    #[test]
    fn save_as_names_get_an_extension() {
        assert_eq!(
            with_png_extension("/a/shot".into()),
            PathBuf::from("/a/shot.png")
        );
        assert_eq!(
            with_png_extension("/a/shot.jpg".into()),
            PathBuf::from("/a/shot.jpg")
        );
    }
}

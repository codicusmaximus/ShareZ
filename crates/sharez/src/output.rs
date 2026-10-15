//! What happens to a capture: screenshots are saved as PNG under Pictures/ShareZ and recordings
//! under Videos/ShareZ, and either is always copied to the clipboard.

use std::{fs::File, io::BufWriter, path::PathBuf};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use sharez_core::{Frame, PixelRect};

/// A new, timestamped file name in the user's `kind` folder, under ShareZ/.
pub fn new_file(kind: glib::UserDirectory, extension: &str) -> Result<PathBuf, String> {
    let dir = glib::user_special_dir(kind)
        .unwrap_or_else(glib::home_dir)
        .join("ShareZ");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let name = chrono::Local::now().format("ShareZ_%Y-%m-%d_%H-%M-%S");
    Ok(dir.join(format!("{name}.{extension}")))
}

/// The file's name, for messages.
fn name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What clicking a "saved" message does first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reveal {
    /// Open the file in its default app (screenshots).
    Open,
    /// Show it selected in the file manager, ready to drag somewhere (recordings).
    InFolder,
}

impl Reveal {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::InFolder => "Show in Folder",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::Open => "app.open-file",
            Self::InFolder => "app.show-file",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Open => Self::InFolder,
            Self::InFolder => Self::Open,
        }
    }

    pub fn run(self, path: &std::path::Path) {
        match self {
            Self::Open => open(path),
            Self::InFolder => show_in_folder(path),
        }
    }
}

/// "Saved … and copied it", with a button that does `reveal`.
pub fn saved_toast(path: PathBuf, reveal: Reveal) -> adw::Toast {
    let toast = adw::Toast::builder()
        .title(format!("Saved {} and copied it", name(&path)))
        .button_label(reveal.label())
        .build();
    toast.connect_button_clicked(move |_| reveal.run(&path));
    toast
}

/// The same news as a desktop notification, for when the window is hidden. Clicking it does
/// `reveal`; a button does the other.
pub fn saved_notification(
    title: &str,
    path: &std::path::Path,
    reveal: Reveal,
) -> gio::Notification {
    let n = gio::Notification::new(title);
    n.set_body(Some(&format!("Saved {}", name(path))));
    let target = path.to_string_lossy().to_variant();
    n.set_default_action_and_target_value(reveal.action(), Some(&target));
    let other = reveal.other();
    n.add_button_with_target_value(other.label(), other.action(), Some(&target));
    n
}

/// Opens a file in its default app.
pub fn open(path: &std::path::Path) {
    gtk::FileLauncher::new(Some(&gio::File::for_path(path))).launch(
        None::<&gtk::Window>,
        None::<&gio::Cancellable>,
        |_| {},
    );
}

/// Opens the file's folder in the file manager with the file selected (through
/// `org.freedesktop.FileManager1`, which Files, Dolphin and most others answer), or just the folder
/// where that isn't available.
pub fn show_in_folder(path: &std::path::Path) {
    gtk::FileLauncher::new(Some(&gio::File::for_path(path))).open_containing_folder(
        None::<&gtk::Window>,
        None::<&gio::Cancellable>,
        |_| {},
    );
}

/// Copies a saved recording to the clipboard as a file, which chat apps and file managers paste.
pub fn copy_video(path: &std::path::Path) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let files = gdk::FileList::from_array(&[gio::File::for_path(path)]);
    display
        .clipboard()
        .set_content(Some(&gdk::ContentProvider::for_value(&files.to_value())))
        .ok();
}

fn write_png(path: &std::path::Path, image: &Frame) -> Result<(), String> {
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(&image.data))
        .map_err(|e| e.to_string())
}

/// Crops the region out of the frame and puts it on the clipboard. Call it while one of ShareZ's
/// windows has focus (the overlay): on Wayland only the focused app may set the clipboard.
pub fn copy_image(frame: &Frame, region: PixelRect) -> Result<Frame, String> {
    let image = frame
        .crop(region)
        .ok_or("The selected region is outside the screen.")?;
    Ok(copy_frame(image))
}

/// Puts an image on the clipboard, with the same caveat, and hands it back.
pub fn copy_frame(image: Frame) -> Frame {
    let texture = gdk::MemoryTexture::new(
        image.width as i32,
        image.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(&image.data[..]),
        image.width as usize * 4,
    );
    if let Some(display) = gdk::Display::default() {
        display.clipboard().set_texture(&texture);
    }
    image
}

/// Saves a screenshot as PNG under Pictures/ShareZ.
pub fn save_image(image: &Frame) -> Result<PathBuf, String> {
    let path = new_file(glib::UserDirectory::Pictures, "png")?;
    write_png(&path, image).map_err(|e| format!("Could not save {}: {e}", path.display()))?;
    Ok(path)
}

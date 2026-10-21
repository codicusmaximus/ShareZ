//! What happens to a capture: screenshots are saved as PNG under Pictures/ShareZ and recordings
//! under Videos/ShareZ, and copied to the clipboard. A hotkey can skip either for screenshots, or
//! ask (see `popup`).

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

/// "Saved … and copied it", or just "Saved …", with a button that does `reveal`.
pub fn saved_toast(path: PathBuf, copied: bool, reveal: Reveal) -> adw::Toast {
    let and = if copied { " and copied it" } else { "" };
    let toast = adw::Toast::builder()
        .title(format!("Saved {}{and}", name(&path)))
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

/// News of a screenshot that was only copied.
pub fn copied_notification() -> gio::Notification {
    let n = gio::Notification::new("Screenshot copied");
    n.set_body(Some("Paste it wherever it needs to go."));
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

/// Crops the region out of the frame.
pub fn crop(frame: &Frame, region: PixelRect) -> Result<Frame, String> {
    frame
        .crop(region)
        .ok_or_else(|| "The selected region is outside the screen.".into())
}

/// Puts an image on the clipboard and hands it back. Call it while one of ShareZ's windows has
/// focus (the overlay, the popup or the flash): on Wayland only the focused app may set the
/// clipboard.
pub fn copy_frame(image: Frame) -> Frame {
    copy_texture(&texture(&image));
    image
}

/// The same, for an image already made into a texture.
pub fn copy_texture(texture: &gdk::MemoryTexture) {
    if let Some(display) = gdk::Display::default() {
        display.clipboard().set_texture(texture);
    }
}

/// Saves a screenshot as PNG under Pictures/ShareZ.
pub fn save_image(image: &Frame) -> Result<PathBuf, String> {
    save_image_to(new_file(glib::UserDirectory::Pictures, "png")?, image)
}

/// Saves a screenshot as PNG where the user chose.
pub fn save_image_to(path: PathBuf, image: &Frame) -> Result<PathBuf, String> {
    write_png(&path, image).map_err(|e| format!("Could not save {}: {e}", path.display()))?;
    Ok(path)
}

/// Reads an image file, in any format GTK can load.
pub fn load_image(path: &std::path::Path) -> Result<Frame, String> {
    let texture = gdk::Texture::from_filename(path).map_err(|e| e.to_string())?;
    let mut downloader = gdk::TextureDownloader::new(&texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let (width, height) = (texture.width() as usize, texture.height() as usize);
    let mut data = Vec::with_capacity(width * height * 4);
    for row in bytes.chunks(stride).take(height) {
        data.extend_from_slice(&row[..width * 4]);
    }
    Ok(Frame::new(width as u32, height as u32, data))
}

/// The image as a texture, for showing or copying it.
pub fn texture(image: &Frame) -> gdk::MemoryTexture {
    gdk::MemoryTexture::new(
        image.width as i32,
        image.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(&image.data[..]),
        image.stride(),
    )
}

//! What happens to a capture: screenshots are saved as PNG under Pictures/ShareZ and recordings
//! under Videos/ShareZ, and copied to the clipboard. A hotkey can skip either for screenshots, or
//! ask (see `popup`).

use std::{fs::File, io::BufWriter, path::PathBuf};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use sharez_core::{Frame, PixelRect};

/// A new, timestamped file name in the user's `kind` folder, under ShareZ/. Names are to the
/// second, so a second capture in the same second gets `_2`, then `_3`, and so on.
pub fn new_file(kind: glib::UserDirectory, extension: &str) -> Result<PathBuf, String> {
    let dir = glib::user_special_dir(kind)
        .unwrap_or_else(glib::home_dir)
        .join("ShareZ");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let name = chrono::Local::now().format("ShareZ_%Y-%m-%d_%H-%M-%S");
    Ok(free_name(&dir, &name.to_string(), extension))
}

/// `dir/stem.extension`, or the first of `stem_2.extension`, `stem_3.extension`… that isn't taken.
fn free_name(dir: &std::path::Path, stem: &str, extension: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.{extension}"));
    for n in 2.. {
        if !path.exists() {
            break;
        }
        path = dir.join(format!("{stem}_{n}.{extension}"));
    }
    path
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

/// Copies a saved file to the clipboard as the file itself, so pasting it in a file manager, a
/// chat app or a browser's upload box pastes the file. Call it while ShareZ has focus, as with
/// `copy_frame`.
pub fn copy_file(path: &std::path::Path) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let file = gio::File::for_path(path);
    // GTK offers a file list as text/uri-list (browsers, Electron and Qt apps, Dolphin), the
    // portal's file transfer (sandboxed apps) and the path as text. GNOME's Files and other GTK 3
    // file managers want their own target on top.
    let files = gdk::FileList::from_array(std::slice::from_ref(&file));
    let gnome = glib::Bytes::from_owned(gnome_copied_files(&file.uri()).into_bytes());
    let content = gdk::ContentProvider::new_union(&[
        gdk::ContentProvider::for_value(&files.to_value()),
        gdk::ContentProvider::for_bytes("x-special/gnome-copied-files", &gnome),
    ]);
    display.clipboard().set_content(Some(&content)).ok();
}

/// The `x-special/gnome-copied-files` form of a copied file: the operation, then its URI.
fn gnome_copied_files(uri: &str) -> String {
    format!("copy\n{uri}")
}

fn write_png(file: File, image: &Frame) -> Result<(), String> {
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

/// Saves a screenshot as PNG under Pictures/ShareZ, never over an existing file.
pub fn save_image(image: &Frame) -> Result<PathBuf, String> {
    loop {
        let path = new_file(glib::UserDirectory::Pictures, "png")?;
        // Another capture can take the name between picking it and creating it: pick again.
        match File::create_new(&path) {
            Ok(file) => return write_to(path, file, image),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("Could not save {}: {e}", path.display())),
        }
    }
}

/// Saves a screenshot as PNG where the user chose, replacing what's there (the file chooser has
/// already asked).
pub fn save_image_to(path: PathBuf, image: &Frame) -> Result<PathBuf, String> {
    let file =
        File::create(&path).map_err(|e| format!("Could not save {}: {e}", path.display()))?;
    write_to(path, file, image)
}

fn write_to(path: PathBuf, file: File, image: &Frame) -> Result<PathBuf, String> {
    write_png(file, image).map_err(|e| format!("Could not save {}: {e}", path.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_files_are_offered_the_way_gnome_files_pastes_them() {
        assert_eq!(
            gnome_copied_files("file:///home/me/Videos/ShareZ/a%20b.webm"),
            "copy\nfile:///home/me/Videos/ShareZ/a%20b.webm"
        );
    }

    #[test]
    fn same_second_names_get_a_number() {
        let dir = std::env::temp_dir().join(format!("sharez-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = free_name(&dir, "ShareZ_x", "png");
        assert_eq!(first, dir.join("ShareZ_x.png"));
        File::create_new(&first).unwrap();
        let second = free_name(&dir, "ShareZ_x", "png");
        assert_eq!(second, dir.join("ShareZ_x_2.png"));
        File::create_new(&second).unwrap();
        assert_eq!(
            free_name(&dir, "ShareZ_x", "png"),
            dir.join("ShareZ_x_3.png")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

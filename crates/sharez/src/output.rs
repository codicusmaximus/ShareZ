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

/// Copies saved files to the clipboard as the files themselves, so pasting them in a chat app
/// (Discord, Telegram), a browser's upload box or a file manager attaches the files. Call it while
/// ShareZ has focus, as with `copy_frame`. `transfer` is the portal key for the same files, from
/// `file_transfer::start`, for sandboxed apps; the clipboard stops it once it is replaced.
///
/// The clipboard holds where the files are, not their bytes: whoever pastes reads them from disk,
/// so they must stay where they are, and ShareZ keeps serving the clipboard while it runs.
pub fn copy_files(paths: &[PathBuf], transfer: Option<String>) {
    let Some(display) = gdk::Display::default() else {
        if let Some(key) = transfer {
            crate::file_transfer::stop(key);
        }
        return;
    };
    let formats = match FileFormats::new(paths) {
        Ok(formats) => formats,
        Err(e) => {
            eprintln!("Could not copy {e}");
            if let Some(key) = transfer {
                crate::file_transfer::stop(key);
            }
            return;
        }
    };
    let bytes = |mime: &str, data: String| {
        gdk::ContentProvider::for_bytes(mime, &glib::Bytes::from_owned(data.into_bytes()))
    };
    // text/uri-list is what Discord, Telegram, browsers and Qt apps read; GNOME's Files and other
    // GTK file managers want their own target, and Dolphin wants to know it's a copy, not a cut.
    let mut providers = vec![
        bytes("text/uri-list", formats.uri_list),
        bytes("x-special/gnome-copied-files", formats.gnome),
        bytes("application/x-kde-cutselection", "0".to_owned()),
    ];
    if let Some(key) = &transfer {
        providers.push(bytes(PORTAL_MIME, key.clone()));
    }
    // A string offers text/plain;charset=utf-8 and text/plain (and on X11 UTF8_STRING and the
    // rest), so pasting into a text box gives the path.
    providers.push(gdk::ContentProvider::for_value(&formats.text.to_value()));
    let content = gdk::ContentProvider::new_union(&providers);
    let clipboard = display.clipboard();
    hold_transfer(&clipboard, &content, transfer);
    if let Err(e) = clipboard.set_content(Some(&content)) {
        eprintln!("Could not set the clipboard: {e}");
    }
}

/// What a sandboxed app reads to get the files through the document portal.
const PORTAL_MIME: &str = "application/vnd.portal.filetransfer";

/// Copied files in the forms the clipboard offers them in, the way file managers and Qt copy them.
#[derive(Debug, PartialEq, Eq)]
struct FileFormats {
    /// `text/uri-list`: each file's percent-encoded `file://` URI, each line ended with CRLF.
    uri_list: String,
    /// `x-special/gnome-copied-files`: `copy`, then the URIs, a line each, with no newline at the
    /// end.
    gnome: String,
    /// `text/plain`: the bare paths, a line each, with no newline at the end.
    text: String,
}

impl FileFormats {
    fn new(paths: &[PathBuf]) -> Result<Self, String> {
        let uris = paths
            .iter()
            .map(|p| {
                glib::filename_to_uri(p, None)
                    .map(String::from)
                    .map_err(|e| format!("{}: {e}", p.display()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let uri_list = uris.iter().flat_map(|u| [u.as_str(), "\r\n"]).collect();
        let gnome = std::iter::once("copy")
            .chain(uris.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        let text = paths
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Self {
            uri_list,
            gnome,
            text,
        })
    }
}

thread_local! {
    /// The clipboard content ShareZ set with a portal transfer, and the transfer's key.
    static TRANSFER: std::cell::RefCell<Option<(gdk::ContentProvider, String)>> =
        const { std::cell::RefCell::new(None) };
    /// Whether `TRANSFER` is watching the clipboard yet.
    static WATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Keeps `key`'s transfer going while `content` is on the clipboard, and stops the one before.
fn hold_transfer(clipboard: &gdk::Clipboard, content: &gdk::ContentProvider, key: Option<String>) {
    if let Some((_, old)) = TRANSFER.take() {
        crate::file_transfer::stop(old);
    }
    let Some(key) = key else {
        return;
    };
    TRANSFER.set(Some((content.clone(), key)));
    if !WATCHING.replace(true) {
        clipboard.connect_changed(|clipboard| {
            let replaced = TRANSFER.with_borrow(|held| {
                held.as_ref()
                    .is_some_and(|(ours, _)| clipboard.content().as_ref() != Some(ours))
            });
            if replaced && let Some((_, key)) = TRANSFER.take() {
                crate::file_transfer::stop(key);
            }
        });
    }
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
    fn a_copied_file_is_offered_the_way_discord_and_file_managers_paste_it() {
        let formats =
            FileFormats::new(&[PathBuf::from("/home/me/Videos/ShareZ/ShareZ_1.mp4")]).unwrap();
        assert_eq!(
            formats,
            FileFormats {
                uri_list: "file:///home/me/Videos/ShareZ/ShareZ_1.mp4\r\n".into(),
                gnome: "copy\nfile:///home/me/Videos/ShareZ/ShareZ_1.mp4".into(),
                text: "/home/me/Videos/ShareZ/ShareZ_1.mp4".into(),
            }
        );
    }

    #[test]
    fn copied_files_are_percent_encoded_one_a_line() {
        let formats = FileFormats::new(&[
            PathBuf::from("/home/me/a.mp4"),
            PathBuf::from("/home/me/My Clip #2 é.mp4"),
        ])
        .unwrap();
        assert_eq!(
            formats.uri_list,
            "file:///home/me/a.mp4\r\nfile:///home/me/My%20Clip%20%232%20%C3%A9.mp4\r\n"
        );
        assert_eq!(
            formats.gnome,
            "copy\nfile:///home/me/a.mp4\nfile:///home/me/My%20Clip%20%232%20%C3%A9.mp4"
        );
        assert_eq!(formats.text, "/home/me/a.mp4\n/home/me/My Clip #2 é.mp4");
    }

    #[test]
    fn only_absolute_paths_can_be_copied() {
        assert!(FileFormats::new(&[PathBuf::from("a.mp4")]).is_err());
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

//! What happens to a capture: screenshots are saved as PNG under Pictures/ShareZ and recordings
//! under Videos/ShareZ, and either is copied to the clipboard.

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

/// "Saved … and copied it", with a button that opens the file.
fn saved_toast(path: PathBuf) -> adw::Toast {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let toast = adw::Toast::builder()
        .title(format!("Saved {name} and copied it"))
        .button_label("Open")
        .build();
    toast.connect_button_clicked(move |_| {
        gtk::FileLauncher::new(Some(&gio::File::for_path(&path))).launch(
            None::<&gtk::Window>,
            None::<&gio::Cancellable>,
            |_| {},
        );
    });
    toast
}

/// Copies a saved recording to the clipboard as a file, which chat apps and file managers paste.
pub fn deliver_video(window: &impl IsA<gtk::Widget>, path: PathBuf) -> adw::Toast {
    let files = gdk::FileList::from_array(&[gio::File::for_path(&path)]);
    window
        .clipboard()
        .set_content(Some(&gdk::ContentProvider::for_value(&files.to_value())))
        .ok();
    saved_toast(path)
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

/// Saves and copies the region, returning a toast that says so.
pub fn deliver(
    window: &impl IsA<gtk::Widget>,
    frame: &Frame,
    region: PixelRect,
) -> Result<adw::Toast, String> {
    let image = frame
        .crop(region)
        .ok_or("The selected region is outside the screen.")?;

    let path = new_file(glib::UserDirectory::Pictures, "png")?;
    write_png(&path, &image).map_err(|e| format!("Could not save {}: {e}", path.display()))?;

    let texture = gdk::MemoryTexture::new(
        image.width as i32,
        image.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(image.data),
        image.width as usize * 4,
    );
    window.clipboard().set_texture(&texture);

    Ok(saved_toast(path))
}

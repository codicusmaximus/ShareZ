//! What happens to a captured region: saved as PNG under Pictures/ShareZ and copied to the
//! clipboard.

use std::{fs::File, io::BufWriter, path::PathBuf};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use sharez_core::{Frame, PixelRect};

fn save_dir() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Pictures)
        .unwrap_or_else(glib::home_dir)
        .join("ShareZ")
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

    let dir = save_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let name = chrono::Local::now()
        .format("ShareZ_%Y-%m-%d_%H-%M-%S.png")
        .to_string();
    let path = dir.join(&name);
    write_png(&path, &image).map_err(|e| format!("Could not save {}: {e}", path.display()))?;

    let texture = gdk::MemoryTexture::new(
        image.width as i32,
        image.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(image.data),
        image.width as usize * 4,
    );
    window.clipboard().set_texture(&texture);

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
    Ok(toast)
}

use std::{fs::File, io::BufReader, path::PathBuf};

use ashpd::desktop::screenshot::Screenshot;
use sharez_core::Frame;

use crate::{Error, Result};

pub async fn freeze() -> Result<Frame> {
    let response = Screenshot::request()
        .interactive(false)
        .modal(false)
        .send()
        .await?
        .response()?;
    let uri = response.uri().as_str();
    let path: PathBuf = url::Url::parse(uri)
        .ok()
        .and_then(|u| u.to_file_path().ok())
        .ok_or_else(|| Error::NotAFile(uri.to_owned()))?;
    let decoded = decode_png(&path);
    // The portal saves into the user's Pictures folder, and two shots in the same second get the
    // same name. The file is ours alone, so remove it whether or not decoding worked.
    let _ = std::fs::remove_file(&path);
    decoded
}

fn decode_png(path: &std::path::Path) -> Result<Frame> {
    let mut decoder = png::Decoder::new(BufReader::new(File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let data = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        other => return Err(Error::UnsupportedFormat(format!("PNG {other:?}"))),
    };
    Ok(Frame::new(info.width, info.height, data))
}

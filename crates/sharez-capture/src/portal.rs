use std::{fs::File, io::BufReader, path::PathBuf};

use ashpd::desktop::{ResponseError, screenshot::Screenshot};
use sharez_core::Frame;

use crate::{Error, Result};

pub async fn freeze() -> Result<Frame> {
    let response = Screenshot::request()
        .interactive(false)
        .modal(false)
        .send()
        .await?
        .response()
        .map_err(|e| match e {
            // GNOME answers a refused permission, or a permission dialog that failed, with
            // "other" and no details.
            ashpd::Error::Response(ResponseError::Other) => Error::Denied,
            e => e.into(),
        })?;
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
    Ok(sharez_core::png::decode(BufReader::new(File::open(path)?))?)
}

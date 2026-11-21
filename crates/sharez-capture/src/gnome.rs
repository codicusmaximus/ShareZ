//! GNOME with the ShareZ Helper extension: the extension paints the screen itself and encodes it
//! as several PNG strips side by side on GNOME's worker threads. The screenshot portal has GNOME
//! encode one PNG on one thread and save it to disk first, which takes about two seconds for a
//! busy 4K screen.

use std::{io::Cursor, time::Duration};

use futures_lite::FutureExt;
use sharez_core::Frame;
use zbus::Connection;

use crate::{Error, Result};

const HELPER: &str = "io.github.codicusmaximus.ShareZ.ShellHelper";
const HELPER_PATH: &str = "/io/github/codicusmaximus/ShareZ/ShellHelper";
/// Painting and encoding the screen. Far longer than it should ever take.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The screen, or `None` if the helper isn't there or can't capture the screen (an older
/// version, or one GNOME hasn't reloaded since an update), so the portal has to.
pub async fn freeze() -> Result<Option<Frame>> {
    let Ok(conn) = Connection::session().await else {
        return Ok(None);
    };
    let call = async {
        conn.call_method(
            Some(HELPER),
            HELPER_PATH,
            Some(HELPER),
            "CaptureScreen",
            &(),
        )
        .await
    };
    let timeout = async {
        async_io::Timer::after(TIMEOUT).await;
        Err(zbus::Error::InputOutput(
            std::io::Error::from(std::io::ErrorKind::TimedOut).into(),
        ))
    };
    let reply = match call.or(timeout).await {
        Ok(reply) => reply,
        Err(e) => {
            if !matches!(&e, zbus::Error::MethodError(name, ..) if is_missing(name.as_str())) {
                eprintln!("The ShareZ Helper couldn't capture the screen, so the portal will: {e}");
            }
            return Ok(None);
        }
    };
    let body = reply.body();
    // Borrowed: copying 10 MB or more into vectors byte by byte takes zbus a good 150 ms.
    let strips: Vec<&[u8]> = body
        .deserialize()
        .map_err(|e| Error::Helper(e.to_string()))?;
    stitch(&strips).map(Some)
}

/// No helper, or one without `CaptureScreen`.
fn is_missing(error: &str) -> bool {
    matches!(
        error,
        "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.UnknownMethod"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
    )
}

/// Decodes PNG strips side by side and stacks them, top first.
fn stitch(strips: &[&[u8]]) -> Result<Frame> {
    let decoded: Vec<Result<Frame>> = std::thread::scope(|s| {
        let handles: Vec<_> = strips
            .iter()
            .map(|png| s.spawn(move || Ok(sharez_core::png::decode(Cursor::new(png))?)))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(Error::Helper("decoder panicked".into())))
            })
            .collect()
    });
    let frames = decoded.into_iter().collect::<Result<Vec<_>>>()?;
    let Some(width) = frames.first().map(|f| f.width) else {
        return Err(Error::Helper("no image".into()));
    };
    if frames.iter().any(|f| f.width != width) {
        return Err(Error::Helper("strips of different widths".into()));
    }
    let height = frames.iter().map(|f| f.height).sum();
    if height > sharez_core::png::MAX_SIDE {
        return Err(sharez_core::png::Error::TooLarge { width, height }.into());
    }
    let mut data = Vec::new();
    data.try_reserve_exact(frames.iter().map(|f| f.data.len()).sum())
        .map_err(|_| sharez_core::png::Error::TooLarge { width, height })?;
    for f in &frames {
        data.extend_from_slice(&f.data);
    }
    Ok(Frame::new(width, height, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, fill: u8) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![fill; (width * height * 4) as usize])
            .unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn stacks_strips_top_first() {
        let frame = stitch(&[&png(3, 2, 1), &png(3, 1, 2)]).unwrap();
        assert_eq!((frame.width, frame.height), (3, 3));
        assert!(frame.data[..24].iter().all(|&b| b == 1));
        assert!(frame.data[24..].iter().all(|&b| b == 2));
    }

    #[test]
    fn refuses_strips_that_dont_line_up() {
        assert!(stitch(&[&png(3, 1, 0), &png(4, 1, 0)]).is_err());
        assert!(stitch(&[]).is_err());
    }
}

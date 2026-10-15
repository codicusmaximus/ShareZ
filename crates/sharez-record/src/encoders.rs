//! Choosing a video and audio encoder from what is installed.
//!
//! The spike found no hardware H.264 encoder and no x264 on a stock Fedora, only `openh264enc` and
//! `vp8enc`, so the choice is made at runtime. MP4 with H.264 is preferred because it plays
//! everywhere a recording is likely to be shared; WebM with VP8 is the fallback.

use gstreamer::{self as gst, prelude::*};

use crate::{Error, Missing, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Video {
    X264,
    OpenH264,
    Vp8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioCodec {
    FdkAac,
    AvAac,
    VoAac,
    Opus,
}

/// The encoders and container for a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    video: Video,
    audio: AudioCodec,
}

/// Bits per pixel per frame for bitrate-controlled encoders. Screen content compresses well, and
/// this keeps text sharp at 30 fps without making large files.
const BITS_PER_PIXEL: f64 = 0.12;
const MIN_BITRATE: f64 = 2_000_000.0;
const MAX_BITRATE: f64 = 40_000_000.0;

pub(crate) fn has(element: &str) -> bool {
    gst::ElementFactory::find(element).is_some()
}

impl Format {
    /// Picks the best installed encoders.
    pub fn detect() -> Result<Self> {
        crate::init()?;
        Self::pick(has)
    }

    pub(crate) fn pick(has: impl Fn(&str) -> bool) -> Result<Self> {
        let video = if has("x264enc") {
            Video::X264
        } else if has("openh264enc") {
            Video::OpenH264
        } else if has("vp8enc") {
            Video::Vp8
        } else {
            return Err(Error::Missing(vec![Missing {
                element: "an H.264 encoder",
                fedora: "gstreamer1-plugin-openh264",
                debian: "gstreamer1.0-plugins-ugly",
            }]));
        };
        let audio = match video {
            Video::Vp8 => AudioCodec::Opus,
            Video::X264 | Video::OpenH264 => [
                ("fdkaacenc", AudioCodec::FdkAac),
                ("avenc_aac", AudioCodec::AvAac),
                ("voaacenc", AudioCodec::VoAac),
            ]
            .into_iter()
            .find(|(e, _)| has(e))
            .map_or(AudioCodec::Opus, |(_, c)| c),
        };
        Ok(Self { video, audio })
    }

    #[must_use]
    pub fn extension(self) -> &'static str {
        match self.video {
            Video::X264 | Video::OpenH264 => "mp4",
            Video::Vp8 => "webm",
        }
    }

    /// The encoder chain, ending in something a muxer accepts. The encoder is named `venc`.
    pub(crate) fn video_encoder(self) -> &'static str {
        match self.video {
            Video::X264 => {
                "x264enc name=venc speed-preset=veryfast tune=zerolatency pass=qual quantizer=21 \
                 key-int-max=120 threads=0 ! h264parse"
            }
            Video::OpenH264 => {
                "openh264enc name=venc usage-type=screen rate-control=bitrate complexity=low \
                 gop-size=120 ! h264parse"
            }
            Video::Vp8 => {
                "vp8enc name=venc deadline=1 cpu-used=8 end-usage=cbr keyframe-max-dist=120 \
                 threads=4"
            }
        }
    }

    pub(crate) fn audio_encoder(self) -> &'static str {
        match self.audio {
            AudioCodec::FdkAac => "fdkaacenc bitrate=160000 ! aacparse",
            AudioCodec::AvAac => "avenc_aac bitrate=160000 ! aacparse",
            AudioCodec::VoAac => "voaacenc bitrate=160000 ! aacparse",
            AudioCodec::Opus => "opusenc bitrate=128000",
        }
    }

    pub(crate) fn muxer(self) -> &'static str {
        match self.video {
            // Headers first, so the file can play while it is still downloading.
            Video::X264 | Video::OpenH264 => "mp4mux name=mux faststart=true",
            Video::Vp8 => "webmmux name=mux",
        }
    }

    /// The parsers and muxer this format needs besides its encoders, with the Fedora and
    /// Debian packages that provide them.
    pub(crate) fn helpers(self) -> Vec<(&'static str, (&'static str, &'static str))> {
        let good = ("gstreamer1-plugins-good", "gstreamer1.0-plugins-good");
        let bad = ("gstreamer1-plugins-bad-free", "gstreamer1.0-plugins-bad");
        let mut v = match self.video {
            Video::X264 | Video::OpenH264 => vec![("h264parse", bad), ("mp4mux", good)],
            Video::Vp8 => vec![("webmmux", good)],
        };
        if matches!(
            self.audio,
            AudioCodec::FdkAac | AudioCodec::AvAac | AudioCodec::VoAac
        ) {
            v.push(("aacparse", good));
        }
        v
    }

    /// Sets the encoder's bitrate for the cropped size, where it has one.
    pub(crate) fn set_bitrate(self, encoder: &gst::Element, width: u32, height: u32, fps: u32) {
        let bits = (f64::from(width) * f64::from(height) * f64::from(fps) * BITS_PER_PIXEL)
            .clamp(MIN_BITRATE, MAX_BITRATE) as u32;
        match self.video {
            Video::X264 => {} // Constant quality, no bitrate.
            Video::OpenH264 => encoder.set_property("bitrate", bits),
            Video::Vp8 => encoder.set_property("target-bitrate", bits as i32),
        }
    }
}

/// Every installed video encoder, each with the best audio encoder for it.
#[cfg(test)]
pub(crate) fn all_installed() -> Vec<Format> {
    let video = ["x264enc", "openh264enc", "vp8enc"];
    video
        .iter()
        .filter(|v| has(v))
        .filter_map(|v| Format::pick(|e| e == *v || (!video.contains(&e) && has(e))).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_fedora_gets_mp4() {
        // What the spike found: openh264 and vp8, no x264, no AAC encoder.
        let f = Format::pick(|e| matches!(e, "openh264enc" | "vp8enc")).unwrap();
        assert_eq!(f.extension(), "mp4");
        assert!(f.video_encoder().starts_with("openh264enc"));
        assert!(f.audio_encoder().starts_with("opusenc"));
    }

    #[test]
    fn x264_and_aac_preferred() {
        let f = Format::pick(|_| true).unwrap();
        assert!(f.video_encoder().starts_with("x264enc"));
        assert!(f.audio_encoder().starts_with("fdkaacenc"));
    }

    #[test]
    fn vp8_only_gets_webm_and_opus() {
        let f = Format::pick(|e| e == "vp8enc").unwrap();
        assert_eq!(f.extension(), "webm");
        assert!(f.audio_encoder().starts_with("opusenc"));
    }

    #[test]
    fn nothing_installed_names_a_package() {
        let Err(Error::Missing(m)) = Format::pick(|_| false) else {
            panic!("expected a missing encoder");
        };
        assert_eq!(m[0].fedora, "gstreamer1-plugin-openh264");
    }
}

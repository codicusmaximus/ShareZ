//! The GStreamer pipeline, as a `gst-launch` style description.
//!
//! The video branch always has a `videocrop` named `crop`, so the crop can be set once the real
//! buffer size is known: on Wayland the stream's buffers can be larger than its logical size
//! (fractional scaling), and that is only known when the caps arrive.

use crate::{
    Format, Missing,
    encoders::has,
    limiter::{CHANNELS, RATE},
};

/// Where the picture comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VideoIn {
    /// A portal screencast. `fd` must stay open while the pipeline runs.
    PipeWire { fd: i32, node: u32 },
    /// A region of the X11 root window, in pixels (`end` inclusive).
    X11 {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    #[cfg(test)]
    Test { width: u32, height: u32 },
}

/// Where a sound track comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AudioIn {
    /// PipeWire's PulseAudio interface; `None` is the default input.
    Pulse(Option<&'static str>),
    #[cfg(test)]
    Test(u32),
}

/// The name of the video source element, so its errors can be told apart.
pub(crate) const VIDEO: &str = "video";

/// The output's monitor in PulseAudio naming, which PipeWire understands too.
pub(crate) const DESKTOP: AudioIn = AudioIn::Pulse(Some("@DEFAULT_MONITOR@"));
pub(crate) const MICROPHONE: AudioIn = AudioIn::Pulse(None);

impl VideoIn {
    fn source(&self) -> String {
        match self {
            // keepalive-time resends the last frame while nothing changes on screen, which
            // compositors don't send on their own; videorate then holds a steady frame rate.
            Self::PipeWire { fd, node } => {
                format!(
                    "pipewiresrc name={VIDEO} fd={fd} path={node} do-timestamp=true \
                     keepalive-time=1000"
                )
            }
            Self::X11 {
                x,
                y,
                width,
                height,
            } => format!(
                "ximagesrc name={VIDEO} startx={x} starty={y} endx={} endy={} \
                 use-damage=false show-pointer=true",
                x + width - 1,
                y + height - 1
            ),
            #[cfg(test)]
            Self::Test { width, height } => format!(
                "videotestsrc name={VIDEO} is-live=true pattern=ball ! \
                 video/x-raw,width={width},height={height}"
            ),
        }
    }

    fn element(&self) -> &'static str {
        match self {
            Self::PipeWire { .. } => "pipewiresrc",
            Self::X11 { .. } => "ximagesrc",
            #[cfg(test)]
            Self::Test { .. } => "videotestsrc",
        }
    }
}

impl AudioIn {
    fn source(&self) -> String {
        match self {
            Self::Pulse(Some(device)) => format!("pulsesrc device={device}"),
            Self::Pulse(None) => "pulsesrc".into(),
            #[cfg(test)]
            Self::Test(freq) => format!("audiotestsrc is-live=true freq={freq}"),
        }
    }

    fn element(&self) -> &'static str {
        match self {
            Self::Pulse(_) => "pulsesrc",
            #[cfg(test)]
            Self::Test(_) => "audiotestsrc",
        }
    }
}

/// The name of the element whose output the limiter works on.
pub(crate) const LIMITER: &str = "limiter";

/// The queue in front of the video encoder. When the encoder can't keep up (a busy, full-screen
/// video), it drops the oldest frame rather than holding up the source, so the video stays in
/// step with real time and never falls behind the sound.
const VIDEO_QUEUE: &str =
    "queue leaky=downstream max-size-buffers=2 max-size-bytes=0 max-size-time=0";

/// The queue between the sound encoder and the muxer. The muxer takes sound only as far as the
/// video has got, and a full queue here would stall the sound source, which then throws away
/// what it hears (heard as garbled, skipping sound). Encoded sound is small, so it gets room for
/// a long wait.
const AUDIO_QUEUE: &str = "queue max-size-buffers=0 max-size-bytes=0 max-size-time=30000000000";

/// The pipeline. Sound tracks are mixed into one, since one track is what players and sharing
/// sites expect. The sound is floating point until it has been through the limiter, so a loud mix
/// is turned down rather than clipped.
pub(crate) fn describe(video: &VideoIn, audio: &[AudioIn], format: Format, fps: u32) -> String {
    let mut parts = vec![format!(
        "{} ! videocrop name=crop ! videorate ! videoconvert n-threads=0 ! \
         video/x-raw,format=I420,framerate={fps}/1 ! {VIDEO_QUEUE} ! {} ! queue ! mux.",
        video.source(),
        format.video_encoder()
    )];
    let limit = format!(
        "audio/x-raw,format=F32LE,rate={RATE},channels={CHANNELS},layout=interleaved ! \
         identity name={LIMITER} ! audioconvert ! {} ! {AUDIO_QUEUE} ! mux.",
        format.audio_encoder()
    );
    match audio {
        [] => {}
        [one] => parts.push(format!(
            "{} ! audioconvert ! audioresample ! {limit}",
            one.source()
        )),
        many => {
            for a in many {
                parts.push(format!(
                    "{} ! audioconvert ! audioresample ! queue ! mix.",
                    a.source()
                ));
            }
            parts.push(format!("audiomixer name=mix ! {limit}"));
        }
    }
    parts.push(format!("{} ! filesink name=sink", format.muxer()));
    parts.join(" ")
}

/// The elements a pipeline needs that aren't installed.
pub(crate) fn missing(video: &VideoIn, audio: &[AudioIn], format: Format) -> Vec<Missing> {
    let good = ("gstreamer1-plugins-good", "gstreamer1.0-plugins-good");
    let base = ("gstreamer1-plugins-base", "gstreamer1.0-plugins-base");
    let mut needed = vec![
        ("videocrop", good),
        ("videorate", base),
        ("videoconvert", base),
        ("filesink", ("gstreamer1", "gstreamer1.0-tools")),
    ];
    needed.push(match video.element() {
        "pipewiresrc" => (
            "pipewiresrc",
            ("pipewire-gstreamer", "gstreamer1.0-pipewire"),
        ),
        other => (other, good),
    });
    if !audio.is_empty() {
        needed.extend([
            ("audioconvert", base),
            ("audioresample", base),
            ("identity", ("gstreamer1", "gstreamer1.0-tools")),
        ]);
        needed.extend(audio.iter().map(|a| (a.element(), good)));
    }
    if audio.len() > 1 {
        needed.push(("audiomixer", base));
    }
    needed.extend(format.helpers());
    let mut out: Vec<Missing> = needed
        .into_iter()
        .filter(|(e, _)| !has(e))
        .map(|(element, (fedora, debian))| Missing {
            element,
            fedora,
            debian,
        })
        .collect();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_track_skips_the_mixer() {
        crate::init().unwrap();
        let format = Format::detect().unwrap();
        let d = describe(
            &VideoIn::X11 {
                x: 10,
                y: 20,
                width: 100,
                height: 50,
            },
            &[MICROPHONE],
            format,
            30,
        );
        assert!(d.starts_with("ximagesrc name=video startx=10 starty=20 endx=109 endy=69 "));
        assert!(d.contains("pulsesrc ! audioconvert"));
        assert!(!d.contains("audiomixer"));
        assert!(d.contains(
            "format=F32LE,rate=48000,channels=2,layout=interleaved ! identity name=limiter"
        ));
    }

    #[test]
    fn two_tracks_are_mixed() {
        crate::init().unwrap();
        let format = Format::detect().unwrap();
        let d = describe(
            &VideoIn::PipeWire { fd: 3, node: 42 },
            &[DESKTOP, MICROPHONE],
            format,
            30,
        );
        assert!(d.starts_with("pipewiresrc name=video fd=3 path=42 "));
        assert!(d.contains(
            "pulsesrc device=@DEFAULT_MONITOR@ ! audioconvert ! audioresample ! queue ! mix."
        ));
        assert!(d.contains("audiomixer name=mix ! audio/x-raw,format=F32LE"));
        assert!(d.contains("identity name=limiter"));
    }

    #[test]
    fn a_slow_encoder_drops_frames_instead_of_starving_the_sound() {
        crate::init().unwrap();
        let format = Format::detect().unwrap();
        let d = describe(
            &VideoIn::PipeWire { fd: 3, node: 42 },
            &[DESKTOP],
            format,
            30,
        );
        assert!(d.contains(&format!("framerate=30/1 ! {VIDEO_QUEUE} ! ")));
        assert!(d.contains(&format!("! {AUDIO_QUEUE} ! mux.")));
    }
}

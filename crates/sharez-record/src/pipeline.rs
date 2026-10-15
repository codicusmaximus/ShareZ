//! The GStreamer pipeline, as a `gst-launch` style description.
//!
//! The video branch always has a `videocrop` named `crop`, so the crop can be set once the real
//! buffer size is known: on Wayland the stream's buffers can be larger than its logical size
//! (fractional scaling), and that is only known when the caps arrive.

use crate::{Format, Missing, encoders::has};

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

/// The output's monitor in PulseAudio naming, which PipeWire understands too.
pub(crate) const DESKTOP: AudioIn = AudioIn::Pulse(Some("@DEFAULT_MONITOR@"));
pub(crate) const MICROPHONE: AudioIn = AudioIn::Pulse(None);

impl VideoIn {
    fn source(&self) -> String {
        match self {
            // keepalive-time resends the last frame while nothing changes on screen, which
            // compositors don't send on their own; videorate then holds a steady frame rate.
            Self::PipeWire { fd, node } => {
                format!("pipewiresrc fd={fd} path={node} do-timestamp=true keepalive-time=1000")
            }
            Self::X11 {
                x,
                y,
                width,
                height,
            } => format!(
                "ximagesrc startx={x} starty={y} endx={} endy={} use-damage=false \
                 show-pointer=true",
                x + width - 1,
                y + height - 1
            ),
            #[cfg(test)]
            Self::Test { width, height } => format!(
                "videotestsrc is-live=true pattern=ball ! video/x-raw,width={width},height={height}"
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

/// The pipeline. Sound tracks are mixed into one, since one track is what players and sharing
/// sites expect.
pub(crate) fn describe(video: &VideoIn, audio: &[AudioIn], format: Format, fps: u32) -> String {
    let mut parts = vec![format!(
        "{} ! videocrop name=crop ! videorate ! videoconvert n-threads=0 ! \
         video/x-raw,format=I420,framerate={fps}/1 ! queue ! {} ! queue ! mux.",
        video.source(),
        format.video_encoder()
    )];
    let encode = format!(
        "audioconvert ! audioresample ! {} ! queue ! mux.",
        format.audio_encoder()
    );
    match audio {
        [] => {}
        [one] => parts.push(format!("{} ! {encode}", one.source())),
        many => {
            for a in many {
                parts.push(format!(
                    "{} ! audioconvert ! audioresample ! queue ! mix.",
                    a.source()
                ));
            }
            parts.push(format!(
                "audiomixer name=mix ! audio/x-raw,rate=48000,channels=2 ! {encode}"
            ));
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
        needed.extend([("audioconvert", base), ("audioresample", base)]);
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
        assert!(d.starts_with("ximagesrc startx=10 starty=20 endx=109 endy=69 "));
        assert!(d.contains("pulsesrc ! audioconvert"));
        assert!(!d.contains("audiomixer"));
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
        assert!(d.starts_with("pipewiresrc fd=3 path=42 "));
        assert!(d.contains(
            "pulsesrc device=@DEFAULT_MONITOR@ ! audioconvert ! audioresample ! queue ! mix."
        ));
        assert!(d.contains("audiomixer name=mix"));
    }
}

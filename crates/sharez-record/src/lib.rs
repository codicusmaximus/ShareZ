//! Region recording: the screen through the screencast portal (Wayland) or the root window (X11),
//! cropped to a region and encoded with GStreamer, with desktop audio and the microphone mixed into
//! one track.
//!
//! Recording follows what the feasibility spike proved on GNOME and KDE: portal `ScreenCast` →
//! PipeWire → crop → encode. The share dialog appears on the first recording only; its restore
//! token is saved so later recordings start straight away.

mod encoders;
mod pipeline;
mod recorder;
mod screencast;

use std::path::PathBuf;

pub use encoders::Format;
pub use recorder::{Recorder, Stop};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("GStreamer: {0}")]
    Gst(String),
    #[error(
        "Missing GStreamer plugins: {}. Install them and try again.",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Missing(Vec<Missing>),
    #[error("Screen sharing: {0}")]
    Portal(#[from] ashpd::Error),
    #[error("The screen share was cancelled or ended.")]
    NoStream,
    #[error("The region isn't on the screen that was shared.")]
    RegionNotShared,
    #[error("The recording did not finish in time, so the file may be unplayable.")]
    StopTimeout,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl From<gstreamer::glib::Error> for Error {
    fn from(e: gstreamer::glib::Error) -> Self {
        Error::Gst(e.to_string())
    }
}

impl From<gstreamer::glib::BoolError> for Error {
    fn from(e: gstreamer::glib::BoolError) -> Self {
        Error::Gst(e.to_string())
    }
}

impl From<gstreamer::StateChangeError> for Error {
    fn from(e: gstreamer::StateChangeError) -> Self {
        Error::Gst(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A GStreamer element that isn't installed, with the packages that usually provide it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub element: &'static str,
    pub fedora: &'static str,
    pub debian: &'static str,
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (Fedora: {}, Debian/Ubuntu: {})",
            self.element, self.fedora, self.debian
        )
    }
}

/// What to record besides the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Audio {
    /// What you hear: the default output's monitor.
    pub desktop: bool,
    /// The default input.
    pub microphone: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub audio: Audio,
    pub fps: u32,
    /// Where to write the file. Its extension should match [`Recorder::format`]; use
    /// [`Format::detect`] to pick it before starting.
    pub path: PathBuf,
}

/// Makes sure GStreamer is initialised. Cheap after the first call.
pub fn init() -> Result<()> {
    gstreamer::init()?;
    Ok(())
}

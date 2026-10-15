//! Running a recording: start, wait for the user to stop, finish the file.

use std::{
    os::fd::AsRawFd,
    path::PathBuf,
    pin::Pin,
    time::{Duration, Instant},
};

use futures_util::{
    StreamExt,
    future::{Either, select},
};
use gstreamer::{self as gst, prelude::*};
use sharez_core::{DisplayServer, Mapping, PixelRect, Rect, Session};

use crate::{
    Audio, Error, Format, Options, Result,
    limiter::Limiter,
    pipeline::{self, AudioIn, VideoIn},
    screencast,
};

/// How long finishing the file may take after Stop. Muxers write their index at the end, and
/// `faststart` MP4 also rewrites the file once.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the video source gets to deliver EOS on its own before its branch is ended for it.
const VIDEO_EOS_GRACE: Duration = Duration::from_secs(2);

/// What to do with the recording when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// Finish and keep the file.
    Save,
    /// Throw it away.
    Discard,
}

/// How the crop is decided.
#[derive(Clone, Copy)]
enum Crop {
    /// The source already covers exactly the region, in pixels.
    Exact { width: u32, height: u32 },
    /// The source shows `area` (logical); crop it to `region` once the buffer size is known.
    Logical { area: Rect, region: Rect },
}

/// A recording in progress.
#[derive(Debug)]
pub struct Recorder {
    pipeline: gst::Pipeline,
    path: PathBuf,
    /// Kept open for as long as the pipeline reads from it.
    cast: Option<screencast::Cast>,
}

fn audio_inputs(audio: Audio) -> Vec<AudioIn> {
    let mut v = Vec::new();
    if audio.desktop {
        v.push(pipeline::DESKTOP);
    }
    if audio.microphone {
        v.push(pipeline::MICROPHONE);
    }
    v
}

fn check(video: &VideoIn, audio: &[AudioIn], format: Format) -> Result<()> {
    let missing = pipeline::missing(video, audio, format);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Error::Missing(missing))
    }
}

impl Recorder {
    /// Starts recording `region` of the screen. Both rectangles are logical; `screen` is the
    /// whole desktop, used when the portal doesn't say which part of it was shared.
    ///
    /// On Wayland the desktop may first ask which screen to share.
    pub async fn start(
        session: &Session,
        region: Rect,
        screen: Rect,
        options: &Options,
        format: Format,
    ) -> Result<Self> {
        crate::init()?;
        let audio = audio_inputs(options.audio);
        match session.display {
            DisplayServer::Wayland => {
                // Before the share dialog, so a missing plugin doesn't waste the user's click.
                check(&VideoIn::PipeWire { fd: -1, node: 0 }, &audio, format)?;
                let cast = screencast::open().await?;
                let area = cast.area.unwrap_or(screen);
                if region.intersection(&area).is_none() {
                    cast.close().await;
                    return Err(Error::RegionNotShared);
                }
                let video = VideoIn::PipeWire {
                    fd: cast.fd.as_raw_fd(),
                    node: cast.node,
                };
                let crop = Crop::Logical { area, region };
                match Self::launch(&video, &audio, options, format, crop) {
                    Ok(pipeline) => Ok(Self {
                        pipeline,
                        path: options.path.clone(),
                        cast: Some(cast),
                    }),
                    Err(e) => {
                        cast.close().await;
                        Err(e)
                    }
                }
            }
            DisplayServer::X11 => {
                // X11 has no scaling: logical coordinates are pixels.
                let px = Mapping {
                    logical: screen,
                    buffer_width: screen.width as u32,
                    buffer_height: screen.height as u32,
                }
                .to_pixels(&region)
                .and_then(PixelRect::even)
                .ok_or(Error::RegionNotShared)?;
                let video = VideoIn::X11 {
                    x: px.x,
                    y: px.y,
                    width: px.width,
                    height: px.height,
                };
                check(&video, &audio, format)?;
                let crop = Crop::Exact {
                    width: px.width,
                    height: px.height,
                };
                Ok(Self {
                    pipeline: Self::launch(&video, &audio, options, format, crop)?,
                    path: options.path.clone(),
                    cast: None,
                })
            }
        }
    }

    fn launch(
        video: &VideoIn,
        audio: &[AudioIn],
        options: &Options,
        format: Format,
        crop: Crop,
    ) -> Result<gst::Pipeline> {
        let description = pipeline::describe(video, audio, format, options.fps);
        let pipeline = gst::parse::launch(&description)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| Error::Gst("not a pipeline".into()))?;
        let element = |name: &str| {
            pipeline
                .by_name(name)
                .ok_or_else(|| Error::Gst(format!("no {name} in the pipeline")))
        };
        element("sink")?.set_property("location", options.path.to_string_lossy().as_ref());
        if let Some(pad) = pipeline
            .by_name(pipeline::LIMITER)
            .and_then(|e| e.static_pad("src"))
        {
            limit(&pad);
        }
        let encoder = element("venc")?;
        let fps = options.fps;
        match crop {
            Crop::Exact { width, height } => format.set_bitrate(&encoder, width, height, fps),
            Crop::Logical { area, region } => {
                let videocrop = element("crop")?;
                let pad = videocrop
                    .static_pad("sink")
                    .ok_or_else(|| Error::Gst("videocrop has no sink pad".into()))?;
                let (videocrop, encoder) = (videocrop.downgrade(), encoder.downgrade());
                pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
                    if let Some(gst::PadProbeData::Event(event)) = &info.data
                        && let gst::EventView::Caps(caps) = event.view()
                        && let Some((width, height)) = buffer_size(caps.caps())
                        && let (Some(videocrop), Some(encoder)) =
                            (videocrop.upgrade(), encoder.upgrade())
                    {
                        let mapping = Mapping {
                            logical: area,
                            buffer_width: width,
                            buffer_height: height,
                        };
                        if let Some(px) = mapping.to_pixels(&region).and_then(PixelRect::even) {
                            let [left, top, right, bottom] = crop_edges(px, width, height);
                            videocrop.set_property("left", left);
                            videocrop.set_property("top", top);
                            videocrop.set_property("right", right);
                            videocrop.set_property("bottom", bottom);
                            format.set_bitrate(&encoder, px.width, px.height, fps);
                        }
                    }
                    gst::PadProbeReturn::Ok
                });
            }
        }
        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            let _ = pipeline.set_state(gst::State::Null);
            return Err(e.into());
        }
        Ok(pipeline)
    }

    /// Records until `stop` says otherwise (or closes, which saves), then finishes the file.
    /// Returns the file's path, or `None` if the recording was discarded.
    pub async fn run(self, stop: async_channel::Receiver<Stop>) -> Result<Option<PathBuf>> {
        let result = self.wait(&stop).await;
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(cast) = self.cast {
            cast.close().await;
        }
        if matches!(result, Ok(None)) {
            let _ = std::fs::remove_file(&self.path);
        }
        result
    }
}

impl Recorder {
    async fn wait(&self, stop: &async_channel::Receiver<Stop>) -> Result<Option<PathBuf>> {
        let bus = self
            .pipeline
            .bus()
            .ok_or_else(|| Error::Gst("the pipeline has no bus".into()))?;
        let mut messages = bus.stream_filtered(&[gst::MessageType::Eos, gst::MessageType::Error]);
        let ended = |msg: &gst::Message| match error_of(msg) {
            Some(e) => Err(e),
            None => Ok(Some(self.path.clone())),
        };
        // Stopping from the desktop (GNOME's "Stop sharing" in the top bar) closes the portal
        // session, and the video source then fails. Both mean "stop and keep the recording".
        let share_ended: Pin<Box<dyn Future<Output = ()>>> = match &self.cast {
            Some(cast) => Box::pin(cast.closed()),
            None => Box::pin(std::future::pending()),
        };
        let user_or_share = async {
            match select(Box::pin(stop.recv()), share_ended).await {
                Either::Left((how, _)) => how.unwrap_or(Stop::Save),
                Either::Right(((), _)) => Stop::Save,
            }
        };
        let how = match select(messages.next(), Box::pin(user_or_share)).await {
            Either::Left((Some(msg), _)) if is_video_error(&msg) => {
                self.end_video();
                Stop::Save
            }
            Either::Left((Some(msg), _)) => return ended(&msg),
            Either::Left((None, _)) => return Err(Error::Gst("bus closed".into())),
            Either::Right((how, _)) => how,
        };
        if how == Stop::Discard {
            return Ok(None);
        }
        self.pipeline.send_event(gst::event::Eos::new());
        let started = Instant::now();
        let mut ended_video = false;
        loop {
            // A source whose stream is gone can't deliver EOS itself; after a moment, end its
            // branch directly so the muxer still finishes the file.
            let wait = if ended_video {
                STOP_TIMEOUT.saturating_sub(started.elapsed())
            } else {
                VIDEO_EOS_GRACE
            };
            match select(messages.next(), async_io::Timer::after(wait)).await {
                Either::Left((Some(msg), _)) if is_video_error(&msg) => {
                    self.end_video();
                    ended_video = true;
                }
                Either::Left((Some(msg), _)) => return ended(&msg),
                Either::Left((None, _)) => return Err(Error::Gst("bus closed".into())),
                Either::Right(_) if !ended_video => {
                    self.end_video();
                    ended_video = true;
                }
                Either::Right(_) => return Err(Error::StopTimeout),
            }
        }
    }

    /// Ends the video branch below its source, for when the source can't (its stream is gone).
    fn end_video(&self) {
        if let Some(pad) = self
            .pipeline
            .by_name("crop")
            .and_then(|crop| crop.static_pad("sink"))
        {
            pad.send_event(gst::event::Eos::new());
        }
    }
}

/// An error from the screen source: the share ended or the stream broke. What was recorded so
/// far is still good.
fn is_video_error(msg: &gst::Message) -> bool {
    matches!(msg.view(), gst::MessageView::Error(_))
        && msg.src().is_some_and(|s| s.name() == pipeline::VIDEO)
}

/// Runs the limiter on every buffer leaving `pad`.
fn limit(pad: &gst::Pad) {
    let limiter = std::sync::Mutex::new(Limiter::default());
    pad.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
        if let Some(gst::PadProbeData::Buffer(buffer)) = &mut info.data
            && let Ok(mut map) = buffer.make_mut().map_writable()
            && let Ok(mut limiter) = limiter.lock()
        {
            limiter.process_bytes(map.as_mut_slice());
        }
        gst::PadProbeReturn::Ok
    });
}

fn buffer_size(caps: &gst::CapsRef) -> Option<(u32, u32)> {
    let s = caps.structure(0)?;
    let w = s.get::<i32>("width").ok()?;
    let h = s.get::<i32>("height").ok()?;
    Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?))
}

/// `videocrop`'s left, top, right and bottom for keeping `px` of a `width`×`height` buffer.
fn crop_edges(px: PixelRect, width: u32, height: u32) -> [i32; 4] {
    [
        px.x as i32,
        px.y as i32,
        (width - px.x - px.width) as i32,
        (height - px.y - px.height) as i32,
    ]
}

fn error_of(msg: &gst::Message) -> Option<Error> {
    let gst::MessageView::Error(err) = msg.view() else {
        return None;
    };
    let source = msg.src().map(|s| s.name().to_string()).unwrap_or_default();
    Some(Error::Gst(match err.debug() {
        Some(debug) => format!("{source}: {} ({debug})", err.error()),
        None => format!("{source}: {}", err.error()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kde_crop_uses_buffer_pixels() {
        // The KDE spike: a 1671×1114 logical monitor streamed as 2256×1504 buffers. The centre
        // quarter must come out 1128×752 at 564,376, not the logical numbers.
        let mapping = Mapping {
            logical: Rect::new(0.0, 0.0, 1671.0, 1114.0),
            buffer_width: 2256,
            buffer_height: 1504,
        };
        let region = Rect::new(417.75, 278.5, 835.5, 557.0);
        let px = mapping
            .to_pixels(&region)
            .and_then(PixelRect::even)
            .unwrap();
        assert_eq!((px.width, px.height), (1128, 752));
        assert_eq!(crop_edges(px, 2256, 1504), [564, 376, 564, 376]);
    }

    /// Records a couple of seconds from test sources through each installed encoder and muxer.
    #[test]
    fn records_and_finishes_a_file() {
        crate::init().unwrap();
        let mut formats = crate::encoders::all_installed();
        assert!(!formats.is_empty(), "no video encoder installed");
        // Stock Fedora: H.264 without an AAC encoder, so Opus in MP4.
        if crate::encoders::has("openh264enc") {
            formats.push(Format::pick(|e| e == "openh264enc").unwrap());
        }
        for format in formats {
            record_with(format);
        }
    }

    /// A recorder on test sources: 320×240 video cropped to 102×80, two tones.
    fn test_recorder(format: Format, name: &str) -> Recorder {
        let path = std::env::temp_dir().join(format!(
            "sharez-test-{}-{name}-{}-{}.{}",
            std::process::id(),
            format.video_encoder().split(' ').next().unwrap(),
            format.audio_encoder().split(' ').next().unwrap(),
            format.extension()
        ));
        let options = Options {
            audio: Audio {
                desktop: true,
                microphone: true,
            },
            fps: 30,
            path: path.clone(),
        };
        let video = VideoIn::Test {
            width: 320,
            height: 240,
        };
        let audio = [AudioIn::Test(440), AudioIn::Test(660)];
        let crop = Crop::Logical {
            area: Rect::new(0.0, 0.0, 160.0, 120.0),
            region: Rect::new(10.0, 10.0, 51.0, 40.0),
        };
        Recorder {
            pipeline: Recorder::launch(&video, &audio, &options, format, crop).unwrap(),
            path,
            cast: None,
        }
    }

    /// Runs `recorder`, doing `meanwhile` after a second and a half and then saving.
    fn run_for(recorder: Recorder, meanwhile: impl FnOnce()) -> Result<Option<PathBuf>> {
        let (tx, rx) = async_channel::bounded(1);
        async_io::block_on(async {
            let stopper = async {
                async_io::Timer::after(Duration::from_millis(1500)).await;
                meanwhile();
                async_io::Timer::after(Duration::from_millis(500)).await;
                let _ = tx.send(Stop::Save).await;
            };
            futures_util::join!(recorder.run(rx), stopper).0
        })
    }

    fn record_with(format: Format) {
        let recorder = test_recorder(format, "save");
        let path = recorder.path.clone();
        let videocrop = recorder.pipeline.by_name("crop").unwrap();
        let saved = run_for(recorder, || {}).unwrap();
        assert_eq!(saved.as_deref(), Some(path.as_path()));
        // 2× scale: 10,10 51×40 logical is 20,20 102×80 pixels.
        assert_eq!(videocrop.property::<i32>("left"), 20);
        assert_eq!(videocrop.property::<i32>("right"), 320 - 20 - 102);
        assert!(std::fs::metadata(&path).unwrap().len() > 1000);
        let _ = std::fs::remove_file(path);
    }

    /// Stopping the share from the desktop kills the video source with an error. The recording
    /// must still be finished and kept.
    #[test]
    fn share_ending_keeps_the_recording() {
        crate::init().unwrap();
        let format = Format::detect().unwrap();
        let recorder = test_recorder(format, "share-ended");
        let path = recorder.path.clone();
        let video = recorder.pipeline.by_name(pipeline::VIDEO).unwrap();
        let saved = run_for(recorder, move || {
            // What pipewiresrc does when the stream goes away: stop producing, post an error.
            video.set_locked_state(true);
            let _ = video.set_state(gst::State::Null);
            video
                .post_message(
                    gst::message::Error::builder(gst::ResourceError::Failed, "stream disconnected")
                        .src(&video)
                        .build(),
                )
                .unwrap();
        })
        .unwrap();
        assert_eq!(saved.as_deref(), Some(path.as_path()));
        assert!(std::fs::metadata(&path).unwrap().len() > 1000);
        let _ = std::fs::remove_file(path);
    }

    /// The loudest sample passing `pad`, kept up to date while the pipeline runs.
    fn loudest(pad: &gst::Pad) -> std::sync::Arc<std::sync::Mutex<f32>> {
        let loudest = std::sync::Arc::new(std::sync::Mutex::new(0.0_f32));
        let seen = loudest.clone();
        pad.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
            if let Some(gst::PadProbeData::Buffer(buffer)) = &info.data
                && let Ok(map) = buffer.map_readable()
            {
                let mut seen = seen.lock().unwrap();
                for b in map.as_slice().as_chunks::<4>().0 {
                    let s = f32::from_le_bytes(*b);
                    *seen = seen.max(s.abs());
                }
            }
            gst::PadProbeReturn::Ok
        });
        loudest
    }

    /// Two loud tones mixed go well over full scale; what gets encoded must not.
    #[test]
    fn mixed_audio_does_not_clip() {
        crate::init().unwrap();
        let format = Format::detect().unwrap();
        let recorder = test_recorder(format, "loud");
        let path = recorder.path.clone();
        let element = recorder.pipeline.by_name(pipeline::LIMITER).unwrap();
        let mixed = loudest(&element.static_pad("sink").unwrap());
        let limited = loudest(&element.static_pad("src").unwrap());
        run_for(recorder, || {}).unwrap();
        let (mixed, limited) = (*mixed.lock().unwrap(), *limited.lock().unwrap());
        assert!(mixed > 1.0, "the test tones should clip unlimited: {mixed}");
        assert!(limited <= crate::limiter::CEILING, "limited to {limited}");
        let _ = std::fs::remove_file(path);
    }
}

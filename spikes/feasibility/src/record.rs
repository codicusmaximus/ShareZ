//! Region recording with desktop audio and microphone on separate tracks.
//!
//! Wayland: portal screencast -> PipeWire fd handed to `gst-launch-1.0` -> crop -> encode.
//! X11: `ximagesrc` with the region bounds. Audio comes from the PulseAudio-compatible API that
//! PipeWire provides: the default sink's monitor (what you hear) and the default source (mic).
//! The spike shells out to gst-launch so it builds without GStreamer dev headers; the real app
//! will use gstreamer-rs or PipeWire directly.

use std::{
    os::fd::{AsRawFd, RawFd},
    os::unix::process::CommandExt,
    path::Path,
    process::Command,
    time::Duration,
};

use anyhow::{Context, bail};
use serde_json::{Value, json};

use crate::{env, portal};

const CHILD_FD: RawFd = 3;
const TOKEN_FILE: &str = "screencast-restore-token";

fn pick_encoder() -> anyhow::Result<&'static str> {
    for (elements, launch) in [
        (
            &["x264enc", "h264parse"][..],
            "x264enc tune=zerolatency speed-preset=veryfast ! h264parse",
        ),
        (&["openh264enc", "h264parse"][..], "openh264enc ! h264parse"),
        (&["vp8enc"][..], "vp8enc deadline=1 cpu-used=8"),
    ] {
        if elements.iter().all(|e| env::has_gst_element(e)) {
            return Ok(launch);
        }
    }
    bail!("no video encoder found (tried x264enc, openh264enc, vp8enc)")
}

/// The centre half of the source, so cropping is visibly exercised.
fn centre_region(w: i32, h: i32) -> (i32, i32, i32, i32) {
    let (rw, rh) = ((w / 2) & !1, (h / 2) & !1);
    ((w - rw) / 2, (h - rh) / 2, rw, rh)
}

fn audio_branches(have_pulse: bool) -> String {
    if !have_pulse {
        return String::new();
    }
    [
        "pulsesrc device=@DEFAULT_MONITOR@ ! audio/x-raw,channels=2 ! audioconvert ! audioresample ! opusenc ! queue ! mux.",
        "pulsesrc ! audioconvert ! audioresample ! opusenc ! queue ! mux.",
    ]
    .join(" ")
}

const EOS_GRACE: Duration = Duration::from_secs(6);

enum Stopped {
    Exited(std::process::ExitStatus),
    Killed(std::process::ExitStatus),
}

/// SIGINT makes `gst-launch -e` send EOS so the file is finalised properly. If EOS never
/// completes (a source that stopped producing data, for example), kill it rather than hang.
fn stop(child: &mut std::process::Child) -> anyhow::Result<Stopped> {
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    let deadline = std::time::Instant::now() + EOS_GRACE;
    while std::time::Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            return Ok(Stopped::Exited(status));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    child.kill()?;
    Ok(Stopped::Killed(child.wait()?))
}

fn run_gst(
    pipeline: &str,
    pass_fd: Option<RawFd>,
    seconds: u64,
    out_file: &Path,
) -> anyhow::Result<Value> {
    let mut cmd = Command::new("gst-launch-1.0");
    cmd.arg("-e").arg("-v").args(pipeline.split_whitespace());
    // Log to a file, not a pipe: `-v` prints a lot, and a full pipe blocks gst-launch forever.
    let log_path = out_file.with_extension("gst.log");
    let log_file = std::fs::File::create(&log_path)?;
    cmd.stdout(log_file.try_clone()?).stderr(log_file);
    if let Some(fd) = pass_fd {
        // SAFETY: dup2 is async-signal-safe; it places the PipeWire fd at a fixed number in the child.
        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(fd, CHILD_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn().context("spawn gst-launch-1.0")?;
    eprintln!("   recording {seconds}s: play some audio and say something into the mic");
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Stopped::Exited(status); // exited early: almost certainly a pipeline error
        }
        if std::time::Instant::now() >= deadline {
            break stop(&mut child)?;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let (status, eos_finished) = match status {
        Stopped::Exited(s) => (s, true),
        Stopped::Killed(s) => (s, false),
    };
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();

    let caps: Vec<&str> = log
        .lines()
        .filter(|l| {
            l.contains("src: caps = video/x-raw")
                || l.contains("pulsesrc") && l.contains("caps = audio")
        })
        .take(6)
        .collect();
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=index,codec_type,codec_name,width,height,channels:format=duration",
            "-of",
            "json",
        ])
        .arg(out_file)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());

    let errors: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("ERROR") || l.contains("WARNING"))
        .take(10)
        .collect();
    if !eos_finished {
        bail!(
            "gst-launch did not finish within {EOS_GRACE:?} of stopping, so it was killed \
             (the file may be unplayable). Last log lines: {}\npipeline: {pipeline}",
            log.lines().rev().take(8).collect::<Vec<_>>().join(" | ")
        );
    }
    if !status.success() || !out_file.exists() {
        bail!(
            "gst-launch failed ({status}): {}\npipeline: {pipeline}",
            errors.join(" | ")
        );
    }

    Ok(json!({
        "pipeline": pipeline,
        "gst_exit": status.code(),
        "negotiated_caps": caps,
        "warnings": errors,
        "file": out_file,
        "log_file": log_path,
        "file_bytes": std::fs::metadata(out_file).map(|m| m.len()).ok(),
        "ffprobe": probe,
    }))
}

pub async fn probe(out: &Path, seconds: u64) -> anyhow::Result<Value> {
    if env::command_line("gst-launch-1.0", &["--version"]).is_none() {
        bail!("gst-launch-1.0 not installed (install gstreamer1.0-tools / gstreamer-tools)");
    }
    let encoder = pick_encoder()?;
    let have_pulse = env::has_gst_element("pulsesrc");
    let audio = audio_branches(have_pulse);
    let out_file = std::fs::canonicalize(out)?.join("region-recording.mkv");
    let _ = std::fs::remove_file(&out_file);
    let sink = format!(
        "matroskamux name=mux ! filesink location={}",
        out_file.display()
    );

    if env::session_type() == "wayland" {
        let token_path = out.join(TOKEN_FILE);
        let token = std::fs::read_to_string(&token_path).ok();
        let cast = portal::screencast(token.as_deref()).await?;
        if let Some(t) = cast.info["new_restore_token"].as_str() {
            std::fs::write(&token_path, t)?;
        }
        let (w, h) = cast.stream.size().context("portal stream has no size")?;
        let (x, y, rw, rh) = centre_region(w, h);
        // Crop in logical coordinates. If the negotiated caps show a larger buffer than `size`
        // (fractional scaling), the crop is off: that is one of the things this run checks.
        let pipeline = format!(
            "pipewiresrc fd={CHILD_FD} path={node} do-timestamp=true keepalive-time=1000 ! videoconvert ! \
             videocrop left={x} top={y} right={r} bottom={b} ! videoconvert ! videorate ! video/x-raw,framerate=30/1 ! videoconvert ! video/x-raw,format=I420 ! \
             queue ! {encoder} ! queue ! mux. {audio} {sink}",
            node = cast.stream.pipe_wire_node_id(),
            r = w - x - rw,
            b = h - y - rh,
        );
        let fd = cast.fd.as_raw_fd();
        let result =
            tokio::task::block_in_place(|| run_gst(&pipeline, Some(fd), seconds, &out_file))?;
        Ok(json!({
            "backend": "portal+pipewire",
            "screencast": cast.info,
            "logical_region": [x, y, rw, rh],
            "audio_tracks_requested": have_pulse,
            "gst": result,
            "tip": "Run `record` a second time: with the saved restore token the share dialog should not appear.",
        }))
    } else {
        let (sw, sh) = {
            use x11rb::connection::Connection;
            let (conn, n) = x11rb::rust_connection::RustConnection::connect(None)?;
            let s = &conn.setup().roots[n];
            (s.width_in_pixels as i32, s.height_in_pixels as i32)
        };
        let (x, y, rw, rh) = centre_region(sw, sh);
        let pipeline = format!(
            "ximagesrc startx={x} starty={y} endx={ex} endy={ey} use-damage=false show-pointer=true ! \
             video/x-raw,framerate=30/1 ! videoconvert ! video/x-raw,format=I420 ! queue ! {encoder} ! queue ! mux. {audio} {sink}",
            ex = x + rw - 1,
            ey = y + rh - 1,
        );
        let result = run_gst(&pipeline, None, seconds, &out_file)?;
        Ok(
            json!({ "backend": "x11 ximagesrc", "region": [x, y, rw, rh], "audio_tracks_requested": have_pulse, "gst": result }),
        )
    }
}

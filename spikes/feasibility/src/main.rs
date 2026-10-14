//! ShareZ feasibility spike.
//!
//! Throwaway probes that answer "can we do this on this desktop?" for the capture paths ShareZ
//! needs. Every probe records what it found (or why it failed) into a report directory, so a run
//! on a real KDE / GNOME / X11 session can be read back without anyone watching the terminal.

mod atspi;
mod dbus;
mod env;
mod gnome;
mod kwin;
mod portal;
mod record;
mod report;
mod x11;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use report::Report;

#[derive(Parser)]
#[command(about = "ShareZ feasibility probes")]
struct Cli {
    /// Where screenshots, recordings and report.{json,md} are written.
    #[arg(long, default_value = "sharez-spike-report")]
    out: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run every probe that applies to the current session, then write the report.
    All {
        /// Seconds to record for in the recording probe.
        #[arg(long, default_value_t = 8)]
        seconds: u64,
        /// Skip the recording probe (it asks for screen-share permission).
        #[arg(long)]
        no_record: bool,
    },
    /// Session, compositor, portal and GStreamer plugin info.
    Env,
    /// X11 window list (EWMH) and root-window capture.
    X11,
    /// xdg-desktop-portal non-interactive screenshot.
    PortalShot,
    /// KWin window geometry via a temporary KWin script, plus the ScreenShot2 D-Bus API.
    Kwin,
    /// GNOME Shell window geometry: Introspect API and the spike helper extension.
    Gnome,
    /// Walk the accessibility tree and report element rectangles.
    Atspi {
        /// Turn on the session accessibility bus if it is off (apps may need a restart).
        #[arg(long)]
        enable: bool,
    },
    /// Record a screen region with desktop audio and microphone as separate tracks.
    Record {
        #[arg(long, default_value_t = 8)]
        seconds: u64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.out)?;
    let mut report = Report::new(cli.out.clone());

    report.run("env", env::probe()).await;
    match cli.cmd {
        Cmd::Env => {}
        Cmd::X11 => report.run("x11", x11::probe(&cli.out)).await,
        Cmd::PortalShot => {
            report
                .run("portal_screenshot", portal::screenshot(&cli.out))
                .await
        }
        Cmd::Kwin => report.run("kwin", kwin::probe(&cli.out)).await,
        Cmd::Gnome => report.run("gnome", gnome::probe()).await,
        Cmd::Atspi { enable } => report.run("atspi", atspi::probe(enable)).await,
        Cmd::Record { seconds } => report.run("record", record::probe(&cli.out, seconds)).await,
        Cmd::All { seconds, no_record } => {
            let session = env::session_type();
            let desktop = env::desktop().to_lowercase();
            if std::env::var_os("DISPLAY").is_some() {
                report.run("x11", x11::probe(&cli.out)).await;
            }
            if session == "wayland" {
                report
                    .run("portal_screenshot", portal::screenshot(&cli.out))
                    .await;
            }
            if desktop.contains("kde") {
                report.run("kwin", kwin::probe(&cli.out)).await;
            }
            if desktop.contains("gnome") {
                report.run("gnome", gnome::probe()).await;
            }
            report.run("atspi", atspi::probe(false)).await;
            if !no_record {
                report.run("record", record::probe(&cli.out, seconds)).await;
            }
        }
    }

    report.write()?;
    Ok(())
}

# Feasibility spike

Throwaway probes for roadmap step 1. They check, on a real desktop session, whether each
capture path ShareZ needs actually works. Each run writes a report directory that can be
compared across sessions. None of this code ships; the findings go into `docs/`.

## What each probe answers

| Probe | Question |
|---|---|
| `env` | Session type, compositor and portal versions, available GStreamer elements |
| `x11` | EWMH window rectangles (incl. CSD shadow insets) and full-screen grab speed. Under Wayland this shows what XWayland alone can see |
| `portal-shot` | Wayland "freeze frame": does a non-interactive portal screenshot work without a dialog, and how long does it take? |
| `kwin` | KDE window rectangles via a temporary KWin script; whether `org.kde.KWin.ScreenShot2` is usable |
| `gnome` | GNOME window rectangles: `org.gnome.Shell.Introspect` and the helper extension below |
| `atspi` | Elements inside windows (buttons, panels, fields) from the accessibility tree, plus a hover-style hit test and its latency |
| `record` | Region recording with desktop audio and mic on separate tracks; on Wayland, whether the restore token skips the share dialog next time |

## Prerequisites

Rust (stable), plus GStreamer command-line tools and plugins for the recording probe:

```sh
# Fedora
sudo dnf install gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugins-ugly-free \
    gstreamer1-plugin-openh264 pipewire-gstreamer gstreamer1-tools ffmpeg-free
# Debian / Ubuntu
sudo apt install gstreamer1.0-tools gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
    gstreamer1.0-plugins-ugly gstreamer1.0-pipewire gstreamer1.0-pulseaudio ffmpeg
# Arch
sudo pacman -S gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-plugin-pipewire ffmpeg
```

## Running

From this directory, in each session you want to test (KDE Wayland, GNOME Wayland, an X11 session):

```sh
cargo run --release -- --out report-$(echo $XDG_CURRENT_DESKTOP-$XDG_SESSION_TYPE | tr 'A-Z:' 'a-z-') all
```

During the recording step, pick a monitor in the share dialog, play some audio and talk into the
mic. Then run `record` a second time with the same `--out` to check that the dialog is skipped
(it is saved as `record_second_run` in the same report):

```sh
cargo run --release -- --out report-... record
```

For more accessibility results, open a couple of apps (Dolphin or Files, a browser, a settings
window) before running. If the report says accessibility was off, run
`cargo run --release -- atspi --enable`, restart those apps, and run `atspi` again.

### GNOME helper extension

GNOME does not tell normal apps where windows are. To test the extension route:

```sh
cp -r gnome-extension/sharez-spike@sharez.github.io ~/.local/share/gnome-shell/extensions/
# Log out and back in (GNOME on Wayland only picks up new extensions at login), then:
gnome-extensions enable sharez-spike@sharez.github.io
```

Remove it afterwards with `gnome-extensions uninstall sharez-spike@sharez.github.io`.

On a Wayland session the `x11` probe only sees XWayland apps, and its screen grab is expected to
fail; that's recorded as a finding, not a problem with your setup.

## Sharing results

Each run's report directory contains `report.json`, `report.md`, the screenshots and the
recording. The `.md` and `.json` are the important parts. Check that the screenshots don't show
anything private before sharing them.

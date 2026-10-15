# ShareZ

A screen capture and recording tool for Linux, in the spirit of [ShareX](https://getsharex.com/).
It targets Wayland (KDE Plasma, GNOME) and X11 as first-class platforms.

> **Status:** early development. Region screenshots and region recordings work: press
> **Ctrl+Print** or **Ctrl+Shift+Print** (or use the buttons, or ShareZ's top-bar icon), then drag a
> region or click a window or an element inside one. Screenshots go to `~/Pictures/ShareZ`,
> recordings to `~/Videos/ShareZ`, and either is always copied to the clipboard.

## Goals

- **Smart region capture**: hover and click a window, or an element inside a window, and ShareZ
  works out the region for you. Freehand regions get a magnified pixel view around the cursor.
- **Region and window recording** with desktop audio and an optional microphone track, so you can
  demo an app while talking over it.
- **Works the same on Wayland and X11.**
- Later: after-capture annotation, upload destinations (including importing ShareX `.sxcu`
  custom uploaders), OCR.

## Roadmap

1. Feasibility spike: prove the hard capture, window-detection and recording paths on KDE Wayland,
   GNOME Wayland and X11.
2. Architecture and repository skeleton.
3. Region screenshot release.
4. Region video recording.

## Running in the background

Once started, ShareZ keeps running when you close its window, ready for its shortcuts. Its icon
sits in the top bar on GNOME (with the [ShareZ Helper extension](#gnome-the-sharez-helper-extension))
and in the system tray on KDE Plasma and most X11 desktops, with Capture Region, Record Region,
Open ShareZ and Quit. Turn on **Start at Login** in the window to have it ready after you log in.
Ctrl+Q in the window, or Quit in the icon's menu, ends it.

| Shortcut | Does |
|---|---|
| Ctrl+Print | Capture a region |
| Ctrl+Shift+Print | Start recording a region, or stop the recording |

On Wayland the desktop owns global shortcuts: the first time ShareZ runs, GNOME or KDE asks you to
confirm these keys (or pick others), and you can change them later in the desktop's keyboard
settings. On X11, click a shortcut in the ShareZ window to change it. `sharez --capture` and
`sharez --record` do the same from a script or your own shortcut.

## Capturing a region

Capture works like ShareX's region capture:

| Do this | To get |
|---|---|
| Drag | A region, with a magnified pixel grid around the cursor |
| Shift while dragging | A square region |
| Ctrl while dragging | Move the region instead of resizing it |
| Click without dragging | The highlighted window, or the button, panel or field inside it |
| Scroll | Zoom the magnifier in or out |
| Space | The whole screen |
| Esc or right-click | Cancel (right-click during a drag only cancels the drag) |

Elements inside windows come from the accessibility tree, which ShareZ turns on for your session
the first time it runs. Apps started before that may need a restart to show their elements.

## Recording a region

Press **Record Region** and pick the region the same way as for a screenshot. ShareZ then records
it at 30 fps until you press **Stop Recording** or the record shortcut again (an open ShareZ
window minimises itself while recording; the tray icon and the notification can stop it too). Stopping the share from the desktop, such as
GNOME's stop button in the top bar, also stops and keeps the recording. **Discard** throws the
recording away. A thin red outline marks the region while it records; it stays out of the video
and lets clicks through.

Choose **Desktop Audio** (what you hear), **Microphone**, or both under *Recording Audio*. With
both, they are mixed into one track, and a limiter turns loud moments down so the mix never clips.
If your voice is distorted before it reaches ShareZ, turn the input volume down in your sound
settings; the spike found laptop mics clipping at their default level.

On Wayland the desktop asks which screen to share the first time you record. ShareZ remembers the
answer, so later recordings start straight away.

Recordings are MP4 (H.264) when an H.264 encoder is installed, otherwise WebM (VP8). ShareZ tells
you which GStreamer plugins are missing if it can't record. On Fedora:

```sh
sudo dnf install pipewire-gstreamer gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugin-openh264
```

### GNOME: the ShareZ Helper extension

GNOME doesn't let normal apps see which window is on top. Without help ShareZ guesses, and can
highlight a window that is behind another one, and has nowhere for a tray icon. The small ShareZ
Helper shell extension in [`gnome-extension/`](gnome-extension/) tells it the real stacking order
and exact window edges, places the outline around a region being recorded, and shows ShareZ's
top-bar icon. `./install.sh` installs it for you, or copy it yourself (again after updating
ShareZ):

```sh
cp -r gnome-extension/sharez-helper@codicusmaximus.github.io ~/.local/share/gnome-shell/extensions/
# Log out and back in, then:
gnome-extensions enable sharez-helper@codicusmaximus.github.io
```

KDE Plasma and X11 need nothing extra.

## Building

You need Rust 1.92 or newer, and GTK 4.14+, libadwaita 1.5+ and GStreamer 1.20+ development
files.

```sh
# Fedora
sudo dnf install gtk4-devel libadwaita-devel gstreamer1-devel
# Debian / Ubuntu
sudo apt install libgtk-4-dev libadwaita-1-dev libgstreamer1.0-dev
# Arch
sudo pacman -S gtk4 libadwaita gstreamer

cargo run                      # the app
cargo run -- --list-windows    # print the windows ShareZ can snap to, as JSON
```

To use ShareZ day to day, install it for your user with `./install.sh`. It builds a release, puts
`sharez` in `~/.local/bin`, adds it to your app launcher, and on GNOME installs the helper
extension. The launcher entry matters on Wayland: GNOME's shortcut dialog and notifications find
apps by it.

To build a Flatpak locally, see [`flatpak/`](flatpak/io.github.codicusmaximus.ShareZ.Devel.yml).
How the code is organised is in [docs/architecture.md](docs/architecture.md).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).

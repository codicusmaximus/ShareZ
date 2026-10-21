# ShareZ

A screen capture and recording tool for Linux, in the spirit of [ShareX](https://getsharex.com/).
It targets Wayland (KDE Plasma, GNOME) and X11 as first-class platforms.

> **Status:** early development. Screenshots and region recordings work: press **Print** for the
> whole screen, **Ctrl+Print** for a region, **Super+Shift+S** for a region and a popup that asks
> what to do with it, or **Ctrl+Shift+Print** to record (or use the buttons, or ShareZ's top-bar
> icon). Screenshots go to `~/Pictures/ShareZ`, recordings to `~/Videos/ShareZ`, and both are
> copied to the clipboard unless a hotkey says otherwise.

## Install

Download the package for your distribution from the
[latest release](https://github.com/codicusmaximus/ShareZ/releases/latest) and install it with your
package manager, which brings in everything ShareZ needs:

```sh
sudo apt install ./sharez_*_amd64.deb             # Ubuntu 24.04+, Debian 13+
sudo dnf install ./sharez-*.x86_64.rpm            # Fedora 40+
sudo pacman -U ./sharez-*-x86_64.pkg.tar.zst      # Arch
```

On GNOME, log out and back in once afterwards so GNOME picks up ShareZ's helper extension. The
release notes ([`packaging/RELEASE_NOTES.md`](packaging/RELEASE_NOTES.md)) cover optional MP4
support and uninstalling.

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

Once started, ShareZ keeps running when you close its window, ready for its hotkeys. Its icon
sits in the top bar on GNOME (with the [ShareZ Helper extension](#gnome-the-sharez-helper-extension))
and in the system tray on KDE Plasma and most X11 desktops, with Capture Region, Capture Screen,
Record Region, Open ShareZ and Quit. Turn on **Start at Login** in the window to have it ready
after you log in. Ctrl+Q in the window, or Quit in the icon's menu, ends it.

## Hotkeys

Each hotkey is a key, something to capture, and what to do with the screenshot. ShareZ starts
with these:

| Hotkey | Captures | Then |
|---|---|---|
| Print | The whole screen | Saves and copies it |
| Ctrl+Print | A region | Saves and copies it |
| Super+Shift+S | A region | Asks what to do |
| Ctrl+Shift+Print | Starts recording a region, or stops the recording | Saves and copies it |

Change, add or delete them under **Hotkeys** in the ShareZ window: click one to edit it, or **+**
to add one. A hotkey can capture:

- **Region**: drag a region, or click a window or anything in it (see below).
- **Whole Screen**: everything on screen, at once.
- **Last Region**: the region you picked last time, at once. The first time, you pick one.
- **Record Region**: start recording a region, or stop the recording.

and then, for screenshots, **Save and Copy**, **Copy Only**, **Save Only**, **Open in Annotator**
or **Ask What to Do**.
Asking shows a popup with the screenshot and these answers, each with a key:

| Key | Does |
|---|---|
| Enter | Save and copy |
| E | Annotate it, then ask again |
| C | Copy to the clipboard |
| S | Save to `~/Pictures/ShareZ` |
| A | Save as…, wherever you choose |
| O | Save and open it in your image viewer |
| Esc | Discard |

On Wayland the desktop owns global shortcuts: when ShareZ first runs, and after you add a hotkey,
GNOME or KDE asks you to confirm the keys (ShareZ's are only suggestions), and you change them
later in the desktop's keyboard settings. The keyboard button next to **+** opens those where the
desktop has them. GNOME and KDE use Print for their own screenshot tools, so the desktop may not
let ShareZ have it until you give their screenshot shortcut another key (on GNOME: Settings →
Keyboard → View and Customize Shortcuts → Screenshots). On X11 ShareZ sets the keys itself: click
**Keys** in a hotkey and press the new ones.

`sharez --capture`, `sharez --capture-screen` and `sharez --record` do the same from a script or
your own shortcut, and `sharez --hotkey ID` does what any hotkey does (the ids are in
`~/.config/sharez/settings.ini`).

## Capturing a region

Capture works like ShareX's region capture:

| Do this | To get |
|---|---|
| Drag | A region, with a magnified pixel grid around the cursor |
| Shift while dragging | A square region |
| Ctrl while dragging | Move the region instead of resizing it |
| Click without dragging | The highlighted window, or the button, panel or field inside it, even where other windows cover it |
| Scroll | Zoom the magnifier in or out |
| Space | The whole screen |
| Esc or right-click | Cancel (right-click during a drag only cancels the drag) |

Elements inside windows come from the accessibility tree, which ShareZ turns on for your session
the first time it runs. Apps started before that may need a restart to show their elements.

## Annotating

The annotator is a port of ShareX's image editor, with its tools, keys and toolbar. Open it from
the popup (**E**), from a hotkey set to **Open in Annotator**, or on any image with
`sharez --annotate FILE`. When you're done, the popup asks what to do with the result.

| Key | Tool |
|---|---|
| M | Select and move |
| R, E | Rectangle, ellipse |
| F, L, A | Freehand, line, arrow (freehand arrow is on the toolbar) |
| O, T, S | Text (outline), text (background), speech balloon |
| I | Step: numbered markers that count up, and renumber when one is deleted |
| B, P, H | Blur, pixelate, highlight |
| C | Crop |
| Numpad 1–9 | Rectangle, ellipse, freehand, line, arrow, text, step, blur, pixelate |

Magnify and smart eraser are on the toolbar. Each tool's colours, sizes, shadow and the like are on
the toolbar next to it, apply to the selected shape too, and are kept for next time.

| Do this | To get |
|---|---|
| Shift while drawing | Lines at 45° steps, squares and circles; corners resize in proportion |
| Ctrl while drawing or resizing | Move the shape instead |
| Hold Ctrl | Draw over shapes instead of picking them up |
| Double-click a text shape | Edit its text |
| Arrows (Shift: 10 px) | Move the selected shape; with Ctrl or Alt, resize it |
| Del, Shift+Del | Delete the selected shape, or all of them |
| Home, End, Page Up, Page Down | Bring to front, send to back, forward, backward |
| Ctrl+Z, Ctrl+Y | Undo, redo |
| Ctrl+D, Ctrl+V | Duplicate at the pointer, paste text as a text shape |
| Ctrl+scroll, Ctrl+plus/minus | Zoom; Ctrl+0 for actual size, Ctrl+Alt+0 to fit |
| Middle-drag | Pan |
| Ctrl+S, Ctrl+Shift+S, Ctrl+C | Save, save as, copy |
| Enter | Done: use the annotated image |
| Space or right-click on nothing | Continue with the image without annotations (crops and rotations stay) |
| Esc | Back to the select tool, then cancel |

## Recording a region

Press **Record Region** and pick the region the same way as for a screenshot. ShareZ then records
it at 30 fps until you press **Stop Recording** or the record shortcut again (an open ShareZ
window minimises itself while recording; the tray icon and the notification can stop it too). Stopping the share from the desktop, such as
GNOME's stop button in the top bar, also stops and keeps the recording. **Discard** throws the
recording away. A thin red outline marks the region while it records; it stays out of the video
and lets clicks through. The finished video is copied, and clicking its notification shows it
selected in your file manager, ready to drag wherever it needs to go.

Turn on **Countdown** to have a large 3, 2, 1 appear in the middle of the region first. Recording
starts the moment the 1 disappears, so the countdown is never in the video. Its **Speed** goes
from 0.25× to 2× (1× is one number a second), and stopping during the countdown cancels the
recording.

Choose **Desktop Audio** (what you hear), **Microphone**, or both under *Recording*. With
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
and exact window edges, captures a clicked window even where others cover it, places the outline
around a region being recorded, and shows ShareZ's top-bar icon. The packages and `./install.sh`
install it, and ShareZ turns it on when it starts (GNOME only sees a newly installed extension
after you log out and back in). To install it by hand (again after updating ShareZ):

```sh
cp -r gnome-extension/sharez-helper@codicusmaximus.github.io ~/.local/share/gnome-shell/extensions/
# Log out and back in, then:
gnome-extensions enable sharez-helper@codicusmaximus.github.io
```

KDE Plasma and X11 need nothing extra. On KDE, a clicked window is captured on its own only when
ShareZ runs from its installed launcher entry (a package or `./install.sh`), because KWin checks the entry
before it allows that; on X11 it needs a compositor. Otherwise ShareZ takes the region as it was
on screen.

## Troubleshooting

**ShareZ asks for permission to take screenshots.** On Wayland the desktop asks once whether
ShareZ may take screenshots, and GNOME only asks while ShareZ's window has focus. So the first
time, ShareZ opens its window with an **Allow…** button: press it and allow ShareZ in the dialog
that follows. If you chose Deny, the desktop remembers that, and ShareZ says so. To allow it
again, run:

```sh
gdbus call --session --dest org.freedesktop.impl.portal.PermissionStore \
  --object-path /org/freedesktop/impl/portal/PermissionStore \
  --method org.freedesktop.impl.portal.PermissionStore.SetPermission \
  screenshot true screenshot io.github.codicusmaximus.ShareZ "['yes']"
```

**Hotkeys say "Unavailable".** The desktop has no global shortcuts portal (GNOME before 48, for
example). The window says why. `sharez --hotkey ID`, `sharez --capture` and `sharez --record`
still work from your desktop's own custom shortcuts.

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

To build the release packages (.deb, .rpm, Arch), install [nfpm](https://nfpm.goreleaser.com)
and run `packaging/build.sh` on Ubuntu 24.04; they land in `dist/`. Pushing a `v*` tag, or running the Packages workflow on main from the Actions tab, builds
them in CI, installs each on a clean Debian, Ubuntu, Fedora and Arch, and publishes the release.

To build a Flatpak locally, see [`flatpak/`](flatpak/io.github.codicusmaximus.ShareZ.Devel.yml).
How the code is organised is in [docs/architecture.md](docs/architecture.md).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).

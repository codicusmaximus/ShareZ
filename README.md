# ShareZ

A screen capture and recording tool for Linux, in the spirit of [ShareX](https://getsharex.com/).
It targets Wayland (KDE Plasma, GNOME) and X11 as first-class platforms.

> **Status:** early development. Region screenshots work: press **Capture screen**, then drag a
> region or click a window or an element inside one. The PNG is saved to `~/Pictures/ShareZ` and
> copied to the clipboard.

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

### GNOME: the ShareZ Helper extension

GNOME doesn't let normal apps see which window is on top. Without help ShareZ guesses, and can
highlight a window that is behind another one. The small ShareZ Helper shell extension in
[`gnome-extension/`](gnome-extension/) tells it the real stacking order and exact window edges:

```sh
cp -r gnome-extension/sharez-helper@codicusmaximus.github.io ~/.local/share/gnome-shell/extensions/
# Log out and back in, then:
gnome-extensions enable sharez-helper@codicusmaximus.github.io
```

KDE Plasma and X11 need nothing extra.

## Building

You need Rust 1.92 or newer, GTK 4.14+ and libadwaita 1.5+ development files.

```sh
# Fedora
sudo dnf install gtk4-devel libadwaita-devel
# Debian / Ubuntu
sudo apt install libgtk-4-dev libadwaita-1-dev
# Arch
sudo pacman -S gtk4 libadwaita

cargo run                      # the app
cargo run -- --list-windows    # print the windows ShareZ can snap to, as JSON
```

To build a Flatpak locally, see [`flatpak/`](flatpak/io.github.codicusmaximus.ShareZ.Devel.yml).
How the code is organised is in [docs/architecture.md](docs/architecture.md).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).

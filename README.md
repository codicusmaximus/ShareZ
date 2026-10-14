# ShareZ

A screen capture and recording tool for Linux, in the spirit of [ShareX](https://getsharex.com/).
It targets Wayland (KDE Plasma, GNOME) and X11 as first-class platforms.

> **Status:** early development. The feasibility spike is done and the application skeleton is
> in place; nothing is useful to end users yet.

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

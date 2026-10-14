# Architecture

ShareZ is a Rust workspace. The split follows what the [feasibility spike](https://github.com/codicusmaximus/ShareZ/pull/1)
showed differs between desktops: **how the screen is captured** depends only on the display
server, while **how windows are found** depends on the desktop.

| Crate | Role |
|---|---|
| `sharez-core` | Geometry (`Rect`, `PixelRect`, `Mapping`), the RGBA `Frame` and session detection. No platform dependencies. |
| `sharez-capture` | Freezing the screen: the screenshot portal on Wayland, the root window on X11. |
| `sharez-windows` | Listing windows: a KWin script on KDE, GNOME Shell's accessibility tree on GNOME, EWMH on X11. |
| `sharez` | The GTK 4 / libadwaita application. |

## Backends

| | GNOME Wayland | KDE Wayland | X11 |
|---|---|---|---|
| Freeze frame | Portal Screenshot | Portal Screenshot | Root `GetImage` |
| Window rectangles | GNOME Shell a11y tree | KWin script | EWMH |
| Elements inside windows | AT-SPI *(milestone 3)* | AT-SPI *(milestone 3)* | AT-SPI *(milestone 3)* |
| Recording | Portal ScreenCast + PipeWire *(milestone 4)* | Same | `ximagesrc` *(milestone 4)* |

Backends are plain enums chosen once from the `Session` (`Backend::for_session`,
`Source::for_session`), not trait objects. There are only a handful of them, and an enum keeps
async methods simple and every case visible in one `match`.

The libraries use runtime-agnostic async (zbus and ashpd on `async-io`). The app drives them from
the GLib main loop with `glib::spawn_future_local`, so there is no Tokio runtime.

## Coordinates

Everything ShareZ reasons about is in **logical pixels**, the unit compositors use for window
and monitor positions. Images and video buffers are in **buffer pixels**. A `Mapping` converts
between them per buffer, and is the only place that conversion happens.

The KDE spike run is why this rule exists: at 135 % scaling the portal described the monitor as
1671×1114, but the video buffers were 2256×1504, and a crop computed in the wrong unit came out the
wrong size in the wrong place. Video crops are also trimmed to even sizes (`PixelRect::even`) for
4:2:0 encoders.

## Toolkit

GTK 4 with libadwaita:

- It is the best supported Rust GUI binding on Linux, and the GStreamer bindings come from the
  same project, which matters for recording previews (`gtk4paintablesink`).
- `GdkMemoryTexture` and `GtkSnapshot` draw the frozen frame and selection with the GPU, which the
  magnifier needs.
- It runs on both Wayland and X11 with the same code.

The trade-off is that ShareZ looks like a GNOME app on KDE. Qt was the alternative, but its Rust
bindings are far less mature.

## Not built yet

- Region selection, window and element snapping, magnifier: milestone 3, on top of the overlay.
- Multi-monitor overlay: the frozen frame covers all monitors, but the overlay is one full-screen
  window today. Milestone 3 places one overlay per monitor using each monitor's `Mapping`.
- Recording: milestone 4, through `gstreamer-rs` in-process.
- GNOME window titles and stacking order: the shell's tree has neither; milestone 3 matches shell
  rectangles against AT-SPI application windows.

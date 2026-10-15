# Architecture

ShareZ is a Rust workspace. The split follows what the [feasibility spike](https://github.com/codicusmaximus/ShareZ/pull/1)
showed differs between desktops: **how the screen is captured** depends only on the display
server, while **how windows are found** depends on the desktop.

| Crate | Role |
|---|---|
| `sharez-core` | Geometry (`Rect`, `PixelRect`, `Mapping`), the RGBA `Frame` and session detection. No platform dependencies. |
| `sharez-capture` | Freezing the screen: the screenshot portal on Wayland, the root window on X11. |
| `sharez-windows` | Listing windows (a KWin script on KDE, the ShareZ Helper extension or GNOME Shell's accessibility tree on GNOME, EWMH on X11) and finding elements inside them through AT-SPI. |
| `sharez-record` | Region recording: the screencast portal or X11 root window into a GStreamer pipeline that crops, encodes and mixes audio. |
| `sharez` | The GTK 4 / libadwaita application, which runs in the background with a tray icon and global shortcuts. |

## Backends

| | GNOME Wayland | KDE Wayland | X11 |
|---|---|---|---|
| Freeze frame | Portal Screenshot | Portal Screenshot | Root `GetImage` |
| Window rectangles | ShareZ Helper extension, else GNOME Shell a11y tree | KWin script | EWMH |
| Elements inside windows | AT-SPI | AT-SPI | AT-SPI |
| A clicked window on its own | ShareZ Helper extension | KWin `ScreenShot2.CaptureWindow` | Composite window pixmap |
| Recording | Portal ScreenCast + PipeWire | Same | `ximagesrc` |

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

## Region capture

`sharez/src/overlay/` follows ShareX's region capture (`ShareX.ScreenCaptureLib`), in "quick
capture" mode: releasing the mouse captures straight away, and there are no resize handles.

- `selector.rs` is the pure state machine (drag threshold, square and move modifiers, cancel
  rules, magnifier zoom) in buffer pixels, with unit tests.
- `view.rs` draws the frozen frame, dimming, the two-tone dashed border, size label and the
  magnifier with `GtkSnapshot`, scaling the frame with nearest-neighbour filtering.
- `mod.rs` owns the full-screen window and its input controllers, and the `Snapper`, which finds
  what is under the pointer.

### Window and element snapping

Windows are listed topmost first. Elements come from AT-SPI, with two wrinkles:

- On Wayland apps only know positions relative to their own window, so each listed window is
  matched to the app's accessible toplevel by size (and title, when there is one) and element
  rectangles are offset by the window's client area. On GNOME without the helper extension the
  same match tells how much GTK 4 shadow to trim from the shell's rectangle.
- GTK 4 answers a hit test with the deepest widget, often a button's label. ShareZ walks back up
  through the parents and snaps a label, icon or image to the control that contains it.

The ShareZ Helper GNOME extension exists because GNOME Shell's accessibility tree is not in
stacking order and `org.gnome.Shell.Introspect` is denied to normal apps. It exposes one D-Bus
method, `GetWindows`, returning the windows on the active workspace from Mutter, topmost first.

### Clicked windows

A click (or Enter) on a highlighted window or element captures that window's own pixels, so other
windows in front of it don't show; a dragged region keeps what was on screen.
`sharez_windows::capture` asks the compositor for the window drawn off screen: the helper extension's
`CaptureWindow` paints the window actor to a PNG as GNOME's screenshot UI does, KWin's
`ScreenShot2.CaptureWindow` writes raw pixels to a pipe (only for apps whose desktop file lists
`X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2`), and on X11 the Composite extension
names the frame window's pixmap when a compositing manager is running. The selection, in logical
coordinates, is cropped from that image at the window's own scale. The overlay stays open until
the window arrives, so the clipboard is still set from a focused window, and if the compositor
can't help ShareZ falls back to the frozen frame.

## Recording

`sharez-record` builds one GStreamer pipeline in-process with `gstreamer-rs`:

```
pipewiresrc | ximagesrc → videocrop → videorate → I420 → H.264 or VP8 ┐
pulsesrc (@DEFAULT_MONITOR@) ┐                                         ├→ mp4mux | webmmux → file
pulsesrc (default input)     ┴→ audiomixer → limiter → AAC or Opus ────┘
```

- **Region.** The overlay hands back the region in logical coordinates. On Wayland the crop is
  set when the stream's caps arrive, from the buffer size and the shared monitor's logical
  rectangle, because at fractional scales the buffers are larger than the logical size (the KDE
  spike run). On X11 `ximagesrc` reads just the region. Sizes are trimmed to even numbers.
- **Encoders** are picked at runtime: `x264enc`, then `openh264enc` (what a stock Fedora has),
  then `vp8enc`. H.264 goes in MP4 with AAC when an AAC encoder is installed, Opus otherwise; VP8
  goes in WebM with Opus. Bitrate-controlled encoders get about 0.12 bits per pixel per frame.
- **Audio** comes through PipeWire's PulseAudio interface. Desktop audio and the microphone are
  mixed into one stereo track, since one track is what players and sharing sites expect. The mix
  stays 32-bit float, where it can go over full scale without distorting, until a peak limiter
  (a pad probe on `identity name=limiter`) brings it under −1 dBFS: the gain drops at once when a
  sample would go over and recovers over about 0.2 s. A single track goes through it too.
- **Portal session.** The ScreenCast restore token is saved in
  `~/.local/state/sharez/screencast-restore-token`, so the share dialog appears only once.
- **Stopping** sends EOS so the muxer writes its index; a recording that doesn't finish within
  10 seconds is reported as possibly unplayable. When the desktop ends the share (the portal
  session's `Closed` signal, or an error from the video source), ShareZ saves just as if Stop had
  been pressed, pushing EOS into the video branch itself if the source can't.
- **Outline.** A transparent, click-through window titled "ShareZ Recording Area" draws a red
  frame just outside the region. Apps can't place windows on Wayland, so the ShareZ Helper
  extension (`PlaceWindow`) does it on GNOME and a KWin script does it on KDE, keeping it above
  other windows on every workspace. On X11 it is placed with EWMH and shaped to a ring with XShape,
  so it works without a compositor. Without the GNOME extension there is no outline.

While recording, the main window minimises itself and shows a timer with Stop and Discard, and a
notification offers Stop.

## Running in the background

The app holds itself open (`ApplicationHoldGuard`), so closing the window only hides it. Everything
goes through app actions (`capture`, `record`, `stop-recording`, `discard-recording`, `show`,
`quit`): the window's buttons, the tray menu, the global shortcuts, notifications, and
`sharez --capture`/`--record`, which GApplication forwards to the running instance over D-Bus.

- **Tray icon.** GNOME has no tray, so the ShareZ Helper extension shows a top-bar icon while
  `io.github.codicusmaximus.ShareZ` is on the session bus and calls the app's actions
  (`org.gtk.Actions.Activate`) from its menu; ShareZ tells it when a recording starts and stops
  (`SetRecording`). ShareZ checks the extension's `Features` for `indicator` and otherwise
  publishes a StatusNotifierItem with `ksni`, which KDE, most X11 panels and GNOME's AppIndicator
  extension show.
- **Global shortcuts.** On Wayland the `GlobalShortcuts` portal binds them. ShareZ registers its
  app ID with the portal's host registry first, lists what the desktop already has for it, and
  only asks to bind (which shows the desktop's dialog) when a shortcut is missing. On X11 ShareZ
  grabs the keys on the root window (with and without Caps Lock and Num Lock) on a thread of its
  own, and keeps them in `settings.ini`.
- **Clipboard.** Wayland only lets the focused app set the clipboard, and with the window hidden
  that is the region overlay. The screenshot is copied while the overlay is still open, before it
  closes, and the clipboard stays served for as long as ShareZ runs.
- **Feedback.** With the window open, results are toasts; with it hidden, notifications. A
  screenshot's click opens the image; a recording's shows the file selected in the file manager
  (`GtkFileLauncher` asks `org.freedesktop.FileManager1.ShowItems`), ready to drag. A button on
  each notification does the other.

## Not built yet

- Multi-monitor overlay: the frozen frame covers all monitors, but the overlay is one full-screen
  window and assumes the screen starts at 0,0. It will become one overlay per monitor using each
  monitor's `Mapping`.
- Moving the cursor with arrow keys, as ShareX does: Wayland doesn't let apps warp the pointer.
- Recording extras: a microphone level meter and gain, and a start countdown.

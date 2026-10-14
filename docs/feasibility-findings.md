# Feasibility findings

Results from the probes in `spikes/feasibility`, run on real sessions. Each section says what
was measured and what it means for ShareZ.

| Session | Machine | Status |
|---|---|---|
| GNOME 50.5 Wayland, Fedora 44, GStreamer 1.28.7 | 2256×1504 laptop panel, scale 1 | Done |
| KDE Plasma 6.7.5 Wayland (KWin 6.7.5), Fedora 44, GStreamer 1.28.8 | Same laptop, 2256×1504 panel at **scale 1.35** (1671×1114 logical) | Done |
| X11 session | | Pending (only XWayland observed so far, on both desktops) |

## GNOME Wayland

### Freeze frame (screenshot before the region overlay)

- `org.freedesktop.portal.Screenshot` with `interactive=false` works with **no dialog**, at
  full resolution (2256×1504).
- It takes **~510–540 ms** per shot and writes a file to `~/Pictures` that we must delete.
- `org.gnome.Shell.Screenshot` is denied to third-party apps ("Screenshot is not allowed").

**Implication:** the portal is the only freeze-frame path on GNOME. Half a second is acceptable
for a hotkey, but the overlay should appear immediately with a "capturing" state rather than
wait silently. A screencast-based grab (one frame from a PipeWire stream with a restore token)
may be faster and is worth measuring in step 2.

### Window rectangles (click-to-snap on windows)

- `org.gnome.Shell.Introspect.GetWindows` is denied ("GetWindows is not allowed").
- **GNOME Shell's own accessibility tree lists every window actor** as a panel named
  `Wayland window` or `X11 window`, with its on-screen rectangle. 26 were reported, and they
  match independent data exactly:
  - XWayland windows match the X11 EWMH rectangles pixel for pixel (Winamp `388,477 275×116`,
    Steam `675,251 1500×974`).
  - Brave `1839×1098` and Telegram `981×1091` match their apps' own accessible frame sizes.
  - GTK 4 / libadwaita apps are consistently **50 px larger** in both dimensions than the app's
    own frame (Nautilus 1330×992 → 1380×1042, Text Editor 2084×1063 → 2134×1113, Ptyxis,
    Bottles). That is the 25 px client-side shadow on each side, so the shell reports the buffer
    rectangle and the visible frame is inset by 25 px.
- The list includes windows on other workspaces and minimised ones; the names don't identify the
  app, and several same-sized windows (e.g. cascaded Nautilus windows) can only be told apart by
  position and stacking.

**Implication:** we likely don't need a GNOME Shell extension for window snapping. Use the shell's
a11y tree for rectangles, inset CSD shadows by matching against the app's own frame size, and
cross-check against the frozen image for visibility. Keep the helper extension as an optional
fallback if a future GNOME release changes the shell's a11y tree.

### Elements inside windows

- GTK 3/4, libadwaita, Qt (Telegram) and Firefox expose detailed trees: buttons, menu items, text
  fields, list rows, tabs.
- **Coordinates are window-relative** on Wayland (screen and window extents are identical), so an
  element's screen position = shell window rectangle (shadow-inset) + element extents.
- `GetAccessibleAtPoint` hit tests take **0.1–3 ms**, fast enough to follow the mouse.
- Chromium, Brave, Steam (CEF) and Electron apps expose only empty frames unless their
  accessibility is forced on. These need an image-based fallback (edge/rectangle detection on the
  frozen frame), which is what ShareX effectively does for apps without UI Automation data.

### Region recording with desktop audio and mic

- Portal ScreenCast → PipeWire → crop → H.264 + two Opus tracks in Matroska works.
- 8 s requested → 7.93 s recorded, 30 fps (238 frames), crop exact (1128×752 from 2256×1504).
- Desktop audio (`@DEFAULT_MONITOR@`, stereo) and mic (default source, mono) land on **separate
  tracks**. With music playing, desktop audio averaged −18 dB.
- The mic track averaged −2 dB and hit 0 dB: it is clipping. ShareZ needs per-track gain and a
  level meter before recording.
- **Restore tokens work:** the first recording showed the share dialog (1.9 s to pick); the second
  started in **18 ms with no dialog**.
- No hardware H.264 encoder was available (`vah264enc`/`vaapih264enc` missing, `x264enc` not
  installed); `openh264enc` and `vp8enc` were. The encoder must be chosen at runtime with a
  fallback chain, and Flatpak packaging should bundle one.

### XWayland

- X11 apps are visible via EWMH with correct rectangles, but grabbing the root window fails with
  `BadMatch`. On Wayland, X11 APIs are only useful for identifying X11 windows, never for pixels.

## KDE Plasma Wayland

### Freeze frame

- The portal Screenshot with `interactive=false` works with **no dialog** at full physical
  resolution (2256×1504), in **~175–180 ms**: three times faster than GNOME.
- It also writes to `~/Pictures`. Both shots in the same second got the same file name, so the
  second overwrote the first. ShareZ must move the file away immediately, not rely on its name.
- `org.kde.KWin.ScreenShot2` refuses us (`NoAuthorized`) as expected. It needs an installed
  `.desktop` file with `X-KDE-DBUS-Restricted-Interfaces`, which a packaged ShareZ can have. It is a
  possible faster path later, not a requirement.

### Window rectangles

- A temporary **KWin script works**: it ran and called back in 9 ms and listed all 9 windows in
  stacking order, with caption, class, pid, type flags, and both `frameGeometry` and
  `bufferGeometry` in logical coordinates (fractional at scale 1.35).
- KWin gives the decoration and shadow geometry directly, so nothing has to be guessed:
  - Server-side decorated apps (Konsole, Chirp): the frame includes the 27 px title bar; the
    buffer is the client area.
  - Client-side decorated GTK 4 apps (Ptyxis): the frame is the visible window; the buffer adds
    about 25 px of shadow on each side (frame 1500×1066, buffer 1550×1116).
- KWin's own accessibility node has no children, so there is no GNOME-style window list in the
  a11y tree. The KWin script makes it unnecessary.

**Implication:** KDE window snapping is a KWin script, loaded at overlay time over D-Bus. The
product should read `clientGeometry` as well, which gives the client area without the frame
arithmetic above.

### Elements inside windows

- Qt apps (Konsole, Plasma's own panel) and GTK 4 (Ptyxis) expose full trees. Hit tests at a
  window's centre took **0.08–0.7 ms** and reached the right element (e.g. the terminal widget).
- Window-relative extents are reliable. **Screen extents are not:** for GTK 4 they are wrong (a
  widget at y=86 in the window reports y=0 on screen), for Qt they equal the window extents. Only
  Plasma's layer-shell panel reports true screen positions.
- The app's toplevel size equals the **client area** in every case (Konsole 911×552, Chirp
  1284×908, Ptyxis 1500×1066), so element on screen = client-area origin + window extents.
- Brave and the Claude desktop app (Electron) expose empty frames, as on GNOME. So does
  **Chirp**: snapping inside it will need the image-based fallback.

### Region recording

- Portal ScreenCast → PipeWire → crop → H.264 + two Opus tracks works: 7.93 s, 238 frames at
  30 fps, desktop audio stereo and mic mono on separate tracks.
- **Restore tokens work:** first start 3.0 s (share dialog), second start **19 ms** with no dialog.
- **Fractional scaling matters.** The portal reports the stream size in logical pixels (1671×1114)
  but the PipeWire buffers are physical (2256×1504). The spike cropped with logical numbers, so
  its "centre" region came out 1419×946 from the wrong origin instead of 1128×752 centred. ShareZ
  must scale crops by buffer size ÷ logical size and round to even dimensions for 4:2:0 encoders
  (the encoder silently trimmed 1419 to 1418).
- Same encoders as GNOME: `openh264enc` and `vp8enc`, no hardware or x264.
- The mic clipped again (mean −1.8 dB, peak 0 dB) while desktop audio averaged −13.8 dB, so the
  mic gain is the hardware's, not GNOME's. Input gain and a level meter stay a requirement.

## Conclusions for step 2

| Need | GNOME Wayland | KDE Wayland | X11 |
|---|---|---|---|
| Freeze frame | Portal Screenshot, ~0.5 s | Portal Screenshot, ~0.18 s | `GetImage`/MIT-SHM (to verify) |
| Window rectangles | gnome-shell a11y tree, shadow inset by size match; extension as fallback | KWin script (`clientGeometry`) | EWMH + `_GTK_FRAME_EXTENTS` |
| Elements in windows | AT-SPI, window-relative extents | AT-SPI, window-relative extents | AT-SPI (screen extents expected to be valid) |
| No a11y data (Chromium, Electron, webviews) | Image-based fallback | Image-based fallback | Image-based fallback |
| Recording | Portal ScreenCast + PipeWire, restore token | Same | `ximagesrc` (to verify) |

Every Wayland path ShareZ needs works without root, without a compositor patch and, after the
first recording, without dialogs. That settles the stack:

- One `CaptureBackend` trait with portal (both Wayland desktops) and X11 implementations, and a
  separate `WindowSource` trait with GNOME, KWin and EWMH implementations.
- Coordinates are kept in logical pixels per monitor with its scale factor, and converted to
  buffer pixels only when touching images or crops.
- Recording drives GStreamer in-process through `gstreamer-rs` instead of `gst-launch`, with a
  runtime encoder fallback chain, per-track gain and level meters.
- Element snapping uses AT-SPI where an app provides it and edge detection on the frozen frame
  everywhere else.

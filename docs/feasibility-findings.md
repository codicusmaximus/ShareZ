# Feasibility findings

Results from the probes in `spikes/feasibility`, run on real sessions. Each section says what
was measured and what it means for ShareZ.

| Session | Machine | Status |
|---|---|---|
| GNOME 50.5 Wayland, Fedora 44, GStreamer 1.28.7 | 2256×1504 laptop panel, scale 1 | Done |
| KDE Plasma Wayland | | Pending |
| X11 session | | Pending (only XWayland observed so far) |

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

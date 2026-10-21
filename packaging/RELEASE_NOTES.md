ShareZ captures screenshots and recordings of regions, windows and elements inside windows on Linux, with global shortcuts and a tray icon. It works on Wayland (GNOME and KDE Plasma) and X11.

## What's new

- **An annotator, ported from ShareX's image editor.** Draw rectangles, ellipses, arrows, lines and freehand; add outlined or boxed text, speech balloons and numbered steps; blur, pixelate or highlight anything private; magnify, crop, rotate and flip. It has ShareX's 17 tools, their toolbar options and keys, plus undo and redo. Open it from the after-capture popup (**E**), from a hotkey set to **Open in Annotator**, or on any image with `sharez --annotate FILE`.
- **Hotkeys you can edit.** Under **Hotkeys** in the ShareZ window, change, add or delete hotkeys. Each one picks what to capture (a region, the whole screen, the last region, or a recording) and what to do with a screenshot (save and copy, copy only, save only, open in the annotator, or ask).
- **An after-capture popup** shows the screenshot and asks what to do with it: save and copy (Enter), annotate (E), copy (C), save (S), save as (A), save and open it (O), or discard (Esc).
- **New default hotkeys:** Print captures the whole screen and Super+Shift+S captures a region, then asks. Ctrl+Print and Ctrl+Shift+Print still capture a region and record. If you're upgrading, your existing keys stay as they were.
- **Capture Screen** is now in the top-bar and tray menu.

## Install

Download the one file for your distribution from the list below, then install it with your package manager from the folder you saved it in. The package manager installs everything ShareZ needs; there is nothing to build.

| Distribution | Download | Install |
|---|---|---|
| Ubuntu 24.04 or newer, Debian 13 or newer, Linux Mint 22, Pop!_OS 24.04 | `sharez_*_amd64.deb` | `sudo apt install ./sharez_*_amd64.deb` |
| Fedora 40 or newer | `sharez-*.x86_64.rpm` | `sudo dnf install ./sharez-*.x86_64.rpm` |
| Arch Linux, EndeavourOS, Manjaro | `sharez-*-x86_64.pkg.tar.zst` | `sudo pacman -U ./sharez-*-x86_64.pkg.tar.zst` |

Older releases (Debian 12, Ubuntu 22.04) are not supported: their GTK is too old.

Then open **ShareZ** from your app launcher.

**On GNOME**, log out and back in once after installing. ShareZ comes with a small GNOME Shell extension that gives it its top-bar icon and lets it see windows, and GNOME only notices new extensions at login. ShareZ turns it on by itself the next time it starts.

**The first screenshot** asks whether ShareZ may take screenshots; allow it once and it won't ask again.

**Upgrading from 0.1?** Install the new file the same way; your settings and hotkeys are kept. On GNOME, log out and back in once more so the top-bar menu picks up Capture Screen.

### Optional: MP4 recordings

Out of the box ShareZ records WebM video, which plays everywhere modern. If you'd rather have MP4 (H.264), install these. Ubuntu and Debian already did if your package manager installs recommended packages (the default), and Fedora did on its own.

- Ubuntu / Debian: `sudo apt install gstreamer1.0-plugins-ugly gstreamer1.0-plugins-bad gstreamer1.0-libav`
- Fedora: `sudo dnf install gstreamer1-plugin-openh264 gstreamer1-plugins-bad-free`
- Arch: `sudo pacman -S --asdeps gst-plugins-ugly gst-plugins-bad gst-libav`

## Using it

| Hotkey | Does |
|---|---|
| Print | Captures the whole screen |
| Ctrl+Print | Captures a region |
| Super+Shift+S | Captures a region, then asks what to do with it |
| Ctrl+Shift+Print | Starts or stops recording a region |

- On GNOME and KDE the desktop asks you to confirm the shortcuts the first time. Both desktops use Print for their own screenshot tool, so to give it to ShareZ, move theirs to another key first (on GNOME: Settings → Keyboard → View and Customize Shortcuts → Screenshots).
- Hover a window or a button and click to capture just that, or drag to draw a region. Screenshots are copied to the clipboard and saved in Pictures/ShareZ; recordings go to Videos/ShareZ.
- Change any of this under **Hotkeys** in the ShareZ window.
- Closing the window keeps ShareZ running in the top bar or tray. Turn on **Start at Login** to have it ready every time.

## Uninstall

- Ubuntu / Debian: `sudo apt remove sharez`
- Fedora: `sudo dnf remove sharez`
- Arch: `sudo pacman -R sharez`

## Feedback

Found a problem or have an idea? Open an issue at https://github.com/codicusmaximus/ShareZ/issues and say which distribution and desktop you use.

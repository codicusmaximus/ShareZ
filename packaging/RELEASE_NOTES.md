ShareZ captures screenshots and recordings of regions, windows and elements inside windows on Linux, with global shortcuts and a tray icon. It works on Wayland (GNOME and KDE Plasma) and X11.

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

### Optional: MP4 recordings

Out of the box ShareZ records WebM video, which plays everywhere modern. If you'd rather have MP4 (H.264), install these. Ubuntu and Debian already did if your package manager installs recommended packages (the default), and Fedora did on its own.

- Ubuntu / Debian: `sudo apt install gstreamer1.0-plugins-ugly gstreamer1.0-plugins-bad gstreamer1.0-libav`
- Fedora: `sudo dnf install gstreamer1-plugin-openh264 gstreamer1-plugins-bad-free`
- Arch: `sudo pacman -S --asdeps gst-plugins-ugly gst-plugins-bad gst-libav`

## Using it

- **Ctrl+Print** captures a region; **Ctrl+Shift+Print** starts or stops recording. On GNOME and KDE the desktop asks you to confirm the shortcuts the first time.
- Hover a window or a button and click to capture just that, or drag to draw a region. Screenshots are copied to the clipboard and saved in Pictures/ShareZ; recordings go to Videos/ShareZ.
- Closing the window keeps ShareZ running in the top bar or tray. Turn on **Start at Login** to have it ready every time.

## Uninstall

- Ubuntu / Debian: `sudo apt remove sharez`
- Fedora: `sudo dnf remove sharez`
- Arch: `sudo pacman -R sharez`

## Feedback

Found a problem or have an idea? Open an issue at https://github.com/codicusmaximus/ShareZ/issues and say which distribution and desktop you use.

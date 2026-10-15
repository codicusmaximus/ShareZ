#!/bin/sh
# Builds ShareZ and installs it for the current user: the app in ~/.local/bin, its launcher entry,
# and on GNOME the ShareZ Helper extension. Run it again after pulling to update.
set -eu
cd "$(dirname "$0")"

cargo build --release
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"
install -m 755 target/release/sharez "$HOME/.local/bin/sharez"
# The launcher entry names the app ID portals and notifications know ShareZ by.
sed "s|^Exec=sharez|Exec=$HOME/.local/bin/sharez|" data/io.github.codicusmaximus.ShareZ.desktop \
    > "$HOME/.local/share/applications/io.github.codicusmaximus.ShareZ.desktop"
echo "Installed ShareZ to $HOME/.local/bin/sharez"

case "${XDG_CURRENT_DESKTOP:-}" in
*GNOME*)
    uuid=sharez-helper@codicusmaximus.github.io
    dir="$HOME/.local/share/gnome-shell/extensions/$uuid"
    mkdir -p "$dir"
    cp gnome-extension/$uuid/* "$dir/"
    if gnome-extensions info "$uuid" >/dev/null 2>&1; then
        gnome-extensions enable "$uuid" || true
        echo "Updated the ShareZ Helper extension. Log out and back in to load the new version."
    else
        echo "Installed the ShareZ Helper extension. Log out and back in, then run:"
        echo "  gnome-extensions enable $uuid"
    fi
    ;;
esac

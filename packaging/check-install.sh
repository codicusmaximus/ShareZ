#!/bin/sh
# Checks an installed ShareZ package: its libraries resolve, the GStreamer elements a recording
# needs are present, and its files are in place. The release workflow runs it on each distribution.
set -e
echo "== libraries"; ldd /usr/bin/sharez | grep "not found" && exit 1 || echo "all libraries found"
echo "== GStreamer elements"
missing=""
for e in pipewiresrc ximagesrc videocrop videorate videoconvert pulsesrc audioconvert audioresample audiomixer opusenc vp8enc webmmux queue filesink identity; do
  gst-inspect-1.0 --exists "$e" || missing="$missing $e"
done
[ -z "$missing" ] && echo "all elements present" || { echo "MISSING:$missing"; exit 1; }
echo "== files"; ls /usr/share/applications/io.github.codicusmaximus.ShareZ.desktop /usr/share/gnome-shell/extensions/sharez-helper@codicusmaximus.github.io/

#!/bin/sh
# Makes AppIcon.icns and assets/logo.png (the sidebar and window icon) from
# assets/ferriteweazle.png.
set -eu
cd "$(dirname "$0")/../.."
art=assets/ferriteweazle.png
sips -z 256 256 "$art" --out assets/logo.png >/dev/null
iconset=target/AppIcon.iconset
rm -rf "$iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    double=$((size * 2))
    sips -z "$size" "$size" "$art" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z "$double" "$double" "$art" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o packaging/macos/AppIcon.icns

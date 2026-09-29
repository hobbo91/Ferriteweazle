#!/bin/sh
# Writes OUT, the source of the LGPL code in the Linux packages, to publish
# beside them: wayland-protocols-plasma's crate, whose KDE protocol
# descriptions the program's Wayland code is generated from, and the
# AppImage runtime with the libfuse and squashfuse it links.
#
#   packaging/linux/lgpl-sources.sh dist/LGPL-sources-VERSION.tar
set -eu
cd "$(dirname "$0")/../.."
. packaging/linux/runtime.sh
out=$1
cache=target/appimage-cache
work=target/lgpl-sources
rm -rf "$work"
mkdir -p "$work"
fetch "type2-runtime-$RUNTIME_COMMIT.tar.gz" \
    "https://github.com/AppImage/type2-runtime/archive/$RUNTIME_COMMIT.tar.gz"
fetch "fuse-$FUSE.tar.xz" \
    "https://github.com/libfuse/libfuse/releases/download/fuse-$FUSE/fuse-$FUSE.tar.xz"
fetch "squashfuse-$SQUASHFUSE.tar.gz" "https://github.com/vasi/squashfuse/archive/$SQUASHFUSE.tar.gz"
cp "$cache/type2-runtime-$RUNTIME_COMMIT.tar.gz" "$cache/fuse-$FUSE.tar.xz" \
    "$cache/squashfuse-$SQUASHFUSE.tar.gz" "$work/"

# The crate as Cargo.lock pins it, from cargo's own download.
lock() {
    awk -v field="$1" '/^name = "wayland-protocols-plasma"$/ { found = 1 }
        found && $1 == field { gsub(/"/, "", $3); print $3; exit }' Cargo.lock
}
crate=wayland-protocols-plasma-$(lock version).crate
registry=${CARGO_HOME:-$HOME/.cargo}/registry/cache
[ -n "$(find "$registry" -name "$crate" 2>/dev/null)" ] ||
    cargo fetch --locked --target x86_64-unknown-linux-gnu
cp "$(find "$registry" -name "$crate" | head -1)" "$work/"
echo "$(lock checksum)  $crate" | (cd "$work" && shasum -a 256 -c -) >/dev/null

cat >"$work/README.txt" <<EOF
The source of the LGPL code in Ferriteweazle's Linux packages.

$crate
    winit's crate for KDE's Wayland protocols. Its code is under the MIT
    licence; its plasma-wayland-protocols folder holds the protocol
    descriptions, under the GNU LGPL 2.1 or later, from which the
    program's Wayland code is generated. To build the program with
    changed descriptions, use Ferriteweazle's source:
    https://github.com/hobbo91/ferriteweazle

type2-runtime-$RUNTIME_COMMIT.tar.gz
    The AppImage runtime at the front of each AppImage (MIT licence).
    Its scripts build it; patches/libfuse is its change to libfuse.

fuse-$FUSE.tar.xz
    libfuse, which the runtime links statically. Its library, lib/ and
    include/, is under the GNU LGPL 2.1.

squashfuse-$SQUASHFUSE.tar.gz
    squashfuse (BSD 2-clause), which the runtime also links.
EOF
tar -cf "$out" -C "$work" .

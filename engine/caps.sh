#!/bin/sh
# Builds the SPS/CAPS library, which gw needs for IPF and CT Raw images, into
# DEST/caps for TRIPLE. Only packages ship it (its licence forbids commercial
# use); a source build leaves gw to find one the user installed. Downloads
# the source, checked against engine/caps.sha256.
#
#   engine/caps.sh TRIPLE DEST
set -eu
cd "$(dirname "$0")/.."
. engine/versions
triple=$1
dest=$2

name=capsimage-$CAPS_COMMIT.tar.gz
cache=target/engine-cache
mkdir -p "$cache"
if [ ! -f "$cache/$name" ]; then
    curl -fL --retry 3 -o "$cache/$name.part" \
        "https://github.com/simonowen/capsimage/archive/$CAPS_COMMIT.tar.gz"
    mv "$cache/$name.part" "$cache/$name"
fi
grep " $name\$" engine/caps.sha256 | (cd "$cache" && shasum -a 256 -c -) >/dev/null
work=target/caps-$triple
rm -rf "$work"
mkdir -p "$work"
tar -xzf "$cache/$name" -C "$work" --strip-components=1

# What CMakeLists.txt configures, written here so no cmake is needed.
printf '#define CAPS_LIB_RELEASE 5\n#define CAPS_LIB_REVISION 1\n' >"$work/CapsLibVersion.h"
arch=${triple%%-*}
case "$triple" in
    *apple-darwin)
        [ "$arch" = aarch64 ] && arch=arm64
        cxx="clang++ -arch $arch -mmacosx-version-min=10.15 -dynamiclib \
            -install_name @rpath/libcapsimage.dylib -DHAVE_STRUCT_DIRENT_D_TYPE=1"
        out=libcapsimage.dylib ;;
    *linux-gnu)
        # Like the program, no newer glibc than 2.17; libc++ goes in statically.
        cxx="zig c++ -target $arch-linux-gnu.2.17 -shared -fPIC -s -Wl,-soname,libcapsimage.so.5 \
            -DHAVE_STRUCT_DIRENT_D_TYPE=1"
        out=libcapsimage.so.5 ;;
    *windows-msvc)
        # MSVC's libraries, linked statically: Python ships no C++ runtime.
        cxx="clang++ --target=$triple -fuse-ld=lld -shared -fms-runtime-lib=static \
            -D_CRT_SECURE_NO_WARNINGS -I$work/src/Compatibility"
        out=CAPSImg.dll ;;
    *) echo "caps: no build for $triple" >&2; exit 1 ;;
esac
touch "$work/config.h"
mkdir -p "$dest/caps"
# shellcheck disable=SC2086
$cxx -std=c++11 -O2 -w -DHAVE_CONFIG_H=1 -I"$work" -I"$work/src/LibIPF" \
    -I"$work/src/CAPSImg" -I"$work/src/Core" -I"$work/src/Codec" -I"$work/src/Device" \
    "$work"/src/Core/*.cpp "$work"/src/CAPSImg/*.cpp "$work"/src/Codec/*.cpp \
    -o "$dest/caps/$out"
# Latin-1 with CRLF line ends in the source.
iconv -f ISO-8859-1 -t UTF-8 "$work/LICENCE.txt" | tr -d '\r' >"$dest/caps/LICENCE.txt"
echo "caps: $dest/caps/$out"

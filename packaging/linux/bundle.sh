#!/bin/sh
# Builds, for ARCH (x86_64 or aarch64, this computer's if none), a tarball
# dist/Ferriteweazle-VERSION-linux-ARCH.tar.gz and an AppImage
# dist/Ferriteweazle-VERSION-ARCH.AppImage that run on glibc 2.17 or newer.
# The bundle is rebuilt first if gw has a newer release. Needs cargo-zigbuild,
# zig, meson, ninja, bison, bsdtar, patchelf and objdump; downloads
# appimagetool, the AppImage runtime, and the source and libraries of lib/
# (libraries.sh).
#
#   packaging/linux/bundle.sh           # this computer
#   packaging/linux/bundle.sh x86_64    # another processor
set -eu
cd "$(dirname "$0")/../.."
. bundle/greaseweazle.sh
. packaging/linux/runtime.sh
. packaging/linux/libraries.sh
APPIMAGETOOL=1.9.1
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
arch=${1:-$(uname -m)}
case "$arch" in
    x86_64 | aarch64) ;;
    *) echo "linux: no build for $arch" >&2; exit 1 ;;
esac
triple=$arch-unknown-linux-gnu

refresh "$triple"
data=$(bundle_dir "$triple")
if [ ! -f "$data/49-greaseweazle.rules" ]; then
    echo "linux: $data has no udev rule; rebuild it with bundle/build.sh $triple" >&2
    exit 1
fi
# The program links only glibc, and zig links it against 2.17's; it opens the
# libraries in lib/ beside it (build.rs), and the rest of what it needs from
# the system.
cargo zigbuild --release --locked --target "$triple.2.17"
program=target/$triple/release/ferriteweazle

# A folder to unpack anywhere: the program, its bundle and libraries beside
# it, and a menu entry with its icon, since a Linux program carries none of
# its own.
stage=target/linux-$arch
top=$stage/Ferriteweazle
rm -rf "$stage"
mkdir -p "$top" dist
cp "$program" "$top/ferriteweazle"
cp -a "$data" "$top/greaseweazle"
libraries "$top/lib" "$arch"
cp packaging/linux/ferriteweazle.desktop packaging/linux/README.txt "$top/"
cp assets/logo.png "$top/ferriteweazle.png"
cp LICENSE "$top/LICENSE.txt"
notices=$top/THIRD-PARTY-NOTICES.txt
packaging/notices.sh "$top/greaseweazle" "$triple" >"$notices"
sed "s/@VERSION@/$version/g" packaging/licences/linux-lgpl.txt >>"$notices"
cat packaging/licences/linux-libraries.txt >>"$notices"
# Nothing in it may need a newer glibc than 2.17.
newest=$(find "$top" -type f -exec objdump -T {} + 2>/dev/null | grep -o 'GLIBC_[0-9.]*' |
    sort -uV | tail -1)
if [ "$(printf '%s\n' "$newest" GLIBC_2.17 | sort -V | tail -1)" != GLIBC_2.17 ]; then
    echo "linux: something in $top needs $newest" >&2
    exit 1
fi
tarball=dist/Ferriteweazle-$version-linux-$arch.tar.gz
tar -czf "$tarball" --owner=0 --group=0 --numeric-owner -C "$stage" Ferriteweazle

# The same as one file. AppRun is the program, which finds its bundle beside
# it, and its libraries in usr/lib by lib/ beside it.
appdir=$stage/AppDir
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/doc/ferriteweazle"
cp "$program" "$appdir/usr/bin/ferriteweazle"
cp -a "$data" "$appdir/usr/bin/greaseweazle"
cp -a "$top/lib" "$appdir/usr/lib"
ln -s ../lib "$appdir/usr/bin/lib"
cp LICENSE "$appdir/usr/share/doc/ferriteweazle/"
# The AppImage also carries the runtime at its front.
{ cat "$notices"; sed "s/@VERSION@/$version/g" "packaging/licences/appimage-runtime-$RUNTIME.txt"; } \
    >"$appdir/usr/share/doc/ferriteweazle/THIRD-PARTY-NOTICES.txt"
ln -s usr/bin/ferriteweazle "$appdir/AppRun"
cp packaging/linux/ferriteweazle.desktop "$appdir/"
cp assets/logo.png "$appdir/ferriteweazle.png"
ln -s ferriteweazle.png "$appdir/.DirIcon"

cache=target/appimage-cache
host=$(uname -m)
tool=appimagetool-$APPIMAGETOOL-$host.AppImage
runtime=runtime-$RUNTIME-$arch
fetch "$tool" "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL/appimagetool-$host.AppImage"
fetch "$runtime" "https://github.com/AppImage/type2-runtime/releases/download/$RUNTIME/runtime-$arch"
# appimagetool is unpacked once and run from the cache: that needs no FUSE,
# and leaves nothing in /tmp.
unpacked=$cache/appimagetool-$APPIMAGETOOL-$host
if [ ! -d "$unpacked" ]; then
    chmod +x "$cache/$tool"
    rm -rf "$cache/squashfs-root"
    (cd "$cache" && "./$tool" --appimage-extract >/dev/null)
    mv "$cache/squashfs-root" "$unpacked"
fi
image=dist/Ferriteweazle-$version-$arch.AppImage
rm -f "$image"
ARCH=$arch "$unpacked/AppRun" --no-appstream --runtime-file "$cache/$runtime" "$appdir" "$image"
du -sh "$tarball" "$image"

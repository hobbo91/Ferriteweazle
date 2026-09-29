#!/bin/sh
# Builds, for ARCH (x86_64 or aarch64, this computer's if none), a tarball
# dist/Ferriteweazle-VERSION-linux-ARCH.tar.gz and an AppImage
# dist/Ferriteweazle-VERSION-ARCH.AppImage that run on glibc 2.17 or newer.
# The engine is rebuilt first if gw has a newer release. Needs cargo-zigbuild
# and zig; downloads appimagetool and the AppImage runtime.
#
#   packaging/linux/bundle.sh           # this computer
#   packaging/linux/bundle.sh x86_64    # another processor
set -eu
cd "$(dirname "$0")/../.."
. engine/greaseweazle.sh
APPIMAGETOOL=1.9.1
RUNTIME=20251108
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
arch=${1:-$(uname -m)}
case "$arch" in
    x86_64 | aarch64) ;;
    *) echo "linux: no build for $arch" >&2; exit 1 ;;
esac
triple=$arch-unknown-linux-gnu

refresh "$triple"
data=$(engine_dir "$triple")
if [ ! -f "$data/49-greaseweazle.rules" ]; then
    echo "linux: $data has no udev rule; rebuild it with engine/build.sh $triple" >&2
    exit 1
fi
# The program links only glibc, and zig links it against 2.17's.
cargo zigbuild --release --locked --target "$triple.2.17"
program=target/$triple/release/ferriteweazle

# A folder to unpack anywhere: the program, its engine beside it, and a menu
# entry with its icon, since a Linux program carries none of its own.
stage=target/linux-$arch
top=$stage/Ferriteweazle
rm -rf "$stage"
mkdir -p "$top" dist
cp "$program" "$top/ferriteweazle"
cp -a "$data" "$top/ferriteweazle-data"
cp packaging/linux/ferriteweazle.desktop packaging/linux/README.txt "$top/"
cp assets/logo.png "$top/ferriteweazle.png"
cp LICENSE "$top/LICENSE.txt"
packaging/notices.sh "$top/ferriteweazle-data" "$triple" >"$top/THIRD-PARTY-NOTICES.txt"
tarball=dist/Ferriteweazle-$version-linux-$arch.tar.gz
tar -czf "$tarball" --owner=0 --group=0 --numeric-owner -C "$stage" Ferriteweazle

# The same as one file. AppRun is the program, which finds its engine beside it.
appdir=$stage/AppDir
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/doc/ferriteweazle"
cp "$program" "$appdir/usr/bin/ferriteweazle"
cp -a "$data" "$appdir/usr/bin/ferriteweazle-data"
cp LICENSE "$top/THIRD-PARTY-NOTICES.txt" "$appdir/usr/share/doc/ferriteweazle/"
ln -s usr/bin/ferriteweazle "$appdir/AppRun"
cp packaging/linux/ferriteweazle.desktop "$appdir/"
cp assets/logo.png "$appdir/ferriteweazle.png"
ln -s ferriteweazle.png "$appdir/.DirIcon"

cache=target/appimage-cache
host=$(uname -m)
tool=appimagetool-$APPIMAGETOOL-$host.AppImage
runtime=runtime-$RUNTIME-$arch
mkdir -p "$cache"
for file in "$tool" "$runtime"; do
    case "$file" in
        appimagetool-*) url=https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL/appimagetool-$host.AppImage ;;
        *) url=https://github.com/AppImage/type2-runtime/releases/download/$RUNTIME/runtime-$arch ;;
    esac
    if [ ! -f "$cache/$file" ]; then
        curl -fL --retry 3 -o "$cache/$file.part" "$url"
        mv "$cache/$file.part" "$cache/$file"
    fi
    grep " $file\$" packaging/linux/appimage.sha256 | (cd "$cache" && sha256sum -c -)
done
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

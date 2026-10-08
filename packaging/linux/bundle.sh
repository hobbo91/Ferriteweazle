#!/bin/sh
# Builds, for ARCH (x86_64 or aarch64, this computer's if none), a tarball
# dist/Ferriteweazle-VERSION-linux-ARCH.tar.gz and an AppImage
# dist/Ferriteweazle-VERSION-ARCH.AppImage that run on glibc 2.17 or newer,
# and the AppImage's zsync file, for AppImageUpdate. With SIGN_KEY, a key's
# fingerprint, the AppImage is signed by that key, which gpg-agent must hold
# unlocked. The bundle is rebuilt first if gw has a newer release. Needs
# cargo-zigbuild, zig, meson, ninja, bison, bsdtar, patchelf, objdump and
# appstreamcli; downloads appimagetool, the AppImage runtime, and the source
# and libraries of lib/ (libraries.sh).
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
# The desktop entry, its icon and its AppStream metadata are named for the
# window's app id (src/main.rs).
id=io.github.hobbo91.ferriteweazle
# A signature must not wait on a passphrase.
if [ -n "${SIGN_KEY:-}" ] && ! gpg --batch --pinentry-mode error --local-user "$SIGN_KEY" \
    --clearsign </dev/null >/dev/null 2>&1; then
    echo "linux: unlock $SIGN_KEY first:" \
        "echo | gpg --pinentry-mode loopback --clearsign --local-user $SIGN_KEY >/dev/null" >&2
    exit 1
fi

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
cp "packaging/linux/$id.desktop" packaging/linux/README.txt "$top/"
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
share=$appdir/usr/share
mkdir -p "$appdir/usr/bin" "$share/doc/ferriteweazle" "$share/applications" \
    "$share/icons/hicolor/256x256/apps" "$share/metainfo"
cp "$program" "$appdir/usr/bin/ferriteweazle"
cp -a "$data" "$appdir/usr/bin/greaseweazle"
cp -a "$top/lib" "$appdir/usr/lib"
ln -s ../lib "$appdir/usr/bin/lib"
cp LICENSE "$share/doc/ferriteweazle/"
# The AppImage also carries the runtime at its front.
{ cat "$notices"; sed "s/@VERSION@/$version/g" "packaging/licences/appimage-runtime-$RUNTIME.txt"; } \
    >"$share/doc/ferriteweazle/THIRD-PARTY-NOTICES.txt"
ln -s usr/bin/ferriteweazle "$appdir/AppRun"
cp "packaging/linux/$id.desktop" "$appdir/"
cp "packaging/linux/$id.desktop" "$share/applications/"
cp assets/logo.png "$appdir/$id.png"
cp assets/logo.png "$share/icons/hicolor/256x256/apps/$id.png"
ln -s "$id.png" "$appdir/.DirIcon"
# AppStream's release is dated by SOURCE_DATE_EPOCH, as release.sh sets it,
# or else today.
date=$(date -u ${SOURCE_DATE_EPOCH:+-d "@$SOURCE_DATE_EPOCH"} +%Y-%m-%d)
sed -e "s/@VERSION@/$version/" -e "s/@DATE@/$date/" "packaging/linux/$id.metainfo.xml" \
    >"$share/metainfo/$id.metainfo.xml"
# appimagetool's own check also fetches each screenshot, which for a release
# is on GitHub only once its branch is merged.
appstreamcli validate-tree --no-net "$appdir"

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
zsync=Ferriteweazle-$version-$arch.AppImage.zsync
rm -f "$image" "dist/$zsync" "$zsync"
update="gh-releases-zsync|hobbo91|Ferriteweazle|latest|Ferriteweazle-*-$arch.AppImage.zsync"
ARCH=$arch "$unpacked/AppRun" --no-appstream --runtime-file "$cache/$runtime" -u "$update" \
    ${SIGN_KEY:+--sign --sign-key "$SIGN_KEY"} "$appdir" "$image"
# appimagetool leaves the zsync file in the folder it runs in.
mv "$zsync" dist/

# The signature, as AppImageUpdate checks it: SIGN_KEY's, of the SHA-256 of the
# AppImage with its signature and key sections zeroed, and SIGN_KEY carried.
section() {
    readelf -S --wide "$image" | sed -n 's/^ *\[ *[0-9]*\] *//p' |
        awk -v name="$1" '$1 == name { print $4, $5 }'
}
if [ -n "${SIGN_KEY:-}" ]; then
    check=target/signature-$arch
    rm -rf "$check"
    mkdir -p "$check"
    cp "$image" "$check/zeroed"
    for part in .sha256_sig:signature .sig_key:key; do
        set -- $(section "${part%:*}")
        at=$((0x$1)) size=$((0x$2))
        dd if=/dev/zero of="$check/zeroed" bs=1 seek="$at" count="$size" conv=notrunc 2>/dev/null
        dd if="$image" bs=1 skip="$at" count="$size" 2>/dev/null | tr -d '\000' >"$check/${part#*:}.asc"
    done
    sha256sum "$check/zeroed" | cut -d' ' -f1 | tr -d '\n' >"$check/digest"
    gpg --batch --status-fd 1 --verify "$check/signature.asc" "$check/digest" 2>/dev/null |
        grep -q "^\[GNUPG:\] VALIDSIG $SIGN_KEY " || { echo "linux: $image is not $SIGN_KEY's" >&2; exit 1; }
    gpg --batch --with-colons --show-keys "$check/key.asc" | grep -q "^fpr:*$SIGN_KEY:" ||
        { echo "linux: $image does not carry $SIGN_KEY" >&2; exit 1; }
    rm -rf "$check"
fi
du -sh "$tarball" "$image" "dist/$zsync"

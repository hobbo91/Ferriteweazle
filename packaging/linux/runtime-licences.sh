#!/bin/sh
# Writes packaging/licences/appimage-runtime-RUNTIME.txt, which an AppImage's
# THIRD-PARTY-NOTICES.txt ends with: the runtime's licence and those of what
# it links. Run it after changing packaging/linux/runtime.sh;
# packaging/linux/appimage.sha256 must list the downloads.
set -eu
cd "$(dirname "$0")/../.."
. packaging/linux/runtime.sh
cache=target/appimage-cache
fetch "type2-runtime-$RUNTIME_COMMIT.tar.gz" \
    "https://github.com/AppImage/type2-runtime/archive/$RUNTIME_COMMIT.tar.gz"
fetch "fuse-$FUSE.tar.xz" \
    "https://github.com/libfuse/libfuse/releases/download/fuse-$FUSE/fuse-$FUSE.tar.xz"
fetch "squashfuse-$SQUASHFUSE.tar.gz" "https://github.com/vasi/squashfuse/archive/$SQUASHFUSE.tar.gz"
fetch "musl-$MUSL-COPYRIGHT" "https://git.musl-libc.org/cgit/musl/plain/COPYRIGHT?h=v$MUSL"
fetch "zstd-$ZSTD-LICENSE" "https://raw.githubusercontent.com/facebook/zstd/v$ZSTD/LICENSE"
fetch "zlib-$ZLIB-LICENSE" "https://raw.githubusercontent.com/madler/zlib/v$ZLIB/LICENSE"
fetch "mimalloc-$MIMALLOC-LICENSE" \
    "https://raw.githubusercontent.com/microsoft/mimalloc/v$MIMALLOC/LICENSE"

rule=----------------------------------------------------------------------
heading() {
    printf '\n%s\n%s\n%s\n\n' "$rule" "$1" "$rule"
}
out=packaging/licences/appimage-runtime-$RUNTIME.txt
rm -f packaging/licences/appimage-runtime-*.txt
{
    line=======================================================================
    printf '\n\n%s\n%s\n%s\n\n' "$line" "The AppImage runtime" "$line"
    fold -s -w 72 <<EOF | sed 's/ *$//'
The AppImage starts with the AppImage runtime, type2-runtime $RUNTIME (https://github.com/AppImage/type2-runtime, commit $RUNTIME_COMMIT), which mounts the rest of the file and runs the program. It is linked statically with libfuse $FUSE, whose library is under the GNU Lesser General Public License 2.1, squashfuse $SQUASHFUSE, and Alpine Linux 3.21's zstd $ZSTD, zlib $ZLIB, mimalloc $MIMALLOC and musl $MUSL, under the licences below. The source of the runtime, libfuse and squashfuse is in LGPL-sources-@VERSION@.tar, published with this AppImage.
EOF
    heading "type2-runtime $RUNTIME"
    tar -xzOf "$cache/type2-runtime-$RUNTIME_COMMIT.tar.gz" "type2-runtime-$RUNTIME_COMMIT/LICENSE"
    heading "libfuse $FUSE (lib/ and include/, which the runtime links)"
    tar -xJOf "$cache/fuse-$FUSE.tar.xz" "fuse-$FUSE/LGPL2.txt"
    heading "squashfuse $SQUASHFUSE"
    tar -xzOf "$cache/squashfuse-$SQUASHFUSE.tar.gz" "squashfuse-$SQUASHFUSE/LICENSE"
    heading "zstd $ZSTD (under the BSD licence, one of its two)"
    cat "$cache/zstd-$ZSTD-LICENSE"
    heading "zlib $ZLIB"
    cat "$cache/zlib-$ZLIB-LICENSE"
    heading "mimalloc $MIMALLOC"
    cat "$cache/mimalloc-$MIMALLOC-LICENSE"
    heading "musl $MUSL"
    cat "$cache/musl-$MUSL-COPYRIGHT"
} >"$out"
echo "$out"

# Sourced by the Linux packaging scripts. The AppImage runtime bundle.sh puts
# at the front of an AppImage, the commit it was built from, and what that
# build links statically: its scripts/common/install-dependencies.sh builds
# libfuse and squashfuse; musl, zstd, zlib and mimalloc are Alpine 3.21's.
RUNTIME=20251108
RUNTIME_COMMIT=dd6cebedcbddde9c82f89b011e8e1d40b6e43868
FUSE=3.15.0
SQUASHFUSE=0.5.2
MUSL=1.2.5
ZSTD=1.5.6
ZLIB=1.3.2
MIMALLOC=2.1.7

# Downloads NAME from URL into target/appimage-cache once, and checks it
# against packaging/linux/appimage.sha256.
fetch() {
    mkdir -p target/appimage-cache
    if [ ! -f "target/appimage-cache/$1" ]; then
        curl -fL --retry 3 -o "target/appimage-cache/$1.part" "$2"
        mv "target/appimage-cache/$1.part" "target/appimage-cache/$1"
    fi
    grep " $1\$" packaging/linux/appimage.sha256 |
        (cd target/appimage-cache && shasum -a 256 -c -) >/dev/null
}

# Sourced by the Linux packaging scripts, after runtime.sh, whose fetch it
# uses. A package's lib/ holds what the program opens that a GTK-only desktop
# can lack: libxkbcommon-x11, for X11, with libxcb-xkb, which it needs; and
# libxkbcommon, which must be libxkbcommon-x11's release and reads Wayland
# compositors' keymaps, so is newer than most systems'. The rest (libX11,
# libxcb, libXcursor, libXi, Wayland, EGL, Vulkan) is the system's.
# libxkbcommon is built with zig for glibc 2.17, as the program is, against
# CentOS 7.9's libxcb, whose libxcb-xkb build is taken as it is.
XKBCOMMON=1.13.2
CENTOS=7.9.2009
LIBRARIES="libxkbcommon.so.0 libxkbcommon-x11.so.0 libxcb-xkb.so.1"

# Fetches CentOS's PACKAGE for ARCH (x86_64 or aarch64) and unpacks it into DIR.
centos_package() {
    rpm=$1.el7.$2.rpm
    case "$2" in
        x86_64) fetch "$rpm" "https://vault.centos.org/$CENTOS/os/x86_64/Packages/$rpm" ;;
        aarch64) fetch "$rpm" "https://vault.centos.org/altarch/$CENTOS/os/aarch64/Packages/$rpm" ;;
    esac
    mkdir -p "$3"
    bsdtar -xf "target/appimage-cache/$rpm" -C "$3"
}

# Fetches libxkbcommon's source and unpacks it into DIR.
xkbcommon_source() {
    fetch "libxkbcommon-$XKBCOMMON.tar.gz" \
        "https://github.com/xkbcommon/libxkbcommon/archive/refs/tags/xkbcommon-$XKBCOMMON.tar.gz"
    rm -rf "$1"
    mkdir -p "$1"
    tar -xzf "target/appimage-cache/libxkbcommon-$XKBCOMMON.tar.gz" -C "$1" --strip-components=1
}

# Builds libxkbcommon and libxkbcommon-x11 for ARCH into target/xkbcommon-ARCH,
# once for each release. Needs meson, ninja and bison.
xkbcommon() {
    out=target/xkbcommon-$XKBCOMMON-$1
    [ ! -f "$out/libxkbcommon-x11.so.0" ] || return 0
    work=target/xkbcommon-build-$1
    rm -rf "$work" "$out"
    centos_package libxcb-1.13-1 "$1" "$work/sysroot"
    centos_package libxcb-devel-1.13-1 "$1" "$work/sysroot"
    # In place of libxcb's own, which name libXau's, unneeded here.
    mkdir -p "$work/pkgconfig"
    for lib in xcb xcb-xkb; do
        printf 'Name: %s\nDescription: %s\nVersion: 1.13\nLibs: -L%s -l%s\nCflags: -I%s\n' \
            "$lib" "$lib" "$PWD/$work/sysroot/usr/lib64" "$lib" "$PWD/$work/sysroot/usr/include" \
            >"$work/pkgconfig/$lib.pc"
    done
    cat >"$work/cross.ini" <<EOF
[binaries]
c = ['zig', 'cc', '-target', '$1-linux-gnu.2.17']
ar = ['zig', 'ar']
pkg-config = 'pkg-config'

[host_machine]
system = 'linux'
cpu_family = '$1'
cpu = '$1'
endian = 'little'

[built-in options]
# Without the debug information zig gives it.
c_link_args = ['-s']
EOF
    xkbcommon_source "$work/source"
    # A prefix of /usr finds the system's XKB data and Compose files.
    PKG_CONFIG_LIBDIR=$PWD/$work/pkgconfig meson setup "$work/build" "$work/source" \
        --cross-file "$work/cross.ini" --prefix=/usr --buildtype=release \
        -Denable-x11=true -Denable-wayland=false -Denable-xkbregistry=false \
        -Denable-tools=false -Denable-docs=false -Denable-bash-completion=false
    meson compile -C "$work/build" xkbcommon xkbcommon-x11
    mkdir -p "$out"
    cp -L "$work/build/libxkbcommon.so.0" "$work/build/libxkbcommon-x11.so.0" "$out/"
}

# Puts LIBRARIES for ARCH in DIR, each finding the others beside it.
libraries() {
    xkbcommon "$2"
    tree=target/libraries-$2
    rm -rf "$tree"
    centos_package libxcb-1.13-1 "$2" "$tree"
    mkdir -p "$1"
    cp "target/xkbcommon-$XKBCOMMON-$2/libxkbcommon.so.0" \
        "target/xkbcommon-$XKBCOMMON-$2/libxkbcommon-x11.so.0" "$1/"
    cp -L "$tree/usr/lib64/libxcb-xkb.so.1" "$1/"
    for library in $LIBRARIES; do
        # In place of CentOS's /usr/lib64, and none of libxkbcommon's.
        patchelf --set-rpath '$ORIGIN' "$1/$library"
        chmod 755 "$1/$library"
    done
}

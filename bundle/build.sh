#!/bin/sh
# Builds the Greaseweazle Tools bundle the app ships, a standalone Python with gw, for
# this computer or TRIPLE: gw's newest release, or the tag GREASEWEAZLE names
# (bundle/versions). Another processor's Python runs emulated (Rosetta, Windows
# on ARM, qemu) so pip builds gw's C code for it. Downloads Python, gw's
# dependencies and, on Linux, gw's udev rule; needs curl, git and a C compiler
# (zig on Linux, unless CC and LDSHARED name another).
#
#   bundle/build.sh                                          # this computer
#   bundle/build.sh x86_64-pc-windows-msvc                   # another triple
#   GREASEWEAZLE=v1.22 bundle/build.sh                       # a given release
#   GREASEWEAZLE_SOURCE=~/src/greaseweazle bundle/build.sh   # a local clone
set -eu
cd "$(dirname "$0")/.."
. bundle/greaseweazle.sh
triple=${1:-$(host)}
dest=$(bundle_dir "$triple")
tag=$(wanted)
echo "bundle: building Greaseweazle $tag for $triple in $dest"

case "$triple" in
    aarch64-apple-darwin | x86_64-apple-darwin | aarch64-unknown-linux-gnu | \
        x86_64-unknown-linux-gnu | aarch64-pc-windows-msvc | x86_64-pc-windows-msvc) ;;
    *) echo "bundle: no Python build for $triple" >&2; exit 1 ;;
esac

name="cpython-$PYTHON+$PYTHON_RELEASE-$triple-install_only_stripped.tar.gz"
url="https://github.com/astral-sh/python-build-standalone/releases/download/$PYTHON_RELEASE/$(echo "$name" | sed 's/+/%2B/')"
cache=target/bundle-cache
mkdir -p "$cache"
if [ ! -f "$cache/$name" ]; then
    curl -fL --retry 3 -o "$cache/$name.part" "$url"
    mv "$cache/$name.part" "$cache/$name"
fi
grep " $name\$" bundle/python.sha256 | (cd "$cache" && shasum -a 256 -c -)

rm -rf "$dest"
mkdir -p "$dest"
tar -xzf "$cache/$name" -C "$dest" --strip-components=1
case "$triple" in
    *windows*) py=$dest/python.exe lib=$dest/Lib ;;
    *) py=$dest/bin/python3 lib=$(echo "$dest"/lib/python3.*) ;;
esac

# On Linux gw's C code, like the program, needs no newer glibc than 2.17.
case "$triple" in *linux*)
    export CC="${CC:-zig cc -target ${triple%%-*}-linux-gnu.2.17}"
    export LDSHARED="${LDSHARED:-$CC -shared}" ;;
esac
case "$source" in /*) source="file://$source" ;; esac
"$py" -m pip install --quiet --no-cache-dir --disable-pip-version-check \
    --no-warn-script-location "git+$source@$tag"
"$py" -m pip uninstall --quiet --yes pip

# Drop what gw never uses and what only the build needed. On Linux and macOS:
# libpython (linked into the interpreter), launchers whose #! names this folder,
# and _dbm (on Linux it holds Berkeley DB, whose licence wants source offered).
rm -rf "$lib/test" "$lib/idlelib" "$lib/tkinter" "$lib/turtledemo" "$lib/ensurepip" \
    "$lib/pydoc_data"
case "$triple" in
    *windows*)
        rm -rf "$dest/include" "$dest/libs" "$dest/Scripts" "$dest/tcl" \
            "$dest"/DLLs/_tkinter* "$dest"/DLLs/tcl* "$dest"/DLLs/tk*
        ;;
    *)
        find "$dest/bin" -type f ! -name 'python3.*[0-9]' -delete
        find "$dest/bin" -type l ! -name python ! -name python3 -delete
        rm -rf "$dest/include" "$dest/share" "$dest"/lib/libpython* "$dest"/lib/libtcl* \
            "$dest"/lib/libtk* "$dest"/lib/tcl* "$dest"/lib/tk* "$dest"/lib/itcl* \
            "$dest"/lib/thread* "$dest/lib/pkgconfig" "$lib"/config-* \
            "$lib/lib-dynload/_tkinter"* "$lib/lib-dynload/_dbm"*
        ;;
esac
# The bundle never changes once built, so its bytecode is not checked against
# the sources' file times, which copies, zips and installers do not all keep.
"$py" -m compileall -q -f --invalidation-mode unchecked-hash "$lib"

# gw cannot run without its C extension, so it must load too.
"$py" -c 'import greaseweazle.optimised.optimised, sys; print("bundle: greaseweazle", greaseweazle.__version__, "on Python", sys.version.split()[0])'

# gw's udev rule, which the app offers when Linux refuses it the port.
case "$triple" in *linux*)
    rule=scripts/49-greaseweazle.rules
    if [ -n "${GREASEWEAZLE_SOURCE:-}" ]; then
        git -C "$GREASEWEAZLE_SOURCE" show "$tag:$rule" >"$dest/49-greaseweazle.rules"
    else
        curl -fsSL --retry 3 -o "$dest/49-greaseweazle.rules" \
            "https://raw.githubusercontent.com/keirf/greaseweazle/$tag/$rule"
    fi ;;
esac
bundle/caps.sh "$triple" "$dest"
"$py" -I -c 'import ctypes, sys, os
name = {"darwin": "libcapsimage.dylib", "win32": "CAPSImg.dll"}.get(sys.platform, "libcapsimage.so.5")
assert ctypes.cdll.LoadLibrary(os.path.join(sys.prefix, "caps", name)).CAPSInit() == 0'

# greaseweazle-version goes last: it marks a finished build.
echo "$PYTHON+$PYTHON_RELEASE" >"$dest/python-version"
echo "$CAPS_COMMIT" >"$dest/caps-version"
echo "$tag" >"$dest/greaseweazle-version"
du -sh "$dest"

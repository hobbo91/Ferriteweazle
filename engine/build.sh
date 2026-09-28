#!/bin/sh
# Builds the engine the app ships, a standalone Python with Greaseweazle, for
# this computer or for TRIPLE: gw's newest release, or the tag GREASEWEAZLE
# names (engine/versions). Another processor's Python runs emulated (Rosetta,
# Windows on ARM, qemu) so pip builds gw's C code for it; on Linux set CC and
# LDSHARED to a compiler for that processor. Downloads Python and gw's pip
# dependencies, and on Linux gw's udev rule; needs curl, git and a C compiler.
#
#   engine/build.sh                                          # this computer
#   engine/build.sh x86_64-pc-windows-msvc                   # another triple
#   GREASEWEAZLE=v1.22 engine/build.sh                       # a given release
#   GREASEWEAZLE_SOURCE=~/src/greaseweazle engine/build.sh   # a local clone
set -eu
cd "$(dirname "$0")/.."
. engine/greaseweazle.sh
triple=${1:-$(host)}
dest=$(engine_dir "$triple")
tag=$(wanted)
echo "engine: building Greaseweazle $tag for $triple in $dest"

case "$triple" in
    aarch64-apple-darwin | x86_64-apple-darwin | aarch64-unknown-linux-gnu | \
        x86_64-unknown-linux-gnu | aarch64-pc-windows-msvc | x86_64-pc-windows-msvc) ;;
    *) echo "engine: no Python build for $triple" >&2; exit 1 ;;
esac

name="cpython-$PYTHON+$PYTHON_RELEASE-$triple-install_only_stripped.tar.gz"
url="https://github.com/astral-sh/python-build-standalone/releases/download/$PYTHON_RELEASE/$(echo "$name" | sed 's/+/%2B/')"
cache=target/engine-cache
mkdir -p "$cache"
if [ ! -f "$cache/$name" ]; then
    curl -fL --retry 3 -o "$cache/$name.part" "$url"
    mv "$cache/$name.part" "$cache/$name"
fi
grep " $name\$" engine/python.sha256 | (cd "$cache" && shasum -a 256 -c -)

rm -rf "$dest"
mkdir -p "$dest"
tar -xzf "$cache/$name" -C "$dest" --strip-components=1
case "$triple" in
    *windows*) py=$dest/python.exe lib=$dest/Lib ;;
    *) py=$dest/bin/python3 lib=$(echo "$dest"/lib/python3.*) ;;
esac

case "$source" in /*) source="file://$source" ;; esac
"$py" -m pip install --quiet --no-cache-dir --disable-pip-version-check \
    --no-warn-script-location "git+$source@$tag"
"$py" -m pip uninstall --quiet --yes pip

# Drop what gw never uses, and what only building needed. On Linux and macOS
# the interpreter is linked statically, so libpython goes too, as do
# launchers whose #! line names this build folder.
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
            "$lib/lib-dynload/_tkinter"*
        ;;
esac
"$py" -m compileall -q "$lib/site-packages"

"$py" -c 'import greaseweazle, sys; print("engine: greaseweazle", greaseweazle.__version__, "on Python", sys.version.split()[0])'

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
echo "$tag" >"$dest/greaseweazle-version"
du -sh "$dest"

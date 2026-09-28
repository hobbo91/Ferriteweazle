#!/bin/sh
# Builds target/engine, the standalone Python with Greaseweazle that the app
# ships: gw's newest release, or the tag GREASEWEAZLE names (engine/versions).
# Downloads Python and gw's pip dependencies; needs curl, git and a C
# compiler.
#
#   engine/build.sh
#   GREASEWEAZLE=v1.22 engine/build.sh                       # a given release
#   GREASEWEAZLE_SOURCE=~/src/greaseweazle engine/build.sh   # a local clone
set -eu
cd "$(dirname "$0")/.."
. engine/greaseweazle.sh
tag=$(wanted)
echo "engine: building Greaseweazle $tag"

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) triple=aarch64-apple-darwin ;;
    Darwin-x86_64) triple=x86_64-apple-darwin ;;
    Linux-aarch64) triple=aarch64-unknown-linux-gnu ;;
    Linux-x86_64) triple=x86_64-unknown-linux-gnu ;;
    *) echo "engine: no Python build for $(uname -s) $(uname -m)" >&2; exit 1 ;;
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

rm -rf target/engine
mkdir -p target/engine
tar -xzf "$cache/$name" -C target/engine --strip-components=1
py=target/engine/bin/python3

case "$source" in /*) source="file://$source" ;; esac
"$py" -m pip install --quiet --no-cache-dir --disable-pip-version-check "git+$source@$tag"
"$py" -m pip uninstall --quiet --yes pip

# Drop what gw never uses, libpython (the interpreter is linked statically)
# and launchers whose #! line names this build folder.
lib=$(echo target/engine/lib/python3.*)
find target/engine/bin -type f ! -name 'python3.*[0-9]' -delete
find target/engine/bin -type l ! -name python ! -name python3 -delete
rm -rf target/engine/include target/engine/share target/engine/lib/libpython* \
    target/engine/lib/libtcl* target/engine/lib/libtk* target/engine/lib/tcl* target/engine/lib/tk* \
    target/engine/lib/itcl* target/engine/lib/thread* target/engine/lib/pkgconfig \
    "$lib"/config-* "$lib/lib-dynload/_tkinter"* \
    "$lib/test" "$lib/idlelib" "$lib/tkinter" "$lib/turtledemo" "$lib/ensurepip" "$lib/pydoc_data"
"$py" -m compileall -q "$lib/site-packages"

"$py" -c 'import greaseweazle, sys; print("engine: greaseweazle", greaseweazle.__version__, "on Python", sys.version.split()[0])'
echo "$tag" >"$built"
du -sh target/engine

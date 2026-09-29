#!/bin/sh
# Writes packaging/licences/python-PYTHON+PYTHON_RELEASE.txt: CPython's licence,
# python-build-standalone's texts for the libraries it links (the same on every
# platform) but Tcl/Tk and X11 (tkinter), Berkeley DB (_dbm) and OpenSSL 1.1,
# which the bundle lacks, and zstd's, which that build leaves out. Run it after
# changing PYTHON or PYTHON_RELEASE in bundle/versions; bundle/python.sha256
# must list the downloads. Needs zstd.
set -eu
cd "$(dirname "$0")/.."
. bundle/versions
ZSTD=1.5.7
cache=target/bundle-cache
work=target/python-licences
mkdir -p "$cache"
rm -rf "$work"
mkdir -p "$work"

# Downloads NAME from URL into the cache once, and checks it.
fetch() {
    sum=$(grep " $1\$" bundle/python.sha256) ||
        { echo "licences: bundle/python.sha256 has no line for $1" >&2; exit 1; }
    if [ ! -f "$cache/$1" ]; then
        curl -fL --retry 3 -o "$cache/$1.part" "$2"
        mv "$cache/$1.part" "$cache/$1"
    fi
    echo "$sum" | (cd "$cache" && shasum -a 256 -c -)
}
full=cpython-$PYTHON+$PYTHON_RELEASE-aarch64-apple-darwin-pgo+lto-full.tar.zst
fetch "$full" "https://github.com/astral-sh/python-build-standalone/releases/download/$PYTHON_RELEASE/$(echo "$full" | sed 's/+/%2B/g')"
fetch "cpython-$PYTHON-license.rst" \
    "https://raw.githubusercontent.com/python/cpython/v$PYTHON/Doc/license.rst"
fetch "zstd-$ZSTD-LICENSE" "https://raw.githubusercontent.com/facebook/zstd/v$ZSTD/LICENSE"
zstd -dc "$cache/$full" | tar -xf - -C "$work" python/licenses
[ -f "$work/python/licenses/LICENSE.openssl-3.txt" ] ||
    { echo "licences: $full holds no python/licenses" >&2; exit 1; }

heading() {
    rule=----------------------------------------------------------------------
    printf '\n%s\n%s\n%s\n\n' "$rule" "$1" "$rule"
}
out=packaging/licences/python-$PYTHON+$PYTHON_RELEASE.txt
rm -f packaging/licences/python-*.txt
{
    heading "CPython $PYTHON"
    cat "$cache/cpython-$PYTHON-license.rst"
    for file in "$work"/python/licenses/LICENSE.*.txt; do
        name=${file##*/LICENSE.}
        name=${name%.txt}
        case "$name" in
            cpython | tcl | tix | libX11 | libXau | libxcb | bdb | openssl-1.1) continue ;;
        esac
        heading "$name"
        cat "$file"
    done
    if [ ! -f "$work/python/licenses/LICENSE.zstd.txt" ]; then
        heading zstd
        cat "$cache/zstd-$ZSTD-LICENSE"
    fi
} >"$out"
echo "licences: wrote $out"

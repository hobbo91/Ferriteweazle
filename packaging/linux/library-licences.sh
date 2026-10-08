#!/bin/sh
# Writes packaging/licences/linux-libraries.txt, which a Linux package's
# THIRD-PARTY-NOTICES.txt ends with: the licences of the libraries in its
# lib/. Run it after changing packaging/linux/libraries.sh;
# packaging/linux/appimage.sha256 must list the downloads.
set -eu
cd "$(dirname "$0")/../.."
. packaging/linux/runtime.sh
. packaging/linux/libraries.sh
xkbcommon_source target/library-licences
# libxcb's package holds no licence of its own.
fetch libxcb-1.13-COPYING "https://gitlab.freedesktop.org/xorg/lib/libxcb/-/raw/libxcb-1.13/COPYING"

rule=----------------------------------------------------------------------
heading() {
    printf '\n%s\n%s\n%s\n\n' "$rule" "$1" "$rule"
}
out=packaging/licences/linux-libraries.txt
{
    line=======================================================================
    printf '\n\n%s\n%s\n%s\n\n' "$line" "The libraries in lib/" "$line"
    fold -s -w 72 <<EOF | sed 's/ *$//'
lib/ holds libxkbcommon and libxkbcommon-x11 $XKBCOMMON, built from their source, and libxcb-xkb 1.13 (libxcb's), CentOS Linux $CENTOS's build, under the licences below.
EOF
    heading "libxkbcommon and libxkbcommon-x11 $XKBCOMMON"
    cat target/library-licences/LICENSE
    heading "libxcb-xkb 1.13"
    cat target/appimage-cache/libxcb-1.13-COPYING
} >"$out"
echo "$out"

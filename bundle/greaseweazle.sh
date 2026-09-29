# Which Greaseweazle release to build, and where each bundle goes. Sourced
# from the repository root by bundle/build.sh and the packaging scripts.

# A tag in the environment beats one in bundle/versions.
pin=${GREASEWEAZLE:-}
. bundle/versions
GREASEWEAZLE=${pin:-$GREASEWEAZLE}
source=${GREASEWEAZLE_SOURCE:-https://github.com/keirf/greaseweazle}

# This computer's target triple. On Windows on ARM, Git's shell runs
# emulated, so uname -m says x86_64; uname -s ends in -ARM64 all the same.
host() {
    sys=$(uname -s)
    case "$sys" in
        Darwin) os=apple-darwin ;;
        Linux) os=unknown-linux-gnu ;;
        MINGW* | MSYS* | CYGWIN*) os=pc-windows-msvc ;;
        *) os=unknown ;;
    esac
    case "$sys-$(uname -m)" in
        *-ARM64-* | *-arm64 | *-aarch64) echo "aarch64-$os" ;;
        *-x86_64 | *-amd64) echo "x86_64-$os" ;;
        *) echo "unknown-$os" ;;
    esac
}

# The bundle for TRIPLE: target/greaseweazle-bundle for this computer's own, which
# `cargo run` finds, else target/greaseweazle-bundle-TRIPLE.
bundle_dir() {
    if [ -z "${1:-}" ] || [ "$1" = "$(host)" ]; then
        echo target/greaseweazle-bundle
    else
        echo "target/greaseweazle-bundle-$1"
    fi
}

# The newest release among the tag names on stdin. "latest" is the nightly
# build, and a tag such as v1.24rc1 is not a release.
newest() {
    grep -E '^v[0-9]+(\.[0-9]+)+$' | sort -t. -k1.2,1n -k2,2n -k3,3n -k4,4n | tail -n 1
}

# The tag to build: GREASEWEAZLE, else upstream's latest release (GitHub leaves
# out drafts and prereleases), else the newest release tag, which is all a
# local clone or a fork has.
wanted() {
    if [ -n "$GREASEWEAZLE" ]; then
        echo "$GREASEWEAZLE"
        return
    fi
    tag=
    if [ -z "${GREASEWEAZLE_SOURCE:-}" ]; then
        tag=$(curl -fsSL --max-time 30 https://api.github.com/repos/keirf/greaseweazle/releases/latest |
            sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | newest)
    fi
    [ -n "$tag" ] || tag=$(git ls-remote --tags --refs "$source" | sed 's|.*refs/tags/||' | newest)
    if [ -z "$tag" ]; then
        echo "bundle: cannot find Greaseweazle's latest release; set GREASEWEAZLE to a tag" >&2
        return 1
    fi
    echo "$tag"
}

# Builds TRIPLE's bundle (this computer's if none) unless it holds the wanted
# release, Python and SPS/CAPS library. Offline, a bundle already built is kept.
refresh() {
    dir=$(bundle_dir "${1:-}")
    if tag=$(wanted); then
        [ "$(cat "$dir/greaseweazle-version" 2>/dev/null)" = "$tag" ] &&
            [ "$(cat "$dir/python-version" 2>/dev/null)" = "$PYTHON+$PYTHON_RELEASE" ] &&
            [ "$(cat "$dir/caps-version" 2>/dev/null)" = "$CAPS_COMMIT" ] ||
            GREASEWEAZLE=$tag bundle/build.sh ${1:+"$1"}
    else
        [ -f "$dir/greaseweazle-version" ] || return 1
        echo "bundle: keeping the Greaseweazle already built" >&2
    fi
}

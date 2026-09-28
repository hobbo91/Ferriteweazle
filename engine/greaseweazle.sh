# Which Greaseweazle release to build. Sourced from the repository root by
# engine/build.sh and packaging/macos/bundle.sh.

# A tag in the environment beats one in engine/versions.
pin=${GREASEWEAZLE:-}
. engine/versions
GREASEWEAZLE=${pin:-$GREASEWEAZLE}
source=${GREASEWEAZLE_SOURCE:-https://github.com/keirf/greaseweazle}
# Holds the tag target/engine was built from.
built=target/engine/greaseweazle-version

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
        echo "engine: cannot find Greaseweazle's latest release; set GREASEWEAZLE to a tag" >&2
        return 1
    fi
    echo "$tag"
}

# Builds target/engine again unless it holds the wanted release. Offline, an
# engine already built is kept.
refresh() {
    if tag=$(wanted); then
        [ "$(cat "$built" 2>/dev/null)" = "$tag" ] || GREASEWEAZLE=$tag engine/build.sh
    else
        [ -x target/engine/bin/python3 ] || return 1
        echo "engine: keeping the Greaseweazle already built" >&2
    fi
}

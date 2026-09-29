#!/bin/sh
# Writes to stdout the THIRD-PARTY-NOTICES.txt a package carries: the licences
# of the Rust crates built into the program for each TRIPLE, then of what the
# engine at ENGINE holds: Python and the libraries linked into it, gw and its
# Python packages, and the SPS/CAPS library. packaging/licences has the texts
# their sources leave out.
#
#   packaging/notices.sh ENGINE TRIPLE...
set -eu
cd "$(dirname "$0")/.."
engine=$1
shift
texts=packaging/licences
registry=${CARGO_HOME:-$HOME/.cargo}/registry/src
work=target/notices
rm -rf "$work"
mkdir -p "$work"
export LC_ALL=C

section() {
    line=======================================================================
    printf '\n\n%s\n%s\n%s\n' "$line" "$1" "$line"
}
heading() {
    line=----------------------------------------------------------------------
    printf '\n%s\n%s\n%s\n\n' "$line" "$(printf '%s\n' "$1" | fold -s -w 70 | sed 's/ *$//')" "$line"
}
# A crate's licence files: any beside its Cargo.toml, and below it any but
# its tests' and examples', with the texts that come with its fonts.
found() {
    find "$1" -type f \( -iname '*licen[cs]e*' -o -iname 'copying*' -o -iname 'notice*' \
        -o -iname 'copyright*' -o -path '*/fonts/*.txt' \) ! -name '*.rs' \
        ! -path '*/tests/*' ! -path '*/examples/*' ! -path '*/benches/*' | sort
}

echo "Ferriteweazle is MIT licensed (LICENSE.txt). It includes the software"
echo "below, each under the licence given with it."

# Each licence file is indexed by its checksum, so that each text is printed
# once, under all the crates that carry it.
targets=
for triple; do targets="$targets --target $triple"; done
cargo tree --locked -e normal --prefix none --format '{p}|{l}' $targets >"$work/tree"
sed -e 's/ (\*)$//' -e 's/ (proc-macro)|/|/' -e '/^ferriteweazle /d' -e '/^$/d' "$work/tree" |
    sort -u >"$work/crates"
: >"$work/index"
: >"$work/bare"
while IFS='|' read -r crate expr; do
    name=${crate% v*}
    version=${crate##* v}
    for dir in "$registry"/*/"$name-$version"; do break; done
    if [ ! -d "$dir" ]; then
        echo "notices: $name $version is not in $registry" >&2
        exit 1
    fi
    found "$dir" >"$work/files"
    # Offered a choice of licences, the program takes one that is no GPL.
    case "$expr" in
        *" OR "* | */*)
            grep -v -i '/[^/]*gpl[^/]*$' "$work/files" >"$work/chosen" || true
            [ ! -s "$work/chosen" ] || mv "$work/chosen" "$work/files"
            ;;
    esac
    own=
    while read -r file; do
        case ${file#"$dir"/} in */*) ;; *) own=1 ;; esac
        echo "$(cksum <"$file")|$name $version|$file" >>"$work/index"
    done <"$work/files"
    if [ -z "$own" ]; then
        deeper=no
        [ ! -s "$work/files" ] || deeper=yes
        echo "$name $version|$expr|$deeper" >>"$work/bare"
    fi
done <"$work/crates"
# A line per text: its first file, and its crates in lines of 70 columns at
# most, a tab between lines.
sort -t'|' -k2,2 "$work/index" | awk -F'|' '
    !($1 in file) { file[$1] = $3; order[++n] = $1 }
    !(($1, $2) in seen) {
        seen[$1, $2]
        if (crates[$1] == "") { crates[$1] = $2; width[$1] = length($2) }
        else if (width[$1] + 2 + length($2) > 70) {
            crates[$1] = crates[$1] ",\t" $2; width[$1] = length($2)
        } else { crates[$1] = crates[$1] ", " $2; width[$1] += 2 + length($2) }
    }
    END { for (i = 1; i <= n; i++) print file[order[i]] "|" crates[order[i]] }' >"$work/groups"

section "Rust crates built into the program"
while IFS='|' read -r file crates; do
    heading "$(printf '%s\n' "$crates" | tr '\t' '\n')"
    tr -d '\r' <"$file"
done <"$work/groups"

# Crates with no licence file beside their Cargo.toml go under the standard
# texts of the licences they name. One with files deeper down, such as its
# fonts', may name licences those files give.
if [ -s "$work/bare" ]; then
    heading "Crates with no licence file, under the standard texts below"
    : >"$work/ids"
    while IFS='|' read -r crate expr deeper; do
        echo "$crate: $expr"
        for id in $(echo "$expr" | tr '()/' '   '); do
            case "$id" in OR | AND | WITH | *-exception) continue ;; esac
            if [ -f "$texts/$id.txt" ]; then
                echo "$id" >>"$work/ids"
            elif [ "$deeper" = no ]; then
                echo "notices: $crate names $id and has no licence file; add $texts/$id.txt" >&2
                exit 1
            fi
        done
    done <"$work/bare"
    for id in $(sort -u "$work/ids"); do
        heading "$id"
        cat "$texts/$id.txt"
    done
fi

if [ ! -f "$engine/python-version" ]; then
    echo "notices: $engine has no python-version; rebuild it with engine/build.sh" >&2
    exit 1
fi
python=$texts/python-$(cat "$engine/python-version").txt
if [ ! -f "$python" ]; then
    echo "notices: no $python; run packaging/python-licences.sh" >&2
    exit 1
fi
section "Python and the libraries linked into it"
cat "$python"

# A package's licence files are in its .dist-info folder, but pyserial 3.5's
# wheel has none.
section "Greaseweazle and its Python packages"
packages=0
for info in "$engine"/lib/python3.*/site-packages/*.dist-info \
    "$engine"/Lib/site-packages/*.dist-info; do
    [ -d "$info" ] || continue
    package=${info##*/}
    package=${package%.dist-info}
    name=${package%%-*}
    find "$info" -type f \( -path '*/licenses/*' -o -iname 'licen[cs]e*' -o -iname 'copying*' \
        -o -iname 'notice*' \) | sort >"$work/files"
    if [ ! -s "$work/files" ]; then
        if [ ! -f "$texts/$name.txt" ]; then
            echo "notices: $package has no licence file; add $texts/$name.txt" >&2
            exit 1
        fi
        echo "$texts/$name.txt" >"$work/files"
    fi
    heading "$name ${package#*-}"
    while read -r file; do
        tr -d '\r' <"$file"
        echo
    done <"$work/files"
    packages=$((packages + 1))
done
if [ "$packages" -eq 0 ]; then
    echo "notices: $engine holds no Python packages" >&2
    exit 1
fi

if [ ! -f "$engine/caps-version" ]; then
    echo "notices: $engine has no SPS/CAPS library; rebuild it with engine/build.sh" >&2
    exit 1
fi
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
section "The SPS Decoder Library"
echo
fold -s -w 72 <<EOF | sed 's/ *$//'
gw reads IPF and CT Raw images with the SPS Decoder Library (CAPSImage 5.1, https://github.com/simonowen/capsimage, commit $(cat "$engine/caps-version")), which is free for non-commercial use only. Its source is in Ferriteweazle-$version-capsimage-source.tar.gz, published with this package.
EOF
echo
cat "$engine/caps/LICENCE.txt"

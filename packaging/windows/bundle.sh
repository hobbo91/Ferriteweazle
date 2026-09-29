#!/bin/sh
# Builds dist/Ferriteweazle-VERSION-win-ARCH.zip, a folder that runs where it is
# unzipped, and the .msi, which installs it for all users; ARCH is x64 or arm64,
# this PC's if none. Both need Windows 10 or newer; the engine is rebuilt first if
# gw has a newer release. Runs in Git Bash; needs Visual Studio's C++ build tools
# and WiX 5 (a .NET tool: install the .NET 8 SDK first) with its UI extension:
#   dotnet tool install --global wix --version 5.0.2
#   wix extension add --global WixToolset.UI.wixext/5.0.2
# TODO: code signing.
set -eu
cd "$(dirname "$0")/../.."
. engine/greaseweazle.sh
case "${1:-$(host)}" in
    x64 | x86_64-pc-windows-msvc) arch=x64 triple=x86_64-pc-windows-msvc ;;
    arm64 | aarch64-pc-windows-msvc) arch=arm64 triple=aarch64-pc-windows-msvc ;;
    *) echo "bundle: ARCH is x64 or arm64" >&2; exit 1 ;;
esac
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
stage=target/windows-$arch
app=$stage/Ferriteweazle
rm -rf "$stage"
mkdir -p "$app" dist

refresh "$triple"
cargo build --release --locked --target "$triple"
cp "target/$triple/release/ferriteweazle.exe" "$app/Ferriteweazle.exe"
cp -a "$(engine_dir "$triple")" "$app/ferriteweazle-data"
packaging/notices.sh "$app/ferriteweazle-data" "$triple" >"$stage/notices.txt"
# CRLF, for Notepad before Windows 10 1809.
sed 's/\r*$/\r/' "$stage/notices.txt" >"$app/THIRD-PARTY-NOTICES.txt"
sed 's/\r*$/\r/' LICENSE >"$app/LICENSE.txt"
sed 's/\r*$/\r/' packaging/windows/README.txt >"$app/README.txt"

# Git's tar cannot write a zip; Windows' own can.
zip=dist/Ferriteweazle-$version-win-$arch.zip
rm -f "$zip"
"$(cygpath -S)/tar.exe" -a -cf "$zip" -C "$stage" Ferriteweazle

# The installer's first page shows the licence, as RTF: a paragraph for each
# block of lines.
awk 'BEGIN { printf "{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Segoe UI;}}\\f0\\fs18 " }
    { sub(/\r$/, "") } NF { printf "%s ", $0; next } { printf "\\par\\par " }
    END { print "}" }' LICENSE >"$stage/LICENSE.rtf"
: >"$stage/msi"
msi=dist/Ferriteweazle-$version-win-$arch.msi
wix build -arch "$arch" -ext WixToolset.UI.wixext -d Version="$version" \
    -d App="$(cygpath -w "$PWD/$app")" -d Licence="$(cygpath -w "$PWD/$stage/LICENSE.rtf")" \
    -d Marker="$(cygpath -w "$PWD/$stage/msi")" -pdbtype none -o "$msi" \
    packaging/windows/ferriteweazle.wxs
du -sh "$zip" "$msi"

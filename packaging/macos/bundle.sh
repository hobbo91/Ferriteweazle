#!/bin/sh
# Builds target/Ferriteweazle.app with the engine inside, and a zip of it.
# Signed ad hoc, so it runs on the Mac that built it.
# TODO: Developer ID signing and notarisation.
set -eu
cd "$(dirname "$0")/../.."
[ -x target/engine/bin/python3 ] || engine/build.sh
cargo build --release

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
app=target/Ferriteweazle.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/ferriteweazle "$app/Contents/MacOS/"
ditto target/engine "$app/Contents/Resources/engine"
cp packaging/macos/AppIcon.icns "$app/Contents/Resources/"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist >"$app/Contents/Info.plist"
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

zip="target/Ferriteweazle-$version-macos-$(uname -m).zip"
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
du -sh "$app" "$zip"

#!/bin/sh
# Builds dist/Ferriteweazle-VERSION-macos-universal.dmg: one app for Apple
# Silicon and Intel Macs, macOS 10.15 on (the oldest the Intel Python runs
# on), signed ad hoc, each bundle rebuilt first if gw has a newer release.
# Needs rustup's stable toolchain with both Mac targets, and Rosetta.
# TODO: Developer ID signing and notarisation.
set -eu
cd "$(dirname "$0")/../.."
. bundle/greaseweazle.sh
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
stage=target/macos
app=$stage/Ferriteweazle.app
rm -rf "$stage"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" dist

# rustup's stable toolchain, as Homebrew's Rust has no x86_64 standard library.
# Run this way its rust-objcopy cannot find libLLVM, so Apple's strip runs after lipo.
rustc=$(rustup which --toolchain stable rustc)
for triple in aarch64-apple-darwin x86_64-apple-darwin; do
    refresh "$triple"
    RUSTC=$rustc MACOSX_DEPLOYMENT_TARGET=10.15 CARGO_PROFILE_RELEASE_STRIP=none \
        cargo build --release --locked --target "$triple"
done
lipo -create -output "$app/Contents/MacOS/ferriteweazle" \
    target/aarch64-apple-darwin/release/ferriteweazle target/x86_64-apple-darwin/release/ferriteweazle
strip "$app/Contents/MacOS/ferriteweazle"

# One bundle for both: the Apple Silicon one, each program joined with its Intel
# twin (the rest differs only in build notes and bytecode) and signed again, as
# Apple Silicon runs no unsigned code. pip resolves each bundle's packages afresh.
arm=$(bundle_dir aarch64-apple-darwin)
intel=$(bundle_dir x86_64-apple-darwin)
versions() {
    cat "$1/python-version"
    "$1/bin/python3" -B -I -c 'import importlib.metadata as m
print(sorted(d.name + " " + d.version for d in m.distributions()))'
}
[ "$(versions "$arm")" = "$(versions "$intel")" ] || {
    echo "bundle: the two bundles differ; rebuild each with bundle/build.sh TRIPLE" >&2
    exit 1
}
bundle=$app/Contents/Resources/greaseweazle
ditto "$arm" "$bundle"
find "$bundle" -type f | while read -r f; do
    file -b "$f" | grep -q Mach-O || continue
    case "$(lipo -archs "$f")" in *x86_64*) continue ;; esac
    lipo -create -output "$f.both" "$f" "$intel/${f#"$bundle"/}"
    mv "$f.both" "$f"
    codesign --force --sign - "$f"
done

cp packaging/macos/AppIcon.icns "$app/Contents/Resources/"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist >"$app/Contents/Info.plist"
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

# The app beside a link to Applications, to drag it across, how to open it
# the first time, and the licences.
ln -s /Applications "$stage/Applications"
cp packaging/macos/README.txt "$stage/README.txt"
cp LICENSE "$stage/LICENSE.txt"
packaging/notices.sh "$bundle" aarch64-apple-darwin x86_64-apple-darwin \
    >"$stage/THIRD-PARTY-NOTICES.txt"
# The disk shows the logo: made writable, given Finder's custom-icon flag,
# then compressed.
cp packaging/macos/AppIcon.icns "$stage/.VolumeIcon.icns"
dmg=dist/Ferriteweazle-$version-macos-universal.dmg
rw=target/macos-rw.dmg
volume=target/macos-volume
rm -f "$dmg" "$rw"
hdiutil create -quiet -volname "Ferriteweazle $version" -srcfolder "$stage" -fs HFS+ \
    -format UDRW "$rw"
mkdir -p "$volume"
hdiutil attach -quiet -nobrowse -noautoopen -mountpoint "$volume" "$rw"
xattr -wx com.apple.FinderInfo \
    0000000000000000040000000000000000000000000000000000000000000000 "$volume"
hdiutil detach -quiet "$volume"
rmdir "$volume"
hdiutil convert -quiet "$rw" -format UDZO -o "$dmg"
rm -f "$rw"
du -sh "$dmg"

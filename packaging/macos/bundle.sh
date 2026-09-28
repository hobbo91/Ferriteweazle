#!/bin/sh
# Builds dist/Ferriteweazle-VERSION-macos-universal.dmg: one app for Apple
# Silicon and Intel Macs, macOS 10.15 on (the oldest the Intel Python runs
# on), signed ad hoc, each engine rebuilt first if gw has a newer release.
# Needs rustup's stable toolchain with both Mac targets, and Rosetta.
# TODO: Developer ID signing and notarisation.
set -eu
cd "$(dirname "$0")/../.."
. engine/greaseweazle.sh
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
stage=target/macos
app=$stage/Ferriteweazle.app
rm -rf "$stage"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" dist

# Homebrew's Rust has no x86_64 library; rustup's stable toolchain has both.
# Its rust-objcopy cannot find its LLVM library run this way, so Apple's
# strip does the stripping, once the two are joined.
rustc=$(rustup which --toolchain stable rustc)
for triple in aarch64-apple-darwin x86_64-apple-darwin; do
    refresh "$triple"
    RUSTC=$rustc MACOSX_DEPLOYMENT_TARGET=10.15 CARGO_PROFILE_RELEASE_STRIP=none \
        cargo build --release --locked --target "$triple"
done
lipo -create -output "$app/Contents/MacOS/ferriteweazle" \
    target/aarch64-apple-darwin/release/ferriteweazle target/x86_64-apple-darwin/release/ferriteweazle
strip "$app/Contents/MacOS/ferriteweazle"

# One engine for both: the Apple Silicon one, each program in it joined with
# its Intel twin (the rest differs only in build notes and cached bytecode),
# unless pip installed one built for both. Joining drops signatures, and
# Apple Silicon will not run unsigned code.
intel=$(engine_dir x86_64-apple-darwin)
engine=$app/Contents/Resources/engine
ditto "$(engine_dir aarch64-apple-darwin)" "$engine"
find "$engine" -type f | while read -r f; do
    file -b "$f" | grep -q Mach-O || continue
    case "$(lipo -archs "$f")" in *x86_64*) continue ;; esac
    lipo -create -output "$f.both" "$f" "$intel/${f#"$engine"/}"
    mv "$f.both" "$f"
    codesign --force --sign - "$f"
done

cp packaging/macos/AppIcon.icns "$app/Contents/Resources/"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist >"$app/Contents/Info.plist"
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

# The app beside a link to Applications, to drag it across, and how to open
# it the first time.
ln -s /Applications "$stage/Applications"
cp packaging/macos/README.txt "$stage/README.txt"
cp LICENSE "$stage/LICENSE.txt"
dmg=dist/Ferriteweazle-$version-macos-universal.dmg
rm -f "$dmg"
hdiutil create -quiet -volname "Ferriteweazle $version" -srcfolder "$stage" -fs HFS+ \
    -format UDZO "$dmg"
du -sh "$dmg"

# Building Ferriteweazle

## From source

You need [Rust](https://rustup.rs) 1.95 or later.

```sh
git clone https://github.com/hobbo91/Ferriteweazle
cd Ferriteweazle
bundle/build.sh     # optional: gw with its own Python, in target/greaseweazle-bundle
cargo run --release
```

The app runs the first gw it finds:

1. The bundle in `target/greaseweazle-bundle`.
2. An installed `gw`: on the PATH, or in `~/.local/bin`, `/opt/homebrew/bin` or
   `/usr/local/bin`.

**Settings > Paths > Greaseweazle Tools (gw cli)** points it at any other gw.

## The Greaseweazle Tools bundle

`bundle/build.sh` builds `target/greaseweazle-bundle`, the gw that packages ship: a standalone
Python from [python-build-standalone](https://github.com/astral-sh/python-build-standalone),
gw's latest release installed into it with pip, and the
[SPS/CAPS library](https://github.com/simonowen/capsimage), which gw needs for IPF and
CT Raw images. Python and the CAPS source are checked against the hashes in
`bundle/`. The CAPS library is free for non-commercial use only.

It needs curl, git and a C/C++ compiler:

- macOS: Xcode's command line tools.
- Windows, in Git Bash: Visual Studio's C++ build tools, and LLVM (clang++ and lld).
- Linux: [zig](https://ziglang.org), which builds for glibc 2.17.

```sh
bundle/build.sh                          # this computer
bundle/build.sh x86_64-apple-darwin      # another processor, run emulated
GREASEWEAZLE=v1.23 bundle/build.sh       # a given gw release
GREASEWEAZLE_SOURCE=~/src/greaseweazle bundle/build.sh   # a local clone
```

To stay on one gw release, set `GREASEWEAZLE` in `bundle/versions` to its tag.

## Tests

```sh
cargo test                                   # unit, window and end-to-end tests
cargo test --test screens -- --ignored       # draws the window to target/screens
```

The end-to-end tests run real gw conversions through the bridge, and skip when
there is no gw; with `FERRITEWEAZLE_REQUIRE_GW=1` they fail instead.
`FERRITEWEAZLE_STANDALONE_GW` names a standalone gw to test as well, such as the
`gw.exe` of gw's Windows download, and `FERRITEWEAZLE_BUNDLE` a bundle to test in
place of `target/greaseweazle-bundle`, such as another processor's, run emulated.
No test opens a device.

## Packages

Each script writes to `dist`, rebuilding the bundle first if gw has a newer
release. `VERSION` is the one in `Cargo.toml`.

| System | Command | Packages |
| --- | --- | --- |
| macOS | `packaging/macos/bundle.sh` | `Ferriteweazle-VERSION-macos-universal.dmg` |
| Windows | `packaging/windows/bundle.sh x64` or `arm64` | `Ferriteweazle-VERSION-win-ARCH.zip` and `.msi` |
| Linux | `packaging/linux/bundle.sh x86_64` or `aarch64` | `Ferriteweazle-VERSION-linux-ARCH.tar.gz`, `Ferriteweazle-VERSION-ARCH.AppImage` and its `.zsync` |

### macOS

One app for Apple Silicon and Intel, macOS 10.15 or newer. Needs rustup's stable
toolchain with both targets, and Rosetta for the Intel bundle:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

The app is signed ad hoc, not notarised.

### Windows

Windows 10 or newer. Run in Git Bash. Needs Rust's `x86_64-pc-windows-msvc` and
`aarch64-pc-windows-msvc` targets, Visual Studio's C++ build tools for the processor,
LLVM, and WiX 5, a .NET tool (install the .NET 8 SDK first):

```sh
dotnet tool install --global wix --version 5.0.2
wix extension add --global WixToolset.UI.wixext/5.0.2
```

The zip runs where it is unzipped. The MSI installs for all users; `INSTALLGW=0`
leaves Greaseweazle Tools out of a silent install, and `LAUNCH=1` opens the app once
installed. Keep the UpgradeCode in `packaging/windows/ferriteweazle.wxs`: Windows
Installer knows a new version by it. Neither is code-signed.

### Linux

glibc 2.17 or newer. Needs [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild),
zig, meson 1.4 or newer, ninja, bison 3.6 or newer, pkg-config, bsdtar, patchelf, objdump,
readelf and appstreamcli (Debian and Ubuntu: `ninja-build`, `bison`, `pkg-config`,
`libarchive-tools`, `patchelf`, `binutils`, `appstream`; meson by `pipx install meson`).
Downloads appimagetool, the AppImage runtime, and the sources and libraries of `lib/`
(`packaging/linux/libraries.sh`), checked against `packaging/linux/appimage.sha256`.
Building for the other processor runs its Python emulated: qemu-user with that processor's
libraries, or Rosetta in a Linux VM. The AppImage's update information names the `.zsync`
file published beside it. With `SIGN_KEY`, a key's fingerprint, the AppImage is signed by
that key, which gpg-agent must hold unlocked: the build checks that at its start and
before signing, then checks the signature.

## Releases

`packaging/release.sh`, on a Mac, builds every package of the commit checked out:
macOS there, Windows and Linux over SSH on the machines named in
`packaging/release.env` (copy `packaging/release.env.example`). It adds
`LGPL-sources-VERSION.tar`, the source of the LGPL code in the Linux packages, and
`SHA256SUMS-VERSION.txt`, which the app's Update checks downloads against. With
`LINUX_SIGN_KEY` in `release.env`, the Linux machine signs the AppImages; unlock the key
there first. Publishing is up to you.

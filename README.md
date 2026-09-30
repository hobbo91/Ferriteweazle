# Ferriteweazle

**Cross-platform, GUI front end for [Greaseweazle](https://github.com/keirf/greaseweazle), Keir Fraser's floppy disk flux reader and writer. It is written in Rust for macOS, Windows and Linux.**

![Ferriteweazle](docs/images/intro_demo.gif)

## What does it do?

- Full feature parity with [Greaseweazle Tools](https://github.com/keirf/greaseweazle) by [Keir Fraser](https://github.com/keirf)
- Modern UI for macOS, Windows and Linux, on x86_64 and arm64
- Supports Greaseweazle and the Adafruit Feather RP2040
- Detects the most likely disk format(s) for reading and writing images
- Single or batched reads, writes and image conversions
- Built-in log viewer that can save to a file
- Shows the command line version of any action, as well as letting you pass extra arguments
- Drag and drop images into the app
- Most settings/options have a helpful tooltip 
- Various safety features, such as confirmations before destructive actions and waiting for an action to finish. 
- Releases bundle the latest [Greaseweazle Tools](https://github.com/keirf/greaseweazle) unmodified, with its dependencies. This is optional: you can point the app at your own `gw` or `gw.exe`


## How does it auto-detect disk formats?

Greaseweazle Tools can't detect formats itself, so the bridge does it using `gw`'s codecs. It decodes both sides of cylinder 0 with every format gw knows and keeps the formats that find every sector. Many formats pass that test, so it ranks them by how closely each sector's position matches that format's layout. If the best still disagree about the track count or an unformatted track, it reads that track to settle it. Cylinder 2 shows whether a 40-track disk needs double step. Apple II disks are told apart by filesystem: ProDOS or DOS 3.3.

Detection isn't always right, so the app also groups formats and image types to make choosing the right one yourself easier.

## Why bundle Greaseweazle Tools in the releases?

Greaseweazle still amazes me: a cheap interface and almost any floppy drive can read almost any 3.5" or 5.25" disk format. Greaseweazle Tools, though, run from the command line, and not everyone wants to work that way. Ferriteweazle provides a graphical interface and ships with Greaseweazle Tools included, so you can download it, run it and get straight to work without following [a bunch of pre-requisite steps first](https://github.com/keirf/greaseweazle/wiki/Software-Installation). 

**You don't have to use the bundled version.** Like other Greaseweazle front ends, Ferriteweazle can [use any installation of Greaseweazle Tools](https://github.com/keirf/greaseweazle/releases): just set the path in **Settings > Paths** when you first start the app to point to the `gw`/`gw.exe` binary.

## Installing

Download the package for your system from the [latest release](https://github.com/hobbo91/Ferriteweazle/releases/latest).

### macOS

macOS 10.15 or newer, Apple Silicon or Intel.

1. Open [`Ferriteweazle-1.1.0-macos-universal.dmg`](https://github.com/hobbo91/Ferriteweazle/releases/download/v1.1.0/Ferriteweazle-1.1.0-macos-universal.dmg).
2. Drag **Ferriteweazle** to **Applications**.
3. Open Ferriteweazle. The app isn't notarised, so macOS blocks it the first time.
4. Open **System Settings > Privacy & Security** and click **Open Anyway**.

### Windows

Windows 10 or newer, x64 or ARM64.

1. Run [`Ferriteweazle-1.1.0-win-x64.msi`](https://github.com/hobbo91/Ferriteweazle/releases/download/v1.1.0/Ferriteweazle-1.1.0-win-x64.msi), or [`Ferriteweazle-1.1.0-win-arm64.msi`](https://github.com/hobbo91/Ferriteweazle/releases/download/v1.1.0/Ferriteweazle-1.1.0-win-arm64.msi) on an ARM PC.
2. If SmartScreen warns you, click **More info**, then **Run anyway**.
3. Follow the installer.

To run without installing, unzip [`Ferriteweazle-1.1.0-win-x64.zip`](https://github.com/hobbo91/Ferriteweazle/releases/download/v1.1.0/Ferriteweazle-1.1.0-win-x64.zip) or [`Ferriteweazle-1.1.0-win-arm64.zip`](https://github.com/hobbo91/Ferriteweazle/releases/download/v1.1.0/Ferriteweazle-1.1.0-win-arm64.zip) and run `Ferriteweazle.exe`.

### Linux

x86-64 or ARM64, glibc 2.17 or newer.

AppImage: [`Ferriteweazle-1.1.0-x86_64.AppImage`](https://github.com/hobbo91/Ferriteweazle/releases/download/v1.1.0/Ferriteweazle-1.1.0-x86_64.AppImage) or [`Ferriteweazle-1.1.0-aarch64.AppImage`](https://github.com/hobbo91/Ferriteweazle/releases/download/v1.1.0/Ferriteweazle-1.1.0-aarch64.AppImage)

```sh
chmod +x Ferriteweazle-1.1.0-x86_64.AppImage
./Ferriteweazle-1.1.0-x86_64.AppImage
```

Or the tarball: [`Ferriteweazle-1.1.0-linux-x86_64.tar.gz`](https://github.com/hobbo91/Ferriteweazle/releases/download/v1.1.0/Ferriteweazle-1.1.0-linux-x86_64.tar.gz) or [`Ferriteweazle-1.1.0-linux-aarch64.tar.gz`](https://github.com/hobbo91/Ferriteweazle/releases/download/v1.1.0/Ferriteweazle-1.1.0-linux-aarch64.tar.gz)

```sh
tar -xzf Ferriteweazle-1.1.0-linux-x86_64.tar.gz
./Ferriteweazle/ferriteweazle
```

On ARM, use the `aarch64` files in the commands.

## Building

You need [Rust](https://rustup.rs) 1.95 or later.

```sh
git clone https://github.com/hobbo91/Ferriteweazle
cd Ferriteweazle
bundle/build.sh     # optional: gw with its own Python
cargo run --release
```

Without the bundle, the app uses an installed `gw`. `bundle/build.sh` needs curl, git and a C/C++ compiler: Xcode's command line tools on macOS, zig on Linux, and on Windows Visual Studio's C++ build tools and LLVM, in Git Bash.

Tests and packages: [BUILDING.md](BUILDING.md).

## Is this just vibe-coded AI slop?

No. I built it in Rust, a language I know well, using Claude as a coding assistant, and I review every change. I also contribute to [Copperline](https://github.com/CopperlineHQ/Copperline), [Coppersynth](https://github.com/CopperlineHQ/Coppersynth) and [FluxBridge](https://github.com/CopperlineHQ/FluxBridge).

## Licence

Ferriteweazle is MIT licensed and comes with no warranty; see [LICENSE](LICENSE). Greaseweazle is by Keir Fraser and is in the public domain. Make an image of any disk you care about before writing to it, use a write-protected disk when you only want to read.

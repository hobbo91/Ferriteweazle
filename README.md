# Ferriteweazle

<img src="assets/ferriteweazle.png" alt="" width="160">

A desktop app for [Greaseweazle](https://github.com/keirf/greaseweazle), Keir Fraser's
floppy disk flux reader and writer. It runs on macOS and Windows; Linux is next.

![Reading a disk](docs/screenshot.png)

## What it does

- Gives every `gw` command a page: read, write, convert, erase, clean, seek, drive
  speed, device info, firmware update, delays, pins, reset and bandwidth.
- Greys out the pages that need a Greaseweazle until one is connected.
- Offers every option of every command. Common options have their own controls; the
  others are under **Advanced options**.
- Finds the format of a disk. **Detect** decodes a few tracks of the disk in the
  drive, or of a flux image, with every format gw knows, and chooses the best
  match and the image type that suits it (Akai to .img, Amiga to .adf).
- Draws a disk map as gw works: a square per track, ten cylinders to a row.
- Shows gw's output from every job since the app opened, to copy, save or clear.
- Shows each page as its `gw` command line with **CLI**. Change either and the
  other follows. It takes only `gw` commands and runs nothing: the page's own
  button runs gw, with no shell.
- Reads a set of disks one after another into numbered files, asking for each.
- Asks before replacing a file.
- Shows the Greaseweazle's model and firmware as soon as it is connected.
- Saves images and presets in Documents/Ferriteweazle, under Images and Presets,
  or in folders chosen under **Settings > Paths**. Nothing else is kept: every
  launch starts afresh.
- Follows the system's light or dark mode, or keeps to either. Choosing one in
  Settings fades to it.
- Takes image files by drag and drop.
- Takes a disk definitions file of your own. gw's parser checks it line by line,
  and its formats come first in the format list.
- Can save gw's output beside each image, and play a sound when a job ends.
- Asks before closing while gw is working, and lets gw stop the drive first.

## How it keeps up with Greaseweazle

Ferriteweazle ships `gw` unmodified, with its own Python, so there is nothing else
to install.

At start-up a bridge ([`src/bridge.py`](src/bridge.py)) reads the commands and
options from gw's argument parsers, and the app builds its pages from them. A new
gw release works once it is bundled: its new options appear under **Advanced options**,
and options it drops disappear.

`engine/build.sh` bundles gw's latest release on GitHub, never a nightly build or a
prerelease. The packaging scripts rebuild the engine first when gw has a newer
release. To stay on one release, set `GREASEWEAZLE` in [`engine/versions`](engine/versions)
to its tag. `cargo build` never checks: a build script that went online would slow
every build and fail offline.

**Settings > Paths > gw** points the app at any installed `gw` instead, such as a
pipx install or a development checkout.

## How it finds a format

gw has no format detection of its own, so the bridge builds it from gw's codecs.
It decodes both sides of cylinder 0 with every format gw knows. It keeps those
that account for every sector on the disk. Many formats agree that far: in gw
1.23, 22 groups share sector IDs, sizes and data rate. gw's template for each
format places every sector, so the bridge ranks them by how far the sectors sit
from where each format writes them. That tells apart the index mark, interleave,
skew and gaps. Where the best still disagree about some track, such as a 40 or
80 track length or an unformatted track, it reads that track. Physical cylinder
2 shows whether a 40-track disk sits in an 80-track drive, which needs double
step. Apple II formats write the same disk, so its filesystem decides: ProDOS or
DOS 3.3.

## Building

You need Rust 1.95 or later, on macOS the Xcode command line tools, and on
Windows Visual Studio's C++ build tools and Git Bash.

```sh
engine/build.sh              # Python and gw's latest release, into target/engine
cargo run                    # uses target/engine, or an installed gw
packaging/macos/bundle.sh    # dist/Ferriteweazle-VERSION-macos-universal.dmg
```

The app is signed ad hoc, so it opens on the Mac that built it. Signed and
notarised releases are still to do.

On Windows, `packaging/windows/bundle.sh x64` or `arm64` (this PC's if neither)
writes two packages to `dist`:

- `Ferriteweazle-VERSION-win-ARCH.zip`: a Ferriteweazle folder that runs where it
  is unzipped, with gw's Python in `ferriteweazle-data` beside the program.
- `Ferriteweazle-VERSION-win-ARCH.msi`: installs the same for all users, in
  Program Files or a folder chosen in the installer, with a Start menu shortcut.
  It replaces any older version in that version's folder, and refuses Windows
  before 10.

The installer needs WiX 5, a .NET tool (with the .NET 8 SDK):

```sh
dotnet tool install --global wix --version 5.0.2
wix extension add --global WixToolset.UI.wixext/5.0.2
```

Keep the UpgradeCode in `packaging/windows/ferriteweazle.wxs`: Windows Installer
knows a new version of Ferriteweazle by it. Neither package is code-signed yet, so
SmartScreen warns about a downloaded copy. `packaging/windows/ferriteweazle.ico`
comes from the logo with `cargo test --test icon windows_icon -- --ignored`.

## Tests

```sh
cargo test                                   # unit, window and end-to-end tests
cargo test --test screens -- --ignored       # draws the window to target/screens
```

The end-to-end tests run real `gw` conversions through the bridge. They skip
when there is no Greaseweazle install. No test opens a device.

## Licence

Ferriteweazle is MIT licensed. Greaseweazle is by Keir Fraser and is in the public
domain.

Ferriteweazle for macOS
=======================

A desktop app for Greaseweazle. It runs on Apple Silicon and Intel Macs,
macOS 10.15 or newer.

Installing
----------
Drag Ferriteweazle.app onto Applications.

The first time
--------------
This build is signed ad hoc, not with a Developer ID, and is not
notarised, so Gatekeeper stops it the first time. Control-click
Ferriteweazle.app in Applications, choose Open, then choose Open again. On
macOS 15 or newer, open System Settings, Privacy & Security, and choose
Open Anyway.

If macOS says the app is damaged, run this in Terminal:

    xattr -dr com.apple.quarantine /Applications/Ferriteweazle.app

Presets
-------
Example presets for common disks, which each page's Presets menu loads,
are in the repository at

    https://github.com/hobbo91/Ferriteweazle/tree/main/presets

Put them in Documents/Ferriteweazle/Presets, or the folder chosen in
Settings, and the menu lists them.

Ferriteweazle is MIT licensed; see LICENSE.txt. THIRD-PARTY-NOTICES.txt
holds the licences of the software it includes.

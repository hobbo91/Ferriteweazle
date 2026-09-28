Ferriteweazle for macOS
=======================

A desktop app for Greaseweazle. It runs on Apple Silicon and Intel Macs,
macOS 10.15 or newer.

Installing
----------
Drag Ferriteweazle.app onto Applications.

The first time
--------------
This build is not signed by Apple, so macOS stops it the first time.
Control-click Ferriteweazle.app in Applications, choose Open, then choose
Open again. On macOS 15 or newer, open System Settings, Privacy & Security,
and choose Open Anyway.

If macOS says the app is damaged, run this in Terminal:

    xattr -dr com.apple.quarantine /Applications/Ferriteweazle.app

Ferriteweazle is MIT licensed; see LICENSE.txt.

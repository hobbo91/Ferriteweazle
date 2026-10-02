# Contributing

Ferriteweazle is a front end for [Greaseweazle Tools](https://github.com/keirf/greaseweazle).
Contributions should describe the user-facing behaviour they change, and which
`gw` command or option it maps to.

The app drives `gw`; it does not reimplement it. Keep disk and format logic in
`gw` where it already lives, and do not modify the bundled Greaseweazle Tools.
A change should work with any `gw` set in **Settings > Paths**, not only the
bundled one.

## Pull requests

Contributions reach `main` through a pull request, and only the maintainer
merges them.

1. Fork the repository and clone your fork.
2. Create a branch from `main`, e.g. `fix/presets-menu-scroll`.
3. Make your change, with a test where one can be written. Window behaviour is
   tested with `egui_kittest` in `tests/ui.rs`.
4. Run the checks below.
5. Push the branch to your fork and open a pull request against `main`.

Keep each pull request to one change. Commit subjects name the area first, as
in `ui: a long Presets menu scrolls in half the window`. Explain in the pull
request what was wrong and how you checked the fix; a screenshot helps for
anything visible.

## Assets

Do not commit or attach copyrighted disk images, flux dumps, IPF files, or
other third-party assets. The CAPS library that the bundle builds is free for
non-commercial use only; do not commit its build output.

Bug reports may name the disk or format used to reproduce a problem, but
should not upload the image itself. Attach the saved Log and the command line
the app shows instead.

## Checks

Run these before opening a pull request:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

You need Rust 1.95 or later; see [BUILDING.md](BUILDING.md). The end-to-end
tests in `tests/gw.rs` run real `gw` conversions and skip when there is no
`gw`. No test opens a device.

## Licence

By contributing you agree that your contribution is licensed under the
[MIT License](LICENSE).

//! Makes assets/ferriteweazle.png, the logo on a clear background, from
//! artwork drawn on a flat placeholder colour (the colour of its corner).
//!
//!     FERRITEWEAZLE_ARTWORK=path/to/art.png cargo test --test icon logo -- --ignored
//!
//! packaging/macos/icon.sh then makes the Mac's sizes from it, and
//! `cargo test --test icon windows_art -- --ignored` the Windows icon and
//! the installer's pictures.

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::imageops::FilterType;
use image::{ExtendedColorType, Rgba, RgbaImage};

const WINDOWS_ICON: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/packaging/windows/ferriteweazle.ico"
);
/// The installer's pictures, at the sizes WiX's dialogs take: the first and
/// last pages' background, whose left 164 pixels hold the picture, and the
/// banner across the other pages.
const INSTALLER: [(&str, u32, u32); 2] = [
    (
        concat!(env!("CARGO_MANIFEST_DIR"), "/packaging/windows/dialog.bmp"),
        493,
        312,
    ),
    (
        concat!(env!("CARGO_MANIFEST_DIR"), "/packaging/windows/banner.bmp"),
        493,
        58,
    ),
];
/// Pixel sizes in the Windows icon: its small and large icons at 100% to 200%
/// scaling, and Explorer's larger views.
const WINDOWS_SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];

/// RGB distance from the placeholder below which a pixel is background.
const NEAR: f32 = 40.0;
/// Width in pixels of the edge where the art blends into the placeholder.
const EDGE: i64 = 4;
/// Mean channel value (0-255) below which an edge pixel is outline.
const OUTLINE: f32 = 70.0;

#[test]
#[ignore = "remakes the logo from artwork"]
fn logo() {
    let source =
        std::env::var("FERRITEWEAZLE_ARTWORK").expect("FERRITEWEAZLE_ARTWORK names the artwork");
    let art = image::open(source).expect("the artwork opens").to_rgba8();
    let clear = clear_background(&art);
    assert_eq!(clear.get_pixel(0, 0)[3], 0, "the corner is clear");
    let centre = clear.get_pixel(art.width() / 2, art.height() / 2);
    assert_eq!(centre[3], 255, "the middle is solid");
    clear
        .save(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/ferriteweazle.png"
        ))
        .unwrap();
}

#[test]
#[ignore = "remakes the Windows icon and installer pictures from the logo"]
fn windows_art() {
    let logo = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/ferriteweazle.png");
    let art = image::open(logo).expect("the logo opens").to_rgba8();
    // The logo on white: the dialog's in its left panel, the banner's at its right.
    for ((path, w, h), (size, x, y)) in INSTALLER.into_iter().zip([(132, 16, 40), (48, 437, 5)]) {
        let mut picture = RgbaImage::from_pixel(w, h, Rgba([255; 4]));
        let small = image::imageops::resize(&art, size, size, FilterType::Lanczos3);
        image::imageops::overlay(&mut picture, &small, x, y);
        let rgb = image::DynamicImage::ImageRgba8(picture).into_rgb8();
        rgb.save_with_format(path, image::ImageFormat::Bmp).unwrap();
    }
    let frames: Vec<IcoFrame> = WINDOWS_SIZES
        .iter()
        .map(|&size| {
            let small = image::imageops::resize(&art, size, size, FilterType::Lanczos3);
            IcoFrame::as_png(small.as_raw(), size, size, ExtendedColorType::Rgba8).unwrap()
        })
        .collect();
    let file = std::fs::File::create(WINDOWS_ICON).unwrap();
    IcoEncoder::new(file).encode_images(&frames).unwrap();
}

#[test]
fn the_installer_pictures_are_the_sizes_wix_takes() {
    for (path, w, h) in INSTALLER {
        assert_eq!(image::image_dimensions(path).unwrap(), (w, h), "{path}");
    }
}

#[test]
fn the_windows_icon_holds_every_size() {
    let ico = std::fs::read(WINDOWS_ICON).expect("packaging/windows/ferriteweazle.ico exists");
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    // Each 16-byte entry starts with the width and height; 0 means 256.
    let sizes: Vec<u32> = (0..count)
        .map(|i| &ico[6 + 16 * i..])
        .inspect(|entry| assert_eq!(entry[0], entry[1], "square"))
        .map(|entry| if entry[0] == 0 { 256 } else { entry[0] as u32 })
        .collect();
    assert_eq!(sizes, WINDOWS_SIZES);
}

/// The artwork with its placeholder colour made clear. Edge pixels, where the
/// outline fades into it, take the outline's colour at the fade's opacity, so
/// no fringe of the placeholder is left.
fn clear_background(art: &RgbaImage) -> RgbaImage {
    let key = rgb(art.get_pixel(0, 0));
    let (w, h) = art.dimensions();
    let background: Vec<bool> = art
        .pixels()
        .map(|p| {
            let c = rgb(p);
            (0..3).map(|i| (c[i] - key[i]).powi(2)).sum::<f32>().sqrt() < NEAR
        })
        .collect();
    let bg = |x: u32, y: u32| background[(y * w + x) as usize];
    let near_background = |x: u32, y: u32| {
        (-EDGE..=EDGE).any(|dy| {
            (-EDGE..=EDGE).any(|dx| {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                (0..w as i64).contains(&nx)
                    && (0..h as i64).contains(&ny)
                    && bg(nx as u32, ny as u32)
            })
        })
    };
    let edge: Vec<(u32, u32, [f32; 3])> = art
        .enumerate_pixels()
        .filter(|&(x, y, _)| !bg(x, y) && near_background(x, y))
        .map(|(x, y, p)| (x, y, rgb(p)))
        .collect();
    // The outline's colour: the mean of the edge's dark pixels.
    let dark: Vec<[f32; 3]> = edge
        .iter()
        .map(|e| e.2)
        .filter(|c| c.iter().sum::<f32>() < 3.0 * OUTLINE)
        .collect();
    let outline =
        [0, 1, 2].map(|i| dark.iter().map(|c| c[i]).sum::<f32>() / dark.len().max(1) as f32);
    let span = [0, 1, 2].map(|i| key[i] - outline[i]);
    let span_len = span.iter().map(|s| s * s).sum::<f32>();
    let mut out = art.clone();
    for (pixel, &clear) in out.pixels_mut().zip(&background) {
        if clear {
            *pixel = Rgba([0, 0, 0, 0]);
        }
    }
    let [r, g, b] = outline.map(|v| v.round() as u8);
    for (x, y, c) in edge {
        // 0 at the outline, 1 at the placeholder.
        let t =
            ((0..3).map(|i| (c[i] - outline[i]) * span[i]).sum::<f32>() / span_len).clamp(0.0, 1.0);
        out.put_pixel(x, y, Rgba([r, g, b, ((1.0 - t) * 255.0).round() as u8]));
    }
    out
}

fn rgb(p: &Rgba<u8>) -> [f32; 3] {
    [p[0] as f32, p[1] as f32, p[2] as f32]
}

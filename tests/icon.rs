//! Makes assets/ferriteweazle.png, the logo on a clear background, from
//! artwork drawn on a flat placeholder colour (the colour of its corner).
//!
//!     FERRITEWEAZLE_ARTWORK=path/to/art.png cargo test --test icon -- --ignored
//!
//! packaging/macos/icon.sh then makes the app's sizes from it.

use image::{Rgba, RgbaImage};

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

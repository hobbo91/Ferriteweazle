#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ferriteweazle")
            .with_app_id("ferriteweazle")
            .with_icon(
                eframe::icon_data::from_png_bytes(ferriteweazle::theme::LOGO)
                    .expect("the logo is a PNG"),
            )
            // As short as the sidebar's full list allows, so it never scrolls.
            .with_inner_size([1040.0, 744.0])
            .with_min_inner_size([980.0, 744.0]),
        // Open at the size above, not as it was left.
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "Ferriteweazle",
        options,
        Box::new(|cc| Ok(Box::new(ferriteweazle::App::new(cc)))),
    )
}

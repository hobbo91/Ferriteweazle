#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

fn main() -> eframe::Result {
    #[cfg_attr(not(windows), expect(unused_mut))]
    let mut options = eframe::NativeOptions {
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
    // Every Windows 10 has DirectX 12, with a software fallback; Windows on ARM
    // has no OpenGL of its own, and wgpu may choose a broken one.
    #[cfg(windows)]
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends =
            eframe::wgpu::Backends::from_env().unwrap_or(eframe::wgpu::Backends::DX12);
    }
    eframe::run_native(
        "Ferriteweazle",
        options,
        Box::new(|cc| Ok(Box::new(ferriteweazle::App::new(cc)))),
    )
}

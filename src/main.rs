// build.rs repeats this subsystem with version 10.0, which Windows 7 and 8 refuse.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

/// The window's size when it opens, and its smallest.
const SIZE: [f32; 2] = [1040.0, 744.0];

fn main() -> eframe::Result {
    #[cfg(windows)]
    if !windows_supported(windows_version::OsVersion::current()) {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Ferriteweazle")
            .set_description("Ferriteweazle needs Windows 10 or newer.")
            .show();
        std::process::exit(1);
    }
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
            .with_inner_size(SIZE)
            .with_min_inner_size(SIZE),
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

/// Windows 10 or newer; Windows 11 is 10.0 too.
#[cfg(windows)]
fn windows_supported(os: windows_version::OsVersion) -> bool {
    os.major >= 10
}

#[cfg(all(test, windows))]
mod tests {
    use windows_version::OsVersion;

    #[test]
    fn windows_8_1_is_refused_and_10_and_11_are_not() {
        assert!(!super::windows_supported(OsVersion::new(6, 3, 0, 9600)));
        assert!(super::windows_supported(OsVersion::new(10, 0, 0, 10240)));
        assert!(super::windows_supported(OsVersion::new(10, 0, 0, 26200)));
    }
}

//! Gives the Windows program its icon and version, and marks it as needing
//! Windows 10. Other builds are left as they are.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // cfg(windows) is the computer building: winresource runs the Windows
    // SDK's rc.exe, and Cargo.toml adds it only on Windows.
    #[cfg(windows)]
    if std::env::var("TARGET").is_ok_and(|t| t.ends_with("-windows-msvc")) {
        windows();
    }
}

#[cfg(windows)]
fn windows() {
    const ICON: &str = "packaging/windows/ferriteweazle.ico";
    println!("cargo::rerun-if-changed={ICON}");
    winresource::WindowsResource::new()
        .set_icon(ICON)
        .set("ProductName", "Ferriteweazle")
        // Task Manager and Open With show this as the program's name.
        .set("FileDescription", "Ferriteweazle")
        .set("OriginalFilename", "Ferriteweazle.exe")
        .set("LegalCopyright", "Copyright 2026 Lee Hobson. MIT License.")
        .compile()
        .expect("the Windows SDK's rc.exe compiles the icon and version");
    // The standard library calls ProcessPrng, which Windows 7 and 8 lack, so
    // they would fail to load the program; subsystem version 10.0 has them
    // refuse it with their own message instead. The subsystem must match
    // main.rs's windows_subsystem.
    let subsystem = match std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS") {
        Some(_) => "CONSOLE",
        None => "WINDOWS",
    };
    println!("cargo::rustc-link-arg-bins=/SUBSYSTEM:{subsystem},10.0");
}

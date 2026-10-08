//! Packs the bridge for the command line, points the Linux program at the
//! libraries beside it, and gives the Windows program its icon and version
//! and marks it as needing Windows 10.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    bridge();
    // A Linux package's lib/, beside the program, holds the libraries some
    // systems lack (packaging/linux/libraries.sh).
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "linux") {
        println!("cargo::rustc-link-arg-bins=-Wl,-rpath,$ORIGIN/lib");
    }
    // cfg(windows) is the computer building: winresource runs the Windows
    // SDK's rc.exe, and Cargo.toml adds it only on Windows.
    #[cfg(windows)]
    if std::env::var("TARGET").is_ok_and(|t| t.ends_with("-windows-msvc")) {
        windows();
    }
}

include!("src/strip.rs");

/// src/bridge.py without its comments and docstrings, zlib-compressed and in
/// base64, for tools.rs: whole, it would soon pass the 32,767 characters of
/// a Windows command line.
fn bridge() {
    const BRIDGE: &str = "src/bridge.py";
    println!("cargo::rerun-if-changed={BRIDGE}");
    println!("cargo::rerun-if-changed=src/strip.rs");
    let source = std::fs::read_to_string(BRIDGE).expect("src/bridge.py is readable");
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(strip(&source).as_bytes(), 9);
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("Cargo sets OUT_DIR"));
    std::fs::write(out.join("bridge.b64"), base64(&packed)).expect("OUT_DIR is writable");
}

fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let byte = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = byte(0) << 16 | byte(1) << 8 | byte(2);
        for i in 0..4 {
            out.push(match i <= chunk.len() {
                true => ABC[(n >> (18 - 6 * i) & 63) as usize] as char,
                false => '=',
            });
        }
    }
    out
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
    // std calls ProcessPrng, which Windows 7 and 8 lack: subsystem version 10.0
    // has them refuse the program with their own message, not fail to load it.
    // The subsystem must match main.rs's windows_subsystem.
    let subsystem = match std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS") {
        Some(_) => "CONSOLE",
        None => "WINDOWS",
    };
    println!("cargo::rustc-link-arg-bins=/SUBSYSTEM:{subsystem},10.0");
}

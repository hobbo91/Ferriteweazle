//! A desktop app for Greaseweazle.
//!
//! Greaseweazle's `gw` does the work. A small Python bridge reads gw's command
//! line as data and runs gw commands. The app builds its pages from that data,
//! so a new gw needs no new app.

mod app;
pub mod command;
pub mod device;
mod diskmap;
pub mod engine;
pub mod form;
pub mod job;
#[cfg(target_os = "linux")]
mod portal;
pub mod presets;
pub mod progress;
pub mod schema;
pub mod service;
pub mod theme;
mod udev;
pub mod update;

pub use app::{App, Drawer, Page, Settings};

/// The user's home folder, if the system names one.
fn home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(Into::into)
}

/// The system's per-user folder for what the app keeps for itself, such as
/// gw releases installed by Update.
pub fn data_folder() -> std::path::PathBuf {
    let home = home().unwrap_or_default();
    let var = |name, or: &str| std::env::var_os(name).map_or_else(|| home.join(or), Into::into);
    let base = match () {
        _ if cfg!(target_os = "macos") => home.join("Library/Application Support"),
        _ if cfg!(windows) => var("LOCALAPPDATA", "AppData/Local"),
        _ => var("XDG_DATA_HOME", ".local/share"),
    };
    base.join("Ferriteweazle")
}

/// Documents/Ferriteweazle in the home folder, where the app saves by default.
pub fn app_folder() -> std::path::PathBuf {
    home()
        .unwrap_or_default()
        .join("Documents")
        .join("Ferriteweazle")
}

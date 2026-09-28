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
pub mod presets;
pub mod progress;
pub mod schema;
pub mod service;
pub mod theme;

pub use app::{App, Drawer, Page, Settings};

/// The user's home folder, if the system names one.
fn home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(Into::into)
}

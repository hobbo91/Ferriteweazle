//! A desktop app for Greaseweazle.
//!
//! Greaseweazle's `gw` does the work. A small Python bridge reads gw's command
//! line as data and runs gw commands. The app builds its pages from that data,
//! so options a new gw adds get fields with no change to the app.

mod app;
pub mod command;
pub mod device;
mod diskmap;
mod filemap;
pub mod form;
pub mod image;
pub mod job;
mod lines;
#[cfg(target_os = "macos")]
mod menu;
#[cfg(target_os = "linux")]
mod portal;
pub mod presets;
pub mod progress;
pub mod schema;
pub mod service;
pub mod standalone;
mod surface;
pub mod theme;
pub mod tools;
pub mod track;
mod udev;
pub mod update;

pub use app::{
    ABOUT_SIZE, App, Drawer, Page, SMALLEST, Settings, WINDOW, about, about_image, opening_size,
};
pub use surface::{Analysis, Media, Shows};

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The user's home folder, if the system names one. On Windows, the profile
/// folder, whatever HOME says.
fn home() -> Option<PathBuf> {
    std::env::home_dir()
}

/// The folder an environment variable names, if absolute: XDG ignores an empty
/// or relative one.
fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The system's per-user folder for what the app keeps for itself, such as
/// gw releases installed by Update.
pub fn data_folder() -> PathBuf {
    let home = home().unwrap_or_default();
    let var = |name, or: &str| absolute(std::env::var_os(name)).unwrap_or_else(|| home.join(or));
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        var("LOCALAPPDATA", "AppData/Local")
    } else {
        var("XDG_DATA_HOME", ".local/share")
    };
    base.join("Ferriteweazle")
}

/// Ferriteweazle in the user's Documents folder, where the app saves by default.
pub fn app_folder() -> PathBuf {
    // Looked up once: Settings asks for it every frame.
    static FOLDER: OnceLock<PathBuf> = OnceLock::new();
    FOLDER
        .get_or_init(|| documents().join("Ferriteweazle"))
        .clone()
}

/// Where the system puts the user's Documents, which may be redirected or
/// have a name in the desktop's language.
fn documents() -> PathBuf {
    let home = home().unwrap_or_default();
    #[cfg(windows)]
    let found = known_documents();
    #[cfg(not(windows))]
    let found = match cfg!(target_os = "macos") {
        true => None,
        false => {
            let config = absolute(std::env::var_os("XDG_CONFIG_HOME"))
                .unwrap_or_else(|| home.join(".config"));
            let dirs = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
            xdg_documents(&dirs, &home)
        }
    };
    found.unwrap_or_else(|| home.join("Documents"))
}

/// XDG_DOCUMENTS_DIR in xdg-user-dirs' file: `"$HOME/Dokumente"` or an absolute path.
#[cfg(not(windows))]
fn xdg_documents(dirs: &str, home: &std::path::Path) -> Option<PathBuf> {
    let value = dirs
        .lines()
        .find_map(|l| l.trim().strip_prefix("XDG_DOCUMENTS_DIR="))?
        .trim_matches('"');
    match value.strip_prefix("$HOME") {
        Some(rest) => Some(home.join(rest.trim_start_matches('/'))),
        None => absolute(Some(value.into())),
    }
}

/// FOLDERID_Documents, which OneDrive or a policy may have moved.
#[cfg(windows)]
fn known_documents() -> Option<PathBuf> {
    known_folders::get_known_folder_path(known_folders::KnownFolder::Documents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_relative_folder_variable_is_ignored() {
        assert_eq!(absolute(Some("".into())), None);
        assert_eq!(absolute(Some("Ferriteweazle".into())), None);
        let whole = std::env::temp_dir();
        assert_eq!(absolute(Some(whole.clone().into())), Some(whole));
    }

    #[cfg(unix)]
    #[test]
    fn documents_is_where_the_xdg_user_directories_put_it() {
        let home = std::path::Path::new("/home/anna");
        let dirs = "# Written by xdg-user-dirs-update\n\
                    XDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n\
                    XDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n";
        assert_eq!(xdg_documents(dirs, home), Some(home.join("Dokumente")));
        let whole = "XDG_DOCUMENTS_DIR=\"/data/docs\"\n";
        assert_eq!(xdg_documents(whole, home), Some("/data/docs".into()));
        assert_eq!(xdg_documents("XDG_DOCUMENTS_DIR=\"docs\"\n", home), None);
        assert_eq!(xdg_documents("", home), None);
    }

    #[cfg(windows)]
    #[test]
    fn documents_is_windows_known_folder() {
        let documents = known_documents().expect("Windows names a Documents folder");
        assert!(documents.is_absolute(), "{documents:?}");
        assert_eq!(app_folder(), documents.join("Ferriteweazle"));
    }
}

//! A page's settings, saved as a file in the presets folder.

use crate::command::Values;
use crate::form::Output;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const EXTENSION: &str = "json";

/// A command's settings, as saved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    /// The gw command the settings are for, such as `read`.
    pub command: String,
    pub values: Values,
    /// Output folders, names and types, keyed as in the app's settings.
    pub outputs: BTreeMap<String, Output>,
}

/// Documents/Ferriteweazle/Presets, made by the first save.
pub fn default_folder() -> PathBuf {
    crate::app_folder().join("Presets")
}

/// The presets for a command in a folder, by name.
pub fn list(folder: &Path, command: &str) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXTENSION))
        .filter(|p| load(p).is_ok_and(|preset| preset.command == command))
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
        .collect();
    found.sort();
    found
}

pub fn load(path: &Path) -> Result<Preset, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    match serde_json::from_str::<Preset>(&text) {
        Ok(preset) if !preset.command.is_empty() => Ok(preset),
        _ => Err("It is not a Ferriteweazle preset.".into()),
    }
}

/// Where a preset of this name is saved. Characters Windows bans in file names
/// become hyphens.
pub fn path(folder: &Path, name: &str) -> PathBuf {
    let bad = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
    let safe = name.trim().replace(bad, "-");
    folder.join(format!("{safe}.{EXTENSION}"))
}

/// Saves a preset, making the folder if needed.
pub fn save(folder: &Path, name: &str, preset: &Preset) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let path = path(folder, name);
    let text = serde_json::to_string_pretty(preset).expect("a preset is plain data");
    std::fs::write(&path, text + "\n")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_preset_lists_under_its_command_and_loads_back() {
        let folder = std::env::temp_dir().join(format!("fw-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let mut preset = Preset {
            command: "read".into(),
            ..Preset::default()
        };
        preset.values.set("revs", "5");
        assert!(list(&folder, "read").is_empty(), "no folder, no presets");
        let saved = save(&folder, "Amiga: DD", &preset).unwrap();
        assert_eq!(saved, folder.join("Amiga- DD.json"));
        std::fs::write(folder.join("notes.json"), "{\"other\": 1}").unwrap();
        assert_eq!(
            list(&folder, "read"),
            [("Amiga- DD".to_owned(), saved.clone())]
        );
        assert!(list(&folder, "write").is_empty());
        assert_eq!(load(&saved).unwrap(), preset);
        assert!(load(&folder.join("notes.json")).is_err());
        std::fs::remove_dir_all(folder).ok();
    }
}

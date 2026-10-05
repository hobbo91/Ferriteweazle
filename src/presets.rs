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
    /// Shown when the preset is hovered in the Presets menu.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// Documents/Ferriteweazle/Presets, made by the first save.
pub fn default_folder() -> PathBuf {
    crate::app_folder().join("Presets")
}

/// The presets for a command in a folder, by name: each one's name, file and description.
pub fn list(folder: &Path, command: &str) -> Vec<(String, PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXTENSION))
        .filter_map(|p| {
            let preset = load(&p).ok().filter(|preset| preset.command == command)?;
            let name = p.file_stem()?.to_string_lossy().into_owned();
            Some((name, p, preset.description))
        })
        .collect();
    found.sort_by_cached_key(|(name, ..)| name.to_lowercase());
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
/// become hyphens; a Windows device name, such as COM3, gains a leading one.
pub fn path(folder: &Path, name: &str) -> PathBuf {
    let bad = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
    let safe = name.trim().replace(bad, "-");
    // Windows 10 takes NUL.json, even NUL.old.json, for the device.
    let stem = safe.split('.').next().unwrap_or_default();
    let stem = stem.trim_end().to_ascii_uppercase();
    let numbered = stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.ends_with(|c: char| c.is_ascii_digit());
    let device = numbered || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str());
    let safe = if device { format!("-{safe}") } else { safe };
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
    fn a_saved_preset_lists_under_its_command_with_its_description_and_loads_back() {
        let folder = std::env::temp_dir().join(format!("fw-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let mut preset = Preset {
            command: "read".into(),
            description: "Both sides, 80 cylinders.".into(),
            ..Preset::default()
        };
        preset.values.set("revs", "5");
        assert!(list(&folder, "read").is_empty(), "no folder, no presets");
        let saved = save(&folder, "Amiga: DD", &preset).unwrap();
        assert_eq!(saved, folder.join("Amiga- DD.json"));
        std::fs::write(folder.join("notes.json"), "{\"other\": 1}").unwrap();
        let listed = (
            "Amiga- DD".to_owned(),
            saved.clone(),
            preset.description.clone(),
        );
        assert_eq!(list(&folder, "read"), [listed]);
        assert!(list(&folder, "write").is_empty());
        assert_eq!(load(&saved).unwrap(), preset);
        assert!(load(&folder.join("notes.json")).is_err());
        let plain = Preset {
            description: String::new(),
            ..preset
        };
        let text = serde_json::to_string(&plain).unwrap();
        assert!(
            !text.contains("description"),
            "an empty one is left out: {text}"
        );
        std::fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn presets_list_in_name_order_whatever_the_case() {
        let folder = std::env::temp_dir().join(format!("fw-presets-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let preset = Preset {
            command: "read".into(),
            ..Preset::default()
        };
        for name in ["PC 1.44", "atari st", "Amiga DD"] {
            save(&folder, name, &preset).unwrap();
        }
        let names: Vec<String> = list(&folder, "read").into_iter().map(|(n, ..)| n).collect();
        assert_eq!(names, ["Amiga DD", "atari st", "PC 1.44"]);
        std::fs::remove_dir_all(folder).ok();
    }

    /// The repository's presets folder, each by name.
    fn examples() -> Vec<(String, Preset)> {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("presets");
        let mut paths: Vec<_> = std::fs::read_dir(folder)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        paths.sort();
        paths
            .iter()
            .map(|p| {
                let name = p.file_stem().unwrap().to_string_lossy().into_owned();
                (
                    name,
                    load(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())),
                )
            })
            .collect()
    }

    #[test]
    fn the_example_presets_set_what_gw_has_and_stay_within_83_cylinders() {
        let schema: crate::schema::Schema =
            serde_json::from_str(include_str!("gw-1.23.json")).unwrap();
        let examples = examples();
        assert_eq!(examples.len(), 33);
        for (name, preset) in &examples {
            let cmd = schema.command(&preset.command).unwrap();
            let page = if preset.command == "read" {
                "Read "
            } else {
                "Write "
            };
            assert!(name.starts_with(page), "{name}: gw {}", preset.command);
            let values = serde_json::to_value(&preset.values).unwrap();
            for dest in values.as_object().unwrap().keys() {
                assert!(
                    cmd.arg(dest).is_some(),
                    "{name}: gw {} has no {dest}",
                    cmd.name
                );
            }
            let format = preset.values.get("format");
            assert!(
                format.is_empty() || schema.formats.iter().any(|f| f == format),
                "{name}"
            );
            let last = crate::form::last_cylinder(preset.values.get("tracks"), None);
            assert!(
                last.is_none_or(|c| c <= crate::form::LAST_USUAL_CYLINDER),
                "{name} steps to cylinder {last:?}"
            );
            // A read makes the type the page picks for its format, or flux without one.
            let ext = preset.outputs.get("read/file").map(|o| o.ext.as_str());
            let wanted = match (preset.command.as_str(), format) {
                // Raw keeps flux, which a format only checks.
                ("read", _) if preset.values.on("raw") => Some(".scp".to_owned()),
                ("read", "") => Some(".scp".to_owned()),
                ("read", format) => Some(crate::form::type_for(&schema, format)),
                _ => None,
            };
            assert_eq!(ext, wanted.as_deref(), "{name}");
        }
    }

    #[test]
    fn a_preset_named_as_a_windows_device_is_saved_as_a_file() {
        let name = |n| path(Path::new("/p"), n).file_name().unwrap().to_owned();
        assert_eq!(name("COM3"), "-COM3.json");
        assert_eq!(name("nul.old"), "-nul.old.json");
        assert_eq!(name("lpt1 "), "-lpt1.json");
        assert_eq!(name("Console"), "Console.json");
        assert_eq!(name("COM10"), "COM10.json");
    }
}

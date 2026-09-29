//! gw's command line as data, as the bridge reports it.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Schema {
    pub version: String,
    pub commands: Vec<Command>,
    pub formats: Vec<String>,
    pub images: BTreeMap<String, Image>,
    /// gw's help for its small value languages, by name: DRIVE, SPEED, TSPEC...
    #[serde(default)]
    pub notes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Command {
    pub name: String,
    pub about: String,
    pub args: Vec<Arg>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Arg {
    /// Option strings, canonical first. Empty for a positional argument.
    #[serde(default)]
    pub flags: Vec<String>,
    pub dest: String,
    /// Takes no value: present or absent.
    #[serde(default)]
    pub switch: bool,
    /// Name of gw's parser for the value: `TrackSet`, `period`, `min_int`...
    #[serde(rename = "type")]
    pub ty: Option<String>,
    pub default: Option<String>,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub required: bool,
    /// Arguments sharing a group are mutually exclusive.
    pub group: Option<usize>,
    /// gw's name for the kind of value, such as TSPEC, which `Schema::note` explains.
    pub metavar: Option<String>,
    #[serde(default)]
    pub help: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Image {
    pub name: String,
    pub writable: bool,
    pub default_format: Option<String>,
    /// gw finds the disk format in the file itself, as it does an .nsi's from its size.
    #[serde(default)]
    pub finds_format: bool,
    /// Holds tracks as they lie on the disk, flux or decoded, not sectors.
    #[serde(default)]
    pub tracks: bool,
    /// A sector image gw opens only with a disk format, as it does an .img.
    #[serde(default)]
    pub needs_format: bool,
    /// Settings for `file.ext::opt=value`.
    #[serde(default)]
    pub read_opts: Vec<ImageOpt>,
    #[serde(default)]
    pub write_opts: Vec<ImageOpt>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageOpt {
    pub name: String,
    /// gw's default: a name, for an option with `choices`.
    pub default: Option<serde_json::Value>,
    /// The names gw takes for the value, such as SCP disk types.
    #[serde(default)]
    pub choices: Vec<String>,
}

impl ImageOpt {
    /// A flag is passed as `::name`: gw counts any value it is given as true.
    pub fn flag(&self) -> bool {
        matches!(self.default, Some(serde_json::Value::Bool(_)))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FormatInfo {
    pub cyls: u32,
    pub heads: u32,
    pub encoding: Option<String>,
    /// The fewest and most sectors on a track.
    pub sectors: Option<(u32, u32)>,
    /// Size of a sector image of the whole disk.
    pub bytes: Option<u64>,
    /// gw write verifies each track of this format before the next.
    #[serde(default)]
    pub verifies: bool,
}

/// What a disk definitions file adds, checked with gw's own parser.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct DiskDefs {
    /// The formats gw can use.
    pub formats: Vec<String>,
    /// The formats it names that gw cannot use.
    #[serde(default)]
    pub failed: Vec<String>,
    /// gw's objections to those, each once.
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Port {
    pub device: String,
    pub name: Option<String>,
    /// gw's guess that this is a Greaseweazle: 0 if not.
    #[serde(default)]
    pub score: i32,
    /// Linux denies this account read and write access to it.
    #[serde(default)]
    pub denied: bool,
}

impl Schema {
    pub fn command(&self, name: &str) -> Option<&Command> {
        self.commands.iter().find(|c| c.name == name)
    }

    /// The image type for a file name, by its suffix.
    pub fn image(&self, path: &str) -> Option<(&str, &Image)> {
        self.images
            .get_key_value(&extension(path)?)
            .map(|(k, v)| (k.as_str(), v))
    }

    pub fn note(&self, name: &str) -> Option<&str> {
        self.notes.get(name).map(String::as_str)
    }
}

impl Command {
    pub fn arg(&self, dest: &str) -> Option<&Arg> {
        self.args.iter().find(|a| a.dest == dest)
    }
}

impl Arg {
    pub fn positional(&self) -> bool {
        self.flags.is_empty()
    }

    pub fn flag(&self) -> Option<&str> {
        self.flags.first().map(String::as_str)
    }

    pub fn is(&self, ty: &str) -> bool {
        self.ty.as_deref() == Some(ty)
    }
}

/// The lower-case suffix of a file name, with its dot, ignoring `::options`.
pub fn extension(path: &str) -> Option<String> {
    let name = path.split("::").next()?.rsplit(['/', '\\']).next()?;
    let dot = name.rfind('.').filter(|&i| i > 0)?;
    Some(name[dot..].to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_ignore_case_folders_and_options() {
        assert_eq!(extension("/a.b/Disk.IMG").as_deref(), Some(".img"));
        assert_eq!(extension("out.hfe::version=3").as_deref(), Some(".hfe"));
        assert_eq!(extension(r"C:\floppies\x.scp").as_deref(), Some(".scp"));
        assert_eq!(extension("/a.b/noext"), None);
        assert_eq!(extension(".hidden"), None);
    }
}

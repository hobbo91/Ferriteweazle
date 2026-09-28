//! Finds a Python that can import greaseweazle, and starts the bridge in it.

use std::path::{Path, PathBuf};
use std::process::Command;

const BRIDGE: &str = include_str!("bridge.py");

#[derive(Debug, Clone, PartialEq)]
pub struct Engine {
    pub python: PathBuf,
    pub origin: Origin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Chosen in Settings.
    Custom,
    /// Shipped inside the app.
    Bundled,
    /// A `gw` installed with pip, pipx or uv.
    Installed,
}

impl Engine {
    /// The custom choice if given, else the bundled engine, else an installed `gw`.
    pub fn find(custom: Option<&Path>) -> Option<Engine> {
        let (python, origin) = if let Some(path) = custom {
            (interpreter(path)?, Origin::Custom)
        } else if let Some(python) = bundled() {
            (python, Origin::Bundled)
        } else {
            (installed()?, Origin::Installed)
        };
        Some(Engine { python, origin })
    }

    /// `python -c BRIDGE MODE`, ready for more arguments.
    pub fn bridge(&self, mode: &str) -> Command {
        let mut cmd = Command::new(&self.python);
        cmd.args(["-c", BRIDGE, mode])
            .env("PYTHONIOENCODING", "utf-8");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }
}

fn python_in(root: &Path) -> PathBuf {
    if cfg!(windows) {
        root.join("python.exe")
    } else {
        root.join("bin/python3")
    }
}

fn bundled() -> Option<PathBuf> {
    bundled_with(&std::env::current_exe().ok()?)
}

/// `Contents/Resources/engine` in a macOS app, `engine` beside the program
/// elsewhere, and `target/engine` for `cargo run`.
fn bundled_with(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    ["../Resources/engine", "engine", "../engine"]
        .into_iter()
        .map(|rel| python_in(&dir.join(rel)))
        .find(|p| p.is_file())
}

/// An installed `gw` on the PATH or in the usual places, which apps started
/// from a desktop do not always have on their PATH.
fn installed() -> Option<PathBuf> {
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .chain(crate::home().map(|h| h.join(".local/bin")))
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .map(|d| d.join("gw"))
        .filter(|p| p.is_file())
        .find_map(|p| interpreter(&p))
}

/// The Python behind a `gw` launcher script, or the path itself if it names
/// a Python.
fn interpreter(path: &Path) -> Option<PathBuf> {
    if path.file_name()?.to_string_lossy().starts_with("python") {
        return Some(path.to_path_buf());
    }
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
    let mut lines = text.lines();
    let first = lines.next()?.strip_prefix("#!")?;
    // pip writes a /bin/sh trampoline when the venv path has a space.
    let line = match first.trim() {
        "/bin/sh" => lines.next()?.strip_prefix("'''exec'")?,
        line => line,
    };
    let mut words = words(line);
    let mut python = words.next()?;
    if python.ends_with("/env") {
        python = words.next()?;
    }
    python.contains("python").then(|| PathBuf::from(python))
}

/// Space-separated words, where a word may be quoted.
fn words(line: &str) -> impl Iterator<Item = &str> {
    let mut rest = line.trim_start();
    std::iter::from_fn(move || {
        let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''));
        let (word, tail) = match quote {
            Some(q) => rest[1..].split_once(q).unwrap_or((&rest[1..], "")),
            None => rest.split_once(char::is_whitespace).unwrap_or((rest, "")),
        };
        rest = tail.trim_start();
        (!word.is_empty()).then_some(word)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher(test: &str, text: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-{test}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gw");
        std::fs::write(&path, text).unwrap();
        interpreter(&path)
    }

    #[test]
    fn a_launcher_names_its_python() {
        let text = "#!/home/x/.local/pipx/venvs/greaseweazle/bin/python\nimport sys\n";
        let python = launcher("plain", text);
        assert_eq!(
            python,
            Some("/home/x/.local/pipx/venvs/greaseweazle/bin/python".into())
        );
    }

    #[test]
    fn a_trampoline_for_a_path_with_spaces_names_its_python() {
        let text = "#!/bin/sh\n'''exec' \"/Users/x/Library/Application Support/pipx/venvs/greaseweazle/bin/python\" \"$0\" \"$@\"\n' '''\n";
        let python = launcher("trampoline", text);
        let expected = "/Users/x/Library/Application Support/pipx/venvs/greaseweazle/bin/python";
        assert_eq!(python, Some(expected.into()));
    }

    #[test]
    fn env_shebangs_name_the_python_on_the_path() {
        assert_eq!(
            launcher("env", "#!/usr/bin/env python3\n"),
            Some("python3".into())
        );
    }

    #[test]
    fn other_scripts_are_not_engines() {
        assert_eq!(launcher("bash", "#!/bin/bash\necho hi\n"), None);
        assert_eq!(interpreter(Path::new("/nonexistent/gw")), None);
    }

    #[test]
    fn the_engine_inside_a_mac_app_is_found() {
        let app = std::env::temp_dir().join("ferriteweazle-bundle/Ferriteweazle.app/Contents");
        let python = python_in(&app.join("Resources/engine"));
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::create_dir_all(app.join("MacOS")).unwrap();
        std::fs::write(&python, "").unwrap();
        let found = bundled_with(&app.join("MacOS/ferriteweazle")).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            python.canonicalize().unwrap()
        );
    }

    #[test]
    fn a_python_path_is_used_as_it_is() {
        let python = Path::new("/opt/py/bin/python3.14");
        assert_eq!(interpreter(python), Some(python.to_path_buf()));
    }
}

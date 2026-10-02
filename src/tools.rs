//! Finds the Greaseweazle Tools the app runs, and starts the bridge in their Python.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// bridge.py as build.rs packs it: zlib, then base64.
const BRIDGE: &str = include_str!(concat!(env!("OUT_DIR"), "/bridge.b64"));

/// The program `python -c` runs: it unpacks the bridge and runs it as
/// __main__, with bridge.py's name in tracebacks.
fn loader() -> &'static str {
    static LOADER: OnceLock<String> = OnceLock::new();
    LOADER.get_or_init(|| {
        format!(
            "import base64,zlib;exec(compile(zlib.decompress(base64.b64decode('{BRIDGE}')),'bridge.py','exec'))"
        )
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tools {
    /// gw's Python, or gw itself when `standalone`.
    pub python: PathBuf,
    pub origin: Origin,
    /// gw is a program with its Python sealed inside, such as the gw.exe of
    /// gw's Windows download: it runs as it is, with no bridge.
    pub standalone: bool,
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

impl Tools {
    /// The custom choice if given, else the bundle, else an installed `gw`.
    pub fn find(custom: Option<&Path>) -> Option<Tools> {
        let ((python, standalone), origin) = if let Some(path) = custom {
            (gw(path)?, Origin::Custom)
        } else if let Some(python) = bundled() {
            ((python, false), Origin::Bundled)
        } else {
            (installed()?, Origin::Installed)
        };
        Some(Tools {
            python,
            origin,
            standalone,
        })
    }

    /// Greaseweazle Tools with a Python to ask GitHub with: these, or beside a
    /// standalone gw the bundle.
    pub fn with_python(&self) -> Option<Tools> {
        match self.standalone {
            false => Some(self.clone()),
            true => Some(Tools {
                python: bundled()?,
                origin: Origin::Bundled,
                standalone: false,
            }),
        }
    }

    /// The release tag the bundled gw was built from, such as `v1.23`.
    pub fn bundled_tag(&self) -> Option<String> {
        if self.origin != Origin::Bundled {
            return None;
        }
        let bin = self.python.parent()?;
        let root = if cfg!(windows) { bin } else { bin.parent()? };
        let tag = std::fs::read_to_string(root.join("greaseweazle-version")).ok()?;
        Some(tag.trim().to_owned())
    }

    /// The newest gw Update installed in `folder`, if newer than the bundled one.
    pub fn update_in(&self, folder: &Path) -> Option<PathBuf> {
        let bundled = version(&self.bundled_tag()?)?;
        std::fs::read_dir(folder)
            .ok()?
            .flatten()
            .filter_map(|entry| {
                let version = version(entry.file_name().to_str()?)?;
                let path = entry.path();
                (version > bundled && path.join("greaseweazle").is_dir()).then_some((version, path))
            })
            .max()
            .map(|(_, path)| path)
    }

    /// `python -c LOADER MODE`, which runs bridge.py, ready for more arguments.
    pub fn bridge(&self, mode: &str) -> Command {
        let mut cmd = Command::new(&self.python);
        if self.origin == Origin::Bundled {
            // Only the built-in gw: ignore the user's site-packages and Python paths.
            cmd.env("PYTHONNOUSERSITE", "1")
                .env_remove("PYTHONPATH")
                .env_remove("PYTHONHOME");
        }
        if let Some(update) = self.update_in(&updates()) {
            cmd.env("PYTHONPATH", update);
        }
        // No .pyc files: the app leaves nothing behind, and a signed bundle
        // must not change.
        cmd.args(["-c", loader(), mode])
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONDONTWRITEBYTECODE", "1");
        quiet(cmd)
    }
}

/// `cmd` with no console window on Windows.
#[cfg(windows)]
pub fn quiet(mut cmd: Command) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(windows))]
pub fn quiet(cmd: Command) -> Command {
    cmd
}

/// Where Update installs newer gw releases, one folder per tag. Each build
/// has its own: a release keeps the compiled module of the Python that
/// installed it, which another Python cannot load.
pub fn updates() -> PathBuf {
    crate::data_folder()
        .join("gw")
        .join(env!("CARGO_PKG_VERSION"))
}

/// `v1.23.1` as [1, 23, 1]; anything else, such as a half-installed
/// `v1.24.part`, as none.
pub fn version(tag: &str) -> Option<Vec<u32>> {
    tag.strip_prefix('v')?
        .split('.')
        .map(|n| n.parse().ok())
        .collect()
}

fn python_in(root: &Path) -> PathBuf {
    if cfg!(windows) {
        root.join("python.exe")
    } else {
        root.join("bin/python3")
    }
}

fn bundled() -> Option<PathBuf> {
    Some(python_in(&data_with(&std::env::current_exe().ok()?)?))
}

/// Whether a package's own Greaseweazle Tools is where Ferriteweazle looks for it.
pub fn has_bundled() -> bool {
    bundled().is_some()
}

/// The folder a package keeps gw's Python in.
pub const DATA: &str = "greaseweazle";

/// gw's udev rule, which a Linux package keeps beside gw's Python.
pub fn udev_rule() -> Option<PathBuf> {
    udev_rule_with(&std::env::current_exe().ok()?)
}

fn udev_rule_with(exe: &Path) -> Option<PathBuf> {
    Some(data_with(exe)?.join(crate::udev::RULE)).filter(|r| r.is_file())
}

/// `Contents/Resources/greaseweazle` in a macOS app, `greaseweazle`
/// beside the program elsewhere, and `target/greaseweazle-bundle` for `cargo run`.
fn data_with(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    // By its name, with no `..`: Settings shows the path.
    let above = dir.parent().unwrap_or(dir);
    [
        above.join("Resources").join(DATA),
        dir.join(DATA),
        above.join("greaseweazle-bundle"),
    ]
    .into_iter()
    .find(|root| python_in(root).is_file())
}

/// gw's launcher as pip, pipx and uv install it.
const GW: &str = if cfg!(windows) { "gw.exe" } else { "gw" };

/// An installed `gw` on the PATH or in the usual places, which apps started
/// from a desktop do not always have on their PATH.
fn installed() -> Option<(PathBuf, bool)> {
    let path = std::env::var_os("PATH");
    let dirs = path
        .iter()
        .flat_map(std::env::split_paths)
        .chain(crate::home().map(|h| h.join(".local/bin")))
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    installed_in(dirs)
}

/// The first gw launcher in `dirs`, else the first standalone gw.
fn installed_in(dirs: impl Iterator<Item = PathBuf>) -> Option<(PathBuf, bool)> {
    let found: Vec<PathBuf> = dirs.map(|d| d.join(GW)).filter(|p| p.is_file()).collect();
    let python = found
        .iter()
        .find_map(|p| interpreter(p))
        .map(|p| (p, false));
    python.or_else(|| found.iter().find_map(|p| program(p)).map(|p| (p, true)))
}

/// The gw at `path`: the Python behind a launcher, or a standalone program.
fn gw(path: &Path) -> Option<(PathBuf, bool)> {
    match interpreter(path) {
        Some(python) => Some((python, false)),
        None => Some((program(path)?, true)),
    }
}

/// `path` if it is a program that runs as it is.
fn program(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    #[cfg(unix)]
    let runs = std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o111 != 0;
    #[cfg(not(unix))]
    let runs = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        && meta.len() > 0;
    runs.then(|| path.to_path_buf())
}

/// The Python behind a `gw` launcher, or the path itself if it names a
/// Python. A launcher is a `#!` script, or on Windows a gw.exe: a program,
/// then the `#!` line it runs, then a zip.
fn interpreter(path: &Path) -> Option<PathBuf> {
    if path.file_name()?.to_string_lossy().starts_with("python") {
        return Some(path.to_path_buf());
    }
    let exe = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    // A venv keeps its Python beside gw.exe. uv's gw.exe has no `#!` line,
    // and a moved venv's names the old path.
    let beside = path.with_file_name("python.exe");
    if exe && beside.is_file() {
        return Some(beside);
    }
    let bytes = std::fs::read(path).ok()?;
    let script = match bytes.windows(4).rposition(|w| w == b"PK\x03\x04") {
        Some(zip) if exe => {
            let line = bytes[..zip].windows(2).rposition(|w| w == b"#!")?;
            &bytes[line..zip]
        }
        _ => &bytes[..bytes.len().min(1024)],
    };
    let text = String::from_utf8_lossy(script);
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

    #[test]
    fn gw_writes_no_bytecode_beside_itself() {
        let tools = Tools {
            python: "python3".into(),
            origin: Origin::Bundled,
            standalone: false,
        };
        let cmd = tools.bridge("serve");
        let set = |(k, v): (&std::ffi::OsStr, Option<&std::ffi::OsStr>)| {
            k == "PYTHONDONTWRITEBYTECODE" && v.is_some_and(|v| v == "1")
        };
        assert!(cmd.get_envs().any(set));
    }

    #[test]
    fn the_built_in_gw_ignores_the_users_own_packages_and_python_paths() {
        // Some(None) is a variable taken away, None one left as inherited.
        let var = |origin, name: &str| {
            let cmd = Tools {
                python: "python3".into(),
                origin,
                standalone: false,
            }
            .bridge("serve");
            cmd.get_envs()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
        };
        assert_eq!(
            var(Origin::Bundled, "PYTHONNOUSERSITE"),
            Some(Some("1".into()))
        );
        assert_eq!(var(Origin::Bundled, "PYTHONPATH"), Some(None));
        assert_eq!(var(Origin::Bundled, "PYTHONHOME"), Some(None));
        for name in ["PYTHONNOUSERSITE", "PYTHONPATH", "PYTHONHOME"] {
            assert_eq!(var(Origin::Custom, name), None, "{name}");
        }
    }

    #[test]
    fn each_build_keeps_its_own_gw_updates() {
        let own = Path::new("gw").join(env!("CARGO_PKG_VERSION"));
        assert!(updates().ends_with(own), "{:?}", updates());
    }

    #[test]
    fn the_bridge_fits_on_a_windows_command_line() {
        // It goes whole on the command line, beside the Python's path, the
        // quoting and a job's arguments.
        assert!(
            loader().len() < 30_000,
            "Windows limits a command line to 32,767 characters"
        );
    }

    /// An empty folder of its own for `test`.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn launcher(test: &str, name: &str, bytes: impl AsRef<[u8]>) -> Option<PathBuf> {
        let dir = scratch(test);
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let python = interpreter(&path);
        std::fs::remove_dir_all(dir).ok();
        python
    }

    #[test]
    fn a_launcher_names_its_python() {
        let text = "#!/home/x/.local/pipx/venvs/greaseweazle/bin/python\nimport sys\n";
        let python = launcher("plain", "gw", text);
        assert_eq!(
            python,
            Some("/home/x/.local/pipx/venvs/greaseweazle/bin/python".into())
        );
    }

    #[test]
    fn a_trampoline_for_a_path_with_spaces_names_its_python() {
        let text = "#!/bin/sh\n'''exec' \"/Users/x/Library/Application Support/pipx/venvs/greaseweazle/bin/python\" \"$0\" \"$@\"\n' '''\n";
        let python = launcher("trampoline", "gw", text);
        let expected = "/Users/x/Library/Application Support/pipx/venvs/greaseweazle/bin/python";
        assert_eq!(python, Some(expected.into()));
    }

    #[test]
    fn env_shebangs_name_the_python_on_the_path() {
        assert_eq!(
            launcher("env", "gw", "#!/usr/bin/env python3\n"),
            Some("python3".into())
        );
    }

    #[test]
    fn other_scripts_are_not_engines() {
        assert_eq!(launcher("bash", "gw", "#!/bin/bash\necho hi\n"), None);
        assert_eq!(interpreter(Path::new("/nonexistent/gw")), None);
    }

    /// A gw.exe as pip writes it: a launcher, the line it runs, then a zip
    /// holding the script.
    fn pip_exe(line: &str) -> Vec<u8> {
        [
            b"MZ\x90\0the launcher".as_slice(),
            line.as_bytes(),
            b"PK\x03\x04\x14\0\0\0__main__.py",
        ]
        .concat()
    }

    #[test]
    fn a_windows_launcher_names_the_python_before_its_zip() {
        let python = r"C:\Users\x\pipx\venvs\greaseweazle\Scripts\python.exe";
        let exe = pip_exe(&format!("#!\"{python}\"\n\r\n"));
        assert_eq!(launcher("pip-exe", "gw.exe", exe), Some(python.into()));
        // Greaseweazle's own Windows gw.exe is frozen, with no Python behind it.
        assert_eq!(launcher("frozen-exe", "gw.exe", b"MZ\x90\0frozen"), None);
    }

    #[test]
    fn a_windows_launcher_in_a_venv_runs_in_the_python_beside_it() {
        let dir = scratch("venv-exe");
        let (gw, python) = (dir.join("gw.exe"), dir.join("python.exe"));
        std::fs::write(&gw, b"MZ\x90\0a launcher with no line").unwrap();
        std::fs::write(&python, "").unwrap();
        assert_eq!(interpreter(&gw), Some(python));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_installed_gw_is_found_by_the_name_pip_gives_it() {
        let dir = scratch("installed");
        let name = if cfg!(windows) { "gw.exe" } else { "gw" };
        std::fs::write(dir.join(name), "#!/venv/bin/python\n").unwrap();
        let dirs = [dir.join("elsewhere"), dir.clone()];
        assert_eq!(
            installed_in(dirs.into_iter()),
            Some(("/venv/bin/python".into(), false))
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_gw_that_is_a_program_of_its_own_runs_standalone_after_any_launcher() {
        let dir = scratch("standalone");
        let (frozen, launcher) = (dir.join("frozen"), dir.join("pip"));
        for d in [&frozen, &launcher] {
            std::fs::create_dir_all(d).unwrap();
        }
        let name = if cfg!(windows) { "gw.exe" } else { "gw" };
        let program = frozen.join(name);
        // A frozen program: no #! line and no Python beside it.
        std::fs::write(&program, b"MZ\0\0frozen").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(
            &program,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(launcher.join(name), "#!/venv/bin/python\n").unwrap();
        let dirs = || [frozen.clone(), launcher.clone()].into_iter();
        assert_eq!(
            installed_in(dirs()),
            Some(("/venv/bin/python".into(), false))
        );
        std::fs::remove_dir_all(&launcher).ok();
        assert_eq!(installed_in(dirs()), Some((program.clone(), true)));
        let chosen = Tools::find(Some(&program)).unwrap();
        assert!(chosen.standalone && chosen.origin == Origin::Custom);
        assert_eq!(
            Tools::find(Some(&dir.join("readme.txt"))),
            None,
            "not there"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_bundle_inside_a_mac_app_is_found() {
        let dir = scratch("bundle");
        let app = dir.join("Ferriteweazle.app/Contents");
        let python = python_in(&app.join("Resources").join(DATA));
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::create_dir_all(app.join("MacOS")).unwrap();
        std::fs::write(&python, "").unwrap();
        let found = python_in(&data_with(&app.join("MacOS/ferriteweazle")).unwrap());
        assert_eq!(found, python, "by name, with no ..");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_update_is_used_only_when_newer_than_the_bundled_gw_and_whole() {
        let dir = scratch("updates");
        let python = python_in(&dir.join(DATA));
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        std::fs::write(dir.join(DATA).join("greaseweazle-version"), "v1.23\n").unwrap();
        let tools = Tools {
            python,
            origin: Origin::Bundled,
            standalone: false,
        };
        assert_eq!(tools.bundled_tag().as_deref(), Some("v1.23"));
        let updates = dir.join("gw");
        for tag in ["v1.22", "v1.23", "v1.24", "v1.24.1", "v1.25.part"] {
            std::fs::create_dir_all(updates.join(tag).join("greaseweazle")).unwrap();
        }
        std::fs::create_dir_all(updates.join("v1.30")).unwrap();
        assert_eq!(tools.update_in(&updates), Some(updates.join("v1.24.1")));
        let custom = Tools {
            origin: Origin::Custom,
            ..tools
        };
        assert_eq!(
            custom.update_in(&updates),
            None,
            "only the bundled gw is updated"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_bundle_beside_the_program_is_found() {
        let dir = scratch("portable");
        let python = python_in(&dir.join(DATA));
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        let found = python_in(&data_with(&dir.join("ferriteweazle")).unwrap());
        assert_eq!(
            found.canonicalize().unwrap(),
            python.canonicalize().unwrap()
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_linux_package_keeps_gws_udev_rule_beside_its_python() {
        let dir = scratch("rule");
        let python = python_in(&dir.join(DATA));
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        let exe = dir.join("ferriteweazle");
        let rule = dir.join(DATA).join(crate::udev::RULE);
        assert_eq!(udev_rule_with(&exe), None);
        std::fs::write(&rule, "").unwrap();
        assert_eq!(udev_rule_with(&exe), Some(rule));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_python_path_is_used_as_it_is() {
        let python = Path::new("/opt/py/bin/python3.14");
        assert_eq!(interpreter(python), Some(python.to_path_buf()));
    }
}

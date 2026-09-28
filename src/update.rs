//! Newer releases on GitHub, of gw and of Ferriteweazle: looking for one and
//! installing it. The bridge talks to GitHub, in the background with a time
//! limit, so no network trouble can hold up the window.

use crate::engine::{self, Engine};
use crate::service::Repaint;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Longest a look at GitHub may take; a slow DNS lookup counts as no answer.
const CHECK_LIMIT: Duration = Duration::from_secs(20);
const INSTALL_LIMIT: Duration = Duration::from_secs(300);
pub const APP_REPO: &str = "hobbo91/ferriteweazle";

type Answer = Receiver<Result<String, String>>;

/// One product's update: what was found, and what is under way.
#[derive(Default)]
pub enum Update {
    /// Not looked for: a test window.
    #[default]
    Idle,
    Checking(Answer),
    /// The newest release, which is the one in use.
    Latest(String),
    Newer(String),
    Unreachable(String),
    Installing(Answer, String),
    Failed(String),
}

impl Update {
    /// Asks GitHub for the newest release of `repo`, gw's if none.
    pub fn check(engine: &Engine, repo: Option<&str>, repaint: Repaint) -> Update {
        let mut cmd = engine.bridge("latest");
        cmd.args(repo);
        Update::Checking(background(move || run(&mut cmd, CHECK_LIMIT), repaint))
    }

    /// Installs gw `tag` beside the built-in gw.
    pub fn gw(engine: &Engine, tag: &str, repaint: Repaint) -> Update {
        let Some(bundled) = engine.bundled_tag() else {
            return Update::Failed("The built-in gw has no version on record.".into());
        };
        let folder = engine::updates();
        let mut cmd = engine.bridge("update");
        cmd.arg(tag).arg(bundled).arg(&folder);
        let work = move || {
            std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            run(&mut cmd, INSTALL_LIMIT)
        };
        Update::Installing(background(work, repaint), tag.to_owned())
    }

    /// Downloads Ferriteweazle `tag` and puts it in place of this copy.
    pub fn app(engine: &Engine, install: Install, tag: &str, repaint: Repaint) -> Update {
        let name = install.asset(tag.trim_start_matches('v'));
        let folder = install.downloads();
        let mut cmd = engine.bridge("fetch");
        cmd.arg(tag).arg(&name).arg(&folder);
        let done = tag.to_owned();
        let work = move || {
            std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            install.replace(Path::new(&run(&mut cmd, INSTALL_LIMIT)?))?;
            Ok(done)
        };
        Update::Installing(background(work, repaint), tag.to_owned())
    }

    /// Takes the answer once there is one, against the version `in_use`,
    /// such as `1.23`. True when a release was installed.
    pub fn poll(&mut self, in_use: Option<&str>) -> bool {
        let (Update::Checking(answer) | Update::Installing(answer, _)) = self else {
            return false;
        };
        let Some(in_use) = in_use else {
            return false;
        };
        let Ok(answer) = answer.try_recv() else {
            return false;
        };
        let installing = matches!(self, Update::Installing(..));
        *self = match answer {
            Ok(tag) if installing => Update::Latest(tag),
            Ok(tag) if engine::version(&tag) > engine::version(&format!("v{in_use}")) => {
                Update::Newer(tag)
            }
            Ok(tag) => Update::Latest(tag),
            Err(why) if installing => Update::Failed(why),
            Err(why) => Update::Unreachable(why),
        };
        installing && matches!(self, Update::Latest(_))
    }

    /// Whether Update can run for `what`, and what its hover says.
    pub fn button(&self, what: &str) -> (bool, String) {
        match self {
            Update::Newer(tag) => (true, format!("Install {what} {tag} from GitHub.")),
            Update::Idle | Update::Checking(_) => (
                false,
                format!("Checking GitHub for a newer release of {what}\u{2026}"),
            ),
            Update::Latest(tag) => (
                false,
                format!("{what} {tag} is the latest release on GitHub."),
            ),
            Update::Installing(_, tag) => (false, format!("Installing {what} {tag}\u{2026}")),
            Update::Unreachable(why) | Update::Failed(why) => (false, why.clone()),
        }
    }
}

/// How this copy of Ferriteweazle was installed, and so how to replace it.
#[derive(Debug, Clone, PartialEq)]
pub enum Install {
    MacApp(PathBuf),
    /// Windows, by the MSI, which replaces the copy itself.
    Msi,
    AppImage(PathBuf),
    /// A folder holding the program and its data: the Windows zip or the
    /// Linux tarball.
    Folder(PathBuf),
}

impl Install {
    /// This copy's; none when it runs from a build folder.
    pub fn this() -> Option<Install> {
        let exe = std::env::current_exe().ok()?;
        if cfg!(target_os = "macos") {
            let app = exe.ancestors().nth(3)?;
            return (app.extension()? == "app").then(|| Install::MacApp(app.to_path_buf()));
        }
        if let Some(image) = std::env::var_os("APPIMAGE") {
            return Some(Install::AppImage(image.into()));
        }
        let dir = exe.parent()?;
        let data = dir.join(engine::DATA);
        match (data.is_dir(), data.join("msi").exists()) {
            (false, _) => None,
            (true, true) => Some(Install::Msi),
            (true, false) => Some(Install::Folder(dir.to_path_buf())),
        }
    }

    /// Why this copy cannot replace itself, if it cannot.
    pub fn stuck(&self) -> Option<&'static str> {
        let Install::MacApp(app) = self else {
            return None;
        };
        let path = app.to_string_lossy();
        (path.contains("/AppTranslocation/") || path.starts_with("/Volumes/")).then_some(
            "macOS runs Ferriteweazle read-only from the disk image: drag it to Applications first.",
        )
    }

    /// The release asset that replaces this copy.
    pub fn asset(&self, version: &str) -> String {
        let arch = std::env::consts::ARCH;
        let win = if arch == "aarch64" { "arm64" } else { "x64" };
        match self {
            Install::MacApp(_) => format!("Ferriteweazle-{version}-macos-universal.dmg"),
            Install::Msi => format!("Ferriteweazle-{version}-win-{win}.msi"),
            Install::AppImage(_) => format!("Ferriteweazle-{version}-{arch}.AppImage"),
            Install::Folder(_) if cfg!(windows) => format!("Ferriteweazle-{version}-win-{win}.zip"),
            Install::Folder(_) => format!("Ferriteweazle-{version}-linux-{arch}.tar.gz"),
        }
    }

    /// Where the download goes: beside a Windows folder, so it can be
    /// renamed into place; elsewhere, the temporary folder.
    fn downloads(&self) -> PathBuf {
        match self {
            Install::Folder(dir) if cfg!(windows) => dir.join(".ferriteweazle-update"),
            _ => std::env::temp_dir().join("ferriteweazle-update"),
        }
    }

    /// Puts `new`, as fetched, in place of this copy, asking for an
    /// administrator where this account cannot write.
    fn replace(&self, new: &Path) -> Result<(), String> {
        match self {
            Install::MacApp(app) => {
                let mount = new.with_extension("mount");
                let attach = format!(
                    "hdiutil attach -nobrowse -readonly -mountpoint {} {}",
                    quote(&mount),
                    quote(new)
                );
                shell(&attach, false)?;
                let from = mount.join("Ferriteweazle.app");
                let swap = swap_script(app, &from);
                let done = shell(&swap, false).or_else(|_| shell(&swap, true));
                let _ = shell(&format!("hdiutil detach -quiet {}", quote(&mount)), false);
                done
            }
            Install::Msi => Ok(()),
            Install::AppImage(image) => {
                let script = format!("install -m 755 {} {}", quote(new), quote(image));
                shell(&script, false).or_else(|_| shell(&script, true))
            }
            Install::Folder(dir) if cfg!(windows) => {
                let from = new.join("Ferriteweazle");
                for name in [program(), engine::DATA.into()] {
                    let (old, now) = (dir.join(format!("{name}.old")), dir.join(&name));
                    let _ = std::fs::remove_dir_all(&old).or_else(|_| std::fs::remove_file(&old));
                    // A running program can be renamed on Windows, not deleted.
                    std::fs::rename(&now, &old).map_err(|e| format!("{}: {e}", now.display()))?;
                    std::fs::rename(from.join(&name), &now).map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            Install::Folder(dir) => {
                let from = new.join("Ferriteweazle");
                let script = [program(), engine::DATA.into()]
                    .map(|name| swap_script(&dir.join(&name), &from.join(&name)))
                    .join(" && ");
                shell(&script, false).or_else(|_| shell(&script, true))
            }
        }
    }

    /// Starts the new copy, or on Windows the MSI that installs it, for the
    /// window then to close.
    pub fn relaunch(&self, version: &str) {
        let _ = match self {
            Install::MacApp(app) => Command::new("open").arg("-n").arg(app).spawn(),
            Install::Msi => Command::new("msiexec")
                .arg("/i")
                .arg(self.downloads().join(self.asset(version)))
                .arg("/passive")
                .spawn(),
            Install::AppImage(image) => Command::new(image).spawn(),
            Install::Folder(dir) => Command::new(dir.join(program())).spawn(),
        };
    }
}

/// Deletes what a Windows update left beside the program, which could not
/// go while it ran.
pub fn tidy() {
    if let Some(Install::Folder(dir)) = Install::this() {
        for name in [program(), engine::DATA.into()] {
            let old = dir.join(format!("{name}.old"));
            let _ = std::fs::remove_dir_all(&old).or_else(|_| std::fs::remove_file(&old));
        }
        let _ = std::fs::remove_dir_all(dir.join(".ferriteweazle-update"));
    }
}

fn program() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.file_name()?.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// Replaces `now` with a copy of `new`, keeping `now` until the copy is whole.
fn swap_script(now: &Path, new: &Path) -> String {
    let beside = |suffix| quote(Path::new(&format!("{}.{suffix}", now.display())));
    let (old, temp, now) = (beside("old"), beside("new"), quote(now));
    format!(
        "rm -rf {temp} {old} && cp -R {} {temp} && mv {now} {old} && mv {temp} {now} && rm -rf {old}",
        quote(new)
    )
}

/// Runs `script` in sh, as root after macOS's or polkit's password prompt if `admin`.
fn shell(script: &str, admin: bool) -> Result<(), String> {
    let mut cmd = match (admin, cfg!(target_os = "macos")) {
        (false, _) => Command::new("sh"),
        (true, true) => {
            let apple = format!(
                "do shell script \"{}\" with administrator privileges",
                script.replace('\\', "\\\\").replace('"', "\\\"")
            );
            let mut cmd = Command::new("osascript");
            cmd.arg("-e").arg(apple);
            return status(&mut cmd);
        }
        (true, false) => {
            let mut cmd = Command::new("pkexec");
            cmd.arg("sh");
            cmd
        }
    };
    cmd.arg("-c").arg(script);
    status(&mut cmd)
}

fn status(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    match out.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
    }
}

/// A path as one word for sh.
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

/// Runs `work` on a thread, then asks for a repaint.
fn background(
    work: impl FnOnce() -> Result<String, String> + Send + 'static,
    repaint: Repaint,
) -> Answer {
    let (send, answer) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(work());
        repaint();
    });
    answer
}

/// Runs `cmd`: its last line out, or on failure its last line of errors.
/// Killed after `limit`.
fn run(cmd: &mut Command, limit: Duration) -> Result<String, String> {
    let fail = |e: std::io::Error| e.to_string();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(fail)?;
    let end = Instant::now() + limit;
    while child.try_wait().map_err(fail)?.is_none() {
        if Instant::now() > end {
            let _ = child.kill();
            return Err("GitHub did not answer in time.".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let out = child.wait_with_output().map_err(fail)?;
    let last = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes);
        text.lines().last().unwrap_or_default().trim().to_owned()
    };
    match out.status.success() {
        true => Ok(last(&out.stdout)),
        false => Err(last(&out.stderr)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answered(answer: Result<&str, &str>) -> Answer {
        let (send, answer_) = mpsc::channel();
        send.send(answer.map(Into::into).map_err(Into::into))
            .unwrap();
        answer_
    }

    const GW: &str = "Greaseweazle Tools";

    #[test]
    fn a_check_finds_a_newer_release_the_latest_or_why_it_could_not_look() {
        let mut check = Update::Checking(answered(Ok("v1.24")));
        assert!(!check.poll(None), "waits for the version in use");
        assert!(!check.poll(Some("1.23")));
        assert!(matches!(&check, Update::Newer(t) if t == "v1.24"));
        assert!(check.button(GW).0);
        let mut check = Update::Checking(answered(Ok("v1.23")));
        check.poll(Some("1.23"));
        let latest = "Greaseweazle Tools v1.23 is the latest release on GitHub.";
        assert_eq!(check.button(GW), (false, latest.into()));
        let why = "hobbo91/ferriteweazle has no release on GitHub.";
        let mut check = Update::Checking(answered(Err(why)));
        check.poll(Some("0.9.0"));
        assert_eq!(check.button("Ferriteweazle"), (false, why.into()));
    }

    #[test]
    fn an_install_reports_success_or_says_why_it_failed() {
        let mut install = Update::Installing(answered(Ok("v1.24")), "v1.24".into());
        assert!(install.poll(Some("1.23")), "the new release takes over");
        let why = "gw v1.24 changes its C code or its dependencies, so it needs a new build of Ferriteweazle.";
        let mut install = Update::Installing(answered(Err(why)), "v1.24".into());
        assert!(!install.poll(Some("1.23")));
        assert_eq!(install.button(GW), (false, why.into()));
    }

    #[test]
    fn each_kind_of_install_takes_its_own_release_asset() {
        let arch = std::env::consts::ARCH;
        let mac = Install::MacApp("/Applications/Ferriteweazle.app".into());
        assert_eq!(
            mac.asset("0.9.1"),
            "Ferriteweazle-0.9.1-macos-universal.dmg"
        );
        let image = Install::AppImage("/home/u/Ferriteweazle.AppImage".into());
        assert_eq!(
            image.asset("0.9.1"),
            format!("Ferriteweazle-0.9.1-{arch}.AppImage")
        );
        let win = if arch == "aarch64" { "arm64" } else { "x64" };
        assert_eq!(
            Install::Msi.asset("0.9.1"),
            format!("Ferriteweazle-0.9.1-win-{win}.msi")
        );
    }

    #[test]
    fn a_copy_run_from_the_disk_image_cannot_replace_itself() {
        let from_image = Install::MacApp("/Volumes/Ferriteweazle 0.9.0/Ferriteweazle.app".into());
        assert!(from_image.stuck().is_some());
        let moved = Install::MacApp("/Applications/Ferriteweazle.app".into());
        assert_eq!(moved.stuck(), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_mac_app_is_replaced_from_the_disk_image() {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-dmg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (app, image) = (dir.join("Ferriteweazle.app"), dir.join("image"));
        for (folder, text) in [(&app, "0.9.0"), (&image.join("Ferriteweazle.app"), "0.9.1")] {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join("marker"), text).unwrap();
        }
        let dmg = dir.join("new.dmg");
        let make = format!(
            "hdiutil create -quiet -srcfolder {} {}",
            quote(&image),
            quote(&dmg)
        );
        shell(&make, false).unwrap();
        Install::MacApp(app.clone()).replace(&dmg).unwrap();
        assert_eq!(
            std::fs::read_to_string(app.join("marker")).unwrap(),
            "0.9.1"
        );
        assert!(!dir.join("Ferriteweazle.app.old").exists());
        assert!(!dir.join("new.mount").exists(), "the image is detached");
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_swap_leaves_the_new_copy_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (now, new) = (dir.join("it's here.app"), dir.join("new/it.app"));
        for (path, text) in [(&now, "old"), (&new, "new")] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join("marker"), text).unwrap();
        }
        shell(&swap_script(&now, &new), false).unwrap();
        assert_eq!(std::fs::read_to_string(now.join("marker")).unwrap(), "new");
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left.len(), 2, "{left:?}");
        std::fs::remove_dir_all(dir).ok();
    }
}

//! Newer releases on GitHub, of gw and of Ferriteweazle: looking for one and
//! installing it. The bridge talks to GitHub in the background, so no network
//! trouble can hold up the window.

use crate::service::Repaint;
use crate::tools::{self, Tools};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Longest a look at GitHub may take; a slow DNS lookup counts as no answer.
/// An install has no limit: the bridge gives up on a download that stalls.
const CHECK_LIMIT: Duration = Duration::from_secs(20);
/// Must match bridge.py's APP_REPO.
pub const APP_REPO: &str = "hobbo91/Ferriteweazle";

type Answer = Receiver<Result<String, String>>;

/// One product's update: what was found, and what is under way.
#[derive(Default)]
pub enum Update {
    /// Not looked for.
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
    pub fn check(tools: &Tools, repo: Option<&str>, repaint: Repaint) -> Update {
        let mut cmd = tools.bridge("latest");
        cmd.args(repo);
        Update::Checking(background(
            move || run(&mut cmd, Some(CHECK_LIMIT)),
            repaint,
        ))
    }

    /// Installs gw `tag` beside the built-in gw.
    pub fn gw(tools: &Tools, tag: &str, repaint: Repaint) -> Update {
        let Some(bundled) = tools.bundled_tag() else {
            return Update::Failed(
                "The built-in Greaseweazle Tools has no version on record.".into(),
            );
        };
        let folder = tools::updates();
        let mut cmd = tools.bridge("update");
        cmd.arg(tag).arg(bundled).arg(&folder);
        let work = move || {
            std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            run(&mut cmd, None)
        };
        Update::Installing(background(work, repaint), tag.to_owned())
    }

    /// Downloads Ferriteweazle `tag` and puts it in place of this copy.
    pub fn app(tools: &Tools, install: Install, tag: &str, repaint: Repaint) -> Update {
        let name = install.asset(bare(tag));
        let folder = install.downloads();
        let mut cmd = tools.bridge("fetch");
        cmd.arg(tag).arg(&name).arg(&folder);
        let done = tag.to_owned();
        let work = move || {
            std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            install.replace(Path::new(&run(&mut cmd, None)?))?;
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
            Ok(tag) if tools::version(&tag) > tools::version(&format!("v{in_use}")) => {
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
            Update::Idle => (false, "Not checked.".into()),
            Update::Newer(tag) => (true, format!("Install {what} {} from GitHub.", bare(tag))),
            Update::Checking(_) => (
                false,
                format!("Checking GitHub for a newer release of {what}\u{2026}"),
            ),
            Update::Latest(tag) => (
                false,
                format!("{what} {} is the latest release on GitHub.", bare(tag)),
            ),
            Update::Installing(_, tag) => {
                (false, format!("Installing {what} {}\u{2026}", bare(tag)))
            }
            Update::Unreachable(why) | Update::Failed(why) => (false, why.clone()),
        }
    }

    /// Settings' line for Ferriteweazle `now`, such as `0.9.0`, and the reason
    /// behind a failure.
    pub fn summary(&self, now: &str) -> (String, Option<String>) {
        match self {
            Update::Idle => ("Not checked.".into(), None),
            Update::Checking(_) => ("Checking GitHub for a newer release\u{2026}".into(), None),
            Update::Latest(_) => (
                format!("You are running the latest release ({now}) of Ferriteweazle."),
                None,
            ),
            Update::Newer(tag) => (format!("Update available ({now} -> {})", bare(tag)), None),
            Update::Installing(_, tag) => (format!("Installing {}\u{2026}", bare(tag)), None),
            Update::Unreachable(why) => (
                "Unable to connect to GitHub repository.".into(),
                Some(why.clone()),
            ),
            Update::Failed(why) => ("Unable to install the update.".into(), Some(why.clone())),
        }
    }
}

/// A release tag as a version: `v1.23` as `1.23`.
pub fn bare(tag: &str) -> &str {
    tag.trim_start_matches('v')
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
    /// How this copy was installed; none when it runs from a build folder.
    pub fn this() -> Option<Install> {
        let exe = std::env::current_exe().ok()?;
        if cfg!(target_os = "macos") {
            let app = exe.ancestors().nth(3)?;
            return (app.extension()? == "app").then(|| Install::MacApp(app.to_path_buf()));
        }
        let (image, appdir) = (std::env::var_os("APPIMAGE"), std::env::var_os("APPDIR"));
        if let Some(image) = appimage(&exe, image, appdir) {
            return Some(Install::AppImage(image));
        }
        let dir = exe.parent()?;
        if msi_folder().is_some_and(|f| same_folder(f, dir)) {
            return Some(Install::Msi);
        }
        dir.join(tools::DATA)
            .is_dir()
            .then(|| Install::Folder(dir.to_path_buf()))
    }

    /// Why this copy cannot replace itself, if it cannot. Tries a file in a Windows folder.
    pub fn stuck(&self) -> Option<&'static str> {
        match self {
            Install::MacApp(app) => {
                let path = app.to_string_lossy();
                // bundle.sh names the image's volume "Ferriteweazle VERSION".
                let image = path.starts_with("/Volumes/Ferriteweazle ");
                (path.contains("/AppTranslocation/") || image).then_some(
                    "macOS runs Ferriteweazle read-only from the disk image: drag it to Applications first.",
                )
            }
            // Its files are renamed by this account: there is no elevated rename.
            Install::Folder(dir) if cfg!(windows) && !writable(dir) => {
                Some("This account cannot change this folder: move Ferriteweazle to one it can.")
            }
            _ => None,
        }
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
    /// renamed into place; elsewhere, this account's data folder, since
    /// another account can own any name in a shared /tmp. tidy() removes it.
    fn downloads(&self) -> PathBuf {
        match self {
            Install::Folder(dir) if cfg!(windows) => dir.join(".ferriteweazle-update"),
            _ => crate::data_folder().join("update"),
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
                let swap = swap_script(&[(app.clone(), mount.join("Ferriteweazle.app"))]);
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
                rename_in(dir, &new.join("Ferriteweazle"), &program())
            }
            Install::Folder(dir) => {
                let from = new.join("Ferriteweazle");
                // The data first, as on Windows.
                let pairs = [tools::DATA.to_owned(), program()]
                    .map(|name| (dir.join(&name), from.join(&name)));
                shell(&swap_script(&pairs), !writable(dir))
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

/// Deletes what the last update left: the old program and data beside a
/// Windows folder, which could not go while they ran, and the download,
/// which msiexec still reads after the window closes.
pub fn tidy(install: &Install) {
    if let Install::Folder(dir) = install {
        for name in [program(), tools::DATA.into()] {
            let old = dir.join(format!("{name}.old"));
            remove(&old);
        }
    }
    let _ = std::fs::remove_dir_all(install.downloads());
}

/// Removes `path`, a folder or a file, if it is there.
fn remove(path: &Path) {
    let _ = std::fs::remove_dir_all(path).or_else(|_| std::fs::remove_file(path));
}

/// The folder the MSI installed Ferriteweazle in, as it records it in the
/// registry (ferriteweazle.wxs), read once.
fn msi_folder() -> Option<&'static Path> {
    static FOLDER: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    let read = || {
        let key = r"HKLM\SOFTWARE\Ferriteweazle";
        let reg = tools::quiet(Command::new("reg"))
            .args(["query", key, "/v", "InstallFolder"])
            .output()
            .ok()?;
        registry_text(&String::from_utf8_lossy(&reg.stdout))
    };
    FOLDER
        .get_or_init(|| read().filter(|_| cfg!(windows)).map(PathBuf::from))
        .as_deref()
}

/// A REG_SZ value's text in `reg query`'s output, spaces and all.
fn registry_text(text: &str) -> Option<String> {
    let (_, value) = text.lines().find_map(|l| l.split_once("REG_SZ"))?;
    Some(value.trim().to_owned()).filter(|v| !v.is_empty())
}

/// Whether two Windows folders are one, whatever their case or a last `\`.
fn same_folder(a: &Path, b: &Path) -> bool {
    let text = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    text(a) == text(b)
}

/// The AppImage `exe` runs from. The AppImage runtime sets APPIMAGE and
/// APPDIR, which a program started from another AppImage inherits.
fn appimage(exe: &Path, image: Option<OsString>, appdir: Option<OsString>) -> Option<PathBuf> {
    appdir.filter(|d| exe.starts_with(d))?;
    image.map(Into::into)
}

/// Puts the new data folder and `program` from `from` in place of those in
/// `dir`, keeping each old one as NAME.old: Windows renames a running program
/// but will not delete it. The data goes first, since Windows will not move it
/// while a gw runs from it; on failure the old program offers the update again.
fn rename_in(dir: &Path, from: &Path, program: &str) -> Result<(), String> {
    let names = [tools::DATA, program];
    if let Some(missing) = names.iter().map(|n| from.join(n)).find(|p| !p.exists()) {
        return Err(format!("{} is not in the download.", missing.display()));
    }
    for name in names {
        let (old, now, new) = (
            dir.join(format!("{name}.old")),
            dir.join(name),
            from.join(name),
        );
        remove(&old);
        std::fs::rename(&now, &old).map_err(|e| format!("{}: {e}", now.display()))?;
        if let Err(e) = std::fs::rename(&new, &now) {
            let _ = std::fs::rename(&old, &now);
            return Err(format!("{}: {e}", new.display()));
        }
    }
    Ok(())
}

fn program() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.file_name()?.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// Replaces each `now` with a copy of its `new`, in order. Every `now` stays
/// until all the copies are whole, and a partial copy is removed.
fn swap_script(pairs: &[(PathBuf, PathBuf)]) -> String {
    let beside = |now: &Path, suffix| quote(Path::new(&format!("{}.{suffix}", now.display())));
    let (mut copies, mut moves, mut temps, mut olds) = (vec![], vec![], vec![], vec![]);
    for (now, new) in pairs {
        let (old, temp, now) = (beside(now, "old"), beside(now, "new"), quote(now));
        copies.push(format!("cp -R {} {temp}", quote(new)));
        moves.push(format!("mv {now} {old} && mv {temp} {now}"));
        temps.push(temp);
        olds.push(old);
    }
    let (temps, olds) = (temps.join(" "), olds.join(" "));
    format!(
        "rm -rf {temps} {olds} && {{ {} || {{ rm -rf {temps}; false; }}; }} && {} && rm -rf {olds}",
        copies.join(" && "),
        moves.join(" && ")
    )
}

/// Whether this account can make files in `dir`.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".ferriteweazle-{}", std::process::id()));
    let made = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    made
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
    crate::command::quote(&path.to_string_lossy())
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
/// Killed after `limit`, if it has one.
fn run(cmd: &mut Command, limit: Option<Duration>) -> Result<String, String> {
    let fail = |e: std::io::Error| e.to_string();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(fail)?;
    if let Some(limit) = limit {
        let end = Instant::now() + limit;
        while child.try_wait().map_err(fail)?.is_none() {
            if Instant::now() > end {
                let _ = child.kill();
                let _ = child.wait();
                return Err("GitHub did not answer in time.".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
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

    #[test]
    fn the_msis_folder_is_read_from_the_registry_spaces_and_all() {
        let reg = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Ferriteweazle\r\n    \
                   InstallFolder    REG_SZ    C:\\Program Files\\Ferriteweazle\\\r\n\r\n";
        let folder = registry_text(reg).unwrap();
        assert_eq!(folder, r"C:\Program Files\Ferriteweazle\");
        let exe_dir = Path::new(r"c:\program files\Ferriteweazle");
        assert!(same_folder(Path::new(&folder), exe_dir));
        assert!(!same_folder(
            Path::new(&folder),
            Path::new(r"C:\Tools\Ferriteweazle")
        ));
        assert_eq!(
            registry_text("ERROR: The system was unable to find the key."),
            None
        );
    }

    fn answered(answer: Result<&str, &str>) -> Answer {
        let (send, answer_) = mpsc::channel();
        send.send(answer.map(Into::into).map_err(Into::into))
            .unwrap();
        answer_
    }

    const GW: &str = "Greaseweazle Tools";

    /// An empty folder of its own for `test`.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(path: impl AsRef<Path>) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn a_check_finds_a_newer_release_the_latest_or_why_it_could_not_look() {
        let mut check = Update::Checking(answered(Ok("v1.24")));
        assert!(!check.poll(None), "waits for the version in use");
        assert!(!check.poll(Some("1.23")));
        assert!(matches!(&check, Update::Newer(t) if t == "v1.24"));
        let install = "Install Greaseweazle Tools 1.24 from GitHub.";
        assert_eq!(check.button(GW), (true, install.into()));
        let mut check = Update::Checking(answered(Ok("v1.23")));
        check.poll(Some("1.23"));
        let latest = "Greaseweazle Tools 1.23 is the latest release on GitHub.";
        assert_eq!(check.button(GW), (false, latest.into()));
        let why = "hobbo91/Ferriteweazle has no release on GitHub.";
        let mut check = Update::Checking(answered(Err(why)));
        check.poll(Some("0.9.0"));
        assert_eq!(check.button("Ferriteweazle"), (false, why.into()));
    }

    #[test]
    fn a_release_not_looked_for_is_not_checked() {
        let not = "Not checked.";
        assert_eq!(Update::Idle.button("Ferriteweazle"), (false, not.into()));
        assert_eq!(Update::Idle.summary("0.9.0"), (not.into(), None));
    }

    #[test]
    fn settings_says_what_github_has_in_a_line_and_keeps_the_reason_for_hover() {
        let line = |u: Update| u.summary("0.9.0");
        assert_eq!(
            line(Update::Latest("v0.9.0".into())),
            (
                "You are running the latest release (0.9.0) of Ferriteweazle.".into(),
                None
            )
        );
        assert_eq!(
            line(Update::Newer("v0.9.1".into())).0,
            "Update available (0.9.0 -> 0.9.1)"
        );
        let why = "Could not reach GitHub.";
        assert_eq!(
            line(Update::Unreachable(why.into())),
            (
                "Unable to connect to GitHub repository.".into(),
                Some(why.into())
            )
        );
    }

    #[test]
    fn an_install_reports_success_or_says_why_it_failed() {
        let mut install = Update::Installing(answered(Ok("v1.24")), "v1.24".into());
        let installing = "Installing Greaseweazle Tools 1.24\u{2026}";
        assert_eq!(install.button(GW), (false, installing.into()));
        assert!(install.poll(Some("1.23")), "the new release takes over");
        let why = "gw 1.24 changes its C code or its dependencies, so it needs a new build of Ferriteweazle.";
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
        let folder = if cfg!(windows) {
            format!("Ferriteweazle-0.9.1-win-{win}.zip")
        } else {
            format!("Ferriteweazle-0.9.1-linux-{arch}.tar.gz")
        };
        assert_eq!(Install::Folder(PathBuf::new()).asset("0.9.1"), folder);
    }

    #[test]
    fn a_copy_run_from_the_disk_image_cannot_replace_itself() {
        for image in ["Ferriteweazle 0.9.0", "Ferriteweazle 0.9.0 1"] {
            let app = Install::MacApp(format!("/Volumes/{image}/Ferriteweazle.app").into());
            assert!(app.stuck().is_some(), "{image}");
        }
        let moved = Install::MacApp("/Applications/Ferriteweazle.app".into());
        assert_eq!(moved.stuck(), None);
        let other = Install::MacApp("/Volumes/Tools/Applications/Ferriteweazle.app".into());
        assert_eq!(other.stuck(), None, "a copy on another disk updates");
    }

    #[test]
    fn a_download_goes_to_this_accounts_own_folder() {
        for install in [
            Install::MacApp("/Applications/Ferriteweazle.app".into()),
            Install::AppImage("/opt/Ferriteweazle.AppImage".into()),
            Install::Msi,
        ] {
            let folder = install.downloads();
            assert!(folder.starts_with(crate::data_folder()), "{folder:?}");
        }
        let dir = Install::Folder("/opt/ferriteweazle".into());
        let beside = dir.downloads().starts_with("/opt/ferriteweazle");
        assert_eq!(beside, cfg!(windows), "only Windows renames it into place");
    }

    #[test]
    fn only_a_program_inside_the_appimage_takes_it_for_its_own() {
        let image = || Some(OsString::from("/home/u/Apps/Ferriteweazle.AppImage"));
        let appdir = || Some(OsString::from("/tmp/.mount_Ferrit1a2b3c"));
        let inside = Path::new("/tmp/.mount_Ferrit1a2b3c/usr/bin/ferriteweazle");
        assert_eq!(
            appimage(inside, image(), appdir()),
            Some("/home/u/Apps/Ferriteweazle.AppImage".into())
        );
        // A tarball's copy, started from a shell another AppImage opened.
        let outside = Path::new("/home/u/Ferriteweazle/ferriteweazle");
        assert_eq!(appimage(outside, image(), appdir()), None);
        assert_eq!(appimage(inside, image(), None), None);
    }

    #[test]
    fn a_folder_update_that_cannot_move_the_data_keeps_the_old_program() {
        let dir = scratch("rename");
        let from = dir.join("new");
        std::fs::create_dir_all(from.join(tools::DATA)).unwrap();
        std::fs::write(from.join("fw.exe"), "new").unwrap();
        std::fs::write(dir.join("fw.exe"), "old").unwrap();
        // With no data folder to move aside, the rename fails as Windows'
        // refusal would.
        assert!(rename_in(&dir, &from, "fw.exe").is_err());
        assert_eq!(read(dir.join("fw.exe")), "old");
        // A renamed program has no match in the download, so nothing moves.
        std::fs::create_dir_all(dir.join(tools::DATA)).unwrap();
        let why = rename_in(&dir, &from, "mine.exe").unwrap_err();
        assert!(why.contains("mine.exe"), "{why}");
        assert!(!dir.join(format!("{}.old", tools::DATA)).exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn windows_refusing_to_move_a_data_folder_leaves_this_copy_whole() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        let dir = scratch("refused");
        let from = dir.join("new");
        for (root, text) in [(&dir, "old"), (&from, "new")] {
            std::fs::create_dir_all(root.join(tools::DATA)).unwrap();
            std::fs::write(root.join(tools::DATA).join("python.exe"), text).unwrap();
            std::fs::write(root.join("fw.exe"), text).unwrap();
        }
        // A file held open, as by a gw running from it, keeps its folder in
        // place: first the old data, then the new.
        for held in [dir.join(tools::DATA), from.join(tools::DATA)] {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .open(held.join("python.exe"))
                .unwrap();
            assert!(rename_in(&dir, &from, "fw.exe").is_err(), "{held:?}");
            drop(file);
            assert_eq!(read(dir.join("fw.exe")), "old");
            assert_eq!(read(dir.join(tools::DATA).join("python.exe")), "old");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_mac_app_is_replaced_from_the_disk_image() {
        let dir = scratch("dmg");
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
        assert_eq!(read(app.join("marker")), "0.9.1");
        assert!(!dir.join("Ferriteweazle.app.old").exists());
        assert!(!dir.join("new.mount").exists(), "the image is detached");
        std::fs::remove_dir_all(dir).ok();
    }

    /// The names in `dir`, sorted.
    #[cfg(unix)]
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[cfg(unix)]
    #[test]
    fn a_swap_leaves_the_new_copy_and_nothing_else() {
        let dir = scratch("swap");
        let (now, new) = (dir.join("it's here.app"), dir.join("new/it.app"));
        for (path, text) in [(&now, "old"), (&new, "new")] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join("marker"), text).unwrap();
        }
        shell(&swap_script(&[(now.clone(), new)]), false).unwrap();
        assert_eq!(read(now.join("marker")), "new");
        assert_eq!(names(&dir), ["it's here.app", "new"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_swap_changes_nothing_unless_every_copy_is_made() {
        let dir = scratch("folder-swap");
        let (data, program, new) = (dir.join("data"), dir.join("fw"), dir.join("new"));
        for (path, text) in [(&data, "old"), (&new.join("data"), "new")] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join("marker"), text).unwrap();
        }
        std::fs::write(&program, "old").unwrap();
        let pairs = [
            (data.clone(), new.join("data")),
            (program.clone(), new.join("fw")),
        ];
        // The download lacks the program.
        assert!(shell(&swap_script(&pairs), false).is_err());
        assert_eq!(read(data.join("marker")), "old");
        assert_eq!(names(&dir), ["data", "fw", "new"], "no partial copy");
        std::fs::write(new.join("fw"), "new").unwrap();
        shell(&swap_script(&pairs), false).unwrap();
        assert_eq!(read(data.join("marker")), "new");
        assert_eq!(read(&program), "new");
        assert_eq!(names(&dir), ["data", "fw", "new"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn only_a_folder_this_account_cannot_write_needs_an_administrator() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("writable");
        assert!(writable(&dir));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        assert!(!writable(&dir));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(names(&dir), Vec::<String>::new(), "the probe is gone");
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn only_a_check_has_a_time_limit_and_one_past_it_is_reaped() {
        let dir = scratch("limit");
        let pid = dir.join("pid");
        let mut slow = Command::new("sh");
        slow.arg("-c")
            .arg(format!("echo $$ > {}; sleep 1; echo done", quote(&pid)));
        assert_eq!(run(&mut slow, None), Ok("done".into()));
        let limit = Some(Duration::from_millis(300));
        let late = "GitHub did not answer in time.";
        assert_eq!(run(&mut slow, limit), Err(late.into()));
        let ps = Command::new("ps")
            .args(["-o", "stat=", "-p", read(&pid).trim()])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&ps.stdout);
        assert_eq!(state.trim(), "", "the killed command is waited for");
        std::fs::remove_dir_all(dir).ok();
    }
}

//! A newer gw release on GitHub: looking for one, and installing it beside
//! the bundled gw. Both run the bridge in the background with a time limit,
//! so no network trouble can hold up the window.

use crate::engine::{self, Engine};
use crate::service::Repaint;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Longest a look at GitHub may take; a slow DNS lookup counts as no answer.
const CHECK_LIMIT: Duration = Duration::from_secs(20);
const INSTALL_LIMIT: Duration = Duration::from_secs(120);

type Answer = Receiver<Result<String, String>>;

#[derive(Default)]
pub enum GwUpdate {
    /// Not looked for: a test window.
    #[default]
    Idle,
    Checking(Answer),
    /// gw's newest release, which is the one in use.
    Latest(String),
    Newer(String),
    Unreachable,
    Installing(Answer, String),
    Failed(String),
}

impl GwUpdate {
    pub fn check(engine: &Engine, repaint: Repaint) -> GwUpdate {
        GwUpdate::Checking(background(engine.bridge("latest"), CHECK_LIMIT, repaint))
    }

    pub fn install(engine: &Engine, tag: &str, repaint: Repaint) -> GwUpdate {
        let Some(bundled) = engine.bundled_tag() else {
            return GwUpdate::Failed("The built-in gw has no version on record.".into());
        };
        let mut cmd = engine.bridge("update");
        cmd.arg(tag).arg(bundled).arg(engine::updates());
        if let Err(e) = std::fs::create_dir_all(engine::updates()) {
            return GwUpdate::Failed(format!("{}: {e}", engine::updates().display()));
        }
        GwUpdate::Installing(background(cmd, INSTALL_LIMIT, repaint), tag.to_owned())
    }

    /// Takes the answer once there is one, against the gw `in_use`, such as
    /// `1.23`. True when a release was installed, so gw should restart.
    pub fn poll(&mut self, in_use: Option<&str>) -> bool {
        let (GwUpdate::Checking(answer) | GwUpdate::Installing(answer, _)) = self else {
            return false;
        };
        let Some(in_use) = in_use else {
            return false;
        };
        let Ok(answer) = answer.try_recv() else {
            return false;
        };
        let installing = matches!(self, GwUpdate::Installing(..));
        *self = match answer {
            Ok(tag) if installing => GwUpdate::Latest(tag),
            Ok(tag) if engine::version(&tag) > engine::version(&format!("v{in_use}")) => {
                GwUpdate::Newer(tag)
            }
            Ok(tag) => GwUpdate::Latest(tag),
            Err(why) if installing => GwUpdate::Failed(why),
            Err(_) => GwUpdate::Unreachable,
        };
        installing && matches!(self, GwUpdate::Latest(_))
    }

    /// Whether Update can run, and what its hover says.
    pub fn button(&self) -> (bool, String) {
        match self {
            GwUpdate::Newer(tag) => (
                true,
                format!("Install Greaseweazle Tools {tag} from GitHub."),
            ),
            GwUpdate::Idle | GwUpdate::Checking(_) => (
                false,
                "Checking GitHub for a newer release of Greaseweazle Tools\u{2026}".into(),
            ),
            GwUpdate::Latest(tag) => (
                false,
                format!("Greaseweazle Tools {tag} is the latest release on GitHub."),
            ),
            GwUpdate::Unreachable => (
                false,
                "Could not reach GitHub to check for a newer release.".into(),
            ),
            GwUpdate::Installing(_, tag) => (
                false,
                format!("Installing Greaseweazle Tools {tag}\u{2026}"),
            ),
            GwUpdate::Failed(why) => (false, why.clone()),
        }
    }
}

/// Runs `cmd` on a thread: its last line out, or on failure its last line of
/// errors. Killed after `limit`.
fn background(mut cmd: Command, limit: Duration, repaint: Repaint) -> Answer {
    let (send, answer) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(finish(&mut cmd, limit));
        repaint();
    });
    answer
}

fn finish(cmd: &mut Command, limit: Duration) -> Result<String, String> {
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
            return Err("No answer in time.".into());
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

    #[test]
    fn a_check_finds_a_newer_release_the_latest_or_no_github() {
        let mut check = GwUpdate::Checking(answered(Ok("v1.24")));
        assert!(!check.poll(None), "waits for the gw in use");
        assert!(!check.poll(Some("1.23")));
        assert!(matches!(&check, GwUpdate::Newer(t) if t == "v1.24"));
        assert!(check.button().0);
        let mut check = GwUpdate::Checking(answered(Ok("v1.23")));
        check.poll(Some("1.23"));
        assert_eq!(
            check.button(),
            (
                false,
                "Greaseweazle Tools v1.23 is the latest release on GitHub.".into()
            )
        );
        let mut check = GwUpdate::Checking(answered(Err("Name or service not known")));
        check.poll(Some("1.23"));
        assert_eq!(
            check.button(),
            (
                false,
                "Could not reach GitHub to check for a newer release.".into()
            )
        );
    }

    #[test]
    fn an_install_restarts_gw_or_says_why_it_failed() {
        let mut install = GwUpdate::Installing(answered(Ok("v1.24")), "v1.24".into());
        assert!(install.poll(Some("1.23")), "gw restarts on the new release");
        let why = "gw v1.24 changes its C code or its dependencies, so it needs a new build of Ferriteweazle.";
        let mut install = GwUpdate::Installing(answered(Err(why)), "v1.24".into());
        assert!(!install.poll(Some("1.23")));
        assert_eq!(install.button(), (false, why.into()));
    }
}

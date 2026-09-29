//! Serial port access on Linux, which gw's udev rule grants: the app ships
//! it in greaseweazle and can install it through pkexec.

use crate::service::Repaint;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};

/// gw's udev rule, as greaseweazle holds it.
pub const RULE: &str = "49-greaseweazle.rules";

/// gw's instructions for Linux, at the revision these commands follow.
pub const WIKI: &str = "https://github.com/keirf/greaseweazle/wiki/Software-Installation/c575ff63bb40f37c7e75e42dd2e9858f89efbd8b";

/// Installs the rule from stdin and has udev apply it to a Greaseweazle already
/// plugged in, waiting until it has. Runs as root, from pkexec.
const INSTALL: &str = "cat >/etc/udev/rules.d/49-greaseweazle.rules \
                       && udevadm control --reload-rules && udevadm trigger && udevadm settle";

const RELOAD: &str = "sudo udevadm control --reload-rules && sudo udevadm trigger";

/// The port pyserial may not open for want of permission (EACCES), from its error:
/// `[Errno 13] could not open port /dev/ttyACM0: [Errno 13] Permission denied: '/dev/ttyACM0'`.
pub fn denied_port(error: &str) -> Option<&str> {
    let (port, why) = error
        .split_once("could not open port ")?
        .1
        .split_once(": ")?;
    why.starts_with("[Errno 13] ").then_some(port)
}

/// The commands that install the rule by hand. With no rule shipped, they
/// name the one in gw's source folder, as gw's instructions do.
pub fn commands(rule: Option<&Path>) -> [String; 2] {
    let appimage = std::env::var_os("APPDIR").map(PathBuf::from);
    commands_for(rule, appimage.as_deref())
}

/// `commands`, where `appimage` is the folder an AppImage is mounted on.
fn commands_for(rule: Option<&Path>, appimage: Option<&Path>) -> [String; 2] {
    let quote = |r: &Path| crate::command::quote(&r.to_string_lossy());
    let copy = match rule {
        None => format!("sudo cp scripts/{RULE} /etc/udev/rules.d/"),
        // Root may not read an AppImage's FUSE mount, so the shell reads it.
        Some(r) if appimage.is_some_and(|a| r.starts_with(a)) => {
            format!(
                "cat {} | sudo tee /etc/udev/rules.d/{RULE} >/dev/null",
                quote(r)
            )
        }
        Some(r) => format!("sudo cp {} /etc/udev/rules.d/", quote(r)),
    };
    [copy, RELOAD.into()]
}

/// Lines for the log: what went wrong, and the commands that put it right.
pub fn advice(port: &str, rule: Option<&Path>) -> Vec<String> {
    let [copy, reload] = commands(rule);
    vec![
        format!("No access to {port}: this account has no permission to open it."),
        "To grant access, install Greaseweazle Tools' udev rule:".into(),
        format!("  {copy}"),
        format!("  {reload}"),
        format!("See {WIKI}"),
    ]
}

/// Runs INSTALL through pkexec, which asks for an administrator's password.
/// The answer comes once the password dialog has closed and udev is done.
pub fn install(rule: &Path, repaint: Repaint) -> Receiver<Result<(), String>> {
    let rule = rule.to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(run_install(&rule));
        repaint();
    });
    rx
}

/// The rule goes to root on stdin: root may not read an AppImage's FUSE mount.
fn run_install(rule: &Path) -> Result<(), String> {
    let text =
        std::fs::read(rule).map_err(|e| format!("Could not read {}: {e}", rule.display()))?;
    let mut child = install_command()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run pkexec: {e}"))?;
    // Far smaller than a pipe holds, so this returns before the password is asked.
    let mut input = child.stdin.take().expect("stdin is piped");
    let _ = input.write_all(&text);
    drop(input);
    let out = child
        .wait_with_output()
        .map_err(|e| format!("pkexec: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(()),
        // pkexec's code for a password dialog that was dismissed.
        Some(126) => Err("Cancelled.".into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
    }
}

fn install_command() -> Command {
    let mut cmd = Command::new("pkexec");
    cmd.args(["/bin/sh", "-c", INSTALL]);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What gw prints when pyserial may not open the port.
    const REFUSED: &str = "[Errno 13] could not open port /dev/ttyACM0: \
                           [Errno 13] Permission denied: '/dev/ttyACM0'";

    #[test]
    fn a_port_refused_for_want_of_permission_is_named() {
        assert_eq!(denied_port(REFUSED), Some("/dev/ttyACM0"));
    }

    #[test]
    fn other_port_errors_are_not_permission() {
        let busy = "[Errno 16] could not open port /dev/ttyACM0: \
                    [Errno 16] Device or resource busy: '/dev/ttyACM0'";
        let windows =
            "could not open port 'COM3': PermissionError(13, 'Access is denied.', None, 5)";
        for error in [busy, windows, "Cannot find the Greaseweazle device"] {
            assert_eq!(denied_port(error), None, "{error}");
        }
    }

    #[test]
    fn the_commands_copy_the_shipped_rule_or_gws_own() {
        let shipped = Path::new("/opt/Ferriteweazle/greaseweazle/49-greaseweazle.rules");
        assert_eq!(
            commands(Some(shipped))[0],
            "sudo cp /opt/Ferriteweazle/greaseweazle/49-greaseweazle.rules /etc/udev/rules.d/"
        );
        let spaced = Path::new("/home/x/My Apps/49-greaseweazle.rules");
        if cfg!(unix) {
            assert_eq!(
                commands(Some(spaced))[0],
                "sudo cp '/home/x/My Apps/49-greaseweazle.rules' /etc/udev/rules.d/"
            );
        }
        assert_eq!(
            commands(None)[0],
            "sudo cp scripts/49-greaseweazle.rules /etc/udev/rules.d/"
        );
    }

    // The commands quote Unix paths; on Windows a path quotes differently.
    #[cfg(unix)]
    #[test]
    fn from_an_appimage_the_shell_reads_the_rule_for_sudo() {
        let mount = Path::new("/tmp/.mount_FerritcNdGCG");
        let rule = mount.join("usr/bin/greaseweazle/49-greaseweazle.rules");
        assert_eq!(
            commands_for(Some(&rule), Some(mount))[0],
            "cat /tmp/.mount_FerritcNdGCG/usr/bin/greaseweazle/49-greaseweazle.rules \
             | sudo tee /etc/udev/rules.d/49-greaseweazle.rules >/dev/null"
        );
        let tarball = Path::new("/opt/Ferriteweazle/greaseweazle/49-greaseweazle.rules");
        assert!(commands_for(Some(tarball), Some(mount))[0].starts_with("sudo cp "));
    }

    #[test]
    fn pkexec_installs_the_rule_from_its_input_then_reloads_and_triggers_udev() {
        let cmd = install_command();
        assert_eq!(cmd.get_program(), "pkexec");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, ["/bin/sh", "-c", INSTALL]);
        assert!(INSTALL.starts_with(&format!("cat >/etc/udev/rules.d/{RULE} ")));
        assert!(INSTALL.contains("&& udevadm control --reload-rules && udevadm trigger"));
    }
}

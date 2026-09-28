//! Which Greaseweazle release engine/greaseweazle.sh picks for the build, and
//! when packaging rebuilds the engine. Offline: curl and git are shell
//! functions, or the tags come from a scratch git repository.
#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// keirf/greaseweazle's tags on 2026-09-28. "latest" is the nightly build.
const UPSTREAM: &str = "latest v0.1 v0.10 v0.11 v0.12 v0.13 v0.14 v0.15 v0.16 v0.17 v0.18 \
    v0.19 v0.2 v0.20 v0.21 v0.22 v0.23 v0.24 v0.25 v0.26 v0.27 v0.28 v0.29 v0.3 v0.30 v0.31 \
    v0.32 v0.33 v0.34 v0.35 v0.36 v0.37 v0.38 v0.39 v0.4 v0.40 v0.41 v0.42 v0.5 v0.6 v0.7 \
    v0.8 v0.9 v1.0 v1.1 v1.10 v1.11 v1.12 v1.13 v1.14 v1.15.1 v1.16 v1.16.1 v1.16.2 v1.16.3 \
    v1.17 v1.17.1 v1.18 v1.19 v1.2 v1.20 v1.21 v1.22 v1.23 v1.3 v1.4 v1.5 v1.6 v1.7 v1.8 v1.9";

/// A folder with engine/greaseweazle.sh, a versions file pinning `pin` (or
/// nothing), and an engine/build.sh that logs the tag it was asked for.
fn repo(test: &str, pin: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ferriteweazle-release-{test}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let engine = dir.join("engine");
    std::fs::create_dir_all(&engine).unwrap();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/engine/greaseweazle.sh");
    std::fs::copy(script, engine.join("greaseweazle.sh")).unwrap();
    std::fs::write(engine.join("versions"), format!("GREASEWEAZLE={pin}\n")).unwrap();
    let build = engine.join("build.sh");
    std::fs::write(&build, "#!/bin/sh\necho \"$GREASEWEAZLE\" >>built.log\n").unwrap();
    std::fs::set_permissions(&build, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

/// A git repository beside `dir` with one commit carrying each of `tags`.
fn clone(dir: &Path, tags: &[&str]) -> PathBuf {
    let clone = dir.join("greaseweazle");
    std::fs::create_dir_all(&clone).unwrap();
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&clone)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["commit", "-q", "--allow-empty", "-m", "a"]);
    for tag in tags {
        git(&["tag", tag]);
    }
    clone
}

/// Runs `script` in `dir` under `set -eu` after sourcing engine/greaseweazle.sh.
fn sh(dir: &Path, env: &[(&str, &str)], stdin: &str, script: &str) -> Output {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!("set -eu; . engine/greaseweazle.sh; {script}"))
        .current_dir(dir)
        .env_remove("GREASEWEAZLE")
        .env_remove("GREASEWEAZLE_SOURCE")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sh runs");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// What `script` prints; it must succeed.
fn run(dir: &Path, env: &[(&str, &str)], stdin: &str, script: &str) -> String {
    let out = sh(dir, env, stdin, script);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{script}: {stderr}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn newest(dir: &Path, tags: &str) -> String {
    run(dir, &[], &tags.replace(' ', "\n"), "newest")
}

fn path(p: &Path) -> &str {
    p.to_str().expect("scratch paths are UTF-8")
}

/// The tags each engine/build.sh run was asked for.
fn builds(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("built.log")).unwrap_or_default()
}

#[test]
fn the_newest_release_is_found_by_version_order_not_by_text() {
    let dir = repo("newest", "");
    assert_eq!(newest(&dir, UPSTREAM), "v1.23");
    assert_eq!(newest(&dir, "v1.9 v1.100 v1.23"), "v1.100");
    assert_eq!(newest(&dir, "v1.16 v1.16.3 v1.16.1"), "v1.16.3");
    assert_eq!(newest(&dir, "v1.23 v1.24rc1 v1.24-beta latest"), "v1.23");
}

#[test]
fn the_latest_release_comes_from_github_and_else_from_its_tags() {
    let dir = repo("github", "");
    let api =
        r#"curl() { printf '{\n  "url": "x",\n  "tag_name": "v1.30",\n  "draft": false\n}\n'; }"#;
    let tags =
        r#"git() { printf 'a1\trefs/tags/latest\nb2\trefs/tags/v1.29\nc3\trefs/tags/v1.28\n'; }"#;
    assert_eq!(
        run(&dir, &[], "", &format!("{api}; {tags}; wanted")),
        "v1.30"
    );
    let down = "curl() { return 7; }";
    assert_eq!(
        run(&dir, &[], "", &format!("{down}; {tags}; wanted")),
        "v1.29"
    );
}

#[test]
fn a_local_clone_builds_its_newest_release_tag() {
    let dir = repo("clone", "");
    let clone = clone(&dir, &["latest", "v1.9", "v1.10", "v1.10.1", "v1.11rc1"]);
    let api = r#"curl() { echo '"tag_name": "v9.9"'; }"#;
    let source = [("GREASEWEAZLE_SOURCE", path(&clone))];
    assert_eq!(run(&dir, &source, "", &format!("{api}; wanted")), "v1.10.1");
}

#[test]
fn a_tag_in_versions_or_the_environment_pins_the_release() {
    let dir = repo("pin", "v1.19");
    let nowhere = dir.join("nowhere");
    let source = [("GREASEWEAZLE_SOURCE", path(&nowhere))];
    assert_eq!(run(&dir, &source, "", "wanted"), "v1.19");
    let pinned = [source[0], ("GREASEWEAZLE", "v1.20")];
    assert_eq!(run(&dir, &pinned, "", "wanted"), "v1.20");
}

#[test]
fn no_release_found_is_an_error_that_says_how_to_pin_one() {
    let dir = repo("none", "");
    let clone = clone(&dir, &["latest"]);
    let out = sh(&dir, &[("GREASEWEAZLE_SOURCE", path(&clone))], "", "wanted");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("set GREASEWEAZLE to a tag"));
}

#[test]
fn packaging_rebuilds_the_engine_only_for_another_release() {
    let dir = repo("refresh", "");
    let clone = clone(&dir, &["v1.22", "v1.23"]);
    let source = [("GREASEWEAZLE_SOURCE", path(&clone))];
    run(&dir, &source, "", "refresh");
    assert_eq!(builds(&dir), "v1.23\n", "no engine yet");

    std::fs::create_dir_all(dir.join("target/engine")).unwrap();
    std::fs::write(dir.join("target/engine/greaseweazle-version"), "v1.23\n").unwrap();
    run(&dir, &source, "", "refresh");
    assert_eq!(
        builds(&dir),
        "v1.23\n",
        "the engine holds the latest release"
    );

    std::fs::write(dir.join("target/engine/greaseweazle-version"), "v1.22\n").unwrap();
    run(&dir, &source, "", "refresh");
    assert_eq!(
        builds(&dir),
        "v1.23\nv1.23\n",
        "the engine is a release behind"
    );
}

#[test]
fn packaging_offline_keeps_the_engine_built_and_fails_without_one() {
    let dir = repo("offline", "");
    let nowhere = dir.join("nowhere");
    let source = [("GREASEWEAZLE_SOURCE", path(&nowhere))];
    assert!(!sh(&dir, &source, "", "refresh").status.success());

    std::fs::create_dir_all(dir.join("target/engine/bin")).unwrap();
    let python = dir.join("target/engine/bin/python3");
    std::fs::write(&python, "").unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    run(&dir, &source, "", "refresh");
    assert_eq!(builds(&dir), "", "nothing was rebuilt");
}

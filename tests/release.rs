//! Which Greaseweazle release engine/greaseweazle.sh picks for the build, when
//! packaging rebuilds the engine, and what engine/build.sh records. Offline:
//! curl, git and the Python download are stubs, or the tags come from a scratch
//! git repository.
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

/// The Python every scratch versions file names, as engine/build.sh records it.
const PYTHON: &str = "3.14.7+1";

/// A folder with engine/greaseweazle.sh, a versions file pinning `pin` (or
/// nothing) and naming [`PYTHON`], and an engine/build.sh that logs the tag it
/// was asked for.
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
    let versions = format!("GREASEWEAZLE={pin}\nPYTHON=3.14.7\nPYTHON_RELEASE=1\n");
    std::fs::write(engine.join("versions"), versions).unwrap();
    let build = "#!/bin/sh\necho \"$GREASEWEAZLE\" >>built.log\n";
    executable(&engine.join("build.sh"), build);
    dir
}

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A git repository beside `dir` with one commit carrying each of `tags`.
fn clone(dir: &Path, tags: &[&str]) -> PathBuf {
    clone_with(dir, tags, &[])
}

/// `clone`, its commit holding `files`, each a path and its text.
fn clone_with(dir: &Path, tags: &[&str], files: &[(&str, &str)]) -> PathBuf {
    let clone = dir.join("greaseweazle");
    for (file, text) in files {
        let path = clone.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    std::fs::create_dir_all(&clone).unwrap();
    commit(&clone);
    for tag in tags {
        git(&clone, &["tag", tag]);
    }
    clone
}

/// Makes `dir` a git repository whose one commit holds all it has.
fn commit(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "--allow-empty", "-m", "a"]);
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

/// Runs `script` in `dir` under `set -eu` after sourcing engine/greaseweazle.sh.
fn sh(dir: &Path, env: &[(&str, &str)], stdin: &str, script: &str) -> Output {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!("set -eu; . engine/greaseweazle.sh; {script}"))
        .current_dir(dir)
        .env_remove("GREASEWEAZLE")
        .env_remove("GREASEWEAZLE_SOURCE")
        .env_remove("CC")
        .env_remove("LDSHARED")
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
    std::fs::remove_dir_all(dir).ok();
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
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_local_clone_builds_its_newest_release_tag() {
    let dir = repo("clone", "");
    let clone = clone(&dir, &["latest", "v1.9", "v1.10", "v1.10.1", "v1.11rc1"]);
    let api = r#"curl() { echo '"tag_name": "v9.9"'; }"#;
    let source = [("GREASEWEAZLE_SOURCE", path(&clone))];
    assert_eq!(run(&dir, &source, "", &format!("{api}; wanted")), "v1.10.1");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_tag_in_versions_or_the_environment_pins_the_release() {
    let dir = repo("pin", "v1.19");
    let nowhere = dir.join("nowhere");
    let source = [("GREASEWEAZLE_SOURCE", path(&nowhere))];
    assert_eq!(run(&dir, &source, "", "wanted"), "v1.19");
    let pinned = [source[0], ("GREASEWEAZLE", "v1.20")];
    assert_eq!(run(&dir, &pinned, "", "wanted"), "v1.20");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn no_release_found_is_an_error_that_says_how_to_pin_one() {
    let dir = repo("none", "");
    let clone = clone(&dir, &["latest"]);
    let out = sh(&dir, &[("GREASEWEAZLE_SOURCE", path(&clone))], "", "wanted");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("set GREASEWEAZLE to a tag"));
    std::fs::remove_dir_all(dir).ok();
}

/// Records in `dir`'s engine the gw tag and Python it was built from.
fn built(dir: &Path, tag: &str, python: &str) {
    let engine = dir.join("target/engine");
    std::fs::create_dir_all(&engine).unwrap();
    std::fs::write(engine.join("python-version"), format!("{python}\n")).unwrap();
    std::fs::write(engine.join("greaseweazle-version"), format!("{tag}\n")).unwrap();
}

#[test]
fn packaging_rebuilds_the_engine_only_for_another_release_or_python() {
    let dir = repo("refresh", "");
    let clone = clone(&dir, &["v1.22", "v1.23"]);
    let source = [("GREASEWEAZLE_SOURCE", path(&clone))];
    run(&dir, &source, "", "refresh");
    assert_eq!(builds(&dir), "v1.23\n", "no engine yet");

    built(&dir, "v1.23", PYTHON);
    run(&dir, &source, "", "refresh");
    assert_eq!(
        builds(&dir),
        "v1.23\n",
        "the engine holds the latest release"
    );

    built(&dir, "v1.22", PYTHON);
    run(&dir, &source, "", "refresh");
    assert_eq!(
        builds(&dir),
        "v1.23\nv1.23\n",
        "the engine is a release behind"
    );

    built(&dir, "v1.23", "3.14.7+0");
    run(&dir, &source, "", "refresh");
    assert_eq!(
        builds(&dir),
        "v1.23\nv1.23\nv1.23\n",
        "the engine holds another Python build"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn packaging_offline_keeps_a_finished_engine_and_fails_without_one() {
    let dir = repo("offline", "");
    let nowhere = dir.join("nowhere");
    let source = [("GREASEWEAZLE_SOURCE", path(&nowhere))];
    assert!(!sh(&dir, &source, "", "refresh").status.success());

    // A build that stopped part way leaves a Python without gw.
    std::fs::create_dir_all(dir.join("target/engine/bin")).unwrap();
    executable(&dir.join("target/engine/bin/python3"), "");
    let out = sh(&dir, &source, "", "refresh");
    assert!(!out.status.success(), "a half-built engine is not kept");

    built(&dir, "v1.22", "3.14.7+0");
    run(&dir, &source, "", "refresh");
    assert_eq!(builds(&dir), "", "nothing was rebuilt");
    std::fs::remove_dir_all(dir).ok();
}

/// engine/build.sh in `dir`, and the PATH that finds its stubs: a download
/// passes its check and holds its URL, and unpacks a Python that logs its
/// arguments and the compiler it would build with.
fn stub_build(dir: &Path) -> String {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/engine/build.sh");
    std::fs::copy(script, dir.join("engine/build.sh")).unwrap();
    let versions = "GREASEWEAZLE=\nPYTHON=3.14.7\nPYTHON_RELEASE=1\n";
    std::fs::write(dir.join("engine/versions"), versions).unwrap();
    std::fs::write(dir.join("engine/python.sha256"), "").unwrap();
    let stubs = dir.join("stubs");
    std::fs::create_dir(&stubs).unwrap();
    let curl = r#"#!/bin/sh
for url; do :; done
while [ "$1" != -o ]; do shift; done
echo "$url" >"$2"
"#;
    executable(&stubs.join("curl"), curl);
    executable(&stubs.join("shasum"), "#!/bin/sh\ncat >/dev/null\n");
    let tar = r#"#!/bin/sh
while [ "$1" != -C ]; do shift; done
mkdir -p "$2/bin" "$2/lib/python3.14/site-packages"
printf '#!/bin/sh\necho "$* CC=${CC-} LDSHARED=${LDSHARED-}" >>python.log\n' >"$2/bin/python3.14"
chmod +x "$2/bin/python3.14"
ln -s python3.14 "$2/bin/python3"
"#;
    executable(&stubs.join("tar"), tar);
    format!("{}:{}", path(&stubs), std::env::var("PATH").unwrap())
}

#[test]
fn a_build_installs_the_release_wanted_and_records_it() {
    let dir = repo("build", "");
    let path = stub_build(&dir);
    let env = [("PATH", path.as_str()), ("GREASEWEAZLE", "v1.30")];
    run(&dir, &env, "", "engine/build.sh");
    let python = std::fs::read_to_string(dir.join("python.log")).unwrap();
    let pip = "git+https://github.com/keirf/greaseweazle@v1.30";
    assert!(python.contains(pip), "{python}");
    let check = "-c import greaseweazle.optimised.optimised,";
    assert!(
        python.contains(check),
        "the check loads gw's C extension: {python}"
    );
    let record = |file: &str| std::fs::read_to_string(dir.join("target/engine").join(file));
    assert_eq!(record("greaseweazle-version").unwrap(), "v1.30\n");
    assert_eq!(record("python-version").unwrap(), format!("{PYTHON}\n"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_linux_engine_holds_gws_udev_rule_from_the_release_it_builds() {
    let dir = repo("rule", "");
    let stubbed = stub_build(&dir);
    let rule = |triple: &str| {
        let engine = run(&dir, &[], "", &format!("engine_dir {triple}"));
        std::fs::read_to_string(dir.join(engine).join("49-greaseweazle.rules")).ok()
    };
    let env = [("PATH", stubbed.as_str()), ("GREASEWEAZLE", "v1.30")];
    for triple in ["aarch64-unknown-linux-gnu", "aarch64-apple-darwin"] {
        run(&dir, &env, "", &format!("engine/build.sh {triple}"));
    }
    let url =
        "https://raw.githubusercontent.com/keirf/greaseweazle/v1.30/scripts/49-greaseweazle.rules";
    assert_eq!(
        rule("aarch64-unknown-linux-gnu").as_deref(),
        Some(format!("{url}\n").as_str())
    );
    assert_eq!(rule("aarch64-apple-darwin"), None, "only Linux uses it");

    // A local clone's rule, as its tag has it.
    let text = "ATTRS{product}==\"Greaseweazle\", TAG+=\"uaccess\"\n";
    let clone = clone_with(&dir, &["v1.30"], &[("scripts/49-greaseweazle.rules", text)]);
    let source = [env[0], env[1], ("GREASEWEAZLE_SOURCE", path(&clone))];
    run(
        &dir,
        &source,
        "",
        "engine/build.sh x86_64-unknown-linux-gnu",
    );
    assert_eq!(rule("x86_64-unknown-linux-gnu").as_deref(), Some(text));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn an_engines_bytecode_is_never_checked_against_file_times() {
    let dir = repo("bytecode", "");
    let path = stub_build(&dir);
    let env = [("PATH", path.as_str()), ("GREASEWEAZLE", "v1.30")];
    run(&dir, &env, "", "engine/build.sh");
    let python = std::fs::read_to_string(dir.join("python.log")).unwrap();
    let compile = python.lines().find(|l| l.contains("compileall")).unwrap();
    assert_eq!(
        compile.split(" CC=").next().unwrap(),
        "-m compileall -q -f --invalidation-mode unchecked-hash target/engine/lib/python3.14",
        "the whole library, gw's packages and Python's own"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_linux_engine_compiles_gws_c_code_with_zig_for_glibc_2_17() {
    let dir = repo("zig", "");
    let stubbed = stub_build(&dir);
    let env = [("PATH", stubbed.as_str()), ("GREASEWEAZLE", "v1.30")];
    // The compiler pip had, as the stub Python logged it.
    let compiler = |env: &[(&str, &str)], triple: &str| {
        std::fs::remove_file(dir.join("python.log")).ok();
        run(&dir, env, "", &format!("engine/build.sh {triple}"));
        let log = std::fs::read_to_string(dir.join("python.log")).unwrap();
        let pip = log.lines().find(|l| l.contains("pip install")).unwrap();
        pip[pip.find(" CC=").unwrap() + 1..].to_owned()
    };
    for arch in ["x86_64", "aarch64"] {
        let zig = format!("zig cc -target {arch}-linux-gnu.2.17");
        assert_eq!(
            compiler(&env, &format!("{arch}-unknown-linux-gnu")),
            format!("CC={zig} LDSHARED={zig} -shared")
        );
    }
    assert_eq!(
        compiler(&env, "aarch64-apple-darwin"),
        "CC= LDSHARED=",
        "macOS uses the compiler Python was built with"
    );
    let cc = [env[0], env[1], ("CC", "gcc")];
    assert_eq!(
        compiler(&cc, "x86_64-unknown-linux-gnu"),
        "CC=gcc LDSHARED=gcc -shared",
        "a compiler named in the environment is kept"
    );
    std::fs::remove_dir_all(dir).ok();
}

/// packaging/release.sh in `dir`, committed with a clone whose newest release
/// is v1.23, and the PATH that finds its ssh stub. The macOS bundle is a stub
/// too. ssh logs each command to ssh.log, ending each with a "--" line; a
/// build on "linux" or "windows" brings back a dist folder with one file,
/// or fails with no output on the machine FAIL names.
fn stub_release(dir: &Path) -> String {
    let packaging = dir.join("packaging");
    std::fs::create_dir_all(packaging.join("macos")).unwrap();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/packaging/release.sh");
    std::fs::copy(script, packaging.join("release.sh")).unwrap();
    let machines = "LINUX_SSH=linux\nLINUX_DIR=fw\nLINUX_SETUP=true\n\
        WINDOWS_SSH=windows\nWINDOWS_DIR=C:/fw\n";
    std::fs::write(packaging.join("release.env"), machines).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nversion = \"0.9.0\"\n").unwrap();
    let mac = "#!/bin/sh\nmkdir -p dist target\n\
        echo \"$GREASEWEAZLE\" >dist/Ferriteweazle-0.9.0-macos-universal.dmg\n";
    executable(&packaging.join("macos/bundle.sh"), mac);
    let ignore = "/dist\n/target\n/stubs\n/greaseweazle\n/*.log\n";
    std::fs::write(dir.join(".gitignore"), ignore).unwrap();
    clone(dir, &["v1.22", "v1.23"]);
    commit(dir);

    let stubs = dir.join("stubs");
    std::fs::create_dir(&stubs).unwrap();
    let ssh = r#"#!/bin/sh
host=$1
shift
printf '%s\n--\n' "$*" >>ssh.log
case "$*" in *bundle.sh*) ;; *) cat >/dev/null; exit 0 ;; esac
[ "${FAIL:-}" != "$host" ] || exit 1
out=$(mktemp -d)
mkdir "$out/dist"
echo "$host" >"$out/dist/Ferriteweazle-0.9.0-$host"
tar -cf - -C "$out" dist
rm -rf "$out"
"#;
    executable(&stubs.join("ssh"), ssh);
    format!("{}:{}", path(&stubs), std::env::var("PATH").unwrap())
}

/// The build commands release.sh sent, in order.
fn remote_builds(dir: &Path) -> Vec<String> {
    let log = std::fs::read_to_string(dir.join("ssh.log")).unwrap_or_default();
    log.split("\n--\n")
        .filter(|command| command.contains("bundle.sh"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_release_builds_every_package_from_one_gw_release() {
    let dir = repo("release", "");
    let stubbed = stub_release(&dir);
    let source = dir.join("greaseweazle");
    let env = [
        ("PATH", stubbed.as_str()),
        ("GREASEWEAZLE_SOURCE", path(&source)),
    ];
    run(&dir, &env, "", "packaging/release.sh");

    let sums = std::fs::read_to_string(dir.join("dist/Ferriteweazle-0.9.0-SHA256SUMS.txt"));
    let sums = sums.unwrap();
    for file in ["macos-universal.dmg", "linux", "windows"] {
        assert!(
            sums.contains(&format!("  Ferriteweazle-0.9.0-{file}\n")),
            "{sums}"
        );
    }
    let dmg = std::fs::read_to_string(dir.join("dist/Ferriteweazle-0.9.0-macos-universal.dmg"));
    assert_eq!(dmg.unwrap(), "v1.23\n", "the Mac builds the release found");
    let builds = remote_builds(&dir);
    assert_eq!(builds.len(), 2, "{builds:?}");
    for command in &builds {
        assert!(command.contains(" export GREASEWEAZLE=v1.23 "), "{command}");
        assert!(
            !command.contains('\n'),
            "cmd.exe ends a command at a line break"
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_failed_build_on_another_machine_stops_the_release() {
    for machine in ["linux", "windows"] {
        let dir = repo(&format!("release-{machine}"), "");
        let stubbed = stub_release(&dir);
        let source = dir.join("greaseweazle");
        let env = [
            ("PATH", stubbed.as_str()),
            ("GREASEWEAZLE_SOURCE", path(&source)),
            ("FAIL", machine),
        ];
        let out = sh(&dir, &env, "", "packaging/release.sh");
        assert!(!out.status.success(), "{machine}");
        let sums = dir.join("dist/Ferriteweazle-0.9.0-SHA256SUMS.txt");
        assert!(!sums.exists(), "no sums without {machine}'s packages");
        let last = remote_builds(&dir).pop().unwrap();
        assert!(last.contains(&format!("{machine}/bundle.sh")), "{last}");
        std::fs::remove_dir_all(dir).ok();
    }
}

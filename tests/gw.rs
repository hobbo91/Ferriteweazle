//! End-to-end tests against real gw: the bundle in target/greaseweazle-bundle if
//! built, else an installed one. They skip when there is neither. None opens
//! a device.

mod common;

use common::{Window, app, app_mut, greaseweazle, line, run_button, squares};
use eframe::egui;
use egui_kittest::kittest::{NodeT, Queryable};
use ferriteweazle::command::quote;
use ferriteweazle::form::{self, Output};
use ferriteweazle::image;
use ferriteweazle::job::{DETECT, Job, Outcome};
use ferriteweazle::presets;
use ferriteweazle::progress::{Progress, Status};
use ferriteweazle::schema::{Port, Schema};
use ferriteweazle::service::{ImageAsk, Load, Service};
use ferriteweazle::tools::{Origin, Tools};
use ferriteweazle::track::{Before, Data, Facts, Header, Id, Seen, Source};
use ferriteweazle::{App, Drawer, Page, Settings};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The bundle in target/greaseweazle-bundle or the folder FERRITEWEAZLE_BUNDLE names (such
/// as another processor's, run emulated), else an installed gw. With none, each test
/// skips, unless FERRITEWEAZLE_REQUIRE_GW is set, as for a release's machines.
fn tools() -> Option<Tools> {
    let dir = std::env::var_os("FERRITEWEAZLE_BUNDLE").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("target/greaseweazle-bundle"),
        PathBuf::from,
    );
    let python = match cfg!(windows) {
        true => dir.join("python.exe"),
        false => dir.join("bin/python3"),
    };
    let found = if python.is_file() {
        Some(Tools {
            python,
            origin: Origin::Bundled,
            standalone: false,
        })
    } else {
        Tools::find(None)
    };
    if found.is_none() {
        assert!(
            std::env::var_os("FERRITEWEAZLE_REQUIRE_GW").is_none(),
            "no Greaseweazle Tools on this machine"
        );
        eprintln!("skipped: no Greaseweazle Tools on this machine");
    }
    found
}

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferriteweazle-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn wait<T>(what: &str, ready: impl FnMut() -> Option<T>) -> T {
    wait_for(what, Duration::from_secs(60), ready)
}

/// As wait, for as long as `most`.
fn wait_for<T>(what: &str, most: Duration, mut ready: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(t) = ready() {
            return t;
        }
        assert!(start.elapsed() < most, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn start(tools: &Tools, command: &str, args: &[&str]) -> Job {
    let args = args.iter().map(|a| a.to_string()).collect();
    Job::start(tools, "Greaseweazle", command, args, &[], Box::new(|| {}))
        .expect("the bridge starts")
}

fn finish(mut job: Job, what: &str) -> Job {
    // A whole disk's job, on a busy machine: a slow one takes minutes.
    wait_for(what, Duration::from_secs(300), || {
        job.poll();
        (!job.running()).then_some(())
    });
    job
}

/// Runs a conversion to the end; it must succeed.
fn run(tools: &Tools, args: &[&str]) -> Job {
    let job = finish(start(tools, "convert", args), "the job to end");
    assert_eq!(job.outcome(), Some(Outcome::Succeeded), "{:#?}", job.log);
    job
}

fn detect(tools: &Tools, image: &Path) -> Job {
    finish(start(tools, DETECT, &[&path(image)]), "detection")
}

/// A port that loses the command sent after each opening listed, as Windows
/// can, run through the bridge's handshake and gw's own.
const LOSSY_PORT: &str = r#"
import runpy, struct, sys
bridge = runpy.run_path(sys.argv[1])
bridge['steady_handshake']()
from greaseweazle import error, usb

class Port:
    def __init__(self, lost):
        self.lost, self.opens, self.timeout, self.out = lost, 0, None, b''
    baudrate = property(lambda self: 9600, lambda self, rate: None)
    def reset_output_buffer(self): pass
    def reset_input_buffer(self): self.out = b''
    def close(self): pass
    def open(self): self.opens += 1
    def write(self, cmd):
        if self.opens not in self.lost:
            info = struct.pack('<4BI4B3H14x', 1, 6, 1, 22, 72000000, 4, 1, 1, 0, 288, 224, 128)
            self.out += bytes([cmd[0], 0]) + info
    def read(self, n):
        got, self.out = self.out[:n], self.out[n:]
        assert len(got) == n or self.timeout is not None, 'gw would wait for ever'
        return got

for lost in eval(sys.argv[2]):
    port = Port(lost)
    try:
        unit = usb.Unit(port)
        print(f'firmware {unit.major}.{unit.minor} after {port.opens} openings, then waits for ever: {port.timeout is None}')
    except error.Fatal as e:
        print(e)
"#;

#[test]
fn a_command_lost_after_opening_the_port_is_sent_again() {
    let Some(tools) = tools() else { return };
    let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.py");
    let out = std::process::Command::new(&tools.python)
        .args(["-c", LOSSY_PORT])
        .arg(&bridge)
        .arg("[set(), {1}, {1, 2}, {1, 2, 3}]")
        .output()
        .expect("python runs");
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        [
            "firmware 1.6 after 1 openings, then waits for ever: True",
            "firmware 1.6 after 2 openings, then waits for ever: True",
            "firmware 1.6 after 3 openings, then waits for ever: True",
            "Greaseweazle interface did not answer.",
        ],
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The device the app drives, as Job::start names it.
    let out = std::process::Command::new(&tools.python)
        .args(["-c", LOSSY_PORT])
        .arg(&bridge)
        .arg("[{1, 2, 3}]")
        .env("FERRITEWEAZLE_DEVICE", "Adafruit RP2040")
        .output()
        .expect("python runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(text.trim(), "Adafruit RP2040 interface did not answer.");
}

#[test]
fn gw_on_the_adafruit_rp2040_does_what_its_firmware_allows_and_no_more() {
    let Some(tools) = tools() else { return };
    let dir = scratch("adafruit");
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.py");
    let out = std::process::Command::new(&tools.python)
        .arg(data.join("adafruit.py"))
        .arg(&bridge)
        .arg(dir.join("read.img"))
        .output()
        .expect("python runs");
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        [
            "info --bootloader: SwitchFwMode: Bad Command",
            "pin set 2 H before any drive: Command Failed: SetPin: Bad Command",
            "seek 79:  [79]",
            // The bridge stops it: the firmware would step to 79.
            "seek 80: The Adafruit RP2040 reaches cylinders 0 to 79, not 80. [79]",
            "seek 80 on a Greaseweazle: ",
            "rpm drive B: Command Failed: Select: No drive unit selected",
            "rpm drive 0: SLOWEST:  Rate: 72000.000 rpm ; Period: 0.833 ms",
            "pin get 26: Pin 26 is High (5v)",
            // GETPIN's refusal leaves a byte, which the next reply trips on.
            "pin get 25: Command returned garbage (00 != 06)",
            "pin set 2 H after a drive: Pin 2 is set High (5v)",
            "pin set 4 H: Command Failed: SetPin: Invalid pin",
            "delays: gw would wait for ever",
            "reset: gw would wait for ever",
            "erase: Command Failed: EraseFlux: Bad Command",
            // gw takes the write as done, but the writer is given nothing.
            "erase --hfreq: T0.0: Erasing Track",
            "hfreq flux written after the writer starts: 0",
            "ordinary flux written after the writer starts: 993",
            "read --densel H: cannot access local variable 'prev_pin2' where it is not associated with a value",
            "write --pre-erase: Command Failed: EraseFlux: Bad Command",
            "write: No tracks verified (Reason: Verify disabled)",
            "detect looks as far as cylinder 79: None (80, 0)",
        ],
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
}

mod strip {
    include!("../src/strip.rs");
}

/// Prints whether argv[1] and argv[2] are one program, statement for
/// statement and line for line, where argv[1]'s strings that stand alone
/// as statements, as docstrings do, are `pass` in argv[2].
const SAME_PROGRAM: &str = r#"
import ast, sys
def program(path, passes):
    tree = ast.parse(open(path).read())
    for node in ast.walk(tree):
        for field in ('body', 'orelse', 'finalbody'):
            body = getattr(node, field, None)
            for i, x in enumerate(body if isinstance(body, list) else []):
                alone = isinstance(x, ast.Expr) and isinstance(x.value, ast.Constant)
                if passes and alone and isinstance(x.value.value, str):
                    body[i] = ast.Pass(lineno=x.lineno, col_offset=x.col_offset)
    lines = [(type(n).__name__, n.lineno) for n in ast.walk(tree) if hasattr(n, 'lineno')]
    return ast.dump(tree), lines
print('one program' if program(sys.argv[1], True) == program(sys.argv[2], False) else 'two')
"#;

#[test]
fn the_bridge_stripped_for_the_command_line_is_the_same_program() {
    let Some(tools) = tools() else { return };
    let dir = scratch("stripped");
    let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.py");
    let stripped = dir.join("stripped.py");
    std::fs::write(&stripped, strip::strip(include_str!("../src/bridge.py"))).unwrap();
    let out = std::process::Command::new(&tools.python)
        .args(["-c", SAME_PROGRAM])
        .args([&bridge, &stripped])
        .output()
        .expect("python runs");
    let said = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        said.trim(),
        "one program",
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(dir).ok();
}

/// Prints what the bridge in argv[1] uses that gw's oldest Python, 3.8,
/// lacks: syntax 3.8 does not parse, these calls of 3.9 and 3.10, and by its
/// tokens what a newer parser takes as 3.8's: a parenthesised `with` of
/// several (3.10), and a string inside an f-string in that f-string's quote
/// (3.12).
const NEWER_PYTHON: &str = r#"
import ast, io, sys, tokenize
source = open(sys.argv[1]).read()
tree = ast.parse(source, feature_version=(3, 8))
newer = {'cache', 'get_annotations', 'removeprefix', 'removesuffix'}
used = {n.attr for n in ast.walk(tree) if isinstance(n, ast.Attribute)} & newer
tokens = list(tokenize.generate_tokens(io.StringIO(source).readline))
quote = lambda s: s.lstrip('rRbBuUfF')[:1]
quotes = []
for i, t in enumerate(tokens):
    if t.type == tokenize.NAME and t.string == 'with' and tokens[i + 1].string == '(':
        depth = 0
        for u in tokens[i + 1:]:
            depth += {'(': 1, ')': -1}.get(u.string, 0) if u.type == tokenize.OP else 0
            if depth == 0:
                break
            if depth == 1 and u.type == tokenize.NAME and u.string == 'as':
                used.add(f'with ( at line {t.start[0]}')
                break
    if t.type == getattr(tokenize, 'FSTRING_START', None):
        if quote(t.string) in quotes:
            used.add(f'f-string in an f-string at line {t.start[0]}')
        quotes.append(quote(t.string))
    elif t.type == getattr(tokenize, 'FSTRING_END', None):
        quotes.pop()
    elif t.type == tokenize.STRING and quote(t.string) in quotes:
        used.add(f'a string in the quote of its f-string at line {t.start[0]}')
print(sorted(used))
"#;

#[test]
fn the_bridge_runs_on_the_oldest_python_gw_does() {
    let Some(tools) = tools() else { return };
    let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.py");
    let out = std::process::Command::new(&tools.python)
        .args(["-c", NEWER_PYTHON])
        .arg(&bridge)
        .output()
        .expect("python runs");
    let used = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        used.trim(),
        "[]",
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Lays out a GitHub for gw in argv[1], its latest release v1.99: source
/// zips made from the installed gw, v1.98's with different C code.
const FAKE_GITHUB: &str = r#"
import json, os, sys, zipfile, greaseweazle
root, gw = sys.argv[1], os.path.dirname(greaseweazle.__file__)
os.makedirs(f'{root}/repos/keirf/greaseweazle/releases')
with open(f'{root}/repos/keirf/greaseweazle/releases/latest', 'w') as f:
    json.dump({'tag_name': 'v1.99'}, f)
tags = f'{root}/keirf/greaseweazle/archive/refs/tags'
os.makedirs(tags)
for tag, c in [('v1.23', 'same'), ('v1.99', 'same'), ('v1.98', 'changed')]:
    top = 'greaseweazle-' + tag[1:]
    with zipfile.ZipFile(f'{tags}/{tag}.zip', 'w') as z:
        z.writestr(f'{top}/setup.py', "install_requires = ['crcmod', 'pyserial']")
        z.writestr(f'{top}/src/greaseweazle/optimised/optimised.c', c)
        for folder, _, files in os.walk(gw):
            for name in files:
                rel = os.path.relpath(os.path.join(folder, name), gw)
                if name.endswith(('.py', '.cfg')) and rel != '__init__.py':
                    z.write(os.path.join(folder, name), f'{top}/src/greaseweazle/{rel}')
"#;

/// A web server for `dir` on localhost, stopped when dropped.
struct Server(std::process::Child, String);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn serve(tools: &Tools, dir: &Path) -> Server {
    let mut child = std::process::Command::new(&tools.python)
        .args([
            "-u",
            "-m",
            "http.server",
            "0",
            "--bind",
            "127.0.0.1",
            "--directory",
        ])
        .arg(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a web server");
    let mut line = String::new();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    std::io::BufRead::read_line(&mut out, &mut line).unwrap();
    // "Serving HTTP on 127.0.0.1 port 51234 (http://127.0.0.1:51234/) ..."
    let url = line.split(['(', ')']).nth(1).expect("the server's address");
    Server(child, url.trim_end_matches('/').to_owned())
}

#[test]
fn update_installs_a_newer_gw_beside_the_bundled_one_unless_its_c_code_changed() {
    let Some(tools) = tools().filter(|e| e.origin == Origin::Bundled) else {
        return;
    };
    let dir = scratch("github");
    let site = dir.join("site");
    let made = std::process::Command::new(&tools.python)
        .args(["-c", FAKE_GITHUB])
        .arg(&site)
        .status()
        .expect("python runs");
    assert!(made.success());
    let server = serve(&tools, &site);
    let bridge = |args: &[&str]| {
        let out = tools
            .bridge(args[0])
            .args(&args[1..])
            .env("FERRITEWEAZLE_GITHUB", &server.1)
            .env("FERRITEWEAZLE_GITHUB_API", &server.1)
            .output()
            .expect("the bridge runs");
        let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_owned();
        (out.status.success(), text(&out.stdout), text(&out.stderr))
    };
    assert_eq!(bridge(&["latest"]).1, "v1.99");

    let updates = dir.join("gw");
    std::fs::create_dir_all(&updates).unwrap();
    let folder = path(&updates);
    let (ok, _, why) = bridge(&["update", "v1.98", "v1.23", &folder]);
    assert!(!ok && why.contains("gw 1.98 changes its C code"), "{why}");
    let (ok, tag, why) = bridge(&["update", "v1.99", "v1.23", &folder]);
    assert!(ok, "{why}");
    assert_eq!(tag, "v1.99");

    assert_eq!(tools.update_in(&updates), Some(updates.join("v1.99")));
    let ran = std::process::Command::new(&tools.python)
        .args(["-c", "import greaseweazle.optimised as o, greaseweazle as g; print(g.__version__, o.enabled)"])
        .env("PYTHONPATH", updates.join("v1.99"))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), "1.99 True");
    std::fs::remove_dir_all(dir).ok();
}

/// Lays out Ferriteweazle 0.9.1's release in argv[1]: a tarball, and one
/// whose SHA-256 is not the one in SHA256SUMS.
const FAKE_RELEASE: &str = r#"
import hashlib, io, os, sys, tarfile
folder = f'{sys.argv[1]}/hobbo91/Ferriteweazle/releases/download/v0.9.1'
os.makedirs(folder)
def tarball(name, text):
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode='w:gz') as t:
        info = tarfile.TarInfo('Ferriteweazle/marker')
        info.size = len(text)
        t.addfile(info, io.BytesIO(text))
    with open(f'{folder}/{name}', 'wb') as f:
        f.write(data.getvalue())
    return hashlib.sha256(data.getvalue()).hexdigest()
good = tarball('good.tar.gz', b'0.9.1')
tarball('bad.tar.gz', b'tampered')
with open(f'{folder}/SHA256SUMS-0.9.1.txt', 'w') as f:
    f.write(f'{good}  good.tar.gz\n{"0" * 64}  bad.tar.gz\n')
"#;

#[test]
fn a_release_is_downloaded_checked_against_its_sums_and_unpacked() {
    let Some(tools) = tools() else { return };
    let dir = scratch("release");
    let site = dir.join("site");
    let made = std::process::Command::new(&tools.python)
        .args(["-c", FAKE_RELEASE])
        .arg(&site)
        .status()
        .expect("python runs");
    assert!(made.success());
    let server = serve(&tools, &site);
    let fetch = |name: &str| {
        let out = tools
            .bridge("fetch")
            .args(["v0.9.1", name])
            .arg(dir.join("got"))
            .env("FERRITEWEAZLE_GITHUB", &server.1)
            .output()
            .expect("the bridge runs");
        let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_owned();
        (out.status.success(), text(&out.stdout), text(&out.stderr))
    };
    std::fs::create_dir_all(dir.join("got")).unwrap();
    let (ok, unpacked, why) = fetch("good.tar.gz");
    assert!(ok, "{why}");
    let marker = Path::new(&unpacked).join("Ferriteweazle/marker");
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "0.9.1");
    let (ok, _, why) = fetch("bad.tar.gz");
    assert!(!ok && why.contains("does not match its SHA-256"), "{why}");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn github_out_of_reach_is_one_sentence_whatever_asked_it() {
    let Some(tools) = tools() else { return };
    let dir = scratch("unreachable");
    let folder = path(&dir);
    for args in [
        vec!["latest"],
        vec![
            "fetch",
            "v0.9.1",
            "Ferriteweazle-0.9.1-linux-x86_64.tar.gz",
            &folder,
        ],
        vec!["update", "v1.99", "v1.23", &folder],
    ] {
        // Nothing listens on the discard port.
        let out = tools
            .bridge(args[0])
            .args(&args[1..])
            .env("FERRITEWEAZLE_GITHUB", "http://127.0.0.1:9")
            .env("FERRITEWEAZLE_GITHUB_API", "http://127.0.0.1:9")
            .output()
            .expect("the bridge runs");
        let why = String::from_utf8_lossy(&out.stderr);
        let last = why.lines().last().unwrap_or_default();
        assert_eq!(last, "Could not reach GitHub.", "{}: {why}", args[0]);
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_service_describes_gw_and_checks_values() {
    let Some(tools) = tools() else { return };
    let mut service = Service::start(&tools, Box::new(|| {}));
    let schema = wait("the schema", || {
        service.poll();
        if let Some(e) = service.schema.error() {
            panic!("no schema: {e}");
        }
        service.schema.ready().cloned()
    });
    for name in ["read", "write", "convert", "erase", "info", "pin set"] {
        assert!(
            schema.command(name).is_some(),
            "gw {} has no {name}",
            schema.version
        );
    }
    assert!(schema.formats.iter().any(|f| f == "ibm.1440"));
    let about = |name| schema.command(name).map(|c| c.about.as_str());
    assert_eq!(
        about("pin get"),
        Some("Read the level of a user-modifiable interface pin.")
    );
    assert_eq!(
        about("pin set"),
        Some("Change the setting of a user-modifiable interface pin.")
    );
    let info = wait("format details", || {
        service.poll();
        match service.format_info("", "amiga.amigados") {
            Load::Ready(info) => Some(info.clone()),
            Load::Failed(e) => panic!("no format details: {e}"),
            Load::Waiting(_) => None,
        }
    });
    assert_eq!(
        (info.cyls, info.heads, info.sectors, info.bytes),
        (80, 2, Some((11, 11)), Some(901_120))
    );
    let complaint = wait("a check", || {
        service.poll();
        service.check("read", "revs", "0").map(str::to_owned)
    });
    assert_eq!(complaint, "must be 1 or greater");
}

#[test]
fn every_value_of_the_example_presets_is_one_gws_own_parser_takes() {
    let Some(tools) = tools() else { return };
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("presets");
    let mut asked = Vec::new();
    for entry in std::fs::read_dir(folder).unwrap().flatten() {
        let preset = presets::load(&entry.path()).unwrap();
        let values = serde_json::to_value(&preset.values).unwrap();
        for (dest, value) in values.as_object().unwrap() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let op = match dest.as_str() {
                "format" => serde_json::json!({"op": "format", "name": value}),
                _ => {
                    serde_json::json!({"op": "check", "command": preset.command, "dest": dest, "value": value})
                }
            };
            asked.push((format!("{name}: {dest}={value}"), op));
        }
    }
    let mut bridge = tools
        .bridge("serve")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the bridge starts");
    let mut input = bridge.stdin.take().unwrap();
    for (_, op) in &asked {
        std::io::Write::write_all(&mut input, format!("{op}\n").as_bytes()).unwrap();
    }
    drop(input);
    let out = bridge.wait_with_output().unwrap();
    let replies: Vec<serde_json::Value> = out
        .stdout
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice(l).unwrap())
        .collect();
    assert_eq!(replies.len(), asked.len());
    for ((what, op), reply) in asked.iter().zip(&replies) {
        let fine = match op["op"].as_str() {
            Some("format") => reply["ok"].is_object(),
            _ => reply.get("ok").is_some_and(|ok| ok.is_null()),
        };
        assert!(fine, "{what}: {reply}");
    }
}

#[test]
fn format_details_describe_the_whole_disk_not_its_first_track() {
    let Some(tools) = tools() else { return };
    let mut service = Service::start(&tools, Box::new(|| {}));
    let mut info = |name: &str| {
        let info = wait(name, || {
            service.poll();
            match service.format_info("", name) {
                Load::Ready(info) => Some(info.clone()),
                Load::Failed(e) => panic!("{name}: {e}"),
                Load::Waiting(_) => None,
            }
        });
        (info.encoding, info.sectors, info.bytes)
    };
    // FM on cylinder 0, with 10 sectors a track; MFM with 18 after.
    let flex = info("tsc.flex.dsdd");
    let both = Some("IBM FM and IBM MFM".into());
    assert_eq!(flex, (both, Some((10, 18)), Some(733_184)));
    // 21 sectors a track on the outer cylinders, 17 on the inner.
    let c64 = info("commodore.1541");
    let gcr = Some("Commodore GCR".into());
    assert_eq!(c64, (gcr, Some((17, 21)), Some(196_608)));
    // A scan's tracks have no layout until gw reads them: IBM, of any layout.
    assert_eq!(info("ibm.scan"), (Some("IBM".into()), None, None));
    assert_eq!(info("raw.250"), (Some("Raw Bitcell".into()), None, None));
    let pc = info("ibm.1440");
    assert_eq!(
        pc,
        (Some("IBM MFM".into()), Some((18, 18)), Some(1_474_560))
    );
}

#[test]
fn the_port_list_says_which_ports_linux_denies_this_account() {
    let Some(tools) = tools() else { return };
    let mut bridge = tools
        .bridge("serve")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the bridge starts");
    // Its input closes after the one question, so it answers and ends.
    let mut input = bridge.stdin.take().unwrap();
    std::io::Write::write_all(&mut input, b"{\"op\": \"ports\"}\n").unwrap();
    drop(input);
    let out = bridge.wait_with_output().unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ports: Vec<Port> = serde_json::from_value(reply["ok"].clone()).unwrap();
    // test(1) asks the kernel by access(2), as open(2) would decide.
    let may_open = |device: &str| {
        let both = r#"test -r "$1" && test -w "$1""#;
        let status = std::process::Command::new("sh")
            .args(["-c", both, "sh", device])
            .status();
        status.unwrap().success()
    };
    for port in &ports {
        let denied = cfg!(target_os = "linux") && !may_open(&port.device);
        assert_eq!(port.denied, denied, "{}", port.device);
    }
}

#[test]
fn a_packaged_engine_opens_ipf_images_with_its_own_caps_library() {
    let Some(tools) = tools() else { return };
    let root = tools.python.parent().unwrap();
    let root = if cfg!(windows) {
        root
    } else {
        root.parent().unwrap()
    };
    if !root.join("caps").is_dir() {
        eprintln!("skipped: this bundle has no SPS/CAPS library");
        return;
    }
    let dir = scratch("caps");
    let ipf = dir.join("junk.ipf");
    std::fs::write(&ipf, b"not an IPF").unwrap();
    let args = [path(&ipf), path(&dir.join("junk.hfe"))];
    let job = finish(
        start(&tools, "convert", &["convert", &args[0], &args[1]]),
        "the job to end",
    );
    let log = job.log.join("\n");
    // The library's own complaint, not gw's "Could not find SPS/CAPS library".
    assert!(log.contains("CAPS: IPF: Could not open image"), "{log}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_conversion_round_trip_is_exact_and_fully_mapped() {
    let Some(tools) = tools() else { return };
    let dir = scratch("round-trip");
    let (img, scp, back) = (dir.join("a.img"), dir.join("a.scp"), dir.join("b.img"));
    std::fs::write(
        &img,
        (0..368_640u32)
            .map(|i| (i * 7 % 251) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    run(
        &tools,
        &["convert", "--format=ibm.360", &path(&img), &path(&scp)],
    );
    let job = run(
        &tools,
        &["convert", "--format=ibm.360", &path(&scp), &path(&back)],
    );

    assert_eq!(std::fs::read(&img).unwrap(), std::fs::read(&back).unwrap());
    let p = &job.progress;
    assert_eq!((p.cyls.len(), p.heads.len()), (40, 2));
    assert!(p.tracks.values().all(|t| t.status == Status::Good));
    assert_eq!(p.total, Some((720, 720)));
    assert_eq!(p.sector_map.len(), 80);
    std::fs::remove_dir_all(dir).ok();
}

/// Formats of each of gw 1.23's codecs that lay out sectors, and their
/// variants, with an image type that holds each.
const FORMATS: [(&str, &str); 23] = [
    ("ibm.1440", ".img"),
    ("ibm.dmf", ".img"),
    ("ibm.360", ".img"),
    ("ibm.1200", ".img"),
    ("amiga.amigados", ".adf"),
    ("amiga.amigados_hd", ".adf"),
    ("commodore.1541", ".d64"),
    ("commodore.1571", ".d71"),
    ("mac.800", ".img"),
    ("mac.400", ".img"),
    ("apple2.appledos.140", ".do"),
    ("apple2.prodos.140", ".po"),
    ("hp.mmfm.9885", ".img"),
    ("hp.mmfm.9895", ".img"),
    ("northstar.fm.ss", ".nsi"),
    ("northstar.mfm.ds", ".nsi"),
    ("micropolis.100tpi.ss", ".img"),
    ("datageneral.2f", ".img"),
    ("dec.rx02", ".img"),
    ("dec.rx01", ".img"),
    ("atarist.720", ".st"),
    ("akai.800", ".img"),
    ("acorn.dfs.ss", ".ssd"),
];

/// What gw prints writing `image` through the bridge onto drive.py's
/// stand-in, which first fills `image` with random bytes in `format`, if one
/// is given.
fn stand_in_write(tools: &Tools, format: &str, image: &Path, tracks: &str) -> String {
    stand_in_write_with(tools, format, image, tracks, &[], &[])
}

/// As stand_in_write, with gw write's `options` and `env` for drive.py, such
/// as FAIL_AT.
fn stand_in_write_with(
    tools: &Tools,
    format: &str,
    image: &Path,
    tracks: &str,
    options: &[&str],
    env: &[(&str, &str)],
) -> String {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.py");
    let out = std::process::Command::new(&tools.python)
        .envs(env.iter().copied())
        .arg(data.join("drive.py"))
        .arg(&bridge)
        .arg(format)
        .arg(image)
        .arg(format!("--tracks={tracks}"))
        .args(options)
        .output()
        .expect("python runs");
    let text = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("drive.py: the stand-in drive"),
        "{format}: not the stand-in: {}",
        unreported(&text)
    );
    text.into_owned()
}

/// gw's own lines of `text`, without the bridge's reports.
fn unreported(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with("@ferriteweazle "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_write_says_where_gw_puts_each_sector_and_its_verify_finds_each_there() {
    let Some(tools) = tools() else { return };
    let dir = scratch("writes");
    for (format, ext) in FORMATS {
        let image = dir.join(format!("{}{ext}", format.replace('.', "_")));
        let text = stand_in_write(&tools, format, &image, "c=0-3");
        assert!(
            text.contains("All tracks verified"),
            "{format}: {}",
            unreported(&text)
        );
        let (mut written, mut verified) = (BTreeMap::new(), BTreeMap::new());
        for line in text
            .lines()
            .filter_map(|l| l.strip_prefix("@ferriteweazle track "))
        {
            let (key, facts) = Facts::parse(line).expect("a report");
            match facts.source {
                Some(Source::Written) => written.insert(key, facts),
                Some(Source::Verify) => verified.insert(key, facts),
                other => panic!("{format}: a report on {other:?}"),
            };
        }
        assert!(!written.is_empty(), "{format}: {}", unreported(&text));
        assert_eq!(
            written.keys().collect::<Vec<_>>(),
            verified.keys().collect::<Vec<_>>(),
            "{format}"
        );
        let checked = written.keys().map(|&k| (k, true)).collect();
        assert_eq!(verifies_said(&text), checked, "{format}");
        // The file as gw lays it out, as the bridge reported it opened.
        let source = text
            .lines()
            .filter_map(|l| l.strip_prefix("@ferriteweazle image "))
            .filter_map(|l| serde_json::from_str(l).ok())
            .find_map(|v| image::Image::parse(&v).filter(|i| i.role == image::Role::Source))
            .expect("the image the write takes its tracks from");
        let laid = source.placed(None).expect("the file laid out");
        let file = std::fs::read(&image).unwrap();
        for (key, w) in &written {
            let v = &verified[key];
            assert!(!w.sectors.is_empty(), "{format} {key:?}: no sectors");
            // The sectors written hold the file's bytes, each its part's.
            let track = laid.iter().find(|t| t.key == *key).expect("laid out");
            let mut file_parts: Vec<&[u8]> = (track.parts.iter())
                .map(|(p, at, _)| &file[*at as usize..(at + p.len) as usize])
                .collect();
            let mut sent: Vec<&[u8]> = w.sectors.iter().map(|s| &s.bytes[..]).collect();
            file_parts.sort();
            sent.sort();
            assert!(sent == file_parts, "{format} {key:?}: not the file's bytes");
            assert!(
                w.missing.is_empty() && v.missing.is_empty(),
                "{format} {key:?}"
            );
            assert_eq!(w.sectors.len(), v.sectors.len(), "{format} {key:?}");
            for (a, b) in w.sectors.iter().zip(&v.sectors) {
                assert_eq!((a.id, &a.bytes), (b.id, &b.bytes), "{format} {key:?}");
                let (a, b) = (a.at.expect("a place"), b.at.expect("a place"));
                // Within a ten-thousandth of a revolution, a few bit cells:
                // the verify read starts where it falls, and its PLL locks on.
                for (x, y) in a.iter().zip(&b) {
                    assert!((x - y).abs() < 1e-4, "{format} {key:?}: {a:?} then {b:?}");
                }
            }
            assert!(
                w.flux.is_some() && v.flux.is_some(),
                "{format} {key:?}: flux"
            );
        }
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_track_is_reported_as_written_only_once_gw_has_written_it() {
    let Some(tools) = tools() else { return };
    let dir = scratch("write-fails");
    let image = dir.join("disk.img");
    let text = stand_in_write_with(
        &tools,
        "ibm.1440",
        &image,
        "c=0-3",
        &[],
        &[("FAIL_AT", "2.0")],
    );
    assert!(
        text.contains("Command Failed: WriteFlux: Disk is Write Protected"),
        "{}",
        unreported(&text)
    );
    let sources: Vec<((u32, u32), Option<Source>)> = text
        .lines()
        .filter_map(|l| l.strip_prefix("@ferriteweazle track "))
        .map(|l| {
            let (key, facts) = Facts::parse(l).expect("a report");
            (key, facts.source)
        })
        .collect();
    let written = |key| sources.contains(&(key, Some(Source::Written)));
    assert!(written((1, 1)) && sources.contains(&((1, 1), Some(Source::Verify))));
    assert!(!written((2, 0)), "the write failed: {sources:?}");
    std::fs::remove_dir_all(dir).ok();
}

/// What the bridge's report_flux, given gw's own Flux, makes of reads gw
/// joined, of pulses that are no index, and of a hard-sectored disk's holes.
const FLUX_CASES: &str = r#"
import json, runpy, sys
bridge = runpy.run_path(sys.argv[1])
from greaseweazle.flux import Flux
bridge['joining']()
# A read from between pulses, of two revolutions and a tenth, then another
# from between pulses, of one revolution and a tenth: in ticks of 1000 a second.
a = Flux([50, 100, 100], [10] * 26, 1000, index_cued=False)
b = Flux([30, 100], [10] * 14, 1000, index_cued=False)
a.append(b)
joined = bridge['report_flux'](a)
# A last pulse of no length, with flux on past it; and a read two revolutions on past its last.
none = [bridge['report_flux'](Flux([1000, 0], [100] * 15, 1e6, index_cued=False)),
        bridge['report_flux'](Flux([100, 100], [10] * 50, 1000, index_cued=False))]
# 16 sector holes a revolution of 1600 ticks, and the index hole between two.
holes = [50, 50] + [100] * 15
raw = Flux(holes * 3, [10] * 480, 1000, index_cued=False)
args = type('Args', (), {'hard_sectors': True})()
told = bridge['indexed'](raw, args)
holes = (bridge['report_flux'](told) or {}).get('holes')
print(json.dumps({'joined': joined, 'none': none, 'index': told.index_list,
                  'raw': raw.index_list[:3], 'holes': holes}))
"#;

#[test]
fn reads_gw_joins_are_each_counted_on_their_own_and_pulses_that_are_no_index_count_nothing() {
    let Some(tools) = tools() else { return };
    let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.py");
    let mut python = std::process::Command::new(&tools.python)
        .args(["-c", FLUX_CASES])
        .arg(&bridge)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("python runs");
    // A count that never ends ends the test, not hangs it.
    let start = Instant::now();
    while python.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(30) {
            python.kill().ok();
            panic!("report_flux did not end");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = python.wait_with_output().unwrap();
    let said: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)));
    let joined = &said["joined"];
    // Three revolutions, not the joint between the reads.
    assert_eq!(
        joined["revs"],
        serde_json::json!([100.0, 100.0, 100.0]),
        "{joined}"
    );
    let passes = serde_json::json!([
        [0.5, 1.0],
        [0.0, 1.0],
        [0.0, 1.0],
        [0.0, 0.1],
        [0.7, 1.0],
        [0.0, 1.0],
        [0.0, 0.1]
    ]);
    assert_eq!(joined["passes"], passes);
    let counted: u64 = joined["bins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap())
        .sum();
    assert_eq!(counted, 40, "every transition of both reads");
    assert_eq!(said["none"], serde_json::json!([null, null]));
    // gw's own index, the sector holes left out, and the flux it was given as it was.
    let index: Vec<f64> = serde_json::from_value(said["index"].clone()).unwrap();
    assert_eq!(index, [50.0, 1600.0, 1600.0]);
    assert_eq!(said["raw"], serde_json::json!([50, 50, 100]));
    // gw takes the hole after the index's as the index: the report says so.
    assert_eq!(said["holes"], true);
    assert!(joined.get("holes").is_none());
}

#[test]
fn a_bitcell_image_converts_as_it_would_unreported_and_each_track_reports_its_flux() {
    let Some(tools) = tools() else { return };
    let dir = scratch("bitcells");
    let (img, hfe, back) = (dir.join("a.img"), dir.join("a.hfe"), dir.join("back.img"));
    let bytes: Vec<u8> = (0..368_640u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(&img, &bytes).unwrap();
    let fmt = "--format=ibm.360";
    run(&tools, &["convert", fmt, &path(&img), &path(&hfe)]);
    // An HFE's tracks are gw's master tracks, which make their flux when asked.
    let job = run(&tools, &["convert", fmt, &path(&hfe), &path(&back)]);
    assert_eq!(std::fs::read(&back).unwrap(), bytes);
    let facts = &job.progress.facts;
    assert_eq!(facts.len(), 80);
    for (key, f) in facts {
        assert!(f.flux.is_some() && f.sectors.len() == 9, "{key:?}");
    }
    std::fs::remove_dir_all(dir).ok();
}

/// tests/data/edsk.py's image, written to `dir`.
fn kinds(tools: &Tools, dir: &Path) -> PathBuf {
    let dsk = dir.join("kinds.dsk");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/edsk.py");
    let made = std::process::Command::new(&tools.python)
        .arg(script)
        .arg(&dsk)
        .status()
        .expect("python runs");
    assert!(made.success());
    dsk
}

#[test]
fn a_track_gw_passes_through_from_an_input_image_is_not_taken_for_the_formats() {
    let Some(tools) = tools() else { return };
    let dir = scratch("passed-through");
    let (dsk, imd, img) = (
        kinds(&tools, &dir),
        dir.join("kinds.imd"),
        dir.join("kinds.img"),
    );
    run(
        &tools,
        &["convert", "--format=ibm.scan", &path(&dsk), &path(&imd)],
    );
    // An IMD's track is gw's IBM track already: gw puts it in as it is, its
    // sectors R1 to R8 and R7 again, not ibm.360's R1 to R9.
    let job = run(
        &tools,
        &["convert", "--format=ibm.360", &path(&imd), &path(&img)],
    );
    let made = job.progress.made.as_ref().expect("the image's report");
    assert!(!made.tracks[&(0, 0)].laid, "{:?}", made.tracks[&(0, 0)]);
    assert_eq!(made.placed(None), None, "not as the layout names them");
    std::fs::remove_dir_all(dir).ok();
}

/// Bit cells as gw's FM and MFM decoders count them: 16 to a byte.
const BYTE: f64 = 16.0;

#[test]
fn each_kind_of_sector_an_edsk_holds_is_told_apart_where_gw_lays_it_out() {
    let Some(tools) = tools() else { return };
    let dir = scratch("kinds");
    let (dsk, imd) = (kinds(&tools, &dir), dir.join("kinds.imd"));
    let job = run(
        &tools,
        &["convert", "--format=ibm.scan", &path(&dsk), &path(&imd)],
    );
    let f = &job.progress.facts[&(0, 0)];
    let r = |s: &ferriteweazle::track::Sector| match s.id {
        Id::Ibm([0, 0, r, 2]) => r,
        id => panic!("{id:?}"),
    };
    let kinds: Vec<(u8, Header, Data, Option<u8>)> = f
        .sectors
        .iter()
        .map(|s| (r(s), s.header, s.data, s.mark))
        .collect();
    // As edsk.py lays the track out: its data counts up, so none is empty.
    assert_eq!(
        kinds,
        [
            (1, Header::Good, Data::Good, Some(0xfb)),
            (2, Header::Good, Data::Good, Some(0xf8)),
            (3, Header::Good, Data::Bad, Some(0xfb)),
            (4, Header::Good, Data::Bad, Some(0xf8)),
            (5, Header::Bad, Data::None, None),
            (6, Header::Good, Data::None, None),
            (7, Header::Good, Data::Good, Some(0xfb)),
            (7, Header::Good, Data::Good, Some(0xfb)),
            (8, Header::Good, Data::Good, Some(0xfb)),
        ]
    );
    // Where gw's EDSK reader writes each: 80 bytes of 4E after the index,
    // 12 of 00 and the index mark, 50 of 4E; then each sector after 12 of
    // 00, its data 22 of 4E and 12 of 00 after its ID field, and 40 of 4E,
    // edsk.py's gap 3, after its data. A header that has no data has only
    // the 22 after it.
    let layout = |i: usize| {
        f.sectors[i]
            .layout
            .unwrap_or_else(|| panic!("{i}: no layout"))
    };
    assert_eq!(layout(0).from_index, (80 + 12 + 4 + 50 + 12) as f64 * BYTE);
    assert_eq!(layout(0).after, Some((62.0 * BYTE, Before::IndexMark)));
    for i in [1, 2, 3, 4, 7, 8] {
        let (cells, _) = layout(i).after.unwrap();
        assert_eq!(cells, 52.0 * BYTE, "{i}: gap 3 and the 00s");
    }
    for i in [5, 6] {
        let (cells, before) = layout(i).after.unwrap();
        assert_eq!(cells, 34.0 * BYTE, "{i}: gap 2 and the 00s");
        assert!(matches!(before, Before::Header(..)), "{i}: {before:?}");
    }
    for i in [0, 1, 2, 3, 6, 7, 8] {
        assert_eq!(layout(i).id_to_data, Some(34.0 * BYTE), "{i}");
    }
    // An image's bitcells, not the disk turning: no revolutions to count.
    assert!(f.sectors.iter().all(|s| s.turns.is_none()));
    // Its flux is gw's, from 2 µs cells: every interval two, three or four
    // cells, nothing between.
    let i = f.flux.as_ref().unwrap().intervals.as_ref().unwrap();
    let at: Vec<f64> = (i.counts.iter().enumerate())
        .filter(|&(_, &n)| n > 0)
        .map(|(k, _)| (i.first as f64 + k as f64) * i.width * 1e6)
        .collect();
    let near = |us: f64, cells: f64| (us - cells).abs() < i.width * 1e6;
    let lengths = [4.0, 6.0, 8.0];
    assert!(
        at.iter().all(|&us| lengths.iter().any(|&c| near(us, c))),
        "{at:?}"
    );
    assert!(
        lengths.iter().all(|&c| at.iter().any(|&us| near(us, c))),
        "{at:?}"
    );
    assert_eq!(i.longer, 0);
    std::fs::remove_dir_all(dir).ok();
}

/// Revolution `rev` of an SCP's first track, spoilt `ms` after its index:
/// `n` intervals made one long one and n−1 of 1.5 µs; the revolution keeps
/// its length.
const DAMAGE: &str = r#"
import struct, sys
path, rev, ms, n = sys.argv[1], int(sys.argv[2]), float(sys.argv[3]), int(sys.argv[4])
d = bytearray(open(path, 'rb').read())
track = struct.unpack_from('<I', d, 16)[0]
_, count, data = struct.unpack_from('<3I', d, track + 4 + 12 * rev)
at, t, i = track + data, 0, 0
while t < ms * 40000:  # 25 ns ticks
    t += struct.unpack_from('>H', d, at + 2 * i)[0]
    i += 1
values = [struct.unpack_from('>H', d, at + 2 * (i + k))[0] for k in range(n)]
short = 60  # 1.5 us
made = [sum(values) - short * (n - 1)] + [short] * (n - 1)
for k, v in enumerate(made):
    struct.pack_into('>H', d, at + 2 * (i + k), v)
struct.pack_into('<I', d, 12, sum(d[16:]) & 0xffffffff)
open(path, 'wb').write(d)
"#;

#[test]
fn each_revolution_of_flux_gw_reads_is_counted_and_one_the_disk_spoilt_told_apart() {
    let Some(tools) = tools() else { return };
    let dir = scratch("turns");
    let (dsk, scp, imd) = (
        kinds(&tools, &dir),
        dir.join("kinds.scp"),
        dir.join("kinds.imd"),
    );
    run(&tools, &["convert", &path(&dsk), &path(&scp)]);
    // In the second of the two revolutions gw writes: R1's data, which runs
    // from 6.5 ms to 23 ms after the index, spoilt at 12; and R2's header,
    // at 24.96, its data left whole.
    for damage in [["1", "12", "10"], ["1", "24.96", "4"]] {
        let spoilt = std::process::Command::new(&tools.python)
            .args(["-c", DAMAGE])
            .arg(&scp)
            .args(damage)
            .status()
            .expect("python runs");
        assert!(spoilt.success());
    }
    let job = run(
        &tools,
        &["convert", "--format=ibm.scan", &path(&scp), &path(&imd)],
    );
    let f = &job.progress.facts[&(0, 0)];
    let turns = |i: usize| f.sectors[i].turns.clone().unwrap_or_else(|| panic!("{i}"));
    // gw keeps the good copy: the sector is good, the revolution was not.
    assert_eq!(f.sectors[0].data, Data::Good);
    assert_eq!(turns(0).seen, [Seen::Good, Seen::BadData]);
    assert_eq!(turns(0).reads, 1);
    assert_eq!(turns(1).seen, [Seen::Good, Seen::BadHeader]);
    assert_eq!(turns(2).seen, [Seen::BadData, Seen::BadData]);
    // R5's header, its CRC failing, with no data after it.
    assert_eq!(turns(4).seen, [Seen::BadHeaderAlone, Seen::BadHeaderAlone]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_track_a_conversions_input_lacks_is_reported_absent() {
    let Some(tools) = tools() else { return };
    let dir = scratch("absent");
    let (img, scp, back) = (dir.join("a.img"), dir.join("a.scp"), dir.join("back.img"));
    std::fs::write(&img, vec![0x5a; 368_640]).unwrap();
    // Flux of cylinders 0 and 1 only.
    let fmt = "--format=ibm.360";
    run(
        &tools,
        &["convert", fmt, "--tracks=c=0-1", &path(&img), &path(&scp)],
    );
    let job = run(&tools, &["convert", fmt, &path(&scp), &path(&back)]);
    let facts = &job.progress.facts;
    assert!(!facts[&(1, 1)].absent && facts[&(1, 1)].flux.is_some());
    assert!(facts[&(2, 0)].absent && facts[&(39, 1)].absent);
    assert_eq!(facts.values().filter(|f| f.absent).count(), 76);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_flux_image_written_as_it_is_is_reported_as_gw_writes_it_one_revolution_from_the_index() {
    let Some(tools) = tools() else { return };
    let dir = scratch("flux-write");
    let (img, scp) = (dir.join("a.img"), dir.join("a.scp"));
    std::fs::write(
        &img,
        (0..368_640u32)
            .map(|i| (i * 7 % 251) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    // gw's own flux of the image, two revolutions a track.
    run(
        &tools,
        &["convert", "--format=ibm.360", &path(&img), &path(&scp)],
    );
    let text = stand_in_write(&tools, "", &scp, "c=0-1");
    // Raw flux has no format to verify it by.
    assert!(text.contains("No tracks verified"), "{}", unreported(&text));
    let reports: Vec<_> = text
        .lines()
        .filter_map(|l| l.strip_prefix("@ferriteweazle track "))
        .map(|l| Facts::parse(l).expect("a report"))
        .collect();
    assert_eq!(reports.len(), 4, "{}", unreported(&text));
    for (key, facts) in reports {
        assert_eq!(facts.source, Some(Source::Image), "{key:?}");
        assert!(
            facts.sectors.is_empty() && facts.summary.is_none(),
            "{key:?}"
        );
        let spin = facts.flux.expect("its flux");
        assert_eq!(spin.revs.len(), 1, "{key:?}: the revolution gw writes");
        assert!(
            (spin.revs[0] - 0.2).abs() < 1e-3,
            "{key:?}: {:?}",
            spin.revs
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

/// What the bridge said of each track gw wrote: whether gw verifies it.
fn verifies_said(text: &str) -> BTreeMap<(u32, u32), bool> {
    let mut said = BTreeMap::new();
    for line in text
        .lines()
        .filter_map(|l| l.strip_prefix("@ferriteweazle verify "))
    {
        let v: serde_json::Value = serde_json::from_str(line).expect("a report");
        let key = (
            v["c"].as_u64().unwrap() as u32,
            v["h"].as_u64().unwrap() as u32,
        );
        assert!(
            said.insert(key, v["verifies"] == true).is_none(),
            "once a track: {line}"
        );
    }
    said
}

/// The track of gw's `T1.0: Writing Track` line, as it numbers it.
fn writing(line: &str) -> Option<(u32, u32)> {
    let (track, text) = line.strip_prefix('T')?.split_once(": ")?;
    let (c, h) = track.split(' ').next()?.split_once('.')?;
    text.starts_with("Writing Track")
        .then_some((c.parse().ok()?, h.parse().ok()?))
}

#[test]
fn gw_verifies_a_write_track_by_track_as_the_bridge_says_whatever_the_image() {
    let Some(tools) = tools() else { return };
    let dir = scratch("verifies");
    let img = dir.join("a.img");
    let bytes = (0..368_640u32).map(|i| (i * 7 % 251) as u8);
    std::fs::write(&img, bytes.collect::<Vec<_>>()).unwrap();
    // An image of each kind of track gw writes: sectors of a format, its own
    // tracks with gw's verify (as an IPF's), bitcells (as a DMK's), raw flux
    // (as a KryoFlux's or an A2R's).
    let (adf, dsk) = (dir.join("a.adf"), kinds(&tools, &dir));
    std::fs::write(&adf, vec![0x5a; 901_120]).unwrap();
    let [imd, scp, hfe] = ["a.imd", "a.scp", "a.hfe"].map(|f| dir.join(f));
    for made in [&imd, &scp, &hfe] {
        run(
            &tools,
            &["convert", "--format=ibm.360", &path(&img), &path(made)],
        );
    }
    let cases: [(&str, &Path, &[&str], bool); 8] = [
        ("an ADF, in its type's format", &adf, &[], true),
        ("an EDSK, its own tracks", &dsk, &[], true),
        ("an IMD, sectors of its own", &imd, &[], true),
        ("raw flux", &scp, &[], false),
        ("an HFE's bitcells", &hfe, &[], false),
        ("raw flux in a format", &scp, &["--format=ibm.360"], true),
        (
            "raw flux in a bitcell format",
            &scp,
            &["--format=raw.250"],
            false,
        ),
        ("an IMD with --no-verify", &imd, &["--no-verify"], false),
    ];
    for (what, image, options, verifies) in cases {
        let text = stand_in_write_with(&tools, "", image, "c=0-1", options, &[]);
        let gw = (
            text.contains("All tracks verified"),
            text.contains("No tracks verified"),
        );
        assert_eq!(gw, (verifies, !verifies), "{what}: {}", unreported(&text));
        let said = verifies_said(&text);
        assert!(said.values().all(|&v| v == verifies), "{what}: {said:?}");
        // The track as gw's verify read it back, of each track it verifies.
        let read_back: Vec<_> = (text.lines())
            .filter_map(|l| Facts::parse(l.strip_prefix("@ferriteweazle track ")?))
            .filter(|(_, f)| f.source == Some(Source::Verify) && f.flux.is_some())
            .map(|(key, _)| key)
            .collect();
        let checked: Vec<_> = said.iter().filter(|(_, v)| **v).map(|(k, _)| *k).collect();
        assert_eq!(read_back, checked, "{what}");
        // Fed as job.rs feeds it: each track good as gw goes on from it.
        let mut p = Progress::default();
        let mut so_far = BTreeMap::new();
        for line in text.lines() {
            if let Some(report) = line.strip_prefix("@ferriteweazle verify ") {
                so_far.extend(verifies_said(line));
                p.verify(report);
            } else if line.starts_with("@ferriteweazle ") {
                continue;
            } else if let Some(key) = writing(line) {
                assert!(
                    so_far.contains_key(&key),
                    "{what}: said before gw wrote {key:?}"
                );
                p.feed(line);
            } else {
                if line.ends_with(" verified") || line.contains(" verified (Reason") {
                    // gw's last line on the write: the tracks before its last.
                    let last = p.current.expect("a track written");
                    let want = if verifies {
                        Status::Good
                    } else {
                        Status::Written
                    };
                    for (key, t) in p.tracks.iter().filter(|(k, _)| **k != last) {
                        assert_eq!(t.status, want, "{what}: {key:?}");
                    }
                    assert_eq!(p.verifying(), verifies, "{what}: its last");
                }
                p.feed(line);
            }
        }
        assert_eq!(
            p.tracks.keys().collect::<Vec<_>>(),
            said.keys().collect::<Vec<_>>(),
            "{what}"
        );
        let want = if verifies {
            Status::Good
        } else {
            Status::Written
        };
        assert!(
            p.tracks.values().all(|t| t.status == want),
            "{what}: {:?}",
            p.tracks
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

/// How far a finished conversion went with its images.
fn converted(job: &Job) -> image::Job<'_> {
    image::Job {
        progress: &job.progress,
        running: false,
    }
}

/// Each part of a made image's file, where the job's report on it lays
/// it, holds what the report says: data as `want`, else gw's filler; and
/// the bytes the report gives for it.
fn holds_as_reported(job: &Job, file: &Path, want: &[u8]) -> (usize, usize, usize) {
    let made = job.progress.made.as_ref().expect("the image's report");
    let layout = made.layout.as_ref().expect("laid out");
    let written = std::fs::read(file).unwrap();
    assert_eq!(made.bytes(), Some(written.len() as u64), "the file's size");
    let (mut data, mut filler, mut unread) = (0, 0, 0);
    for track in made.placed(Some(&converted(job))).expect("as written") {
        for (k, (part, at, state)) in track.parts.iter().enumerate() {
            let (at, len) = (*at as usize, part.len as usize);
            let got = &written[at..at + len];
            let reported = made.part_bytes(&track, k);
            assert_eq!(reported.as_deref(), Some(got), "{:?} {:?}", track.key, part);
            match state {
                image::State::Data => {
                    data += 1;
                    assert_eq!(got, &want[at..at + len], "{:?} {:?}", track.key, part);
                }
                image::State::Filler | image::State::Unread => {
                    match state {
                        image::State::Filler => filler += 1,
                        _ => unread += 1,
                    }
                    assert_eq!(
                        got, layout.fillers[part.filler],
                        "{:?} {:?}",
                        track.key, part
                    );
                }
                other => panic!("{other:?} in a made image"),
            }
        }
    }
    (data, filler, unread)
}

#[test]
fn an_image_gw_makes_holds_each_sector_where_the_report_lays_it_as_data_or_filler() {
    let Some(tools) = tools() else { return };
    let dir = scratch("image-layout");
    let mut service = Service::start(&tools, Box::new(|| {}));
    // As many bytes as gw lays out for each format.
    let sizes: Vec<u64> = FORMATS
        .iter()
        .map(|(format, _)| {
            wait(format, || {
                service.poll();
                match service.format_info("", format) {
                    Load::Ready(info) => Some(info.bytes.expect("a layout")),
                    Load::Failed(e) => panic!("{format}: {e}"),
                    Load::Waiting(_) => None,
                }
            })
        })
        .collect();
    // Each format in a gw of its own, as many at once as the machine has cores.
    let formats = std::sync::Mutex::new(FORMATS.iter().zip(sizes));
    // Half the cores: the other tests run beside it.
    let cores = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).max(1));
    std::thread::scope(|scope| {
        for _ in 0..cores.min(FORMATS.len()) {
            let (tools, dir, formats) = (&tools, &dir, &formats);
            scope.spawn(move || {
                loop {
                    let next = formats.lock().unwrap_or_else(|e| e.into_inner()).next();
                    let Some((&(format, ext), size)) = next else {
                        break;
                    };
                    let name = format.replace('.', "_");
                    let img = dir.join(format!("{name}{ext}"));
                    let (scp, back) = (
                        dir.join(format!("{name}.scp")),
                        dir.join(format!("{name}-back{ext}")),
                    );
                    let bytes: Vec<u8> = (0..size as u32).map(|i| (i * 7 % 251) as u8).collect();
                    std::fs::write(&img, &bytes).unwrap();
                    let fmt = format!("--format={format}");
                    // The image gw takes its tracks from, checked against its file,
                    // and each part's bytes as the file holds them.
                    let job = run(tools, &["convert", &fmt, &path(&img), &path(&scp)]);
                    let source = job.progress.source.as_ref().expect("the source's report");
                    assert!(source.layout.is_some(), "{format}: laid out");
                    assert_eq!(source.size, Some(size), "{format}");
                    let mut padded = bytes.clone();
                    for track in source.placed(Some(&converted(&job))).expect("laid out") {
                        for (k, (part, at, state)) in track.parts.iter().enumerate() {
                            let (at, len) = (*at as usize, part.len as usize);
                            padded.resize(padded.len().max(at + len), 0);
                            let reported = source.part_bytes(&track, k);
                            assert_eq!(
                                reported.as_deref(),
                                Some(&padded[at..at + len]),
                                "{format}"
                            );
                            assert!(
                                matches!(state, image::State::Data | image::State::PastEnd),
                                "{format}: {:?} {state:?}",
                                track.key
                            );
                        }
                    }
                    // Back again: every sector's data where it was, past the
                    // source's end gw's zeros.
                    let job = run(tools, &["convert", &fmt, &path(&scp), &path(&back)]);
                    let made = std::fs::read(&back).unwrap();
                    let mut want = bytes.clone();
                    want.resize(made.len().max(want.len()), 0);
                    let (data, filler, unread) = holds_as_reported(&job, &back, &want);
                    assert!(
                        data > 0 && filler == 0 && unread == 0,
                        "{format}: {data} {filler} {unread}"
                    );
                }
            });
        }
    });
    // Decoded as a format the flux is not: gw's filler throughout.
    let (scp, wrong) = (dir.join("ibm_1440.scp"), dir.join("wrong.img"));
    let job = run(
        &tools,
        &["convert", "--format=ibm.720", &path(&scp), &path(&wrong)],
    );
    let (data, filler, unread) = holds_as_reported(&job, &wrong, &[]);
    assert_eq!((data, filler, unread), (0, 1440, 0));
    // Two cylinders converted: gw's filler for the rest, which it did not read.
    let part = dir.join("part.img");
    let args = [
        "convert",
        "--format=ibm.1440",
        "--tracks=c=0-1",
        &path(&scp),
        &path(&part),
    ];
    let job = run(&tools, &args);
    let want: Vec<u8> = (0..1_474_560u32).map(|i| (i * 7 % 251) as u8).collect();
    let (data, filler, unread) = holds_as_reported(&job, &part, &want);
    assert_eq!((data, filler, unread), (72, 0, 2808));
    // Every other cylinder of the input, which gw's track lines name as
    // their own, half as far in.
    let (img, stepped) = (dir.join("ibm_1440.img"), dir.join("stepped.scp"));
    let args = [
        "convert",
        "--format=ibm.1440",
        "--tracks=c=0-39:step=2",
        &path(&img),
        &path(&stepped),
    ];
    let job = run(&tools, &args);
    let route = image::Route {
        own: (5, 1),
        from: Some((10, 1)),
        to: Some((5, 1)),
    };
    let routes = job
        .progress
        .routes
        .as_ref()
        .expect("the conversion's routes");
    assert_eq!(routes.len(), 80);
    assert!(routes.contains(&route), "{routes:?}");
    let job_of = converted(&job);
    assert_eq!(job_of.named(image::Role::Source, (10, 1)), [(5, 1)]);
    assert!(job_of.named(image::Role::Source, (11, 1)).is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn before_a_job_gw_opens_the_image_a_write_or_a_conversion_is_to_take_its_tracks_from() {
    let Some(tools) = tools() else { return };
    let dir = scratch("image-before");
    let (adf, scp) = (dir.join("Disk.adf"), dir.join("Disk.scp"));
    let mut bytes: Vec<u8> = (0..901_120u32).map(|i| (i * 7 % 251) as u8).collect();
    // gw's filler where the file's track 0.1 has its sector 3.
    let at = 11 * 512 + 3 * 512;
    bytes[at..at + 512].copy_from_slice(&b"-=[BAD SECTOR]=-".repeat(32));
    std::fs::write(&adf, &bytes).unwrap();
    let mut service = Service::start(&tools, Box::new(|| {}));
    let mut opened = |args: &[&str], file: &Path| {
        let ask = ImageAsk {
            args: args.iter().map(|a| a.to_string()).collect(),
            path: path(file),
            diskdefs: String::new(),
        };
        wait("the image", || {
            service.poll();
            match service.image(&ask) {
                Load::Ready(p) => Some(Ok(p.0.clone())),
                Load::Failed(e) => Some(Err(e.clone())),
                Load::Waiting(_) => None,
            }
        })
    };
    let states = |image: &image::Image| {
        let mut counts = BTreeMap::new();
        for track in image.placed(None).expect("laid out") {
            for (_, _, state) in track.parts {
                *counts.entry(format!("{state:?}")).or_insert(0) += 1;
            }
        }
        counts
    };
    // A write's: gw's filler as the file holds it, the rest its data.
    let write = opened(&["write", &path(&adf)], &adf).unwrap();
    assert_eq!(write.role, image::Role::Source);
    assert_eq!((write.kind.as_str(), write.size), ("ADF", Some(901_120)));
    assert_eq!(write.content.as_deref(), Some(&bytes[..]));
    let counts = states(&write);
    assert_eq!((counts["Data"], counts["Filler"]), (1759, 1), "{counts:?}");
    // As another format lays it out: past the file's end, gw's zeros.
    let wide = opened(&["write", "--format=ibm.1440", &path(&adf)], &adf).unwrap();
    assert_eq!(states(&wide)["PastEnd"], (1_474_560 - 901_120) / 512);
    // A file longer than its format lays out: the rest is gw's to leave, and
    // only what gw reads comes over.
    let big = dir.join("Big.img");
    std::fs::write(&big, vec![0x5a; 1_474_560]).unwrap();
    let long = opened(&["write", "--format=amiga.amigados", &path(&big)], &big).unwrap();
    assert_eq!((long.size, long.unread()), (Some(1_474_560), Some(573_440)));
    assert_eq!(long.content.as_ref().map(Vec::len), Some(901_120));
    // A conversion's input, its output not made.
    let input = opened(&["convert", &path(&adf), &path(&scp)], &adf).unwrap();
    assert!(input.layout.is_some() && !scp.exists());
    // With no output named, gw's parser wants one; given an SCP's name, as
    // the page gives gw then, the input opens.
    let none = opened(&["convert", &path(&adf)], &adf).unwrap_err();
    assert!(none.contains("out_file"), "{none}");
    let alone = opened(&["convert", &path(&adf), "out.scp"], &adf).unwrap();
    assert!(alone.layout.is_some());
    // An SCP's type names no format, as an ADF's does: an IMG, which names
    // none of its own, opens only with one.
    let img = dir.join("Disk.img");
    std::fs::write(&img, &bytes).unwrap();
    let unnamed = opened(&["convert", &path(&img), "out.scp"], &img).unwrap_err();
    assert!(unnamed.contains("requires a disk format"), "{unnamed}");
    let amiga = opened(&["convert", &path(&img), "out.adf"], &img).unwrap();
    assert!(amiga.layout.is_some());
    // gw's own words where it cannot.
    let wrong = opened(&["write", "--format=no.such", &path(&adf)], &adf).unwrap_err();
    assert!(wrong.starts_with("Unknown format 'no.such'"), "{wrong}");
    std::fs::remove_dir_all(dir).ok();
}

/// A standalone gw: FERRITEWEAZLE_STANDALONE_GW, such as the gw.exe of gw's
/// Windows download, else on Unix the bundle's gw run by a script of its own,
/// as the frozen gw.exe runs it in its own Python.
fn standalone(tools: &Tools, dir: &Path) -> Option<Tools> {
    if let Some(gw) = std::env::var_os("FERRITEWEAZLE_STANDALONE_GW") {
        return Tools::find(Some(Path::new(&gw))).filter(|e| e.standalone);
    }
    script_gw(tools, dir)
}

#[cfg(not(unix))]
fn script_gw(_: &Tools, _: &Path) -> Option<Tools> {
    None
}

#[cfg(unix)]
fn script_gw(tools: &Tools, dir: &Path) -> Option<Tools> {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("gw");
    let gw = "import sys; from greaseweazle import cli; sys.argv[0] = 'gw'; sys.exit(cli.main())";
    let text = format!(
        "#!/bin/sh\nexec '{}' -c \"{gw}\" \"$@\"\n",
        path(&tools.python)
    );
    std::fs::write(&script, text).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    Tools::find(Some(&script)).filter(|e| e.standalone)
}

#[test]
fn a_standalone_gw_gets_its_pages_from_its_help_and_converts_as_it_is() {
    let Some(tools) = tools() else { return };
    let dir = scratch("standalone-gw");
    let Some(gw) = standalone(&tools, &dir) else {
        eprintln!("skipped: no standalone gw here");
        return;
    };
    let schema = |tools: &Tools| {
        let mut service = Service::start(tools, Box::new(|| {}));
        wait("the schema", || {
            service.poll();
            service.schema.ready().cloned()
        })
    };
    let (help, parsers) = (schema(&gw), schema(&tools));
    let options = |s: &Schema| {
        let dests =
            |c: &ferriteweazle::schema::Command| c.args.iter().map(|a| a.dest.clone()).collect();
        s.commands
            .iter()
            .map(|c| (c.name.clone(), dests(c)))
            .collect::<Vec<(String, Vec<String>)>>()
    };
    assert_eq!(options(&help), options(&parsers));
    assert_eq!(help.gw(), parsers.gw());

    let (img, scp, back) = (dir.join("a.img"), dir.join("a.scp"), dir.join("b.img"));
    std::fs::write(
        &img,
        (0..368_640u32).map(|i| (i % 253) as u8).collect::<Vec<_>>(),
    )
    .unwrap();
    run(
        &gw,
        &["convert", "--format=ibm.360", &path(&img), &path(&scp)],
    );
    let job = run(
        &gw,
        &["convert", "--format=ibm.360", &path(&scp), &path(&back)],
    );
    assert_eq!(std::fs::read(&img).unwrap(), std::fs::read(&back).unwrap());
    assert!(
        job.progress
            .tracks
            .values()
            .all(|t| t.status == Status::Good)
    );
    assert_eq!(job.progress.total, Some((720, 720)));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn stopping_a_job_ends_it_and_gw_tidies_up() {
    let Some(tools) = tools() else { return };
    let dir = scratch("stop");
    let (img, scp, back) = (dir.join("a.img"), dir.join("a.scp"), dir.join("b.img"));
    std::fs::write(&img, vec![0u8; 1_474_560]).unwrap();
    run(
        &tools,
        &["convert", "--format=ibm.1440", &path(&img), &path(&scp)],
    );

    let mut job = start(
        &tools,
        "convert",
        &["convert", "--format=ibm.1440", &path(&scp), &path(&back)],
    );
    wait("the first track", || {
        job.poll();
        (!job.progress.tracks.is_empty()).then_some(())
    });
    job.stop();
    let asked = Instant::now();
    let job = finish(job, "the job to stop");
    assert_eq!(job.outcome(), Some(Outcome::Stopped));
    assert!(
        asked.elapsed() < Duration::from_secs(3),
        "stopped in {:?}",
        asked.elapsed()
    );
    assert!(job.progress.tracks.len() < 160);
    assert!(!back.exists(), "gw removes an unfinished conversion");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_question_from_gw_waits_for_an_answer() {
    let Some(tools) = tools() else { return };
    // Past cylinder 83 gw asks first; answering No ends it before any device
    // is opened, and the one it would open does not exist.
    let device = format!("--device={NO_SUCH_PORT}");
    let mut job = start(&tools, "seek", &["seek", &device, "90"]);
    let question = wait("the question", || {
        job.poll();
        job.question.clone()
    });
    assert_eq!(question, "Seek to extreme cylinder 90, Yes/No? ");
    job.answer("No");
    let job = finish(job, "the job to end");
    assert_eq!(job.outcome(), Some(Outcome::Succeeded));
    assert!(job.log.is_empty(), "{:?}", job.log);
}

/// The app's window on `tools` once gw has described itself, with no Greaseweazle
/// listed, whatever is plugged in, until gw restarts.
fn window(tools: &Tools, settings: Settings) -> Window {
    let settings = Settings {
        tools: Some(tools.python.clone()),
        ..settings
    };
    // Tall enough for a page opened out below its track settings.
    let mut w = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1240.0, 1100.0))
        .build_ui_state(
            move |ui, app: &mut Option<App>| {
                let app = app.get_or_insert_with(|| {
                    let mut app = App::with_settings(ui.ctx(), settings.clone());
                    app.pin_ports(Vec::new());
                    app
                });
                common::show(ui, app);
            },
            None,
        );
    until(&mut w, "Greaseweazle Tools", |app| app.schema().is_some());
    // A click lands on what the last frame drew, as for a person.
    w.run_steps(2);
    w
}

fn until(w: &mut Window, what: &str, done: impl Fn(&App) -> bool) {
    until_shown(w, what, |w| w.state().as_ref().is_some_and(&done));
}

/// Writes a blank ibm.360 image, dir/Game.img, and returns the Convert page
/// set to turn it into Game.scp beside it.
fn convert_page(dir: &Path) -> Settings {
    let img = dir.join("Game.img");
    std::fs::write(&img, vec![0u8; 368_640]).unwrap();
    let mut settings = Settings {
        page: Page::Command("convert".into()),
        ..Settings::default()
    };
    let values = settings.values.entry("convert".into()).or_default();
    values.set("in_file", img.to_string_lossy());
    values.set("format", "ibm.360");
    let out = Output {
        beside_input: true,
        ext: ".scp".into(),
        ..Output::default()
    };
    settings.outputs.insert("convert/out_file".into(), out);
    settings
}

#[test]
fn the_convert_page_makes_an_image_and_saves_its_log_beside_it() {
    let Some(tools) = tools() else { return };
    let dir = scratch("page");
    let settings = Settings {
        save_logs: true,
        ..convert_page(&dir)
    };
    let mut w = window(&tools, settings);
    w.get_by_label("Convert").click();
    until(&mut w, "the conversion", |app| {
        app.disk.as_ref().is_some_and(|j| !j.running())
    });

    let job = app(&w).disk.as_ref().unwrap();
    assert_eq!(job.outcome(), Some(Outcome::Succeeded), "{:#?}", job.log);
    assert!(dir.join("Game.scp").is_file());
    let log =
        std::fs::read_to_string(dir.join("Game.scp.log")).expect("the log is beside the image");
    assert!(log.contains("Found 720 sectors of 720 (100%)"), "{log}");
    // The bridge's reports, the disk's bytes and all, are the app's alone.
    assert!(!job.progress.facts.is_empty(), "reported");
    assert!(!log.contains("@ferriteweazle"), "{log}");
    assert!(!job.log.iter().any(|l| l.starts_with("@ferriteweazle")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_folder_of_images_converts_one_by_one_into_another_type_and_says_how_it_went() {
    let Some(tools) = tools() else { return };
    let dir = scratch("batch");
    let (inputs, outputs) = (dir.join("in"), dir.join("out"));
    std::fs::create_dir_all(&inputs).unwrap();
    std::fs::create_dir_all(&outputs).unwrap();
    let names = ["Game_Disk1", "Game_Disk2", "Game_Disk10"];
    for name in names {
        std::fs::write(inputs.join(format!("{name}.img")), vec![0u8; 368_640]).unwrap();
    }
    std::fs::write(inputs.join("notes.txt"), "not an image").unwrap();
    let mut settings = Settings {
        page: Page::Command("convert".into()),
        ..Settings::default()
    };
    let values = settings.values.entry("convert".into()).or_default();
    values.set(form::BATCH, "on");
    values.set(form::BATCH_FOLDER, path(&inputs));
    values.set("format", "ibm.360");
    let out = Output {
        folder: path(&outputs),
        ext: ".scp".into(),
        batch_label: "Backup".into(),
        label_first: true,
        ..Output::default()
    };
    settings.outputs.insert("convert/out_file".into(), out);
    let mut w = window(&tools, settings);
    w.get_by_label_contains("3 images: Game_Disk1.img, Game_Disk2.img, Game_Disk10.img");
    run_button(&w, "Convert images").click();
    until(&mut w, "the batch", |app| {
        app.notices.contains_key("convert")
    });

    let app = app(&w);
    assert_eq!(app.notices["convert"], "Converted 3 of 3 images.");
    let last = outputs.join("Backup_Game_Disk10.scp");
    assert_eq!(
        app.disk.as_ref().unwrap().output.as_ref(),
        Some(&last),
        "each run's own image"
    );
    for name in names {
        assert!(
            outputs.join(format!("Backup_{name}.scp")).is_file(),
            "{name}"
        );
    }
    let headings: Vec<&String> = app
        .log
        .lines()
        .iter()
        .filter(|l| l.starts_with("gw convert"))
        .collect();
    assert_eq!(headings.len(), 3, "{headings:#?}");
    for (heading, name) in headings.iter().zip(names) {
        assert!(
            heading.contains(&format!("{name}.img")),
            "{heading} is not {name}"
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_job_that_would_replace_a_file_asks_first() {
    let Some(tools) = tools() else { return };
    let dir = scratch("overwrite");
    std::fs::write(dir.join("Game.scp"), b"keep me").unwrap();
    let mut w = window(&tools, convert_page(&dir));
    w.get_by_label("Convert").click();
    w.run_steps(2);
    w.get_by_label("Overwrite \"Game.scp\"?");
    w.get_by_label("Cancel").click();
    w.run_steps(2);
    assert!(app(&w).disk.is_none(), "nothing ran");
    assert_eq!(std::fs::read(dir.join("Game.scp")).unwrap(), b"keep me");

    w.get_by_label("Convert").click();
    w.run_steps(2);
    w.get_by_label("Overwrite").click();
    until(&mut w, "the conversion", |app| {
        app.disk.as_ref().is_some_and(|j| !j.running())
    });
    let job = app(&w).disk.as_ref().unwrap();
    assert_eq!(job.outcome(), Some(Outcome::Succeeded), "{:#?}", job.log);
    assert!(std::fs::metadata(dir.join("Game.scp")).unwrap().len() > 1000);
    std::fs::remove_dir_all(dir).ok();
}

fn read_button(w: &Window) -> egui_kittest::Node<'_> {
    run_button(w, "Read disk")
}

/// Starts `seek 90` as the tool job, which waits on gw's question until stopped.
fn waiting_tool(w: &mut Window, tools: &Tools) {
    let device = format!("--device={NO_SUCH_PORT}");
    app_mut(w).tool = Some(start(tools, "seek", &["seek", &device, "90"]));
    until(w, "gw's question", |app| {
        app.tool.as_ref().is_some_and(|j| j.question.is_some())
    });
}

#[test]
fn reading_waits_for_a_format_an_image_type_and_a_greaseweazle() {
    let Some(tools) = tools() else { return };
    let mut w = window(&tools, Settings::default());
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    w.run_steps(2);
    assert!(read_button(&w).accesskit_node().is_disabled());
    choose_format(&mut w, "amiga.amigados");
    assert!(
        !read_button(&w).accesskit_node().is_disabled(),
        "a format picks its image type, so the read can start"
    );
    app_mut(&mut w).pin_ports(Vec::new());
    w.run_steps(2);
    assert!(read_button(&w).accesskit_node().is_disabled());
    read_button(&w).hover();
    until_shown(&mut w, "why it cannot read", |w| {
        w.query_by_label("Connect a Greaseweazle.").is_some()
    });
}

#[test]
fn a_device_page_stays_open_when_the_greaseweazle_goes_and_its_job_runs_on() {
    let Some(tools) = tools() else { return };
    let settings = Settings {
        page: Page::Command("erase".into()),
        ..Settings::default()
    };
    let mut w = window(&tools, settings);
    let greyed = |w: &Window| run_button(w, "Erase disk").accesskit_node().is_disabled();
    assert!(greyed(&w));
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    w.run_steps(2);
    assert!(!greyed(&w));
    app_mut(&mut w).pin_ports(Vec::new());
    w.run_steps(2);
    assert_eq!(app_mut(&mut w).settings.page, Page::Command("erase".into()));
    assert!(greyed(&w));

    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    app_mut(&mut w).settings.page = Page::Command("seek".into());
    waiting_tool(&mut w, &tools);
    app_mut(&mut w).pin_ports(Vec::new());
    w.run_steps(2);
    assert!(app_mut(&mut w).tool.as_ref().unwrap().running());
    let stop = w.get_by_role_and_label(egui::accesskit::Role::Button, "Stop");
    assert!(!stop.accesskit_node().is_disabled());
    app_mut(&mut w).tool.as_mut().unwrap().stop();
    until(&mut w, "the job to stop", |app| {
        app.tool.as_ref().is_some_and(|j| !j.running())
    });
}

/// The sidebar's entries, Settings first.
fn entries(w: &Window) -> Vec<String> {
    w.get_all_by_role(egui::accesskit::Role::Button)
        .filter(|n| n.rect().left() < 60.0 && n.rect().width() > 150.0)
        .filter_map(|n| n.accesskit_node().label())
        .collect()
}

/// Whether the last frame painted `text`, as the sidebar paints gw's version.
fn painted(w: &Window, text: &str) -> bool {
    fn has(shape: &egui::Shape, text: &str) -> bool {
        match shape {
            egui::Shape::Text(t) => t.galley.text() == text,
            egui::Shape::Vec(shapes) => shapes.iter().any(|s| has(s, text)),
            _ => false,
        }
    }
    w.output().shapes.iter().any(|c| has(&c.shape, text))
}

#[test]
fn the_sidebar_keeps_its_entries_while_gw_restarts() {
    let Some(tools) = tools() else { return };
    let settings = Settings {
        page: Page::Settings,
        ..Settings::default()
    };
    let mut w = window(&tools, settings);
    let before = entries(&w);
    assert!(before.contains(&"Erase disk".to_owned()), "{before:?}");
    let version = format!("gw {}", app(&w).schema().unwrap().version);
    w.get_by_label("Restart").click();
    w.step();
    assert!(app(&w).schema().is_none(), "gw restarts");
    wait("gw to start again", || {
        w.step();
        assert_eq!(entries(&w), before);
        assert!(painted(&w, &version), "the version beside Settings went");
        app(&w).schema().map(|_| ())
    });
    w.run_steps(2);
    assert_eq!(entries(&w), before);
}

#[test]
fn a_restarted_gw_keeps_the_greaseweazle_until_it_has_looked() {
    let Some(tools) = tools() else { return };
    let settings = Settings {
        page: Page::Settings,
        ..Settings::default()
    };
    let mut w = window(&tools, settings);
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    w.run_steps(2);
    w.get_by_label("Restart").click();
    w.run_steps(2);
    assert!(app_mut(&mut w).schema().is_none(), "gw restarts");
    assert!(w.query_by_label("Disconnected").is_none());
}

#[test]
fn closing_the_window_during_a_job_asks_then_stops_gw_before_closing() {
    let Some(tools) = tools() else { return };
    let mut w = window(&tools, Settings::default());
    waiting_tool(&mut w, &tools);

    w.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    w.step();
    let sent = |w: &Window, command: egui::ViewportCommand| {
        w.output().viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&command)
    };
    assert!(sent(&w, egui::ViewportCommand::CancelClose));
    w.step();
    w.get_by_label("Stop and quit").click();
    until(&mut w, "the job to stop", |app| {
        app.tool.as_ref().is_some_and(|j| !j.running())
    });
    assert_eq!(
        app(&w).tool.as_ref().unwrap().outcome(),
        Some(Outcome::Stopped)
    );
    wait("the window to close", || {
        w.step();
        sent(&w, egui::ViewportCommand::Close).then_some(())
    });
}

#[test]
fn a_job_that_ends_while_quit_asks_lets_the_window_close() {
    let Some(tools) = tools() else { return };
    let mut w = window(&tools, Settings::default());
    waiting_tool(&mut w, &tools);
    w.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    w.step();
    w.step();
    w.get_by_label("Stop and quit");

    app_mut(&mut w).tool.as_mut().unwrap().stop();
    wait("the window to close", || {
        w.step();
        w.output().viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Close)
            .then_some(())
    });
}

/// A flux image of `format`, which gw makes from a sector image `bytes` long.
fn flux_of(tools: &Tools, dir: &Path, format: &str, bytes: usize) -> PathBuf {
    let (img, scp) = (
        dir.join(format!("{format}.img")),
        dir.join(format!("{format}.scp")),
    );
    std::fs::write(
        &img,
        (0..bytes).map(|i| (i * 13 % 251) as u8).collect::<Vec<_>>(),
    )
    .unwrap();
    run(
        tools,
        &[
            "convert",
            &format!("--format={format}"),
            &path(&img),
            &path(&scp),
        ],
    );
    scp
}

#[test]
fn detection_names_the_format_of_a_flux_image() {
    let Some(tools) = tools() else { return };
    let dir = scratch("detect");
    let job = detect(&tools, &flux_of(&tools, &dir, "akai.800", 819_200));
    assert_eq!(job.outcome(), Some(Outcome::Succeeded), "{:#?}", job.log);
    assert_eq!(
        job.detected.first().map(String::as_str),
        Some("akai.800"),
        "{:?}",
        job.detected
    );
    assert!(!job.detected.iter().any(|f| f.ends_with(".scan")));
    // Each track it read, as gw's line names it, decoded as the format it
    // found, and the flux it read.
    let read = &job.progress.tracks;
    assert!(read.len() >= 3, "{read:?}");
    for key in read.keys() {
        let facts = &job.progress.facts[key];
        assert_eq!(
            facts.summary.as_deref(),
            Some("IBM MFM (5/5 sectors)"),
            "{key:?}"
        );
        assert!(facts.flux.is_some(), "{key:?}");
        assert_eq!(facts.sectors.len(), 5, "{key:?}");
        assert!(facts.sectors.iter().all(|s| s.at.is_some()), "{key:?}");
    }
    // A codec that keeps no places: gw's decoder noted placing each sector.
    let job = detect(&tools, &flux_of(&tools, &dir, "amiga.amigados", 901_120));
    assert_eq!(
        job.detected.first().map(String::as_str),
        Some("amiga.amigados")
    );
    let facts = &job.progress.facts;
    assert!(facts.len() >= 3, "{:?}", facts.keys());
    for (key, facts) in facts {
        assert_eq!(
            facts.summary.as_deref(),
            Some("AmigaDOS (11/11 sectors)"),
            "{key:?}"
        );
        assert_eq!(facts.sectors.len(), 11, "{key:?}");
        assert!(facts.sectors.iter().all(|s| s.at.is_some()), "{key:?}");
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_blank_image_is_no_format_and_says_so() {
    let Some(tools) = tools() else { return };
    let dir = scratch("detect-blank");
    // Nothing written, so every track is empty.
    let job = detect(&tools, &flux_of(&tools, &dir, "raw.250", 0));
    assert_eq!(job.outcome(), Some(Outcome::Failed));
    assert!(job.detected.is_empty());
    // An image's tracks, which gw takes as they are with no format.
    assert_eq!(
        job.progress.error.as_deref(),
        Some(
            "No format Greaseweazle Tools knows reads this image in full. \
             Set Disk format to None to use its tracks as they are."
        )
    );
    // The flux of each track it read, and no format's sectors.
    let facts = &job.progress.facts;
    assert!(!facts.is_empty());
    assert!(
        facts
            .values()
            .all(|f| f.flux.is_some() && f.summary.is_none())
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_detect_button_chooses_the_format_of_the_input_and_says_so_on_its_page() {
    let Some(tools) = tools() else { return };
    let dir = scratch("detect-button");
    let scp = flux_of(&tools, &dir, "amiga.amigados", 901_120);
    let mut settings = Settings {
        page: Page::Command("convert".into()),
        ..Settings::default()
    };
    settings
        .values
        .entry("convert".into())
        .or_default()
        .set("in_file", scp.to_string_lossy());
    let beside = Output {
        beside_input: true,
        ..Output::default()
    };
    settings.outputs.insert("convert/out_file".into(), beside);
    let mut w = window(&tools, settings);
    let ended = |command: &'static str| {
        move |app: &App| {
            app.disk
                .as_ref()
                .is_some_and(|j| j.command == command && !j.running())
        }
    };
    w.get_by_label("Detect").click();
    until(&mut w, "detection", ended(DETECT));
    w.run_steps(2);
    let app = app(&w);
    assert_eq!(
        app.settings.values["convert"].get("format"),
        "amiga.amigados"
    );
    assert_eq!(app.settings.outputs["convert/out_file"].ext, ".adf");
    let found = |w: &Window| w.query_by_label_contains("Found amiga.amigados.").is_some();
    assert!(found(&w));

    w.get_by_label("Read disk").click();
    w.run_steps(2);
    assert!(!found(&w), "it shows on the Read page");
    w.get_by_label("Convert image").click();
    w.run_steps(2);
    assert!(found(&w));

    w.get_by_label("Convert").click();
    until(&mut w, "the conversion", ended("convert"));
    w.run_steps(2);
    assert!(found(&w), "another job took it away");

    // Detect again: the old answer goes until the new one comes.
    w.get_by_label("Detect").click();
    w.run_steps(2);
    assert!(!found(&w));
    until(&mut w, "detection", ended(DETECT));
    w.run_steps(2);
    assert!(found(&w));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_log_keeps_every_job_of_the_session_in_order_under_its_command_line() {
    let Some(tools) = tools() else { return };
    let dir = scratch("session-log");
    let scp = flux_of(&tools, &dir, "amiga.amigados", 901_120);
    let mut settings = Settings {
        page: Page::Command("convert".into()),
        drawer: Some(Drawer::Log),
        ..Settings::default()
    };
    settings
        .values
        .entry("convert".into())
        .or_default()
        .set("in_file", path(&scp));
    let beside = Output {
        beside_input: true,
        ..Output::default()
    };
    settings.outputs.insert("convert/out_file".into(), beside);
    let mut w = window(&tools, settings);
    w.get_by_label("Detect").click();
    until(&mut w, "detection", |app| {
        app.disk.as_ref().is_some_and(|j| !j.running())
    });
    w.run_steps(2);
    w.get_by_label("Convert").click();
    until(&mut w, "the conversion", |app| {
        app.disk
            .as_ref()
            .is_some_and(|j| j.command == "convert" && !j.running())
    });
    w.run_steps(2);

    let app = app(&w);
    let lines = app.log.lines();
    let at = |line: &str| {
        lines
            .iter()
            .position(|l| l == line)
            .unwrap_or_else(|| panic!("no {line:?} in {lines:#?}"))
    };
    let adf = dir.join("amiga.amigados.adf");
    let detect = at(&format!("Detect disk format {}", quote(&path(&scp))));
    let convert = at(&format!(
        "gw convert --format=amiga.amigados {} {}",
        quote(&path(&scp)),
        quote(&path(&adf))
    ));
    assert_eq!(detect, 0, "{lines:#?}");
    let under = |from: usize, to: usize, line: &str| lines[from..to].iter().any(|l| l == line);
    assert!(
        under(detect, convert, "Format amiga.amigados"),
        "{lines:#?}"
    );
    assert!(lines[convert - 2].starts_with("Done in "), "{lines:#?}");
    assert!(under(
        convert,
        lines.len(),
        "Found 1760 sectors of 1760 (100%)"
    ));
    assert!(lines.last().unwrap().starts_with("Done in "), "{lines:#?}");
    let heads: Vec<usize> = (0..lines.len()).filter(|&i| app.log.is_head(i)).collect();
    assert_eq!(heads, [detect, convert]);
    // The drawer shows it, ending with the conversion.
    w.get_by_label(lines.last().unwrap());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn detection_tells_apart_formats_that_differ_only_in_layout() {
    let Some(tools) = tools() else { return };
    let dir = scratch("detect-layout");
    // Each decodes like another format on cylinder 0; only the index mark,
    // interleave, skew, gaps or length differ.
    for (format, bytes) in [
        ("atarist.720", 737_280), // no index mark, where ibm.720 has one
        ("ibm.720", 737_280),
        ("akai.800", 819_200), // skews each cylinder, where eagle.dsqd.800 does not
        ("thomson.2s320", 655_360), // interleaves 7:1, where luxor does not
        ("acorn.adfs.320", 327_680), // 80 cylinders, where acorn.adfs.160 has 40
    ] {
        let job = detect(&tools, &flux_of(&tools, &dir, format, bytes));
        assert_eq!(
            job.detected.first().map(String::as_str),
            Some(format),
            "{:#?}",
            job.log
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_forty_track_disk_in_an_eighty_track_drive_needs_double_step() {
    let Some(tools) = tools() else { return };
    let dir = scratch("detect-step");
    let (img, scp) = (dir.join("d.img"), dir.join("d.scp"));
    std::fs::write(
        &img,
        (0..368_640u32).map(|i| (i % 253) as u8).collect::<Vec<_>>(),
    )
    .unwrap();
    // Cylinder c of the disk lands on cylinder 2c, as an 80-track drive sees it.
    run(
        &tools,
        &[
            "convert",
            "--format=ibm.360",
            "--out-tracks=step=2",
            &path(&img),
            &path(&scp),
        ],
    );
    let job = detect(&tools, &scp);
    assert_eq!(
        job.detected.first().map(String::as_str),
        Some("ibm.360"),
        "{:#?}",
        job.log
    );
    assert_eq!(job.step, 2);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn apple_ii_disks_are_told_apart_by_their_filesystem() {
    let Some(tools) = tools() else { return };
    let dir = scratch("detect-apple");
    let mut prodos = vec![0u8; 143_360];
    prodos[1024 + 2] = 3; // block 2: no previous block, the next is 3,
    prodos[1024 + 4] = 0xF0 | 4; // and a volume directory header, named in 4 letters
    let mut dos = vec![0u8; 143_360];
    let vtoc = 17 * 16 * 256; // track 17, sector 0
    for (at, byte) in [
        (1, 17),
        (2, 15),
        (3, 3),
        (0x27, 122),
        (0x34, 35),
        (0x35, 16),
    ] {
        dos[vtoc + at] = byte;
    }
    let neither = (0..143_360u32).map(|i| (i % 249) as u8).collect::<Vec<_>>();
    for (name, format, image) in [
        ("p.po", "apple2.prodos.140", prodos),
        ("d.do", "apple2.appledos.140", dos),
        ("n.do", "apple2.nofs.140", neither),
    ] {
        let (img, scp) = (dir.join(name), dir.join(format!("{name}.scp")));
        std::fs::write(&img, image).unwrap();
        run(
            &tools,
            &[
                "convert",
                &format!("--format={format}"),
                &path(&img),
                &path(&scp),
            ],
        );
        let job = detect(&tools, &scp);
        assert_eq!(
            job.detected.first().map(String::as_str),
            Some(format),
            "{:#?}",
            job.log
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

/// A disk definitions file for mine.800, an Akai-like 800K format with no skew.
fn custom_defs(dir: &Path) -> PathBuf {
    let defs = dir.join("mine.cfg");
    std::fs::write(
        &defs,
        "disk mine.800\n    cyls = 80\n    heads = 2\n    tracks * ibm.mfm\n        secs = 5\n        \
         bps = 1024\n        gap3 = 116\n        rate = 250\n    end\nend\n",
    )
    .unwrap();
    defs
}

#[test]
fn gw_checks_a_disk_definitions_file_line_by_line() {
    let Some(tools) = tools() else { return };
    let dir = scratch("diskdefs-check");
    let good = custom_defs(&dir);
    let bad = dir.join("bad.cfg");
    std::fs::write(
        &bad,
        "disk mine.bad\n    cyls = 80\n    heads = 2\n    tracks * ibm.mfm\n        secs = 5\n        \
         bogus = 3\n    end\nend\ndisk mine.worse\n    cyls = eighty\nend\n",
    )
    .unwrap();
    let mut service = Service::start(&tools, Box::new(|| {}));
    let mut read = |file: &Path| {
        let file = path(file);
        wait("the file's check", || {
            service.poll();
            match service.diskdefs(&file) {
                Load::Ready(d) => Some(Ok(d.clone())),
                Load::Failed(e) => Some(Err(e.clone())),
                Load::Waiting(_) => None,
            }
        })
    };
    let good = read(&good).unwrap();
    assert_eq!(
        (good.formats, good.errors),
        (vec!["mine.800".to_owned()], vec![])
    );
    let bad = read(&bad).unwrap();
    assert!(bad.formats.is_empty(), "{bad:?}");
    assert_eq!(bad.failed, ["mine.bad", "mine.worse"]);
    assert!(
        bad.errors[0].ends_with("line 6: unrecognised track option bogus"),
        "{bad:?}"
    );
    assert!(bad.errors[1].contains("line 10"), "{bad:?}");
    // gw reads the file from the top for each disk, so a mistake above them
    // all spoils each, and is named once.
    let stray = dir.join("stray.cfg");
    let disks = std::fs::read_to_string(dir.join("mine.cfg")).unwrap();
    std::fs::write(
        &stray,
        format!("oops\n{disks}{}", disks.replace("mine.800", "mine.900")),
    )
    .unwrap();
    let stray = read(&stray).unwrap();
    assert!(stray.formats.is_empty(), "{stray:?}");
    assert_eq!(stray.failed, ["mine.800", "mine.900"]);
    assert_eq!(stray.errors.len(), 1, "{stray:?}");
    assert!(
        stray.errors[0].ends_with("line 1: syntax error"),
        "{stray:?}"
    );
    assert_eq!(
        read(&dir.join("missing.cfg")),
        Err("There is no such file.".into())
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn an_edited_disk_definitions_file_gives_its_new_layout() {
    let Some(tools) = tools() else { return };
    let dir = scratch("diskdefs-edit");
    let defs = custom_defs(&dir);
    let file = path(&defs);
    let mut service = Service::start(&tools, Box::new(|| {}));
    let mut cyls = |want: u32| {
        wait("the layout", || {
            service.poll();
            let info = service.format_info(&file, "mine.800");
            if let Some(e) = info.error() {
                panic!("no layout: {e}");
            }
            (info.ready().map(|i| i.cyls) == Some(want)).then_some(())
        })
    };
    cyls(80);
    let text = std::fs::read_to_string(&defs).unwrap();
    std::fs::write(&defs, text.replace("cyls = 80", "cyls = 40")).unwrap();
    cyls(40);
    std::fs::remove_dir_all(dir).ok();
}

/// Steps the window until `shown`; on a timeout, lists what it showed instead.
fn until_shown(w: &mut Window, what: &str, shown: impl Fn(&Window) -> bool) {
    let start = Instant::now();
    loop {
        w.step();
        if shown(w) {
            return;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let seen: Vec<String> = w
                .root()
                .children_recursive()
                .filter_map(|n| n.accesskit_node().label())
                .filter(|l| !l.is_empty())
                .collect();
            panic!("timed out waiting for {what}; the window showed {seen:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Chooses a format from the page's list by searching for it.
fn choose_format(w: &mut Window, format: &str) {
    w.get_all_by_role(egui::accesskit::Role::ComboBox)
        .nth(1)
        .expect("a format picker")
        .click();
    w.run_steps(2);
    w.event(egui::Event::Text(format.into()));
    w.run_steps(2);
    w.get_by_label(format).click();
    w.run_steps(3);
}

/// The track picker's button for a side, right of the drive identifier of the same name.
fn side<'a>(w: &'a Window, name: &'a str) -> egui_kittest::Node<'a> {
    w.get_all_by_role_and_label(egui::accesskit::Role::Button, name)
        .max_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .expect("a side button")
}

/// Whether each side's button is enabled, and lit.
fn sides(w: &Window) -> [(bool, bool); 2] {
    ["0", "1"].map(|name| {
        let node = side(w, name);
        let node = node.accesskit_node();
        let lit = node.toggled() == Some(egui::accesskit::Toggled::True);
        (!node.is_disabled(), lit)
    })
}

#[test]
fn a_one_sided_format_greys_the_sides_unless_the_list_names_side_1() {
    let Some(tools) = tools() else { return };
    let mut w = window(&tools, Settings::default());
    choose_format(&mut w, "ibm.1440");
    until_shown(&mut w, "two sides", |w| {
        w.query_by_label_contains("2\u{a0}sides").is_some()
    });
    assert_eq!(sides(&w), [(true, true), (true, true)]);
    side(&w, "0").click();
    w.run_steps(2);
    assert_eq!(sides(&w), [(true, false), (true, true)]);

    // Another format brings its own sides.
    choose_format(&mut w, "ibm.160");
    until_shown(&mut w, "one side", |w| {
        w.query_by_label_contains("1\u{a0}side").is_some()
    });
    assert_eq!(sides(&w), [(false, true), (false, false)]);
    side(&w, "1").hover();
    until_shown(&mut w, "why side 1 is greyed", |w| {
        w.query_by_label("This format is single sided.").is_some()
    });
    assert!(w.query_by_label("Which tracks to read.").is_none());

    // A list of the page's own, such as a preset's, keeps side 1 alone, so a
    // click still changes the sides.
    let mut settings = Settings::default();
    let read = settings.values.entry("read".into()).or_default();
    read.set("format", "ibm.160");
    read.set("tracks", "h=1");
    let mut w = window(&tools, settings);
    until_shown(&mut w, "one side", |w| {
        w.query_by_label_contains("1\u{a0}side").is_some()
    });
    assert_eq!(sides(&w), [(true, false), (true, true)]);
    side(&w, "0").click();
    w.run_steps(2);
    assert_eq!(sides(&w), [(false, true), (false, false)]);
}

#[test]
fn the_write_page_takes_a_north_star_images_format_from_gw() {
    let Some(tools) = tools() else { return };
    let dir = scratch("nsi");
    // gw knows an .nsi's format only by its size: 89,600 bytes is one-sided FM.
    let nsi = dir.join("Disk.nsi");
    std::fs::write(&nsi, vec![0u8; 89_600]).unwrap();
    let mut settings = Settings {
        page: Page::Command("write".into()),
        ..Settings::default()
    };
    let values = settings.values.entry("write".into()).or_default();
    values.set("file", nsi.to_string_lossy());
    let mut w = window(&tools, settings);
    until_shown(&mut w, "the image's format", |w| {
        w.query_by_label_contains("35\u{a0}cylinders").is_some()
    });
    w.get_by_label_contains("1\u{a0}side");
    let format = |w: &Window| {
        w.get_all_by_role(egui::accesskit::Role::ComboBox)
            .nth(1)
            .and_then(|f| f.value())
            .unwrap_or_default()
    };
    assert_eq!(format(&w), "North Star · northstar.fm.ss (from the input)");
    assert_eq!(sides(&w), [(false, true), (false, false)]);
    let last = w
        .get_all_by_role(egui::accesskit::Role::SpinButton)
        .nth(1)
        .and_then(|c| c.accesskit_node().numeric_value());
    assert_eq!(last, Some(34.0), "cylinders 0 to 34");
    until_shown(&mut w, "the disk's 35 squares", |w| {
        squares(w).count() == 35
    });

    // The file is looked at again when it changes.
    std::fs::write(&nsi, vec![0u8; 179_200]).unwrap();
    until_shown(&mut w, "one-sided MFM", |w| {
        format(w) == "North Star · northstar.mfm.ss (from the input)"
            && w.query_by_label_contains("175\u{a0}KB").is_some()
    });
    assert_eq!(sides(&w), [(false, true), (false, false)]);
    std::fs::write(&nsi, vec![0u8; 358_400]).unwrap();
    until_shown(&mut w, "the new format", |w| {
        w.query_by_label_contains("2\u{a0}sides").is_some()
    });
    assert_eq!(sides(&w), [(true, true), (true, true)]);

    std::fs::write(&nsi, vec![0u8; 1000]).unwrap();
    until_shown(&mut w, "gw's objection", |w| {
        w.query_by_label("NSI: Disk.nsi: unrecognised file size.")
            .is_some()
    });
    // A device comes before the page's own settings.
    w.get_all_by_role_and_label(egui::accesskit::Role::Button, "Write disk")
        .last()
        .expect("the run button")
        .hover();
    until_shown(&mut w, "the device it needs", |w| {
        w.query_by_label("Connect a Greaseweazle.").is_some()
    });
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    until_shown(&mut w, "why it cannot run", |w| {
        w.query_by_label("Greaseweazle Tools cannot read this image. See Disk format.")
            .is_some()
    });
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_north_star_image_converts_with_the_format_gw_finds_in_it() {
    let Some(tools) = tools() else { return };
    let dir = scratch("nsi-convert");
    let nsi = dir.join("Disk.nsi");
    std::fs::write(&nsi, vec![0u8; 179_200]).unwrap();
    let mut settings = Settings {
        page: Page::Command("convert".into()),
        ..Settings::default()
    };
    let values = settings.values.entry("convert".into()).or_default();
    values.set("in_file", nsi.to_string_lossy());
    let out = Output {
        beside_input: true,
        ext: ".scp".into(),
        ..Output::default()
    };
    settings.outputs.insert("convert/out_file".into(), out);
    let mut w = window(&tools, settings);
    until_shown(&mut w, "Convert to be ready", |w| {
        !w.get_by_label("Convert").accesskit_node().is_disabled()
    });

    // gw convert takes an output type's own format before the input's.
    let output = |w: &mut Window, ext: &str| {
        let outputs = &mut app_mut(w).settings.outputs;
        outputs.get_mut("convert/out_file").unwrap().ext = ext.into();
        w.run_steps(3);
    };
    let format = |w: &Window| {
        w.get_all_by_role(egui::accesskit::Role::ComboBox)
            .nth(1)
            .and_then(|f| f.value())
            .unwrap_or_default()
    };
    output(&mut w, ".adf");
    assert_eq!(format(&w), "Amiga · amiga.amigados (from the image type)");
    assert!(!w.get_by_label("Convert").accesskit_node().is_disabled());
    output(&mut w, ".scp");
    assert_eq!(format(&w), "North Star · northstar.mfm.ss (from the input)");
    w.get_by_label("Convert").click();
    until(&mut w, "the conversion", |app| {
        app.disk.as_ref().is_some_and(|j| !j.running())
    });
    let job = app(&w).disk.as_ref().unwrap();
    assert_eq!(job.outcome(), Some(Outcome::Succeeded), "{:#?}", job.log);
    assert!(
        job.log.iter().any(|l| l.contains("northstar.mfm.ss")),
        "{:#?}",
        job.log
    );
    assert!(dir.join("Disk.scp").is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_disk_definitions_file_puts_its_formats_first_and_goes_to_gw_only_with_them() {
    let Some(tools) = tools() else { return };
    let dir = scratch("diskdefs-page");
    let defs = custom_defs(&dir);
    let mut settings = Settings::default();
    let values = settings.values.entry("read".into()).or_default();
    values.set("diskdefs", defs.to_string_lossy());
    let mut w = window(&tools, settings);
    w.get_by_label("CLI").click();
    w.run_steps(2);

    w.get_all_by_role(egui::accesskit::Role::ComboBox)
        .nth(1)
        .expect("a format picker")
        .click();
    until_shown(&mut w, "the file's formats", |w| {
        w.query_by_label("Custom disk definitions").is_some()
    });
    // Acorn is the first of gw's own families.
    let top = |label| w.get_by_label(label).rect().top();
    assert!(top("Custom disk definitions") < top("Acorn"));
    w.get_by_label("mine.800").click();
    w.run_steps(3);
    let cli = line(&w);
    assert!(
        cli.contains(&format!("--diskdefs={}", defs.display())),
        "{cli}"
    );
    assert!(cli.contains("--format=mine.800"), "{cli}");
    // Non-breaking spaces keep each fact in the format's description whole.
    until_shown(&mut w, "the format's description", |w| {
        w.query_by_label_contains("5\u{a0}sectors").is_some()
    });

    choose_format(&mut w, "ibm.1440");
    let cli = line(&w);
    assert!(cli.contains("--format=ibm.1440"), "{cli}");
    assert!(
        !cli.contains("--diskdefs"),
        "gw's own format needs no file: {cli}"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_broken_definition_stops_only_a_page_that_uses_it() {
    let Some(tools) = tools() else { return };
    let dir = scratch("diskdefs-broken");
    let defs = dir.join("mixed.cfg");
    let sound = std::fs::read_to_string(custom_defs(&dir)).unwrap();
    let later = "disk two.800\n    cyls = 80\n    heads = 2\nend\n";
    let three = later.replace("two", "three");
    std::fs::write(&defs, format!("{sound}garbage\n{later}{three}")).unwrap();
    let mut settings = Settings::default();
    let values = settings.values.entry("read".into()).or_default();
    values.set("diskdefs", defs.to_string_lossy());
    let mut w = window(&tools, settings);
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    w.get_all_by_role(egui::accesskit::Role::ComboBox)
        .nth(1)
        .expect("a format picker")
        .click();
    until_shown(&mut w, "the file's sound format", |w| {
        w.query_by_label("mine.800").is_some()
    });
    w.get_by_label("mine.800").click();
    // The format's details move the header down when gw's answer comes.
    until_shown(&mut w, "the format's details", |w| {
        w.query_by_label_contains("cylinders").is_some()
    });
    w.run_steps(2);
    w.get_by_label_contains("Advanced options").click();
    until_shown(&mut w, "gw's objection", |w| {
        w.query_by_label_contains("mixed.cfg, line 11").is_some()
    });
    let objections = w.query_all_by_label_contains("line 11").count();
    assert_eq!(objections, 1, "one error spoils two definitions, said once");
    until_shown(&mut w, "Read ready", |w| {
        !read_button(w).accesskit_node().is_disabled()
    });

    let values = app_mut(&mut w).settings.values.get_mut("read").unwrap();
    values.set("format", "two.800");
    w.run_steps(3);
    assert!(read_button(&w).accesskit_node().is_disabled());
    read_button(&w).hover();
    until_shown(&mut w, "why it cannot read", |w| {
        w.query_by_label("The format's definition has errors. See Advanced options.")
            .is_some()
    });
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_disk_definitions_path_may_start_at_the_home_folder() {
    let Some(tools) = tools() else { return };
    let home = scratch("diskdefs-home");
    custom_defs(&home);
    let mut bridge = tools
        .bridge("serve")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the bridge starts");
    let mut input = bridge.stdin.take().unwrap();
    let ask = b"{\"op\": \"diskdefs\", \"path\": \"~/mine.cfg\"}\n";
    std::io::Write::write_all(&mut input, ask).unwrap();
    drop(input);
    let out = bridge.wait_with_output().unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        reply["ok"]["formats"],
        serde_json::json!(["mine.800"]),
        "{reply}"
    );
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn detection_finds_a_format_from_a_disk_definitions_file() {
    let Some(tools) = tools() else { return };
    let dir = scratch("diskdefs-detect");
    let defs = format!("--diskdefs={}", custom_defs(&dir).display());
    let (img, scp) = (dir.join("mine.img"), dir.join("mine.scp"));
    std::fs::write(
        &img,
        (0..819_200)
            .map(|i| (i * 7 % 251) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    run(
        &tools,
        &[
            "convert",
            &defs,
            "--format=mine.800",
            &path(&img),
            &path(&scp),
        ],
    );
    let job = finish(start(&tools, DETECT, &[&defs, &path(&scp)]), "detection");
    assert!(
        job.detected.iter().any(|f| f == "mine.800"),
        "{:#?}",
        job.log
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_kryoflux_stream_is_saved_as_the_set_of_files_gw_names() {
    let Some(tools) = tools() else { return };
    let dir = scratch("kryoflux");
    let mut settings = convert_page(&dir);
    settings.outputs.get_mut("convert/out_file").unwrap().ext = ".raw".into();
    let mut w = window(&tools, settings);
    w.get_by_label("Convert").click();
    until(&mut w, "the conversion", |app| {
        app.disk.as_ref().is_some_and(|j| !j.running())
    });
    let job = app(&w).disk.as_ref().unwrap();
    assert_eq!(job.outcome(), Some(Outcome::Succeeded), "{:#?}", job.log);
    for track in ["00.0", "39.1"] {
        assert!(dir.join(format!("Game{track}.raw")).is_file(), "{track}");
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn detection_finds_the_format_of_a_track_image() {
    let Some(tools) = tools() else { return };
    let dir = scratch("detect-imd");
    let (img, imd) = (dir.join("d.img"), dir.join("d.imd"));
    let data: Vec<u8> = (0..737_280).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(&img, data).unwrap();
    run(
        &tools,
        &["convert", "--format=ibm.720", &path(&img), &path(&imd)],
    );
    let job = detect(&tools, &imd);
    assert_eq!(
        job.detected.first().map(String::as_str),
        Some("ibm.720"),
        "{:#?}",
        job.log
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn an_image_type_that_cannot_hold_the_format_stops_the_read_and_says_why() {
    let Some(tools) = tools() else { return };
    let mut service = Service::start(&tools, Box::new(|| {}));
    let mut fits = |format: &str, ext: &str| {
        wait("gw's answer", || {
            service.poll();
            match service.fits("", format, ext) {
                Load::Ready(e) => Some(e.clone()),
                Load::Failed(e) => panic!("no answer: {e}"),
                Load::Waiting(_) => None,
            }
        })
    };
    assert_eq!(fits("amiga.amigados", ".adf"), None);
    let imd = fits("amiga.amigados", ".imd").unwrap_or_default();
    assert!(imd.contains("Not IBM.FM nor IBM.MFM"), "{imd}");
    assert!(
        fits("ibm.1440", ".d64").is_some(),
        "gw reads a .d64 as C64 only"
    );
    assert!(
        fits("acorn.dfs.ss", ".d81").is_some(),
        "one side where it swaps two"
    );
    assert_eq!(
        fits("raw.250", ".img").as_deref(),
        Some("The image would be empty.")
    );

    let mut w = window(&tools, Settings::default());
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    choose_format(&mut w, "amiga.amigados");
    assert!(!read_button(&w).accesskit_node().is_disabled());
    let out = app_mut(&mut w)
        .settings
        .outputs
        .get_mut("read/file")
        .unwrap();
    out.ext = ".imd".into();
    until_shown(&mut w, "gw's objection", |w| {
        w.query_by_label_contains("Not IBM.FM nor IBM.MFM")
            .is_some()
    });
    w.run_steps(2);
    assert!(read_button(&w).accesskit_node().is_disabled());
    read_button(&w).hover();
    until_shown(&mut w, "why it cannot read", |w| {
        w.query_by_label("The image type cannot hold the disk format. See Image type.")
            .is_some()
    });
}

#[test]
fn a_format_says_how_its_sectors_vary_and_a_broken_one_stops_the_read() {
    let Some(tools) = tools() else { return };
    let mut service = Service::start(&tools, Box::new(|| {}));
    let mut info = |name: &str| {
        wait("format details", || {
            service.poll();
            match service.format_info("", name) {
                Load::Ready(info) => Some(Ok(info.clone())),
                Load::Failed(e) => Some(Err(e.clone())),
                Load::Waiting(_) => None,
            }
        })
    };
    let c64 = info("commodore.1541").unwrap();
    assert_eq!(c64.sectors, Some((17, 21)));
    let scan = info("ibm.scan").unwrap();
    assert_eq!(scan.encoding.as_deref(), Some("IBM"), "not IBM Empty");
    let broken = info("zx.rocky.ss40").unwrap_err();
    assert!(broken.contains("cylinder out of range"), "{broken}");

    let mut w = window(&tools, Settings::default());
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    choose_format(&mut w, "zx.rocky.ss40");
    until_shown(&mut w, "gw's objection to the format", |w| {
        w.query_by_label_contains("cylinder out of range").is_some()
    });
    w.run_steps(2);
    assert!(read_button(&w).accesskit_node().is_disabled());
    read_button(&w).hover();
    until_shown(&mut w, "why it cannot read", |w| {
        w.query_by_label("Greaseweazle Tools cannot use this disk format. See Disk format.")
            .is_some()
    });
}

#[test]
fn image_options_offer_gws_names_and_show_its_objections() {
    let Some(tools) = tools() else { return };
    let mut service = Service::start(&tools, Box::new(|| {}));
    let schema = wait("the schema", || {
        service.poll();
        service.schema.ready().cloned()
    });
    let opt = |ext: &str, name: &str| {
        let opts = &schema.images[ext].write_opts;
        opts.iter().find(|o| o.name == name).unwrap().clone()
    };
    let disktype = opt(".scp", "disktype");
    for name in ["amiga", "ibmpc-1m44"] {
        assert!(disktype.choices.iter().any(|c| c == name), "{name}");
    }
    let default = disktype.default.as_ref().and_then(|d| d.as_str());
    assert_eq!(default, Some("other-320k"));
    let interface = opt(".hfe", "interface");
    assert!(interface.choices.iter().any(|c| c == "ibmpc_dd"));
    let complaint = wait("a complaint", || {
        service.poll();
        service.check_opt(".hfe", "version", "2").map(str::to_owned)
    });
    assert_eq!(complaint, "HFE: Invalid version: '2'");

    let mut settings = Settings::default();
    let mut out = Output {
        ext: ".hfe".into(),
        ..Output::default()
    };
    out.opts.insert("version".into(), "2".into());
    settings.outputs.insert("read/file".into(), out);
    let mut w = window(&tools, settings);
    until_shown(&mut w, "gw's complaint on the page", |w| {
        w.query_by_label("HFE: Invalid version: '2'.").is_some()
    });
}

/// A port no computer has: gw fails to open it, and touches no device.
const NO_SUCH_PORT: &str = "/dev/ferriteweazle-no-such-port";

#[test]
fn a_disk_that_fails_is_read_again_into_its_own_file() {
    let Some(tools) = tools() else { return };
    let dir = scratch("again");
    let mut settings = Settings {
        drawer: Some(Drawer::Cli),
        ..Settings::default()
    };
    let values = settings.values.entry("read".into()).or_default();
    values.set("format", "ibm.1440");
    let out = Output {
        folder: path(&dir),
        name: "Game".into(),
        label: "Disk".into(),
        ext: ".img".into(),
        disks: 2,
        ..Output::default()
    };
    settings.outputs.insert("read/file".into(), out);
    let mut w = window(&tools, settings);
    let port = Port {
        device: NO_SUCH_PORT.into(),
        ..greaseweazle()
    };
    app_mut(&mut w).pin_ports(vec![port]);
    w.run_steps(2);
    let device = format!("--device={NO_SUCH_PORT}");
    assert!(line(&w).contains(&device), "{}", line(&w));
    run_button(&w, "Read disks").click();
    let reads = |app: &App| {
        let ended = app.disk.as_ref().is_some_and(|j| !j.running());
        let lines = app.log.lines().iter();
        ended.then(|| lines.filter(|l| l.starts_with("gw read")).count())
    };
    until(&mut w, "disk 1 to fail", |app| reads(app) == Some(1));
    w.run_steps(2);
    w.get_by_label("Disk 1 failed. The Log says why.");
    w.get_by_label("Read disk 1 again").click();
    until(&mut w, "disk 1 again", |app| reads(app) == Some(2));
    let job = w.state().as_ref().unwrap().disk.as_ref().unwrap();
    assert_eq!(job.part, Some((1, 2)));
    assert_eq!(job.output, Some(dir.join("Game_Disk1.img")));
    let error = job.progress.error.as_deref().unwrap_or_default();
    assert!(error.contains(NO_SUCH_PORT), "{:#?}", job.log);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_format_gives_the_revolutions_gw_read_takes_of_each_track() {
    let Some(tools) = tools() else { return };
    let mut service = Service::start(&tools, Box::new(|| {}));
    let mut info = |name: &str| {
        wait(name, || {
            service.poll();
            service.format_info("", name).ready().cloned()
        })
    };
    assert_eq!(
        info("ibm.1440").revs,
        Some(2.0),
        "IBM reads two revolutions"
    );
    assert_eq!(
        info("amiga.amigados").revs,
        Some(1.1),
        "a timed fraction past one"
    );
}

/// Runs the bridge's (argv[1]) detection with the options after argv[2] on a made-up
/// Greaseweazle whose drive holds image argv[2]'s disk two cylinders out, sides swapped,
/// with no index pulse when asked for none; prints what it found and did to the drive.
const FAKE_DRIVE: &str = r#"
import contextlib, io, json, runpy, sys
bridge = runpy.run_path(sys.argv[1])
from greaseweazle.flux import Flux
from greaseweazle.tools import util
image = util.get_image_class(sys.argv[2]).from_file(sys.argv[2], None, {})

class Unit:
    sample_freq = image.get_track(0, 0).sample_freq
    def __init__(self):
        self.pin2, self.pins, self.seeks, self.revs = False, [], [], set()
    def get_pin(self, pin):
        return self.pin2
    def set_pin(self, pin, level):
        self.pin2 = level
        self.pins.append(level)
    def seek(self, c, h):
        self.seeks.append([c, h])
        self.at = c - 2, 1 - h
    def read_track(self, revs, ticks=0):
        self.revs.add(revs)
        track = image.get_track(*self.at) if self.at[0] >= 0 else None
        if track is None:
            return Flux([], [], self.sample_freq, index_cued=False)
        return Flux(track.index_list if revs else [], track.list, track.sample_freq, index_cued=False)
    def __getattr__(self, name):  # selecting the drive, turning its motor
        return lambda *args: None

unit = Unit()
util.usb_open = lambda device: unit
out = io.StringIO()
with contextlib.redirect_stdout(out):
    bridge['detect'](sys.argv[3:])
lines = out.getvalue().splitlines()
result = next(l for l in lines if l.startswith(bridge['RESULT']))
found = json.loads(result[len(bridge['RESULT']):])
fatal = [b for a, b in zip(lines, lines[1:]) if a == '** FATAL ERROR:']
print(json.dumps({**found, 'pins': unit.pins, 'seeks': unit.seeks[:3], 'revs': sorted(unit.revs),
                  'error': fatal[0] if fatal else None}))
"#;

#[test]
fn detection_reads_the_disk_or_image_as_its_page_would() {
    let Some(tools) = tools() else { return };
    let dir = scratch("detect-options");
    let disk = flux_of(&tools, &dir, "ibm.1440", 1_474_560);
    // A flippy's side B, as a drive reads it without --reverse.
    let flipped = dir.join("flipped.scp");
    run(
        &tools,
        &["convert", "--reverse", &path(&disk), &path(&flipped)],
    );
    let job = finish(
        start(&tools, DETECT, &["--reverse", &path(&flipped)]),
        "detection",
    );
    assert_eq!(
        job.detected.first().map(String::as_str),
        Some("ibm.1440"),
        "{:#?}",
        job.log
    );

    let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.py");
    let drive = |image: &Path, options: &[&str]| {
        let out = std::process::Command::new(&tools.python)
            .args(["-c", FAKE_DRIVE])
            .arg(&bridge)
            .arg(image)
            .arg(format!("--device={NO_SUCH_PORT}"))
            .arg("--tracks=h0.off=+2:h1.off=+2:hswap")
            .args(options)
            .output()
            .expect("python runs");
        serde_json::from_slice::<serde_json::Value>(&out.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)))
    };
    let flippy = drive(&flipped, &["--reverse", "--densel=H"]);
    assert_eq!(flippy["formats"][0], "ibm.1440", "{flippy}");
    assert_eq!(
        flippy["pins"],
        serde_json::json!([true, false]),
        "pin 2 high, then as it was"
    );
    assert_eq!(flippy["seeks"], serde_json::json!([[2, 1], [2, 0], [4, 1]]));
    let no_index = drive(&disk, &["--fake-index=300rpm"]);
    assert_eq!(no_index["formats"][0], "ibm.1440", "{no_index}");
    assert_eq!(
        no_index["revs"],
        serde_json::json!([0]),
        "no index pulse waited for"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_blank_disk_in_the_drive_is_no_format_and_says_to_read_it_as_raw_flux() {
    let Some(tools) = tools() else { return };
    let dir = scratch("detect-blank-disk");
    let blank = flux_of(&tools, &dir, "raw.250", 0);
    let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.py");
    let out = std::process::Command::new(&tools.python)
        .args(["-c", FAKE_DRIVE])
        .arg(&bridge)
        .arg(&blank)
        .arg(format!("--device={NO_SUCH_PORT}"))
        .arg("--tracks=c=0-2")
        .output()
        .expect("python runs");
    let said: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)));
    assert_eq!(said["formats"], serde_json::json!([]));
    assert_eq!(
        said["error"],
        "No format Greaseweazle Tools knows reads this disk in full. \
         Set Disk format to None to read as raw flux (.scp)."
    );
    std::fs::remove_dir_all(dir).ok();
}

/// Runs `gw` argv[4:] through the bridge (argv[1]) with the environment in JSON
/// argv[3], on a made-up Greaseweazle whose drive holds image argv[2]'s disk.
/// Odd reads of track 0.0 lose the second half of each revolution, even reads the
/// first, or read n the half LOSE's nth names; read STOP_AT of it is stopped.
/// Prints how often each track was read, and the revs and ticks asked for.
const PASSES_DRIVE: &str = r#"
import json, os, runpy, sys
bridge = runpy.run_path(sys.argv[1])
from greaseweazle.flux import Flux
from greaseweazle.tools import util
image = util.get_image_class(sys.argv[2]).from_file(sys.argv[2], None, {})

def silence(flux, lost):
    rev = sum(flux.index_list[1:]) / (len(flux.index_list) - 1)
    out, t, gap = [], 0, 0
    for x in flux.list:
        t += x
        if lost((t - flux.index_list[0]) % rev / rev):
            gap += x
        else:
            out.append(gap + x)
            gap = 0
    return Flux(flux.index_list, out + [gap] * (gap > 0), flux.sample_freq, index_cued=False)

class Unit:
    sample_freq = image.get_track(0, 0).sample_freq
    reads, asked, at = {}, set(), (0, 0)
    def seek(self, c, h):
        self.at = c, h
    def read_track(self, revs, ticks=0):
        Unit.asked.add((revs, ticks))
        n = Unit.reads[self.at] = Unit.reads.get(self.at, 0) + 1
        track = image.get_track(*self.at)
        flux = Flux(track.index_list, track.list, track.sample_freq, index_cued=False)
        if self.at == (0, 0):
            if n == int(os.environ.get('STOP_AT') or 0):
                raise KeyboardInterrupt
            lose = json.loads(os.environ.get('LOSE') or 'null')
            second = lose[n - 1] == 'second' if lose else n % 2
            flux = silence(flux, (lambda p: p >= 0.55) if second else (lambda p: p < 0.45))
        return flux
    def __getattr__(self, name):  # selecting the drive, turning its motor
        return lambda *args: None

util.usb_open = lambda device: Unit()
os.environ.update(json.loads(sys.argv[3]))
try:
    bridge['gw'](sys.argv[4:])
except KeyboardInterrupt:
    pass
reads = {f'{c}.{h}': n for (c, h), n in Unit.reads.items()}
print(json.dumps({'reads': reads, 'asked': sorted(Unit.asked)}), file=sys.__stdout__)
"#;

/// `gw read` of `disk`'s cylinders 0 and 1 into `image` on PASSES_DRIVE, with no
/// retries: what the drive saw, and gw's output.
fn read_in_passes(
    tools: &Tools,
    disk: &Path,
    format: &str,
    env: serde_json::Value,
    image: &Path,
    options: &[&str],
) -> (serde_json::Value, String) {
    let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.py");
    let out = std::process::Command::new(&tools.python)
        .args(["-c", PASSES_DRIVE])
        .arg(&bridge)
        .arg(disk)
        .arg(env.to_string())
        .args(["read", &format!("--device={NO_SUCH_PORT}")])
        .args([&format!("--format={format}"), "--retries=0"])
        .args(options)
        .args(["--tracks=c=0-1", &path(image)])
        .output()
        .expect("python runs");
    let log = String::from_utf8_lossy(&out.stderr).into_owned();
    let drive = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("{log}"));
    (drive, log)
}

#[test]
fn a_read_in_passes_takes_every_sector_any_pass_found() {
    let Some(tools) = tools() else { return };
    let dir = scratch("passes");
    let disk = flux_of(&tools, &dir, "ibm.1440", 1_474_560);
    let read =
        |env, name: &str| read_in_passes(&tools, &disk, "ibm.1440", env, &dir.join(name), &[]);
    let (_, log) = read(serde_json::json!({}), "once.img");
    assert!(log.contains("T0.0: Giving up"), "{log}");

    let (drive, log) = read(
        serde_json::json!({"FERRITEWEAZLE_PASSES": "2"}),
        "twice.img",
    );
    assert!(log.lines().any(|l| l == "Pass 2 of 2: 1 track"), "{log}");
    assert!(
        log.contains("T0.0: IBM MFM (18/18 sectors) from 2 passes"),
        "{log}"
    );
    assert!(log.contains("Found 72 sectors of 72"), "{log}");
    let reads = serde_json::json!({"0.0": 2, "0.1": 1, "1.0": 1, "1.1": 1});
    assert_eq!(drive["reads"], reads);
    let whole = std::fs::read(dir.join("ibm.1440.img")).unwrap();
    let twice = std::fs::read(dir.join("twice.img")).unwrap();
    assert!(
        twice[..4 * 18 * 512] == whole[..4 * 18 * 512],
        "the image is the disk's"
    );

    let keep = dir.join("Read passes").join("Disk pass");
    let env = serde_json::json!({
        "FERRITEWEAZLE_PASSES": "3",
        "FERRITEWEAZLE_REREAD": "disk",
        "FERRITEWEAZLE_KEEP": path(&keep),
    });
    let (drive, _) = read(env, "whole.img");
    let reads = serde_json::json!({"0.0": 2, "0.1": 2, "1.0": 2, "1.1": 2});
    assert_eq!(
        drive["reads"], reads,
        "a disk read in full needs no third pass"
    );
    let kept = |n: u32| dir.join(format!("Read passes/Disk pass {n}.scp"));
    assert!(kept(1).exists() && kept(2).exists() && !kept(3).exists());
    std::fs::remove_dir_all(dir.join("Read passes")).unwrap();

    // Stopped in pass 2: the image and pass 1 stay.
    let env = serde_json::json!({
        "FERRITEWEAZLE_PASSES": "3",
        "FERRITEWEAZLE_KEEP": path(&keep),
        "STOP_AT": "2",
    });
    read(env, "stopped.img");
    let stopped = std::fs::read(dir.join("stopped.img")).unwrap();
    let side1 = 18 * 512..2 * 18 * 512;
    assert!(stopped[side1.clone()] == whole[side1], "track 0.1 kept");
    assert!(kept(1).exists());

    let env = serde_json::json!({"FERRITEWEAZLE_PASSES": "2"});
    let image = dir.join("raw.scp");
    let (_, log) = read_in_passes(&tools, &disk, "ibm.1440", env, &image, &["--raw"]);
    assert!(
        log.contains("T0.0: IBM MFM (18/18 sectors) from 2 passes"),
        "{log}"
    );
    assert!(image.exists(), "{log}");
    std::fs::remove_dir_all(dir).ok();
}

/// The bridge's reports on track `key` in `log`, in turn.
fn reports_on(log: &str, key: (u64, u64)) -> Vec<serde_json::Value> {
    log.lines()
        .filter_map(|l| l.strip_prefix("@ferriteweazle track "))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| (v["c"].as_u64(), v["h"].as_u64()) == (Some(key.0), Some(key.1)))
        .collect()
}

#[test]
fn a_later_pass_decodes_each_read_on_its_own_as_gw_decodes_its_retries() {
    let Some(tools) = tools() else { return };
    let dir = scratch("passes-reads");
    let disk = flux_of(&tools, &dir, "ibm.1440", 1_474_560);
    // Reads 1 to 3 of track 0.0 lose its second half, read 4 its first: pass
    // 2 reads it twice, which gw joins.
    let lose = r#"["second", "second", "second", "first"]"#;
    let env = serde_json::json!({"FERRITEWEAZLE_PASSES": "2", "LOSE": lose});
    let image = dir.join("x.img");
    let (drive, log) = read_in_passes(&tools, &disk, "ibm.1440", env, &image, &["--retries=1"]);
    assert_eq!(drive["reads"]["0.0"], 4, "{log}");
    let last = reports_on(&log, (0, 0)).pop().expect("track 0.0 reported");
    let decodes = last["codec"]["decodes"].as_array().expect("its decodes");
    let fluxes: std::collections::BTreeSet<u64> =
        decodes.iter().filter_map(|d| d["flux"].as_u64()).collect();
    assert_eq!(fluxes.len(), 4, "each read on its own");
    let cells: std::collections::BTreeSet<usize> = (decodes.iter())
        .filter_map(|d| Some(d["cells"].as_array()?.len()))
        .collect();
    assert_eq!(cells.len(), 1, "no decode of two reads joined: {cells:?}");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_read_whose_times_gw_scales_says_they_are_not_the_disks() {
    let Some(tools) = tools() else { return };
    let dir = scratch("scaled");
    let disk = flux_of(&tools, &dir, "ibm.1440", 1_474_560);
    let read = |options: &[&str]| {
        let (_, log) = read_in_passes(
            &tools,
            &disk,
            "ibm.1440",
            serde_json::json!({}),
            &dir.join("x.img"),
            options,
        );
        let fluxes: Vec<serde_json::Value> = (log.lines())
            .filter_map(|l| l.strip_prefix("@ferriteweazle track "))
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| v.get("flux").cloned())
            .collect();
        assert!(!fluxes.is_empty(), "{log}");
        fluxes
    };
    assert!(
        read(&["--adjust-speed=310rpm"])
            .iter()
            .all(|f| f["scaled"] == true)
    );
    assert!(read(&[]).iter().all(|f| f.get("scaled").is_none()));
    std::fs::remove_dir_all(dir).ok();
}

/// The bridge (argv[1]) converting argv[2] to argv[3] in ibm.360, a hook of
/// its own made to fail as with a gw whose insides it does not know: gw's
/// output, then whether gw's own functions are as they were.
const UNKNOWN_GW: &str = r#"
import runpy, sys
bridge = runpy.run_path(sys.argv[1])
from greaseweazle import track
from greaseweazle.tools import convert
parts = lambda: (track.PLLTrack.__init__, convert.process_input_track, convert.open_input_image)
before = parts()
def failing(converting):
    raise AttributeError('gw has no such part')
bridge['gw'].__globals__['routing'] = failing
bridge['gw'](['convert', '--format=ibm.360', sys.argv[2], sys.argv[3]])
print('as they were' if parts() == before else 'patched', file=sys.stderr)
"#;

#[test]
fn a_gw_the_bridge_cannot_patch_runs_as_it_is_unreported() {
    let Some(tools) = tools() else { return };
    let dir = scratch("unknown-gw");
    let (img, scp) = (dir.join("a.img"), dir.join("a.scp"));
    std::fs::write(&img, vec![0x5a; 368_640]).unwrap();
    let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.py");
    let out = std::process::Command::new(&tools.python)
        .args(["-c", UNKNOWN_GW])
        .arg(&bridge)
        .args([&img, &scp])
        .output()
        .expect("python runs");
    let text = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
    assert!(scp.exists(), "gw converted all the same: {text}");
    assert!(
        text.contains("no reports from this Greaseweazle Tools"),
        "{text}"
    );
    assert!(!text.contains("@ferriteweazle "), "unreported");
    assert!(text.contains("as they were"), "every patch undone");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_kept_pass_is_whole_revolutions_where_the_format_reads_less() {
    let Some(tools) = tools() else { return };
    let dir = scratch("passes-revs");
    let disk = flux_of(&tools, &dir, "commodore.1541", 196_608);
    let read = |env: serde_json::Value| {
        let image = dir.join("disk.d64");
        read_in_passes(&tools, &disk, "commodore.1541", env, &image, &[])
    };
    let (drive, log) = read(serde_json::json!({"FERRITEWEAZLE_PASSES": "2"}));
    let timed = drive["asked"].as_array().unwrap().iter().any(|a| a[1] != 0);
    assert!(timed, "gw's 1.1 revolutions: {log}");
    let keep = dir.join("Read passes").join("Disk pass");
    let env = serde_json::json!({"FERRITEWEAZLE_PASSES": "2", "FERRITEWEAZLE_KEEP": path(&keep)});
    let (drive, log) = read(env);
    assert_eq!(drive["asked"], serde_json::json!([[2, 0]]), "{log}");
    assert!(dir.join("Read passes/Disk pass 1.scp").exists(), "{log}");
    std::fs::remove_dir_all(dir).ok();
}

//! End-to-end tests against real gw: the bundle in target/greaseweazle-bundle if
//! built, else an installed one. They skip when there is neither. None opens
//! a device.

mod common;

use common::{Window, app, app_mut, greaseweazle, line, run_button, squares};
use eframe::egui;
use egui_kittest::kittest::{NodeT, Queryable};
use ferriteweazle::command::quote;
use ferriteweazle::form::{self, Output};
use ferriteweazle::job::{DETECT, Job, Outcome};
use ferriteweazle::presets;
use ferriteweazle::progress::Status;
use ferriteweazle::schema::{Port, Schema};
use ferriteweazle::service::{Load, Service};
use ferriteweazle::tools::{Origin, Tools};
use ferriteweazle::{App, Drawer, Page, Settings};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The bundle in target/greaseweazle-bundle or the folder FERRITEWEAZLE_BUNDLE names (such
/// as another processor's, run emulated), else an installed gw.
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

fn wait<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(t) = ready() {
            return t;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn start(tools: &Tools, command: &str, args: &[&str]) -> Job {
    let args = args.iter().map(|a| a.to_string()).collect();
    Job::start(tools, "Greaseweazle", command, args, Box::new(|| {})).expect("the bridge starts")
}

fn finish(mut job: Job, what: &str) -> Job {
    wait(what, || {
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

/// Prints what the bridge in argv[1] uses that gw's oldest Python, 3.8,
/// lacks: newer syntax fails to parse, and these came in 3.9 and 3.10.
const NEWER_PYTHON: &str = r#"
import ast, sys
tree = ast.parse(open(sys.argv[1]).read(), feature_version=(3, 8))
newer = {'cache', 'get_annotations', 'removeprefix', 'removesuffix'}
print(sorted({n.attr for n in ast.walk(tree) if isinstance(n, ast.Attribute)} & newer))
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
    // Past cylinder 83 gw asks first; answering No ends it before any device is opened.
    let mut job = start(&tools, "seek", &["seek", "90"]);
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
    let mut w = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1240.0, 780.0))
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
    app_mut(w).tool = Some(start(tools, "seek", &["seek", "90"]));
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
    assert!(
        job.progress
            .error
            .as_deref()
            .is_some_and(|e| e.contains("No format")),
        "{:?}",
        job.progress.error
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

    // The list keeps side 1 alone, so a click still changes the sides.
    choose_format(&mut w, "ibm.160");
    until_shown(&mut w, "one side", |w| {
        w.query_by_label_contains("1\u{a0}side").is_some()
    });
    assert_eq!(sides(&w), [(true, false), (true, true)]);
    side(&w, "0").click();
    w.run_steps(2);
    assert_eq!(sides(&w), [(false, true), (false, false)]);

    side(&w, "1").hover();
    until_shown(&mut w, "why side 1 is greyed", |w| {
        w.query_by_label("This format is single sided.").is_some()
    });
    assert!(w.query_by_label("Which tracks to read.").is_none());
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
    assert_eq!(squares(&w).count(), 35, "the blank map is the disk's");

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
    // Why the page cannot run comes before that it needs a device.
    w.get_all_by_role_and_label(egui::accesskit::Role::Button, "Write disk")
        .last()
        .expect("the run button")
        .hover();
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
fn a_write_is_verified_track_by_track_only_in_a_format_gw_can_check() {
    let Some(tools) = tools() else { return };
    let mut service = Service::start(&tools, Box::new(|| {}));
    let schema = wait("the schema", || {
        service.poll();
        service.schema.ready().cloned()
    });
    let mut info = |name: &str| {
        wait(name, || {
            service.poll();
            service.format_info("", name).ready().cloned()
        })
    };
    assert!(info("ibm.1440").verifies);
    assert!(info("amiga.amigados").verifies);
    assert!(!info("raw.250").verifies, "gw cannot check bitcells");
    let mut verifies = |args: &[&str]| {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        form::verifies(&mut service, &schema, &args)
    };
    assert!(verifies(&["write", "--format=ibm.1440", "a.img"]));
    assert!(verifies(&["write", "a.adf"]), "an .adf's own format");
    assert!(!verifies(&[
        "write",
        "--format=ibm.1440",
        "--no-verify",
        "a.img"
    ]));
    assert!(!verifies(&["write", "--format=raw.250", "a.hfe"]));
    assert!(!verifies(&["write", "a.scp"]), "flux written as it is");
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
result = next(l for l in out.getvalue().splitlines() if l.startswith(bridge['RESULT']))
found = json.loads(result[len(bridge['RESULT']):])
print(json.dumps({**found, 'pins': unit.pins, 'seeks': unit.seeks[:3], 'revs': sorted(unit.revs)}))
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

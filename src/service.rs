//! The long-lived bridge that answers questions about gw: its schema,
//! connected devices, disk formats, and whether a value is valid.

use crate::engine::Engine;
use crate::schema::{DiskDefs, FormatInfo, Port, Schema};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

/// How often the list of connected devices is refreshed.
const PORTS_EVERY: Duration = Duration::from_secs(2);
/// How often the files asked about are looked at again.
const WATCH_EVERY: Duration = Duration::from_secs(1);

/// When each file asked about last changed, by path.
type Times = Mutex<HashMap<String, Option<SystemTime>>>;

pub type Repaint = Box<dyn Fn() + Send>;

/// Something asked of the bridge: waiting for a reply, or answered.
pub enum Load<T> {
    Waiting(Pending<T>),
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(t) => Some(t),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Load::Failed(e) => Some(e),
            _ => None,
        }
    }
}

impl<T: DeserializeOwned> Load<T> {
    fn poll(&mut self) -> bool {
        let Load::Waiting(p) = self else { return false };
        let Some(r) = p.poll() else { return false };
        *self = r.map_or_else(Load::Failed, Load::Ready);
        true
    }
}

/// A reply still on its way.
pub struct Pending<T> {
    rx: Receiver<Result<Value, String>>,
    _type: PhantomData<fn() -> T>,
}

impl<T: DeserializeOwned> Pending<T> {
    fn poll(&self) -> Option<Result<T, String>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r.and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("The Greaseweazle engine stopped.".into())),
        }
    }
}

struct Request {
    body: Value,
    reply: Sender<Result<Value, String>>,
}

pub struct Service {
    requests: Sender<Request>,
    pub schema: Load<Schema>,
    ports: Load<Vec<Port>>,
    ports_asked: Instant,
    last_ports: Vec<Port>,
    /// Why the last list of devices failed, if it did.
    ports_error: Option<String>,
    /// Set by `pin_ports`: gw is not asked for devices.
    pinned: bool,
    /// By path and the time the file last changed, so an edit is checked again.
    diskdefs: HashMap<(String, Option<SystemTime>), Load<DiskDefs>>,
    /// Keyed as `diskdefs`.
    image_formats: HashMap<(String, Option<SystemTime>), Load<Option<String>>>,
    /// By disk definitions file, as `diskdefs`, and format name.
    infos: HashMap<(String, Option<SystemTime>, String), Load<FormatInfo>>,
    checks: HashMap<(String, String, String), Load<Option<String>>>,
    /// The files in a folder, keyed as `diskdefs` by the folder's last change.
    folders: HashMap<(String, Option<SystemTime>), Vec<PathBuf>>,
    /// The file times in the keys of `diskdefs`, `image_formats` and
    /// `infos`, which `watch` keeps current.
    times: Arc<Times>,
}

impl Service {
    pub fn start(engine: &Engine, repaint: Repaint) -> Service {
        let (requests, rx) = mpsc::channel();
        let cmd = engine.bridge("serve");
        std::thread::spawn(move || serve(cmd, rx, repaint));
        let schema = Load::Waiting(call(&requests, json!({"op": "schema"})));
        let ports = Load::Waiting(call(&requests, json!({"op": "ports"})));
        Service::new(requests, schema, ports)
    }

    /// A service with no engine behind it. Every question fails, apart from
    /// the schema if one is given.
    pub fn offline(schema: Result<Schema, String>) -> Service {
        let (requests, _) = mpsc::channel();
        let schema = schema.map_or_else(Load::Failed, Load::Ready);
        Service::new(requests, schema, Load::Ready(Vec::new()))
    }

    fn new(requests: Sender<Request>, schema: Load<Schema>, ports: Load<Vec<Port>>) -> Service {
        let times = Arc::default();
        let watched = Arc::downgrade(&times);
        std::thread::spawn(move || watch(&watched));
        Service {
            requests,
            schema,
            ports,
            ports_asked: Instant::now(),
            last_ports: Vec::new(),
            ports_error: None,
            pinned: false,
            diskdefs: HashMap::new(),
            image_formats: HashMap::new(),
            infos: HashMap::new(),
            checks: HashMap::new(),
            folders: HashMap::new(),
            times,
        }
    }

    /// Takes in replies that have arrived.
    pub fn poll(&mut self) {
        self.schema.poll();
        if self.ports.poll() {
            self.ports_error = self.ports.error().map(str::to_owned);
            self.last_ports = self.ports.ready().cloned().unwrap_or_default();
        }
        for load in self.diskdefs.values_mut() {
            load.poll();
        }
        for load in self.image_formats.values_mut() {
            load.poll();
        }
        for load in self.infos.values_mut() {
            load.poll();
        }
        for load in self.checks.values_mut() {
            load.poll();
        }
    }

    /// Every serial port, likeliest Greaseweazle first, refreshed every
    /// PORTS_EVERY.
    pub fn ports(&mut self) -> &[Port] {
        if self.ports_asked.elapsed() > PORTS_EVERY {
            self.refresh_ports();
        }
        &self.last_ports
    }

    /// The devices last listed, without asking again.
    pub fn known_ports(&self) -> &[Port] {
        &self.last_ports
    }

    /// Why gw could not list the devices, such as its engine having stopped.
    pub fn ports_error(&self) -> Option<&str> {
        self.ports_error.as_deref()
    }

    /// Asks for the list of devices now, not when it is next due.
    pub fn refresh_ports(&mut self) {
        if !self.pinned && !matches!(self.ports, Load::Waiting(_)) {
            self.ports = Load::Waiting(call(&self.requests, json!({"op": "ports"})));
            self.ports_asked = Instant::now();
        }
    }

    /// The devices to show until gw first lists them; ignored with no gw to ask.
    pub fn seed_ports(&mut self, ports: Vec<Port>) {
        if matches!(self.ports, Load::Waiting(_)) {
            self.last_ports = ports;
        }
    }

    /// Lists these devices and stops asking gw: a window with a made-up
    /// Greaseweazle, for tests and pictures.
    pub fn pin_ports(&mut self, ports: Vec<Port>) {
        // Drops a reply on its way, which would replace them.
        self.ports = Load::Ready(Vec::new());
        self.last_ports = ports;
        self.ports_error = None;
        self.pinned = true;
    }

    /// gw's own format names.
    pub fn formats(&self) -> Option<&[String]> {
        self.schema.ready().map(|s| s.formats.as_slice())
    }

    /// The formats a disk definitions file adds, and what gw says is wrong with it.
    pub fn diskdefs(&mut self, path: &str) -> &Load<DiskDefs> {
        let key = (path.to_owned(), self.modified(path));
        let requests = &self.requests;
        self.diskdefs.entry(key).or_insert_with(|| {
            Load::Waiting(call(requests, json!({"op": "diskdefs", "path": path})))
        })
    }

    /// The formats a disk definitions file adds, once gw has read it without
    /// fault. Asks gw if it has not read this file yet.
    pub fn custom_formats(&mut self, path: &str) -> &[String] {
        if path.is_empty() {
            return &[];
        }
        match self.diskdefs(path) {
            Load::Ready(d) if d.errors.is_empty() => &d.formats,
            _ => &[],
        }
    }

    /// What gw has said so far about a disk definitions file, without asking.
    pub fn known_diskdefs(&self, path: &str) -> Option<&Load<DiskDefs>> {
        self.diskdefs.get(&(path.to_owned(), self.modified(path)))
    }

    /// As `custom_formats`, from what gw has already said.
    pub fn known_custom_formats(&self, path: &str) -> &[String] {
        match self.known_diskdefs(path) {
            Some(Load::Ready(d)) if d.errors.is_empty() => &d.formats,
            _ => &[],
        }
    }

    /// The format gw takes from an image file when none is chosen, if any.
    /// Asks gw if it has not opened this file yet.
    pub fn image_format(&mut self, path: &str) -> &Load<Option<String>> {
        let key = (path.to_owned(), self.modified(path));
        let requests = &self.requests;
        self.image_formats.entry(key).or_insert_with(|| {
            Load::Waiting(call(requests, json!({"op": "image_format", "path": path})))
        })
    }

    /// As `image_format`, from what gw has already said.
    pub fn known_image_format(&self, path: &str) -> Option<&str> {
        let load = self
            .image_formats
            .get(&(path.to_owned(), self.modified(path)))?;
        load.ready()?.as_deref()
    }

    /// gw's objection to an image file, from what gw has already said.
    pub fn image_fault(&self, path: &str) -> Option<&str> {
        let load = self
            .image_formats
            .get(&(path.to_owned(), self.modified(path)))?;
        load.error()
    }

    /// The files in `folder`, listed again when it changes. The folder is
    /// looked at on each call, not by `watch`, so a file added shows at once.
    pub fn folder(&mut self, folder: &str) -> &[PathBuf] {
        let key = (folder.to_owned(), stat(folder));
        if !self.folders.contains_key(&key) {
            self.folders.retain(|(f, _), _| f != folder);
            let files = std::fs::read_dir(folder).map_or_else(
                |_| Vec::new(),
                |dir| {
                    dir.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_file())
                        .collect()
                },
            );
            self.folders.insert(key.clone(), files);
        }
        &self.folders[&key]
    }

    /// As `folder`, from the last listing.
    pub fn known_folder(&self, folder: &str) -> &[PathBuf] {
        let last = self.folders.iter().find(|((f, _), _)| f == folder);
        last.map_or(&[], |(_, files)| files)
    }

    /// A format's layout, gw's own or from a disk definitions file, which
    /// gw reads again when it changes.
    pub fn format_info(&mut self, diskdefs: &str, name: &str) -> &Load<FormatInfo> {
        let key = (
            diskdefs.to_owned(),
            self.modified(diskdefs),
            name.to_owned(),
        );
        let requests = &self.requests;
        self.infos.entry(key).or_insert_with(|| {
            let diskdefs = (!diskdefs.is_empty()).then_some(diskdefs);
            let body = json!({"op": "format", "name": name, "diskdefs": diskdefs});
            Load::Waiting(call(requests, body))
        })
    }

    /// gw's complaint about a value, if it has one.
    pub fn check(&mut self, command: &str, dest: &str, value: &str) -> Option<&str> {
        let requests = &self.requests;
        self.checks
            .entry((command.to_owned(), dest.to_owned(), value.to_owned()))
            .or_insert_with(|| {
                let body = json!({"op": "check", "command": command, "dest": dest, "value": value});
                Load::Waiting(call(requests, body))
            })
            .ready()
            .and_then(|e| e.as_deref())
    }

    /// When a file last changed, so gw looks at an edited file again. After
    /// the first look it is `watch` that looks, so the window never waits on
    /// a network mount that has stalled.
    fn modified(&self, path: &str) -> Option<SystemTime> {
        if path.is_empty() {
            return None;
        }
        let seen = self.times.lock().ok().and_then(|t| t.get(path).copied());
        seen.unwrap_or_else(|| {
            let time = stat(path);
            if let Ok(mut times) = self.times.lock() {
                times.insert(path.to_owned(), time);
            }
            time
        })
    }
}

fn stat(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Looks again at the files in `times` every WATCH_EVERY, until the Service
/// is dropped. No lock is held while the file system answers. The window
/// draws at least every PORTS_EVERY (see `serve`), and so sees a change.
fn watch(times: &Weak<Times>) {
    loop {
        std::thread::sleep(WATCH_EVERY);
        let Some(live) = times.upgrade() else {
            return;
        };
        let paths: Vec<String> = match live.lock() {
            Ok(known) => known.keys().cloned().collect(),
            Err(_) => return,
        };
        let seen: Vec<_> = paths
            .into_iter()
            .map(|p| {
                let time = stat(&p);
                (p, time)
            })
            .collect();
        if let Ok(mut known) = live.lock() {
            known.extend(seen);
        }
    }
}

fn call<T>(requests: &Sender<Request>, body: Value) -> Pending<T> {
    let (reply, rx) = mpsc::channel();
    let _ = requests.send(Request { body, reply });
    Pending {
        rx,
        _type: PhantomData,
    }
}

/// Runs the bridge and answers requests in order until the Service is
/// dropped. With nothing asked for PORTS_EVERY it wakes the window, which
/// asks for the devices only when it draws.
fn serve(mut cmd: Command, requests: Receiver<Request>, repaint: Repaint) {
    let spawned = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            let why = format!("Could not start {}: {e}", cmd.get_program().display());
            return refuse(requests, &repaint, why);
        }
    };
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    // Drained all along: a full pipe would stall the bridge.
    let stderr = child.stderr.take().expect("stderr is piped");
    let last_words = std::thread::spawn(move || last_line(stderr));
    let mut line = String::new();
    loop {
        let r = match requests.recv_timeout(PORTS_EVERY) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => {
                repaint();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        line.clear();
        let sent = writeln!(stdin, "{}", r.body).and_then(|()| stdin.flush());
        if let Ok(1..) = sent.and_then(|()| stdout.read_line(&mut line)) {
            let _ = r.reply.send(parse(&line));
            repaint();
            continue;
        }
        let _ = child.kill();
        let _ = child.wait();
        let why = match last_words.join().unwrap_or_default() {
            last if last.is_empty() => "The Greaseweazle engine stopped.".to_owned(),
            last => format!("The Greaseweazle engine stopped: {}", last.trim()),
        };
        let _ = r.reply.send(Err(why.clone()));
        return refuse(requests, &repaint, why);
    }
    // The bridge ends when its input closes; waiting reaps it.
    drop(stdin);
    let _ = child.wait();
}

/// Answers every request with the same error.
fn refuse(requests: Receiver<Request>, repaint: &Repaint, why: String) {
    for r in requests {
        let _ = r.reply.send(Err(why.clone()));
        repaint();
    }
}

fn last_line(from: impl Read) -> String {
    BufReader::new(from)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .last()
        .unwrap_or_default()
}

fn parse(line: &str) -> Result<Value, String> {
    let mut reply: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    match reply.get("error").and_then(Value::as_str) {
        Some(e) => Err(e.to_owned()),
        None => Ok(reply["ok"].take()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Origin;

    /// An empty folder of its own for `test`.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait_for<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(t) = ready() {
                return t;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "no {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A bridge that answers every request with an empty list, and writes its
    /// process id beside itself.
    #[cfg(unix)]
    fn fake_engine(dir: &std::path::Path) -> Engine {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("bridge");
        let text =
            "#!/bin/sh\necho $$ > \"$0.pid\"\nwhile read -r line; do echo '{\"ok\": []}'; done\n";
        std::fs::write(&script, text).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Engine {
            python: script,
            origin: Origin::Custom,
        }
    }

    #[test]
    fn a_failed_device_list_keeps_its_reason() {
        let engine = Engine {
            python: std::env::temp_dir().join("ferriteweazle-no-such-python"),
            origin: Origin::Custom,
        };
        let mut service = Service::start(&engine, Box::new(|| {}));
        let why = wait_for("reason", || {
            service.poll();
            service.ports_error().map(str::to_owned)
        });
        assert!(why.starts_with("Could not start "), "{why}");
        assert!(service.known_ports().is_empty());
    }

    #[test]
    fn the_window_never_waits_on_a_file_it_has_looked_at_once() {
        let dir = scratch("watch");
        let file = dir.join("Disk.img");
        std::fs::write(&file, "").unwrap();
        let path = file.to_string_lossy();
        let service = Service::offline(Err(String::new()));
        let seen = service.modified(&path);
        assert!(seen.is_some());
        std::fs::remove_file(&file).unwrap();
        assert_eq!(
            service.modified(&path),
            seen,
            "the window does not look again"
        );
        wait_for("the file to go", || {
            service.modified(&path).is_none().then_some(())
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn an_idle_window_is_woken_when_the_devices_are_due() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = scratch("idle");
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        let repaint = Box::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
        let _service = Service::start(&fake_engine(&dir), repaint);
        // One wake for each of the first two replies, then one with nothing asked.
        wait_for("a wake", || {
            (wakes.load(Ordering::SeqCst) > 2).then_some(())
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_dropped_service_leaves_no_bridge_behind() {
        let dir = scratch("reaped");
        let engine = fake_engine(&dir);
        let mut service = Service::start(&engine, Box::new(|| {}));
        wait_for("the devices", || {
            service.poll();
            service.ports.ready().map(|_| ())
        });
        let pid = std::fs::read_to_string(dir.join("bridge.pid")).unwrap();
        drop(service);
        wait_for("the bridge to be reaped", || {
            let ps = Command::new("ps")
                .args(["-o", "stat=", "-p", pid.trim()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&ps.stdout)
                .trim()
                .is_empty()
                .then_some(())
        });
        std::fs::remove_dir_all(dir).ok();
    }
}

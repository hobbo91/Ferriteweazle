//! The long-lived bridge that answers questions about gw: its schema,
//! connected devices, disk formats, and whether a value is valid.

use crate::schema::{DiskDefs, FormatInfo, Port, Schema};
use crate::standalone;
use crate::tools::Tools;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

/// How often the bridge's thread lists the connected devices while the
/// window asks nothing.
const PORTS_EVERY: Duration = Duration::from_secs(2);
/// How often the files asked about are looked at again.
const WATCH_EVERY: Duration = Duration::from_secs(1);
/// How long a folder's listing stands while the folder's time is unchanged.
/// FAT keeps times to 2 s and Windows may leave a folder's alone, so a file
/// added soon after a listing need not change it.
const FOLDER_EVERY: Duration = Duration::from_millis(500);

/// When each file asked about last changed, by path.
type Times = Mutex<HashMap<String, Option<SystemTime>>>;

pub type Repaint = Box<dyn Fn() + Send>;

/// A list of the devices as gw gives it, or why it could not.
type Listed = Result<Value, String>;

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
            Err(TryRecvError::Disconnected) => Some(Err("Greaseweazle Tools stopped.".into())),
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
    /// Every list of devices: those asked for, and those the bridge's thread
    /// finds changed on its own.
    listed: Receiver<Listed>,
    /// Where a list asked for is sent.
    lister: Sender<Listed>,
    /// A list asked for and not yet answered.
    asking: bool,
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
    /// The last folder listed: its time then, when, and its files.
    folders: HashMap<String, (Option<SystemTime>, Instant, Vec<PathBuf>)>,
    /// The file times in the keys of `diskdefs`, `image_formats` and
    /// `infos`, which `watch` keeps current.
    times: Arc<Times>,
    /// gw's objections, or none, by the request that asked for them.
    objections: HashMap<String, Load<Option<String>>>,
}

impl Service {
    pub fn start(tools: &Tools, repaint: Repaint) -> Service {
        let (requests, rx) = mpsc::channel();
        let (lister, listed) = mpsc::channel();
        let found = lister.clone();
        match tools.standalone {
            true => {
                let gw = tools.python.clone();
                std::thread::spawn(move || serve_standalone(&gw, rx, found, repaint));
            }
            false => {
                let cmd = tools.bridge("serve");
                std::thread::spawn(move || serve(cmd, rx, found, repaint));
            }
        }
        let schema = Load::Waiting(call(&requests, json!({"op": "schema"})));
        let mut service = Service::new(requests, schema, lister, listed);
        service.refresh_ports();
        service
    }

    /// A service with no Greaseweazle Tools behind it: every question fails but the schema, if given.
    pub fn offline(schema: Result<Schema, String>) -> Service {
        let (requests, _) = mpsc::channel();
        let (lister, listed) = mpsc::channel();
        let schema = schema.map_or_else(Load::Failed, Load::Ready);
        Service::new(requests, schema, lister, listed)
    }

    fn new(
        requests: Sender<Request>,
        schema: Load<Schema>,
        lister: Sender<Listed>,
        listed: Receiver<Listed>,
    ) -> Service {
        let times = Arc::default();
        let watched = Arc::downgrade(&times);
        std::thread::spawn(move || watch(&watched));
        Service {
            requests,
            schema,
            listed,
            lister,
            asking: false,
            last_ports: Vec::new(),
            ports_error: None,
            pinned: false,
            diskdefs: HashMap::new(),
            image_formats: HashMap::new(),
            infos: HashMap::new(),
            checks: HashMap::new(),
            folders: HashMap::new(),
            times,
            objections: HashMap::new(),
        }
    }

    /// Takes in replies that have arrived.
    pub fn poll(&mut self) {
        self.schema.poll();
        while let Ok(list) = self.listed.try_recv() {
            self.asking = false;
            if self.pinned {
                continue;
            }
            let ports: Result<Vec<Port>, String> =
                list.and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()));
            self.ports_error = ports.as_ref().err().cloned();
            self.last_ports = ports.unwrap_or_default();
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
        for load in self.objections.values_mut() {
            load.poll();
        }
    }

    /// Every serial port as last listed, likeliest Greaseweazle first. The
    /// bridge's thread lists them again every PORTS_EVERY and sends a changed
    /// list, which `poll` takes in.
    pub fn known_ports(&self) -> &[Port] {
        &self.last_ports
    }

    /// Why gw could not list the devices, such as its Python having stopped.
    pub fn ports_error(&self) -> Option<&str> {
        self.ports_error.as_deref()
    }

    /// Asks for the list of devices now, not when the thread next lists them.
    pub fn refresh_ports(&mut self) {
        if !self.pinned && !self.asking {
            let request = Request {
                body: json!({"op": "ports"}),
                reply: self.lister.clone(),
            };
            // Not sent with no gw to ask, so the list stays as it is.
            self.asking = self.requests.send(request).is_ok();
        }
    }

    /// The devices to show until gw first lists them; ignored with no gw to ask.
    pub fn seed_ports(&mut self, ports: Vec<Port>) {
        if self.asking {
            self.last_ports = ports;
        }
    }

    /// Lists these devices and takes no more lists from gw: a window with a
    /// made-up Greaseweazle, for tests and pictures.
    pub fn pin_ports(&mut self, ports: Vec<Port>) {
        self.asking = false;
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

    /// The formats a disk definitions file adds that gw can use. Asks gw if
    /// it has not read this file yet.
    pub fn custom_formats(&mut self, path: &str) -> &[String] {
        if path.is_empty() {
            return &[];
        }
        match self.diskdefs(path) {
            Load::Ready(d) => &d.formats,
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
            Some(Load::Ready(d)) => &d.formats,
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
        let time = stat(folder);
        let fresh = self
            .folders
            .get(folder)
            .is_some_and(|(then, at, _)| *then == time && at.elapsed() < FOLDER_EVERY);
        if !fresh {
            let files = std::fs::read_dir(folder).map_or_else(
                |_| Vec::new(),
                |dir| {
                    dir.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_file())
                        .collect()
                },
            );
            self.folders.clear();
            self.folders
                .insert(folder.to_owned(), (time, Instant::now(), files));
        }
        &self.folders[folder].2
    }

    /// As `folder`, from the last listing.
    pub fn known_folder(&self, folder: &str) -> &[PathBuf] {
        self.folders.get(folder).map_or(&[], |(_, _, files)| files)
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

    /// When a file last changed, so gw looks at an edited file again. After the
    /// first look `watch` looks, so the window never waits on a stalled network mount.
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

    /// gw's objection to an image of type `ext` in `format`, or none, as gw
    /// finds when it makes one in memory. Asks gw if it has not tried yet.
    pub fn fits(&mut self, diskdefs: &str, format: &str, ext: &str) -> &Load<Option<String>> {
        self.ask(fits_body(diskdefs, format, ext))
    }

    /// gw's complaint about a value of an image type's option, if it has one.
    pub fn check_opt(&mut self, ext: &str, name: &str, value: &str) -> Option<&str> {
        let body = json!({"op": "check_opt", "ext": ext, "name": name, "value": value});
        self.ask(body).ready().and_then(|e| e.as_deref())
    }

    /// What gw objects to in `body`'s request, asked once.
    fn ask(&mut self, body: Value) -> &Load<Option<String>> {
        let requests = &self.requests;
        self.objections
            .entry(body.to_string())
            .or_insert_with(|| Load::Waiting(call(requests, body)))
    }

    /// Takes a format's size as if gw had given it, for tests.
    #[cfg(test)]
    pub fn describe(&mut self, name: &str, cyls: u32, heads: u32) {
        let info = FormatInfo {
            cyls,
            heads,
            encoding: None,
            sectors: None,
            bytes: None,
            verifies: false,
            revs: None,
        };
        let key = (String::new(), None, name.to_owned());
        self.infos.insert(key, Load::Ready(info));
    }

    /// Gives a described format the revolutions gw read takes per track.
    #[cfg(test)]
    pub fn describe_revs(&mut self, name: &str, revs: f64) {
        let key = (String::new(), None, name.to_owned());
        if let Some(Load::Ready(info)) = self.infos.get_mut(&key) {
            info.revs = Some(revs);
        }
    }

    /// As `format_info`, from what gw has already said.
    pub fn known_format_info(&self, diskdefs: &str, name: &str) -> Option<&Load<FormatInfo>> {
        let key = (
            diskdefs.to_owned(),
            self.modified(diskdefs),
            name.to_owned(),
        );
        self.infos.get(&key)
    }

    /// As `fits`, the objection alone, from what gw has already said.
    pub fn known_fits(&self, diskdefs: &str, format: &str, ext: &str) -> Option<&str> {
        let load = self
            .objections
            .get(&fits_body(diskdefs, format, ext).to_string())?;
        load.ready()?.as_deref()
    }
}

fn fits_body(diskdefs: &str, format: &str, ext: &str) -> Value {
    let diskdefs = (!diskdefs.is_empty()).then_some(diskdefs);
    json!({"op": "fits", "ext": ext, "name": format, "diskdefs": diskdefs})
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

/// The bridge thread's own listing of the devices: the last list it or the
/// window got, and when the next is due. A device plugged in or taken out
/// wakes the window; an idle window with nothing changing is left alone.
struct Listing {
    last: Option<Listed>,
    due: Instant,
    /// The serial device nodes as they were when `last` was listed.
    nodes: Option<Vec<Node>>,
}

impl Listing {
    fn new() -> Listing {
        Listing {
            last: None,
            due: Instant::now() + PORTS_EVERY,
            nodes: None,
        }
    }

    fn wait(&self) -> Duration {
        self.due.saturating_duration_since(Instant::now())
    }

    /// A list the window asked for, which the next own listing is compared with.
    fn asked(&mut self, list: &Listed) {
        self.last = Some(list.clone());
        self.nodes = serial_nodes();
        self.due = Instant::now() + PORTS_EVERY;
    }

    /// Whether gw is to list the devices now: when due, unless on macOS the
    /// serial device nodes are as they were, as gw's list there comes from
    /// them alone. Elsewhere the list can change without them, as when Linux
    /// grants access to one, so gw is asked.
    fn due(&mut self) -> bool {
        if self.wait() > Duration::ZERO {
            return false;
        }
        let nodes = serial_nodes();
        let same = nodes.is_some() && nodes == self.nodes;
        self.nodes = nodes;
        if same {
            self.due = Instant::now() + PORTS_EVERY;
        }
        !same
    }

    /// A list made on its own: sent to `found`, and the window woken, when
    /// it differs from the last.
    fn found(&mut self, list: Listed, found: &Sender<Listed>, repaint: &Repaint) {
        self.due = Instant::now() + PORTS_EVERY;
        if self.last.as_ref() != Some(&list) {
            let _ = found.send(list.clone());
            repaint();
            self.last = Some(list);
        }
    }
}

/// A serial device node's name.
type Node = std::ffi::OsString;

/// The serial device nodes as they are now, or none where gw's list is not
/// drawn from them alone. The unit tests' bridges list what they like,
/// whatever the system holds, so they are not looked at there.
fn serial_nodes() -> Option<Vec<Node>> {
    match cfg!(test) {
        true => None,
        false => system_nodes(),
    }
}

/// macOS's serial device nodes: /dev/cu.*, the callout devices pyserial
/// lists, whose name, product and USB ID are all gw reports there.
#[cfg(target_os = "macos")]
fn system_nodes() -> Option<Vec<Node>> {
    let mut nodes: Vec<Node> = std::fs::read_dir("/dev")
        .ok()?
        .flatten()
        .map(|e| e.file_name())
        .filter(|name| name.to_string_lossy().starts_with("cu."))
        .collect();
    nodes.sort();
    Some(nodes)
}

#[cfg(not(target_os = "macos"))]
fn system_nodes() -> Option<Vec<Node>> {
    None
}

/// Runs the bridge and answers requests in order until the Service is
/// dropped. Between requests it lists the devices itself every PORTS_EVERY,
/// and sends a list that differs from the last to `found` and wakes the
/// window.
fn serve(mut cmd: Command, requests: Receiver<Request>, found: Sender<Listed>, repaint: Repaint) {
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
    // Sends a request and reads its reply; none once the bridge has stopped.
    let mut ask = |body: &Value| -> Option<Listed> {
        line.clear();
        let sent = writeln!(stdin, "{body}").and_then(|()| stdin.flush());
        match sent.and_then(|()| stdout.read_line(&mut line)) {
            Ok(1..) => Some(parse(&line)),
            _ => None,
        }
    };
    let mut listing = Listing::new();
    // The request the bridge stopped on, if it did.
    let mut unanswered = None;
    let stopped = loop {
        if listing.due() {
            let Some(list) = ask(&json!({"op": "ports"})) else {
                break true;
            };
            listing.found(list, &found, &repaint);
            continue;
        }
        let r = match requests.recv_timeout(listing.wait()) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break false,
        };
        let Some(reply) = ask(&r.body) else {
            unanswered = Some(r);
            break true;
        };
        if r.body["op"] == "ports" {
            listing.asked(&reply);
        }
        let _ = r.reply.send(reply);
        repaint();
    };
    if stopped {
        let _ = child.kill();
        let _ = child.wait();
        let why = match last_words.join().unwrap_or_default() {
            last if last.is_empty() => "Greaseweazle Tools stopped.".to_owned(),
            last => format!("Greaseweazle Tools stopped: {}", last.trim()),
        };
        if let Some(r) = unanswered {
            let _ = r.reply.send(Err(why.clone()));
        }
        let _ = found.send(Err(why.clone()));
        return refuse(requests, &repaint, why);
    }
    // The bridge ends when its input closes; waiting reaps it.
    drop(stdin);
    let _ = child.wait();
}

/// Answers for a standalone gw, which has no bridge: its schema from its help,
/// the serial ports Windows lists, and no objections. What only the bridge
/// can tell, such as a format's layout, stays unanswered.
fn serve_standalone(
    gw: &Path,
    requests: Receiver<Request>,
    found: Sender<Listed>,
    repaint: Repaint,
) {
    let mut unanswered = Vec::new();
    let mut listing = Listing::new();
    loop {
        if listing.due() {
            listing.found(Ok(json!(standalone::ports())), &found, &repaint);
            continue;
        }
        let r = match requests.recv_timeout(listing.wait()) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let reply = match r.body["op"].as_str() {
            Some("schema") => standalone::schema(gw)
                .and_then(|s| serde_json::to_value(s).map_err(|e| e.to_string())),
            Some("ports") => {
                let list = Ok(json!(standalone::ports()));
                listing.asked(&list);
                list
            }
            Some("check" | "check_opt" | "fits") => Ok(Value::Null),
            _ => {
                unanswered.push(r.reply);
                continue;
            }
        };
        let _ = r.reply.send(reply);
        repaint();
    }
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
    use crate::tools::Origin;

    // A file added in the same tick as a listing leaves the folder's time as
    // it was; here the time is set back to show that.
    #[cfg(unix)]
    #[test]
    fn a_folder_is_listed_again_when_its_time_has_not_changed() {
        let dir = scratch("folder-tick");
        let folder = dir.to_string_lossy().into_owned();
        let mut service = Service::offline(Err(String::new()));
        assert!(service.folder(&folder).is_empty());
        let before = std::fs::metadata(&dir).unwrap().modified().unwrap();
        std::fs::write(dir.join("Game.adf"), "").unwrap();
        std::fs::File::open(&dir)
            .unwrap()
            .set_modified(before)
            .unwrap();
        std::thread::sleep(FOLDER_EVERY);
        assert_eq!(service.folder(&folder), [dir.join("Game.adf")]);
        std::fs::remove_dir_all(dir).ok();
    }

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

    /// A bridge that answers every request with an empty list, until the
    /// `from`th, from which it lists a Greaseweazle (0: never), and writes
    /// its process id beside itself.
    #[cfg(unix)]
    fn fake_tools(dir: &std::path::Path, from: usize) -> Tools {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("bridge");
        let device =
            r#"{"device": "/dev/ttyACM0", "name": "Greaseweazle", "score": 20, "denied": false}"#;
        let text = format!(
            "#!/bin/sh\necho $$ > \"$0.pid\"\nn=0\nwhile read -r line; do\n  n=$((n+1))\n  \
             if [ {from} -gt 0 ] && [ $n -ge {from} ]; then echo '{{\"ok\": [{device}]}}'; \
             else echo '{{\"ok\": []}}'; fi\ndone\n"
        );
        std::fs::write(&script, text).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Tools {
            python: script,
            origin: Origin::Custom,
            standalone: false,
        }
    }

    /// A standalone gw that prints gw 1.23's help as recorded.
    #[cfg(unix)]
    fn recorded_gw(dir: &std::path::Path) -> Tools {
        use std::os::unix::fs::PermissionsExt;
        let help = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/help-1.23");
        let text = format!(
            "#!/bin/sh\nargs=\"$*\"\ncase \"$args\" in\n  \
             --help) cat '{help}/gw.txt' >&2 ;;\n  \
             info*) echo 'Host Tools: 1.23' >&2 ;;\n  \
             *) cat \"{help}/${{args% --help}}.txt\" >&2 ;;\nesac\n"
        );
        let script = dir.join("gw");
        std::fs::write(&script, text).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Tools::find(Some(&script)).expect("a program of its own")
    }

    #[test]
    #[cfg(unix)]
    fn a_standalone_gw_is_read_from_its_help_and_not_asked_what_only_the_bridge_knows() {
        let dir = scratch("standalone-service");
        let gw = recorded_gw(&dir);
        assert!(gw.standalone);
        let mut service = Service::start(&gw, Box::new(|| {}));
        let schema = wait_for("the schema", || {
            service.poll();
            service.schema.ready().cloned()
        });
        assert_eq!(schema.gw(), "gw 1.23");
        assert_eq!(schema.commands.len(), 14);
        assert_eq!(
            schema.images[".adf"].default_format.as_deref(),
            Some("amiga.amigados")
        );
        // Asked in order: the format's layout, then whether it fits an image.
        service.format_info("", "ibm.1440");
        let fits = wait_for("an answer", || {
            service.poll();
            service.fits("", "ibm.1440", ".img").ready().cloned()
        });
        assert_eq!(fits, None, "no objection");
        let layout = service.known_format_info("", "ibm.1440");
        assert!(matches!(layout, Some(Load::Waiting(_))), "left unanswered");
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_serial_device_nodes_are_named_as_pyserial_lists_them() {
        let nodes = system_nodes().expect("/dev is readable");
        let names: Vec<String> = nodes
            .iter()
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| n.starts_with("cu.")), "{names:?}");
        assert!(names.is_sorted(), "{names:?}");
        assert_eq!(
            system_nodes().as_ref(),
            Some(&nodes),
            "the same until a device changes"
        );
    }

    #[test]
    fn a_failed_device_list_keeps_its_reason() {
        let tools = Tools {
            python: std::env::temp_dir().join("ferriteweazle-no-such-python"),
            origin: Origin::Custom,
            standalone: false,
        };
        let mut service = Service::start(&tools, Box::new(|| {}));
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
    fn an_idle_window_is_woken_only_when_the_devices_change() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = scratch("idle");
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        let repaint = Box::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
        // The schema and the devices are asked for, then the bridge's thread
        // lists them itself: the third list has the Greaseweazle.
        let mut service = Service::start(&fake_tools(&dir, 3), repaint);
        wait_for("the Greaseweazle", || {
            service.poll();
            (!service.known_ports().is_empty()).then_some(())
        });
        assert_eq!(service.known_ports()[0].device, "/dev/ttyACM0");
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            3,
            "two replies, then the change"
        );
        // Listed again and again, unchanged: no more wakes.
        std::thread::sleep(2 * PORTS_EVERY + Duration::from_millis(500));
        service.poll();
        assert_eq!(wakes.load(Ordering::SeqCst), 3);
        assert_eq!(service.known_ports().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_dropped_service_leaves_no_bridge_behind() {
        let dir = scratch("reaped");
        let tools = fake_tools(&dir, 0);
        let mut service = Service::start(&tools, Box::new(|| {}));
        wait_for("the devices", || {
            service.poll();
            (!service.asking).then_some(())
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

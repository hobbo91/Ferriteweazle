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
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

/// How often the list of connected devices is refreshed.
const PORTS_EVERY: Duration = Duration::from_secs(2);

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
    /// The devices are set by `pin_ports`, and gw is no longer asked.
    pinned: bool,
    /// By path and the time the file last changed, so an edit is checked again.
    diskdefs: HashMap<(String, Option<SystemTime>), Load<DiskDefs>>,
    /// Keyed as `diskdefs`.
    image_formats: HashMap<(String, Option<SystemTime>), Load<Option<String>>>,
    infos: HashMap<(String, String), Load<FormatInfo>>,
    checks: HashMap<(String, String, String), Load<Option<String>>>,
    /// The files in a folder, keyed as `diskdefs` by the folder's last change.
    folders: HashMap<(String, Option<SystemTime>), Vec<PathBuf>>,
    /// gw's objections, or none, by the request that asked for them.
    objections: HashMap<String, Load<Option<String>>>,
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
        Service {
            requests,
            schema,
            ports,
            ports_asked: Instant::now(),
            last_ports: Vec::new(),
            pinned: false,
            diskdefs: HashMap::new(),
            image_formats: HashMap::new(),
            infos: HashMap::new(),
            checks: HashMap::new(),
            folders: HashMap::new(),
            objections: HashMap::new(),
        }
    }

    /// Takes in replies that have arrived. Returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = self.schema.poll();
        if self.ports.poll() {
            let now = self.ports.ready().cloned().unwrap_or_default();
            changed |= now != self.last_ports;
            self.last_ports = now;
        }
        for load in self.diskdefs.values_mut() {
            changed |= load.poll();
        }
        for load in self.image_formats.values_mut() {
            changed |= load.poll();
        }
        for load in self.infos.values_mut() {
            changed |= load.poll();
        }
        for load in self.checks.values_mut() {
            changed |= load.poll();
        }
        for load in self.objections.values_mut() {
            changed |= load.poll();
        }
        changed
    }

    /// Connected Greaseweazles, best match first, refreshed every few seconds.
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

    /// Asks for the list of devices now, not when it is next due.
    pub fn refresh_ports(&mut self) {
        if !self.pinned && !matches!(self.ports, Load::Waiting(_)) {
            self.ports = Load::Waiting(call(&self.requests, json!({"op": "ports"})));
            self.ports_asked = Instant::now();
        }
    }

    /// The devices to show until gw first lists them. None with no gw to ask.
    pub fn seed_ports(&mut self, ports: Vec<Port>) {
        if matches!(self.ports, Load::Waiting(_)) {
            self.last_ports = ports;
        }
    }

    /// Lists these devices from now on, and no longer asks gw: a window
    /// with a made-up Greaseweazle, for tests and pictures.
    pub fn pin_ports(&mut self, ports: Vec<Port>) {
        // Drops a reply on its way, which would replace them.
        self.ports = Load::Ready(Vec::new());
        self.last_ports = ports;
        self.pinned = true;
    }

    /// gw's own format names.
    pub fn formats(&self) -> Option<&[String]> {
        self.schema.ready().map(|s| s.formats.as_slice())
    }

    /// The formats a disk definitions file adds, and what gw says is wrong with it.
    pub fn diskdefs(&mut self, path: &str) -> &Load<DiskDefs> {
        let requests = &self.requests;
        self.diskdefs
            .entry((path.to_owned(), modified(path)))
            .or_insert_with(|| {
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
        self.diskdefs.get(&(path.to_owned(), modified(path)))
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
        let requests = &self.requests;
        self.image_formats
            .entry((path.to_owned(), modified(path)))
            .or_insert_with(|| {
                Load::Waiting(call(requests, json!({"op": "image_format", "path": path})))
            })
    }

    /// As `image_format`, from what gw has already said.
    pub fn known_image_format(&self, path: &str) -> Option<&str> {
        let load = self.image_formats.get(&(path.to_owned(), modified(path)))?;
        load.ready()?.as_deref()
    }

    /// gw's objection to an image file, from what gw has already said.
    pub fn image_fault(&self, path: &str) -> Option<&str> {
        let load = self.image_formats.get(&(path.to_owned(), modified(path)))?;
        load.error()
    }

    /// The files in `folder`, listed again when it changes.
    pub fn folder(&mut self, folder: &str) -> &[PathBuf] {
        let key = (folder.to_owned(), modified(folder));
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
        let key = (folder.to_owned(), modified(folder));
        self.folders.get(&key).map_or(&[], Vec::as_slice)
    }

    pub fn format_info(&mut self, diskdefs: &str, name: &str) -> &Load<FormatInfo> {
        let requests = &self.requests;
        self.infos
            .entry((diskdefs.to_owned(), name.to_owned()))
            .or_insert_with(|| {
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

    /// gw's objection to an image of type `ext` in `format`, or none, as gw
    /// finds when it makes one in memory. Asks gw if it has not tried yet.
    pub fn fits(&mut self, diskdefs: &str, format: &str, ext: &str) -> &Load<Option<String>> {
        let body = fits_body(diskdefs, format, ext);
        let requests = &self.requests;
        self.objections
            .entry(body.to_string())
            .or_insert_with(|| Load::Waiting(call(requests, body)))
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

/// When a file last changed, so gw looks at an edited file again.
fn modified(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn call<T>(requests: &Sender<Request>, body: Value) -> Pending<T> {
    let (reply, rx) = mpsc::channel();
    let _ = requests.send(Request { body, reply });
    Pending {
        rx,
        _type: PhantomData,
    }
}

/// Runs the bridge and answers requests in order until the Service is dropped.
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
    while let Ok(r) = requests.recv() {
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

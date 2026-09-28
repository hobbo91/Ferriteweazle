//! One gw command, run through the bridge, with its output as it arrives.

use crate::engine::Engine;
use crate::progress::Progress;
use crate::service::Repaint;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

// Line prefixes for gw's questions and the bridge's result. Must match bridge.py.
const ASK: &str = "@ferriteweazle ask ";
const RESULT: &str = "@ferriteweazle result ";

/// The bridge's own command that finds a disk's format.
pub const DETECT: &str = "detect";

/// How long a stopped job has to wind down before it is killed.
const GRACE: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    Failed,
    Stopped,
}

pub struct Job {
    /// The gw command, such as `read`.
    pub command: String,
    pub args: Vec<String>,
    pub log: Vec<String>,
    pub progress: Progress,
    /// A question gw is waiting on.
    pub question: Option<String>,
    pub started: Instant,
    pub ended: Option<(Instant, Outcome)>,
    /// The image the job makes.
    pub output: Option<PathBuf>,
    /// The job's disk format, if known.
    pub format: Option<String>,
    /// Formats that read the disk in full, best first, from a detect job.
    pub detected: Vec<String>,
    /// The head step the detected disk needs: 2 for a 40-track disk in an
    /// 80-track drive.
    pub step: u32,
    /// Which disk of a session this is, counting from 1, and how many.
    pub part: Option<(usize, usize)>,
    stopping: Option<Instant>,
    /// None for a job replayed from its output.
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    eof: bool,
}

impl Job {
    /// Runs `gw ARGS`, or with [`DETECT`] finds the format of the disk they name.
    pub fn start(
        engine: &Engine,
        command: &str,
        args: Vec<String>,
        repaint: Repaint,
    ) -> std::io::Result<Job> {
        let mode = if command == DETECT { "detect" } else { "run" };
        let mut child = engine
            .bridge(mode)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let stderr = child.stderr.take().expect("stderr is piped");
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buf = Vec::new();
            while matches!(reader.read_until(b'\n', &mut buf), Ok(1..)) {
                let line = String::from_utf8_lossy(&buf)
                    .trim_end_matches(['\n', '\r'])
                    .to_owned();
                buf.clear();
                if tx.send(line).is_err() {
                    break;
                }
                repaint();
            }
            drop(tx); // before the repaint, so the poll it wakes sees the end
            repaint();
        });
        Ok(Job {
            child: Some(child),
            stdin,
            ..Job::new(command, args, lines)
        })
    }

    /// A finished job rebuilt from gw's output, as saved from an earlier run.
    pub fn replay(command: &str, log: &str) -> Job {
        let mut job = Job {
            eof: true,
            ..Job::new(command, Vec::new(), mpsc::channel().1)
        };
        log.lines().for_each(|l| job.take(l.to_owned()));
        let outcome = match job.progress.error {
            Some(_) => Outcome::Failed,
            None => Outcome::Succeeded,
        };
        job.ended = Some((job.started, outcome));
        job.progress.finish();
        job
    }

    pub fn running(&self) -> bool {
        self.ended.is_none()
    }

    pub fn stopping(&self) -> bool {
        self.stopping.is_some() && self.running()
    }

    pub fn outcome(&self) -> Option<Outcome> {
        self.ended.map(|(_, o)| o)
    }

    pub fn elapsed(&self) -> Duration {
        self.ended.map_or_else(Instant::now, |(t, _)| t) - self.started
    }

    /// When to look again while nothing new arrives: the clock ticks, and an
    /// ending process is not always reaped by the time its output closes.
    pub fn wake_in(&self) -> Option<Duration> {
        match (self.running(), self.eof || self.stopping.is_some()) {
            (false, _) => None,
            (true, true) => Some(Duration::from_millis(50)),
            (true, false) => Some(Duration::from_secs(1)),
        }
    }

    /// Takes in new output and notices the end. Call every frame.
    pub fn poll(&mut self) {
        loop {
            match self.lines.try_recv() {
                Ok(line) => self.take(line),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.eof = true;
                    break;
                }
            }
        }
        let Some(child) = &mut self.child else { return };
        if self.stopping.is_some_and(|t| t.elapsed() > GRACE) {
            let _ = child.kill();
        }
        if self.eof
            && self.ended.is_none()
            && let Ok(Some(status)) = child.try_wait()
        {
            let outcome = if self.stopping.is_some() {
                Outcome::Stopped
            } else if status.success() && self.progress.error.is_none() {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            };
            self.ended = Some((Instant::now(), outcome));
            self.progress.finish();
            self.question = None;
        }
    }

    /// Asks gw to stop. It turns the motor off, and `read` keeps what it has.
    pub fn stop(&mut self) {
        if self.running() && self.stopping.is_none() {
            self.stdin = None; // the bridge stops gw when its input closes
            self.stopping = Some(Instant::now());
            self.question = None;
        }
    }

    pub fn answer(&mut self, text: &str) {
        if let Some(stdin) = &mut self.stdin {
            let _ = writeln!(stdin, "answer {text}");
        }
        self.question = None;
    }

    fn new(command: &str, args: Vec<String>, lines: Receiver<String>) -> Job {
        Job {
            command: command.to_owned(),
            args,
            log: Vec::new(),
            progress: Progress::default(),
            question: None,
            started: Instant::now(),
            ended: None,
            output: None,
            format: None,
            detected: Vec::new(),
            step: 1,
            part: None,
            stopping: None,
            child: None,
            stdin: None,
            lines,
            eof: false,
        }
    }

    fn take(&mut self, line: String) {
        if let Some(q) = line.strip_prefix(ASK) {
            self.question = Some(serde_json::from_str(q).unwrap_or_else(|_| q.to_owned()));
        } else if let Some(r) = line.strip_prefix(RESULT) {
            let result: Value = serde_json::from_str(r).unwrap_or_default();
            self.detected = result["formats"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect();
            self.step = result["step"].as_u64().map_or(1, |s| s.max(1) as u32);
        } else {
            self.progress.feed(&line);
            self.log.push(line);
        }
    }
}

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
    /// Lines of `log` the session log has taken.
    logged: usize,
    /// The line number of the job's heading in the session log, and the heading.
    head: Option<(usize, String)>,
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
            logged: 0,
            head: None,
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

/// Most lines the session log keeps; it drops the oldest past this.
pub const LOG_LINES: usize = 20_000;

/// gw's output from every job of the session, oldest first, each job's lines
/// under its heading.
#[derive(Default)]
pub struct SessionLog {
    lines: Vec<String>,
    /// Line numbers of the headings, counted from the session's first line.
    heads: Vec<usize>,
    /// Lines gone from the start: dropped to keep within LOG_LINES, or cleared.
    dropped: usize,
    /// Lines were dropped to keep within LOG_LINES since the log was last cleared.
    trimmed: bool,
    /// The job the last lines are from, by its heading's line number.
    last: Option<usize>,
}

impl SessionLog {
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn trimmed(&self) -> bool {
        self.trimmed
    }

    /// Empties the log. A job still running goes on under its heading again.
    pub fn clear(&mut self) {
        self.dropped += self.lines.len();
        self.lines.clear();
        self.heads.clear();
        self.last = None;
        self.trimmed = false;
    }

    /// Whether `lines()[index]` heads a job.
    pub fn is_head(&self, index: usize) -> bool {
        self.heads.binary_search(&(self.dropped + index)).is_ok()
    }

    /// Starts a job's lines under `heading`.
    pub fn begin(&mut self, heading: String, job: &mut Job) {
        let at = self.head(heading.clone());
        self.last = Some(at);
        job.head = Some((at, heading));
    }

    /// Takes the job's output that is new since the last call.
    pub fn follow(&mut self, job: &mut Job) {
        if job.logged < job.log.len() {
            self.resume(job);
            self.lines.extend_from_slice(&job.log[job.logged..]);
            job.logged = job.log.len();
            self.trim();
        }
    }

    /// Takes the rest of the job's output, then `ending`, which says how it ended.
    pub fn end(&mut self, job: &mut Job, ending: String) {
        self.follow(job);
        self.resume(job);
        self.lines.push(ending);
        self.trim();
    }

    /// Heads the job's lines again when another job's came in between.
    fn resume(&mut self, job: &Job) {
        if let Some((at, heading)) = &job.head
            && self.last != Some(*at)
        {
            self.head(format!("{heading} (continued)"));
            self.last = Some(*at);
        }
    }

    /// Adds a heading, a blank line after the lines before, and gives its line number.
    fn head(&mut self, heading: String) -> usize {
        if !self.lines.is_empty() {
            self.lines.push(String::new());
        }
        let at = self.dropped + self.lines.len();
        self.heads.push(at);
        self.lines.push(heading);
        self.trim();
        at
    }

    fn trim(&mut self) {
        let over = self.lines.len().saturating_sub(LOG_LINES);
        if over > 0 {
            self.lines.drain(..over);
            self.dropped += over;
            self.trimmed = true;
            self.heads.retain(|&h| h >= self.dropped);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(command: &str) -> Job {
        Job::new(command, Vec::new(), mpsc::channel().1)
    }

    #[test]
    fn the_session_log_keeps_each_job_under_its_heading_in_order() {
        let mut log = SessionLog::default();
        let mut info = job("info");
        log.begin("gw info".into(), &mut info);
        info.log.push("Host Tools: 1.23".into());
        log.follow(&mut info);
        info.log.push("Device:".into());
        log.end(&mut info, "Done in 0:01.".into());
        let mut read = job("read");
        log.begin("gw read x.img".into(), &mut read);
        read.log.push("Reading c=0-79:h=0-1 revs=2".into());
        log.follow(&mut read);
        log.follow(&mut read);
        log.end(&mut read, "Stopped after 0:04.".into());
        assert_eq!(
            log.lines(),
            [
                "gw info",
                "Host Tools: 1.23",
                "Device:",
                "Done in 0:01.",
                "",
                "gw read x.img",
                "Reading c=0-79:h=0-1 revs=2",
                "Stopped after 0:04."
            ]
        );
        let heads: Vec<usize> = (0..log.lines().len()).filter(|&i| log.is_head(i)).collect();
        assert_eq!(heads, [0, 5]);
    }

    #[test]
    fn the_session_log_drops_its_oldest_lines_past_its_limit() {
        let mut log = SessionLog::default();
        let mut first = job("read");
        log.begin("gw read a.img".into(), &mut first);
        first.log = (0..LOG_LINES).map(|i| format!("T{i}")).collect();
        log.follow(&mut first);
        first.log.push("Found 2880 sectors of 2880 (100%)".into());
        log.end(&mut first, "Done in 9:00.".into());
        let mut second = job("read");
        log.begin("gw read b.img".into(), &mut second);
        assert_eq!(log.lines().len(), LOG_LINES);
        assert!(log.trimmed());
        assert_eq!(
            log.lines()[0],
            "T4",
            "the first heading and four lines went"
        );
        let heads: Vec<usize> = (0..log.lines().len()).filter(|&i| log.is_head(i)).collect();
        assert_eq!(
            heads,
            [LOG_LINES - 1],
            "the first heading went with its lines, and did not come back"
        );
    }

    #[test]
    fn clearing_the_log_empties_it_and_a_running_job_goes_on_under_its_heading() {
        let mut log = SessionLog::default();
        let mut first = job("read");
        log.begin("gw read a.img".into(), &mut first);
        first.log = (0..LOG_LINES).map(|i| format!("T{i}")).collect();
        log.follow(&mut first);
        assert!(log.trimmed());
        log.clear();
        assert!(log.lines().is_empty() && !log.trimmed());
        first.log.push("T last".into());
        log.end(&mut first, "Done in 9:00.".into());
        assert_eq!(
            log.lines(),
            ["gw read a.img (continued)", "T last", "Done in 9:00."]
        );
        assert!(log.is_head(0) && !log.is_head(1));
    }

    #[test]
    fn a_job_running_beside_another_keeps_its_lines_under_its_own_heading() {
        let mut log = SessionLog::default();
        let (mut info, mut detect) = (job("info"), job(DETECT));
        log.begin("gw info".into(), &mut info);
        info.log.push("Host Tools: 1.23".into());
        log.follow(&mut info);
        log.begin("Detect disk format in.scp".into(), &mut detect);
        detect.log.push("Format ibm.1440".into());
        log.follow(&mut detect);
        info.log.push("Device:".into());
        log.follow(&mut info);
        log.follow(&mut detect);
        log.end(&mut info, "Done in 0:01.".into());
        log.end(&mut detect, "Done in 0:02.".into());
        assert_eq!(
            log.lines(),
            [
                "gw info",
                "Host Tools: 1.23",
                "",
                "Detect disk format in.scp",
                "Format ibm.1440",
                "",
                "gw info (continued)",
                "Device:",
                "Done in 0:01.",
                "",
                "Detect disk format in.scp (continued)",
                "Done in 0:02.",
            ]
        );
        let heads: Vec<usize> = (0..log.lines().len()).filter(|&i| log.is_head(i)).collect();
        assert_eq!(heads, [0, 3, 6, 10]);
    }
}

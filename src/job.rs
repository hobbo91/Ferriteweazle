//! One gw command, run through the bridge, with its output as it arrives.

use crate::device;
use crate::progress::Progress;
use crate::service::Repaint;
use crate::tools::{Tools, quiet};
use serde_json::Value;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
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
    /// The job failed and its image is gone: gw deletes it.
    pub no_image: bool,
    /// The job's disk format, if known.
    pub format: Option<String>,
    /// Formats that read the disk in full, best first, from a detect job.
    pub detected: Vec<String>,
    /// The head step the detected disk needs: 2 for a 40-track disk in an
    /// 80-track drive.
    pub step: u32,
    /// Which disk of a session this is, counting from 1, and how many.
    pub part: Option<(usize, usize)>,
    /// The line gw is printing, until it ends it: gw clean prints each
    /// cylinder as the heads reach it.
    pub partial: String,
    /// Lines of `log` the session log has taken.
    logged: usize,
    /// The line number of the job's heading in the session log, and the heading.
    head: Option<(usize, String)>,
    stopping: Option<Instant>,
    /// None for a job replayed from its output.
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    lines: Receiver<Chunk>,
    eof: bool,
    /// A standalone gw, run as it is: it asks on its own output and stops when killed.
    standalone: bool,
}

impl Job {
    /// Runs `gw ARGS`, or with [`DETECT`] finds the format of the disk they name.
    pub fn start(
        tools: &Tools,
        device: &str,
        command: &str,
        args: Vec<String>,
        repaint: Repaint,
    ) -> std::io::Result<Job> {
        let mode = if command == DETECT { "detect" } else { "run" };
        let mut cmd = match tools.standalone {
            true if command == DETECT => {
                return Err(std::io::Error::other(
                    "Standalone Greaseweazle Tools cannot run Detect.",
                ));
            }
            true => quiet(std::process::Command::new(&tools.python)),
            false => tools.bridge(mode),
        };
        let mut child = cmd
            // The device the bridge's own messages name.
            .env("FERRITEWEAZLE_DEVICE", device)
            .env("PYTHONIOENCODING", "utf-8")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let stderr = child.stderr.take().expect("stderr is piped");
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || relay(stderr, tx, repaint));
        Ok(Job {
            child: Some(child),
            stdin,
            standalone: tools.standalone,
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
        let outcome = match job.worked(true) {
            true => Outcome::Succeeded,
            false => Outcome::Failed,
        };
        job.end(job.started, outcome);
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
                Ok(Chunk::Line(line)) => {
                    self.partial.clear();
                    self.take(line);
                }
                Ok(Chunk::Partial(text)) => {
                    // gw's one question, `gw seek`'s, ends "Yes/No? " with no line end.
                    if self.standalone && self.question.is_none() && text.ends_with("Yes/No? ") {
                        self.question = Some(text.trim().to_owned());
                    }
                    self.partial = text;
                }
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
            } else if self.worked(status.success()) {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            };
            self.end(Instant::now(), outcome);
            self.question = None;
        }
    }

    /// Ends the job. A write or conversion that worked passed over the
    /// tracks it did not report.
    fn end(&mut self, at: Instant, outcome: Outcome) {
        self.ended = Some((at, outcome));
        self.progress.finish();
        if outcome == Outcome::Succeeded && matches!(self.command.as_str(), "write" | "convert") {
            self.progress.skip_unreported();
        }
    }

    /// Asks gw to stop. It turns the motor off, and `read` keeps what it has.
    pub fn stop(&mut self) {
        if self.running() && self.stopping.is_none() {
            self.stdin = None; // the bridge stops gw when its input closes
            if self.standalone
                && let Some(child) = &mut self.child
            {
                let _ = child.kill();
            }
            self.stopping = Some(Instant::now());
            self.question = None;
        }
    }

    /// Whether a job that ran to its end worked. gw info exits 0 when it
    /// finds no Greaseweazle, and 1 when its device has answered in full and
    /// only the check online for newer firmware fails.
    fn worked(&self, exited_ok: bool) -> bool {
        let clean = exited_ok && self.progress.error.is_none();
        match self.command.as_str() {
            // USB ends gw info's report on the device.
            "info" => device::parse(&self.log).is_some_and(|d| clean || d.get("USB").is_some()),
            _ => clean,
        }
    }

    pub fn answer(&mut self, text: &str) {
        if let Some(stdin) = &mut self.stdin {
            let _ = match self.standalone {
                true => writeln!(stdin, "{text}"),
                false => writeln!(stdin, "answer {text}"),
            };
        }
        self.question = None;
    }

    fn new(command: &str, args: Vec<String>, lines: Receiver<Chunk>) -> Job {
        let mut progress = Progress::default();
        progress.raw = command == "read" && args.iter().any(|a| a == "--raw");
        Job {
            command: command.to_owned(),
            args,
            log: Vec::new(),
            progress,
            question: None,
            started: Instant::now(),
            ended: None,
            output: None,
            no_image: false,
            format: None,
            detected: Vec::new(),
            step: 1,
            part: None,
            partial: String::new(),
            logged: 0,
            head: None,
            stopping: None,
            child: None,
            stdin: None,
            lines,
            eof: false,
            standalone: false,
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
            // With no --format, gw names the image type's own or the one it finds in the file.
            if self.format.is_none() {
                self.format = line
                    .strip_prefix("Format ")
                    .or_else(|| Some(line.split_once(": Image format ")?.1))
                    .map(str::to_owned);
            }
            self.progress.feed(&line);
            self.log.push(line);
        }
    }
}

/// What the reader passes on from gw's output.
enum Chunk {
    Line(String),
    /// The line so far, where gw has not ended it yet.
    Partial(String),
}

/// Passes on gw's output from `from` as it comes: each line, and a line not
/// yet ended each time it grows.
fn relay(from: impl Read, to: Sender<Chunk>, repaint: Repaint) {
    let mut reader = BufReader::new(from);
    let mut line = Vec::new();
    loop {
        let (used, ended) = match reader.fill_buf() {
            Ok([]) if line.is_empty() => break,
            // The end of the output ends the line too.
            Ok([]) => (0, true),
            Ok(buf) => match buf.iter().position(|&b| b == b'\n') {
                Some(end) => {
                    line.extend_from_slice(&buf[..end]);
                    (end + 1, true)
                }
                None => {
                    line.extend_from_slice(buf);
                    (buf.len(), false)
                }
            },
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        reader.consume(used);
        let text = String::from_utf8_lossy(&line)
            .trim_end_matches('\r')
            .to_owned();
        let chunk = match ended {
            true => {
                line.clear();
                Chunk::Line(text)
            }
            false => Chunk::Partial(text),
        };
        if to.send(chunk).is_err() {
            return;
        }
        repaint();
    }
    drop(to); // before the repaint, so the poll it wakes sees the end
    repaint();
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

    /// The line `job` has not ended yet, while its lines are the last here.
    pub fn tail<'j>(&self, job: &'j Job) -> &'j str {
        match &job.head {
            Some((at, _)) if self.last == Some(*at) => &job.partial,
            _ => "",
        }
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

    /// Ends the job's own lines with `ending`, which says how it ended, and
    /// takes the rest: its page's output box shows what the Log shows.
    pub fn end(&mut self, job: &mut Job, ending: String) {
        job.log.push(ending);
        self.follow(job);
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

    /// A standalone gw that runs `body` in sh.
    #[cfg(unix)]
    fn standalone(test: &str, body: &str) -> (Tools, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ferriteweazle-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("gw");
        std::fs::write(&script, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (Tools::find(Some(&script)).unwrap(), dir)
    }

    #[test]
    #[cfg(unix)]
    fn a_standalone_gw_asks_on_its_output_and_takes_the_answer_on_its_input() {
        let body = "printf 'Seek to extreme cylinder 90, Yes/No? ' >&2\nread answer\n\
                    echo \"Seeking: $answer\" >&2\n";
        let (gw, dir) = standalone("asks", body);
        let args = vec!["seek".into(), "90".into()];
        let mut job = Job::start(&gw, "Greaseweazle", "seek", args, Box::new(|| {})).unwrap();
        poll_until(&mut job, |j| j.question.is_some());
        let question = job.question.as_deref();
        assert_eq!(question, Some("Seek to extreme cylinder 90, Yes/No?"));
        job.answer("Yes");
        poll_until(&mut job, |j| !j.running());
        assert!(
            job.log.iter().any(|l| l.ends_with("Seeking: Yes")),
            "{:?}",
            job.log
        );
        assert_eq!(job.outcome(), Some(Outcome::Succeeded));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    #[cfg(unix)]
    fn stopping_a_standalone_gw_ends_it_at_once() {
        let (gw, dir) = standalone("stops", "sleep 30\n");
        let args = vec!["read".into(), "disk.scp".into()];
        let mut job = Job::start(&gw, "Greaseweazle", "read", args, Box::new(|| {})).unwrap();
        job.stop();
        poll_until(&mut job, |j| !j.running());
        assert_eq!(job.outcome(), Some(Outcome::Stopped));
        assert!(job.elapsed() < GRACE, "it waited out the grace period");
        std::fs::remove_dir_all(dir).ok();
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

    /// Polls `job` until `done`, for up to five seconds.
    fn poll_until(job: &mut Job, done: impl Fn(&Job) -> bool) {
        let start = Instant::now();
        while !done(job) {
            assert!(start.elapsed() < Duration::from_secs(5), "{:?}", job.log);
            std::thread::sleep(Duration::from_millis(5));
            job.poll();
        }
    }

    #[test]
    fn a_line_gw_has_not_ended_shows_as_it_grows() {
        let (from, mut gw) = std::io::pipe().unwrap();
        let (to, lines) = mpsc::channel();
        std::thread::spawn(move || relay(from, to, Box::new(|| ())));
        let mut clean = Job::new("clean", Vec::new(), lines);
        gw.write_all(b"Pass 0: 0 10 ").unwrap();
        poll_until(&mut clean, |j| j.partial == "Pass 0: 0 10 ");
        assert!(clean.log.is_empty());
        gw.write_all(b"20\r\nPass 1: 0").unwrap();
        poll_until(&mut clean, |j| j.partial == "Pass 1: 0");
        assert_eq!(clean.log, ["Pass 0: 0 10 20"]);
        drop(gw);
        poll_until(&mut clean, |j| j.eof);
        assert_eq!(
            clean.log,
            ["Pass 0: 0 10 20", "Pass 1: 0"],
            "the end ends it"
        );
        assert_eq!(clean.partial, "");
        let mut log = SessionLog::default();
        log.begin("gw clean".into(), &mut clean);
        clean.partial = "Pass 2: 0".into();
        assert_eq!(log.tail(&clean), "Pass 2: 0");
        log.begin("gw info".into(), &mut job("info"));
        assert_eq!(log.tail(&clean), "", "another job's lines came after");
    }

    #[test]
    fn the_format_gw_takes_from_the_image_is_the_jobs() {
        let format = |log| Job::replay("convert", log).format;
        let adf = "Format amiga.amigados\nConverting c=0-79:h=0-1 -> c=0-79:h=0-1";
        assert_eq!(format(adf).as_deref(), Some("amiga.amigados"));
        let nsi = "NSI: Image format northstar.fm.ss\nConverting c=0-34:h=0 -> c=0-34:h=0";
        assert_eq!(format(nsi).as_deref(), Some("northstar.fm.ss"));
    }

    #[test]
    fn gw_info_works_when_its_device_answers_in_full_and_fails_when_it_finds_none() {
        let outcome = |log: &str| Job::replay("info", log).outcome();
        // gw info exits 0 here.
        let none = "Host Tools: 1.23\nDevice:\n  Not found";
        assert_eq!(outcome(none), Some(Outcome::Failed));
        // And 1 here, offline.
        let report = "Host Tools: 1.23\nDevice:\n  Port:     /dev/cu.usbmodem1\n  \
                      Model:    Greaseweazle V4.1\n  Firmware: 1.6\n  Serial:   GW01\n  \
                      USB:      Full Speed (12 Mbit/s), 128kB Buffer";
        let offline = format!("{report}\n** FATAL ERROR:\nGitHub API Rate Limit exceeded");
        assert_eq!(outcome(&offline), Some(Outcome::Succeeded));
        assert_eq!(outcome(report), Some(Outcome::Succeeded));
        let cut = "Host Tools: 1.23\nDevice:\n  Port:     /dev/cu.usbmodem1\n\
                   ** FATAL ERROR:\nGreaseweazle interface did not answer.";
        assert_eq!(outcome(cut), Some(Outcome::Failed));
    }
}

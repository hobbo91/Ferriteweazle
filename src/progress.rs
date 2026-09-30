//! Per-track results, read from gw's output as it arrives.
//!
//! Unrecognised lines are left to the log, so a gw that rewords its output
//! loses the map, never the job.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Every sector found, or written and verified.
    Good,
    /// Some sectors missing.
    Partial,
    /// No sectors found, or the write failed.
    Bad,
    /// Read or converted as flux, with nothing decoded.
    Flux,
    /// Written, with no verify reported.
    Written,
    Erased,
    /// Outside the chosen format, or not in the input.
    Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub status: Status,
    pub retries: u32,
    /// What gw last said about the track.
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Progress {
    /// The cylinders gw said it would visit; `heads` the heads.
    pub cyls: Vec<u32>,
    pub heads: Vec<u32>,
    pub tracks: BTreeMap<(u32, u32), Track>,
    /// Sector by sector, from the map gw prints when it has finished.
    pub sector_map: BTreeMap<(u32, u32), Vec<Option<bool>>>,
    /// Sectors found and expected over the whole disk.
    pub total: Option<(u32, u32)>,
    /// The track being worked on.
    pub current: Option<(u32, u32)>,
    pub error: Option<String>,
    /// gw's line on why a write left tracks unverified, such as "No tracks
    /// verified (Reason: Verify unavailable)".
    pub unverified: Option<String>,
    /// gw's warnings about the job as a whole, such as a damaged input's.
    pub warnings: Vec<String>,
    /// A read with --raw: gw keeps the flux of tracks outside the format.
    pub raw: bool,
    /// A write gw verifies: it goes on from a track only once that verified.
    pub verifies: bool,
    /// The read pass under way after the first, and the most there may be.
    pub pass: Option<(u32, u32)>,
    /// The cylinders of gw's sector map, those it read. A conversion's
    /// track lines name the tracks it writes.
    columns: Vec<u32>,
    /// What the lines that follow belong to.
    block: Block,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Block {
    /// gw's output as it goes.
    #[default]
    Output,
    /// The error's message.
    Error,
    /// A Python traceback, whose last unindented lines are the error.
    Traceback,
    /// A list of what gw knows, such as its formats, after an error.
    List,
}

/// Headings of the lists gw puts after some errors. Must match gw's messages.
const LISTS: [&str; 4] = [
    "Known formats:",
    "Known suffixes:",
    "Valid modes:",
    "Valid types:",
];

/// How a traceback names gw's own errors, which it prints bare without --bt.
const FATAL: &str = "greaseweazle.error.Fatal: ";

impl Progress {
    /// Nothing done yet on these cylinders and sides.
    pub fn blank(cyls: Vec<u32>, heads: Vec<u32>) -> Progress {
        Progress {
            cyls,
            heads,
            ..Progress::default()
        }
    }

    pub fn feed(&mut self, line: &str) {
        let line = line.trim_end();
        match self.block {
            Block::Output => {}
            Block::Error => {
                // gw indents some lines, such as its bootloader warning's second.
                let line = line.trim_start();
                if let Some(t) = line
                    .strip_prefix("Failed to verify Track ")
                    .and_then(key)
                    .and_then(|k| self.tracks.get_mut(&k))
                {
                    t.status = Status::Bad;
                } else if let Some((key, text)) = track_line(line) {
                    // gw stopped before the track, as for sectors its input lacks.
                    self.moved_to(key);
                    let t = self.tracks.entry(key).or_insert(Track {
                        status: Status::Bad,
                        retries: 0,
                        text: String::new(),
                    });
                    t.status = Status::Bad;
                    text.clone_into(&mut t.text);
                }
                return self.add_to_error(line);
            }
            Block::Traceback if line.starts_with(' ') => {
                // A frame or its code: the exception comes after the last.
                self.error = None;
                return;
            }
            Block::Traceback => return self.add_to_error(line.strip_prefix(FATAL).unwrap_or(line)),
            Block::List => return,
        }
        if line == "** FATAL ERROR:" {
            self.error.get_or_insert_default();
            self.block = Block::Error;
        } else if let Some(e) = ["ERROR: ", "** UPDATE FAILED: "]
            .iter()
            .find_map(|p| line.strip_prefix(p))
        {
            // gw's advice, if any, follows on the next lines.
            self.error = Some(e.to_owned());
            self.block = Block::Error;
        } else if line == "Traceback (most recent call last):" {
            self.block = Block::Traceback;
        } else if let Some(e) = line
            .strip_prefix("Command Failed: ")
            .or_else(|| Some(line.strip_prefix("gw ")?.split_once(": error: ")?.1))
            // bridge.py's detect parser.
            .or_else(|| line.strip_prefix("detect: error: "))
        {
            self.error = Some(e.to_owned());
        } else if let Some(rest) = ["Reading ", "Writing ", "Converting ", "Erasing "]
            .iter()
            .find_map(|p| line.strip_prefix(p))
            && let Some(tracks) = announced(rest)
        {
            (self.columns, self.cyls, self.heads) = tracks;
        } else if line == "All tracks verified" {
            for t in self
                .tracks
                .values_mut()
                .filter(|t| t.status == Status::Written)
            {
                t.status = Status::Good;
            }
        } else if line.starts_with("No tracks verified ")
            || line.contains(" tracks *not* verified ")
        {
            self.unverified = Some(line.to_owned());
        } else if let Some(rest) = line.strip_prefix("Found ") {
            self.total = found(rest);
        } else if let Some(pass) = line.strip_prefix("Pass ").and_then(pass) {
            self.pass = Some(pass);
        } else if let Some((key, text)) = track_line(line) {
            self.track(key, text);
        } else if let Some((head, sector, cells)) = map_row(line) {
            self.map_row(head, sector, cells);
        } else if line.contains("WARNING:") {
            self.warnings.push(line.to_owned());
        }
    }

    /// Adds a line to the error's message, which ends where a list begins.
    fn add_to_error(&mut self, line: &str) {
        if LISTS.contains(&line) {
            self.block = Block::List;
        } else if !line.is_empty() {
            let error = self.error.get_or_insert_default();
            if !error.is_empty() {
                error.push('\n');
            }
            error.push_str(line);
        }
    }

    /// The job has ended.
    pub fn finish(&mut self) {
        self.current = None;
    }

    /// Marks the announced tracks gw has not reported as skipped: a write or
    /// conversion passes over those its input lacks without a word.
    pub fn skip_unreported(&mut self) {
        for &cyl in &self.cyls {
            for &head in &self.heads {
                self.tracks.entry((cyl, head)).or_insert_with(|| Track {
                    status: Status::Skipped,
                    retries: 0,
                    text: "Not in the input, so Greaseweazle Tools passed over it.".into(),
                });
            }
        }
    }

    /// Cylinders and heads to draw: those gw announced, and any it visited.
    pub fn layout(&self) -> (Vec<u32>, Vec<u32>) {
        let mut cyls: BTreeSet<u32> = self.cyls.iter().copied().collect();
        let mut heads: BTreeSet<u32> = self.heads.iter().copied().collect();
        for &(c, h) in self.tracks.keys() {
            cyls.insert(c);
            heads.insert(h);
        }
        (cyls.into_iter().collect(), heads.into_iter().collect())
    }

    /// Counts over the tracks done, except the current one.
    pub fn tally(&self) -> Tally {
        let mut tally = Tally::default();
        for (&key, t) in &self.tracks {
            if Some(key) == self.current {
                continue;
            }
            tally.done += 1;
            tally.retries += t.retries;
            match t.status {
                Status::Partial => tally.partial += 1,
                Status::Bad => tally.bad += 1,
                _ => {}
            }
        }
        tally
    }

    fn track(&mut self, key: (u32, u32), text: &str) {
        let count = sectors(text);
        if count.is_none() && !RESULTS.iter().any(|r| text.starts_with(r)) {
            return;
        }
        self.moved_to(key);
        self.current = Some(key);
        // Newer than a sector map printed after an earlier pass.
        self.sector_map.remove(&key);
        let t = self.tracks.entry(key).or_insert(Track {
            status: Status::Flux,
            retries: 0,
            text: String::new(),
        });
        if text.contains("(Retry #") || text.contains("(Verify Failure") {
            t.retries += 1;
        }
        // "Giving up" keeps the result of the last attempt, and adds to it.
        if text.starts_with("Giving up") {
            if !t.text.is_empty() {
                t.text.push('\n');
            }
            t.text.push_str(text);
            return;
        }
        text.clone_into(&mut t.text);
        t.status = if text.starts_with("WARNING") {
            // Outside the format: a read with --raw keeps its flux all the same.
            match self.raw && text.contains("No format conversion applied") {
                true => Status::Flux,
                false => Status::Skipped,
            }
        } else if text.starts_with("Erasing") {
            Status::Erased
        } else if text.starts_with("Writing") {
            Status::Written
        } else if text.starts_with("IBM Empty") {
            Status::Bad
        } else if let Some((good, all)) = count {
            match good {
                0 if all > 0 => Status::Bad,
                g if g < all => Status::Partial,
                _ => Status::Good,
            }
        } else {
            Status::Flux
        };
    }

    /// gw has gone on to track `key`, so a write that verifies has checked
    /// the track before.
    fn moved_to(&mut self, key: (u32, u32)) {
        if !self.verifies || self.current == Some(key) {
            return;
        }
        if let Some(t) = self.current.and_then(|c| self.tracks.get_mut(&c))
            && t.status == Status::Written
        {
            t.status = Status::Good;
        }
    }

    fn map_row(&mut self, head: u32, sector: usize, cells: &str) {
        for (&cyl, cell) in self.columns.iter().zip(cells.chars()) {
            let good = match cell {
                '.' => true,
                'X' => false,
                _ => continue,
            };
            let row = self.sector_map.entry((cyl, head)).or_default();
            if row.len() <= sector {
                row.resize(sector + 1, None);
            }
            row[sector] = Some(good);
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub done: u32,
    pub partial: u32,
    pub bad: u32,
    pub retries: u32,
}

/// How a track's result begins when it gives no sector count. gw's other
/// `T12.1:` lines are remarks, such as a codec's about a stray sector.
const RESULTS: [&str; 8] = [
    "WARNING",
    "Erasing",
    "Writing",
    "Giving up",
    "Raw Flux",
    "Raw Bitcell",
    "Bitcells",
    "IBM Empty",
];

/// `12.1` as (cylinder, head).
fn key(s: &str) -> Option<(u32, u32)> {
    let (c, h) = s.trim().split_once('.')?;
    Some((c.parse().ok()?, h.parse().ok()?))
}

/// `T12.1: text` or `T12.1 <- Drive 12.0: text`.
fn track_line(line: &str) -> Option<((u32, u32), &str)> {
    let (id, text) = line.strip_prefix('T')?.split_once(": ")?;
    Some((key(id.split_whitespace().next()?)?, text))
}

/// `(17/18 sectors)` anywhere in the text.
fn sectors(text: &str) -> Option<(u32, u32)> {
    let end = text.find(" sectors)")?;
    let start = text[..end].rfind('(')? + 1;
    let (good, all) = text[start..end].split_once('/')?;
    Some((good.parse().ok()?, all.parse().ok()?))
}

/// `2 of 3: 4 tracks`, from the bridge's passes.
fn pass(text: &str) -> Option<(u32, u32)> {
    let (n, rest) = text.split_once(" of ")?;
    let (of, _) = rest.split_once(':')?;
    Some((n.parse().ok()?, of.parse().ok()?))
}

/// `2878 sectors of 2880 (99%)`.
fn found(text: &str) -> Option<(u32, u32)> {
    let (good, rest) = text.split_once(" sectors of ")?;
    let all = rest.split_whitespace().next()?;
    Some((good.parse().ok()?, all.parse().ok()?))
}

/// A row of gw's sector map: `1.17: ..X..`, one cell per cylinder.
fn map_row(line: &str) -> Option<(u32, usize, &str)> {
    let (id, cells) = line.split_once(": ")?;
    let (head, sector) = id.split_once('.')?;
    Some((head.parse().ok()?, sector.trim().parse().ok()?, cells))
}

/// The tracks gw announces, as `c=0-79:h=0-1 revs=2` or a conversion's
/// `c=0-79:h=0-1 -> c=0-39:h=0-1`: the cylinders it reads, then the
/// cylinders and heads its track lines name, which a conversion writes.
fn announced(rest: &str) -> Option<(Vec<u32>, Vec<u32>, Vec<u32>)> {
    let mut words = rest.split_whitespace();
    let (read, heads) = track_set(words.next()?.trim_end_matches(','))?;
    let (cyls, heads) = match (words.next(), words.next()) {
        (Some("->"), Some(out)) => track_set(out)?,
        _ => (read.clone(), heads),
    };
    Some((read, cyls, heads))
}

/// Cylinders and heads from a printed track set such as `c=0-79:h=0-1`.
fn track_set(s: &str) -> Option<(Vec<u32>, Vec<u32>)> {
    let (mut cyls, mut heads) = (None, None);
    for part in s.split(':') {
        match part.split_once('=') {
            Some(("c", v)) => cyls = numbers(v),
            Some(("h", v)) => heads = numbers(v),
            _ => {}
        }
    }
    Some((cyls?, heads?))
}

/// `0-7,9,12-15`, or `0-79/2` for every other one, as gw takes them.
pub(crate) fn numbers(s: &str) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    for range in s.split(',') {
        let (range, every) = range.split_once('/').unwrap_or((range, "1"));
        let (a, b) = range.split_once('-').unwrap_or((range, range));
        let (a, b): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
        let every: usize = every.parse().ok().filter(|&n| n > 0)?;
        if b.saturating_sub(a) > 1024 {
            return None;
        }
        out.extend((a..=b).step_by(every));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(log: &str) -> Progress {
        let mut p = Progress::default();
        log.lines().for_each(|l| p.feed(l));
        p
    }

    #[test]
    fn a_later_pass_says_which_it_is_and_its_combined_tracks_count() {
        let p = fed("Reading c=0-1:h=0-1 revs=2\n\
             Pass 2 of 3: 1 track\n\
             T0.0: IBM MFM (10/18 sectors) from Raw Flux (41657 flux in 200.00ms)\n\
             T0.0: IBM MFM (18/18 sectors) from 2 passes");
        assert_eq!(p.pass, Some((2, 3)));
        assert_eq!(p.tracks[&(0, 0)].status, Status::Good);
    }

    #[test]
    fn a_track_read_again_after_the_sector_map_is_shown_by_its_new_line() {
        let p = fed("Reading c=0-1:h=0 revs=2\n\
             T0.0: IBM MFM (17/18 sectors) from Raw Flux (41657 flux in 200.00ms)\n\
             T1.0: IBM MFM (18/18 sectors) from Raw Flux (41657 flux in 200.00ms)\n\
             Cyl-> 0\n\
             H. S: 01\n\
             0. 0: X.\n\
             Pass 2 of 2: 1 track\n\
             T0.0: IBM MFM (18/18 sectors) from 2 passes");
        assert_eq!(p.sector_map.get(&(0, 0)), None);
        assert_eq!(p.sector_map[&(1, 0)], [Some(true)]);
    }

    #[test]
    fn a_conversion_to_flux_marks_every_track_good() {
        let p = fed(include_str!("../tests/data/convert-img-to-scp.log"));
        assert_eq!((p.cyls.len(), p.heads), (80, vec![0, 1]));
        assert_eq!(p.tracks.len(), 160);
        assert!(p.tracks.values().all(|t| t.status == Status::Good));
        assert_eq!(p.total, Some((2880, 2880)));
        assert_eq!(p.error, None);
    }

    #[test]
    fn a_damaged_disk_shows_which_sectors_are_missing() {
        let mut p = fed(include_str!("../tests/data/convert-damaged.log"));
        p.finish();
        let status = |c, h| p.tracks[&(c, h)].status;
        assert_eq!(status(20, 0), Status::Partial);
        assert_eq!(status(55, 1), Status::Partial);
        assert_eq!(status(75, 0), Status::Bad);
        assert_eq!(status(0, 0), Status::Good);
        assert_eq!(p.sector_map[&(20, 0)][5], Some(false));
        assert_eq!(p.sector_map[&(21, 1)][9], Some(false));
        assert!(p.sector_map[&(75, 0)].iter().all(|s| *s == Some(false)));
        assert_eq!(p.total, Some((2856, 2880)));
        assert_eq!(
            p.tally(),
            Tally {
                done: 160,
                partial: 4,
                bad: 1,
                retries: 0
            }
        );
        assert_eq!(p.warnings, ["SCP: WARNING: Bad image checksum"]);
    }

    #[test]
    fn the_final_sector_map_fills_in_every_sector() {
        let p = fed(include_str!("../tests/data/convert-scp-to-img.log"));
        assert_eq!(p.sector_map.len(), 160);
        assert!(
            p.sector_map
                .values()
                .all(|row| row.len() == 18 && row.iter().all(|s| *s == Some(true)))
        );
        assert_eq!(
            p.tally(),
            Tally {
                done: 159,
                ..Tally::default()
            },
            "the last track is still current"
        );
    }

    #[test]
    fn a_stopped_job_keeps_what_it_did() {
        let mut p = fed(include_str!("../tests/data/convert-stopped.log"));
        p.finish();
        assert_eq!(p.tracks.len(), 18);
        assert_eq!(p.tally().done, 18);
        assert_eq!(p.total, None);
    }

    #[test]
    fn retries_and_giving_up_leave_a_partial_track() {
        let p = fed("Reading c=0-1:h=0 revs=2\n\
            T0.0: IBM MFM (18/18 sectors) from Raw Flux (1 flux in 200.00ms)\n\
            T1.0: IBM MFM (0/18 sectors) from Raw Flux (1 flux in 200.00ms)\n\
            T1.0: IBM MFM (17/18 sectors) from Raw Flux (1 flux in 200.00ms) (Retry #1.1)\n\
            T1.0: IBM MFM (17/18 sectors) from Raw Flux (1 flux in 200.00ms) (Retry #1.2)\n\
            T1.0: Giving up: 1 sectors missing\n\
            Cyl-> 0\n\
            H. S: 01\n\
            0. 0: ..\n\
            0. 1: .X\n\
            Found 35 sectors of 36 (97%)");
        let t = &p.tracks[&(1, 0)];
        assert_eq!((t.status, t.retries), (Status::Partial, 2));
        assert_eq!(
            t.text,
            "IBM MFM (17/18 sectors) from Raw Flux (1 flux in 200.00ms) (Retry #1.2)\n\
             Giving up: 1 sectors missing",
            "the last attempt's result, then gw's"
        );
        assert_eq!(p.sector_map[&(1, 0)], vec![Some(true), Some(false)]);
        assert_eq!(p.total, Some((35, 36)));
    }

    #[test]
    fn a_track_with_nothing_found_is_bad() {
        let p =
            fed("T5.1 <- Drive 10.1: AmigaDOS (0/11 sectors) from Raw Flux (9 flux in 200.00ms)");
        assert_eq!(p.tracks[&(5, 1)].status, Status::Bad);
    }

    #[test]
    fn a_raw_read_records_flux_only() {
        let p = fed("Reading c=0-39:h=0-1 revs=3\nT0.0: Raw Flux (149963 flux in 600.17ms)");
        assert_eq!(p.tracks[&(0, 0)].status, Status::Flux);
        assert_eq!(p.layout().0.len(), 40);
    }

    #[test]
    fn writes_turn_good_once_verified() {
        let mut p = fed("Writing c=0-1:h=0-1\n\
            T0.0: Erasing Track\n\
            T0.0: Writing Track (Flux: 200.0ms period, 200.2 ms total, Write all)\n\
            T0.1: Writing Track (Flux: 200.0ms period, 200.2 ms total, Write all)\n\
            T0.1: Writing Track (Verify Failure: Retry #1)\n\
            T1.0: Erasing Track");
        assert_eq!(p.tracks[&(0, 0)].status, Status::Written);
        assert_eq!(p.tracks[&(0, 1)].retries, 1);
        assert_eq!(p.tracks[&(1, 0)].status, Status::Erased);
        p.feed("All tracks verified");
        assert_eq!(p.tracks[&(0, 1)].status, Status::Good);
    }

    #[test]
    fn a_failed_verify_is_bad_and_explained() {
        let p = fed("T3.0: Writing Track (Flux: 1)\n** FATAL ERROR:\nFailed to verify Track 3.0");
        assert_eq!(p.tracks[&(3, 0)].status, Status::Bad);
        assert_eq!(p.error.as_deref(), Some("Failed to verify Track 3.0"));
    }

    #[test]
    fn a_track_gw_stops_at_before_writing_it_is_bad() {
        let p = fed("Writing c=0-1:h=0\n\
            T0.0: Writing Track (Flux: 200.0ms period, 200.2 ms total, Write all)\n\
            ** FATAL ERROR:\n\
            T1.0: 3 missing sectors in input image");
        let t = &p.tracks[&(1, 0)];
        assert_eq!(t.status, Status::Bad);
        assert_eq!(t.text, "3 missing sectors in input image");
        assert_eq!(p.tracks[&(0, 0)].status, Status::Written);
        assert_eq!(
            p.error.as_deref(),
            Some("T1.0: 3 missing sectors in input image")
        );
    }

    #[test]
    fn gws_reason_for_leaving_tracks_unverified_is_kept() {
        let written = "Writing c=0:h=0\n\
            T0.0: Writing Track (Flux: 200.0ms period, 200.2 ms total, Write all)";
        for reason in [
            "No tracks verified (Reason: Verify disabled)",
            "2 tracks verified; 1 tracks *not* verified (Reason: Verify unavailable)",
        ] {
            let p = fed(&format!("{written}\n{reason}"));
            assert_eq!(p.unverified.as_deref(), Some(reason));
            assert_eq!(p.tracks[&(0, 0)].status, Status::Written);
        }
    }

    #[test]
    fn errors_are_collected() {
        let p = fed(
            "usage: gw read [options] file\ngw read: error: argument --revs: must be 1 or greater",
        );
        assert_eq!(
            p.error.as_deref(),
            Some("argument --revs: must be 1 or greater")
        );
        assert_eq!(
            fed("Command Failed: GetFluxStatus: No Index")
                .error
                .as_deref(),
            Some("GetFluxStatus: No Index")
        );
    }

    #[test]
    fn update_failed_and_error_lines_are_errors_with_the_lines_after_them() {
        assert_eq!(
            fed("** UPDATE FAILED: Please retry!").error.as_deref(),
            Some("Please retry!")
        );
        let bootloader = fed(
            "** UPDATE FAILED: Please retry immediately or your Weazle may need\n        \
             full reflashing via a suitable programming adapter!",
        );
        assert_eq!(
            bootloader.error.as_deref(),
            Some(
                "Please retry immediately or your Weazle may need\n\
                 full reflashing via a suitable programming adapter!"
            )
        );
        let unsupported = fed("ERROR: Device firmware version 0.29 is unsupported\n\
            To perform an Update:\n \
            - Run \"gw update\" to download and install latest firmware");
        assert_eq!(
            unsupported.error.as_deref(),
            Some(
                "Device firmware version 0.29 is unsupported\nTo perform an Update:\n\
                 - Run \"gw update\" to download and install latest firmware"
            )
        );
    }

    #[test]
    fn a_python_traceback_gives_its_last_exception_as_the_error() {
        let p = fed(r#"Traceback (most recent call last):
  File "greaseweazle/image/scp.py", line 150, in from_bytes
    checksum) = struct.unpack("<3s9BI", dat[0:16])
                ~~~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^
struct.error: unpack requires a buffer of 16 bytes"#);
        assert_eq!(
            p.error.as_deref(),
            Some("struct.error: unpack requires a buffer of 16 bytes")
        );
        let chained = fed(r#"Traceback (most recent call last):
  File "serial/serialposix.py", line 322, in open
PermissionError: [Errno 13] Permission denied: '/dev/ttyACM0'

During handling of the above exception, another exception occurred:

Traceback (most recent call last):
  File "serial/serialposix.py", line 325, in open
serial.serialutil.SerialException: [Errno 13] could not open port /dev/ttyACM0"#);
        assert_eq!(
            chained.error.as_deref(),
            Some("serial.serialutil.SerialException: [Errno 13] could not open port /dev/ttyACM0")
        );
    }

    #[test]
    fn a_traceback_gives_all_of_gws_message_as_gw_prints_it_without_one() {
        let p = fed(r#"Traceback (most recent call last):
  File "greaseweazle/error.py", line 15, in check
    raise Fatal(desc)
greaseweazle.error.Fatal: out.hfe: Invalid file option: bogus
Valid options: bitrate, version, interface, encoding, double_step, uniform"#);
        assert_eq!(
            p.error.as_deref(),
            Some(
                "out.hfe: Invalid file option: bogus\n\
                 Valid options: bitrate, version, interface, encoding, double_step, uniform"
            )
        );
    }

    #[test]
    fn the_list_gw_gives_after_an_error_stays_in_the_log() {
        let unknown = "Unknown format 'nosuch.fmt'";
        let list = "Known formats:\n\
            acorn.adfs.160            acorn.adfs.1600           acorn.adfs.320\n\
            acorn.adfs.640            acorn.adfs.800            acorn.dfs.ds";
        let fatal = fed(&format!("** FATAL ERROR:\n{unknown}\n{list}"));
        assert_eq!(fatal.error.as_deref(), Some(unknown));
        let traceback = fed(&format!(
            "Traceback (most recent call last):\n  \
             File \"greaseweazle/tools/convert.py\", line 170, in main\n    \
             raise error.Fatal(\"\"\"\\\n    \
             ...<3 lines>...\n\
             greaseweazle.error.Fatal: {unknown}\n{list}"
        ));
        assert_eq!(traceback.error.as_deref(), Some(unknown));
        let suffix = fed("** FATAL ERROR:\n\
            a.xyz: Unrecognised file suffix '.xyz'\n\
            Known suffixes:\n\
            .a2r   .adf   .ads   .adm   .adl   .ctr   .d1m   .d2m   .d4m   .d64   .d71");
        assert_eq!(
            suffix.error.as_deref(),
            Some("a.xyz: Unrecognised file suffix '.xyz'")
        );
    }

    #[test]
    fn an_erase_announces_its_tracks() {
        let p = fed("Erasing c=0-81:h=0-1, revs=1");
        assert_eq!((p.cyls.len(), p.heads), (82, vec![0, 1]));
    }

    #[test]
    fn remarks_about_a_track_leave_it_as_it_was() {
        let p = fed(
            "T45.0: D88: Removed 2 duplicate sectors from oversized track\n\
            T5.0: IBM: WARNING: Track is 7.50% too long\n\
            Writing c=0-1:h=0\n\
            T0.0: Writing Track (Flux: 1)\n\
            T0.0: Ignoring unexpected sector C:0 H:0 R:19 N:2\n\
            All tracks verified",
        );
        assert_eq!(p.tracks.keys().collect::<Vec<_>>(), [&(0, 0)]);
        assert_eq!(p.tracks[&(0, 0)].status, Status::Good);
        assert!(p.warnings.is_empty(), "the job's own warnings only");
    }

    #[test]
    fn a_scan_that_finds_no_sectors_is_bad() {
        let p = fed("T40.0: IBM Empty from Raw Flux (1234 flux in 400.00ms)");
        assert_eq!(p.tracks[&(40, 0)].status, Status::Bad);
    }

    #[test]
    fn tracks_out_of_the_format_are_skipped() {
        let p = fed("T80.0: WARNING: Out of range for format 'ibm.1440': Track skipped");
        assert_eq!(p.tracks[&(80, 0)].status, Status::Skipped);
    }

    #[test]
    fn a_raw_read_keeps_the_flux_of_a_track_outside_the_format() {
        let line = "T80.0: WARNING: Out of range for format 'ibm.1440': \
                    No format conversion applied: Raw Flux (149963 flux in 600.17ms)";
        let mut raw = Progress {
            raw: true,
            ..Progress::default()
        };
        raw.feed(line);
        assert_eq!(raw.tracks[&(80, 0)].status, Status::Flux);
        let read = fed(line);
        assert_eq!(
            read.tracks[&(80, 0)].status,
            Status::Skipped,
            "no flux kept"
        );
    }

    #[test]
    fn a_conversion_maps_the_tracks_it_writes() {
        // gw 1.23 converting with --out-tracks=c=5-9.
        let tracks =
            (5..=9).flat_map(|c| (0..2).map(move |h| format!("T{c}.{h}: IBM MFM (18/18 sectors)")));
        let log = ["Format ibm.1440", "Converting c=0-79:h=0-1 -> c=5-9:h=0-1"]
            .map(String::from)
            .into_iter()
            .chain(tracks)
            .chain(["0. 0:      .....", "0. 1:      ..X.."].map(String::from))
            .collect::<Vec<_>>()
            .join("\n");
        let p = fed(&log);
        assert_eq!((p.cyls, p.heads), ((5..=9).collect(), vec![0, 1]));
        assert_eq!(p.sector_map[&(7, 0)], [Some(true), Some(false)]);
        assert_eq!(p.sector_map.len(), 5, "side 1's rows are not given here");
    }

    #[test]
    fn a_write_or_conversion_that_worked_skipped_the_tracks_it_did_not_report() {
        let mut p = fed("Writing c=0-1:h=0\nT0.0: Writing Track (Flux: 1)\nAll tracks verified");
        p.skip_unreported();
        assert_eq!(p.tracks[&(1, 0)].status, Status::Skipped);
        assert_eq!(p.tracks[&(0, 0)].status, Status::Good);
    }

    #[test]
    fn unrecognised_lines_change_nothing() {
        let p = fed(
            "HFE: Data bitrate detected: 250 kbit/s\nTime elapsed: 2.00 seconds\nDrive reports 16 hard sectors",
        );
        assert!(p.tracks.is_empty() && p.sector_map.is_empty() && p.error.is_none());
    }

    #[test]
    fn track_sets_read_as_gw_prints_them() {
        assert_eq!(
            track_set("c=0-2,5:h=1:step=2"),
            Some((vec![0, 1, 2, 5], vec![1]))
        );
        assert_eq!(track_set("c=<none>:h=0"), None);
        assert_eq!(
            track_set("c=4294967295:h=0"),
            Some((vec![u32::MAX], vec![0]))
        );
    }

    #[test]
    fn a_failed_or_stopped_write_keeps_the_tracks_gw_verified_before_it() {
        use Status::{Bad, Good, Written};
        let log = "Writing c=0-2:h=0\n\
            T0.0: Writing Track (Flux: 200.0ms period, 200.2 ms total, Write all)\n\
            T1.0: Erasing Track\n\
            T1.0: Writing Track (Flux: 200.0ms period, 200.2 ms total, Write all)\n\
            T1.0: Writing Track (Verify Failure: Retry #1)";
        let status = |log: &str, verifies: bool| {
            let mut p = Progress {
                verifies,
                ..Progress::default()
            };
            log.lines().for_each(|l| p.feed(l));
            [0, 1, 2].map(|c| p.tracks.get(&(c, 0)).map(|t| t.status))
        };
        let stopped = status(log, true);
        assert_eq!(stopped, [Some(Good), Some(Written), None]);
        let failed = format!("{log}\n** FATAL ERROR:\nFailed to verify Track 1.0");
        assert_eq!(status(&failed, true), [Some(Good), Some(Bad), None]);
        let lacking = format!("{log}\n** FATAL ERROR:\nT2.0: 3 missing sectors in input image");
        assert_eq!(status(&lacking, true), [Some(Good), Some(Good), Some(Bad)]);
        let unchecked = status(log, false);
        assert_eq!(unchecked, [Some(Written), Some(Written), None]);
    }

    #[test]
    fn a_page_setting_detection_cannot_take_is_its_error() {
        let p = fed("usage: detect [-h] [--device DEVICE] [--drive DRIVE]\n\
            detect: error: argument --fake-index: invalid period value: 'abc'");
        assert_eq!(
            p.error.as_deref(),
            Some("argument --fake-index: invalid period value: 'abc'")
        );
    }
}

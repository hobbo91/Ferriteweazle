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
    /// No sectors found, or the write failed to verify.
    Bad,
    /// Read or converted as flux, with nothing decoded.
    Flux,
    /// Written and not verified.
    Written,
    Erased,
    /// Outside the chosen format.
    Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub status: Status,
    /// Sectors found and expected.
    pub sectors: Option<(u32, u32)>,
    pub retries: u32,
    /// What gw last said about the track.
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Progress {
    /// The tracks gw said it would visit.
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
    /// The lines that follow belong to the error.
    fatal: bool,
    /// Inside a Python traceback, whose last unindented line is the error.
    traceback: bool,
}

impl Progress {
    /// Nothing done yet, on a disk of this size.
    pub fn blank(cyls: u32, heads: u32) -> Progress {
        Progress {
            cyls: (0..cyls).collect(),
            heads: (0..heads).collect(),
            ..Progress::default()
        }
    }

    pub fn feed(&mut self, line: &str) {
        let line = line.trim_end();
        if self.fatal {
            // gw indents some lines, such as its bootloader warning's second.
            let line = line.trim_start();
            let error = self.error.get_or_insert_default();
            if !line.is_empty() {
                error.push_str(if error.is_empty() { "" } else { "\n" });
                error.push_str(line);
            }
            if let Some(t) = line
                .strip_prefix("Failed to verify Track ")
                .and_then(key)
                .and_then(|k| self.tracks.get_mut(&k))
            {
                t.status = Status::Bad;
            }
            return;
        }
        if self.traceback {
            if !line.is_empty() && !line.starts_with(' ') {
                self.error = Some(line.to_owned());
            }
            return;
        }
        if line == "** FATAL ERROR:" {
            self.fatal = true;
        } else if let Some(e) = ["ERROR: ", "** UPDATE FAILED: "]
            .iter()
            .find_map(|p| line.strip_prefix(p))
        {
            // gw's advice, if any, follows on the next lines.
            self.error = Some(e.to_owned());
            self.fatal = true;
        } else if line == "Traceback (most recent call last):" {
            self.traceback = true;
        } else if let Some(e) = line
            .strip_prefix("Command Failed: ")
            .or_else(|| Some(line.strip_prefix("gw ")?.split_once(": error: ")?.1))
        {
            self.error = Some(e.to_owned());
        } else if let Some(set) = ["Reading ", "Writing ", "Converting "]
            .iter()
            .find_map(|p| line.strip_prefix(p))
            .and_then(|rest| track_set(rest.split_whitespace().next()?))
        {
            (self.cyls, self.heads) = set;
        } else if line == "All tracks verified" {
            for t in self
                .tracks
                .values_mut()
                .filter(|t| t.status == Status::Written)
            {
                t.status = Status::Good;
            }
        } else if let Some(rest) = line.strip_prefix("Found ") {
            self.total = found(rest);
        } else if let Some((key, text)) = track_line(line) {
            self.track(key, text);
        } else if let Some((head, sector, cells)) = map_row(line) {
            self.map_row(head, sector, cells);
        }
    }

    /// The job has ended.
    pub fn finish(&mut self) {
        self.current = None;
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
        self.current = Some(key);
        let t = self.tracks.entry(key).or_insert(Track {
            status: Status::Flux,
            sectors: None,
            retries: 0,
            text: String::new(),
        });
        if text.contains("(Retry #") || text.contains("(Verify Failure") {
            t.retries += 1;
        }
        text.clone_into(&mut t.text);
        // "Giving up" keeps the result of the last attempt.
        if text.starts_with("Giving up") {
            return;
        }
        t.status = if text.starts_with("WARNING") {
            Status::Skipped
        } else if text.starts_with("Erasing") {
            Status::Erased
        } else if text.starts_with("Writing") {
            Status::Written
        } else if let Some((good, all)) = sectors(text) {
            t.sectors = Some((good, all));
            match good {
                0 if all > 0 => Status::Bad,
                g if g < all => Status::Partial,
                _ => Status::Good,
            }
        } else {
            Status::Flux
        };
    }

    fn map_row(&mut self, head: u32, sector: usize, cells: &str) {
        for (&cyl, cell) in self.cyls.iter().zip(cells.chars()) {
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

/// `0-7,9,12-15`.
fn numbers(s: &str) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    for range in s.split(',') {
        let (a, b) = range.split_once('-').unwrap_or((range, range));
        let (a, b): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
        if b.saturating_sub(a) > 1024 {
            return None;
        }
        out.extend(a..=b);
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
    fn a_conversion_to_flux_marks_every_track_good() {
        let p = fed(include_str!("../tests/data/convert-img-to-scp.log"));
        assert_eq!((p.cyls.len(), p.heads), (80, vec![0, 1]));
        assert_eq!(p.tracks.len(), 160);
        assert!(
            p.tracks
                .values()
                .all(|t| t.status == Status::Good && t.sectors == Some((18, 18)))
        );
        assert_eq!(p.total, Some((2880, 2880)));
        assert_eq!(p.error, None);
    }

    #[test]
    fn a_damaged_disk_shows_which_sectors_are_missing() {
        let mut p = fed(include_str!("../tests/data/convert-damaged.log"));
        p.finish();
        let status = |c, h| p.tracks[&(c, h)].status;
        assert_eq!(status(20, 0), Status::Partial);
        assert_eq!(p.tracks[&(55, 1)].sectors, Some((15, 18)));
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
            T1.0: IBM MFM (16/18 sectors) from Raw Flux (1 flux in 200.00ms)\n\
            T1.0: IBM MFM (17/18 sectors) from Raw Flux (1 flux in 200.00ms) (Retry #1.1)\n\
            T1.0: IBM MFM (17/18 sectors) from Raw Flux (1 flux in 200.00ms) (Retry #1.2)\n\
            T1.0: Giving up: 1 sectors missing\n\
            Cyl-> 0\n\
            H. S: 01\n\
            0. 0: ..\n\
            0. 1: .X\n\
            Found 35 sectors of 36 (97%)");
        let t = &p.tracks[&(1, 0)];
        assert_eq!(
            (t.status, t.sectors, t.retries),
            (Status::Partial, Some((17, 18)), 2)
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
    fn tracks_out_of_the_format_are_skipped() {
        let p = fed("T80.0: WARNING: Out of range for format 'ibm.1440': Track skipped");
        assert_eq!(p.tracks[&(80, 0)].status, Status::Skipped);
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
}

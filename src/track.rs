//! What gw holds of a track, as the bridge reports it, and what that shows:
//! where each sector lies round the track and how it decoded, and how the
//! flux falls round it. The disk view draws these: only what was found,
//! where it was found.

use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

/// The bridge's report on a track, as bridge.py's report() prints it.
#[derive(Debug, Deserialize)]
struct Report {
    c: u32,
    h: u32,
    /// For a write: `image`, `written` or `verify`; see Source.
    source: Option<String>,
    flux: Option<Flux>,
    codec: Option<Codec>,
}

/// A track's flux, in its sample rate's ticks.
#[derive(Debug, Deserialize)]
struct Flux {
    /// Ticks per second.
    freq: f64,
    /// Ticks between index pulses, the first from the read's start unless it starts at one.
    index: Vec<f64>,
    cued: bool,
    /// Ticks per revolution: the mean of the full ones read, else gw's measure of the drive.
    period: f64,
    /// Ticks the read lasted.
    end: f64,
    /// Flux transitions in equal parts of a revolution over the whole read,
    /// each revolution from its own index to the next; see spin().
    bins: Vec<u32>,
}

/// What gw decoded of a track.
#[derive(Debug, Clone, Deserialize)]
struct Codec {
    /// Such as `IBM MFM (18/18 sectors)`.
    summary: String,
    nsec: u32,
    /// The sectors that decoded, by number.
    #[serde(default)]
    good: Vec<u32>,
    time_per_rev: Option<f64>,
    /// The bit cell, in seconds.
    clock: Option<f64>,
    /// An IBM-style track's index marks, in bit cells from the index, and
    /// where decoded from flux, in time: see Found::times.
    #[serde(default)]
    iams: Vec<f64>,
    #[serde(default)]
    iam_times: Vec<Option<(f64, f64, Option<f64>)>>,
    /// An IBM-style track's sectors as found round it.
    #[serde(default)]
    found: Vec<Found>,
    /// The sectors its format lays out, where flux was decoded into it.
    laid: Option<Vec<Found>>,
    /// Headers found with no data after them and data with no header.
    #[serde(default)]
    apart: Vec<Apart>,
    /// Where other codecs' sectors were found, by number.
    #[serde(default)]
    places: BTreeMap<u32, Place>,
    /// Other codecs' sectors' data, by number.
    #[serde(default)]
    data: BTreeMap<u32, Bytes>,
}

/// An IBM-style sector: its places in bit cells from the index, and where
/// decoded from flux, in time.
#[derive(Debug, Clone, Deserialize)]
struct Found {
    /// C, H, R and N.
    id: [u8; 4],
    /// Where its ID field starts and ends, its data field starts, and it ends.
    start: f64,
    header_end: f64,
    data_start: f64,
    end: f64,
    /// Where decoded from flux, those in seconds from the index its
    /// revolution starts at, by the clock of gw's PLL as it followed the
    /// flux, and that revolution's length, if it ends.
    times: Option<[f64; 4]>,
    turn: Option<f64>,
    /// Its header's CRC holds, and its data's.
    header: bool,
    data: bool,
    mark: u8,
    bytes: Option<Bytes>,
}

/// A header or data block found by itself, which gw's decoder drops.
#[derive(Debug, Clone, Deserialize)]
struct Apart {
    /// A header's C, H, R and N, and whether its CRC holds; none for data.
    id: Option<[u8; 4]>,
    header: Option<bool>,
    start: f64,
    end: f64,
    /// A data block's mark.
    mark: Option<u8>,
    /// Its start and end in time: see Found::times.
    times: Option<[f64; 2]>,
    turn: Option<f64>,
}

/// Where a sector of a codec that is not IBM's lies on its PLL track.
#[derive(Debug, Clone, Deserialize)]
struct Place {
    /// When it starts, its data starts, and it ends, in seconds from the PLL
    /// track's first bit cell, by the PLL's clock as it followed the flux.
    at: f64,
    data: Option<f64>,
    end: f64,
    /// The length of each of the PLL track's revolutions, from its first bit
    /// cell, in seconds.
    revs: Vec<f64>,
    /// The PLL track starts at an index, not between two.
    cued: bool,
}

/// Bytes the bridge sends as hex.
#[derive(Debug, Clone, Default, PartialEq)]
struct Bytes(Vec<u8>);

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Bytes, D::Error> {
        let hex = String::deserialize(d)?;
        let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        hex.as_bytes()
            .chunks(2)
            .map(|pair| match pair {
                [a, b] => Some(digit(*a)? << 4 | digit(*b)?),
                _ => None,
            })
            .collect::<Option<Vec<u8>>>()
            .map(Bytes)
            .ok_or_else(|| serde::de::Error::custom("not hex"))
    }
}

/// A track as the disk view draws it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    /// gw's summary of what it decoded, such as `IBM MFM (18/18 sectors)`.
    pub summary: Option<String>,
    /// The sectors found, in turn round the track from the index.
    pub sectors: Vec<Sector>,
    /// The sectors the format lays out that gw did not find.
    pub missing: Vec<Id>,
    /// Index address marks, each a share of a revolution from the index.
    pub iams: Vec<f32>,
    /// The bit cell gw decoded at, in seconds.
    pub cell: Option<f64>,
    pub flux: Option<Spin>,
    /// For a write, what these are of; none for a read or a conversion,
    /// whose input they are.
    pub source: Option<Source>,
    /// The job's revision when these came: see Progress::revision.
    pub revision: u64,
}

/// What a write's report on a track is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The image's flux, which gw writes as it is.
    Image,
    /// The master track gw writes from a format's sectors, from the index:
    /// each sector exactly where gw puts it.
    Written,
    /// The track gw read back from the disk to verify it.
    Verify,
}

/// How the disk turned under the head as gw read the track, and how its flux fell.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spin {
    /// Seconds a revolution takes: the mean of the full ones read, else gw's
    /// measure of the drive.
    pub period: f64,
    /// Each full revolution's time, in seconds.
    pub revs: Vec<f64>,
    /// Flux transitions per revolution.
    pub per_rev: f64,
    /// Flux transitions in equal parts of a revolution from the index, per
    /// time the read passed each part.
    pub bins: Vec<f32>,
}

impl Spin {
    /// Each part's flux transitions against the track's mean part: 1 where
    /// the flux runs as it does round the rest of the track, 0 where there
    /// is none.
    pub fn relative(&self) -> Vec<f32> {
        let mean = self.per_rev / self.bins.len() as f64;
        if mean <= 0.0 {
            return vec![0.0; self.bins.len()];
        }
        self.bins
            .iter()
            .map(|&n| (f64::from(n) / mean) as f32)
            .collect()
    }
}

/// A sector's ID: C, H, R and N from an IBM-style header, else its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Id {
    Ibm([u8; 4]),
    Number(u32),
}

/// A sector as found on the track: a header block, then a data block.
#[derive(Debug, Clone, PartialEq)]
pub struct Sector {
    pub id: Id,
    /// Where it starts, where its data starts and where it ends, each a share
    /// of a revolution from the index; past 1 where it runs over the index.
    /// None where gw notes no place.
    pub at: Option<[f32; 3]>,
    /// Where an IBM-style sector's ID field ends, likewise: a gap lies
    /// between it and the data.
    pub header_end: Option<f32>,
    pub header: Header,
    pub data: Data,
    /// Its data mark, such as FB, or F8 for deleted data.
    pub mark: Option<u8>,
    pub bytes: Vec<u8>,
    /// Its format lays out no such sector, so gw leaves it out of the image.
    pub extra: bool,
    /// Read before the read's first index pulse, so placed back from it by
    /// the length of a revolution: the next one's, or gw's measure of the
    /// drive's.
    pub before: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    Good,
    /// Its CRC fails.
    Bad,
    /// None was found before the data.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Data {
    Good,
    /// Good, every byte this one.
    Empty(u8),
    /// Its CRC fails.
    Bad,
    /// Found with no header, so gw reads none of it.
    Unread,
    /// None was found after the header.
    None,
}

/// gw's tolerance for one sector seen twice, in bit cells: its decoders
/// count areas this close as one.
const SAME: f64 = 1000.0;

impl Facts {
    /// Parses a TRACK line's JSON: the track's cylinder and head, and its facts.
    pub fn parse(json: &str) -> Option<((u32, u32), Facts)> {
        let report: Report = serde_json::from_str(json).ok()?;
        let mut facts = report
            .codec
            .as_ref()
            .map(Facts::decoded)
            .unwrap_or_default();
        facts.flux = report.flux.as_ref().and_then(spin);
        facts.source = match report.source.as_deref() {
            Some("image") => Some(Source::Image),
            Some("written") => Some(Source::Written),
            Some("verify") => Some(Source::Verify),
            _ => None,
        };
        Some(((report.c, report.h), facts))
    }

    fn decoded(codec: &Codec) -> Facts {
        let mut facts = Facts {
            summary: Some(codec.summary.clone()),
            cell: codec.clock,
            ..Facts::default()
        };
        match codec.time_per_rev.zip(codec.clock) {
            Some(_) if !codec.found.is_empty() || codec.laid.is_some() => facts.ibm(codec),
            _ => facts.numbered(codec),
        }
        facts
    }

    /// An IBM-style track's sectors, placed as gw found them.
    fn ibm(&mut self, codec: &Codec) {
        let laid = codec.laid.as_deref();
        let expected = |id: &[u8; 4]| laid.is_none_or(|l| l.iter().any(|s| s.id == *id));
        for s in &codec.found {
            let bytes = s.bytes.clone().unwrap_or_default().0;
            let data = match s.data {
                false => Data::Bad,
                true => match bytes.first() {
                    Some(&b) if bytes.iter().all(|&x| x == b) => Data::Empty(b),
                    _ => Data::Good,
                },
            };
            let [start, header_end, data_start, end] = codec.shares(
                [s.start, s.header_end, s.data_start, s.end],
                s.times,
                s.turn,
            );
            let at = span(start, data_start, end);
            self.sectors.push(Sector {
                id: Id::Ibm(s.id),
                at: Some(at),
                header_end: Some(header_end - (start - at[0])),
                header: if s.header { Header::Good } else { Header::Bad },
                data,
                mark: Some(s.mark),
                bytes,
                extra: s.header && !expected(&s.id),
                before: false,
            });
        }
        // Blocks found apart, once each, unless part of a sector found whole.
        let mut apart: Vec<&Apart> = Vec::new();
        for a in &codec.apart {
            let whole = codec.found.iter().any(|s| match a.id {
                Some(_) => (s.start - a.start).abs() < SAME,
                None => (s.data_start - a.start).abs() < SAME,
            });
            let seen = apart
                .iter()
                .any(|b| b.id.is_some() == a.id.is_some() && (b.start - a.start).abs() < SAME);
            if !whole && !seen {
                apart.push(a);
            }
        }
        for a in apart {
            let [start, end] = codec.shares([a.start, a.end], a.times, a.turn);
            self.sectors.push(match a.id {
                Some(id) => Sector {
                    id: Id::Ibm(id),
                    at: Some(span(start, end, end)),
                    header_end: None,
                    header: if a.header == Some(true) {
                        Header::Good
                    } else {
                        Header::Bad
                    },
                    data: Data::None,
                    mark: None,
                    bytes: Vec::new(),
                    extra: false,
                    before: false,
                },
                None => Sector {
                    id: Id::Number(0),
                    at: Some(span(start, start, end)),
                    header_end: None,
                    header: Header::None,
                    data: Data::Unread,
                    mark: a.mark,
                    bytes: Vec::new(),
                    extra: false,
                    before: false,
                },
            });
        }
        self.sectors
            .sort_by(|a, b| start_of(a).total_cmp(&start_of(b)));
        self.missing = laid
            .into_iter()
            .flatten()
            .filter(|s| !s.header)
            .map(|s| Id::Ibm(s.id))
            .collect();
        self.iams = codec
            .iams
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                let at = codec.iam_times.get(i).copied().flatten();
                let [share] = codec.shares([x], at.map(|(t, ..)| [t]), at.and_then(|a| a.2));
                share.rem_euclid(1.0)
            })
            .collect();
    }

    /// Another codec's sectors, by number, placed where gw found them.
    fn numbered(&mut self, codec: &Codec) {
        for &n in &codec.good {
            let bytes = codec.data.get(&n).cloned().unwrap_or_default().0;
            let data = match bytes.first() {
                Some(&b) if bytes.iter().all(|&x| x == b) => Data::Empty(b),
                _ => Data::Good,
            };
            let placed = codec
                .places
                .get(&n)
                .and_then(|p| place(p, codec.time_per_rev));
            self.sectors.push(Sector {
                id: Id::Number(n),
                at: placed.map(|(at, _)| at),
                header_end: None,
                header: Header::Good,
                data,
                mark: None,
                bytes,
                extra: false,
                before: placed.is_some_and(|(_, before)| before),
            });
        }
        self.sectors
            .sort_by(|a, b| start_of(a).total_cmp(&start_of(b)));
        self.missing = (0..codec.nsec)
            .filter(|n| !codec.good.contains(n))
            .map(Id::Number)
            .collect();
    }
}

impl Codec {
    /// Where IBM-style areas lie round the track, as shares of a revolution
    /// from the index: by time where gw decoded them from flux, `times` from
    /// the index their revolution starts at and
    /// `turn` its length, else gw's revolution; with none, by `bits` of the
    /// format's bit cells, as an image's track is laid out.
    fn shares<const N: usize>(
        &self,
        bits: [f64; N],
        times: Option<[f64; N]>,
        turn: Option<f64>,
    ) -> [f32; N] {
        let per_rev = self.time_per_rev.unwrap_or(1.0);
        match times {
            Some(times) => times.map(|t| (t / turn.unwrap_or(per_rev)) as f32),
            None => {
                let cells = per_rev / self.clock.unwrap_or(1.0);
                bits.map(|b| (b / cells) as f32)
            }
        }
    }
}

/// A sector's start, data and end, its start taken into the first revolution.
fn span(start: f32, data: f32, end: f32) -> [f32; 3] {
    let turn = start.div_euclid(1.0);
    [start - turn, data - turn, end - turn]
}

fn start_of(s: &Sector) -> f32 {
    s.at.map_or(f32::INFINITY, |[start, ..]| start)
}

/// Where a sector lies round the track, as shares of a revolution from the
/// index, and whether it was read before the first index pulse: from its
/// place on a PLL track, whose revolutions run from index to index after
/// any read before the first. gw's PLL scales the flux to the codec's
/// revolution, `per_rev` seconds, so a read of under two index pulses
/// measures its revolutions by that.
fn place(p: &Place, per_rev: Option<f64>) -> Option<([f32; 3], bool)> {
    let mut index = Vec::new();
    let mut at = 0.0;
    for (i, &bits) in p.revs.iter().enumerate() {
        // Bits read before the first index make no revolution.
        if p.cued || i > 0 {
            index.push(at);
        }
        at += bits;
    }
    index.push(at);
    // The last index at or before the sector, or the first, and the
    // revolution after it, or before it.
    let after = index.iter().rposition(|&x| x <= p.at);
    let i = after.unwrap_or(0);
    let bits = match (index.get(i + 1), i.checked_sub(1)) {
        (Some(next), _) => next - index[i],
        (None, Some(last)) => index[i] - index[last],
        (None, None) => per_rev?,
    };
    let share = |x: f64| ((x - index[i]) / bits) as f32;
    let at = span(share(p.at), share(p.data.unwrap_or(p.at)), share(p.end));
    Some((at, after.is_none()))
}

/// How the disk turned and its flux fell, from the bridge's count of it.
fn spin(f: &Flux) -> Option<Spin> {
    let parts = f.bins.len();
    if parts == 0 || f.period <= 0.0 || f.freq <= 0.0 {
        return None;
    }
    // How often the read passed each part, as the bridge counted: each
    // revolution from its index pulse to the next; before the first pulse
    // and after the last, by the length of the revolution beside it.
    let mut pulses: Vec<f64> = f
        .index
        .iter()
        .scan(0.0, |at, &x| {
            *at += x;
            Some(*at)
        })
        .collect();
    if f.cued {
        pulses.insert(0, 0.0);
    }
    let (&first_pulse, &last_pulse) = pulses.first().zip(pulses.last())?;
    let turns: Vec<f64> = pulses.windows(2).map(|w| w[1] - w[0]).collect();
    let first = turns.first().copied().unwrap_or(f.period);
    let last = turns.last().copied().unwrap_or(f.period);
    let mut cover = vec![0.0; parts];
    let mut pass = |from: f64, to: f64| {
        // Shares of a revolution, which may run on past 1.
        let n = parts as f64;
        let mut part = (from * n).floor();
        while part < to * n {
            let covered = (to * n).min(part + 1.0) - (from * n).max(part);
            cover[part.rem_euclid(n) as usize] += covered.max(0.0);
            part += 1.0;
        }
    };
    if !f.cued {
        pass(1.0 - first_pulse / first, 1.0);
    }
    turns.iter().for_each(|_| pass(0.0, 1.0));
    pass(0.0, (f.end - last_pulse).max(0.0) / last);
    let bins: Vec<f32> = f
        .bins
        .iter()
        .zip(&cover)
        .map(|(&n, &c)| {
            if c > 0.0 {
                (f64::from(n) / c) as f32
            } else {
                0.0
            }
        })
        .collect();
    Some(Spin {
        period: f.period / f.freq,
        revs: turns.iter().map(|r| r / f.freq).collect(),
        per_rev: bins.iter().map(|&b| f64::from(b)).sum(),
        bins,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A track of the Workbench 3.1 Install disk, read from a real drive.
    const AMIGA: &str = include_str!("../tests/data/report-amiga.txt");
    /// A track of a real Akai S950 disk's HFE image.
    const AKAI: &str = include_str!("../tests/data/report-akai.txt");

    fn first(lines: &str) -> ((u32, u32), Facts) {
        let line = lines.lines().next().unwrap();
        Facts::parse(line.strip_prefix("@ferriteweazle track ").unwrap()).unwrap()
    }

    #[test]
    fn a_real_amiga_tracks_sectors_lie_end_to_end_where_gw_found_them() {
        let ((c, h), facts) = first(AMIGA);
        assert_eq!((c, h), (0, 0));
        assert_eq!(facts.summary.as_deref(), Some("AmigaDOS (11/11 sectors)"));
        assert_eq!(facts.sectors.len(), 11);
        assert!(facts.missing.is_empty());
        // gw reads 544 MFM words from each sync word: the sectors abut, but
        // where the read began. Those before the first index pulse are placed
        // back from it by the next revolution's length, and the turn of a real
        // drive varies.
        let seams: Vec<f32> = facts
            .sectors
            .windows(2)
            .map(|pair| pair[0].at.unwrap()[2] - pair[1].at.unwrap()[0])
            .filter(|gap| gap.abs() > 1e-6)
            .collect();
        assert_eq!(seams.len(), 1, "{seams:?}");
        assert!(seams[0].abs() < 2e-3, "{seams:?}");
        let before = facts.sectors.iter().filter(|s| s.before).count();
        assert!(before > 0 && before < 11, "{before} before the first index");
        let spin = facts.flux.unwrap();
        let rpm = 60.0 / spin.period;
        assert!((299.0..301.0).contains(&rpm), "{rpm} rpm");
        assert_eq!(spin.revs.len(), 3, "the three revolutions read whole");
    }

    #[test]
    fn a_real_akai_track_starts_with_the_sector_its_formatter_wrote_first() {
        let (_, facts) = first(AKAI);
        let Id::Ibm([_, _, r, n]) = facts.sectors[0].id else {
            panic!("an IBM sector")
        };
        assert_eq!(
            (r, n),
            (7, 3),
            "sector 7, 1024 bytes, first after the index"
        );
        assert!(facts.sectors.iter().all(|s| s.header == Header::Good));
        let [start, data, end] = facts.sectors[0].at.unwrap();
        assert!(start < data && data < end);
        assert!(facts.iams.is_empty(), "this disk has no index mark");
        // A whole track of data: its flux even round it.
        let relative = facts.flux.unwrap().relative();
        let mean = relative.iter().sum::<f32>() / relative.len() as f32;
        assert!((mean - 1.0).abs() < 1e-3, "{mean}");
        assert!(
            relative.iter().all(|&d| (0.5..=1.5).contains(&d)),
            "{relative:?}"
        );
    }

    #[test]
    fn a_read_that_starts_between_index_pulses_counts_each_part_as_often_as_it_passed() {
        // Half a revolution, then the index, then one whole revolution.
        let flux = Flux {
            freq: 1000.0,
            index: vec![50.0, 100.0],
            cued: false,
            period: 100.0,
            end: 150.0,
            bins: vec![2, 2, 1, 1],
        };
        let spin = spin(&flux).unwrap();
        assert_eq!(spin.bins, [2.0, 2.0, 0.5, 0.5]);
        assert_eq!(spin.revs, [0.1]);
        assert_eq!(spin.period, 0.1);
    }

    #[test]
    fn a_place_before_the_first_index_lies_late_in_the_revolution() {
        let p = Place {
            at: 75.0,
            data: None,
            end: 125.0,
            revs: vec![100.0, 200.0],
            cued: false,
        };
        assert_eq!(place(&p, None), Some(([0.875, 0.875, 1.125], true)));
        let cued = Place {
            cued: true,
            ..p.clone()
        };
        assert_eq!(place(&cued, None), Some(([0.75, 0.75, 1.25], false)));
        // One index read: the revolution is the codec's length, as gw's PLL takes it.
        let once = Place {
            revs: vec![100.0],
            ..p
        };
        assert_eq!(place(&once, None), None);
        assert_eq!(
            place(&once, Some(200.0)),
            Some(([0.875, 0.875, 1.125], true))
        );
    }

    #[test]
    fn hex_reads_as_bytes() {
        let b: Bytes = serde_json::from_str("\"00ff7f\"").unwrap();
        assert_eq!(b.0, [0, 255, 127]);
        assert!(serde_json::from_str::<Bytes>("\"0g\"").is_err());
    }
}

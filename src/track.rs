//! What gw holds of a track, as the bridge reports it, and what that shows:
//! where each sector lies round the track and how it decoded, and how the
//! flux falls round it. The disk view draws these: only what was found,
//! where it was found.

use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

/// The bridge's report on a track, as bridge.py's report() prints it. A
/// part that does not parse is left out, as the bridge leaves out one it
/// cannot make, and the rest kept.
#[derive(Debug, Deserialize)]
struct Report {
    c: u32,
    h: u32,
    /// For a write: `image`, `written` or `verify`; see Source.
    source: Option<String>,
    /// gw's image holds no such track.
    #[serde(default)]
    absent: bool,
    /// Its flux's revolutions are the disk's under a head: raw flux gw read,
    /// not flux gw made, as of its master track or an image's bitcells.
    #[serde(default)]
    turned: bool,
    #[serde(default, deserialize_with = "lenient")]
    flux: Option<Flux>,
    #[serde(default, deserialize_with = "lenient")]
    codec: Option<Codec>,
}

/// A part of a report, or None where it does not parse.
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

/// A track's flux, in its sample rate's ticks, as the bridge counted it.
#[derive(Debug, Deserialize)]
struct Flux {
    /// Ticks per second.
    freq: f64,
    /// Ticks per revolution: the mean of those read, else gw's measure of the drive.
    period: f64,
    /// Each revolution read, from index pulse to index pulse.
    #[serde(default)]
    revs: Vec<f64>,
    /// Each pass the reads made round the track: from a share of a
    /// revolution from the index to another, before 0 or past 1 where it
    /// runs over the index. Each revolution whole, and what a read took
    /// before its first pulse or after its last.
    passes: Vec<[f64; 2]>,
    /// Flux transitions in equal parts of a revolution, over those passes.
    bins: Vec<u32>,
    #[serde(default, deserialize_with = "lenient")]
    intervals: Option<Counted>,
}

/// How far apart a track's flux transitions are, as the bridge counted
/// gw's flux values: in bins `width` sample ticks wide, from bin `first`,
/// the first with any, to the last before bin `top`, and how many were in
/// that bin or past it.
#[derive(Debug, Deserialize)]
struct Counted {
    width: f64,
    first: u64,
    top: u64,
    counts: Vec<u32>,
    longer: u64,
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
    /// An IBM-style track's mode, as gw names it: IBM FM, IBM MFM or DEC RX02.
    mode: Option<String>,
    /// Each decode gw made of an IBM-style track from flux.
    #[serde(default, deserialize_with = "lenient")]
    decodes: Option<Vec<Decode>>,
}

/// One decode gw made of an IBM-style track, as bridge.py's decoded_from
/// notes it: the flux's number, each revolution's bit cells and those after
/// the last index, and the areas found, in turn.
#[derive(Debug, Clone, Deserialize)]
struct Decode {
    flux: u64,
    cells: Vec<f64>,
    tail: f64,
    areas: Vec<Vec<f64>>,
}

/// An area gw's decoder found, as a decode lists it: where in bit cells from
/// the index of the revolution it lies in, `rev`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Area {
    /// An index address mark.
    Mark { rev: usize, start: f64, end: f64 },
    /// A header, and data after it.
    Whole {
        rev: usize,
        start: f64,
        header_end: f64,
        data_start: f64,
        end: f64,
        header: bool,
        data: bool,
        id: [u8; 4],
    },
    /// A header, and no data after it.
    Header {
        rev: usize,
        start: f64,
        end: f64,
        ok: bool,
        id: [u8; 4],
    },
    /// Data with no header: gw reads only its mark, so its end is not known.
    Data { rev: usize, start: f64 },
}

impl Area {
    /// An area from its list: its kind, revolution, places and what gw read.
    fn of(v: &[f64]) -> Option<Area> {
        let whole = |x: f64| x >= 0.0 && x.fract() == 0.0 && x < f64::from(u32::MAX);
        if !v.iter().all(|&x| whole(x)) {
            return None;
        }
        let byte = |x: f64| u8::try_from(x as u32).ok();
        let id = |at: &[f64]| Some([byte(at[0])?, byte(at[1])?, byte(at[2])?, byte(at[3])?]);
        let rev = *v.get(1)? as usize;
        Some(match (v.first()?, v.len()) {
            (0.0, 4) => Area::Mark {
                rev,
                start: v[2],
                end: v[3],
            },
            (1.0, 13) => Area::Whole {
                rev,
                start: v[2],
                header_end: v[3],
                data_start: v[4],
                end: v[5],
                header: v[6] == 1.0,
                data: v[7] == 1.0,
                id: id(&v[9..13])?,
            },
            (2.0, 9) => Area::Header {
                rev,
                start: v[2],
                end: v[3],
                ok: v[4] == 1.0,
                id: id(&v[5..9])?,
            },
            (3.0, 5) => Area::Data { rev, start: v[2] },
            _ => return None,
        })
    }

    fn rev(self) -> usize {
        match self {
            Area::Mark { rev, .. }
            | Area::Whole { rev, .. }
            | Area::Header { rev, .. }
            | Area::Data { rev, .. } => rev,
        }
    }

    fn start(self) -> f64 {
        match self {
            Area::Mark { start, .. }
            | Area::Whole { start, .. }
            | Area::Header { start, .. }
            | Area::Data { start, .. } => start,
        }
    }

    /// Where it ends, if gw read so far.
    fn end(self) -> Option<f64> {
        match self {
            Area::Mark { end, .. } | Area::Whole { end, .. } | Area::Header { end, .. } => {
                Some(end)
            }
            Area::Data { .. } => None,
        }
    }
}

/// A decode, its areas parsed and placed: each revolution's start in bit
/// cells from the decode's first index, then where the last ends, and
/// where the flux ends.
struct Decoded {
    flux: u64,
    starts: Vec<f64>,
    total: f64,
    areas: Vec<Area>,
}

impl Decoded {
    /// None for a decode whose parts do not all parse: its areas are
    /// numbered in the list.
    fn of(d: &Decode) -> Option<Decoded> {
        let areas = d
            .areas
            .iter()
            .map(|v| Area::of(v))
            .collect::<Option<Vec<_>>>()?;
        let mut starts = vec![0.0];
        for &cells in &d.cells {
            starts.push(starts.last()? + cells);
        }
        let total = starts.last()? + d.tail;
        let placed = areas.iter().all(|a| a.rev() < starts.len());
        placed.then_some(Decoded {
            flux: d.flux,
            starts,
            total,
            areas,
        })
    }

    /// Where `a` lies, in bit cells from the decode's first index.
    fn at(&self, a: Area) -> f64 {
        self.starts[a.rev()] + a.start()
    }
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
    /// The decode it was found in, and its place in that decode's areas.
    copy: Option<[usize; 2]>,
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
    /// See Found::copy.
    copy: Option<[usize; 2]>,
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
        struct Hex;
        impl serde::de::Visitor<'_> for Hex {
            type Value = Bytes;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bytes in hex")
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Bytes, E> {
                hex(s).map(Bytes).ok_or_else(|| E::custom("not hex"))
            }
        }
        d.deserialize_str(Hex)
    }
}

/// Bytes from hex digits, two to a byte; None for anything else.
pub(crate) fn hex(s: &str) -> Option<Vec<u8>> {
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let (pairs, odd) = s.as_bytes().as_chunks::<2>();
    if !odd.is_empty() {
        return None;
    }
    pairs
        .iter()
        .map(|&[a, b]| Some(digit(a)? << 4 | digit(b)?))
        .collect()
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
    /// The bit cell gw decoded at, in seconds.
    pub cell: Option<f64>,
    pub flux: Option<Spin>,
    /// For a write, what these are of; none for a read or a conversion,
    /// whose input they are.
    pub source: Option<Source>,
    /// gw's image holds no such track: a conversion's input, or the image a
    /// write takes its tracks from.
    pub absent: bool,
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
    pub intervals: Option<Intervals>,
}

/// How far apart a track's flux transitions are, as gw holds its flux: how
/// many of its values fall in each bin, `width` seconds wide, of whole
/// sample ticks `tick` seconds long, from bin `first`, the first with any;
/// and how many were `longer`, as long as `top` seconds or more.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Intervals {
    pub width: f64,
    pub tick: f64,
    pub first: u64,
    pub counts: Vec<u32>,
    pub top: f64,
    pub longer: u64,
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

/// A sector's ID: C, H, R and N from an IBM-style header, else its number;
/// none for a data block found with no header before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Id {
    Ibm([u8; 4]),
    Number(u32),
    None,
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
    /// How gw found it in each revolution of the disk it read.
    pub turns: Option<Turns>,
    /// Where it lies in bit cells, as gw found it in flux.
    pub layout: Option<Layout>,
}

/// How gw found a sector in a revolution, the best of its decodes of it:
/// gw's decoder sees each revolution's copy of an IBM-style sector, and
/// keeps one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Seen {
    /// Not found, in a revolution that held its place.
    NotFound,
    /// Its data, with no header before it.
    DataAlone,
    /// Its header, with no data after it.
    HeaderAlone,
    /// Its header's CRC fails.
    BadHeader,
    /// Its header's CRC holds and its data's fails.
    BadData,
    Good,
}

/// How gw found a sector in each revolution of the disk it read, in turn:
/// those read whole, then any part of one read after the last index that
/// held its place; over `reads` reads, as gw reads a track again.
#[derive(Debug, Clone, PartialEq)]
pub struct Turns {
    pub seen: Vec<Seen>,
    pub reads: usize,
}

/// Where a sector lies, in bit cells by gw's PLL as it decoded the flux:
/// from its revolution's index to its start, from the end of what gw found
/// before it, and from the end of its ID field to its data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub from_index: f64,
    /// None where gw found nothing before it after the index the decode
    /// starts at, or only a data block's mark.
    pub after: Option<(f64, Before)>,
    pub id_to_data: Option<f64>,
}

/// What gw found just before a sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Before {
    IndexMark,
    /// A header and its data: its ID, and whether its CRC holds.
    Sector(Id, bool),
    /// A header with no data after it.
    Header(Id, bool),
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

/// Areas whose starts lie this close, in bit cells, are one area found
/// again in another revolution or decode: in real reads one moves under 8
/// cells between revolutions, and two headers or data blocks start at
/// least an FM ID field, 112 cells, apart. gw's own 1000 would take a
/// header 44 bytes before a sector for part of it.
const COPY: f64 = 64.0;

/// The layout of the area a sector was found as, `copy`, in its decode;
/// `mmfm` where a data block's end in FM cells does not hold, as a DEC
/// RX02 track's double-density data's does not.
fn layout(decodes: &[Option<Decoded>], copy: Option<[usize; 2]>, mmfm: bool) -> Option<Layout> {
    let [d, i] = copy?;
    let decode = decodes.get(d)?.as_ref()?;
    let area = *decode.areas.get(i)?;
    let after = i.checked_sub(1).and_then(|j| {
        let before = decode.areas[j];
        let end = decode.starts[before.rev()] + before.end()?;
        let what = match before {
            Area::Mark { .. } => Before::IndexMark,
            Area::Whole { .. } if mmfm => return None,
            Area::Whole { id, header, .. } => Before::Sector(Id::Ibm(id), header),
            Area::Header { id, ok, .. } => Before::Header(Id::Ibm(id), ok),
            Area::Data { .. } => return None,
        };
        Some((decode.at(area) - end, what))
    });
    let id_to_data = match area {
        Area::Whole {
            header_end,
            data_start,
            ..
        } => Some(data_start - header_end),
        _ => None,
    };
    Some(Layout {
        from_index: area.start(),
        after,
        id_to_data,
    })
}

/// How gw found the sector it kept as area `copy` in each revolution of
/// each flux: the best of each decode's copies within COPY of its place. A
/// revolution the read held to COPY past the sector's end counts it not
/// found without one; any other counts only a copy found whole.
fn turns(decodes: &[Option<Decoded>], copy: Option<[usize; 2]>) -> Option<Turns> {
    let [d, i] = copy?;
    let kept = *decodes.get(d)?.as_ref()?.areas.get(i)?;
    let place = kept.start();
    let length = kept.end().map_or(0.0, |end| end - place);
    // Where its data starts after its start, to know its data found alone.
    let data = match kept {
        Area::Whole {
            start, data_start, ..
        } => Some(data_start - start),
        Area::Data { .. } => Some(0.0),
        _ => None,
    };
    let decodes: Vec<&Decoded> = decodes.iter().flatten().collect();
    let mut fluxes: Vec<u64> = Vec::new();
    for x in &decodes {
        if !fluxes.contains(&x.flux) {
            fluxes.push(x.flux);
        }
    }
    let mut seen = Vec::new();
    for &flux in &fluxes {
        let of: Vec<&&Decoded> = decodes.iter().filter(|x| x.flux == flux).collect();
        for k in 0..of[0].starts.len() {
            let best = of.iter().filter_map(|x| {
                let at = x.starts.get(k)? + place;
                let found = x.areas.iter().filter_map(|&a| {
                    let (to, how) = match a {
                        Area::Whole { header, data, .. } => (
                            at,
                            match (header, data) {
                                (true, true) => Seen::Good,
                                (true, false) => Seen::BadData,
                                (false, _) => Seen::BadHeader,
                            },
                        ),
                        Area::Header { .. } => (at, Seen::HeaderAlone),
                        Area::Data { .. } => (at + data?, Seen::DataAlone),
                        Area::Mark { .. } => return None,
                    };
                    let off = (x.at(a) - to).abs();
                    (off < COPY).then_some((off, how))
                });
                found.min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, how)| how)
            });
            // Where the read ran out, only a sector found whole counts: a
            // header with no data may be one the read cut short.
            let held = of[0].starts[k] + place + length + COPY <= of[0].total;
            match best.max() {
                Some(how) if held || how >= Seen::BadHeader => seen.push(how),
                None if held => seen.push(Seen::NotFound),
                _ => {}
            }
        }
    }
    Some(Turns {
        seen,
        reads: fluxes.len(),
    })
}

impl Facts {
    /// Parses a TRACK line's JSON: the track's cylinder and head, and its facts.
    pub fn parse(json: &str) -> Option<((u32, u32), Facts)> {
        let report: Report = serde_json::from_str(json).ok()?;
        let turned = report.turned;
        let mut facts = report
            .codec
            .map(|codec| Facts::decoded(codec, turned))
            .unwrap_or_default();
        facts.flux = report.flux.as_ref().and_then(spin);
        facts.absent = report.absent;
        facts.source = match report.source.as_deref() {
            Some("image") => Some(Source::Image),
            Some("written") => Some(Source::Written),
            Some("verify") => Some(Source::Verify),
            _ => None,
        };
        Some(((report.c, report.h), facts))
    }

    /// What gw decoded of a track; how it found each sector in each
    /// revolution only where they are the disk's, `turned`.
    fn decoded(mut codec: Codec, turned: bool) -> Facts {
        let mut facts = Facts {
            summary: Some(std::mem::take(&mut codec.summary)),
            cell: codec.clock,
            ..Facts::default()
        };
        // An IBM-style track keeps its places whatever gw found whole: the
        // bridge gives its mode, where reports before it had sectors or a layout.
        let ibm = codec.mode.is_some() || !codec.found.is_empty() || codec.laid.is_some();
        match codec.time_per_rev.zip(codec.clock) {
            Some((per_rev, cell)) if ibm => facts.ibm(&mut codec, Timing { per_rev, cell }, turned),
            _ => facts.numbered(&mut codec),
        }
        facts
    }

    /// An IBM-style track's sectors, placed as gw found them, with how gw
    /// found each in each revolution where they are the disk's, `turned`.
    fn ibm(&mut self, codec: &mut Codec, timing: Timing, turned: bool) {
        let mut found = std::mem::take(&mut codec.found);
        let laid = codec.laid.as_deref();
        let decodes: Vec<Option<Decoded>> =
            codec.decodes.iter().flatten().map(Decoded::of).collect();
        let mmfm = codec.mode.as_deref() == Some("DEC RX02");
        let turns = |copy| if turned { turns(&decodes, copy) } else { None };
        let expected = |id: &[u8; 4]| laid.is_none_or(|l| l.iter().any(|s| s.id == *id));
        for s in &mut found {
            let bytes = s.bytes.take().unwrap_or_default().0;
            let data = match s.data {
                false => Data::Bad,
                true => match bytes.first() {
                    Some(&b) if bytes.iter().all(|&x| x == b) => Data::Empty(b),
                    _ => Data::Good,
                },
            };
            let [start, header_end, data_start, end] = timing.shares(
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
                turns: turns(s.copy),
                layout: layout(&decodes, s.copy, mmfm),
            });
        }
        // Blocks found apart, once each, unless part of a sector found
        // whole; of a header seen twice, one whose CRC holds, as gw keeps.
        let mut apart: Vec<&Apart> = Vec::new();
        for a in &codec.apart {
            let whole = found.iter().any(|s| match a.id {
                Some(_) => (s.start - a.start).abs() < COPY,
                None => (s.data_start - a.start).abs() < COPY,
            });
            let seen = apart
                .iter()
                .position(|b| b.id.is_some() == a.id.is_some() && (b.start - a.start).abs() < COPY);
            match seen {
                _ if whole => {}
                Some(i) if a.header == Some(true) && apart[i].header != Some(true) => apart[i] = a,
                Some(_) => {}
                None => apart.push(a),
            }
        }
        for a in apart {
            let [start, end] = timing.shares([a.start, a.end], a.times, a.turn);
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
                    turns: turns(a.copy),
                    layout: layout(&decodes, a.copy, mmfm),
                },
                None => Sector {
                    id: Id::None,
                    at: Some(span(start, start, end)),
                    header_end: None,
                    header: Header::None,
                    data: Data::Unread,
                    mark: a.mark,
                    bytes: Vec::new(),
                    extra: false,
                    before: false,
                    turns: turns(a.copy),
                    layout: layout(&decodes, a.copy, mmfm),
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
    }

    /// Another codec's sectors, by number, placed where gw found them.
    fn numbered(&mut self, codec: &mut Codec) {
        for &n in &codec.good {
            let bytes = codec.data.remove(&n).unwrap_or_default().0;
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
                turns: None,
                layout: None,
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

/// An IBM-style track's revolution as gw decodes it, and its bit cell, in seconds.
#[derive(Clone, Copy)]
struct Timing {
    per_rev: f64,
    cell: f64,
}

impl Timing {
    /// Where IBM-style areas lie round the track, as shares of a revolution
    /// from the index: by time where gw decoded them from flux, `times` from
    /// the index their revolution starts at and `turn` its length, else gw's
    /// revolution; with none, by `bits` of the format's bit cells, as an
    /// image's track is laid out.
    fn shares<const N: usize>(
        self,
        bits: [f64; N],
        times: Option<[f64; N]>,
        turn: Option<f64>,
    ) -> [f32; N] {
        match times {
            Some(times) => times.map(|t| (t / turn.unwrap_or(self.per_rev)) as f32),
            None => {
                let cells = self.per_rev / self.cell;
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
    // No revolution, so no index to place it from.
    if p.revs.is_empty() {
        return None;
    }
    let mut index = Vec::new();
    let mut at = 0.0;
    for (i, &length) in p.revs.iter().enumerate() {
        // What was read before the first index makes no revolution.
        if p.cued || i > 0 {
            index.push(at);
        }
        at += length;
    }
    index.push(at);
    // The last index at or before the sector, or the first, and the
    // revolution after it, or before it.
    let after = index.iter().rposition(|&x| x <= p.at);
    let i = after.unwrap_or(0);
    let length = match (index.get(i + 1), i.checked_sub(1)) {
        (Some(next), _) => next - index[i],
        (None, Some(last)) => index[i] - index[last],
        (None, None) => per_rev?,
    };
    let share = |x: f64| ((x - index[i]) / length) as f32;
    let at = span(share(p.at), share(p.data.unwrap_or(p.at)), share(p.end));
    Some((at, after.is_none()))
}

/// How the disk turned and its flux fell, from the bridge's count of it:
/// each part's transitions over how often the passes crossed it. None for
/// a count the bridge does not make.
fn spin(f: &Flux) -> Option<Spin> {
    let parts = f.bins.len();
    // A pass runs from at most a revolution before the index, for under two.
    let sane =
        |&[from, to]: &[f64; 2]| from < to && to - from < 2.0 && (-1.0..=1.0).contains(&from);
    let positive = |x: &f64| *x > 0.0;
    if parts == 0
        || ![f.period, f.freq].iter().all(positive)
        || !f.passes.iter().all(sane)
        || !f.revs.iter().all(positive)
    {
        return None;
    }
    let n = parts as f64;
    let mut cover = vec![0.0; parts];
    for &[from, to] in &f.passes {
        let mut part = (from * n).floor();
        while part < to * n {
            let covered = (to * n).min(part + 1.0) - (from * n).max(part);
            cover[part.rem_euclid(n) as usize] += covered.max(0.0);
            part += 1.0;
        }
    }
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
    let intervals = f
        .intervals
        .as_ref()
        .filter(|c| c.width > 0.0)
        .map(|c| Intervals {
            width: c.width / f.freq,
            tick: 1.0 / f.freq,
            first: c.first,
            counts: c.counts.clone(),
            top: c.top as f64 * c.width / f.freq,
            longer: c.longer,
        });
    Some(Spin {
        period: f.period / f.freq,
        revs: f.revs.iter().map(|r| r / f.freq).collect(),
        per_rev: bins.iter().map(|&b| f64::from(b)).sum(),
        bins,
        intervals,
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
        // Sectors abut but for one seam: those read before the first index,
        // placed back by the next revolution's length, meet the rest.
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
        // A whole track of data: its flux even round it.
        let relative = facts.flux.unwrap().relative();
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
            period: 100.0,
            revs: vec![100.0],
            passes: vec![[0.5, 1.0], [0.0, 1.0]],
            bins: vec![2, 2, 1, 1],
            intervals: None,
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
        let none = Place {
            revs: Vec::new(),
            ..p.clone()
        };
        assert_eq!(
            place(&none, Some(100.0)),
            None,
            "no revolution to place it in"
        );
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
        assert_eq!(hex("+f"), None, "only hex digits");
        assert_eq!(hex("abc"), None, "two to a byte");
    }

    /// An IBM-style track as the bridge reports an image's: its format lays
    /// out sectors 1 and 2, neither found whole, and gw found these blocks
    /// by themselves.
    fn apart(blocks: serde_json::Value) -> Facts {
        let laid = |r: u8, start: u32| {
            serde_json::json!({"id": [0, 0, r, 2], "start": start, "header_end": start + 100,
                "data_start": start + 200, "end": start + 4400, "header": false,
                "data": false, "mark": 251})
        };
        let report = serde_json::json!({"c": 0, "h": 0, "codec": {
            "summary": "IBM MFM (0/2 sectors)", "nsec": 2, "good": [],
            "time_per_rev": 0.2, "clock": 2e-6, "found": [], "apart": blocks,
            "laid": [laid(1, 100), laid(2, 50_000)]}});
        Facts::parse(&report.to_string()).unwrap().1
    }

    #[test]
    fn data_found_with_no_header_has_no_id_and_its_sectors_are_missing() {
        let facts = apart(serde_json::json!([
            {"id": null, "header": null, "start": 20_000, "end": 24_000, "mark": 251}
        ]));
        let [s] = &facts.sectors[..] else {
            panic!("{:?}", facts.sectors)
        };
        assert_eq!(
            (s.id, s.header, s.data),
            (Id::None, Header::None, Data::Unread)
        );
        assert_eq!(s.at, Some([0.2, 0.2, 0.24]), "by the format's bit cells");
        let ids = [Id::Ibm([0, 0, 1, 2]), Id::Ibm([0, 0, 2, 2])];
        assert_eq!(facts.missing, ids);
    }

    #[test]
    fn a_header_found_twice_by_itself_is_the_copy_whose_crc_holds() {
        let header = |crc: bool, start: u32| {
            serde_json::json!({"id": [0, 0, 1, 2], "header": crc, "start": start,
                "end": start + 100})
        };
        let facts = apart(serde_json::json!([header(false, 100), header(true, 110)]));
        let [s] = &facts.sectors[..] else {
            panic!("{:?}", facts.sectors)
        };
        assert_eq!((s.header, s.data), (Header::Good, Data::None));
        let facts = apart(serde_json::json!([header(true, 100), header(false, 110)]));
        let [s] = &facts.sectors[..] else {
            panic!("{:?}", facts.sectors)
        };
        assert_eq!(s.header, Header::Good, "kept");
    }

    #[test]
    fn a_part_of_a_report_that_does_not_parse_leaves_the_rest() {
        // gw's revolution unmeasured: no period.
        let report = serde_json::json!({"c": 1, "h": 0,
            "flux": {"freq": 1000, "period": null, "revs": [], "passes": [[0.0, 1.0]],
                     "bins": [1, 1]},
            "codec": {"summary": "AmigaDOS (1/11 sectors)", "nsec": 11, "good": [0],
                      "data": {"0": "00ff"}}});
        let (key, facts) = Facts::parse(&report.to_string()).unwrap();
        assert_eq!(key, (1, 0));
        assert!(facts.flux.is_none());
        assert_eq!(facts.sectors.len(), 1);
        assert_eq!(facts.sectors[0].bytes, [0, 255]);
        assert_eq!(facts.missing.len(), 10);
    }

    #[test]
    fn a_track_gws_image_does_not_hold_is_said_to_be_absent() {
        let (key, facts) = Facts::parse(r#"{"c":16,"h":0,"absent":true}"#).unwrap();
        assert_eq!(key, (16, 0));
        assert!(facts.absent && facts.flux.is_none() && facts.sectors.is_empty());
        assert!(!Facts::parse(r#"{"c":16,"h":0}"#).unwrap().1.absent);
    }

    #[test]
    fn a_count_the_bridge_does_not_make_shows_no_flux() {
        let flux = |revs: Vec<f64>, passes: Vec<[f64; 2]>| Flux {
            freq: 1000.0,
            period: 100.0,
            revs,
            passes,
            bins: vec![1; 4],
            intervals: None,
        };
        assert!(spin(&flux(vec![100.0], vec![[0.0, 1.0]])).is_some());
        assert!(
            spin(&flux(vec![0.0], vec![[0.0, 1.0]])).is_none(),
            "a revolution of no length"
        );
        assert!(
            spin(&flux(vec![100.0], vec![[-9.0, 1.0]])).is_none(),
            "nine before"
        );
        assert!(
            spin(&flux(vec![100.0], vec![[0.0, 1e9]])).is_none(),
            "on and on after"
        );
        assert!(
            spin(&flux(vec![100.0], vec![[0.5, 0.5]])).is_none(),
            "nowhere"
        );
    }

    #[test]
    fn a_pass_over_the_index_counts_on_either_side_of_it() {
        let flux = |passes: Vec<[f64; 2]>| Flux {
            freq: 1000.0,
            period: 100.0,
            revs: vec![100.0],
            passes,
            bins: vec![1; 4],
            intervals: None,
        };
        let bins = |passes| spin(&flux(passes)).unwrap().bins;
        // From a quarter before the index: its last part, then the first three.
        assert_eq!(bins(vec![[-0.25, 0.75]]), [1.0, 1.0, 1.0, 1.0]);
        // On a quarter past it: the second part never passed.
        assert_eq!(bins(vec![[0.5, 1.25]]), [1.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn a_sector_its_format_does_not_lay_out_is_said_to_be_so() {
        let report = serde_json::json!({"c": 0, "h": 0, "codec": {
            "summary": "IBM MFM (1/1 sectors)", "nsec": 1, "good": [0],
            "time_per_rev": 0.2, "clock": 2e-6, "mode": "IBM MFM",
            "found": [{"id": [0, 0, 9, 2], "start": 1001, "header_end": 1161,
                "data_start": 1705, "end": 10001, "header": true, "data": true,
                "mark": 251}],
            "laid": [{"id": [0, 0, 1, 2], "start": 0, "header_end": 0, "data_start": 0,
                "end": 0, "header": false, "data": false, "mark": 251}]}});
        let facts = Facts::parse(&report.to_string()).unwrap().1;
        let [s] = &facts.sectors[..] else {
            panic!("{:?}", facts.sectors)
        };
        assert!(s.extra, "R9, where the format lays out R1");
        assert_eq!(facts.missing, [Id::Ibm([0, 0, 1, 2])]);
    }

    /// An IBM-style track as the bridge reports one from flux the disk
    /// turned under the head: sector R1 kept as `copy`, with each decode.
    fn decoded(copy: [usize; 2], decodes: serde_json::Value) -> Facts {
        let report = serde_json::json!({"c": 0, "h": 0, "turned": true, "codec": {
            "summary": "IBM MFM (1/1 sectors)", "nsec": 1, "good": [0],
            "time_per_rev": 0.2, "clock": 2e-6, "mode": "IBM MFM",
            "found": [{"id": [0, 0, 1, 2], "start": 1001, "header_end": 1161,
                "data_start": 1705, "end": 10001, "header": true, "data": true,
                "mark": 251, "copy": copy}],
            "apart": [], "decodes": decodes}});
        Facts::parse(&report.to_string()).unwrap().1
    }

    /// Sector R1 whole in revolution `rev`, `start` cells from its index,
    /// its header's CRC and its data's holding or not.
    fn whole(rev: u32, start: u32, header: u8, data: u8) -> serde_json::Value {
        serde_json::json!([
            1,
            rev,
            start,
            start + 160,
            start + 704,
            start + 9000,
            header,
            data,
            251,
            0,
            0,
            1,
            2
        ])
    }

    #[test]
    fn each_revolution_counts_once_as_the_best_of_gws_decodes_of_its_flux() {
        let facts = decoded(
            [1, 0],
            serde_json::json!([
                // Flux 7 decoded twice, as gw does again with another PLL:
                // revolution 0 bad, then good; revolution 1 good.
                {"flux": 7, "cells": [100_000, 100_000], "tail": 5000,
                 "areas": [whole(0, 1000, 1, 0), whole(1, 1003, 1, 1)]},
                {"flux": 7, "cells": [100_001, 100_001], "tail": 5000,
                 "areas": [whole(0, 1001, 1, 1)]},
                // A read again: its header alone, then a revolution that
                // holds its place and none of it.
                {"flux": 8, "cells": [100_000, 100_000], "tail": 0,
                 "areas": [[2, 0, 1002, 1162, 1, 0, 0, 1, 2]]},
            ]),
        );
        let turns = facts.sectors[0].turns.clone().unwrap();
        // Flux 7's part after its last index ends 5000 cells on: short of
        // the sector's end, so not counted; flux 8's has none.
        assert_eq!(
            turns.seen,
            [Seen::Good, Seen::Good, Seen::HeaderAlone, Seen::NotFound]
        );
        assert_eq!(turns.reads, 2);
    }

    #[test]
    fn where_a_read_ran_out_only_a_sector_found_whole_counts() {
        let facts = decoded(
            [0, 0],
            serde_json::json!([{"flux": 1, "cells": [100_000], "tail": 9500, "areas": [
                whole(0, 1001, 1, 1),
                // After the last index, the read ran out within its data:
                // its header alone says nothing of the disk.
                [2, 1, 1001, 1161, 1, 0, 0, 1, 2],
            ]}]),
        );
        assert_eq!(facts.sectors[0].turns.as_ref().unwrap().seen, [Seen::Good]);
        let found = decoded(
            [0, 0],
            serde_json::json!([{"flux": 1, "cells": [100_000], "tail": 9500, "areas": [
                whole(0, 1001, 1, 1), whole(1, 1001, 1, 0)]}]),
        );
        let seen = &found.sectors[0].turns.as_ref().unwrap().seen;
        assert_eq!(
            seen,
            &[Seen::Good, Seen::BadData],
            "found whole all the same"
        );
    }

    #[test]
    fn a_sectors_layout_is_in_bit_cells_from_its_index_and_what_gw_found_before_it() {
        let facts = decoded(
            [0, 1],
            serde_json::json!([{"flux": 1, "cells": [100_000], "tail": 0, "areas": [
                [0, 0, 400, 464], whole(0, 1001, 1, 1)]}]),
        );
        let layout = facts.sectors[0].layout.unwrap();
        assert_eq!(layout.from_index, 1001.0);
        assert_eq!(layout.after, Some((537.0, Before::IndexMark)));
        assert_eq!(layout.id_to_data, Some(544.0));
        // Across the index from the sector before; after a data block's mark
        // alone, its end not known.
        let across = decoded(
            [0, 1],
            serde_json::json!([{"flux": 1, "cells": [100_000, 100_000], "tail": 0, "areas": [
                [2, 0, 99_000, 99_160, 1, 0, 0, 9, 2], whole(1, 1001, 1, 1)]}]),
        );
        let after = across.sectors[0].layout.unwrap().after;
        assert_eq!(
            after,
            Some((1841.0, Before::Header(Id::Ibm([0, 0, 9, 2]), true)))
        );
        let mark = decoded(
            [0, 1],
            serde_json::json!([{"flux": 1, "cells": [100_000], "tail": 0, "areas": [
                [3, 0, 400, 464, 251], whole(0, 1001, 1, 1)]}]),
        );
        assert_eq!(mark.sectors[0].layout.unwrap().after, None);
        // Not the disk's revolutions: where it lies, but no count of them.
        let report = serde_json::json!({"c": 0, "h": 0, "codec": {
            "summary": "IBM MFM (1/1 sectors)", "nsec": 1, "good": [0],
            "time_per_rev": 0.2, "clock": 2e-6, "found": [{"id": [0, 0, 1, 2],
            "start": 1001, "header_end": 1161, "data_start": 1705, "end": 10001,
            "header": true, "data": true, "mark": 251, "copy": [0, 0]}],
            "decodes": [{"flux": 1, "cells": [100_000], "tail": 0,
                "areas": [whole(0, 1001, 1, 1)]}]}});
        let made = Facts::parse(&report.to_string()).unwrap().1;
        assert!(made.sectors[0].layout.is_some() && made.sectors[0].turns.is_none());
    }

    #[test]
    fn a_header_alone_just_before_a_sector_is_its_own() {
        // As gw lays out an EDSK's header with no data: 44 bytes before the
        // next sector, inside gw's own 1000 cells for one sector.
        let report = serde_json::json!({"c": 0, "h": 0, "codec": {
            "summary": "IBM MFM (1/1 sectors)", "nsec": 1, "good": [0],
            "time_per_rev": 0.2, "clock": 2e-6,
            "found": [{"id": [0, 0, 7, 2], "start": 43_232, "header_end": 43_392,
                "data_start": 43_936, "end": 52_224, "header": true, "data": true,
                "mark": 251}],
            "apart": [{"id": [0, 0, 6, 2], "header": true, "start": 42_528, "end": 42_688}]}});
        let facts = Facts::parse(&report.to_string()).unwrap().1;
        let ids: Vec<Id> = facts.sectors.iter().map(|s| s.id).collect();
        assert_eq!(ids, [Id::Ibm([0, 0, 6, 2]), Id::Ibm([0, 0, 7, 2])]);
    }

    #[test]
    fn a_block_found_apart_within_a_sector_found_whole_is_part_of_it() {
        // R1's header and data found again 10 cells on, and a header 70 on.
        let report = serde_json::json!({"c": 0, "h": 0, "codec": {
            "summary": "IBM MFM (1/1 sectors)", "nsec": 1, "good": [0],
            "time_per_rev": 0.2, "clock": 2e-6, "mode": "IBM MFM",
            "found": [{"id": [0, 0, 1, 2], "start": 1001, "header_end": 1161,
                "data_start": 1705, "end": 10001, "header": true, "data": true,
                "mark": 251}],
            "apart": [
                {"id": [0, 0, 1, 2], "header": true, "start": 1011, "end": 1171},
                {"id": null, "header": null, "start": 1715, "end": 10011, "mark": 251},
                {"id": [0, 0, 9, 2], "header": true, "start": 1071, "end": 1231}]}});
        let facts = Facts::parse(&report.to_string()).unwrap().1;
        let ids: Vec<Id> = facts.sectors.iter().map(|s| s.id).collect();
        assert_eq!(ids, [Id::Ibm([0, 0, 1, 2]), Id::Ibm([0, 0, 9, 2])]);
    }

    #[test]
    fn a_revolution_that_found_only_a_sectors_data_where_it_lies_counts_it_as_data_alone() {
        let with_data_at = |at: u32| {
            let areas = serde_json::json!([whole(0, 1001, 1, 1), [3, 1, at, at + 8296, 251]]);
            let decodes = serde_json::json!([{"flux": 1, "cells": [100_000, 100_000],
                "tail": 0, "areas": areas}]);
            decoded([0, 0], decodes).sectors[0]
                .turns
                .clone()
                .unwrap()
                .seen
        };
        assert_eq!(with_data_at(1001 + 704), [Seen::Good, Seen::DataAlone]);
        assert_eq!(
            with_data_at(1001),
            [Seen::Good, Seen::NotFound],
            "not where its data lies"
        );
    }

    #[test]
    fn an_ibm_track_with_no_sector_found_whole_keeps_what_gw_found_apart() {
        // As a write with no format decodes its verify: no layout, no sector.
        let report = serde_json::json!({"c": 0, "h": 0, "codec": {
            "summary": "IBM MFM (0/0 sectors)", "nsec": 0, "good": [],
            "time_per_rev": 0.2, "clock": 2e-6, "mode": "IBM MFM", "found": [],
            "apart": [{"id": [0, 0, 1, 2], "header": true, "start": 1001, "end": 1161}]}});
        let facts = Facts::parse(&report.to_string()).unwrap().1;
        let [s] = &facts.sectors[..] else {
            panic!("{:?}", facts.sectors)
        };
        assert_eq!(
            (s.id, s.header, s.data),
            (Id::Ibm([0, 0, 1, 2]), Header::Good, Data::None)
        );
    }

    #[test]
    fn intervals_are_counted_in_seconds_as_the_bridge_binned_its_ticks() {
        let flux = Flux {
            freq: 40e6,
            period: 8e6,
            revs: vec![8e6],
            passes: vec![[0.0, 1.0]],
            bins: vec![1; 4],
            intervals: Some(Counted {
                width: 2.0,
                first: 80,
                top: 400,
                counts: vec![3, 0, 5],
                longer: 2,
            }),
        };
        let i = spin(&flux).unwrap().intervals.unwrap();
        assert!((i.width - 50e-9).abs() < 1e-15 && (i.top - 20e-6).abs() < 1e-12);
        assert!((i.tick - 25e-9).abs() < 1e-18, "a 40 MHz tick");
        assert_eq!((i.first, i.counts, i.longer), (80, vec![3, 0, 5], 2));
    }

    /// Bytes no disk holds of its own: gw's filler, one byte over and over,
    /// or tests/data/scrub.py's count.
    fn made_up(bytes: &[u8]) -> bool {
        let filler = b"-=[BAD SECTOR]=-";
        bytes.windows(2).all(|w| w[0] == w[1])
            || bytes.chunks(16).all(|c| c == &filler[..c.len()])
            || bytes.windows(2).all(|w| w[1] == w[0].wrapping_add(1))
    }

    #[test]
    fn every_recorded_report_parses_and_holds_no_disks_data() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data");
        let mut reports = 0;
        for file in std::fs::read_dir(dir).unwrap() {
            let path = file.unwrap().path();
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in text.lines() {
                let named = |v: &serde_json::Value| -> Vec<String> {
                    let mut hex = Vec::new();
                    walk(v, &mut hex);
                    hex
                };
                let (json, kind) = match line.split_once(' ') {
                    Some(("@ferriteweazle", rest)) => match rest.split_once(' ') {
                        Some((kind @ ("track" | "image"), json)) => (json, kind),
                        _ => continue,
                    },
                    _ => continue,
                };
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                // Each part as well as the line: a part of the wrong shape is
                // left out, the rest kept.
                match (kind, v["event"].as_str()) {
                    ("track", _) => {
                        let parsed = Facts::parse(json);
                        let (_, f) = parsed.unwrap_or_else(|| panic!("{path:?}: {json:.80}"));
                        assert!(v["flux"].is_null() || f.flux.is_some(), "{path:?}: flux");
                        assert!(
                            v["codec"].is_null() || f.summary.is_some(),
                            "{path:?}: codec"
                        );
                    }
                    (_, Some("open")) => {
                        assert!(
                            crate::image::Image::parse(&v).is_some(),
                            "{path:?}: {json:.80}"
                        );
                    }
                    (_, Some("routes")) => {
                        assert!(crate::image::routes(&v).is_some(), "{path:?}: {json:.80}");
                    }
                    _ => {}
                }
                for h in named(&v) {
                    let bytes = hex(&h).unwrap_or_else(|| panic!("{path:?}: not hex"));
                    assert!(
                        made_up(&bytes),
                        "{path:?}: a disk's data; see tests/data/scrub.py"
                    );
                }
                reports += 1;
            }
        }
        assert!(reports > 1000, "{reports}");

        /// Every "bytes" string and "data" value in a report.
        fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
            match v {
                serde_json::Value::Object(m) => {
                    for (k, x) in m {
                        match (k.as_str(), x) {
                            ("bytes", serde_json::Value::String(h)) => out.push(h.clone()),
                            ("data", serde_json::Value::Object(d)) => {
                                out.extend(d.values().filter_map(|h| h.as_str().map(str::to_owned)))
                            }
                            _ => walk(x, out),
                        }
                    }
                }
                serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
    }
}

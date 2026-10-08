//! The image a disk job makes or takes its tracks from, as the bridge
//! reports gw's own image object: how gw lays out its file, track by track
//! and sector by sector; which sectors of a made image hold data and which
//! gw's filler; and the file as gw wrote it, checked byte by byte.

use crate::progress::Progress;
use crate::track::hex;
use serde_json::Value;
use std::collections::BTreeMap;

/// What a job does with its image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Makes it, as a read does, or a conversion its output.
    Made,
    /// Takes its tracks from it, as a write does, or a conversion its input.
    Source,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub role: Role,
    /// The file, as gw names it.
    pub file: Option<String>,
    /// gw's name for its type, such as IMG, ADF or SCP.
    pub kind: String,
    /// How gw lays out its file: None where gw keeps its type some other
    /// way, such as flux.
    pub layout: Option<Layout>,
    /// A source's size in bytes.
    pub size: Option<u64>,
    /// A laid out source's bytes as gw read them, up to its layout's end.
    pub content: Option<Vec<u8>>,
    /// A made image's tracks as gw put them in it, keyed as gw keeps them.
    pub tracks: BTreeMap<(u32, u32), Held>,
    /// A made image's file once gw has made its bytes.
    pub written: Option<Written>,
    /// A source gw lays out its type of, whose file is not as gw lays it out.
    pub differs: bool,
}

/// A track as gw put it in an image it makes: whether each sector, by its
/// index on the track, holds data, and the bytes it takes in the file; and
/// whether its sectors are the layout's, `laid`, not an input image's own
/// track gw passed through.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub has: Vec<bool>,
    pub bytes: Vec<u8>,
    pub laid: bool,
}

/// A track a conversion goes through: as gw's track lines name it, where it
/// lies in the input image, and where gw puts it in the output. None where
/// gw's track lists move it before the first cylinder, as with h0.off=-1:
/// no image holds it there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub own: (u32, u32),
    pub from: Option<(u32, u32)>,
    pub to: Option<(u32, u32)>,
}

impl Route {
    /// Where the track lies in an image of `role`.
    fn place(&self, role: Role) -> Option<(u32, u32)> {
        match role {
            Role::Source => self.from,
            Role::Made => self.to,
        }
    }
}

/// The image a write or a conversion is to take its tracks from, as gw
/// opens it before any job: what its job reports of it on opening.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(try_from = "Value")]
pub struct Preview(pub Image);

impl TryFrom<Value> for Preview {
    type Error = String;

    fn try_from(v: Value) -> Result<Preview, String> {
        Image::parse(&v)
            .map(Preview)
            .ok_or_else(|| "gw reported no image.".to_owned())
    }
}

/// A `routes` event's tracks: whole numbers, or none of it.
pub fn routes(v: &Value) -> Option<Vec<Route>> {
    v["tracks"]
        .as_array()?
        .iter()
        .map(|t| {
            let n: Vec<i64> = (0..6).map(|i| t.get(i)?.as_i64()).collect::<Option<_>>()?;
            let at = |i: usize| Some((u32::try_from(n[i]).ok()?, u32::try_from(n[i + 1]).ok()?));
            Some(Route {
                own: at(0)?,
                from: at(2),
                to: at(4),
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// The file's tracks, in turn.
    pub tracks: Vec<Laid>,
    /// The bytes gw puts in place of a sector it lacks, by kind.
    pub fillers: Vec<Vec<u8>>,
    /// The least cylinders the file holds: past them, gw writes cylinders
    /// up to the last it holds data in.
    pub min_cyls: Option<u32>,
}

/// A track of the file, keyed as gw keeps it, and its sectors in turn.
#[derive(Debug, Clone, PartialEq)]
pub struct Laid {
    pub key: (u32, u32),
    pub sectors: Vec<Part>,
}

/// A sector's part of the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// Its index among the track's sectors, as gw numbers them.
    pub index: usize,
    /// An IBM-style sector's C, H, R and N.
    pub id: Option<[u8; 4]>,
    pub len: u64,
    /// The filler that takes its place where gw lacks it.
    pub filler: usize,
}

/// A track's bytes in the file: its key, and where they start and end.
pub type Extent = ((u32, u32), u64, u64);

#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    pub size: u64,
    /// Each track's bytes, if every track there is gw's bytes for it, in
    /// turn, to the file's end; None if they are not.
    pub tracks: Option<Vec<Extent>>,
}

/// What a part of the file holds, or will.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// A made image's: the sector's data, as gw read or decoded it; a
    /// source's, as its file holds it.
    Data,
    /// gw's filler, in place of a sector it could not read; or as a
    /// source's file holds it.
    Filler,
    /// gw's filler, for a track the job did not read.
    Unread,
    /// A track gw is yet to read.
    ToDo,
    /// Past the end of a source's file: gw takes it as zeros.
    PastEnd,
}

/// How far a job has gone with its image.
pub struct Job<'a> {
    pub progress: &'a Progress,
    pub running: bool,
}

impl Job<'_> {
    /// Whether the job is to take or put an image of `role`'s track `key`:
    /// by a conversion's routes, else by the tracks gw announced, which its
    /// track lines name as the image does.
    fn planned(&self, role: Role, key: (u32, u32)) -> bool {
        let p = self.progress;
        match &p.routes {
            Some(routes) => routes.iter().any(|r| r.place(role) == Some(key)),
            None => p.cyls.contains(&key.0) && p.heads.contains(&key.1),
        }
    }

    /// Where in an image of `role` the track gw last reported lies, while
    /// the job runs: as its track lines name it, moved by a conversion's
    /// routes.
    pub fn current(&self, role: Role) -> Option<(u32, u32)> {
        let own = self.progress.current.filter(|_| self.running)?;
        match &self.progress.routes {
            Some(routes) => routes.iter().find(|r| r.own == own)?.place(role),
            None => Some(own),
        }
    }

    /// The names gw's track lines give an image of `role`'s track `key`,
    /// where a conversion's routes move it from its own.
    pub fn named(&self, role: Role, key: (u32, u32)) -> Vec<(u32, u32)> {
        let routes = self.progress.routes.iter().flatten();
        routes
            .filter(|r| r.place(role) == Some(key) && r.own != key)
            .map(|r| r.own)
            .collect()
    }
}

/// A track of the file where it lies, its sectors' parts where they lie
/// and what each holds. `kept` is false, until the file is written, for a
/// made image's track past its least cylinders, which gw writes only up to
/// the last cylinder holding data.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub key: (u32, u32),
    pub start: u64,
    pub len: u64,
    pub parts: Vec<(Part, u64, State)>,
    pub kept: bool,
}

impl Image {
    /// An `open` event's image.
    pub fn parse(v: &Value) -> Option<Image> {
        let role = match v["role"].as_str()? {
            "made" => Role::Made,
            "source" => Role::Source,
            _ => return None,
        };
        Some(Image {
            role,
            file: v["file"].as_str().map(str::to_owned),
            kind: v["type"].as_str().unwrap_or_default().to_owned(),
            layout: layout(&v["layout"]),
            size: v["size"].as_u64(),
            content: v["bytes"].as_str().and_then(hex),
            tracks: BTreeMap::new(),
            written: None,
            differs: v["differs"].as_bool() == Some(true),
        })
    }

    /// Takes a `track` or `written` event on the image.
    pub fn take(&mut self, v: &Value) {
        match v["event"].as_str() {
            Some("track") => {
                // gw's own facts in full, or none of them; its bytes none
                // where not reported, as in recordings, which keep none.
                let number = |k: &str| u32::try_from(v[k].as_u64()?).ok();
                let has = v["has"]
                    .as_array()
                    .and_then(|has| has.iter().map(Value::as_bool).collect());
                let bytes = match &v["bytes"] {
                    Value::Null => Some(Vec::new()),
                    b => b.as_str().and_then(hex),
                };
                let laid = v["laid"].as_bool() != Some(false);
                if let (Some(c), Some(h), Some(has), Some(bytes)) =
                    (number("c"), number("h"), has, bytes)
                {
                    self.tracks.insert((c, h), Held { has, bytes, laid });
                }
            }
            Some("written") => {
                // Each track's range, or none where one is not whole numbers.
                let tracks = v["tracks"].as_array().and_then(|ts| {
                    ts.iter()
                        .map(|t| {
                            let n = |i: usize| t.get(i)?.as_u64();
                            let key = (u32::try_from(n(0)?).ok()?, u32::try_from(n(1)?).ok()?);
                            Some((key, n(2)?, n(3)?))
                        })
                        .collect()
                });
                self.written = v["size"].as_u64().map(|size| Written { size, tracks });
            }
            _ => {}
        }
    }

    /// How much of a source's file lies past every track gw lays out in it,
    /// which gw does not read; none where the file ends within them.
    pub fn unread(&self) -> Option<u64> {
        let laid: u64 = self.layout.as_ref()?.tracks.iter().map(track_len).sum();
        let size = self.size.filter(|_| self.role == Role::Source)?;
        size.checked_sub(laid).filter(|&n| n > 0)
    }

    /// The file's size: a source's, a made image's as written, else as gw
    /// would write it from what it holds so far.
    pub fn bytes(&self) -> Option<u64> {
        match self.role {
            Role::Source => self.size,
            Role::Made => self.written.as_ref().map(|w| w.size).or_else(|| {
                let layout = self.layout.as_ref()?;
                let kept = self.kept();
                let tracks = layout
                    .tracks
                    .iter()
                    .filter(|t| kept.is_none_or(|k| t.key.0 <= k));
                Some(tracks.map(track_len).sum())
            }),
        }
    }

    /// The last cylinder gw would put in a made image not yet written, as
    /// it does: those up to the least the file has, and past them, up to
    /// the last in which a track of its layout holds a sector. None where
    /// it puts them all.
    fn kept(&self) -> Option<u32> {
        let layout = self.layout.as_ref()?;
        let least = layout.min_cyls?;
        if self.role != Role::Made || self.written.is_some() {
            return None;
        }
        let holding = self.tracks.iter().filter(|(key, held)| {
            held.has.contains(&true) && layout.tracks.iter().any(|t| t.key == **key)
        });
        Some(
            holding
                .map(|(&(cyl, _), _)| cyl)
                .fold(least.saturating_sub(1), u32::max),
        )
    }

    /// Each track of the file in turn, where it lies, with what each of its
    /// sectors' parts holds, as far as `job` has gone. A made image once
    /// written as gw wrote it; before that, as gw will write it if it holds
    /// every track. None where the file is not laid out, or not as written,
    /// or holds a track whose sectors are not the layout's.
    pub fn placed(&self, job: Option<&Job>) -> Option<Vec<Placed>> {
        let layout = self.layout.as_ref()?;
        if self.tracks.values().any(|held| !held.laid) {
            return None;
        }
        let written = self.written.as_ref().map(|w| w.tracks.as_ref());
        let starts: Vec<(usize, u64)> = match written {
            Some(Some(ranges)) => {
                // Each where gw wrote it, as long as the layout has it.
                let at = |key, start, end: u64| {
                    let i = layout.tracks.iter().position(|t| t.key == key)?;
                    (end.checked_sub(start) == Some(track_len(&layout.tracks[i])))
                        .then_some((i, start))
                };
                ranges
                    .iter()
                    .map(|&(key, start, end)| at(key, start, end))
                    .collect::<Option<_>>()?
            }
            Some(None) => return None,
            None => {
                let mut at = 0;
                let lens = layout.tracks.iter().map(track_len);
                lens.enumerate()
                    .map(|(i, len)| {
                        at += len;
                        (i, at - len)
                    })
                    .collect()
            }
        };
        let kept = self.kept();
        Some(
            starts
                .into_iter()
                .map(|(i, start)| {
                    let laid = &layout.tracks[i];
                    let to_do = job.is_some_and(|j| j.running && j.planned(Role::Made, laid.key));
                    let mut at = start;
                    let parts = laid
                        .sectors
                        .iter()
                        .map(|p| {
                            let state = self.state(laid.key, p, at, to_do);
                            at += p.len;
                            (p.clone(), at - p.len, state)
                        })
                        .collect();
                    Placed {
                        key: laid.key,
                        start,
                        len: track_len(laid),
                        parts,
                        kept: kept.is_none_or(|k| laid.key.0 <= k),
                    }
                })
                .collect(),
        )
    }

    /// What a part holds: a made image's, as gw put its track in, or as far
    /// as the job has gone, `to_do` where gw is yet to put it in; a
    /// source's, as its file holds it, whatever a job does with it: gw's
    /// zeros where it lies past the file's end, gw's filler, or its data.
    fn state(&self, key: (u32, u32), part: &Part, at: u64, to_do: bool) -> State {
        match self.role {
            Role::Made => match self.tracks.get(&key) {
                Some(held) if held.has.get(part.index) == Some(&true) => State::Data,
                Some(_) => State::Filler,
                None if to_do => State::ToDo,
                None => State::Unread,
            },
            Role::Source if self.size.is_some_and(|size| at >= size) => State::PastEnd,
            Role::Source if self.holds_filler(part, at) => State::Filler,
            Role::Source => State::Data,
        }
    }

    /// Whether a source's part holds, as its file has it, the filler gw
    /// puts in place of a sector it lacks.
    fn holds_filler(&self, part: &Part, at: u64) -> bool {
        let filler = self
            .layout
            .as_ref()
            .and_then(|l| l.fillers.get(part.filler));
        let content = self.content.as_ref();
        let held = content.and_then(|c| c.get(at as usize..(at + part.len) as usize));
        matches!((filler, held), (Some(f), Some(h)) if f.as_slice() == h)
    }

    /// A placed part's bytes as gw holds them: a made image's as gw put
    /// its track in the image, or for a track it did not, its filler; a
    /// source's as gw read its file, and its zeros past the file's end.
    /// None where gw has not reported them.
    pub fn part_bytes(&self, track: &Placed, k: usize) -> Option<Vec<u8>> {
        let (part, at, state) = track.parts.get(k)?;
        let len = part.len as usize;
        match self.role {
            Role::Made => match self.tracks.get(&track.key) {
                Some(held) => {
                    let from = (at - track.start) as usize;
                    Some(held.bytes.get(from..from + len)?.to_vec())
                }
                None if *state == State::Unread => {
                    self.layout.as_ref()?.fillers.get(part.filler).cloned()
                }
                None => None,
            },
            Role::Source => {
                let content = self.content.as_ref()?;
                let from = (*at as usize).min(content.len());
                let mut bytes = content[from..(from + len).min(content.len())].to_vec();
                bytes.resize(len, 0);
                Some(bytes)
            }
        }
    }
}

fn track_len(t: &Laid) -> u64 {
    t.sectors.iter().map(|p| p.len).sum()
}

fn layout(v: &Value) -> Option<Layout> {
    let tracks = v["tracks"]
        .as_array()?
        .iter()
        .map(|t| {
            let key = (whole(&t["c"])?, whole(&t["h"])?);
            let sectors = t["sectors"]
                .as_array()?
                .iter()
                .map(|s| {
                    // Four bytes, where gw gives an ID.
                    let id = match &s["id"] {
                        Value::Null => None,
                        id => Some(
                            id.as_array()?
                                .iter()
                                .map(|b| u8::try_from(b.as_u64()?).ok())
                                .collect::<Option<Vec<u8>>>()?
                                .try_into()
                                .ok()?,
                        ),
                    };
                    Some(Part {
                        index: whole(&s["i"])? as usize,
                        id,
                        len: s["len"].as_u64()?,
                        filler: whole(&s["fill"])? as usize,
                    })
                })
                .collect::<Option<_>>()?;
            Some(Laid { key, sectors })
        })
        .collect::<Option<_>>()?;
    let fillers = v["fillers"]
        .as_array()?
        .iter()
        .map(|f| hex(f.as_str()?))
        .collect::<Option<_>>()?;
    let min_cyls = match &v["min_cyls"] {
        Value::Null => None,
        m => Some(whole(m)?),
    };
    Some(Layout {
        tracks,
        fillers,
        min_cyls,
    })
}

/// A whole number gw gives, as a u32.
fn whole(v: &Value) -> Option<u32> {
    u32::try_from(v.as_u64()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two tracks of two 512-byte sectors, cylinder 0's and 1's, as gw
    /// reports an ADF it makes.
    fn made() -> Image {
        let open = serde_json::json!({
            "event": "open", "role": "made", "file": "Disk.adf", "type": "ADF",
            "layout": {"tracks": [
                {"c": 0, "h": 0, "sectors": [{"i": 0, "id": null, "len": 512, "fill": 0},
                                              {"i": 1, "id": null, "len": 512, "fill": 0}]},
                {"c": 1, "h": 0, "sectors": [{"i": 0, "id": null, "len": 512, "fill": 0},
                                              {"i": 1, "id": null, "len": 512, "fill": 0}]}],
                "fillers": [to_hex(&filler())], "min_cyls": null}
        });
        Image::parse(&open).unwrap()
    }

    /// gw's filler for a 512-byte sector it lacks.
    fn filler() -> Vec<u8> {
        b"-=[BAD SECTOR]=-".repeat(32)
    }

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A read of cylinders 0 and 1's side 0, which gw has said nothing of.
    fn announced() -> Progress {
        let mut progress = Progress::default();
        progress.feed("Reading c=0-1:h=0 revs=2");
        progress
    }

    fn job(progress: &Progress, running: bool) -> Job<'_> {
        Job { progress, running }
    }

    #[test]
    fn a_made_image_is_laid_out_from_its_first_byte_and_holds_what_gw_put_in_it() {
        let mut image = made();
        assert_eq!(image.bytes(), Some(2048), "as gw will write it");
        assert_eq!(image.layout.as_ref().unwrap().fillers[0], filler());
        let track = serde_json::json!({"event": "track", "c": 0, "h": 0, "has": [true, false], "bytes": ""});
        image.take(&track);
        let progress = announced();
        let placed = image.placed(Some(&job(&progress, true))).unwrap();
        let states: Vec<(u64, State)> = placed
            .iter()
            .flat_map(|t| t.parts.iter().map(|p| (p.1, p.2)))
            .collect();
        assert_eq!(
            states,
            [
                (0, State::Data),
                (512, State::Filler),
                (1024, State::ToDo),
                (1536, State::ToDo)
            ]
        );
        // Done with cylinder 1 not read: gw's filler.
        let placed = image.placed(Some(&job(&progress, false))).unwrap();
        assert_eq!(placed[1].parts[0].2, State::Unread);
    }

    #[test]
    fn a_written_image_lies_as_gw_wrote_it_and_one_not_as_laid_out_is_not_placed() {
        let mut image = made();
        let written =
            serde_json::json!({"event": "written", "size": 1024, "tracks": [[0, 0, 0, 1024]]});
        image.take(&written);
        let progress = announced();
        let placed = image.placed(Some(&job(&progress, false))).unwrap();
        assert_eq!(placed.len(), 1, "cylinder 1 left out");
        assert_eq!(image.bytes(), Some(1024));
        let odd = serde_json::json!({"event": "written", "size": 1024, "tracks": null});
        image.take(&odd);
        assert_eq!(image.placed(Some(&job(&progress, false))), None);
    }

    #[test]
    fn a_sources_parts_past_its_files_end_are_gws_zeros() {
        let mut image = made();
        image.role = Role::Source;
        image.size = Some(1536);
        let placed = image.placed(None).unwrap();
        let states: Vec<State> = placed
            .iter()
            .flat_map(|t| t.parts.iter().map(|p| p.2))
            .collect();
        use State::*;
        assert_eq!(states, [Data, Data, Data, PastEnd]);
    }

    #[test]
    fn a_parts_bytes_are_those_gw_holds_for_it() {
        let mut image = made();
        let held: String = (0..1024).map(|i| format!("{:02x}", i % 256)).collect();
        let track = serde_json::json!({
            "event": "track", "c": 0, "h": 0, "has": [true, false], "bytes": held
        });
        image.take(&track);
        let progress = announced();
        let placed = image.placed(Some(&job(&progress, false))).unwrap();
        // As gw put the track in the image, its filler too.
        let second: Vec<u8> = (512..1024).map(|i| (i % 256) as u8).collect();
        assert_eq!(placed[0].parts[1].2, State::Filler);
        assert_eq!(image.part_bytes(&placed[0], 1), Some(second));
        // A track gw did not read: the filler it writes for it.
        assert_eq!(image.part_bytes(&placed[1], 0), Some(filler()));
        // Nothing yet of a track it is to read.
        let placed = image.placed(Some(&job(&progress, true))).unwrap();
        assert_eq!(image.part_bytes(&placed[1], 0), None);
        // A source's as gw read its file, with its zeros past the end.
        let mut source = made();
        source.role = Role::Source;
        source.size = Some(1436);
        source.content = Some(vec![7; 1436]);
        let placed = source.placed(Some(&job(&progress, false))).unwrap();
        let mut want = vec![7; 412];
        want.resize(512, 0);
        assert_eq!(source.part_bytes(&placed[1], 0), Some(want));
        assert_eq!(source.part_bytes(&placed[1], 1), Some(vec![0; 512]));
    }

    #[test]
    fn a_conversions_routes_say_where_its_tracks_lie_in_each_image() {
        // gw's T0.0 taken from the input's cylinder 1, put in the output's 0.
        let mut progress = Progress::default();
        progress.feed("Converting c=0:h=0:step=2 -> c=0:h=0");
        progress.image(r#"{"event":"routes","tracks":[[0,0,1,0,0,0]]}"#);
        progress
            .feed("T0.0 <- Image 1.0: AmigaDOS (2/2 sectors) from Raw Flux (9 flux in 200.00ms)");
        let running = Job {
            progress: &progress,
            running: true,
        };
        assert_eq!(running.current(Role::Source), Some((1, 0)));
        assert_eq!(running.current(Role::Made), Some((0, 0)));
        assert_eq!(running.named(Role::Source, (1, 0)), [(0, 0)]);
        assert!(running.named(Role::Made, (0, 0)).is_empty(), "as its own");
        // gw is to put the output's cylinder 0 in it, not its cylinder 1.
        let image = made();
        let placed = image.placed(Some(&running)).unwrap();
        assert_eq!(placed[0].parts[0].2, State::ToDo);
        assert_eq!(placed[1].parts[0].2, State::Unread);
        let done = Job {
            running: false,
            ..running
        };
        assert_eq!(done.current(Role::Source), None, "only while gw works");
    }

    #[test]
    fn a_sources_parts_hold_its_data_or_gws_filler_as_its_file_has_them() {
        let mut source = made();
        source.role = Role::Source;
        let filler = filler();
        let mut content = vec![7; 1800];
        content[512..1024].copy_from_slice(&filler);
        source.size = Some(1800);
        source.content = Some(content);
        let states = |job: Option<&Job>| -> Vec<State> {
            let placed = source.placed(job).unwrap();
            placed
                .iter()
                .flat_map(|t| t.parts.iter().map(|p| p.2))
                .collect()
        };
        use State::*;
        // The last part starts in the file, 1,800 bytes long: its data, then gw's zeros.
        assert_eq!(states(None), [Data, Filler, Data, Data], "before a job");
        let mut progress = Progress::default();
        progress.feed("Writing c=0-1:h=0");
        progress.feed("T0.0: Writing Track (Flux: 200.0ms period, 200.2 ms total, Write all)");
        progress.feed("All tracks verified");
        for running in [true, false] {
            assert_eq!(
                states(Some(&job(&progress, running))),
                [Data, Filler, Data, Data],
                "as the file holds them, whatever the job does"
            );
        }
    }

    #[test]
    fn a_track_moved_before_the_first_cylinder_lies_in_that_image_alone_not() {
        // T0.0 read from the input's cylinder -1, none; T1.0 put at the output's -1.
        let v = serde_json::json!({"tracks": [[0, 0, -1, 0, 0, 0], [1, 0, 1, 0, -1, 0]]});
        let moved = routes(&v).unwrap();
        assert_eq!(
            moved,
            [
                Route {
                    own: (0, 0),
                    from: None,
                    to: Some((0, 0))
                },
                Route {
                    own: (1, 0),
                    from: Some((1, 0)),
                    to: None
                },
            ]
        );
        let mut progress = announced();
        progress.routes = Some(moved);
        progress.feed("T1.0: IBM MFM (18/18 sectors) from Raw Flux (1 flux in 200.00ms)");
        let running = job(&progress, true);
        assert_eq!(
            running.current(Role::Source),
            Some((1, 0)),
            "gw reads it from the input"
        );
        assert_eq!(
            running.current(Role::Made),
            None,
            "and puts it in no output track"
        );
        let v = serde_json::json!({"tracks": [[0, 0, 0.5, 0, 0, 0]]});
        assert_eq!(routes(&v), None, "not whole numbers");
    }

    #[test]
    fn a_track_gw_passes_through_from_its_input_leaves_the_image_unplaced() {
        let mut image = made();
        let track = |laid: bool| {
            serde_json::json!({"event": "track", "c": 0, "h": 0, "has": [true, true],
                "bytes": "", "laid": laid})
        };
        image.take(&track(true));
        assert!(image.placed(None).is_some());
        image.take(&track(false));
        assert_eq!(image.placed(None), None, "its sectors are not the layout's");
    }

    #[test]
    fn a_file_written_otherwise_than_laid_out_is_not_placed() {
        let mut image = made();
        let short = serde_json::json!({"event": "written", "size": 2047,
            "tracks": [[0, 0, 0, 1024], [1, 0, 1024, 2047]]});
        image.take(&short);
        assert_eq!(image.placed(None), None, "a track a byte short");
        let odd = serde_json::json!({"event": "written", "size": 2048,
            "tracks": [[0, 0, 0, 1024], [1, 0, "1024", 2048]]});
        image.take(&odd);
        assert_eq!(image.placed(None), None);
        assert_eq!(image.bytes(), Some(2048), "as written");
    }

    #[test]
    fn a_made_image_holds_cylinders_past_its_least_up_to_the_last_with_a_sector() {
        let mut image = made();
        image.layout.as_mut().unwrap().min_cyls = Some(1);
        let kept = |image: &Image| -> Vec<bool> {
            image.placed(None).unwrap().iter().map(|t| t.kept).collect()
        };
        assert_eq!(
            (kept(&image), image.bytes()),
            (vec![true, false], Some(1024))
        );
        let held = |c, has: [bool; 2]| serde_json::json!({"event": "track", "c": c, "h": 0, "has": has, "bytes": ""});
        image.take(&held(1, [false, false]));
        assert_eq!(
            kept(&image),
            [true, false],
            "a track of no sector holds no data"
        );
        image.take(&held(1, [false, true]));
        assert_eq!(
            (kept(&image), image.bytes()),
            (vec![true, true], Some(2048))
        );
    }

    #[test]
    fn a_source_longer_than_its_layout_has_bytes_gw_does_not_read() {
        let mut source = made();
        source.role = Role::Source;
        source.size = Some(2048 + 300);
        assert_eq!(source.unread(), Some(300));
        source.size = Some(2048);
        assert_eq!(source.unread(), None);
        source.size = Some(1500);
        assert_eq!(source.unread(), None, "it ends within them");
        assert_eq!(made().unread(), None, "a made image's is gw's own");
    }
}

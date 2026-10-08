//! What the window tests share: the harness, gw's output and helpers.
#![allow(dead_code)] // each test crate uses some of it

use eframe::egui::{self, accesskit::Role};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, Node};
use ferriteweazle::App;
use ferriteweazle::schema::Port;

pub type Window = Harness<'static, Option<App>>;

pub const DAMAGED: &str = include_str!("../data/convert-damaged.log");

/// The Workbench disk's flux with a scratch cut into side 0, cylinders 18 to
/// 62, converted to an ADF: real flux, gw's decode of it, and gw's filler in
/// place of the 55 sectors that did not decode.
pub const SCRATCHED: &str = include_str!("../data/convert-workbench-scratched.log");
/// A real disk read: the Workbench 3.1 Install disk in a real drive, read
/// with gw's own revolutions for its format: the bridge's reports, without
/// the disk's bytes.
pub const WORKBENCH: &str = include_str!("../data/read-workbench.log");
/// The Workbench disk written back from its ADF in a real drive: each track
/// as gw writes it, then as gw's verify read it back.
pub const WRITTEN: &str = include_str!("../data/write-workbench.log");
/// Detect of a flux image gw made of an AmigaDOS disk: the three tracks it
/// read, each reported as read, then as AmigaDOS decodes it.
pub const DETECTED: &str = include_str!("../data/detect-amiga.log");
/// A track of a real Akai S950 disk's HFE image, as the bridge reports it,
/// its sectors' data made up.
pub const AKAI_TRACK: &str = include_str!("../data/report-akai.txt");
/// tests/data/edsk.py's image converted as ibm.scan: one track, cylinder 0
/// head 0, of each kind of sector, as gw lays an EDSK's track out: bitcells,
/// not flux a drive read.
pub const KINDS: &str = include_str!("../data/convert-kinds.log");
/// That track as flux gw made of it, two revolutions, the second spoilt in
/// R1's data (gw.rs's DAMAGE), converted as ibm.scan.
pub const SPOILT: &str = include_str!("../data/convert-kinds-spoilt.log");
/// Track 0's centreline on a 3½-inch disk's side 0, as a share of the way
/// from its centre to its edge, as ECMA-125 has it: 39.5 mm of 42.9 mm.
pub const TRACK_0: f32 = 39.5 / 42.9;

/// DETECTED as Detect read its tracks, before it found the format: each
/// track's line and its flux, up to the first it reports again, decoded.
pub fn detect_reads() -> String {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = String::new();
    for line in DETECTED.lines() {
        if let Some(json) = line.strip_prefix("@ferriteweazle track ") {
            let v: serde_json::Value = serde_json::from_str(json).expect("a report");
            if !seen.insert((v["c"].as_u64(), v["h"].as_u64())) {
                break;
            }
        }
        out += line;
        out.push('\n');
    }
    out
}

/// AKAI_TRACK's report, as the bridge prints it after its prefix.
pub fn akai_report() -> &'static str {
    AKAI_TRACK
        .trim()
        .strip_prefix("@ferriteweazle track ")
        .expect("a track report")
}

/// A `command` job replaying `log` as it reaches track 41.0, gw still at it.
pub fn reaching_41(command: &str, log: &str) -> ferriteweazle::job::Job {
    let reached = log
        .split_inclusive('\n')
        .take_while(|l| !l.starts_with("T41.1"))
        .collect::<String>();
    let mut job = ferriteweazle::job::Job::replay(command, &reached);
    job.ended = None;
    job.progress.current = Some((41, 0));
    job
}

/// Gives track `key` of the image `job` makes made-up bytes, as the bridge
/// reports the bytes gw holds: gw's filler where gw lacks a sector, else
/// byte i of the track i % 251. The recordings keep no disk's data.
pub fn held(job: &mut ferriteweazle::job::Job, key: (u32, u32)) {
    let made = job.progress.made.as_ref().expect("the image's report");
    let layout = made.layout.as_ref().expect("laid out");
    let laid = layout.tracks.iter().find(|t| t.key == key).unwrap();
    let has = made.tracks[&key].has.clone();
    let bytes = made_up(laid, &has, &layout.fillers, 0);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let has = serde_json::to_string(&has).unwrap();
    let (c, h) = key;
    let line = format!(r#"{{"event":"track","c":{c},"h":{h},"has":{has},"bytes":"{hex}"}}"#);
    job.progress.image(&line);
}

/// Track `laid`'s bytes as the recordings stand in for them: gw's filler
/// where `has` says gw lacks the sector, else byte i, counted from `from`,
/// i % 251.
fn made_up(
    laid: &ferriteweazle::image::Laid,
    has: &[bool],
    fillers: &[Vec<u8>],
    from: usize,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    for part in &laid.sectors {
        let at = from + bytes.len();
        match has[part.index] {
            true => bytes.extend((at..at + part.len as usize).map(|i| (i % 251) as u8)),
            false => bytes.extend(&fillers[part.filler]),
        }
    }
    bytes
}

/// Where in the image map sector `sector` of track `track` lies, the file in
/// two columns of `rows` rows of `parts` sectors each: the second column's
/// rows after its offsets, five hex digits in 11-point type, and the gaps
/// either side of them.
pub fn image_part(
    w: &Window,
    rows: usize,
    parts: usize,
) -> impl Fn(usize, usize) -> egui::Pos2 + use<> {
    let map = w.get_by_label("Image map").rect();
    let font = egui::FontId::monospace(11.0);
    let glyph = w.ctx.fonts_mut(|f| f.glyph_width(&font, '0'));
    let between = 12.0 + 5.0 * glyph + 8.0;
    let column = (map.width() - between) / 2.0;
    let row = map.height() / rows as f32;
    assert!((4.0..=12.0).contains(&row), "{row}");
    move |track, sector| {
        let left = map.left() + (track / rows) as f32 * (column + between);
        let x = left + column * (sector as f32 + 0.5) / parts as f32;
        egui::pos2(x, map.top() + ((track % rows) as f32 + 0.5) * row)
    }
}

/// SCRATCHED's ADF as a write takes it: gw's layout and filler, and byte i
/// of the file i % 251 for data the recording does not keep.
pub fn scratched_adf() -> ferriteweazle::image::Image {
    use ferriteweazle::image::Role;
    let job = ferriteweazle::job::Job::replay("convert", SCRATCHED);
    let mut image = job.progress.made.clone().expect("the image's report");
    let layout = image.layout.clone().expect("laid out");
    let mut content = Vec::new();
    for laid in &layout.tracks {
        let has = &image.tracks[&laid.key].has;
        content.extend(made_up(laid, has, &layout.fillers, content.len()));
    }
    image.role = Role::Source;
    image.size = Some(content.len() as u64);
    image.content = Some(content);
    image.tracks.clear();
    image.written = None;
    image
}

/// DAMAGED as a read would print it: a read loads no .scp to warn about.
pub fn damaged_read() -> String {
    DAMAGED.replace("SCP: WARNING: Bad image checksum\n", "")
}

pub const FOUND: &str =
    "Found akai.800. Disk also matches eagle.dsqd.800, epson.qx10.400 and zx.quorum.ds80.";

/// What gw prints when Linux refuses it the port: pyserial's EACCES error.
pub const REFUSED: &str = "** FATAL ERROR:
[Errno 13] could not open port /dev/ttyACM0: [Errno 13] Permission denied: '/dev/ttyACM0'";

/// The window's size as it opens.
pub const DEFAULT: egui::Vec2 = ferriteweazle::WINDOW;

/// Shows `app` over the whole harness: kittest insets its ui by 8 points, so
/// a harness of the window's size lays the app out as the window does.
pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let rect = ui.ctx().content_rect();
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect);
        app.show(ui);
    });
}

/// A Greaseweazle as gw lists it, on a made-up port.
pub fn greaseweazle() -> Port {
    Port {
        device: "/dev/cu.usbmodem14201".into(),
        name: Some("Greaseweazle".into()),
        score: 20,
        denied: false,
    }
}

pub fn app(w: &Window) -> &App {
    w.state().as_ref().expect("the first frame made the app")
}

pub fn app_mut(w: &mut Window) -> &mut App {
    w.state_mut()
        .as_mut()
        .expect("the first frame made the app")
}

/// The page's run button, not the sidebar entry of the same name.
pub fn run_button<'w>(w: &'w Window, name: &'w str) -> Node<'w> {
    w.get_all_by_role_and_label(Role::Button, name)
        .find(|n| n.rect().left() > 240.0)
        .expect("the run button")
}

/// What the command line holds.
pub fn line(w: &Window) -> String {
    w.get_by_role(Role::MultilineTextInput)
        .value()
        .unwrap_or_default()
}

/// The disk map's squares, larger than the legend's 10-point swatches.
pub fn squares(w: &Window) -> impl Iterator<Item = &egui::epaint::RectShape> {
    let left = w.get_by_label("Disk status").rect().left();
    w.output()
        .shapes
        .iter()
        .filter_map(move |c| match &c.shape {
            egui::Shape::Rect(r)
                if r.rect.left() > left
                    && (r.rect.width() - r.rect.height()).abs() < 0.5
                    && r.rect.width() > 10.5 =>
            {
                Some(r)
            }
            _ => None,
        })
}

/// A point on head 0's disk, the first of the two side by side in the disk
/// map, each under its side's name, 20 points tall, 32 points apart:
/// `share` of the way from its centre to its edge, at `degrees` from the
/// right, anticlockwise.
pub fn on_disk(w: &Window, share: f32, degrees: f32) -> egui::Pos2 {
    let map = w.get_by_label("Disk map").rect();
    let diameter = map.height() - 20.0;
    let left = map.center().x - (2.0 * diameter + 32.0) / 2.0;
    let radius = diameter / 2.0;
    let centre = egui::pos2(left + radius, map.top() + 20.0 + radius);
    let a = degrees.to_radians();
    centre + share * radius * egui::vec2(a.cos(), -a.sin())
}

/// A read still running whose disks show every entry their legend has, at
/// once, on made-up tracks: on 118 tracks
/// nine sectors of each kind, sectors that meet, an ID repeated, nine
/// headers and nine data blocks found alone and ten sectors missing; a
/// track gw read whole; one where it found none; one read as flux alone;
/// one it named and did not report; a retry; and 38 tracks to do. Its
/// sectors' data counts up, so no disk's data is in it.
pub fn every_entry() -> ferriteweazle::job::Job {
    use serde_json::{Value, json};
    let mut job = ferriteweazle::job::Job::replay("read", "");
    job.ended = None;
    let p = &mut job.progress;
    p.feed("Reading c=0-79:h=0-1 revs=2");
    let flux = json!({"freq": 72e6, "period": 14.4e6, "revs": [14.4e6], "passes": [[0.0, 1.0]],
        "bins": [100, 100, 100, 100],
        "intervals": {"width": 4, "first": 70, "top": 360, "counts": [5, 10, 5], "longer": 0}});
    let counting = |from: u8| {
        (0..128)
            .map(|i: u8| format!("{:02x}", from.wrapping_add(i)))
            .collect::<String>()
    };
    // 128-byte sectors, end to end, 1400 bit cells each.
    let sector =
        |c: u32, h: u32, r: u8, i: u32, header: bool, data: bool, mark: u8, bytes: String| {
            let start = 500 + i * 1400;
            json!({"id": [c, h, r, 0], "start": start, "header_end": start + 160,
            "data_start": start + 400, "end": start + 1400, "header": header, "data": data,
            "mark": mark, "bytes": bytes})
        };
    let laid = |c: u32, h: u32, found: &[u8], all: &[u8]| -> Value {
        all.iter()
            .map(|&r| sector(c, h, r, 0, found.contains(&r), false, 251, String::new()))
            .collect()
    };
    let report =
        |p: &mut ferriteweazle::progress::Progress, c: u32, h: u32, line: &str, codec: Value| {
            p.feed(&format!(
                "T{c}.{h}: {line} from Raw Flux (100000 flux in 400.00ms)"
            ));
            let r = json!({"c": c, "h": h, "turned": true, "flux": flux.clone(), "codec": codec});
            p.report(&r.to_string());
        };
    for c in 0..61u32 {
        for h in 0..2u32 {
            match (c, h) {
                // Read whole.
                (0, 0) => {
                    let found: Vec<Value> = (0..3)
                        .map(|i| sector(c, h, i as u8 + 1, i, true, true, 251, counting(i as u8)))
                        .collect();
                    let codec = json!({"summary": "IBM MFM (3/3 sectors)", "nsec": 3, "good": [0, 1, 2],
                        "time_per_rev": 0.2, "clock": 2e-6, "found": found,
                        "laid": laid(c, h, &[1, 2, 3], &[1, 2, 3])});
                    report(p, c, h, "IBM MFM (3/3 sectors)", codec);
                }
                // None found.
                (0, 1) => {
                    let codec = json!({"summary": "IBM MFM (0/7 sectors)", "nsec": 7, "good": [],
                        "time_per_rev": 0.2, "clock": 2e-6, "found": [],
                        "laid": laid(c, h, &[], &[1, 2, 3, 4, 5, 6, 7])});
                    report(p, c, h, "IBM MFM (0/7 sectors)", codec);
                }
                // Flux alone.
                (1, 0) => {
                    p.feed("T1.0: Raw Flux (100000 flux in 400.00ms)");
                    let r = json!({"c": c, "h": h, "turned": true, "flux": flux.clone()});
                    p.report(&r.to_string());
                }
                // Named, not reported.
                (1, 1) => {
                    p.feed("T1.1: IBM MFM (9/9 sectors) from Raw Flux (100000 flux in 400.00ms)")
                }
                _ => {
                    // Nine of each kind: good, empty, deleted, its data bad, its header bad.
                    let kinds = [
                        (true, true, 251),
                        (true, true, 251),
                        (true, true, 248),
                        (true, false, 251),
                        (false, false, 251),
                    ];
                    let mut found = Vec::new();
                    for (k, &(header, data, mark)) in kinds.iter().enumerate() {
                        for j in 0..9u32 {
                            let (i, r) = (k as u32 * 9 + j, (k * 9) as u8 + j as u8 + 1);
                            let bytes = match k {
                                1 => "e5".repeat(128),
                                _ => counting(r),
                            };
                            found.push(sector(c, h, r, i, header, data, mark, bytes));
                        }
                    }
                    // R1 again, its header's CRC holding.
                    found.push(sector(c, h, 1, 45, true, true, 251, counting(99)));
                    let mut apart = Vec::new();
                    for j in 0..9u32 {
                        let at = 500 + (46 + 2 * j) * 1400;
                        apart.push(json!({"id": [c, h, 50 + j, 0], "header": true, "start": at, "end": at + 160}));
                        apart.push(json!({"id": null, "header": null, "start": at + 1400, "end": at + 1464, "mark": 251}));
                    }
                    let all: Vec<u8> = (1..=45).chain([99]).collect();
                    let codec = json!({"summary": "IBM MFM (36/46 sectors)", "nsec": 46,
                        "good": (0..36).collect::<Vec<_>>(), "time_per_rev": 0.2, "clock": 2e-6,
                        "found": found, "apart": apart,
                        "laid": laid(c, h, &(1..=36).collect::<Vec<u8>>(), &all)});
                    report(p, c, h, "IBM MFM (36/46 sectors)", codec);
                }
            }
        }
    }
    p.feed("T60.1: IBM MFM (36/46 sectors) from Raw Flux (100000 flux in 400.00ms) (Retry #1.1)");
    // The track gw is on: its line alone.
    p.feed("T61.0: IBM MFM (36/46 sectors) from Raw Flux (100000 flux in 400.00ms)");
    job
}

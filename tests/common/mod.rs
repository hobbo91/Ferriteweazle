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
/// with gw's own revolutions for its format, the bridge's reports and all.
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

/// Gives track `key` of the image `job` makes made-up bytes, as the bridge
/// reports the bytes gw holds: gw's filler where gw lacks a sector, else
/// byte i of the track i % 251. The recordings keep no disk's data.
pub fn held(job: &mut ferriteweazle::job::Job, key: (u32, u32)) {
    let made = job.progress.made.as_ref().expect("the image's report");
    let layout = made.layout.as_ref().expect("laid out");
    let laid = layout.tracks.iter().find(|t| t.key == key).unwrap();
    let has = made.tracks[&key].has.clone();
    let mut bytes = Vec::new();
    for part in &laid.sectors {
        let at = bytes.len();
        match has[part.index] {
            true => bytes.extend((at..at + part.len as usize).map(|i| (i % 251) as u8)),
            false => bytes.extend(&layout.fillers[part.filler]),
        }
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let has = serde_json::to_string(&has).unwrap();
    let (c, h) = key;
    let line = format!(r#"{{"event":"track","c":{c},"h":{h},"has":{has},"bytes":"{hex}"}}"#);
    job.progress.image(&line);
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

/// The ADF that conversion made, as a write would take its tracks from it:
/// laid out as gw laid it out, gw's filler where gw put it, and in place of
/// the disk's data, which the recordings do not keep, byte i of the file
/// i % 251.
pub fn scratched_adf() -> ferriteweazle::image::Image {
    use ferriteweazle::image::Role;
    let job = ferriteweazle::job::Job::replay("convert", SCRATCHED);
    let mut image = job.progress.made.clone().expect("the image's report");
    let layout = image.layout.clone().expect("laid out");
    let mut content = Vec::new();
    for laid in &layout.tracks {
        let has = &image.tracks[&laid.key].has;
        for part in &laid.sectors {
            let at = content.len();
            match has[part.index] {
                true => content.extend((at..at + part.len as usize).map(|i| (i % 251) as u8)),
                false => content.extend(&layout.fillers[part.filler]),
            }
        }
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

//! The Analyse drawer's view of the disk: each side as a round disk, a ring
//! per track, and on it each sector gw found, where it found it, round from
//! the index at the top, clockwise, each side as seen from side 0. Nothing is
//! drawn that gw did not report.

use crate::diskmap;
use crate::form;
use crate::lines::{Lines, Pane};
use crate::progress::{self, Progress, Status};
use crate::theme::{self, Palette};
use crate::track::{
    Before, Data, Facts, Header, Id, Intervals, Layout, Sector, Seen, Source, Spin, Turns,
};
use eframe::egui::{
    self, Align2, Color32, FontId, Galley, Pos2, Rect, RichText, Sense, Shape, Stroke,
    emath::GuiRounding, plugin::TypedPluginHandle, vec2,
};
use std::f64::consts::TAU;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

/// How the disk is drawn: its tracks fitted to the room, or a disk to scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Media {
    #[default]
    Fit,
    ThreeHalf,
    FiveQuarter96,
    FiveQuarter48,
    Eight,
}

/// Each way: its name, and the word analyse.txt keeps.
pub const MEDIA: [(Media, &str, &str); 5] = [
    (Media::Fit, "Fit", "fit"),
    (Media::ThreeHalf, "3½-inch, 135 TPI", "3.5"),
    (Media::FiveQuarter96, "5¼-inch, 96 TPI", "5.25-96"),
    (Media::FiveQuarter48, "5¼-inch, 48 TPI", "5.25-48"),
    (Media::Eight, "8-inch, 48 TPI", "8"),
];

/// What the tracks show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Shows {
    /// The sectors gw found.
    #[default]
    Sectors,
    /// The flux round each track.
    Flux,
}

/// Each: its name, and the word analyse.txt keeps.
pub const SHOWS: [(Shows, &str, &str); 2] = [
    (Shows::Sectors, "Sectors", "sectors"),
    (Shows::Flux, "Flux", "flux"),
];

impl Shows {
    /// What the tracks show with this chosen: their flux only where a track
    /// drawn has some, `fluxed`; else their sectors.
    fn given(self, fluxed: bool) -> Shows {
        match self {
            Shows::Flux if fluxed => Shows::Flux,
            _ => Shows::Sectors,
        }
    }
}

/// What the drawer analyses: the disk, or the image the job makes or takes
/// its tracks from; of a conversion, which has no disk, its input or its
/// output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Analysis {
    #[default]
    Disk,
    Image,
}

/// Each: its name, and the word analyse.txt keeps.
pub const ANALYSES: [(Analysis, &str, &str); 2] = [
    (Analysis::Disk, "Disk analysis", "disk"),
    (Analysis::Image, "Image analysis", "image"),
];

/// A disk's measurements in millimetres, as its ECMA standard gives them.
struct Size {
    /// The disk's radius, and its metal hub's or its centre hole's.
    radius: f64,
    hub: Option<f64>,
    hole: f64,
    /// Track 0's centreline on side 0 and on side 1, from one track's
    /// centreline to the next, and the width recorded: between the tracks,
    /// the disk is erased.
    track_0: [f64; 2],
    pitch: f64,
    width: f64,
    /// The recording area's inner edge.
    innermost: f64,
}

/// 25.4 mm: tracks per inch give the pitch.
const INCH: f64 = 25.4;

impl Media {
    fn size(self) -> Option<Size> {
        match self {
            Media::Fit => None,
            // ECMA-125: 85.8 mm across (6.3.1), its hub's flange 31.15 mm
            // across at most (6.4.1), the recording area in to 20.6 mm (8.1),
            // track n at 39.5 or 38.0 - 0.1875 n mm, 0.115 mm wide (8.2).
            Media::ThreeHalf => Some(Size {
                radius: 85.8 / 2.0,
                hub: Some(31.15 / 2.0),
                hole: 0.0,
                track_0: [39.5, 38.0],
                pitch: 0.1875,
                width: 0.115,
                innermost: 20.6,
            }),
            // ECMA-78: 130.2 mm across (3.3.1), the recording area in to
            // 31.3 mm (3.3.4), track n at 57.150 or 55.033 mm less n/96 inch,
            // 0.155 mm wide (5.1). Its hole 28.575 mm, as ECMA-99 gives the
            // same disk's (3.3.1), where ECMA-78 gives 28.57.
            Media::FiveQuarter96 => Some(Size {
                radius: 130.2 / 2.0,
                hub: None,
                hole: 28.575 / 2.0,
                track_0: [57.150, 55.033],
                pitch: INCH / 96.0,
                width: 0.155,
                innermost: 31.3,
            }),
            // ECMA-70: the disk as ECMA-78's, track n at 57.150 or 55.033 mm
            // less n/48 inch, 0.300 mm wide (5.1).
            Media::FiveQuarter48 => Some(Size {
                radius: 130.2 / 2.0,
                hub: None,
                hole: 28.575 / 2.0,
                track_0: [57.150, 55.033],
                pitch: INCH / 48.0,
                width: 0.300,
                innermost: 31.3,
            }),
            // ECMA-69: 200.2 mm across, its hole 38.1 mm (3.3.1), the
            // recording area in to 49.0 mm (3.3.4), track n at 51.537 or
            // 49.421 mm and (76 - n)/48 inch, 0.300 mm wide (5.1).
            Media::Eight => Some(Size {
                radius: 200.2 / 2.0,
                hub: None,
                hole: 38.1 / 2.0,
                track_0: [51.537, 49.421].map(|x| x + 76.0 / 48.0 * INCH),
                pitch: INCH / 48.0,
                width: 0.300,
                innermost: 49.0,
            }),
        }
    }

    pub fn name(self) -> &'static str {
        MEDIA.iter().find(|m| m.0 == self).map_or("", |m| m.1)
    }

    /// How many tracks the disk holds, from track 0 in to its recording
    /// area's inner edge on the side with less room.
    pub fn holds(self) -> Option<u32> {
        let s = self.size()?;
        let room = s.track_0[1] - s.width / 2.0 - s.innermost;
        Some((room / s.pitch).floor() as u32 + 1)
    }
}

/// What the disk view draws: a job's progress, and how.
pub struct Map<'a> {
    pub progress: &'a Progress,
    /// The job takes its tracks from an image, as a conversion does, or
    /// Detect of a file, not from the drive.
    pub image: bool,
    /// The disk's cylinders and sides, if known.
    pub disk: (u32, u32),
    /// The heads swap: side 1 is read by head 0.
    pub swapped: bool,
    pub verifying: bool,
    pub media: Media,
    pub shows: Shows,
    /// The last track gw reported, while it works.
    pub current: Option<(u32, u32)>,
    /// gw works on the disk: its legend's counts change as it goes.
    pub running: bool,
}

/// The cylinders the disk view spans for `progress` on a disk of `disk`'s.
pub fn span(progress: &Progress, disk: (u32, u32)) -> u32 {
    let cyls = progress
        .cyls
        .iter()
        .chain(progress.tracks.keys().map(|(c, _)| c));
    disk.0.max(cyls.max().map_or(0, |c| c + 1))
}

/// The sides it shows, one or two.
fn sides(progress: &Progress, disk: (u32, u32)) -> u32 {
    let heads = progress
        .heads
        .iter()
        .chain(progress.tracks.keys().map(|(_, h)| h));
    disk.1.max(heads.max().map_or(0, |h| h + 1)).clamp(1, 2)
}

/// Whether any track the disk view spans, `span` cylinders on `sides`
/// sides, has flux reported.
fn fluxed(progress: &Progress, span: u32, sides: u32) -> bool {
    progress
        .facts
        .iter()
        .any(|(&(c, h), f)| c < span && h < sides && !f.absent && f.flux.is_some())
}

/// The choice of what the tracks show: Flux greyed with why where no track
/// the disk view spans has flux, and their sectors shown.
pub fn choose_shows(ui: &mut egui::Ui, shows: &mut Shows, progress: &Progress, disk: (u32, u32)) {
    let fluxed = fluxed(progress, span(progress, disk), sides(progress, disk));
    let showing = shows.given(fluxed);
    for (view, name, _) in SHOWS.into_iter().rev() {
        let why = (view == Shows::Flux && !fluxed).then_some("No flux reported.");
        let button = egui::Button::new(name);
        let chosen = ui
            .add_enabled_ui(why.is_none(), |ui| {
                form::selectable(ui, showing == view, button)
            })
            .inner
            .on_disabled_hover_text(why.unwrap_or_default());
        if chosen.clicked() {
            *shows = view;
        }
    }
}

/// Room for a side's name above its disk.
pub(crate) const TITLE: f32 = 20.0;
/// The sector window's title bar, as tall as a macOS window's, its title in
/// the same 13-point type, as each side's name.
const TITLE_BAR: f32 = 28.0;
const TITLE_SIZE: f32 = 13.0;
/// The sector window's bytes, and the first rows of them a sector's tip
/// shows, in 12-point monospace.
const DUMP_SIZE: f32 = 12.0;
/// The sector window's way round the disk: the ring, its track's width and
/// how much wider the sector under the pointer and the open one's mark are,
/// how far the index's notch reaches, the name in its middle; the arrows,
/// the values between them and the gap before a stepper's name; the gaps
/// between the steppers' rows, before the ring, and before it all; and the
/// most height the bytes take.
const DIAL: f32 = 72.0;
const DIAL_TRACK: f32 = 10.0;
const DIAL_RAISE: f32 = 4.0;
const DIAL_NOTCH: f32 = 5.0;
const DIAL_NAME: f32 = 12.0;
const ARROW: f32 = 22.0;
const VALUE: f32 = 34.0;
const NAME_GAP: f32 = 6.0;
const ROW_GAP: f32 = 4.0;
const RING_GAP: f32 = 12.0;
const WAY_GAP: f32 = 20.0;
const BYTES_MOST: f32 = 360.0;
/// The room the sector window keeps from the app's edges, its own margins
/// and edges, and the least what is said takes.
const APP_EDGE: f32 = 16.0;
const WINDOW_EDGES: f32 = 26.0;
const SAID_LEAST: f32 = 160.0;
/// How long the open sector's mark takes to move round to the next, and to
/// fade where a sector has no place; and the ring to fade from one track's
/// sectors to another's, in seconds.
const GLIDE: f64 = 0.18;
const MARK_FADE: f32 = 0.12;
const RING_SWAP: f64 = 0.12;
/// A held arrow steps again REPEAT_AFTER seconds after its press, then
/// REPEAT_LEAST times a second, faster and faster to REPEAT_MOST over
/// REPEAT_RAMP seconds.
const REPEAT_AFTER: f64 = 0.4;
const REPEAT_LEAST: f64 = 6.0;
const REPEAT_MOST: f64 = 30.0;
const REPEAT_RAMP: f64 = 2.0;
/// The legend: the room above it and after each marked entry.
const LEGEND_GAP: f32 = 6.0;
const SIDE_GAP: f32 = 32.0;
/// How long the last track reported takes to fade in or out, in seconds.
const RING_FADE: f32 = 0.12;
/// The smallest a disk is drawn, in points.
const LEAST: f32 = 48.0;
/// Fitted, the rim round the tracks, room for the index's mark; how far in
/// they may reach; and the hole: each as a share of the disk's radius.
const FIT_RIM: f64 = 0.05;
const FIT_INNER: f64 = 0.30;
const FIT_HOLE: f64 = 0.22;
/// Fitted tracks WHOLE pixels apart or more are a whole number of pixels
/// apart, from a whole pixel, and SEPARATE lines apart or more have a line
/// between them; tracks to scale SEPARATE pixels apart or more are a whole
/// number of pixels apart.
const WHOLE: f64 = 2.0;
const SEPARATE: f64 = 4.0;
/// How close, in revolutions, one sector starts to where another ends for
/// the two to meet, and a line to divide them. Sectors placed from one
/// revolution meet to within a few bit cells; across the seam between two
/// revolutions, a real drive's changes of speed leave them up to a degree
/// or so apart, which counts on a track where others meet exactly. Formats
/// with a gap of EXACT or more after each sector have none that meet.
const EXACT: f64 = 1e-4;
const MEET: f64 = 0.003;
/// How far, behind a track with sectors missing or none found, the grid's
/// colour is toned toward the disk's; an incomplete sector keeps it at full
/// strength.
const MISSING_TONE: f32 = 0.15;
/// The least time between paintings of a disk while what it shows changes,
/// in seconds: a conversion reports tracks faster than they are worth painting.
const REPAINT: f64 = 0.1;
/// How long a disk's size must hold, in seconds, for its picture to be
/// painted at it once it has changed more than once in that time: while the
/// drawer's edge is dragged or the window resized, the last picture stands
/// in, scaled.
const SETTLE: f64 = 0.15;
/// How far a pixel's area reaches from its middle along any line, in
/// pixels, at most: half its diagonal, and a margin. A track's colours take
/// part in the pixels as near its room as this.
const REACH: f64 = 1.0;

/// The disk's colours, from the window's palette.
pub(crate) struct Look {
    /// The disk's surface, erased between tracks; and a 3½-inch disk's hub.
    body: Color32,
    hub: Color32,
    rim: Color32,
    /// Where no sector was found on a track gw decoded from flux; on a track
    /// with sectors missing, or with none found, the grid's colour for it,
    /// toned toward the disk's.
    gap: Color32,
    missing_gap: Color32,
    bad_gap: Color32,
    /// A track gw is to work on and has not reported: in the image view, and
    /// on the disk, a shade off its surface toward the text's colour.
    pub(crate) pending: Color32,
    to_do: Color32,
    /// A track of which what the view shows was not reported: where its
    /// sectors lie, or its flux. Further toward the text's colour, no hue.
    unknown: Color32,
    pub(crate) good: Color32,
    empty: Color32,
    /// Data its mark calls deleted, its CRC holding.
    deleted: Color32,
    /// A CRC that fails: of the data; of the header, a shade further toward
    /// the ink.
    pub(crate) bad: Color32,
    bad_header: Color32,
    /// A header with no data after it, or data with no header.
    pub(crate) alone: Color32,
    flux: Color32,
    pub(crate) last: Color32,
    index: Color32,
    /// The palette's strongest colour, furthest from the disk's: round what
    /// the pointer is over, and toward it, ID fields and the most flux.
    pub(crate) ink: Color32,
    /// How much of the disk the tracks cover: see Geometry::covered.
    covered: f32,
}

impl Look {
    pub(crate) fn of(p: &Palette, media: Media) -> Look {
        let body = theme::lerp(p.card, p.line, 0.5);
        Look {
            body,
            hub: theme::lerp(body, p.line_strong, 0.8),
            rim: p.line_strong,
            gap: theme::lerp(body, p.flux, 0.25),
            missing_gap: theme::lerp(p.partial, body, MISSING_TONE),
            bad_gap: theme::lerp(p.bad, body, MISSING_TONE),
            pending: p.pending,
            to_do: theme::lerp(body, p.text, 0.12),
            unknown: theme::lerp(body, p.text, 0.35),
            good: theme::lerp(p.good, body, 0.15),
            empty: theme::lerp(p.good, body, 0.55),
            deleted: theme::lerp(p.written, body, 0.15),
            bad: p.bad,
            bad_header: theme::lerp(p.bad, p.strong, 0.45),
            alone: p.partial,
            flux: p.flux,
            last: p.accent,
            index: p.dim,
            ink: p.strong,
            covered: media.size().map_or(1.0, |s| (s.width / s.pitch) as f32),
        }
    }

    /// `colour` on the tracks as it looks across them, with the disk erased
    /// between: the legend's swatch of it.
    fn seen(&self, colour: Color32) -> Color32 {
        theme::lerp(self.body, colour, self.covered)
    }

    /// A sector's colour, by how it decoded: as the legend counts it.
    fn status(&self, s: &Sector) -> Color32 {
        match Class::of(s) {
            Class::Good => self.good,
            Class::Empty => self.empty,
            Class::Deleted => self.deleted,
            Class::BadData => self.bad,
            Class::BadHeader => self.bad_header,
            Class::Incomplete => self.alone,
        }
    }

    /// The ID field of a sector in `colour`: a shade further from the disk's.
    fn id(&self, colour: Color32) -> Color32 {
        theme::lerp(colour, self.ink, 0.4)
    }

    /// Flux `d` times the track's average, as the Flux view shades it: none
    /// the disk's own colour, and the more, the further from it: the average
    /// a mid tone, half as much again the flux colour, then toward the ink.
    fn flux_at(&self, d: f32) -> Color32 {
        let mid = theme::lerp(self.body, self.flux, 0.7);
        match d {
            d if d <= 1.0 => theme::lerp(self.body, mid, d.max(0.0)),
            d if d <= 1.5 => theme::lerp(mid, self.flux, (d - 1.0) * 2.0),
            d => theme::lerp(self.flux, self.ink, ((d - 1.5) * 1.2).min(0.7)),
        }
    }
}

/// How the disks lie in the room `ui` has left, and what they show: how
/// many cylinders and sides, and whether the disk holds them; each disk's
/// picture and its diameter on whole pixels, its lines' width, and the
/// width of them all with the gaps between; their legend, laid out under
/// them, and their colours at that size; and the most height they can use,
/// as wide as the room lets them.
struct Room {
    span: u32,
    sides: u32,
    fits: bool,
    drawn: Arc<Drawn>,
    geometry: Geometry,
    line: f64,
    diameter: f32,
    width: f32,
    legend: Legend,
    look: Look,
    most: f32,
}

/// What a room was laid out for: the pass, whose one map it shows, and the
/// ui and the room it had left.
#[derive(Clone, Copy, PartialEq)]
struct RoomKey {
    pass: u64,
    ui: egui::Id,
    rect: Rect,
}

impl Room {
    /// The disks' room in what `ui` has left for `map`, laid out once a pass.
    /// None with no cylinders to show.
    fn of(ui: &egui::Ui, map: &Map) -> Option<Arc<Room>> {
        let ctx = ui.ctx();
        let kept = ctx.plugin_or_default::<Kept>();
        let key = RoomKey {
            pass: ctx.cumulative_pass_nr(),
            ui: ui.id(),
            rect: ui.available_rect_before_wrap(),
        };
        if let Some((_, room)) = kept.lock().room.as_ref().filter(|(k, _)| *k == key) {
            return Some(room.clone());
        }
        let room = Arc::new(Room::lay(ui, map, &kept)?);
        let mut kept = kept.lock();
        // The most height the disks can use changes with their legend's:
        // the pass again, the drawer as tall as that, not a frame late.
        let height = room.legend.height;
        if kept.legend.is_some_and(|h| h != height) {
            ctx.request_discard("the disks' legend");
            if !ctx.will_discard() {
                ctx.request_repaint();
            }
        }
        kept.legend = Some(height);
        kept.room = Some((key, room.clone()));
        Some(room)
    }

    fn lay(ui: &egui::Ui, map: &Map, kept: &TypedPluginHandle<Kept>) -> Option<Room> {
        let progress = map.progress;
        let span = span(progress, map.disk);
        if span == 0 {
            return None;
        }
        let sides = sides(progress, map.disk);
        let fits = map.media.holds().is_none_or(|n| span <= n);
        let p = theme::palette(ui);
        let drawn = Drawn::kept(kept, map, span, sides, fits, p);
        let ppp = ui.ctx().pixels_per_point();
        let line = f64::from(ppp).round().max(1.0);
        let room = ui.available_size();
        let n = sides as f32;
        let across = (room.x - SIDE_GAP * (n - 1.0)) / n;
        // The space between the disks and their legend.
        let gap = ui.spacing().item_spacing.y;
        let holds = map.media.holds().filter(|_| !fits).map(|n| {
            let name = map.media.name();
            format!("{span} cylinders: a {name} disk has room for {n}.")
        });
        // The legend runs across the room under the disks, whatever their
        // size, so its height sets theirs and never the other way round.
        // While gw works its counts change: the room it has taken it keeps.
        let mut look = Look::of(p, map.media);
        let mut legend = Legend::of(ui, map, &drawn, &look, holds);
        let mut height = legend.flow(ui, room.x);
        {
            let mut kept = kept.lock();
            match map.running {
                true => {
                    let taken = kept.floor.filter(|&(width, _)| width == room.x);
                    height = height.max(taken.map_or(0.0, |(_, h)| h));
                    kept.floor = Some((room.x, height));
                }
                false => kept.floor = None,
            }
        }
        let widest = Geometry::new(map.media, span, f64::from(across.max(LEAST) * ppp));
        let most = TITLE + widest.pixels as f32 / ppp + gap + height;
        let diameter = across.min(room.y - TITLE - gap - height).max(LEAST);
        let geometry = Geometry::new(map.media, span, f64::from(diameter * ppp));
        // Its colours as the tracks show them at that size; and the index's
        // mark only where a disk has room for it, in the room laid out.
        look.covered = geometry.covered(line);
        legend.see(&look);
        let notched =
            placed(sides, map.swapped).any(|(_, head)| geometry.notch(head, line).is_some());
        if !notched && legend.drop_index() {
            legend.flow(ui, room.x);
        }
        legend.height = height;
        let diameter = geometry.pixels as f32 / ppp;
        Some(Room {
            span,
            sides,
            fits,
            drawn,
            geometry,
            line,
            diameter,
            width: n * diameter + SIDE_GAP * (n - 1.0),
            legend,
            look,
            most,
        })
    }
}

/// Where the disks lie in the room left, centred: how many sides, each
/// disk's diameter, and the width of them all with the gaps between.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Place {
    pub sides: u32,
    pub diameter: f32,
    pub width: f32,
}

/// Where the disks would lie in the room `ui` has left, as `show` draws
/// them: for the image view to lie where they do.
pub(crate) fn place(ui: &egui::Ui, map: &Map) -> Option<Place> {
    Room::of(ui, map).map(|r| Place {
        sides: r.sides,
        diameter: r.diameter,
        width: r.width,
    })
}

/// The most height the disks can use in the room `ui` has left, their
/// legend under them: as wide as the room lets them.
pub fn most(ui: &egui::Ui, map: &Map) -> Option<f32> {
    Room::of(ui, map).map(|r| r.most)
}

/// Draws the disks side by side in the room the drawer gives them, with
/// their legend under them: as wide as the room lets them.
pub fn show(ui: &mut egui::Ui, map: &Map) {
    let Some(room) = Room::of(ui, map) else {
        return;
    };
    let progress = map.progress;
    let p = theme::palette(ui);
    let ppp = ui.ctx().pixels_per_point();
    let (look, diameter) = (&room.look, room.diameter);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), TITLE + diameter), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Disk map"));
    let painter = ui.painter_at(rect.expand(2.0));
    let left = rect.center().x - room.width / 2.0;
    let disks: Vec<Disk> = placed(room.sides, map.swapped)
        .enumerate()
        .map(|(place, (side, head))| {
            let x = left + place as f32 * (diameter + SIDE_GAP);
            let min = egui::pos2(x, rect.top() + TITLE).round_to_pixels(ppp);
            let picture = Rect::from_min_size(min, vec2(diameter, diameter));
            Disk {
                side,
                head,
                span: room.span,
                rect: picture,
                centre: picture.center(),
                scale: 1.0 / ppp,
                line: room.line,
                geometry: room.geometry,
            }
        })
        .collect();
    for d in &disks {
        let title = Rect::from_min_size(d.rect.min - vec2(0.0, TITLE), vec2(diameter, TITLE));
        painter.text(
            title.center(),
            Align2::CENTER_CENTER,
            format!("Side {}", d.side),
            FontId::proportional(TITLE_SIZE),
            p.dim,
        );
    }
    let kept = ui.ctx().plugin_or_default::<Kept>();
    {
        let mut kept = kept.lock();
        (kept.shown, kept.viewport) = (true, ui.ctx().viewport_id());
        for d in &disks {
            let head = d.head as usize;
            kept.seen[head] = true;
            d.draw(ui, &painter, map, &room, &mut kept.pictures[head]);
        }
    }
    let pointer = response.hover_pos();
    let hovered = pointer.and_then(|at| {
        let d = disks
            .iter()
            .find(|d| (at - d.centre).length() <= diameter / 2.0)?;
        Some((d, at, d.track_at(at)?))
    });
    if let Some((d, at, (cyl, share))) = hovered.filter(|_| room.fits) {
        let key = (cyl, d.side);
        let found = progress.facts.get(&key);
        let least = d.line_at(at);
        let index = found.and_then(|f| under(&f.sectors, share, least));
        let sector = index.zip(found).map(|(i, f)| &f.sectors[i]);
        let faint = Stroke::new(1.0, look.ink.gamma_multiply(0.45));
        d.outline(&painter, cyl, faint);
        if let Some((start, end)) = sector.and_then(|s| extent(s, least)) {
            let stroke = Stroke::new(1.5, look.ink);
            d.outline_arc(&painter, cyl, start, end, stroke);
        }
        if let Some((i, f)) = index.zip(found).filter(|_| response.clicked()) {
            let opened = ui
                .data(|d| d.get_temp::<Inspected>(inspected()))
                .map_or_else(|| opening(ui.ctx()), |was| was.opened);
            let open = Inspected {
                key,
                revision: f.revision,
                open: Open::Found(i),
                opened,
            };
            ui.data_mut(|d| d.insert_temp(inspected(), open));
        }
        response
            .clone()
            .on_hover_ui_at_pointer(|ui| tip(ui, map, key, share, index));
    }
    let over_title = pointer.and_then(|at| {
        disks
            .iter()
            .find(|d| at.y < d.rect.top() && (d.rect.left()..=d.rect.right()).contains(&at.x))
    });
    if let Some(d) = over_title {
        response
            .clone()
            .on_hover_ui_at_pointer(|ui| side_tip(ui, map, &room, d.side, d.head));
    }
    // The sector open in its window, outlined where it lies, its mark moving
    // with the ring's.
    if let Some(open) = ui.data(|d| d.get_temp::<Inspected>(inspected()))
        && room.fits
        && let Some(d) = disks.iter().find(|d| d.side == open.key.1)
    {
        let (cyl, _) = open.key;
        let target = match open.open {
            Open::Found(i) => track(progress, open.key).and_then(|f| f.sectors.get(i)?.at),
            Open::Missing { .. } => {
                d.outline(
                    &painter,
                    cyl,
                    Stroke::new(1.0, look.ink.gamma_multiply(0.45)),
                );
                None
            }
        };
        let target = target.map(|[a, _, b]| (f64::from(a), f64::from(b)));
        if let Some((span, lit)) = mark(ui.ctx(), open, target) {
            let (outer, inner) = d.ring(cyl);
            let least = d.line / (TAU * ((outer + inner) / 2.0).max(1.0));
            let (from, to) = widened(span, least);
            let stroke = Stroke::new(1.5, look.ink.gamma_multiply(lit));
            d.outline_arc(&painter, cyl, from, to, stroke);
        }
    }
    inspector(ui.ctx(), map);
    let top = ui.cursor().top();
    // From the disks' left edge; wider than they are, centred on them.
    let x = left.min(rect.center().x - room.legend.width / 2.0);
    room.legend.draw(ui, egui::pos2(x.max(rect.left()), top));
}

/// A disk as drawn, in pixels from its centre.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Geometry {
    /// The picture's width and height; the disk is centred in it.
    pixels: usize,
    edge: f64,
    /// A 3½-inch disk's hub, over its centre; else the hole.
    hub: Option<f64>,
    hole: f64,
    /// The outer edge of track 0's room on each side, from one track's room
    /// to the next, and how much of it the track takes.
    outer: [f64; 2],
    pitch: f64,
    width: f64,
}

impl Geometry {
    /// Whether fitted tracks have a line `line` pixels wide between them:
    /// where they are SEPARATE lines apart or more.
    fn separate(&self, line: f64) -> bool {
        self.width == self.pitch && self.pitch >= SEPARATE * line
    }

    /// How much of the disk the tracks cover, with lines `line` pixels wide:
    /// to scale, the width recorded of each track's room, the rest erased;
    /// fitted, all of it but any line.
    fn covered(&self, line: f64) -> f32 {
        let line = if self.separate(line) { line } else { 0.0 };
        ((self.width - line) / self.pitch) as f32
    }

    /// The index's notch on the disk `head` reads, with lines `line` pixels
    /// wide: how far its base and its tip lie from the centre, in the rim
    /// and pointing in at track 0; none where the rim has no room for it.
    fn notch(&self, head: u32, line: f64) -> Option<(f64, f64)> {
        let base = self.edge - line;
        let tip = (self.outer[head as usize] + line).max(base - 10.0 * line);
        (base - tip >= 2.0).then_some((base, tip))
    }

    /// A disk at most `room` pixels across for `span` tracks, which are
    /// drawn alike: fitted, a whole number of pixels wide from a whole
    /// pixel where they are WHOLE or more; to scale, a whole number of
    /// pixels apart where they are SEPARATE or more.
    fn new(media: Media, span: u32, room: f64) -> Geometry {
        let radius = (room / 2.0).floor().max(8.0);
        let Some(size) = media.size() else {
            let rim = (radius * FIT_RIM).round().max(2.0);
            let outer = radius - rim;
            let natural = (outer - radius * FIT_INNER) / f64::from(span.max(1));
            let pitch = if natural >= WHOLE {
                natural.floor()
            } else {
                natural
            };
            return Geometry {
                pixels: 2 * radius as usize,
                edge: radius,
                hub: None,
                hole: (radius * FIT_HOLE).round(),
                outer: [outer; 2],
                pitch,
                width: pitch,
            };
        };
        let mut per_mm = radius / size.radius;
        let pitch = size.pitch * per_mm;
        if pitch >= SEPARATE {
            per_mm = pitch.floor() / size.pitch;
        }
        let edge = size.radius * per_mm;
        Geometry {
            // The radius scaled back may come out a hair over the room.
            pixels: 2 * (edge.ceil() as usize).min(radius as usize),
            edge,
            hub: size.hub.map(|r| r * per_mm),
            hole: size.hole * per_mm,
            outer: size.track_0.map(|x| (x + size.pitch / 2.0) * per_mm),
            pitch: size.pitch * per_mm,
            width: size.width * per_mm,
        }
    }
}

/// One side's disk as drawn.
struct Disk {
    /// The disk's side, as gw numbers it, and the drive head that reads it,
    /// whose side of the disk its tracks lie on.
    side: u32,
    head: u32,
    /// The cylinders drawn.
    span: u32,
    /// The picture, on whole pixels.
    rect: Rect,
    centre: Pos2,
    /// Points per pixel, and the width of a line, a point in whole pixels.
    scale: f32,
    line: f64,
    geometry: Geometry,
}

/// The side each disk shows, from the left, and the drive head that reads
/// it: with the heads swapped, side 0 is read by head 1, and with both
/// sides shown, side 1 comes first.
fn placed(sides: u32, swapped: bool) -> impl Iterator<Item = (u32, u32)> {
    (0..sides).map(move |place| {
        let side = match swapped && sides == 2 {
            true => 1 - place,
            false => place,
        };
        (side, if swapped { 1 - side } else { side })
    })
}

/// Where a share of a revolution from the index points, from the centre:
/// the index at the top, the track running on clockwise. Seen from side 0
/// the disk turns counter-clockwise (ECMA-125, 4.15), so this is either
/// side's track as seen from side 0.
fn heading(share: f64) -> (f64, f64) {
    let a = TAU * share;
    (a.sin(), -a.cos())
}

/// The share of a revolution from the index that a point `dx`, `dy` from
/// the centre lies at.
fn share_at(dx: f64, dy: f64) -> f64 {
    dx.atan2(-dy).rem_euclid(TAU) / TAU
}

impl Disk {
    /// The track's outer and inner radii as drawn, in pixels: the width
    /// recorded, in the middle of its room.
    fn ring(&self, cyl: u32) -> (f64, f64) {
        let g = &self.geometry;
        let middle = g.outer[self.head as usize] - (f64::from(cyl) + 0.5) * g.pitch;
        (middle + g.width / 2.0, middle - g.width / 2.0)
    }

    /// A line's length as a share of a revolution, at `at`: the least a
    /// sector is drawn.
    fn line_at(&self, at: Pos2) -> f64 {
        let r = f64::from(((at - self.centre) / self.scale).length());
        self.line / (TAU * r.max(1.0))
    }

    /// The track whose room lies under `at`, and the share of a revolution
    /// from the index.
    fn track_at(&self, at: Pos2) -> Option<(u32, f64)> {
        let g = &self.geometry;
        let v = (at - self.centre) / self.scale;
        let r = f64::from(v.length());
        let cyl = ((g.outer[self.head as usize] - r) / g.pitch).floor();
        if cyl < 0.0 || cyl >= f64::from(self.span) {
            return None;
        }
        Some((cyl as u32, share_at(f64::from(v.x), f64::from(v.y))))
    }

    /// Draws the disk in `room`: its picture, kept in `slot`, painted again
    /// where what it shows has changed and at its size once that holds; the
    /// index's mark at the top; and round the last track gw reported, a ring.
    fn draw(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        map: &Map,
        room: &Room,
        slot: &mut Option<Picture>,
    ) {
        let ctx = ui.ctx();
        let p = theme::palette(ui);
        let (drawn, fits, look) = (&*room.drawn, room.fits, &room.look);
        let key = Key {
            media: map.media,
            shows: drawn.shows,
            span: self.span,
            side: self.side,
            head: self.head,
            geometry: self.geometry,
            palette: p,
            line: room.line,
            fits,
            pure: drawn.pure,
        };
        let track = |cyl: usize| row(ring(map, (cyl as u32, self.side), drawn, fits, p), look);
        // A pass to be laid out again shows nothing: it paints nothing.
        if !ctx.will_discard() {
            let known = stamps(map, self.side, self.span, p);
            let (now, released) = ui.input(|i| (i.time, i.pointer.any_released()));
            let painted = |known| Painted {
                key,
                stamps: known,
                at: now,
                asked: (key.geometry, now),
            };
            match slot {
                None => *slot = Some(Picture::new(ctx, self, look, track, painted(known))),
                Some(picture) => match picture.painted.due(&key, &known, now, released) {
                    Due::No => {}
                    Due::Later(wait) => ctx.request_repaint_after(Duration::from_secs_f64(wait)),
                    Due::Tracks(changed) => {
                        picture.paint(self, look, track, &changed, false, painted(known))
                    }
                    Due::All(changed) => {
                        picture.paint(self, look, track, &changed, true, painted(known))
                    }
                },
            }
        }
        if let Some(picture) = slot {
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(picture.texture.id(), self.rect, uv, Color32::WHITE);
        }
        self.index_mark(painter, look);
        // The last track reported, ringed. Going from side to side, it fades
        // out on one as it fades in on the other, from the report on.
        let id = egui::Id::new("last reported").with(self.side);
        let reported = map.current.filter(|&(_, s)| fits && s == self.side);
        if let Some((cyl, _)) = reported {
            ctx.data_mut(|d| d.insert_temp(id, cyl));
        }
        let shown = ctx.animate_bool_with_time(id.with("shown"), reported.is_some(), RING_FADE);
        let cyl = reported
            .map(|r| r.0)
            .or_else(|| ctx.data(|d| d.get_temp::<u32>(id)));
        if let Some(cyl) = cyl.filter(|_| shown > 0.0) {
            let eased = shown * shown * (3.0 - 2.0 * shown);
            self.outline(
                painter,
                cyl,
                Stroke::new(1.5, look.last.gamma_multiply(eased)),
            );
        }
    }

    /// The index's mark: a notch in the rim at the top, pointing in at track
    /// 0, where the rim has room for it.
    fn index_mark(&self, painter: &egui::Painter, look: &Look) {
        let Some((base, tip)) = self.geometry.notch(self.head, self.line) else {
            return;
        };
        let (tip, base) = (tip as f32 * self.scale, base as f32 * self.scale);
        painter.add(notch(self.centre, (tip, base), look.index));
    }

    /// Rings track `cyl` at its edges.
    fn outline(&self, painter: &egui::Painter, cyl: u32, stroke: Stroke) {
        let (outer, inner) = self.ring(cyl);
        let half = stroke.width / 2.0;
        let outer = outer as f32 * self.scale + half;
        let inner = (inner as f32 * self.scale - half).max(0.0);
        painter.circle_stroke(self.centre, outer, stroke);
        painter.circle_stroke(self.centre, inner, stroke);
    }

    /// Outlines track `cyl` from share `from` to `to` of a revolution.
    fn outline_arc(&self, painter: &egui::Painter, cyl: u32, from: f64, to: f64, stroke: Stroke) {
        let (outer, inner) = self.ring(cyl);
        let edges = (outer as f32 * self.scale, inner as f32 * self.scale);
        painter.add(outline(self.centre, edges, (from, to), stroke));
    }
}

/// What a track's ring shows, as the legend names it.
#[derive(Clone, Copy)]
enum Ring<'a> {
    /// Nothing: the bare disk. gw's image holds no such track, or gw passed
    /// over it, or the job has nothing to do with it.
    Bare,
    /// gw is to work on it and has not reported it.
    ToDo,
    /// What the view shows of it was not reported: where its sectors lie, or
    /// its flux.
    Unknown,
    /// Each sector gw found, where it found it; where gw decoded flux and
    /// found none, the gap's colour, or its shortfall's.
    Sectors(&'a Facts, Shortfall),
    /// Read as flux, not decoded.
    Flux,
    /// The flux round it, against the track's average.
    Spin(&'a Spin),
    /// Its colour on the grid, where no track drawn shows more than gw's
    /// line: the grid's legend names it.
    Status(Color32),
}

/// What track `key`'s ring shows: in the Sectors view, the sectors gw found
/// where it found them, or that it read flux and decoded none of it; in the
/// Flux view, its flux. With neither, its status as the grid shows it if no
/// track drawn has more; else to do while gw is on it, not known, or nothing
/// if gw passed over it. Then, if gw is to work on it, to do; else nothing.
fn ring<'a>(map: &Map<'a>, key: (u32, u32), drawn: &Drawn, fits: bool, p: &Palette) -> Ring<'a> {
    let progress = map.progress;
    let facts = progress.facts.get(&key);
    let track = progress.tracks.get(&key);
    if !fits || facts.is_some_and(|f| f.absent) {
        return Ring::Bare;
    }
    if facts.is_none() && track.is_none() {
        return match planned(progress, key) {
            true => Ring::ToDo,
            false => Ring::Bare,
        };
    }
    if drawn.pure {
        return match diskmap::fill(progress, key, p) {
            Some(colour) => Ring::Status(colour),
            None if planned(progress, key) => Ring::ToDo,
            None => Ring::Bare,
        };
    }
    // The track gw is on, its line all gw has said of it yet: its report is
    // to come, as a write's comes once gw has written the track.
    if facts.is_none()
        && working_on(map, key)
        && track.is_some_and(|t| t.status != Status::Skipped)
        && planned(progress, key)
    {
        return Ring::ToDo;
    }
    let placed = facts.is_some_and(|f| f.sectors.iter().any(|s| s.at.is_some()));
    match (drawn.shows, facts) {
        (
            Shows::Flux,
            Some(Facts {
                flux: Some(spin), ..
            }),
        ) => Ring::Spin(spin),
        (Shows::Sectors, Some(f)) if placed || (f.summary.is_some() && f.sectors.is_empty()) => {
            Ring::Sectors(f, Shortfall::of(f))
        }
        (Shows::Sectors, Some(f)) if f.summary.is_none() && f.flux.is_some() => Ring::Flux,
        (_, None) if track.is_some_and(|t| t.status == Status::Skipped) => Ring::Bare,
        _ => Ring::Unknown,
    }
}

/// What a ring shows round it: in the Sectors view, each sector where it
/// was found, its ID field shaded, where none was found the gap's colour,
/// and a line where two meet; in the Flux view, its flux against the
/// track's average; else one colour all round.
fn row(ring: Ring, look: &Look) -> Row {
    let mut row = match ring {
        Ring::Bare => Row::new(look.body),
        Ring::ToDo => Row::new(look.to_do),
        Ring::Unknown => Row::new(look.unknown),
        Ring::Flux => Row::new(look.flux),
        Ring::Status(colour) => Row::new(colour),
        Ring::Spin(spin) => Row::pieces(spin.relative().iter().map(|&d| look.flux_at(d))),
        Ring::Sectors(f, shortfall) => {
            let background = match (f.flux.is_some(), shortfall) {
                (false, _) => look.body,
                (true, Shortfall::None) => look.gap,
                (true, Shortfall::Missing) => look.missing_gap,
                (true, Shortfall::Bad) => look.bad_gap,
            };
            let mut row = Row::new(background);
            for s in &f.sectors {
                row.sector(s, look);
            }
            row
        }
    };
    row.finish();
    row
}

/// What a track lacks of the sectors its format lays out, as the grid
/// counts it from gw's own "(n/m sectors)": some, as Sectors missing; all,
/// as Bad, as is a track gw's ibm.scan calls IBM Empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shortfall {
    None,
    Missing,
    Bad,
}

impl Shortfall {
    fn of(f: &Facts) -> Shortfall {
        let summary = f.summary.as_deref().unwrap_or_default();
        match progress::sectors(summary) {
            _ if summary.starts_with("IBM Empty") => Shortfall::Bad,
            Some((0, all)) if all > 0 => Shortfall::Bad,
            Some((good, all)) if good < all => Shortfall::Missing,
            _ => Shortfall::None,
        }
    }
}

/// Whether gw announced it would work on `key`.
fn planned(progress: &Progress, (cyl, head): (u32, u32)) -> bool {
    progress.cyls.contains(&cyl) && progress.heads.contains(&head)
}

/// Whether gw is on track `key` as it works: the track it last reported.
fn working_on(map: &Map, key: (u32, u32)) -> bool {
    map.running && map.current == Some(key)
}

/// What the disk view draws, as its legend counts it.
#[derive(Debug, Default, PartialEq)]
struct Drawn {
    /// What the tracks show: their flux only where a track drawn has some.
    shows: Shows,
    /// No track drawn shows more than gw's line: see Ring::Status.
    pure: bool,
    /// The sectors drawn, by how they decoded, as Class lists them; and of
    /// the incomplete, whether headers alone and data alone are drawn.
    sectors: [usize; Class::ALL.len()],
    headers_alone: bool,
    data_alone: bool,
    /// ID fields; sectors that meet; where gw decoded flux, places with no
    /// sector on tracks gw found whole; and tracks with sectors missing, or
    /// none found, so drawn.
    id_fields: bool,
    meet: bool,
    gaps: bool,
    missing_tracks: usize,
    bad_tracks: usize,
    /// Tracks read as flux and not decoded, not known, and to do.
    flux: usize,
    unknown: usize,
    to_do: usize,
    /// The sectors the formats lay out that gw did not find, and those that
    /// share their ID with another of their track's.
    missing: usize,
    shared: usize,
    /// Each track's colour on the grid, where it shows that.
    statuses: Vec<Color32>,
    /// A hard-sectored disk's: its index, as gw takes it, sector 0's hole.
    holes: bool,
}

/// What a count of what the disk view draws was of: the job's revision,
/// the track gw is on as it works, how many tracks the view spans, what it
/// shows and in which colours. The tracks gw was to work on are kept beside.
#[derive(Clone, Copy, PartialEq)]
struct DrawnKey {
    revision: u64,
    on: Option<(u32, u32)>,
    span: u32,
    sides: u32,
    shows: Shows,
    fits: bool,
    palette: &'static Palette,
}

/// A count of what the disk view draws, what it was of, and the cylinders
/// and heads gw was to work on then.
struct Counted {
    key: DrawnKey,
    cyls: Vec<u32>,
    heads: Vec<u32>,
    drawn: Arc<Drawn>,
}

impl Drawn {
    /// Drawn::of, kept in `kept` until what it is of changes.
    fn kept(
        kept: &TypedPluginHandle<Kept>,
        map: &Map,
        span: u32,
        sides: u32,
        fits: bool,
        p: &'static Palette,
    ) -> Arc<Drawn> {
        let progress = map.progress;
        let key = DrawnKey {
            revision: progress.revision,
            on: map.current.filter(|_| map.running),
            span,
            sides,
            shows: map.shows,
            fits,
            palette: p,
        };
        // The tracks gw was to work on, compared where they lie.
        let same =
            |c: &&Counted| c.key == key && c.cyls == progress.cyls && c.heads == progress.heads;
        if let Some(counted) = kept.lock().counted.as_ref().filter(same) {
            return counted.drawn.clone();
        }
        let drawn = Arc::new(Drawn::of(map, span, sides, fits, p));
        kept.lock().counted = Some(Counted {
            key,
            cyls: progress.cyls.clone(),
            heads: progress.heads.clone(),
            drawn: drawn.clone(),
        });
        drawn
    }

    /// What the disk view draws of `map`'s tracks: `span` cylinders on
    /// `sides` sides, if the disk holds them.
    fn of(map: &Map, span: u32, sides: u32, fits: bool, p: &Palette) -> Drawn {
        let progress = map.progress;
        let fluxed = fluxed(progress, span, sides);
        let placed = progress
            .facts
            .iter()
            .any(|(&(c, h), f)| c < span && h < sides && f.sectors.iter().any(|s| s.at.is_some()));
        let mut drawn = Drawn {
            shows: map.shows.given(fluxed),
            pure: !fluxed && !placed,
            ..Drawn::default()
        };
        drawn.count(map, span, 0..sides, fits, p);
        drawn
    }

    /// Counts what the view, as `self` shows it, draws of `map`'s tracks:
    /// `span` cylinders on `sides`, if the disk holds them.
    fn count(&mut self, map: &Map, span: u32, sides: Range<u32>, fits: bool, p: &Palette) {
        let progress = map.progress;
        for (cyl, side) in (0..span).flat_map(|c| sides.clone().map(move |s| (c, s))) {
            match ring(map, (cyl, side), self, fits, p) {
                Ring::Sectors(f, shortfall) => {
                    for s in &f.sectors {
                        let Some(at) = s.at.map(|a| a.map(f64::from)) else {
                            continue;
                        };
                        let class = Class::of(s);
                        self.sectors[class as usize] += 1;
                        let alone = class == Class::Incomplete;
                        self.headers_alone |= alone && s.header != Header::None;
                        self.data_alone |= alone && s.header == Header::None;
                        // A header alone, all ID field, is the incomplete's.
                        self.id_fields |= s.data != Data::None && id_end(s, at) > at[0];
                    }
                    self.meet |= meet(&f.sectors);
                    if f.flux.is_some() {
                        match shortfall {
                            Shortfall::None => self.gaps = true,
                            Shortfall::Missing => self.missing_tracks += 1,
                            Shortfall::Bad => self.bad_tracks += 1,
                        }
                    }
                }
                Ring::Flux => self.flux += 1,
                Ring::Unknown => self.unknown += 1,
                Ring::ToDo => self.to_do += 1,
                Ring::Status(colour) => self.statuses.push(colour),
                Ring::Bare | Ring::Spin(_) => {}
            }
            let facts = progress.facts.get(&(cyl, side)).filter(|_| fits);
            self.holes |= facts.is_some_and(|f| f.flux.as_ref().is_some_and(|s| s.holes));
            self.missing += facts.map_or(0, |f| f.missing.len());
            self.shared += facts.map_or(0, |f| {
                let sectors = &f.sectors;
                let shares =
                    |s: &Sector| sectors.iter().any(|t| !std::ptr::eq(s, t) && same_id(s, t));
                sectors.iter().filter(|s| shares(s)).count()
            });
        }
    }
}

/// What a disk's picture is of, beside its tracks.
#[derive(Clone, Copy, PartialEq)]
struct Key {
    media: Media,
    shows: Shows,
    span: u32,
    side: u32,
    head: u32,
    geometry: Geometry,
    /// The palette its colours are from, which a theme changes, and the
    /// width of its lines, a point in whole pixels.
    palette: &'static Palette,
    line: f64,
    fits: bool,
    /// See Drawn::pure.
    pure: bool,
}

impl Key {
    /// The same disk drawn the same way, at another size: while the drawer's
    /// edge is dragged, the last picture stands in, scaled.
    fn same_but_size(&self, other: &Key) -> bool {
        Key {
            geometry: other.geometry,
            ..*self
        } == *other
    }
}

/// The disk view's pictures, one for each head's disk, kept while the view
/// draws them: at the end of a pass that does not draw one, it is let go,
/// and with the view, the sector window shuts.
#[derive(Default)]
struct Kept {
    pictures: [Option<Picture>; 2],
    /// The pictures drawn this pass, and whether the view was, in which
    /// viewport.
    seen: [bool; 2],
    shown: bool,
    viewport: egui::ViewportId,
    /// The room last laid out, and what the view draws, as the legend
    /// counts it, for what each was of; and the legend's height, last laid
    /// out.
    room: Option<(RoomKey, Arc<Room>)>,
    counted: Option<Counted>,
    legend: Option<f32>,
    /// While gw works, the room's width and the height its legend has
    /// taken there, which it keeps: changing counts do not resize the disks.
    floor: Option<(f32, f32)>,
}

impl egui::Plugin for Kept {
    fn debug_name(&self) -> &'static str {
        "disk view"
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx();
        if ctx.viewport_id() != self.viewport {
            return;
        }
        for (picture, seen) in self.pictures.iter_mut().zip(&mut self.seen) {
            if !std::mem::take(seen) {
                *picture = None;
            }
        }
        if !std::mem::take(&mut self.shown) {
            ctx.data_mut(|d| d.remove::<Inspected>(inspected()));
        }
    }
}

/// A disk's picture as last painted: its tracks' rows then, its pixels, and
/// the texture that shows them.
struct Picture {
    painted: Painted,
    rows: Vec<Row>,
    image: Arc<egui::ColorImage>,
    texture: egui::TextureHandle,
}

impl Picture {
    /// `disk`'s picture, painted in full, `row` making each track's row.
    fn new(
        ctx: &egui::Context,
        disk: &Disk,
        look: &Look,
        row: impl Fn(usize) -> Row,
        painted: Painted,
    ) -> Picture {
        let rows: Vec<Row> = (0..disk.span as usize).map(row).collect();
        let mut image = egui::ColorImage::filled([disk.geometry.pixels; 2], Color32::TRANSPARENT);
        Canvas::new(disk, look, &rows, painted.key.line).paint(&mut image, None);
        let image = Arc::new(image);
        let options = egui::TextureOptions::LINEAR;
        let texture = ctx.load_texture("disk", image.clone(), options);
        Picture {
            painted,
            rows,
            image,
            texture,
        }
    }

    /// Paints the pixels over the tracks `changed` again, their rows made
    /// again by `row`; with `all`, every pixel, at the disk's size. The
    /// texture takes them in place where its size holds: of the tracks
    /// changed, only the rows of pixels painted again.
    fn paint(
        &mut self,
        disk: &Disk,
        look: &Look,
        row: impl Fn(usize) -> Row,
        changed: &[usize],
        all: bool,
        painted: Painted,
    ) {
        self.rows.resize_with(disk.span as usize, Row::default);
        for &cyl in changed {
            self.rows[cyl] = row(cyl);
        }
        let size = [disk.geometry.pixels; 2];
        let resized = self.image.size != size;
        if resized {
            self.image = Arc::new(egui::ColorImage::filled(size, Color32::TRANSPARENT));
        }
        let dirty = (!all).then(|| {
            let mut dirty = vec![false; self.rows.len()];
            changed.iter().for_each(|&c| dirty[c] = true);
            dirty
        });
        let canvas = Canvas::new(disk, look, &self.rows, painted.key.line);
        let rows = canvas.paint(Arc::make_mut(&mut self.image), dirty.as_deref());
        let options = egui::TextureOptions::LINEAR;
        match (resized, dirty) {
            (true, _) => self.texture.set(self.image.clone(), options),
            (false, None) => self
                .texture
                .set_partial([0, 0], self.image.clone(), options),
            (false, Some(_)) if !rows.is_empty() => {
                let part = self
                    .image
                    .region_by_pixels([0, rows.start], [size[0], rows.len()]);
                self.texture.set_partial([0, rows.start], part, options);
            }
            (false, Some(_)) => {}
        }
        let asked = self.painted.asked;
        self.painted = Painted { asked, ..painted };
    }
}

/// What a picture was last painted of: its key, its tracks' stamps, and
/// when, in egui's seconds; and the size last asked of it, and since when.
#[derive(Clone, PartialEq)]
struct Painted {
    key: Key,
    stamps: Vec<Stamp>,
    at: f64,
    asked: (Geometry, f64),
}

/// What of a picture is to be painted.
#[derive(Debug, PartialEq)]
enum Due {
    No,
    /// Nothing yet: again in so many seconds.
    Later(f64),
    /// The pixels over these tracks, their rows made again.
    Tracks(Vec<usize>),
    /// Every pixel, at another size or another way, these tracks' rows made
    /// again.
    All(Vec<usize>),
}

impl Painted {
    /// What is due to be painted of a picture of `key` and `stamps` at `now`:
    /// the tracks whose stamps have changed, once REPAINT has passed since
    /// the last painting; and at another size, all of it, at once if the
    /// size had held for SETTLE, else once the size holds that long or the
    /// pointer that drags it lets go.
    fn due(&mut self, key: &Key, stamps: &[Stamp], now: f64, released: bool) -> Due {
        let same = self.key == *key;
        let resized = !same && self.key.same_but_size(key);
        let held = now - self.asked.1 >= SETTLE;
        if self.asked.0 != key.geometry {
            self.asked = (key.geometry, now);
        }
        let changed: Vec<usize> = (0..stamps.len())
            .filter(|&c| !(same || resized) || self.stamps.get(c) != stamps.get(c))
            .collect();
        if same && changed.is_empty() {
            Due::No
        } else if same && now - self.at < REPAINT {
            Due::Later(REPAINT - (now - self.at))
        } else if same {
            Due::Tracks(changed)
        } else if resized && !held && !released {
            Due::Later(SETTLE - (now - self.asked.1))
        } else {
            Due::All(changed)
        }
    }
}

/// What is known of a track that its ring shows: its status as gw printed
/// it, the bridge's report, whether gw is to work on it, its colour on the
/// grid, and whether gw is on it as it works, its report yet to come.
type Stamp = (Option<Status>, Option<u64>, bool, Option<Color32>, bool);

/// What is known of each of `side`'s tracks in `map`.
fn stamps(map: &Map, side: u32, span: u32, p: &Palette) -> Vec<Stamp> {
    let progress = map.progress;
    (0..span)
        .map(|cyl| {
            let key = (cyl, side);
            let facts = progress.facts.get(&key);
            (
                progress.tracks.get(&key).map(|t| t.status),
                facts.map(|f| f.revision),
                planned(progress, key),
                diskmap::fill(progress, key, p),
                // Once reported, the track gw is on shows its report.
                facts.is_none() && working_on(map, key),
            )
        })
        .collect()
}

/// What paints a disk's picture, pixel by pixel.
struct Canvas<'a> {
    pixels: usize,
    centre: f64,
    geometry: Geometry,
    /// The outer edge of track 0's room on this side.
    outer: f64,
    rows: &'a [Row],
    /// The width of a line, a point in whole pixels; and whether fitted
    /// tracks are wide enough to have a line between them.
    line: f64,
    separate: bool,
    body: [f64; 3],
    hub: [f64; 3],
    rim: [f64; 3],
}

impl<'a> Canvas<'a> {
    /// `disk`'s picture of `rows`, its lines `line` pixels wide.
    fn new(disk: &Disk, look: &Look, rows: &'a [Row], line: f64) -> Canvas<'a> {
        let g = disk.geometry;
        Canvas {
            pixels: g.pixels,
            centre: g.pixels as f64 / 2.0,
            geometry: g,
            outer: g.outer[disk.head as usize],
            rows,
            line,
            separate: g.separate(line),
            body: rgb(look.body),
            hub: rgb(look.hub),
            rim: rgb(look.rim),
        }
    }

    /// Paints `image`, or where `only` is given, the pixels as near those of
    /// its tracks as their colours reach: each row of pixels in a share of
    /// the threads there are, and of a row, only where it crosses them. The
    /// rows of pixels it may have painted.
    fn paint(&self, image: &mut egui::ColorImage, only: Option<&[bool]>) -> Range<usize> {
        let width = self.pixels;
        let bands = only.map(|dirty| self.bands(dirty));
        let reached = bands.as_deref().map_or(0..width, |b| self.reach(b));
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
        let rows = reached.len().div_ceil(threads).max(1);
        let pixels = &mut image.pixels[reached.start * width..reached.end * width];
        std::thread::scope(|scope| {
            for (i, chunk) in pixels.chunks_mut(rows * width).enumerate() {
                let bands = bands.as_deref();
                let first = reached.start + i * rows;
                scope.spawn(move || {
                    for (j, line) in chunk.chunks_mut(width).enumerate() {
                        let y = first + j;
                        let spans = match bands {
                            Some(bands) => self.spans(y, bands),
                            None => vec![(0, width)],
                        };
                        for (from, to) in spans {
                            for (x, pixel) in line.iter_mut().enumerate().take(to).skip(from) {
                                *pixel = self.pixel(x, y);
                            }
                        }
                    }
                });
            }
        });
        reached
    }

    /// The rows of pixels whose middles lie nearer the centre's row than the
    /// outermost of `bands` reaches: those spans finds pixels in.
    fn reach(&self, bands: &[(f64, f64)]) -> Range<usize> {
        let outer = bands.iter().map(|b| b.1).fold(0.0, f64::max);
        let (above, below) = (self.centre - outer - 0.5, self.centre + outer - 0.5);
        let from = ((above.floor() + 1.0).max(0.0) as usize).min(self.pixels);
        let to = (below.ceil().max(0.0) as usize).clamp(from, self.pixels);
        from..to
    }

    /// The radii, from the inner to the outer, between which the pixels lie
    /// that the tracks marked in `dirty` take part in: each one's room, REACH
    /// either side; those that overlap made one.
    fn bands(&self, dirty: &[bool]) -> Vec<(f64, f64)> {
        let pitch = self.geometry.pitch;
        let mut bands: Vec<(f64, f64)> = Vec::new();
        for cyl in (0..dirty.len()).filter(|&c| dirty[c]) {
            // Tracks in turn, from the outermost in.
            let outer = self.outer - cyl as f64 * pitch;
            let (from, to) = (outer - pitch - REACH, outer + REACH);
            match bands.last_mut() {
                Some(last) if to >= last.0 => last.0 = from,
                _ => bands.push((from, to)),
            }
        }
        bands
    }

    /// Where along the row of pixels `y` they lie between the radii of
    /// `bands`, by their middles, and a pixel more either side: from each
    /// to before each.
    fn spans(&self, y: usize, bands: &[(f64, f64)]) -> Vec<(usize, usize)> {
        let dy = (y as f64 + 0.5 - self.centre).abs();
        let mut spans = Vec::new();
        for &(inner, outer) in bands.iter().filter(|&&(_, outer)| outer > dy) {
            let far = (outer * outer - dy * dy).sqrt();
            let near = match inner > dy {
                true => (inner * inner - dy * dy).sqrt(),
                false => 0.0,
            };
            // The pixels whose middles lie `near` to `far` from the centre's
            // column, either side of it.
            let x = |dx: f64| self.centre - 0.5 + dx;
            for (a, b) in [(x(-far), x(-near)), (x(near), x(far))] {
                let from = (a.floor() - 1.0).max(0.0) as usize;
                let to = ((b.ceil() + 2.0).max(0.0) as usize).min(self.pixels);
                if to > from {
                    spans.push((from, to));
                }
            }
        }
        spans
    }

    /// The pixel's offset from the centre, to its middle, and its distance.
    fn offset(&self, x: usize, y: usize) -> (f64, f64, f64) {
        let (dx, dy) = (x as f64 + 0.5 - self.centre, y as f64 + 0.5 - self.centre);
        (dx, dy, (dx * dx + dy * dy).sqrt())
    }

    /// A pixel of the picture, premultiplied: what of the disk its area
    /// covers, by its share of each: the rim, the hub, each track, and the
    /// bare disk, the lines between fitted tracks with it.
    fn pixel(&self, x: usize, y: usize) -> Color32 {
        let g = &self.geometry;
        let (dx, dy, r) = self.offset(x, y);
        let across = Shadow::of(dx, dy, r);
        if r - across.half >= g.edge || r + across.half <= g.hole {
            return Color32::TRANSPARENT;
        }
        let within = |a: f64, b: f64| across.within(a - r, b - r);
        // Most pixels lie wholly on the disk, clear of its rim and hub.
        let (near, far) = (r - across.half, r + across.half);
        let disk = match near >= g.hole && far <= g.edge {
            true => 1.0,
            false => within(g.hole, g.edge),
        };
        let rim = match far <= g.edge - self.line {
            true => 0.0,
            false => within(g.edge - self.line, g.edge),
        };
        let hub = match g.hub {
            Some(hub) if near < hub => within(g.hole, hub),
            _ => 0.0,
        };
        let (tracks, taken) = self.tracks(dx, dy, r, across);
        let bare = (disk - rim - hub - taken).max(0.0);
        let colour = [0, 1, 2]
            .map(|k| tracks[k] + self.body[k] * bare + self.rim[k] * rim + self.hub[k] * hub);
        let [red, green, blue] = colour.map(|c| c.round().clamp(0.0, 255.0) as u8);
        Color32::from_rgba_premultiplied(red, green, blue, (disk * 255.0).round() as u8)
    }

    /// The tracks under the pixel `dx`, `dy` from the centre, `r` pixels
    /// out, its shadow across them `across`: each track's colour, as much as
    /// the pixel's area its width covers, a fitted track's line inside its
    /// outer edge taken from it; and how much they cover together.
    fn tracks(&self, dx: f64, dy: f64, r: f64, across: Shadow) -> ([f64; 3], f64) {
        let g = &self.geometry;
        let mut colour = [0.0; 3];
        let mut taken = 0.0;
        let (near, far) = (r - across.half, r + across.half);
        let first = ((self.outer - far) / g.pitch).floor().max(0.0) as usize;
        let last = ((self.outer - near) / g.pitch).floor();
        if last < 0.0 || first >= self.rows.len() {
            return (colour, taken);
        }
        let last = (last as usize).min(self.rows.len() - 1);
        let share = share_at(dx, dy);
        // A pixel's length round the track there, as a share of a revolution.
        let pixel = (1.0 / (TAU * r.max(0.5))).min(1.0);
        let along = across.scaled(pixel);
        for cyl in first..=last {
            let edge = self.outer - cyl as f64 * g.pitch;
            let middle = edge - g.pitch / 2.0;
            let half = g.width / 2.0;
            let mut part = across.within(middle - half - r, middle + half - r);
            if self.separate {
                part -= across.within(edge - self.line - r, edge - r);
            }
            if part > 0.0 {
                let c = self.rows[cyl].sample(share, along, self.line * pixel, self.body);
                (0..3).for_each(|k| colour[k] += c[k] * part);
                taken += part;
            }
        }
        (colour, taken)
    }
}

/// How a pixel's area lies along a line through its middle at an angle to
/// its sides: the share of it within a distance either side, a square's
/// shadow on the line. A trapezoid reaching `half` either side, flat for
/// `flat` either side: square to the line, a box; on the diagonal, a
/// triangle.
#[derive(Debug, Clone, Copy)]
struct Shadow {
    half: f64,
    flat: f64,
}

impl Shadow {
    /// A pixel's shadow on the line from the disk's centre through its
    /// middle, `dx`, `dy` from it and `r` out; across that line, the same.
    fn of(dx: f64, dy: f64, r: f64) -> Shadow {
        let (c, s) = match r > 0.0 {
            true => (dx.abs() / r, dy.abs() / r),
            false => (1.0, 0.0),
        };
        Shadow {
            half: (c + s) / 2.0,
            flat: (c - s).abs() / 2.0,
        }
    }

    /// A pixel square to the line, `width` long on it.
    #[cfg(test)]
    fn square(width: f64) -> Shadow {
        Shadow {
            half: width / 2.0,
            flat: width / 2.0,
        }
    }

    /// The same shadow `scale` times as long.
    fn scaled(self, scale: f64) -> Shadow {
        Shadow {
            half: self.half * scale,
            flat: self.flat * scale,
        }
    }

    /// The share of the pixel less than `t` past its middle.
    fn below(self, t: f64) -> f64 {
        let (half, flat) = (self.half, self.flat);
        if t <= -half {
            return 0.0;
        }
        if t >= half {
            return 1.0;
        }
        let (ramp, height) = (half - flat, 1.0 / (half + flat));
        if t < -flat {
            height * (t + half).powi(2) / (2.0 * ramp)
        } else if t <= flat {
            height * (ramp / 2.0 + t + flat)
        } else {
            1.0 - height * (half - t).powi(2) / (2.0 * ramp)
        }
    }

    /// The share of the pixel from `a` to `b` past its middle.
    fn within(self, a: f64, b: f64) -> f64 {
        match b > a {
            true => self.below(b) - self.below(a),
            false => 0.0,
        }
    }
}

/// A track round a revolution, from the index: the pieces it is made of,
/// each from its start to the next one's, the last to 1; where two sectors
/// meet; and the sectors too short to see.
#[derive(Clone, Default)]
struct Row {
    /// Where each piece starts; none where all are as long as each other.
    starts: Vec<f64>,
    colours: Vec<Color32>,
    /// Where each sector starts and ends, while the row is laid out; then
    /// where two meet.
    edges: Vec<(f64, f64)>,
    meets: Vec<f64>,
    /// Each sector's middle, length and colour, the shortest first: where
    /// one is shorter than a line, it is drawn a line long.
    slivers: Vec<(f64, f64, Color32)>,
}

impl Row {
    fn new(colour: Color32) -> Row {
        Row {
            starts: vec![0.0],
            colours: vec![colour],
            ..Row::default()
        }
    }

    /// Pieces all as long as each other round the revolution, one per colour,
    /// of which there are some: track::spin makes no count of no parts.
    fn pieces(colours: impl Iterator<Item = Color32>) -> Row {
        Row {
            colours: colours.collect(),
            ..Row::default()
        }
    }

    /// The piece share `t` of a revolution lies in, from 0 to 1.
    fn piece(&self, t: f64) -> usize {
        let n = self.colours.len();
        let i = match self.starts.is_empty() {
            true => ((t * n as f64) as usize).min(n - 1),
            false => self.starts.partition_point(|&s| s <= t).saturating_sub(1),
        };
        // An equal piece's start, worked out again, may lie a hair past `t`.
        match i > 0 && self.bounds(i).0 > t {
            true => i - 1,
            false => i,
        }
    }

    /// Where piece `i` starts and ends.
    fn bounds(&self, i: usize) -> (f64, f64) {
        let n = self.colours.len();
        match self.starts.is_empty() {
            true => (i as f64 / n as f64, (i + 1) as f64 / n as f64),
            false => (
                self.starts[i],
                self.starts.get(i + 1).copied().unwrap_or(1.0),
            ),
        }
    }

    /// The piece that starts at `at`, from 0 to 1, splitting the one it falls in.
    fn split(&mut self, at: f64) -> usize {
        let i = self.starts.partition_point(|&s| s <= at);
        if self.starts[i - 1] == at {
            return i - 1;
        }
        let colour = self.colours[i - 1];
        self.starts.insert(i, at);
        self.colours.insert(i, colour);
        i
    }

    /// `colour` over the row from share `from` to `to` of a revolution,
    /// which may run on past 1, over the index, but no further round than
    /// `from`.
    fn lay(&mut self, from: f64, to: f64, colour: Color32) {
        let length = (to - from).min(1.0);
        let from = from.rem_euclid(1.0);
        let to = from + length;
        let parts = [(from, to.min(1.0)), (0.0, (to - 1.0).max(0.0))];
        for (from, to) in parts.into_iter().filter(|(a, b)| b > a) {
            let a = self.split(from);
            let b = if to >= 1.0 {
                self.starts.len()
            } else {
                self.split(to)
            };
            self.colours[a..b].fill(colour);
        }
    }

    /// A sector where it lies, from its start to its end in the colour the
    /// legend counts it by, its ID field a shade of it.
    fn sector(&mut self, s: &Sector, look: &Look) {
        let Some(at) = s.at.map(|a| a.map(f64::from)) else {
            return;
        };
        let [start, _, end] = at;
        let colour = look.status(s);
        self.lay(start, end, colour);
        self.lay(start, id_end(s, at), look.id(colour));
        // A header alone is all ID field.
        let whole = match (s.header, s.data) {
            (Header::Good | Header::Bad, Data::None) => look.id(colour),
            _ => colour,
        };
        let middle = ((start + end) / 2.0).rem_euclid(1.0);
        self.slivers.push((middle, end - start, whole));
        self.edges
            .push((start.rem_euclid(1.0), end.rem_euclid(1.0)));
    }

    /// Finds where two sectors meet, and puts the sectors in order of
    /// length, once the row is laid out.
    fn finish(&mut self) {
        self.meets = meets(&std::mem::take(&mut self.edges));
        self.slivers.sort_by(|a, b| a.1.total_cmp(&b.1));
    }

    /// The row as a pixel's area covers it, `along` the pixel's shadow round
    /// the track, centred on `share`: with a line `line` long in `colour`
    /// where two sectors meet, and over any sector shorter than that, a line
    /// in its colour, the longest on top.
    fn sample(&self, share: f64, along: Shadow, line: f64, colour: [f64; 3]) -> [f64; 3] {
        let short = &self.slivers[..self.slivers.partition_point(|s| s.1 < line)];
        if self.colours.len() == 1 && self.meets.is_empty() && short.is_empty() {
            return rgb(self.colours[0]);
        }
        let half = line / 2.0;
        let reach = along.half + half;
        let lined =
            near(&self.meets, share, reach) || short.iter().any(|s| apart(s.0, share) < reach);
        match lined {
            false => self.pieces_under(share, along),
            true => self.lined(share, along, half, short, colour),
        }
    }

    /// The pieces' colours as the pixel's area covers them.
    fn pieces_under(&self, share: f64, along: Shadow) -> [f64; 3] {
        let (lo, hi) = (share - along.half, share + along.half);
        // Most pixels lie within a piece.
        if lo >= 0.0 && hi < 1.0 {
            let i = self.piece(lo);
            if self.bounds(i).1 >= hi {
                return rgb(self.colours[i]);
            }
        }
        let mut c = [0.0; 3];
        let mut turn = lo.floor();
        while turn < hi {
            let (a, b) = ((lo - turn).max(0.0), (hi - turn).min(1.0));
            let mut i = self.piece(a);
            loop {
                let (start, end) = self.bounds(i);
                let (u, v) = (start.max(a), end.min(b));
                let m = along.within(u + turn - share, v + turn - share);
                let p = rgb(self.colours[i]);
                (0..3).for_each(|k| c[k] += p[k] * m);
                i += 1;
                if end >= b || i == self.colours.len() {
                    break;
                }
            }
            turn += 1.0;
        }
        c
    }

    /// The colours as the pixel's area covers them where lines lie over the
    /// pieces: between each edge and the next, what shows on top there.
    fn lined(
        &self,
        share: f64,
        along: Shadow,
        half: f64,
        short: &[(f64, f64, Color32)],
        colour: [f64; 3],
    ) -> [f64; 3] {
        let (lo, hi) = (share - along.half, share + along.half);
        let mut edges = vec![lo, hi];
        let mut turn = lo.floor();
        while turn < hi {
            let (a, b) = ((lo - turn).max(0.0), (hi - turn).min(1.0));
            let mut i = self.piece(a);
            loop {
                let (start, end) = self.bounds(i);
                edges.push(start + turn);
                i += 1;
                if end >= b || i == self.colours.len() {
                    break;
                }
            }
            turn += 1.0;
        }
        for turn in [-1.0, 0.0, 1.0] {
            let (from, to) = (lo - half - turn, hi + half - turn);
            let first = self.meets.partition_point(|&x| x < from);
            let meets = self.meets[first..].iter().take_while(|&&x| x <= to);
            let lines = meets.chain(short.iter().map(|s| &s.0));
            for x in lines.filter(|&&x| (from..=to).contains(&x)) {
                edges.extend([x - half + turn, x + half + turn]);
            }
        }
        edges.retain(|&t| (lo..=hi).contains(&t));
        edges.sort_by(f64::total_cmp);
        let mut c = [0.0; 3];
        for pair in edges.windows(2) {
            let (u, v) = (pair[0], pair[1]);
            let m = along.within(u - share, v - share);
            if m > 0.0 {
                let p = self.shown((u + v) / 2.0, half, short, colour);
                (0..3).for_each(|k| c[k] += p[k] * m);
            }
        }
        c
    }

    /// What shows at share `t`: the longest of the sectors too short to see
    /// whose line lies there, else a line where two meet, else the piece.
    fn shown(
        &self,
        t: f64,
        half: f64,
        short: &[(f64, f64, Color32)],
        colour: [f64; 3],
    ) -> [f64; 3] {
        if let Some(s) = short.iter().rev().find(|s| apart(s.0, t) <= half) {
            return rgb(s.2);
        }
        if self.meets.iter().any(|&m| apart(m, t) <= half) {
            return colour;
        }
        rgb(self.colours[self.piece(t.rem_euclid(1.0))])
    }
}

/// Where the ID field of a sector found `at` its start, data and end is
/// drawn to: where gw gives the field's end; a header alone, all of it;
/// else nowhere, from its start, as gw gives no end to draw it to.
fn id_end(s: &Sector, [start, data, end]: [f64; 3]) -> f64 {
    match (s.header, s.header_end, s.data) {
        (Header::None, ..) => start,
        (_, Some(e), _) => f64::from(e).min(data),
        (_, None, Data::None) => end,
        (_, None, _) => start,
    }
}

/// How far apart two shares of a revolution are, round the shorter way.
fn apart(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(1.0);
    d.min(1.0 - d)
}

/// Whether any of `xs`, in order from 0 to 1, lies within `reach` of
/// `share`, round the index if need be.
fn near(xs: &[f64], share: f64, reach: f64) -> bool {
    let (lo, hi) = (share - reach, share + reach);
    let turns: &[f64] = match lo >= 0.0 && hi <= 1.0 {
        true => &[0.0],
        false => &[-1.0, 0.0, 1.0],
    };
    turns.iter().any(|turn| {
        let i = xs.partition_point(|&x| x < lo - turn);
        xs.get(i).is_some_and(|&x| x <= hi - turn)
    })
}

fn rgb(c: Color32) -> [f64; 3] {
    [c.r(), c.g(), c.b()].map(f64::from)
}

/// What a track's facts are of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// Flux read from the disk.
    Read,
    /// Flux read back from the disk to verify a write.
    Verify,
    /// An image's flux, a conversion's input.
    Image,
    /// An image's flux that gw writes as it is.
    WrittenImage,
    /// The master track gw writes, from the index.
    Written,
}

impl Origin {
    fn of(image: bool, source: Option<Source>) -> Origin {
        match (image, source) {
            (_, Some(Source::Verify)) => Origin::Verify,
            (_, Some(Source::Written)) => Origin::Written,
            (_, Some(Source::Image)) => Origin::WrittenImage,
            (true, None) => Origin::Image,
            (false, None) => Origin::Read,
        }
    }

    fn name(self) -> Option<&'static str> {
        match self {
            Origin::Read => None,
            Origin::Verify => Some("Read back to verify"),
            Origin::Image => Some("From the image"),
            Origin::WrittenImage => Some("As written, from the image"),
            Origin::Written => Some("As written"),
        }
    }
}

/// What is known of the hovered track, at `share` of a revolution from the
/// index, and of its sector there, `index` of its sectors.
fn tip(ui: &mut egui::Ui, map: &Map, (cyl, side): (u32, u32), share: f64, index: Option<usize>) {
    let progress = map.progress;
    let reported = progress.facts.get(&(cyl, side));
    let absent = reported.is_some_and(|f| f.absent);
    let facts = reported.filter(|f| !f.absent);
    let track = progress.tracks.get(&(cyl, side));
    ui.strong(format!("Cylinder {cyl} · side {side}"));
    match (facts.and_then(|f| f.summary.as_deref()), track) {
        _ if absent => {
            ui.label(crate::progress::NOT_IN_INPUT);
        }
        (Some(summary), _) => {
            ui.label(summary);
        }
        (None, Some(t)) => {
            ui.label(&t.text);
        }
        (None, None) => {
            ui.weak("Not reported.");
        }
    }
    let mut notes = Vec::new();
    if let Some(name) = facts.and_then(|f| Origin::of(map.image, f.source).name()) {
        notes.push(name.to_owned());
    }
    if let Some(place) = track.and_then(|t| t.place.as_deref()) {
        notes.push(place.to_owned());
    }
    if let Some(t) = track.filter(|t| t.retries > 0) {
        notes.push(diskmap::retry_text(t.retries));
    }
    if map.current == Some((cyl, side)) {
        notes.push("Last reported".to_owned());
    }
    if !notes.is_empty() {
        ui.weak(notes.join(" · "));
    }
    // Where the pointer is, and when, in each revolution read whole.
    let spin = facts.and_then(|f| f.flux.as_ref());
    let zero = match spin.is_some_and(|s| s.holes) {
        true => "sector 0's hole",
        false => "the index",
    };
    ui.weak(match spin.map(|spin| from_index(spin, share)) {
        Some(ms) => format!("At {:.1}° · {ms} ms from {zero}", share * 360.0),
        None => format!("At {:.1}° from {zero}", share * 360.0),
    });
    let Some(f) = facts else {
        return;
    };
    if let Some(s) = index.and_then(|i| f.sectors.get(i)) {
        ui.separator();
        sector_tip(ui, s, &f.sectors);
        if s.bytes.len() > 64 {
            ui.weak(format!("Click for all {} bytes.", s.bytes.len()));
        }
    }
    let unplaced = f.sectors.iter().filter(|s| s.at.is_none()).count();
    let order = order(&f.sectors);
    let repeated = repeated(&f.sectors);
    if !f.missing.is_empty() || unplaced > 0 || order.is_some() || !repeated.is_empty() {
        ui.separator();
    }
    if let Some(order) = order {
        ui.label(format!("Order: {order}"));
    }
    if !repeated.is_empty() {
        ui.label(format!("ID repeated: {}", repeated.join(", ")));
    }
    if !f.missing.is_empty() {
        let ids: Vec<String> = f.missing.iter().map(short_id).collect();
        ui.label(format!("Missing: {}", ids.join(", ")));
    }
    if unplaced > 0 {
        ui.label(match unplaced {
            1 => "1 sector not placed".to_owned(),
            n => format!("{n} sectors not placed"),
        });
    }
    if let Some(spin) = &f.flux {
        ui.separator();
        if map.shows == Shows::Flux {
            let here = (relative_at(spin, share) * 100.0).round();
            ui.label(format!("Flux here: {here}% of the track's average"));
        }
        spin_tip(ui, f, spin, Origin::of(map.image, f.source));
        if let Some(intervals) = &spin.intervals {
            intervals_chart(ui, intervals);
        }
    }
}

/// How far `share` of a revolution lies from the index, in milliseconds:
/// that share of each revolution read whole, the same in all to 0.01 ms or
/// else from the least to the most; with none, of gw's measure of the drive.
fn from_index(spin: &Spin, share: f64) -> String {
    let ms = |seconds: f64| format!("{:.2}", share * seconds * 1e3);
    if spin.revs.is_empty() {
        return ms(spin.period);
    }
    let least = spin.revs.iter().copied().fold(f64::INFINITY, f64::min);
    let most = spin.revs.iter().copied().fold(0.0, f64::max);
    match (ms(least), ms(most)) {
        (least, most) if least == most => least,
        (least, most) => format!("{least}–{most}"),
    }
}

/// The track's flux at `share` of a revolution against its mean, as
/// Spin::relative gives it for that part alone.
fn relative_at(spin: &Spin, share: f64) -> f32 {
    let parts = spin.bins.len();
    let at = ((share * parts as f64) as usize).min(parts - 1);
    let mean = spin.per_rev / parts as f64;
    match mean > 0.0 {
        true => (f64::from(spin.bins[at]) / mean) as f32,
        false => 0.0,
    }
}

/// The IDs of a track's sectors in turn round it from the index, each R,
/// "?" where its header's CRC fails, or number: of every sector with an ID,
/// where all are placed and there are two or more.
fn order(sectors: &[Sector]) -> Option<String> {
    let named: Vec<&Sector> = sectors.iter().filter(|s| s.id != Id::None).collect();
    if named.len() < 2 || named.iter().any(|s| s.at.is_none()) {
        return None;
    }
    let ids: Vec<String> = named
        .iter()
        .map(|s| match s.id {
            Id::Ibm([.., r, _]) if s.header == Header::Good => r.to_string(),
            Id::Ibm(_) => "?".to_owned(),
            Id::Number(n) => n.to_string(),
            Id::None => unreachable!(),
        })
        .collect();
    Some(ids.join(" "))
}

/// Each ID two or more of a track's sectors carry, from headers whose CRC
/// holds, and how many: as `R3 ×2`, or in full where another repeated ID
/// shares its R.
fn repeated(sectors: &[Sector]) -> Vec<String> {
    let mut seen: Vec<(Id, usize)> = Vec::new();
    for s in sectors.iter().filter(|s| sure_id(s)) {
        match seen.iter_mut().find(|(id, _)| *id == s.id) {
            Some((_, n)) => *n += 1,
            None => seen.push((s.id, 1)),
        }
    }
    seen.retain(|&(_, n)| n > 1);
    // An R that two of them share names neither: their whole IDs do.
    let name = |id: &Id| {
        let r = short_id(id);
        match seen
            .iter()
            .filter(|(other, _)| short_id(other) == r)
            .count()
        {
            1 => r,
            _ => id_text(id),
        }
    };
    seen.iter()
        .map(|(id, n)| format!("{} ×{n}", name(id)))
        .collect()
}

/// How far apart the track's flux transitions are: a count of each bin's,
/// from no time to the longest counted, and how many were longer.
fn intervals_chart(ui: &mut egui::Ui, i: &Intervals) {
    if i.counts.is_empty() && i.longer == 0 {
        return;
    }
    let bin = match i.width < 1e-6 {
        true => format!("{:.1} ns", i.width * 1e9),
        false => format!("{} µs", (i.width * 1e9).round() / 1e3),
    };
    ui.label(format!("Flux intervals in µs, bins of {bin}"));
    if !i.counts.is_empty() {
        intervals_plot(ui, i);
    }
    if i.longer > 0 {
        ui.weak(format!(
            "{} of {} µs or longer",
            grouped(i.longer),
            top_text(i.top)
        ));
    }
}

/// `top` seconds in microseconds as the chart says it: whole, or rounded
/// down at 0.01 µs, so that each interval counted is as long or longer.
fn top_text(top: f64) -> String {
    let us = top * 1e6;
    match (us - us.round()).abs() < 1e-6 {
        true => format!("{}", us.round()),
        false => format!("{:.2}", (us * 100.0 + 1e-6).floor() / 100.0),
    }
}

/// Where the chart's axis ends for bins up to `span` µs, and its ticks'
/// step: whole microseconds, one or two apart. A hair over a whole number,
/// as floating point leaves bins' ends, is that number.
fn axis(span: f64) -> (f64, f64) {
    let span = span - 1e-9;
    let step = if span > 12.0 { 2.0 } else { 1.0 };
    (((span / step).ceil() * step).max(step), step)
}

/// The chart's bars over its axis, each bin's count against the most.
fn intervals_plot(ui: &mut egui::Ui, i: &Intervals) {
    let p = theme::palette(ui);
    let width_us = i.width * 1e6;
    let (end, step) = axis((i.first as usize + i.counts.len()) as f64 * width_us);
    let font = egui::TextStyle::Small.resolve(ui.style());
    let label = ui.text_style_height(&egui::TextStyle::Small);
    let (rect, _) = ui.allocate_exact_size(vec2(240.0, 56.0 + label + 2.0), Sense::hover());
    // Room either side for half a label, as each is centred on its tick.
    let plot = Rect::from_min_max(
        rect.min + vec2(6.0, 0.0),
        egui::pos2(rect.right() - 8.0, rect.bottom() - label - 2.0),
    );
    let painter = ui.painter();
    let most = f64::from(i.counts.iter().copied().max().unwrap_or(1).max(1));
    let x = |us: f64| plot.left() + (us / end) as f32 * plot.width();
    for (k, &n) in i.counts.iter().enumerate() {
        if n == 0 {
            continue;
        }
        // Over the values its bin holds, from its first tick to its last.
        let from = (i.first as usize + k) as f64 * width_us;
        let to = from + width_us - i.tick * 1e6;
        let height = (f64::from(n) / most) as f32 * plot.height();
        let bar = Rect::from_min_max(
            egui::pos2(x(from), plot.bottom() - height.max(1.0)),
            egui::pos2(x(to).max(x(from) + 1.0), plot.bottom()),
        );
        painter.rect_filled(bar, 0.0, p.flux);
    }
    painter.hline(
        plot.x_range(),
        plot.bottom() + 0.5,
        Stroke::new(1.0, p.line_strong),
    );
    let mut us = 0.0;
    while us <= end + 1e-9 {
        let at = egui::pos2(x(us), plot.bottom() + 2.0);
        painter.text(at, Align2::CENTER_TOP, format!("{us}"), font.clone(), p.dim);
        us += step;
    }
}

/// The track's revolutions and flux.
fn spin_tip(ui: &mut egui::Ui, f: &Facts, spin: &Spin, from: Origin) {
    ui.label(spin_line(spin, from));
    let mut flux = format!("{} flux/rev", grouped(spin.per_rev.round() as u64));
    if let Some(cell) = f.cell {
        flux += &format!(" · {:.3} µs cells", cell * 1e6);
    }
    ui.label(flux);
}

/// The track's revolutions in a line, with the rate of their mean: each
/// read whole; of a track gw wrote, its format's; with none read, gw's
/// measure of the drive, or the image's.
fn spin_line(spin: &Spin, from: Origin) -> String {
    let ms = |seconds: f64| format!("{:.2}", seconds * 1e3);
    let rpm = format!("{:.2} rpm", 60.0 / spin.period);
    let revs: Vec<String> = spin.revs.iter().map(|&r| ms(r)).collect();
    match (from, revs.is_empty()) {
        (Origin::Written, _) => format!("Format: {} ms · {rpm}", ms(spin.period)),
        // As gw scaled them, not as the drive turned.
        (_, false) if spin.scaled => format!("Scaled: {} ms · {rpm}", revs.join(", ")),
        (_, true) if spin.scaled => format!("Scaled: {} ms · {rpm}", ms(spin.period)),
        (_, false) => format!("{} ms · {rpm}", revs.join(", ")),
        (Origin::Read | Origin::Verify, true) => format!("Drive: {} ms · {rpm}", ms(spin.period)),
        (_, true) => format!("{} ms · {rpm}", ms(spin.period)),
    }
}

/// `n` with its thousands apart.
pub(crate) fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A side's sums over the cylinders `room` draws: see sums.
fn side_tip(ui: &mut egui::Ui, map: &Map, room: &Room, side: u32, head: u32) {
    match side == head {
        true => ui.strong(format!("Side {side}")),
        false => ui.strong(format!("Side {side} · head {head}")),
    };
    let p = theme::palette(ui);
    let (sums, encodings) = sums(map, &room.drawn, room.span, room.fits, side, p);
    if !sums.is_empty() {
        ui.label(sums);
    }
    if !encodings.is_empty() {
        ui.label(encodings.join(", "));
    }
}

/// A side's sectors drawn by how they decoded, and those missing, as the
/// legend of the view `drawn` counts them, in a line; and the encodings gw
/// decoded: over the `span` cylinders drawn, if the disk holds them.
fn sums<'a>(
    map: &Map<'a>,
    drawn: &Drawn,
    span: u32,
    fits: bool,
    side: u32,
    p: &Palette,
) -> (String, Vec<&'a str>) {
    let mut counted = Drawn {
        shows: drawn.shows,
        pure: drawn.pure,
        ..Drawn::default()
    };
    counted.count(map, span, side..side + 1, fits, p);
    let mut sums: Vec<String> = (Class::ALL.map(Class::name).iter().zip(counted.sectors))
        .filter(|&(_, n)| n > 0)
        .map(|(name, n)| format!("{name} {n}"))
        .collect();
    if counted.missing > 0 && counted.shows == Shows::Sectors && !counted.pure {
        sums.push(format!("{} missing", counted.missing));
    }
    let facts = map.progress.facts.iter();
    let facts = facts.filter(|((c, h), f)| *h == side && *c < span && !f.absent);
    let mut encodings: Vec<&str> = Vec::new();
    for (_, f) in facts {
        let summary = f.summary.as_deref().unwrap_or_default();
        let encoding = summary.split(" (").next().unwrap_or_default();
        if !encoding.is_empty() && !encodings.contains(&encoding) {
            encodings.push(encoding);
        }
    }
    (sums.join(" · "), encodings)
}

/// The sector whose data is open: its track, the track's facts' revision,
/// which sector, and which opening of its window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Inspected {
    key: (u32, u32),
    revision: u64,
    open: Open,
    /// Counts the window's openings: each is measured and centred afresh,
    /// and its mark starts where its sector lies.
    opened: u64,
}

/// Which sector of a track is open.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Open {
    /// The track's sector at this place among them, round from the index.
    Found(usize),
    /// None that gw found, stepped to from a sector with this ID on another
    /// track: the ID the format gives it here where the format lays it out
    /// on the track (`laid`), else the one it had there.
    Missing { id: Id, laid: bool },
}

/// A step the sector window's arrows, ring or keys ask for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    /// So many sectors on round the track, back where fewer than none.
    Round(i32),
    /// The track's sector at this place among them.
    To(usize),
    /// So many cylinders on, of those gw reported on the side.
    Cylinder(i32),
    /// The other side.
    Side,
}

impl Inspected {
    /// Where `step` leads from here over the tracks `progress` holds; None
    /// where it leads to no other sector.
    fn stepped(self, progress: &Progress, step: Step) -> Option<Inspected> {
        let here = track(progress, self.key)?;
        let (cyl, side) = self.key;
        let found = |i| Inspected {
            open: Open::Found(i),
            ..self
        };
        // On another track, the sector with this one's ID, where it lies.
        let elsewhere = |key| {
            let there = track(progress, key)?;
            let (id, near) = match self.open {
                Open::Found(i) => {
                    let s = here.sectors.get(i)?;
                    (s.id, s.at.map(|[start, ..]| start))
                }
                Open::Missing { id, .. } => (id, None),
            };
            Some(Inspected {
                key,
                revision: there.revision,
                open: find(there, id, near),
                opened: self.opened,
            })
        };
        match step {
            Step::Round(by) => round(self.open, here.sectors.len(), by).map(found),
            Step::To(i) => (i < here.sectors.len()).then(|| found(i)),
            Step::Cylinder(by) => next_cylinder(&cylinders(progress, side), cyl, by)
                .and_then(|c| elsewhere((c, side))),
            Step::Side => (side < 2).then(|| elsewhere((cyl, 1 - side))).flatten(),
        }
    }
}

/// What gw reported of track `key`, where its image holds the track.
fn track(progress: &Progress, key: (u32, u32)) -> Option<&Facts> {
    progress.facts.get(&key).filter(|f| !f.absent)
}

/// The cylinders of `side` that gw reported a track of, in order.
fn cylinders(progress: &Progress, side: u32) -> Vec<u32> {
    progress
        .facts
        .iter()
        .filter(|&(&(_, h), f)| h == side && !f.absent)
        .map(|(&(c, _), _)| c)
        .collect()
}

/// The place among a track's `count` sectors that `by` steps round lead to
/// from `open`, on from the last to the first and back, and from a sector
/// gw did not find, from the index. None with no other to go to.
fn round(open: Open, count: usize, by: i32) -> Option<usize> {
    let count = i64::try_from(count).ok().filter(|&n| n > 0)?;
    let from = match open {
        Open::Found(_) if count == 1 => return None,
        Open::Found(i) => i64::try_from(i).ok()?,
        Open::Missing { .. } if by > 0 => -1,
        Open::Missing { .. } => count,
    };
    usize::try_from((from + i64::from(by)).rem_euclid(count)).ok()
}

/// The cylinder `by` on from `cyl` among `cyls`, on from the last to the
/// first and back. None with no other.
fn next_cylinder(cyls: &[u32], cyl: u32, by: i32) -> Option<u32> {
    let count = i64::try_from(cyls.len()).ok().filter(|&n| n > 1)?;
    let from = i64::try_from(cyls.iter().position(|&c| c == cyl)?).ok()?;
    let to = usize::try_from((from + i64::from(by)).rem_euclid(count)).ok()?;
    cyls.get(to).copied()
}

/// What opens on a track gw reported `f` of, for a sector with `id` on
/// another, which starts `near` its index there: the sector with that ID
/// whose header's checks hold, the nearest of two; with no ID, the nearest
/// sector; else the ID, missing.
fn find(f: &Facts, id: Id, near: Option<f32>) -> Open {
    let apart = |i: usize| match (near, f.sectors[i].at) {
        (Some(near), Some([start, ..])) => {
            let d = (start - near).rem_euclid(1.0);
            d.min(1.0 - d)
        }
        _ => f32::INFINITY,
    };
    let same = |i: &usize| {
        let s = &f.sectors[*i];
        id == Id::None || (s.header == Header::Good && alike(&s.id, &id))
    };
    let nearest = (0..f.sectors.len())
        .filter(same)
        .min_by(|&a, &b| apart(a).total_cmp(&apart(b)));
    if let Some(i) = nearest {
        return Open::Found(i);
    }
    match f.missing.iter().find(|m| alike(m, &id)) {
        Some(&laid) => Open::Missing {
            id: laid,
            laid: true,
        },
        None => Open::Missing { id, laid: false },
    }
}

/// Whether two IDs name the same sector, each of its own track: an IBM-style
/// header's R, or a number.
fn alike(a: &Id, b: &Id) -> bool {
    match (a, b) {
        (Id::Ibm([.., a, _]), Id::Ibm([.., b, _])) => a == b,
        (Id::Number(a), Id::Number(b)) => a == b,
        _ => false,
    }
}

/// A sector gw did not find, as its window names it: its ID as the format
/// lays it out on the track, or its R alone.
fn missing_name(id: Id, laid: bool) -> String {
    match id {
        Id::Ibm(_) if !laid => short_id(&id),
        _ => id_text(&id),
    }
}

/// What is said of a sector gw did not find on a track it reported `f` of:
/// its name, whether the format lays it out there, and gw's line on the
/// track.
fn missing_lines(f: &Facts, id: Id, laid: bool) -> Vec<(String, Tone)> {
    let name = missing_name(id, laid);
    let name = match id {
        Id::Ibm(_) => format!("Sector {name}"),
        _ => name,
    };
    let said = if laid { "Missing" } else { "Not found" };
    let mut lines = vec![(name, Tone::Strong), (said.to_owned(), Tone::Plain)];
    lines.extend(f.summary.clone().map(|s| (s, Tone::Weak)));
    lines
}

/// The id under which the sector whose data is open is kept.
fn inspected() -> egui::Id {
    egui::Id::new("disk sector")
}

/// A new opening of the sector window, counted.
fn opening(ctx: &egui::Context) -> u64 {
    ctx.data_mut(|d| {
        let opened = d.get_temp_mut_or_default::<u64>(egui::Id::new("disk sector openings"));
        *opened += 1;
        *opened
    })
}

/// The window a click on a sector opens: all gw decoded of it, its data in
/// full, and the way to the others: round the track, on the other cylinders
/// and on the other side.
fn inspector(ctx: &egui::Context, map: &Map) {
    let Some(open) = ctx.data(|d| d.get_temp::<Inspected>(inspected())) else {
        return;
    };
    let shut = || ctx.data_mut(|d| d.remove::<Inspected>(inspected()));
    // Gone once the track is reported again, or another job's shows.
    let Some(f) = track(map.progress, open.key).filter(|f| f.revision == open.revision) else {
        return shut();
    };
    let (cyl, side) = open.key;
    let (name, lines, bytes) = match open.open {
        Open::Found(i) => {
            let Some(s) = f.sectors.get(i) else {
                return shut();
            };
            (id_text(&s.id), sector_lines(s, &f.sectors), &s.bytes[..])
        }
        Open::Missing { id, laid } => (missing_name(id, laid), missing_lines(f, id, laid), &[][..]),
    };
    let title = format!("{name} · cylinder {cyl}, side {side}");
    let goes = |step| open.stepped(map.progress, step).is_some();
    let nav = Nav {
        progress: map.progress,
        facts: f,
        open,
        round: goes(Step::Round(1)),
        cylinder: goes(Step::Cylinder(1)),
        side: goes(Step::Side),
    };
    let shown = Shown {
        title: &title,
        lines: &lines,
        bytes,
        base: 0,
        nav: Some(nav),
    };
    let id = egui::Id::new("disk sector window").with(open.opened);
    let asked = sector_window(ctx, id, &shown);
    if asked.close {
        shut();
    } else if let Some(next) = asked.step.and_then(|step| open.stepped(map.progress, step)) {
        ctx.data_mut(|d| d.insert_temp(inspected(), next));
    }
}

/// What a sector window shows: its title, what is said of the sector, its
/// data, the first byte numbered `base`, and the way round the disk.
pub(crate) struct Shown<'a> {
    pub title: &'a str,
    pub lines: &'a [(String, Tone)],
    pub bytes: &'a [u8],
    pub base: usize,
    pub nav: Option<Nav<'a>>,
}

/// The way from the open sector to the others: its track, on the ring, and
/// whether each arrow has another to go to: round the track, to another
/// cylinder, to the other side.
pub(crate) struct Nav<'a> {
    progress: &'a Progress,
    facts: &'a Facts,
    open: Inspected,
    round: bool,
    cylinder: bool,
    side: bool,
}

/// What a sector window is asked this frame: to shut, or to step.
#[derive(Default)]
pub(crate) struct Asked {
    pub close: bool,
    step: Option<Step>,
}

/// A sector's window: a title bar as tall as a macOS window's, what is said
/// of the sector, its data in full, and the way round the disk. It opens in
/// the middle of the app's, once measured, then grows from its top right
/// corner, never smaller than it has been: the arrows there stay put as
/// what it shows changes.
pub(crate) fn sector_window(ctx: &egui::Context, id: egui::Id, shown: &Shown) -> Asked {
    let mut asked = Asked::default();
    let title = shown.title;
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(0);
    let window = egui::Window::new(title)
        .id(id)
        .title_bar(false)
        .frame(frame)
        .resizable(false);
    let state = egui::AreaState::load(ctx, id);
    // Once measured, where its right edge stays.
    let right = state.filter(|s| s.size.is_some()).map(|s| s.rect().right());
    let window = match state {
        Some(state) if state.pivot == Align2::RIGHT_TOP => window.pivot(Align2::RIGHT_TOP),
        Some(state) if state.size.is_some() => window
            .pivot(Align2::RIGHT_TOP)
            .current_pos(state.rect().right_top()),
        _ => window
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center()),
    };
    window.show(ctx, |ui| {
        let p = theme::palette(ui);
        let least_id = ui.id().with("least");
        let least = ui.data(|d| d.get_temp::<egui::Vec2>(least_id));
        ui.set_min_size(least.unwrap_or_default());
        let font = FontId::proportional(TITLE_SIZE);
        let galley = ui
            .painter()
            .layout_no_wrap(title.to_owned(), font, p.strong);
        // The title's room, between a button's each side, so it centres.
        let room = galley.size().x + 2.0 * TITLE_BAR + 2.0;
        let (bar, _) = ui.allocate_exact_size(vec2(room, TITLE_BAR), Sense::hover());
        let margin = egui::Margin::symmetric(12, 10);
        egui::Frame::new()
            .inner_margin(margin)
            .show(ui, |ui| asked.step = sector_text(ui, shown, right, p));
        let bar = Rect::from_min_size(bar.min, vec2(ui.min_rect().width(), TITLE_BAR));
        let named = RichText::new(title).size(TITLE_SIZE).color(p.strong);
        ui.put(bar.shrink2(vec2(TITLE_BAR, 0.0)), egui::Label::new(named));
        let line = Stroke::new(1.0, p.line);
        ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, line);
        let cross = Rect::from_center_size(
            egui::pos2(bar.right() - TITLE_BAR / 2.0, bar.center().y),
            vec2(18.0, 18.0),
        );
        let close = ui.interact(cross, ui.id().with("close"), Sense::click());
        close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close"));
        let colour = if close.hovered() {
            ui.painter().rect_filled(cross, 4.0, p.hover);
            p.strong
        } else {
            p.dim
        };
        let arm = cross.shrink(5.0);
        let stroke = Stroke::new(1.5, colour);
        ui.painter()
            .line_segment([arm.left_top(), arm.right_bottom()], stroke);
        ui.painter()
            .line_segment([arm.right_top(), arm.left_bottom()], stroke);
        asked.close = close.clicked();
        let size = ui.min_rect().size();
        ui.data_mut(|d| d.insert_temp(least_id, least.map_or(size, |l| l.max(size))));
    });
    asked
}

/// The sector window's text, to select and copy: what is said of the
/// sector, beside the way round the disk, then its data, which scrolls; in
/// a window whose right edge stays at `right` once measured. What the way
/// round the disk is asked this frame.
fn sector_text(ui: &mut egui::Ui, shown: &Shown, right: Option<f32>, p: &Palette) -> Option<Step> {
    let lines = shown.lines;
    let text = lines
        .iter()
        .map(|(line, _)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let body = egui::TextStyle::Body.resolve(ui.style());
    let plain = ui.visuals().text_color();
    let colours: Vec<Color32> = lines
        .iter()
        .map(|&(_, tone)| match tone {
            Tone::Strong => p.strong,
            Tone::Plain => plain,
            Tone::Weak => p.dim,
        })
        .collect();
    // Lines the type's size and the spacing between widgets apart, wrapped
    // only at `wrap`.
    let spaced = body.size + ui.spacing().item_spacing.y;
    let lay = |ui: &egui::Ui, text: &str, wrap: f32| {
        let mut job = egui::text::LayoutJob::default();
        for (i, line) in text.split('\n').enumerate() {
            let format = egui::TextFormat {
                font_id: body.clone(),
                color: colours.get(i).copied().unwrap_or(plain),
                line_height: Some(spaced),
                ..Default::default()
            };
            let line = if i == 0 {
                line.to_owned()
            } else {
                format!("\n{line}")
            };
            job.append(&line, 0.0, format);
        }
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    // Room for any sector the window can go to, so it keeps its size.
    let reserved = shown
        .nav
        .as_ref()
        .map(|nav| reserve(ui, nav.progress, &body))
        .unwrap_or_default();
    let natural = lay(ui, &text, f32::INFINITY).size().x.max(reserved.widest);
    let dumped = Dumped::kept(ui, ui.id().with("dumped"), shown.bytes, shown.base);
    let mono = FontId::monospace(DUMP_SIZE);
    let advance = ui.fonts_mut(|f| f.glyph_width(&mono, '0'));
    let full = match reserved.bytes {
        0..16 => 0,
        _ => Dumped::of(&[0; 16], shown.base).widest,
    };
    let rows_width = dumped.widest.max(full) as f32 * advance;
    // A text view's bars, as the Log's: beside the rows, not over them.
    theme::solid_bars(ui, p);
    let bar = ui.spacing().scroll.allocated_width();
    let beside = shown.nav.as_ref().map_or(0.0, |_| WAY_GAP + way_width(ui));
    // Beside the way round the disk, what is said wraps only where its longest
    // line would take the window past the app's, growing from its right edge.
    let app = ui.ctx().content_rect();
    let mut room = app.width() - 2.0 * APP_EDGE - WINDOW_EDGES - beside;
    if let Some(right) = right {
        room = room.min(right - app.left() - APP_EDGE - WINDOW_EDGES - beside);
    }
    let room = room.max(rows_width + bar - beside).max(SAID_LEAST);
    let (said_width, wrap) = match natural > room {
        true => (room, room),
        false => (natural, f32::INFINITY),
    };
    ui.set_min_width((said_width + beside).max(rows_width + bar).ceil() + 1.0);
    let mut layouter =
        |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _: f32| lay(ui, buffer.as_str(), wrap);
    let mut buffer = text.as_str();
    let said = egui::TextEdit::multiline(&mut buffer)
        .layouter(&mut layouter)
        .frame(egui::Frame::NONE)
        .margin(0)
        .desired_rows(1)
        .desired_width(said_width.ceil() + 1.0);
    let said_id = ui.id().with("said");
    // Another sector's text: none of it selected.
    let of = ui.id().with("said of");
    if ui.data(|d| d.get_temp::<String>(of)).as_deref() != Some(shown.title) {
        if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), said_id) {
            state.cursor.set_char_range(None);
            state.store(ui.ctx(), said_id);
        }
        ui.data_mut(|d| d.insert_temp(of, shown.title.to_owned()));
    }
    // What is said and the way round the disk, never shorter than they have
    // been, so the data below stays put.
    let top_id = ui.id().with("top");
    let least = ui.data(|d| d.get_temp::<f32>(top_id)).unwrap_or(0.0);
    let least = least.max(reserved.lines as f32 * spaced);
    let mut step = None;
    let top = ui.horizontal_top(|ui| {
        ui.set_min_height(least);
        form::read_only_box(ui, said_id, said);
        if let Some(nav) = &shown.nav {
            let layout = egui::Layout::right_to_left(egui::Align::Min);
            step = ui.with_layout(layout, |ui| way(ui, nav, p)).inner;
        }
    });
    let height = top.response.rect.height();
    ui.data_mut(|d| d.insert_temp(top_id, least.max(height)));
    let rows = dumped.rows.len().max(reserved.bytes.div_ceil(16));
    if rows == 0 {
        return step;
    }
    ui.separator();
    let row = ui.fonts_mut(|f| f.row_height(&mono));
    // At most BYTES_MOST, and as much as the app's height has room for.
    let fits = app.height() - 2.0 * APP_EDGE - TITLE_BAR - WINDOW_EDGES - least.max(height);
    let room = (rows as f32 * row).min(BYTES_MOST).min(fits.max(4.0 * row));
    let line = |i: usize| (&dumped.text[dumped.rows[i].clone()], plain);
    let lines = Lines {
        count: dumped.rows.len(),
        line: &line,
        font: mono,
        gap: 0.0,
    };
    ui.allocate_ui(vec2(ui.available_width(), room), |ui| {
        ui.set_min_height(room);
        if lines.count > 0 {
            let area = egui::ScrollArea::vertical().max_height(room);
            let pane = Pane {
                name: "Sector bytes",
                ..Pane::default()
            };
            lines.show_as(ui, ui.id().with("bytes"), area, pane);
        }
    });
    step
}

/// What the sector window keeps room for, whichever sector it goes to: the
/// widest line said of any sector gw reported, or of one gw did not find,
/// each digit as wide as the widest; the most lines; the most bytes.
#[derive(Clone, Copy, Debug, Default)]
struct Reserved {
    widest: f32,
    lines: usize,
    bytes: usize,
}

/// The room the sector window keeps for every sector of `progress`, said in
/// `font`, worked out again only once gw reports more.
fn reserve(ui: &egui::Ui, progress: &Progress, font: &FontId) -> Reserved {
    let id = egui::Id::new("disk sector room");
    if let Some((revision, reserved)) = ui.data(|d| d.get_temp::<(u64, Reserved)>(id))
        && revision == progress.revision
    {
        return reserved;
    }
    let width = |c: char| ui.fonts_mut(|f| f.glyph_width(font, c));
    let digit = ('0'..='9')
        .max_by(|&a, &b| width(a).total_cmp(&width(b)))
        .unwrap_or('0');
    let shape = |line: &str| -> String {
        line.chars()
            .map(|c| if c.is_ascii_digit() { digit } else { c })
            .collect()
    };
    let mut shapes = std::collections::HashSet::new();
    // A sector gw did not find: its name, whether its format lays it out
    // there, and gw's line on the track.
    let mut reserved = Reserved {
        lines: 3,
        ..Reserved::default()
    };
    for f in progress.facts.values().filter(|f| !f.absent) {
        shapes.extend(f.summary.as_deref().map(shape));
        for &id in &f.missing {
            shapes.extend(missing_lines(f, id, true).iter().map(|(l, _)| shape(l)));
        }
        for s in &f.sectors {
            let lines = sector_lines(s, &f.sectors);
            reserved.lines = reserved.lines.max(lines.len());
            reserved.bytes = reserved.bytes.max(s.bytes.len());
            shapes.extend(lines.iter().map(|(l, _)| shape(l)));
        }
    }
    reserved.widest = shapes
        .into_iter()
        .map(|t| {
            let laid = ui.fonts_mut(|f| f.layout_no_wrap(t, font.clone(), Color32::PLACEHOLDER));
            laid.size().x
        })
        .fold(0.0, f32::max);
    ui.data_mut(|d| d.insert_temp(id, (progress.revision, reserved)));
    reserved
}

/// The way round the disk's width: the steppers, the gap, and the ring
/// between its arrows.
fn way_width(ui: &egui::Ui) -> f32 {
    steppers_width(ui) + RING_GAP + 2.0 * ARROW + DIAL
}

/// The steppers' width: the longer name, its gap, and the value between
/// two arrows.
fn steppers_width(ui: &egui::Ui) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let name = ui.fonts_mut(|f| {
        f.layout_no_wrap("Cylinder".to_owned(), font, Color32::PLACEHOLDER)
            .size()
            .x
    });
    name.ceil() + NAME_GAP + 2.0 * ARROW + VALUE
}

/// The way round the disk, right of what is said: the cylinder and the side
/// each between arrows, and the track as a ring between the arrows round
/// it; the keyboard's arrows step round and across too. What it is asked
/// this frame.
fn way(ui: &mut egui::Ui, nav: &Nav, p: &Palette) -> Option<Step> {
    let column = steppers_width(ui);
    let size = vec2(way_width(ui), DIAL);
    let (cyl, side) = nav.open.key;
    let mut asked = Vec::new();
    let row = egui::Layout::left_to_right(egui::Align::Min);
    ui.allocate_ui_with_layout(size, row, |ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let steppers = egui::Layout::top_down(egui::Align::Max);
        ui.allocate_ui_with_layout(vec2(column, DIAL), steppers, |ui| {
            ui.add_space((DIAL - 2.0 * ARROW - ROW_GAP) / 2.0);
            let by = stepper(ui, "Cylinder", &cyl.to_string(), nav.cylinder, true, p);
            if by != 0 {
                asked.push(Step::Cylinder(by));
            }
            ui.add_space(ROW_GAP);
            if stepper(ui, "Side", &side.to_string(), nav.side, false, p) != 0 {
                asked.push(Step::Side);
            }
        });
        ui.add_space(RING_GAP);
        let beside = egui::Layout::top_down(egui::Align::Center);
        let mut by = 0;
        ui.allocate_ui_with_layout(vec2(ARROW, DIAL), beside, |ui| {
            ui.add_space((DIAL - ARROW) / 2.0);
            by -= arrow(ui, false, nav.round, true, "sector", p);
        });
        if let Some(i) = dial(ui, nav, p) {
            asked.push(Step::To(i));
        }
        ui.allocate_ui_with_layout(vec2(ARROW, DIAL), beside, |ui| {
            ui.add_space((DIAL - ARROW) / 2.0);
            by += arrow(ui, true, nav.round, true, "sector", p);
        });
        if by != 0 {
            asked.push(Step::Round(by));
        }
    });
    if keys_free(ui) {
        let none = egui::Modifiers::NONE;
        let [left, right, up, down] = ui.input_mut(|i| {
            [
                egui::Key::ArrowLeft,
                egui::Key::ArrowRight,
                egui::Key::ArrowUp,
                egui::Key::ArrowDown,
            ]
            .map(|key| i32::try_from(i.count_and_consume_key(none, key)).unwrap_or(0))
        });
        if nav.round && right != left {
            asked.push(Step::Round(right - left));
        }
        if nav.cylinder && down != up {
            asked.push(Step::Cylinder(down - up));
        }
    }
    asked.first().copied()
}

/// Whether the keyboard's arrows are the sector window's: nothing has the
/// keyboard, or something in the window has.
fn keys_free(ui: &egui::Ui) -> bool {
    ui.memory(|m| m.focused()).is_none_or(|id| {
        ui.ctx()
            .read_response(id)
            .is_some_and(|r| r.layer_id == ui.layer_id())
    })
}

/// A stepper's row from the right: its value between arrows, then its name.
/// How many steps it is asked for this frame, back as fewer than none.
fn stepper(
    ui: &mut egui::Ui,
    name: &str,
    value: &str,
    live: bool,
    repeats: bool,
    p: &Palette,
) -> i32 {
    let mut by = 0;
    let row = vec2(ui.available_width(), ARROW);
    let layout = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(row, layout, |ui| {
        let what = name.to_lowercase();
        by += arrow(ui, true, live, repeats, &what, p);
        let value = RichText::new(value).color(p.strong);
        ui.add_sized(vec2(VALUE, ARROW), egui::Label::new(value));
        by -= arrow(ui, false, live, repeats, &what, p);
        ui.add_space(NAME_GAP);
        ui.label(RichText::new(name).color(p.dim));
    });
    by
}

/// An arrow, ‹ or › as it steps back or `on`, greyed unless `live`, named for
/// the access tree as the previous or next `what`. How many steps it is
/// asked for this frame: one as it is pressed, then while it is held on it,
/// if it `repeats`, more and more often (held_steps).
fn arrow(ui: &mut egui::Ui, on: bool, live: bool, repeats: bool, what: &str, p: &Palette) -> i32 {
    // Its own drags, so a hand that moves as it holds the arrow does not
    // move the window.
    let sense = if live {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ARROW, ARROW), sense);
    let name = format!("{} {what}", if on { "Next" } else { "Previous" });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, live, &name));
    let id = response.id;
    let down = live && response.is_pointer_button_down_on();
    let held = ui.data(|d| d.get_temp::<Held>(id));
    let mut steps = 0;
    if down {
        let now = ui.input(|i| i.time);
        let Held { since, made } = held.unwrap_or(Held {
            since: now,
            made: 0,
        });
        let due = if repeats { held_steps(now - since) } else { 1 };
        // Held off the arrow, it waits, and goes on from there.
        if response.contains_pointer() {
            steps = due.saturating_sub(made);
        }
        ui.data_mut(|d| d.insert_temp(id, Held { since, made: due }));
        if repeats {
            ui.ctx().request_repaint();
        }
    } else {
        // A click with no press held before it: the keyboard's, the access
        // tree's, or a press and its release in one frame.
        if response.clicked() && held.is_none() {
            steps = 1;
        }
        ui.data_mut(|d| d.remove::<Held>(id));
    }
    let colour = if !live {
        theme::lerp(p.card, p.dim, 0.35)
    } else if response.hovered() || down {
        let fill = if down { p.line } else { p.hover };
        ui.painter().rect_filled(rect, 4.0, fill);
        p.strong
    } else {
        p.text
    };
    let c = rect.center();
    let x = if on { -2.5 } else { 2.5 };
    let points = vec![c + vec2(x, -5.0), c + vec2(-x, 0.0), c + vec2(x, 5.0)];
    ui.painter()
        .add(Shape::line(points, Stroke::new(1.5, colour)));
    i32::try_from(steps).unwrap_or(i32::MAX)
}

/// An arrow held down: since when, and the steps it has made.
#[derive(Clone, Copy, Debug)]
struct Held {
    since: f64,
    made: u32,
}

/// The steps an arrow held for `held` seconds has asked for: one as it was
/// pressed, another REPEAT_AFTER on, then REPEAT_LEAST a second, faster and
/// faster to REPEAT_MOST a second over REPEAT_RAMP seconds.
fn held_steps(held: f64) -> u32 {
    let t = held - REPEAT_AFTER;
    if t < 0.0 {
        return 1;
    }
    let ramp = t.min(REPEAT_RAMP);
    let gain = (REPEAT_MOST - REPEAT_LEAST) / REPEAT_RAMP;
    let more = REPEAT_LEAST * ramp + gain * ramp * ramp / 2.0 + REPEAT_MOST * (t - ramp);
    // A step due as `held` is reached counts, whatever the rounding.
    2 + (more + 1e-9) as u32
}

/// The open track as a ring, drawn as the disk view draws a track: the
/// index's notch at the top, the track running on clockwise, each sector gw
/// placed in its colour on the colour of the track's gaps, and the open one
/// outlined, the outline moving round to the next. From one track to another
/// the ring fades. The sector under the pointer stands out and is named in
/// the middle; a click on it asks for it, which this returns.
fn dial(ui: &mut egui::Ui, nav: &Nav, p: &Palette) -> Option<usize> {
    let f = nav.facts;
    let look = Look::of(p, Media::Fit);
    let (rect, response) = ui.allocate_exact_size(vec2(DIAL, DIAL), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Track"));
    let centre = rect.center();
    // The track's middle and its edges, with room above for the notch.
    let r = DIAL / 2.0 - DIAL_NOTCH - 1.0 - (DIAL_TRACK + DIAL_RAISE) / 2.0;
    let edges = (r + DIAL_TRACK / 2.0, r - DIAL_TRACK / 2.0);
    // A length round the track's middle, as a share of a revolution.
    let share = |points: f32| f64::from(points) / (TAU * f64::from(r));
    let hovered = response
        .hover_pos()
        .filter(|at| ((*at - centre).length() - r).abs() <= (DIAL_TRACK + DIAL_RAISE) / 2.0)
        .and_then(|at| {
            let d = at - centre;
            let pointed = share_at(f64::from(d.x), f64::from(d.y));
            under(&f.sectors, pointed, share(4.0))
        });
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let least = share(1.5);
    let arcs = f
        .sectors
        .iter()
        .filter_map(|s| Some((extent(s, least)?, look.status(s))))
        .collect();
    let ground = match Shortfall::of(f) {
        Shortfall::None => look.gap,
        Shortfall::Missing => look.missing_gap,
        Shortfall::Bad => look.bad_gap,
    };
    let (was, face, shown) = faded(ui, nav.open, Face { ground, arcs });
    let painter = ui.painter();
    let window = ui.visuals().window_fill;
    let paint = |face: &Face, strength: f32| {
        // epaint strokes a circle outside it.
        let ground = Stroke::new(DIAL_TRACK, face.ground.gamma_multiply(strength));
        painter.circle_stroke(centre, edges.1, ground);
        for &(span, colour) in &face.arcs {
            painter.add(arc(
                centre,
                r,
                span,
                DIAL_TRACK,
                colour.gamma_multiply(strength),
            ));
        }
        // Each sector's ends cut in the window's colour, so those that meet
        // show apart, and the gaps' colour only where gw found none.
        let cut = Stroke::new(1.0, window.gamma_multiply(strength));
        for &((from, to), _) in &face.arcs {
            for end in [from, to] {
                let across = [edges.0, edges.1].map(|r| point_at(centre, end, r));
                painter.line_segment(across, cut);
            }
        }
    };
    if let Some(was) = &was {
        paint(was, 1.0);
    }
    paint(&face, shown);
    if let Some(s) = hovered.map(|i| &f.sectors[i])
        && let Some(span) = extent(s, least)
    {
        painter.add(arc(
            centre,
            r,
            span,
            DIAL_TRACK + DIAL_RAISE,
            look.status(s),
        ));
    }
    let target = match nav.open.open {
        Open::Found(i) => f.sectors.get(i).and_then(|s| s.at),
        Open::Missing { .. } => None,
    };
    let target = target.map(|[a, _, b]| (f64::from(a), f64::from(b)));
    if let Some((span, lit)) = mark(ui.ctx(), nav.open, target) {
        let stroke = Stroke::new(1.5, look.ink.gamma_multiply(lit));
        painter.add(outline(centre, edges, widened(span, share(3.0)), stroke));
    }
    let tip = edges.0 + DIAL_RAISE / 2.0 + 1.0;
    painter.add(notch(centre, (tip, tip + DIAL_NOTCH), look.index));
    let (name, colour) = match (hovered, nav.open.open) {
        (Some(i), _) => (short_id(&f.sectors[i].id), p.text),
        (None, Open::Found(i)) => (
            f.sectors
                .get(i)
                .map_or_else(String::new, |s| short_id(&s.id)),
            p.strong,
        ),
        (None, Open::Missing { id, .. }) => (short_id(&id), p.partial),
    };
    let font = FontId::proportional(DIAL_NAME);
    painter.text(centre, Align2::CENTER_CENTER, name, font, colour);
    hovered.filter(|_| response.clicked())
}

/// Where share `share` of a revolution from the index lies, `r` from
/// `centre`, as `heading` points.
fn point_at(centre: Pos2, share: f64, r: f32) -> Pos2 {
    let (x, y) = heading(share);
    centre + vec2(x as f32, y as f32) * r
}

/// Points round `centre`, `r` from it, from share `from` to `to` of a
/// revolution from the index, in `steps` steps.
fn arc_points(
    centre: Pos2,
    r: f32,
    (from, to): (f64, f64),
    steps: usize,
) -> impl DoubleEndedIterator<Item = Pos2> {
    (0..=steps).map(move |i| point_at(centre, from + (to - from) * i as f64 / steps as f64, r))
}

/// How many steps a point long round `r` from the centre take from share
/// `from` to `to`, at least eight.
fn steps_round(r: f32, (from, to): (f64, f64)) -> usize {
    ((TAU * f64::from(r) * (to - from)).ceil() as usize).clamp(8, 2048)
}

/// A stroke `width` wide round `centre`, `r` from it, from share `from` to
/// `to` of a revolution from the index.
fn arc(centre: Pos2, r: f32, span: (f64, f64), width: f32, colour: Color32) -> Shape {
    let points = arc_points(centre, r, span, steps_round(r, span)).collect();
    Shape::line(points, Stroke::new(width, colour))
}

/// The outline of a track's part between its `edges`, outer and inner, from
/// share `from` to `to` of a revolution: half its width outside them, so it
/// touches what it rings.
fn outline(centre: Pos2, (outer, inner): (f32, f32), span: (f64, f64), stroke: Stroke) -> Shape {
    let half = stroke.width / 2.0;
    let (outer, inner) = (outer + half, (inner - half).max(0.0));
    let steps = steps_round(outer, span);
    let mut points: Vec<Pos2> = arc_points(centre, outer, span, steps).collect();
    points.extend(arc_points(centre, inner, span, steps).rev());
    Shape::closed_line(points, stroke)
}

/// The index's mark at the top of a disk or a track about `centre`: a notch
/// from its tip, `tip` from the centre, out to its base, `base` from it.
fn notch(centre: Pos2, (tip, base): (f32, f32), colour: Color32) -> Shape {
    let half = (base - tip) * 0.7;
    let points = vec![
        centre - vec2(0.0, tip),
        centre + vec2(half, -base),
        centre + vec2(-half, -base),
    ];
    Shape::convex_polygon(points, colour, Stroke::NONE)
}

/// How a track looks on the ring: the colour of its gaps, and each sector's
/// start, end and colour.
#[derive(Clone, Debug, Default)]
struct Face {
    ground: Color32,
    arcs: Vec<((f64, f64), Color32)>,
}

/// The ring as it goes from one track to another: the track it shows, how
/// it looks, how the last looked, and since when it has shown this one.
#[derive(Clone, Debug)]
struct Swap {
    key: (u32, u32),
    face: Face,
    was: Option<Face>,
    since: f64,
}

/// The ring's look for the open track, `face`, and for RING_SWAP seconds
/// after it goes to another track, the last's under it: how strongly the
/// new one shows over it.
fn faded(ui: &egui::Ui, open: Inspected, face: Face) -> (Option<Face>, Face, f32) {
    let id = egui::Id::new("disk sector ring").with(open.opened);
    let now = ui.input(|i| i.time);
    let swap = match ui.data(|d| d.get_temp::<Swap>(id)) {
        Some(swap) if swap.key == open.key => Swap { face, ..swap },
        Some(swap) => Swap {
            key: open.key,
            face,
            was: Some(swap.face),
            since: now,
        },
        None => Swap {
            key: open.key,
            face,
            was: None,
            since: f64::NEG_INFINITY,
        },
    };
    ui.data_mut(|d| d.insert_temp(id, swap.clone()));
    let t = ((now - swap.since) / RING_SWAP).clamp(0.0, 1.0);
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    (swap.was.filter(|_| t < 1.0), swap.face, t as f32)
}

/// The open sector's mark moving round its track: from where, to where,
/// each its start and end as shares of a revolution, and since when.
#[derive(Clone, Copy, Debug)]
struct Glide {
    from: [f64; 2],
    to: [f64; 2],
    since: f64,
}

impl Glide {
    /// Where the mark is at `now`, easing out to `to` over GLIDE seconds.
    fn at(&self, now: f64) -> [f64; 2] {
        let t = ((now - self.since) / GLIDE).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        [0, 1].map(|i| self.from[i] + (self.to[i] - self.from[i]) * eased)
    }

    /// Going to `to` from where it is at `now`: the shorter way round, unless
    /// it is going there already.
    fn toward(self, to: [f64; 2], now: f64) -> Glide {
        let turns = (self.to[0] - to[0]).round();
        if (0..2).all(|i| (to[i] + turns - self.to[i]).abs() < 1e-9) {
            return self;
        }
        let from = self.at(now);
        let turns = (from[0] - to[0]).round();
        Glide {
            from,
            to: to.map(|x| x + turns),
            since: now,
        }
    }
}

/// The mark of the sector open in the window opened `open.opened`, on its
/// ring and on the disk, moving to `to`, its start and end, and how
/// strongly it shows: where the open sector has no place, it fades where
/// it was.
fn mark(ctx: &egui::Context, open: Inspected, to: Option<(f64, f64)>) -> Option<((f64, f64), f32)> {
    let id = egui::Id::new("disk sector mark").with(open.opened);
    let now = ctx.input(|i| i.time);
    let lit = ctx.animate_bool_with_time(id.with("lit"), to.is_some(), MARK_FADE);
    let kept = ctx.data(|d| d.get_temp::<Glide>(id));
    let glide = match (kept, to.map(|(a, b)| [a, b])) {
        (Some(glide), Some(to)) => glide.toward(to, now),
        (None, Some(to)) => Glide {
            from: to,
            to,
            since: f64::NEG_INFINITY,
        },
        (Some(glide), None) => glide,
        (None, None) => return None,
    };
    ctx.data_mut(|d| d.insert_temp(id, glide));
    if now - glide.since < GLIDE {
        ctx.request_repaint();
    }
    let [from, to] = glide.at(now);
    (lit > 0.0).then_some(((from, to), lit))
}

/// A sector window's bytes as dump writes them out, and what of: kept while
/// the bytes and their numbering hold, not written again each frame.
struct Dumped {
    bytes: Vec<u8>,
    base: usize,
    text: String,
    /// Where each row lies in the text, and the most characters in one.
    rows: Vec<Range<usize>>,
    widest: usize,
}

impl Dumped {
    /// `bytes` numbered from `base` as dumped, kept under `id`.
    fn kept(ui: &egui::Ui, id: egui::Id, bytes: &[u8], base: usize) -> Arc<Dumped> {
        let kept = ui.data(|d| d.get_temp::<Arc<Dumped>>(id));
        if let Some(dumped) = kept.filter(|d| d.base == base && d.bytes == bytes) {
            return dumped;
        }
        let dumped = Arc::new(Dumped::of(bytes, base));
        ui.data_mut(|d| d.insert_temp(id, dumped.clone()));
        dumped
    }

    fn of(bytes: &[u8], base: usize) -> Dumped {
        let text = dump(bytes, usize::MAX, base);
        let mut rows = Vec::new();
        let mut at = 0;
        for row in text.lines() {
            rows.push(at..at + row.len());
            at += row.len() + 1;
        }
        let widest = rows.iter().map(|r| text[r.clone()].chars().count());
        Dumped {
            bytes: bytes.to_vec(),
            base,
            widest: widest.max().unwrap_or(0),
            text,
            rows,
        }
    }
}

/// Up to `rows` rows of 16 bytes: the offset, from `base`, the bytes in hex,
/// then as ASCII, others as dots.
fn dump(bytes: &[u8], rows: usize, base: usize) -> String {
    use std::fmt::Write as _;
    // As many hex digits as the last offset needs, and at least four.
    let last = base + bytes.len().saturating_sub(1);
    let digits = (usize::BITS - last.leading_zeros()).div_ceil(4).max(4) as usize;
    let ascii = |&b: &u8| match b {
        32..127 => b as char,
        _ => '.',
    };
    // Each row its offset, 51 characters of hex and its ASCII, and a newline.
    let mut out = String::with_capacity(bytes.len().div_ceil(16).min(rows) * (digits + 68));
    for (row, chunk) in bytes.chunks(16).take(rows).enumerate() {
        if row > 0 {
            out.push('\n');
        }
        // Writing to a String cannot fail.
        let _ = write!(out, "{:0digits$X} ", base + row * 16);
        for b in chunk {
            let _ = write!(out, " {b:02X}");
        }
        // The hex as wide as 16 bytes', then two spaces.
        out.extend(std::iter::repeat_n(' ', 50 - 3 * chunk.len()));
        out.extend(chunk.iter().map(ascii));
    }
    out
}

/// The sector drawn at `share` of a revolution, as the track is painted with
/// lines `least` long: those shorter than a line lie over the rest, a line
/// long, the longest of them on top; under them, the last of the sectors
/// laid there.
fn under(sectors: &[Sector], share: f64, least: f64) -> Option<usize> {
    let length = |s: &Sector| {
        s.at.map_or(f64::INFINITY, |[a, _, b]| f64::from(b) - f64::from(a))
    };
    let mut short: Vec<usize> = (0..sectors.len())
        .filter(|&i| length(&sectors[i]) < least)
        .collect();
    short.sort_by(|&a, &b| length(&sectors[a]).total_cmp(&length(&sectors[b])));
    let on = |&i: &usize| holds(&sectors[i], share, least);
    short
        .into_iter()
        .rev()
        .find(on)
        .or_else(|| (0..sectors.len()).rev().find(on))
}

fn holds(s: &Sector, share: f64, least: f64) -> bool {
    extent(s, least).is_some_and(|(start, end)| {
        [share - 1.0, share, share + 1.0]
            .iter()
            .any(|x| (start..end).contains(x))
    })
}

/// Where a sector is drawn, in shares of a revolution: where it was found,
/// or one shorter than `least`, that much about its middle.
fn extent(s: &Sector, least: f64) -> Option<(f64, f64)> {
    let [start, _, end] = s.at?.map(f64::from);
    Some(widened((start, end), least))
}

/// From `from` to `to`, or `least` long round its middle where shorter.
fn widened((from, to): (f64, f64), least: f64) -> (f64, f64) {
    match to - from >= least {
        true => (from, to),
        false => ((from + to - least) / 2.0, (from + to + least) / 2.0),
    }
}

pub(crate) fn id_text(id: &Id) -> String {
    match id {
        Id::Ibm([c, h, r, n]) => format!("C{c} H{h} R{r} N{n}"),
        Id::Number(n) => format!("Sector {n}"),
        Id::None => "Data block".to_owned(),
    }
}

/// A sector's ID as the list of those missing names it: its R, or its number.
fn short_id(id: &Id) -> String {
    match id {
        Id::Ibm([.., r, _]) => format!("R{r}"),
        Id::Number(n) => n.to_string(),
        Id::None => "No ID".to_owned(),
    }
}

/// How strongly a line of what is said of a sector shows.
#[derive(Clone, Copy)]
pub(crate) enum Tone {
    Strong,
    Plain,
    Weak,
}

/// What is said of a sector of a track's `sectors`, line by line: its ID
/// and size, its checks and mark, its place, in degrees and in bytes, how gw
/// found it in each revolution, any other sector with its ID, and notes.
fn sector_lines(s: &Sector, sectors: &[Sector]) -> Vec<(String, Tone)> {
    let size = match (s.id, s.data) {
        _ if !s.bytes.is_empty() => Some(s.bytes.len() as u32),
        // A header alone has no data: its N is in its ID.
        (_, Data::None) => None,
        // The data its header calls for, as gw's decoder reads it.
        (Id::Ibm([.., n]), _) if n <= 7 => Some(128u32 << n),
        _ => None,
    };
    let name = match s.id {
        Id::Ibm(_) => format!("Sector {}", id_text(&s.id)),
        Id::Number(_) | Id::None => id_text(&s.id),
    };
    let mut lines = vec![(
        match size {
            Some(size) => format!("{name} · {size} bytes"),
            None => name,
        },
        Tone::Strong,
    )];
    let mut checks = match (s.id, s.header, s.data) {
        // gw adds a sector by number only once its checks pass; some such
        // codecs have no header, or no check of it apart from the data's.
        (Id::Number(_), Header::Good, Data::Good) => vec!["Checks OK".to_owned()],
        (Id::Number(_), Header::Good, Data::Empty(b)) => vec![format!("Checks OK, all {b:02X}")],
        _ => vec![
            match s.header {
                Header::Good => "Header OK",
                Header::Bad => "Header bad",
                Header::None => "No header",
            }
            .to_owned(),
            match s.data {
                Data::Good => "Data OK".to_owned(),
                Data::Empty(b) => format!("Data OK, all {b:02X}"),
                Data::Bad => "Data bad".to_owned(),
                Data::Unread => "Data unread".to_owned(),
                Data::None => "No data".to_owned(),
            },
        ],
    };
    if let Some(mark) = s.mark {
        checks.push(format!(
            "Mark {mark:02X}{}",
            if deleted(s) { " (deleted)" } else { "" }
        ));
    }
    lines.push((checks.join(" · "), Tone::Plain));
    let deg = |x: f32| x * 360.0;
    if let Some([start, data, end]) = s.at {
        let mut place = format!("{:.1}°–{:.1}°", deg(start), deg(end));
        // A header with no data after it has none to place.
        if data > start && s.data != Data::None {
            place += &format!(" · data {:.1}°", deg(data));
            if let Some(cells) = s.layout.and_then(|l| l.id_to_data) {
                place += &format!(", {} after the ID", bytes(cells));
            }
        }
        lines.push((place, Tone::Weak));
    }
    if let Some(layout) = &s.layout {
        lines.push((layout_line(layout), Tone::Weak));
    }
    if let Some(turns) = s.turns.as_ref().and_then(turns_line) {
        lines.push((turns, Tone::Plain));
    }
    let twins: Vec<String> = sectors
        .iter()
        .filter(|t| !std::ptr::eq(*t, s) && same_id(s, t))
        .filter_map(|t| t.at.map(|[start, ..]| format!("{:.1}°", deg(start))))
        .collect();
    if !twins.is_empty() {
        lines.push((format!("Its ID also at {}", twins.join(", ")), Tone::Plain));
    }
    let notes: Vec<&str> = [
        (s.extra, "Not in the format"),
        (s.before, "Read before the first index"),
    ]
    .into_iter()
    .filter_map(|(on, note)| on.then_some(note))
    .collect();
    if !notes.is_empty() {
        lines.push((notes.join(" · "), Tone::Weak));
    }
    lines
}

/// Whether a sector's ID is one gw read from a header whose CRC holds.
fn sure_id(s: &Sector) -> bool {
    s.header == Header::Good && matches!(s.id, Id::Ibm(_))
}

/// Whether two sectors carry the same ID, each sure.
fn same_id(a: &Sector, b: &Sector) -> bool {
    sure_id(a) && sure_id(b) && a.id == b.id
}

/// Bit cells as bytes, 16 cells to a byte, as gw's FM and MFM decoders
/// count them: whole bytes, and any cells over.
fn bytes(cells: f64) -> String {
    let cells = cells.round() as i64;
    let (whole, over) = (cells.abs() / 16, cells.abs() % 16);
    let unit = if whole == 1 { "byte" } else { "bytes" };
    match over {
        0 => format!("{} {unit}", grouped(whole as u64)),
        1 => format!("{} {unit} 1 cell", grouped(whole as u64)),
        _ => format!("{} {unit} {over} cells", grouped(whole as u64)),
    }
}

/// Where a sector lies in bytes: from the index, and after what gw found
/// before it, its ID said to be a bad header's where that one's CRC fails.
fn layout_line(l: &Layout) -> String {
    let mut parts = vec![format!("{} from the index", bytes(l.from_index))];
    if let Some((cells, before)) = l.after {
        let what = match before {
            Before::IndexMark => "the index mark".to_owned(),
            Before::Sector(id, true) => short_id(&id),
            Before::Sector(id, false) => format!("{} (bad header)", short_id(&id)),
            Before::Header(id, true) => format!("{}'s header", short_id(&id)),
            Before::Header(id, false) => format!("{}'s bad header", short_id(&id)),
        };
        parts.push(match cells < 0.0 {
            true => format!("into {what} by {}", bytes(-cells)),
            false => format!("{} after {what}", bytes(cells)),
        });
    }
    parts.join(" · ")
}

/// How gw found a sector in each revolution read: how often each way, the
/// best first, as `Good in 2 of 3 revolutions · data bad in 1`.
fn turns_line(t: &Turns) -> Option<String> {
    let all = t.seen.len();
    let ways = [
        (Seen::Good, "good"),
        (Seen::BadData, "data bad"),
        (Seen::BadHeader, "header bad"),
        (Seen::HeaderAlone, "no data"),
        (Seen::BadHeaderAlone, "header bad, no data"),
        (Seen::DataAlone, "no header"),
        (Seen::NotFound, "not found"),
    ];
    let mut parts = ways.iter().filter_map(|&(way, name)| {
        let n = t.seen.iter().filter(|&&s| s == way).count();
        (n > 0).then(|| format!("{name} in {n}"))
    });
    let first = parts.next()?;
    let noun = if all == 1 {
        "revolution"
    } else {
        "revolutions"
    };
    let reads = match t.reads {
        1 => String::new(),
        n => format!(" over {n} reads"),
    };
    // Each name starts with a lower-case ASCII letter.
    let first = format!(
        "{}{} of {all} {noun}{reads}",
        first[..1].to_uppercase(),
        &first[1..]
    );
    Some(
        std::iter::once(first)
            .chain(parts)
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

/// A sector of a track's `sectors`: what is said of it, and its data's
/// first rows.
fn sector_tip(ui: &mut egui::Ui, s: &Sector, sectors: &[Sector]) {
    for (text, tone) in sector_lines(s, sectors) {
        match tone {
            Tone::Strong => ui.strong(text),
            Tone::Plain => ui.label(text),
            Tone::Weak => ui.weak(text),
        };
    }
    if !s.bytes.is_empty() {
        let rows = RichText::new(dump(&s.bytes, 4, 0)).font(FontId::monospace(DUMP_SIZE));
        ui.add(egui::Label::new(rows).extend());
    }
}

/// The disks' legend, laid out: its entries, each where it lies from the
/// legend's top left, and its height, the room above it with it; or to
/// scale, in place of the entries, why the disk holds none of the tracks.
struct Legend {
    entries: Vec<Entry>,
    at: Vec<egui::Vec2>,
    holds: Option<(String, Option<Arc<Galley>>)>,
    /// Its rows' height, its text's and its marks', and its own; the widest
    /// of its rows.
    row: f32,
    height: f32,
    width: f32,
}

/// A legend entry: a word before its mark, as the flux scale's "Less", its
/// mark and its text, kept on one line, and what it says on a hover; with
/// no mark, a note, its text as laid out where it lies in its row.
struct Entry {
    lead: Option<Arc<Galley>>,
    mark: Option<Mark>,
    text: Arc<Galley>,
    tip: Option<String>,
    width: f32,
    note: Option<Arc<Galley>>,
}

impl Legend {
    /// The key to what the disks show, `drawn`: how their sectors decoded, or
    /// the grid's statuses, or the flux's shading; what else is drawn; then
    /// the counts of sectors missing, retries and shared IDs. With `holds`,
    /// why nothing is drawn.
    fn of(ui: &egui::Ui, map: &Map, drawn: &Drawn, look: &Look, holds: Option<String>) -> Legend {
        let mut legend = Legend {
            entries: Vec::new(),
            at: Vec::new(),
            holds: holds.map(|text| (text, None)),
            row: 0.0,
            height: 0.0,
            width: 0.0,
        };
        if legend.holds.is_some() {
            return legend;
        }
        let p = theme::palette(ui);
        let progress = map.progress;
        let font = egui::TextStyle::Small.resolve(ui.style());
        let (strong, weak) = (ui.visuals().text_color(), ui.visuals().weak_text_color());
        let galley = |text: String, colour| ui.painter().layout_no_wrap(text, font.clone(), colour);
        let gap = ui.spacing().item_spacing.x;
        let entry = |mark: Option<Mark>, text: String, tip: Option<&str>| {
            let text = galley(text, if mark.is_some() { strong } else { weak });
            let marked = mark.map_or(0.0, |m| m.width() + gap);
            Entry {
                lead: None,
                mark,
                width: marked + text.size().x,
                text,
                tip: tip.map(str::to_owned),
                note: None,
            }
        };
        let entries = &mut legend.entries;
        match drawn.shows {
            Shows::Sectors if drawn.pure => {
                let statuses = diskmap::entries(&drawn.statuses, progress, map.verifying, p);
                for (colour, skipped, name, tracks, tip) in statuses {
                    let mark = match skipped {
                        true => Mark::Hole(colour, p.line_strong),
                        false => Mark::Swatch(colour),
                    };
                    let text = tracks.map_or(name.to_owned(), |n| {
                        format!("{name} {}", diskmap::tracks(n))
                    });
                    entries.push(entry(Some(mark), text, Some(tip)));
                }
            }
            Shows::Sectors => {
                // An incomplete sector: a header alone, all ID field, or
                // data alone; as the sectors drawn are.
                let (header, data) = (look.id(look.alone), look.alone);
                let incomplete = match (drawn.headers_alone, drawn.data_alone) {
                    (true, true) => Mark::Split(header, data),
                    (true, false) => Mark::Swatch(header),
                    _ => Mark::Swatch(data),
                };
                for (class, n) in Class::ALL.into_iter().zip(drawn.sectors) {
                    let mark = match class {
                        Class::Good => Mark::Swatch(look.good),
                        Class::Empty => Mark::Swatch(look.empty),
                        Class::Deleted => Mark::Swatch(look.deleted),
                        Class::BadData => Mark::Swatch(look.bad),
                        Class::BadHeader => Mark::Swatch(look.bad_header),
                        Class::Incomplete => incomplete,
                    };
                    if n > 0 {
                        let text = format!("{} {n}", class.name());
                        entries.push(entry(Some(mark), text, class.tip()));
                    }
                }
                if drawn.id_fields {
                    let mark = Mark::Swatch(look.id(look.good));
                    entries.push(entry(Some(mark), "ID field".into(), None));
                }
                if drawn.meet {
                    let mark = Mark::Line(look.good, look.body);
                    entries.push(entry(Some(mark), "Sectors meet".into(), None));
                }
                if drawn.gaps {
                    let mark = Mark::Swatch(look.gap);
                    entries.push(entry(Some(mark), "No sector found".into(), None));
                }
                for (n, colour, name, tip) in [
                    (
                        drawn.missing_tracks,
                        look.missing_gap,
                        "Sectors missing",
                        "Where gw found no sector, on a track with sectors missing.",
                    ),
                    (
                        drawn.bad_tracks,
                        look.bad_gap,
                        "Bad",
                        "Where gw found no sector, on a track with none decoded.",
                    ),
                ] {
                    if n > 0 {
                        let text = format!("{name} {}", diskmap::tracks(n));
                        entries.push(entry(Some(Mark::Swatch(colour)), text, Some(tip)));
                    }
                }
                if drawn.flux > 0 {
                    let mark = Mark::Swatch(look.flux);
                    let tip = Some("Flux, not decoded.");
                    let text = format!("Flux {}", diskmap::tracks(drawn.flux));
                    entries.push(entry(Some(mark), text, tip));
                }
            }
            Shows::Flux => {
                let shades = [0.0, 0.5, 1.0, 1.5, 2.0].map(|d| look.flux_at(d));
                let tip = Some("Against the track's average.");
                let mut scale = entry(Some(Mark::Shades(shades)), "More flux".into(), tip);
                let less = galley("Less".into(), strong);
                scale.width += less.size().x + gap;
                scale.lead = Some(less);
                entries.push(scale);
            }
        }
        if drawn.unknown > 0 {
            let tip = match drawn.shows {
                Shows::Sectors => "No sector places reported.",
                Shows::Flux => "No flux reported.",
            };
            let mark = Mark::Swatch(look.unknown);
            entries.push(entry(
                Some(mark),
                format!("Not known {}", diskmap::tracks(drawn.unknown)),
                Some(tip),
            ));
        }
        let index = match drawn.holes {
            true => "Sector 0's hole",
            false => "Index",
        };
        entries.push(entry(Some(Mark::Index(look.index)), index.into(), None));
        if drawn.to_do > 0 {
            let mark = Mark::Swatch(look.to_do);
            let text = format!("To do {}", diskmap::tracks(drawn.to_do));
            entries.push(entry(Some(mark), text, None));
        }
        if map.current.is_some() {
            entries.push(entry(
                Some(Mark::Ring(look.last)),
                "Last reported".into(),
                None,
            ));
        }
        if drawn.missing > 0 && drawn.shows == Shows::Sectors && !drawn.pure {
            let tip = Some("In the format, not found.");
            entries.push(entry(None, format!("{} missing", drawn.missing), tip));
        }
        let retries = progress.tally().retries;
        if retries > 0 {
            entries.push(entry(None, diskmap::retry_text(retries), None));
        }
        if drawn.shared > 0 && drawn.shows == Shows::Sectors && !drawn.pure {
            let tip = Some("Sectors of one track with the same C, H, R and N.");
            entries.push(entry(None, format!("{} share an ID", drawn.shared), tip));
        }
        legend
    }

    /// The marks `of` gives in the tracks' own colours, as the tracks show
    /// them by `look`: see Look::seen.
    fn see(&mut self, look: &Look) {
        for e in &mut self.entries {
            e.mark = e.mark.map(|m| m.seen(look));
        }
    }

    /// Takes the index's entry out: whether there was one.
    fn drop_index(&mut self) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|e| !matches!(e.mark, Some(Mark::Index(_))));
        self.entries.len() < before
    }

    /// Lays the legend out `width` wide, each entry after the last, or where
    /// it would not fit, on the next row; and gives its height.
    fn flow(&mut self, ui: &egui::Ui, width: f32) -> f32 {
        if let Some((text, galley)) = &mut self.holds {
            let p = theme::palette(ui);
            let font = egui::TextStyle::Body.resolve(ui.style());
            let laid = ui.painter().layout(text.clone(), font, p.bad, width);
            self.height = LEGEND_GAP + laid.size().y;
            self.width = laid.size().x;
            *galley = Some(laid);
            return self.height;
        }
        let spacing = ui.spacing().item_spacing;
        let texts = self
            .entries
            .iter()
            .flat_map(|e| e.lead.iter().chain([&e.text]));
        let small = ui.text_style_height(&egui::TextStyle::Small);
        let row = texts.fold(small.max(10.0), |row, t| row.max(t.size().y));
        self.row = row;
        let (mut x, mut y) = (0.0, 0.0);
        self.at.clear();
        self.width = 0.0;
        for e in &mut self.entries {
            if x > 0.0 && x + e.width > width {
                (x, y) = (0.0, y + row + spacing.y);
            }
            self.at.push(vec2(x, y));
            self.width = self.width.max(x + e.width);
            // A note, as a label in a row of them lays its text out: from
            // the row's start, after as much room as lies before it.
            if e.mark.is_none() {
                let mut job = egui::text::LayoutJob::simple_singleline(
                    e.text.text().to_owned(),
                    egui::TextStyle::Small.resolve(ui.style()),
                    ui.visuals().weak_text_color(),
                );
                job.first_row_min_height = row;
                job.sections[0].leading_space = x;
                job.sections[0].format.valign = ui.text_valign();
                e.note = Some(ui.fonts_mut(|f| f.layout_job(job)));
            }
            // Room after a marked entry, as between a legend's keys.
            let after = if e.mark.is_some() { LEGEND_GAP } else { 0.0 };
            x += e.width + spacing.x + after;
        }
        self.height = match self.entries.is_empty() {
            true => 0.0,
            false => LEGEND_GAP + y + row,
        };
        self.height
    }

    /// Draws the legend as laid out, from `at`, the room above it first.
    fn draw(&self, ui: &mut egui::Ui, at: Pos2) {
        let top = at + vec2(0.0, LEGEND_GAP);
        let id = ui.id().with("disk legend");
        let mut bounds = Rect::from_min_size(at, vec2(0.0, self.height));
        // Each piece of text where it lies, a label to a hover, with its tip.
        let said =
            |ui: &egui::Ui, galley: &Arc<Galley>, rect: Rect, id: egui::Id, tip: Option<&str>| {
                ui.painter()
                    .galley(rect.min, galley.clone(), Color32::PLACEHOLDER);
                let response = ui.interact(rect, id, Sense::hover());
                let (kind, text) = (egui::WidgetType::Label, galley.text());
                response.widget_info(|| egui::WidgetInfo::labeled(kind, true, text));
                if let Some(tip) = tip {
                    response.on_hover_text(tip);
                }
            };
        if let Some((_, Some(galley))) = &self.holds {
            let rect = Rect::from_min_size(top, galley.size());
            said(ui, galley, rect, id, None);
            bounds = bounds.union(rect);
        }
        let (row, gap) = (self.row, ui.spacing().item_spacing.x);
        for (i, (e, &offset)) in self.entries.iter().zip(&self.at).enumerate() {
            let rect = Rect::from_min_size(top + offset, vec2(e.width, row));
            bounds = bounds.union(rect);
            let middle = rect.center().y;
            let mut x = rect.left();
            if let Some(lead) = &e.lead {
                let at = egui::pos2(x, middle - lead.size().y / 2.0);
                let lead_rect = Rect::from_min_size(at, lead.size());
                said(ui, lead, lead_rect, id.with((i, "lead")), None);
                x = lead_rect.right() + gap;
            }
            // The entry answers a hover over its mark and its text.
            let from = x;
            if let Some(mark) = e.mark {
                let at = egui::pos2(x, middle - 5.0);
                mark.paint(
                    ui.painter(),
                    Rect::from_min_size(at, vec2(mark.width(), 10.0)),
                );
                x += mark.width() + gap;
            }
            match e.mark {
                Some(_) => {
                    let at = egui::pos2(x, middle - e.text.size().y / 2.0);
                    ui.painter()
                        .galley(at, e.text.clone(), Color32::PLACEHOLDER);
                }
                // A note, from its row's start: see flow.
                None => {
                    if let Some(note) = &e.note {
                        let start = egui::pos2(top.x, rect.top());
                        ui.painter()
                            .galley(start, note.clone(), Color32::PLACEHOLDER);
                    }
                }
            }
            let whole = Rect::from_x_y_ranges(from..=x + e.text.size().x, rect.y_range());
            let response = ui.interact(whole, id.with(i), Sense::hover());
            let (kind, text) = (egui::WidgetType::Label, e.text.text());
            response.widget_info(|| egui::WidgetInfo::labeled(kind, true, text));
            if let Some(tip) = &e.tip {
                response.on_hover_text(tip);
            }
        }
        ui.advance_cursor_after_rect(bounds);
    }
}

/// How a sector decoded, as the legend lists them, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Good,
    /// Good, every byte the same.
    Empty,
    /// Good, its mark calling its data deleted.
    Deleted,
    /// Its header's CRC holds, its data's fails.
    BadData,
    /// Its header's CRC fails: its ID is not known for sure.
    BadHeader,
    /// A header with no data, or data with no header.
    Incomplete,
}

impl Class {
    const ALL: [Class; 6] = [
        Class::Good,
        Class::Empty,
        Class::Deleted,
        Class::BadData,
        Class::BadHeader,
        Class::Incomplete,
    ];

    /// Its name in the legend and the side's sums.
    fn name(self) -> &'static str {
        match self {
            Class::Good => "Good",
            Class::Empty => "Empty",
            Class::Deleted => "Deleted",
            Class::BadData => "Bad data",
            Class::BadHeader => "Bad header",
            Class::Incomplete => "Incomplete",
        }
    }

    /// What its legend entry says on a hover.
    fn tip(self) -> Option<&'static str> {
        match self {
            Class::Good => None,
            Class::Empty => Some("Every byte the same."),
            Class::Deleted => Some("Data mark F8, or F9 on a DEC RX02."),
            Class::BadData => Some("The data's CRC fails."),
            Class::BadHeader => Some("The header's CRC fails."),
            Class::Incomplete => Some("A header with no data, or data with no header."),
        }
    }

    fn of(s: &Sector) -> Class {
        match (s.header, s.data) {
            (Header::None, _) | (_, Data::None) => Class::Incomplete,
            (Header::Bad, _) => Class::BadHeader,
            (_, Data::Bad) => Class::BadData,
            _ if deleted(s) => Class::Deleted,
            (_, Data::Empty(_)) => Class::Empty,
            (_, Data::Good | Data::Unread) => Class::Good,
        }
    }
}

/// Whether a sector's data mark calls its data deleted: gw's IBM deleted
/// data mark, F8, or a DEC RX02's for double density, F9.
fn deleted(s: &Sector) -> bool {
    matches!(s.mark, Some(0xf8 | 0xf9))
}

/// Whether any two of `sectors` meet, one starting where another ends.
fn meet(sectors: &[Sector]) -> bool {
    let edges: Vec<(f64, f64)> = sectors
        .iter()
        .filter_map(|s| s.at.map(|a| a.map(f64::from)))
        .map(|[start, _, end]| (start.rem_euclid(1.0), end.rem_euclid(1.0)))
        .collect();
    !meets(&edges).is_empty()
}

/// Where, of sectors that start and end at `edges` round a track, one
/// starts where another ends, as EXACT and MEET tell: never where it ends
/// itself.
fn meets(edges: &[(f64, f64)]) -> Vec<f64> {
    let within = |near: f64| {
        let ends = move |i: usize| edges.iter().enumerate().filter(move |&(j, _)| j != i);
        let starts = edges.iter().enumerate();
        starts
            .filter(move |&(i, &(b, _))| ends(i).any(|(_, &(_, e))| apart(b, e) < near))
            .map(|(_, &(b, _))| b)
    };
    let near = if within(EXACT).next().is_some() {
        MEET
    } else {
        EXACT
    };
    let mut meets: Vec<f64> = within(near).collect();
    meets.sort_by(f64::total_cmp);
    meets.dedup_by(|a, b| (*a - *b).abs() < MEET);
    meets
}

/// How a legend entry shows what it names.
#[derive(Clone, Copy)]
pub(crate) enum Mark {
    /// A square of the colour.
    Swatch(Color32),
    /// The colours side by side, as a scale.
    Shades([Color32; 5]),
    /// A line across a sector's colour, as where two sectors meet.
    Line(Color32, Color32),
    /// A square of one colour, then another, as an ID field and data.
    Split(Color32, Color32),
    /// A square outlined, as a track gw passed over.
    Hole(Color32, Color32),
    /// The index's notch.
    Index(Color32),
    /// A ring, as round the last track reported.
    Ring(Color32),
    /// A frame, as round the image's track last reported.
    Frame(Color32),
}

impl Mark {
    /// Its width; its height is 10 points.
    fn width(self) -> f32 {
        match self {
            Mark::Shades(_) => 40.0,
            _ => 10.0,
        }
    }

    /// Its colours of the tracks as they show across them by `look`, as the
    /// disks' legend shows them: see Look::seen. Its lines' and outlines'
    /// as they are.
    fn seen(self, look: &Look) -> Mark {
        let seen = |colour| look.seen(colour);
        match self {
            Mark::Swatch(colour) => Mark::Swatch(seen(colour)),
            Mark::Shades(colours) => Mark::Shades(colours.map(seen)),
            Mark::Line(fill, colour) => Mark::Line(seen(fill), colour),
            Mark::Split(first, then) => Mark::Split(seen(first), seen(then)),
            Mark::Hole(fill, edge) => Mark::Hole(seen(fill), edge),
            Mark::Index(_) | Mark::Ring(_) | Mark::Frame(_) => self,
        }
    }

    /// Paints it in `square`.
    fn paint(self, painter: &egui::Painter, square: Rect) {
        match self {
            Mark::Swatch(colour) => {
                painter.rect_filled(square, 2.0, colour);
            }
            Mark::Shades(colours) => {
                let step = square.width() / colours.len() as f32;
                for (i, colour) in colours.into_iter().enumerate() {
                    let x = square.left() + step * i as f32;
                    let part = Rect::from_min_size(egui::pos2(x, square.top()), vec2(step, 10.0));
                    painter.rect_filled(part, 0.0, colour);
                }
            }
            Mark::Split(first, then) => {
                let (left, right) = square.split_left_right_at_fraction(0.5);
                let round = |west: u8, east: u8| egui::CornerRadius {
                    nw: west,
                    sw: west,
                    ne: east,
                    se: east,
                };
                painter.rect_filled(left, round(2, 0), first);
                painter.rect_filled(right, round(0, 2), then);
            }
            Mark::Hole(fill, edge) => {
                let edge = Stroke::new(1.0, edge);
                painter.rect(square, 2.0, fill, edge, egui::StrokeKind::Inside);
            }
            Mark::Line(fill, colour) => {
                painter.rect_filled(square, 2.0, fill);
                let x = square.center().x;
                let stroke = Stroke::new(1.5, colour);
                painter.line_segment(
                    [egui::pos2(x, square.top()), egui::pos2(x, square.bottom())],
                    stroke,
                );
            }
            Mark::Index(colour) => {
                let (t, c) = (square.top() + 1.0, square.center().x);
                let points = vec![
                    egui::pos2(c - 5.0, t),
                    egui::pos2(c + 5.0, t),
                    egui::pos2(c, t + 7.0),
                ];
                painter.add(Shape::convex_polygon(points, colour, Stroke::NONE));
            }
            Mark::Ring(colour) => {
                painter.circle_stroke(square.center(), 4.0, Stroke::new(1.5, colour));
            }
            Mark::Frame(colour) => {
                let frame = Rect::from_center_size(square.center(), vec2(8.0, 8.0));
                painter.rect_stroke(
                    frame,
                    0.0,
                    Stroke::new(1.5, colour),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}

/// A legend entry in a wrapping row: its mark and its text, kept on one line.
pub(crate) fn key(ui: &mut egui::Ui, mark: Mark, text: &str) -> egui::Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, ui.visuals().text_color());
    let gap = ui.spacing().item_spacing.x;
    let size = vec2(
        mark.width() + gap + galley.size().x,
        galley.size().y.max(10.0),
    );
    if ui.available_width() < size.x {
        ui.end_row();
    }
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    let square = Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - 5.0),
        vec2(mark.width(), 10.0),
    );
    mark.paint(ui.painter(), square);
    let at = egui::pos2(
        square.right() + gap,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(at, galley, ui.visuals().text_color());
    ui.add_space(LEGEND_GAP);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_round_the_track_goes_on_from_the_last_sector_to_the_first() {
        let found = Open::Found;
        assert_eq!(round(found(0), 10, 1), Some(1));
        assert_eq!(round(found(9), 10, 1), Some(0));
        assert_eq!(round(found(0), 10, -1), Some(9));
        assert_eq!(round(found(3), 10, 25), Some(8));
        assert_eq!(round(found(0), 1, 1), None, "no other sector");
        let missing = Open::Missing {
            id: Id::Number(3),
            laid: true,
        };
        assert_eq!(round(missing, 10, 1), Some(0), "on from the index");
        assert_eq!(round(missing, 10, -1), Some(9), "back from it");
        assert_eq!(round(missing, 0, 1), None);
    }

    #[test]
    fn a_step_across_goes_on_from_the_last_cylinder_gw_reported_to_the_first() {
        let cyls = [0, 1, 2, 5, 79];
        assert_eq!(next_cylinder(&cyls, 0, 1), Some(1));
        assert_eq!(
            next_cylinder(&cyls, 2, 1),
            Some(5),
            "past those not reported"
        );
        assert_eq!(next_cylinder(&cyls, 79, 1), Some(0));
        assert_eq!(next_cylinder(&cyls, 0, -1), Some(79));
        assert_eq!(next_cylinder(&cyls, 1, 8), Some(79));
        assert_eq!(next_cylinder(&[4], 4, 1), None, "no other");
        assert_eq!(next_cylinder(&cyls, 3, 1), None, "not reported");
    }

    #[test]
    fn across_tracks_a_sector_is_found_by_its_id_where_its_header_holds() {
        let at = |r, start: f32, header| Sector {
            id: Id::Ibm([1, 0, r, 2]),
            header,
            ..sector([start, start + 0.01, start + 0.05], None)
        };
        let f = Facts {
            sectors: vec![
                at(1, 0.1, Header::Good),
                at(2, 0.3, Header::Bad),
                at(2, 0.5, Header::Good),
                at(2, 0.9, Header::Good),
            ],
            missing: vec![Id::Ibm([1, 0, 3, 2])],
            ..Facts::default()
        };
        // By R alone: C and H are each track's own.
        assert_eq!(find(&f, Id::Ibm([0, 0, 1, 2]), Some(0.7)), Open::Found(0));
        // Of two, the nearer where it lay, round past the index too; never
        // one whose header's checks fail.
        let r2 = Id::Ibm([0, 0, 2, 2]);
        assert_eq!(find(&f, r2, Some(0.6)), Open::Found(2));
        assert_eq!(find(&f, r2, Some(0.05)), Open::Found(3));
        assert_eq!(find(&f, r2, Some(0.31)), Open::Found(2));
        assert_eq!(
            find(&f, r2, None),
            Open::Found(2),
            "with no place, the first"
        );
        // Not found: the format's ID where it lays the sector out there.
        let laid = Open::Missing {
            id: Id::Ibm([1, 0, 3, 2]),
            laid: true,
        };
        assert_eq!(find(&f, Id::Ibm([0, 0, 3, 2]), Some(0.2)), laid);
        let r9 = Id::Ibm([0, 0, 9, 2]);
        let unlaid = Open::Missing {
            id: r9,
            laid: false,
        };
        assert_eq!(find(&f, r9, None), unlaid);
        // Data found with no header goes to the sector nearest it.
        assert_eq!(find(&f, Id::None, Some(0.45)), Open::Found(2));
        let none = Open::Missing {
            id: Id::Number(4),
            laid: false,
        };
        assert_eq!(find(&Facts::default(), Id::Number(4), None), none);
    }

    #[test]
    fn a_held_arrow_steps_once_then_again_and_again_faster_and_faster() {
        assert_eq!(held_steps(0.0), 1);
        assert_eq!(held_steps(REPEAT_AFTER - 0.01), 1);
        assert_eq!(held_steps(REPEAT_AFTER), 2);
        let in_a_second = |from: f64| held_steps(from + 1.0) - held_steps(from);
        assert_eq!(in_a_second(REPEAT_AFTER), 12);
        assert!(in_a_second(REPEAT_AFTER + 1.0) > in_a_second(REPEAT_AFTER));
        assert_eq!(in_a_second(REPEAT_AFTER + REPEAT_RAMP), 30);
        assert_eq!(in_a_second(REPEAT_AFTER + 10.0), 30, "no faster");
    }

    #[test]
    fn the_mark_moves_round_the_shorter_way_and_eases_out() {
        let near =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
        let still = |at| Glide {
            from: at,
            to: at,
            since: f64::NEG_INFINITY,
        };
        // On from the last sector to the first, on past the index.
        let on = still([0.9, 0.95]).toward([0.01, 0.05], 0.0);
        assert!(near(on.to, [1.01, 1.05]), "{:?}", on.to);
        let half = on.at(GLIDE / 2.0)[0];
        assert!(
            (0.9 + 0.11 * 0.5..1.01).contains(&half),
            "more than half way: {half}"
        );
        assert!(near(on.at(GLIDE), [1.01, 1.05]));
        // Back from the first to the last, back past it.
        let back = still([0.01, 0.05]).toward([0.9, 0.95], 0.0);
        assert!(near(back.to, [-0.1, -0.05]), "{:?}", back.to);
        // Asked again where it goes, it goes on as it was going.
        assert_eq!(on.toward([0.01, 0.05], 0.05).since, 0.0);
        // Asked elsewhere on its way, it goes from where it is.
        let turned = on.toward([0.2, 0.25], GLIDE / 2.0);
        assert!(near(turned.from, on.at(GLIDE / 2.0)));
        assert!(near(turned.to, [1.2, 1.25]), "{:?}", turned.to);
    }

    fn disk(head: u32, media: Media) -> Disk {
        let geometry = Geometry::new(media, 80, 600.0);
        let centre = egui::pos2(300.0, 300.0);
        Disk {
            side: head,
            head,
            span: 80,
            rect: Rect::from_center_size(centre, vec2(600.0, 600.0)),
            centre,
            scale: 1.0,
            line: 1.0,
            geometry,
        }
    }

    #[test]
    fn each_side_lies_on_the_side_of_the_disk_its_head_reads() {
        let all = |sides, swapped| placed(sides, swapped).collect::<Vec<_>>();
        assert_eq!(all(2, false), [(0, 0), (1, 1)]);
        assert_eq!(all(1, false), [(0, 0)]);
        // Swapped, side 1 is read by head 0, and shown first.
        assert_eq!(all(2, true), [(1, 0), (0, 1)]);
        // Side 0 alone is read by head 1, on side 1 of the disk.
        assert_eq!(all(1, true), [(0, 1)]);
        let media = Media::ThreeHalf;
        let g = Geometry::new(media, 80, 2.0 * 42.9 * 10.0);
        let swapped = Disk {
            side: 0,
            head: 1,
            geometry: g,
            ..disk(1, media)
        };
        // Track 0's centreline at 38.0 mm, as ECMA-125 has side 1's.
        let (outer, inner) = swapped.ring(0);
        let per_mm = g.edge / 42.9;
        assert!(((outer + inner) / 2.0 / per_mm - 38.0).abs() < 1e-9);
    }

    #[test]
    fn a_disk_to_scale_is_no_wider_than_its_room() {
        for room in [352.0, 353.0] {
            let g = Geometry::new(Media::ThreeHalf, 80, room);
            assert!(g.pixels as f64 <= room, "{} px in {room}", g.pixels);
            assert!(g.edge <= g.pixels as f64 / 2.0 + 1e-9);
        }
    }

    #[test]
    fn each_disk_holds_the_tracks_its_recording_area_has_room_for() {
        assert_eq!(Media::Fit.holds(), None);
        // ECMA-125's 80, ECMA-78's 80, ECMA-70's 40 and ECMA-69's 77 tracks,
        // and what more each recording area has room for on side 1.
        assert_eq!(Media::ThreeHalf.holds(), Some(93));
        assert_eq!(Media::FiveQuarter96.holds(), Some(90));
        assert_eq!(Media::FiveQuarter48.holds(), Some(45));
        assert_eq!(Media::Eight.holds(), Some(77));
    }

    #[test]
    fn the_index_is_at_the_top_and_each_track_runs_on_clockwise() {
        for d in [disk(0, Media::Fit), disk(1, Media::Fit)] {
            let index = point_at(d.centre, 0.0, 200.0 * d.scale);
            assert!((index.x - d.centre.x).abs() < 1e-3 && index.y < d.centre.y);
            // A quarter turn on, at the right.
            let quarter = point_at(d.centre, 0.25, 200.0 * d.scale);
            assert!(quarter.x > d.centre.x && (quarter.y - d.centre.y).abs() < 1e-3);
        }
    }

    #[test]
    fn the_track_under_a_point_is_the_one_drawn_there() {
        for (media, _, _) in MEDIA {
            for d in [disk(0, media), disk(1, media)] {
                for cyl in [0, 1, 40, 76] {
                    for share in [0.0, 0.1, 0.5, 0.9] {
                        let (outer, inner) = d.ring(cyl);
                        let middle = (outer + inner) as f32 / 2.0;
                        let at = point_at(d.centre, share, middle * d.scale);
                        let (found, at_share) = d.track_at(at).unwrap();
                        assert_eq!(found, cyl, "{media:?}");
                        assert!((at_share - share).abs() < 1e-6, "{at_share} for {share}");
                    }
                }
            }
        }
    }

    #[test]
    fn each_disk_to_scale_has_its_tracks_where_its_standard_puts_them() {
        // Track 00's and the last's centrelines, side 0 and side 1, in mm.
        for (media, last, side_0, side_1) in [
            (Media::ThreeHalf, 79.0, [39.5, 24.6875], [38.0, 23.1875]),
            (
                Media::FiveQuarter96,
                79.0,
                [57.150, 36.248],
                [55.033, 34.131],
            ),
            (
                Media::FiveQuarter48,
                39.0,
                [57.150, 36.513],
                [55.033, 34.396],
            ),
            (Media::Eight, 76.0, [91.754, 51.537], [89.638, 49.421]),
        ] {
            let size = media.size().unwrap();
            let g = Geometry::new(media, 80, 2.0 * size.radius * 10.0);
            let per_mm = g.edge / size.radius;
            for (side, [first, end]) in [(0, side_0), (1, side_1)] {
                let centre = |n: f64| (g.outer[side] - (n + 0.5) * g.pitch) / per_mm;
                assert!(
                    (centre(0.0) - first).abs() < 1e-3,
                    "{media:?} {}",
                    centre(0.0)
                );
                assert!(
                    (centre(last) - end).abs() < 1e-3,
                    "{media:?} {}",
                    centre(last)
                );
            }
            assert!((g.width / per_mm - size.width).abs() < 1e-9);
        }
        let g = Geometry::new(Media::ThreeHalf, 80, 858.0);
        assert!((g.hub.unwrap() / (g.edge / 42.9) - 15.575).abs() < 1e-9);
        // Big enough to have room for lines: a whole number of pixels apart.
        let big = Geometry::new(Media::ThreeHalf, 80, 2000.0);
        assert!(big.pitch >= SEPARATE && (big.pitch - big.pitch.round()).abs() < 1e-9);
    }

    #[test]
    fn fitted_tracks_are_a_whole_number_of_pixels_wide_from_a_whole_pixel() {
        for (span, room) in [(80, 600.0), (40, 333.0), (84, 1000.0), (35, 250.0)] {
            let g = Geometry::new(Media::Fit, span, room);
            assert!(g.pitch >= WHOLE, "{} px for {span} in {room}", g.pitch);
            assert_eq!(g.pitch, g.pitch.round());
            assert_eq!(g.width, g.pitch);
            assert_eq!(g.outer[0], g.outer[0].round());
            assert!(g.pixels as f64 <= room);
            let inner = g.outer[0] - g.pitch * f64::from(span);
            assert!(
                inner >= g.edge * FIT_INNER - 1e-9 && inner > g.hole,
                "{inner}"
            );
        }
        // Too small for whole pixels: spread over the room.
        let small = Geometry::new(Media::Fit, 84, 300.0);
        assert!(small.pitch < WHOLE && small.pitch > 1.0, "{}", small.pitch);
    }

    fn sector(at: [f32; 3], header_end: Option<f32>) -> Sector {
        Sector {
            id: Id::Number(0),
            at: Some(at),
            header_end,
            header: Header::Good,
            data: Data::Good,
            mark: None,
            bytes: Vec::new(),
            extra: false,
            before: false,
            turns: None,
            layout: None,
        }
    }

    fn row_of(sectors: &[Sector]) -> Row {
        let look = Look::of(&theme::DARK, Media::Fit);
        let mut row = Row::new(look.gap);
        for s in sectors {
            row.sector(s, &look);
        }
        row.finish();
        row
    }

    /// `b` laid over `a` at `alpha`.
    fn mix(a: [f64; 3], b: [f64; 3], alpha: f64) -> [f64; 3] {
        [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * alpha)
    }

    fn near(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-6)
    }

    #[test]
    fn a_row_holds_its_sectors_exactly_where_they_lie() {
        let look = Look::of(&theme::DARK, Media::Fit);
        let (gap, good, id) = (rgb(look.gap), rgb(look.good), rgb(look.id(look.good)));
        let line = [9.0; 3];
        let w = 1.0 / 1024.0;
        // An ID field to 0.25 + 1/64, then the sector, its data from
        // 0.25 + 1/32, to 0.5.
        let row = row_of(&[sector([0.25, 0.28125, 0.5], Some(0.265625))]);
        assert!(near(row.sample(0.2578125, Shadow::square(w), w, line), id));
        assert!(near(
            row.sample(0.2734375, Shadow::square(w), w, line),
            good
        ));
        assert!(near(row.sample(0.375, Shadow::square(w), w, line), good));
        assert!(near(row.sample(0.75, Shadow::square(w), w, line), gap));
        // Pixels straddling the ID field's end and the sector's: half each.
        let edge = row.sample(0.265625, Shadow::square(w), w, line);
        assert!(near(edge, mix(id, good, 0.5)), "{edge:?}");
        let edge = row.sample(0.5, Shadow::square(w), w, line);
        assert!(near(edge, mix(good, gap, 0.5)), "{edge:?}");
        // Over the index, the row wraps.
        let over = row_of(&[sector([0.9, 0.9, 1.1], None)]);
        assert!(near(over.sample(0.05, Shadow::square(w), w, line), good));
        assert!(near(over.sample(0.15, Shadow::square(w), w, line), gap));
        assert!(near(
            over.sample(0.0, Shadow::square(0.02), 0.0, line),
            good
        ));
    }

    #[test]
    fn a_line_divides_sectors_only_where_they_meet() {
        let line = [9.0; 3];
        let w = 1.0 / 1024.0;
        let row = row_of(&[
            sector([0.125, 0.125, 0.25], None),
            sector([0.25, 0.25, 0.375], None),
            sector([0.5, 0.5, 0.625], None),
        ]);
        assert_eq!(row.meets, [0.25]);
        assert!(
            near(row.sample(0.25, Shadow::square(w), w, line), line),
            "where two meet"
        );
        // Where a sector starts after a gap, no line.
        let look = Look::of(&theme::DARK, Media::Fit);
        let edge = row.sample(0.5, Shadow::square(w), w, line);
        assert!(
            near(edge, mix(rgb(look.gap), rgb(look.good), 0.5)),
            "{edge:?}"
        );
    }

    #[test]
    fn sectors_a_short_gap_apart_do_not_meet_unless_others_on_the_track_meet_exactly() {
        // DMF's sectors, 24 bytes of gap and sync apart: 0.0019 of a turn.
        let gapped = [(0.1, 0.1981), (0.2, 0.2981), (0.3, 0.3981)];
        assert!(meets(&gapped).is_empty());
        // Sectors end to end, one pair across the seam between revolutions.
        let seamed = [(0.1, 0.2), (0.2, 0.2981), (0.3, 0.4)];
        assert_eq!(meets(&seamed), [0.2, 0.3]);
        // A sector shorter than EXACT, or than MEET where others meet
        // exactly, does not meet itself.
        assert!(meets(&[(0.5, 0.50005)]).is_empty());
        let short = [(0.1, 0.2), (0.2, 0.3), (0.6, 0.6006)];
        assert_eq!(meets(&short), [0.2]);
    }

    #[test]
    fn a_sector_too_short_to_see_is_drawn_and_found_a_line_wide() {
        let look = Look::of(&theme::DARK, Media::Fit);
        let line = [9.0; 3];
        let w = 1.0 / 1024.0;
        // A data mark alone, an eighth of a pixel long.
        let mark = Sector {
            header: Header::None,
            data: Data::Unread,
            ..sector([0.5, 0.5, 0.5 + 1.0 / 8192.0], None)
        };
        let row = row_of(std::slice::from_ref(&mark));
        let middle = 0.5 + w / 16.0;
        assert!(near(
            row.sample(middle, Shadow::square(w), w, line),
            rgb(look.alone)
        ));
        assert!(near(
            row.sample(middle + w, Shadow::square(w), w, line),
            rgb(look.gap)
        ));
        // Half a line either side of its middle, the pointer finds it.
        assert!(holds(&mark, middle + 0.4 * w, w));
        assert!(!holds(&mark, middle + 0.6 * w, w));
        // A sector a line long or more is found only where it lies.
        let whole = sector([0.25, 0.25, 0.5], None);
        assert!(holds(&whole, 0.25, w) && !holds(&whole, 0.25 - w / 4.0, w));
    }

    #[test]
    fn a_sector_from_just_before_the_index_lies_only_from_it() {
        // gw's start a hair before the index, taken into the next revolution
        // in f32: 1.0, and on to 1.05.
        let look = Look::of(&theme::DARK, Media::Fit);
        let (gap, good) = (rgb(look.gap), rgb(look.good));
        let row = row_of(&[sector([1.0, 1.0, 1.05], None)]);
        let w = 1.0 / 1024.0;
        assert!(near(
            row.sample(0.025, Shadow::square(w), w, [9.0; 3]),
            good
        ));
        assert!(
            near(row.sample(0.5, Shadow::square(w), w, [9.0; 3]), gap),
            "not round the track"
        );
        assert!(holds(&sector([1.0, 1.0, 1.05], None), 0.025, w));
    }

    #[test]
    fn the_pointer_names_the_sector_drawn_on_top() {
        let look = Look::of(&theme::DARK, Media::Fit);
        let w = 1.0 / 1024.0;
        // Across the seam between two revolutions, one runs on over the next.
        let bad = Sector {
            data: Data::Bad,
            ..sector([0.299, 0.299, 0.5], None)
        };
        let sectors = [sector([0.1, 0.1, 0.3], None), bad];
        let row = row_of(&sectors);
        assert!(near(
            row.sample(0.2995, Shadow::square(w / 4.0), w, [9.0; 3]),
            rgb(look.bad)
        ));
        assert_eq!(under(&sectors, 0.2995, w), Some(1));
        assert_eq!(under(&sectors, 0.2, w), Some(0));
        // Two too short to see: each a line long, the longer over the other.
        let mark = |at: f64, long: f64, header, data| Sector {
            header,
            data,
            ..sector([at as f32, at as f32, (at + long) as f32], None)
        };
        let marks = [
            mark(0.5, w / 4.0, Header::None, Data::Unread),
            mark(0.5 + w / 8.0, w / 8.0, Header::Good, Data::Bad),
        ];
        let row = row_of(&marks);
        let middle = 0.5 + w / 8.0;
        assert!(near(
            row.sample(middle, Shadow::square(w / 4.0), w, [9.0; 3]),
            rgb(look.alone)
        ));
        assert_eq!(under(&marks, middle, w), Some(0));
    }

    #[test]
    fn a_legend_swatch_is_its_colour_as_the_tracks_show_it() {
        // As Room::lay sees the tracks: by the share of the disk they cover.
        let look = |media: Media, g: &Geometry| Look {
            covered: g.covered(1.0),
            ..Look::of(&theme::LIGHT, media)
        };
        // Fitted with lines between, all but the lines; too close for
        // lines, the whole disk, as drawn.
        let lined = Geometry::new(Media::Fit, 20, 400.0);
        let fit = look(Media::Fit, &lined);
        let share = ((lined.pitch - 1.0) / lined.pitch) as f32;
        assert!(share < 1.0);
        assert_eq!(fit.seen(fit.good), theme::lerp(fit.body, fit.good, share));
        let close = look(Media::Fit, &Geometry::new(Media::Fit, 84, 300.0));
        assert_eq!(close.seen(close.good), close.good);
        // To scale, ECMA-125's 0.115 mm tracks 0.1875 mm apart, erased between.
        let g = Geometry::new(Media::ThreeHalf, 80, 858.0);
        let scale = look(Media::ThreeHalf, &g);
        let share = (0.115 / 0.1875) as f32;
        assert!((scale.covered - share).abs() < 1e-6);
        assert_eq!(
            scale.seen(scale.good),
            theme::lerp(scale.body, scale.good, share)
        );
    }

    #[test]
    fn every_ring_is_drawn_alike() {
        let look = Look::of(&theme::DARK, Media::Fit);
        let span = 20;
        let geometry = Geometry::new(Media::Fit, span, 400.0);
        let wide = geometry.pitch as usize;
        assert!(
            geometry.pitch >= SEPARATE,
            "lines between them, {wide} px apart"
        );
        let d = Disk {
            span,
            geometry,
            ..disk(0, Media::Fit)
        };
        let mut row = Row::new(look.good);
        row.finish();
        let rows = vec![row; span as usize];
        let canvas = Canvas::new(&d, &look, &rows, 1.0);
        let mut image = egui::ColorImage::filled([geometry.pixels; 2], Color32::TRANSPARENT);
        canvas.paint(&mut image, None);
        let centre = geometry.pixels / 2;
        let at = |x: usize, y: usize| image.pixels[y * geometry.pixels + x];
        let outer = canvas.outer as usize - 1;
        // Within a level: a pixel's middle lies half a pixel off the axis,
        // a little further out the nearer the centre.
        let alike =
            |a: Color32, b: Color32| (0..4).all(|i| (i32::from(a[i]) - i32::from(b[i])).abs() <= 1);
        let first: Vec<Color32> = (0..wide).map(|i| at(centre + outer - i, centre)).collect();
        // Track 0's line inside its outer edge, the bare disk's colour, then
        // the row's.
        assert!(alike(first[0], look.body), "{:?}", first[0]);
        for (i, &pixel) in first.iter().enumerate().skip(1) {
            assert!(alike(pixel, look.good), "pixel {i}: {pixel:?}");
        }
        for cyl in 1..span as usize {
            for (i, &want) in first.iter().enumerate() {
                let r = outer - cyl * wide - i;
                let right = at(centre + r, centre);
                assert!(
                    alike(right, want),
                    "ring {cyl}, pixel {i}: {right:?}, {want:?}"
                );
                // The four ways out from the centre are the same pixels.
                assert_eq!(at(centre, centre + r), right, "down");
                assert_eq!(at(centre, centre - r - 1), right, "up");
                assert_eq!(at(centre - r - 1, centre), right, "left");
            }
        }
    }

    /// What each ring shows, by name.
    fn kind(ring: Ring) -> &'static str {
        match ring {
            Ring::Bare => "bare",
            Ring::ToDo => "to do",
            Ring::Unknown => "not known",
            Ring::Sectors(..) => "sectors",
            Ring::Flux => "flux",
            Ring::Spin(_) => "spin",
            Ring::Status(_) => "status",
        }
    }

    fn map_of(progress: &Progress, shows: Shows) -> Map<'_> {
        Map {
            progress,
            image: false,
            disk: (0, 0),
            swapped: false,
            verifying: false,
            media: Media::Fit,
            shows,
            current: None,
            running: false,
        }
    }

    /// The rings of `map`'s side 0, and what its legend counts.
    fn rings(map: &Map) -> (Vec<&'static str>, Drawn) {
        let span = span(map.progress, map.disk);
        let drawn = Drawn::of(map, span, 1, true, &theme::DARK);
        let rings = (0..span)
            .map(|c| kind(ring(map, (c, 0), &drawn, true, &theme::DARK)))
            .collect();
        (rings, drawn)
    }

    #[test]
    fn each_ring_shows_only_what_gw_reported_of_its_track() {
        // What the bridge's reports become: flux round a track, and
        // AmigaDOS's sector 0, where gw found it or with no place.
        let spin = Spin {
            period: 0.2,
            revs: vec![0.2],
            per_rev: 4.0,
            bins: vec![1.0; 4],
            intervals: None,
            ..Spin::default()
        };
        let amiga = |placed: bool, flux: bool| Facts {
            summary: Some("AmigaDOS (1/11 sectors)".into()),
            sectors: vec![Sector {
                at: placed.then_some([0.1, 0.12, 0.2]),
                bytes: vec![0, 255],
                ..sector([0.1, 0.12, 0.2], None)
            }],
            missing: (1..11).map(Id::Number).collect(),
            flux: flux.then(|| spin.clone()),
            ..Facts::default()
        };
        let none = Facts {
            summary: Some("AmigaDOS (0/11 sectors)".into()),
            missing: (0..11).map(Id::Number).collect(),
            flux: Some(spin.clone()),
            ..Facts::default()
        };
        let mut progress = Progress::blank((0..8).collect(), vec![0]);
        for (c, facts) in [
            (0, amiga(true, true)),
            (
                1,
                Facts {
                    absent: true,
                    ..Facts::default()
                },
            ),
            (
                2,
                Facts {
                    flux: Some(spin.clone()),
                    ..Facts::default()
                },
            ),
            (4, amiga(false, true)),
            (7, none),
        ] {
            progress.facts.insert((c, 0), facts);
        }
        progress.feed("T3.0: AmigaDOS (10/11 sectors) from Raw Flux (95000 flux in 400.00ms)");
        progress.feed("T6.0: WARNING: Track is outside the format");
        let (sectors, drawn) = rings(&map_of(&progress, Shows::Sectors));
        assert_eq!(
            sectors,
            [
                "sectors",
                "bare",
                "flux",
                "not known",
                "not known",
                "to do",
                "bare",
                "sectors"
            ]
        );
        assert_eq!(drawn.sectors, [1, 0, 0, 0, 0, 0]);
        let counted = (drawn.flux, drawn.unknown, drawn.to_do, drawn.gaps);
        assert_eq!(counted, (1, 2, 1, false), "not in the image: not to do");
        // Track 0, with sector 0 alone, has sectors missing; track 7, with
        // none, is bad.
        assert_eq!((drawn.missing_tracks, drawn.bad_tracks), (1, 1));
        let (flux, drawn) = rings(&map_of(&progress, Shows::Flux));
        assert_eq!(drawn.shows, Shows::Flux);
        assert_eq!(
            flux,
            [
                "spin",
                "bare",
                "spin",
                "not known",
                "spin",
                "to do",
                "bare",
                "spin"
            ]
        );
        assert_eq!((drawn.unknown, drawn.to_do), (1, 1));
        // While gw works, the track it is on, its line alone: to do, unless
        // gw passes over it.
        for (current, shown) in [((3, 0), "to do"), ((6, 0), "bare")] {
            let running = Map {
                current: Some(current),
                running: true,
                ..map_of(&progress, Shows::Sectors)
            };
            assert_eq!(rings(&running).0[current.0 as usize], shown);
        }
        // With no flux, the sectors show.
        let mut sectors_only = Progress::blank(vec![0], vec![0]);
        sectors_only.facts.insert((0, 0), amiga(true, false));
        let (shown, drawn) = rings(&map_of(&sectors_only, Shows::Flux));
        assert_eq!((shown, drawn.shows), (vec!["sectors"], Shows::Sectors));
        // gw's lines alone, or sectors with no place and no flux, as an
        // image's are: each track as the grid has it.
        let mut lines = Progress::blank((0..3).collect(), vec![0]);
        lines.feed("T0.0: AmigaDOS (11/11 sectors) from Raw Flux (95000 flux in 400.00ms)");
        lines.feed("T1.0: AmigaDOS (5/11 sectors) from Raw Flux (95000 flux in 400.00ms)");
        lines.facts.insert((1, 0), amiga(false, false));
        let (shown, drawn) = rings(&map_of(&lines, Shows::Sectors));
        assert_eq!(shown, ["status", "status", "to do"]);
        assert!(drawn.pure);
        let p = &theme::DARK;
        assert_eq!(drawn.statuses, [p.good, p.partial]);
        // The track gw is on, too, while it works.
        let running = Map {
            current: Some((1, 0)),
            running: true,
            ..map_of(&lines, Shows::Sectors)
        };
        assert_eq!(rings(&running).0, shown);
    }

    #[test]
    fn a_tracks_shortfall_is_gws_own_count_as_the_grid_takes_it() {
        let of = |summary: &str, missing: Vec<Id>| {
            Shortfall::of(&Facts {
                summary: Some(summary.into()),
                missing,
                ..Facts::default()
            })
        };
        // Data that fails is a sector missing to gw, its header found or not.
        assert_eq!(
            of("IBM MFM (17/18 sectors)", Vec::new()),
            Shortfall::Missing
        );
        assert_eq!(of("IBM MFM (0/18 sectors)", Vec::new()), Shortfall::Bad);
        assert_eq!(of("IBM MFM (18/18 sectors)", Vec::new()), Shortfall::None);
        assert_eq!(
            of("AmigaDOS (1/11 sectors)", vec![Id::Number(1)]),
            Shortfall::Missing
        );
        // ibm.scan's track with no format it knows.
        assert_eq!(of("IBM Empty", Vec::new()), Shortfall::Bad);
        assert_eq!(of("IBM MFM (0/0 sectors)", Vec::new()), Shortfall::None);
        assert_eq!(Shortfall::of(&Facts::default()), Shortfall::None);
    }

    #[test]
    fn where_no_sector_was_found_a_ring_shows_its_tracks_shortfall() {
        let look = Look::of(&theme::DARK, Media::Fit);
        let facts = |flux: bool| Facts {
            sectors: vec![sector([0.1, 0.12, 0.2], None)],
            flux: flux.then(|| spin_of(vec![0.2])),
            ..Facts::default()
        };
        let w = 1.0 / 1024.0;
        let gap = |f: &Facts, shortfall| {
            let row = row(Ring::Sectors(f, shortfall), &look);
            row.sample(0.6, Shadow::square(w), w, [9.0; 3])
        };
        let fluxed = facts(true);
        assert!(near(gap(&fluxed, Shortfall::None), rgb(look.gap)));
        assert!(near(
            gap(&fluxed, Shortfall::Missing),
            rgb(look.missing_gap)
        ));
        assert!(near(gap(&fluxed, Shortfall::Bad), rgb(look.bad_gap)));
        // With no flux decoded, nothing is known between: the bare disk.
        let image = facts(false);
        for shortfall in [Shortfall::None, Shortfall::Missing, Shortfall::Bad] {
            assert!(near(gap(&image, shortfall), rgb(look.body)));
        }
        let all = [look.gap, look.missing_gap, look.bad_gap, look.body];
        assert!((0..4).all(|i| (i + 1..4).all(|j| all[i] != all[j])));
    }

    /// A picture's key at `room` pixels across.
    fn key(room: f64) -> Key {
        Key {
            media: Media::Fit,
            shows: Shows::Sectors,
            span: 80,
            side: 0,
            head: 0,
            geometry: Geometry::new(Media::Fit, 80, room),
            palette: &theme::DARK,
            line: 2.0,
            fits: true,
            pure: false,
        }
    }

    #[test]
    fn a_ring_is_painted_again_once_gw_goes_on_from_its_track_or_stops() {
        // gw on track 3, its line alone, and track 0 read as flux.
        let mut progress = Progress::blank((0..8).collect(), vec![0]);
        let flux = Facts {
            flux: Some(spin_of(vec![0.2])),
            ..Facts::default()
        };
        progress.facts.insert((0, 0), flux);
        progress.feed("T3.0: AmigaDOS (11/11 sectors) from Raw Flux (95000 flux in 400.00ms)");
        let on = |current: (u32, u32), running: bool| Map {
            current: Some(current),
            running,
            ..map_of(&progress, Shows::Sectors)
        };
        let p = &theme::DARK;
        let at_3 = stamps(&on((3, 0), true), 0, 8, p);
        let mut painted = Painted {
            key: key(600.0),
            stamps: at_3.clone(),
            at: 0.0,
            asked: (key(600.0).geometry, 0.0),
        };
        assert_eq!(rings(&on((3, 0), true)).0[3], "to do");
        // On to track 4: track 3's report did not come, and the ring says so.
        let at_4 = stamps(&on((4, 0), true), 0, 8, p);
        assert_eq!(
            painted.due(&key(600.0), &at_4, 1.0, false),
            Due::Tracks(vec![3, 4])
        );
        assert_eq!(rings(&on((4, 0), true)).0[3], "not known");
        // Stopped there: so too.
        let stopped = stamps(&on((3, 0), false), 0, 8, p);
        assert_eq!(
            painted.due(&key(600.0), &stopped, 1.0, false),
            Due::Tracks(vec![3])
        );
        assert_eq!(rings(&on((3, 0), false)).0[3], "not known");
        // And the legend's counts with them.
        let to_do = |map: &Map| Drawn::of(map, 8, 1, true, p).to_do;
        assert_eq!(to_do(&on((3, 0), true)), to_do(&on((3, 0), false)) + 1);
        assert_eq!(painted.due(&key(600.0), &at_3, 1.0, false), Due::No);
        // A track reported shows its report, gw on it or not: from track 0
        // to 1, only 1 is painted again.
        let (at_0, at_1) = (
            stamps(&on((0, 0), true), 0, 8, p),
            stamps(&on((1, 0), true), 0, 8, p),
        );
        let changed: Vec<usize> = (0..8).filter(|&c| at_0[c] != at_1[c]).collect();
        assert_eq!(changed, [1]);
    }

    #[test]
    fn a_sides_sums_count_each_sector_as_the_legend_does() {
        // A data block found with no header, and a header with no data: two
        // incomplete sectors, the format's two missing; and a sector found
        // with no place, which is not drawn.
        let data = Sector {
            id: Id::None,
            header: Header::None,
            data: Data::Unread,
            ..sector([0.2, 0.2, 0.24], None)
        };
        let header = Sector {
            data: Data::None,
            ..sector([0.5, 0.51, 0.51], None)
        };
        let unplaced = Sector {
            at: None,
            ..sector([0.7, 0.71, 0.8], None)
        };
        let mut progress = Progress::blank(vec![0, 1], vec![0]);
        let facts = Facts {
            summary: Some("IBM MFM (0/2 sectors)".into()),
            sectors: vec![data, header, unplaced.clone()],
            missing: vec![Id::Ibm([0, 0, 1, 2]), Id::Ibm([0, 0, 2, 2])],
            ..Facts::default()
        };
        progress.facts.insert((0, 0), facts);
        // A track of sectors with no place, not known: the format's one more
        // missing there counts, as the legend's does.
        let unknown = Facts {
            summary: Some("IBM MFM (1/2 sectors)".into()),
            sectors: vec![unplaced],
            missing: vec![Id::Ibm([1, 0, 2, 2])],
            ..Facts::default()
        };
        progress.facts.insert((1, 0), unknown);
        let map = map_of(&progress, Shows::Sectors);
        let (shown, drawn) = rings(&map);
        assert_eq!(shown, ["sectors", "not known"]);
        assert_eq!((drawn.sectors, drawn.missing), ([0, 0, 0, 0, 0, 2], 3));
        let (line, encodings) = sums(&map, &drawn, 2, true, 0, &theme::DARK);
        assert_eq!(line, "Incomplete 2 · 3 missing");
        assert_eq!(encodings, ["IBM MFM"]);
    }

    #[test]
    fn a_sector_is_classed_by_its_worst_check_and_deleted_data_by_its_mark() {
        let with = |header, data, mark| Sector {
            id: Id::Ibm([0, 0, 1, 2]),
            header,
            data,
            mark,
            ..sector([0.1, 0.11, 0.2], Some(0.105))
        };
        let (fb, f8, f9, fa) = (Some(0xfb), Some(0xf8), Some(0xf9), Some(0xfa));
        let cases = [
            (with(Header::Good, Data::Good, fb), Class::Good),
            (with(Header::Good, Data::Empty(0xe5), fb), Class::Empty),
            (with(Header::Good, Data::Good, f8), Class::Deleted),
            (with(Header::Good, Data::Empty(0xe5), f8), Class::Deleted),
            (with(Header::Good, Data::Good, f9), Class::Deleted),
            // TRS-80's directory mark is not one gw calls deleted.
            (with(Header::Good, Data::Good, fa), Class::Good),
            (with(Header::Good, Data::Bad, f8), Class::BadData),
            (with(Header::Bad, Data::Good, fb), Class::BadHeader),
            (with(Header::Bad, Data::Bad, fb), Class::BadHeader),
            (with(Header::Bad, Data::None, None), Class::Incomplete),
            (with(Header::None, Data::Unread, f8), Class::Incomplete),
        ];
        for (i, (s, class)) in cases.iter().enumerate() {
            assert_eq!(Class::of(s), *class, "{i}");
        }
        // A bad header's sector a shade off bad data's, its ID field
        // shaded as any.
        let look = Look::of(&theme::DARK, Media::Fit);
        let w = 1.0 / 4096.0;
        for (i, colour) in [(6, look.bad), (7, look.bad_header)] {
            let row = row_of(std::slice::from_ref(&cases[i].0));
            let at = |share: f64| row.sample(share, Shadow::square(w), w, [9.0; 3]);
            assert!(near(at(0.15), rgb(colour)), "{i}");
            assert!(near(at(0.102), rgb(look.id(colour))), "{i}");
        }
        assert_ne!(look.bad, look.bad_header);
    }

    #[test]
    fn where_a_sector_lies_and_how_each_revolution_read_it_are_said_in_bytes_and_counts() {
        assert_eq!(bytes(528.0), "33 bytes");
        assert_eq!(bytes(16.0), "1 byte");
        assert_eq!(bytes(55_239.0), "3,452 bytes 7 cells");
        assert_eq!(bytes(17.0), "1 byte 1 cell");
        assert_eq!(bytes(-33.0), "2 bytes 1 cell");
        let layout = |after| Layout {
            from_index: 2528.0,
            after,
            id_to_data: Some(544.0),
        };
        let r8 = Before::Sector(Id::Ibm([0, 0, 8, 2]), true);
        let header = Before::Header(Id::Ibm([0, 0, 25, 0]), true);
        assert_eq!(
            layout_line(&layout(Some((992.0, Before::IndexMark)))),
            "158 bytes from the index · 62 bytes after the index mark"
        );
        assert_eq!(
            layout_line(&layout(Some((2896.0, header)))),
            "158 bytes from the index · 181 bytes after R25's header"
        );
        assert_eq!(
            layout_line(&layout(Some((-32.0, r8)))),
            "158 bytes from the index · into R8 by 2 bytes"
        );
        assert_eq!(layout_line(&layout(None)), "158 bytes from the index");
        // Where the CRC of what lay before fails, its ID is not known for sure.
        let r5 = Id::Ibm([0, 0, 5, 2]);
        let (sector, header) = (Before::Sector(r5, false), Before::Header(r5, false));
        let cases = [
            (992.0, sector, "62 bytes after R5 (bad header)"),
            (-32.0, sector, "into R5 (bad header) by 2 bytes"),
            (992.0, header, "62 bytes after R5's bad header"),
            (-32.0, header, "into R5's bad header by 2 bytes"),
        ];
        for (cells, before, said) in cases {
            let line = layout_line(&layout(Some((cells, before))));
            assert_eq!(line, format!("158 bytes from the index · {said}"));
        }
        let turns = |seen: Vec<Seen>, reads| Turns { seen, reads };
        let line = |t: Turns| turns_line(&t).unwrap();
        assert_eq!(
            line(turns(vec![Seen::Good, Seen::BadData, Seen::Good], 1)),
            "Good in 2 of 3 revolutions · data bad in 1"
        );
        assert_eq!(
            line(turns(vec![Seen::BadHeader, Seen::NotFound], 2)),
            "Header bad in 1 of 2 revolutions over 2 reads · not found in 1"
        );
        assert_eq!(
            line(turns(vec![Seen::HeaderAlone], 1)),
            "No data in 1 of 1 revolution"
        );
        assert_eq!(turns_line(&turns(Vec::new(), 1)), None);
    }

    #[test]
    fn a_tracks_order_and_its_repeated_ids_are_of_ids_gw_read_from_good_headers() {
        let at = |share: f32, r: u8, header| Sector {
            id: Id::Ibm([0, 0, r, 2]),
            header,
            ..sector([share, share + 0.01, share + 0.1], Some(share + 0.005))
        };
        let data = Sector {
            id: Id::None,
            header: Header::None,
            data: Data::Unread,
            ..sector([0.9, 0.9, 0.95], None)
        };
        let sectors = [
            at(0.1, 1, Header::Good),
            at(0.3, 7, Header::Good),
            at(0.5, 7, Header::Good),
            at(0.7, 7, Header::Bad),
            data,
        ];
        // The bad header's R is not known for sure.
        assert_eq!(order(&sectors).as_deref(), Some("1 7 7 ?"));
        assert_eq!(repeated(&sectors), ["R7 ×2"]);
        // Two IDs repeated that share their R: each in full.
        let ided = |share: f32, id: [u8; 4]| Sector {
            id: Id::Ibm(id),
            ..at(share, 0, Header::Good)
        };
        let twice = [
            ided(0.1, [0, 0, 3, 2]),
            ided(0.2, [1, 0, 3, 2]),
            ided(0.3, [0, 0, 3, 2]),
            ided(0.4, [1, 0, 3, 2]),
            ided(0.5, [0, 0, 7, 2]),
            ided(0.6, [0, 0, 7, 2]),
        ];
        assert_eq!(
            repeated(&twice),
            ["C0 H0 R3 N2 ×2", "C1 H0 R3 N2 ×2", "R7 ×2"]
        );
        let said = |s: &Sector| -> Vec<String> {
            sector_lines(s, &sectors)
                .into_iter()
                .map(|(l, _)| l)
                .collect()
        };
        let lines = said(&sectors[1]);
        assert!(
            lines.iter().any(|l| l == "Its ID also at 180.0°"),
            "{lines:?}"
        );
        let unsure = said(&sectors[3]);
        assert!(
            !unsure.iter().any(|l| l.starts_with("Its ID")),
            "{unsure:?}"
        );
        // A sector with no place leaves the order unsaid.
        let unplaced = Sector {
            at: None,
            ..at(0.2, 2, Header::Good)
        };
        assert_eq!(order(&[sectors[0].clone(), unplaced]), None);
    }

    #[test]
    fn a_sector_is_said_to_pass_the_checks_gw_made_of_it_and_no_more() {
        let said = |s: &Sector| -> Vec<String> {
            let lines = sector_lines(s, std::slice::from_ref(s));
            lines.into_iter().map(|(l, _)| l).collect()
        };
        // gw adds a sector by number only once its checks pass.
        let numbered = Sector {
            id: Id::Number(3),
            bytes: vec![1, 2],
            ..sector([0.1, 0.15, 0.2], None)
        };
        assert_eq!(said(&numbered)[..2], ["Sector 3 · 2 bytes", "Checks OK"]);
        let empty = Sector {
            data: Data::Empty(0xe5),
            ..numbered.clone()
        };
        assert_eq!(said(&empty)[1], "Checks OK, all E5");
        // An IBM-style sector's header and data apart.
        let ibm = Sector {
            id: Id::Ibm([0, 0, 1, 2]),
            mark: Some(0xfb),
            ..numbered
        };
        assert_eq!(said(&ibm)[1], "Header OK · Data OK · Mark FB");
        // A header alone: no data, so no size; N is in its ID.
        let alone = Sector {
            id: Id::Ibm([0, 0, 5, 2]),
            data: Data::None,
            bytes: Vec::new(),
            ..sector([0.5, 0.52, 0.52], None)
        };
        assert_eq!(
            said(&alone)[..2],
            ["Sector C0 H0 R5 N2", "Header OK · No data"]
        );
        // Data whose bytes were not reported: what its header calls for.
        let unreported = Sector {
            id: Id::Ibm([0, 0, 6, 2]),
            ..sector([0.6, 0.62, 0.7], Some(0.61))
        };
        assert_eq!(said(&unreported)[0], "Sector C0 H0 R6 N2 · 512 bytes");
    }

    #[test]
    fn an_id_field_is_drawn_only_as_far_as_gw_gives_its_end() {
        let look = Look::of(&theme::DARK, Media::Fit);
        let w = 1.0 / 4096.0;
        let at = |row: &Row, share: f64| row.sample(share, Shadow::square(w), w, [9.0; 3]);
        // A sector by number: gw gives no end to an ID field, so none shows.
        let numbered = sector([0.1, 0.15, 0.2], None);
        let row = row_of(std::slice::from_ref(&numbered));
        assert!(near(at(&row, 0.102), rgb(look.good)));
        // An IBM-style sector's, to where gw says it ends.
        let ibm = Sector {
            id: Id::Ibm([0, 0, 1, 2]),
            ..sector([0.1, 0.15, 0.2], Some(0.11))
        };
        let row = row_of(std::slice::from_ref(&ibm));
        assert!(near(at(&row, 0.105), rgb(look.id(look.good))));
        assert!(near(at(&row, 0.13), rgb(look.good)));
        // A header alone, all ID field.
        let alone = Sector {
            id: Id::Ibm([0, 0, 2, 2]),
            data: Data::None,
            ..sector([0.5, 0.52, 0.52], None)
        };
        let row = row_of(std::slice::from_ref(&alone));
        assert!(near(at(&row, 0.51), rgb(look.id(look.alone))));
        // The legend names an ID field only where one is drawn: a header
        // alone's is the incomplete's.
        let listed = |s: &Sector| {
            let mut progress = Progress::blank(vec![0], vec![0]);
            let facts = Facts {
                summary: Some("IBM MFM (1/1 sectors)".into()),
                sectors: vec![s.clone()],
                ..Facts::default()
            };
            progress.facts.insert((0, 0), facts);
            rings(&map_of(&progress, Shows::Sectors)).1.id_fields
        };
        assert_eq!([&numbered, &ibm, &alone].map(listed), [false, true, false]);
    }

    /// Pixel (x, y) as the mean of n×n points over it, premultiplied.
    fn supersampled(canvas: &Canvas, x: usize, y: usize, n: usize) -> [f64; 4] {
        let g = &canvas.geometry;
        let mut sum = [0.0; 4];
        for (i, j) in (0..n).flat_map(|i| (0..n).map(move |j| (i, j))) {
            let along = |p: usize, k: usize| p as f64 + (k as f64 + 0.5) / n as f64;
            let (dx, dy) = (along(x, i) - canvas.centre, along(y, j) - canvas.centre);
            let r = (dx * dx + dy * dy).sqrt();
            if r < g.hole || r > g.edge {
                continue;
            }
            let cyl = ((canvas.outer - r) / g.pitch).floor();
            let edge = canvas.outer - cyl * g.pitch;
            let middle = edge - g.pitch / 2.0;
            let on_track = cyl >= 0.0
                && (cyl as usize) < canvas.rows.len()
                && (r - middle).abs() <= g.width / 2.0;
            let colour = if r >= g.edge - canvas.line {
                canvas.rim
            } else if g.hub.is_some_and(|hub| r < hub) {
                canvas.hub
            } else if !on_track || (canvas.separate && r > edge - canvas.line) {
                canvas.body
            } else {
                let row = &canvas.rows[cyl as usize];
                let share = share_at(dx, dy);
                let half = canvas.line / 2.0 / (TAU * r);
                match row.meets.iter().any(|&m| apart(m, share) <= half) {
                    true => canvas.body,
                    false => rgb(row.colours[row.piece(share)]),
                }
            };
            (0..3).for_each(|k| sum[k] += colour[k]);
            sum[3] += 255.0;
        }
        sum.map(|v| v / (n * n) as f64)
    }

    /// The most a pixel of `canvas`'s picture differs from the mean of the
    /// points over it, of those `at` gives, in levels.
    fn worst(canvas: &Canvas, at: impl Iterator<Item = (usize, usize)>) -> f64 {
        let mut image = egui::ColorImage::filled([canvas.pixels; 2], Color32::TRANSPARENT);
        canvas.paint(&mut image, None);
        let mut worst: f64 = 0.0;
        for (x, y) in at {
            let got = image.pixels[y * canvas.pixels + x].to_array();
            let want = supersampled(canvas, x, y, 128);
            for k in 0..4 {
                worst = worst.max((f64::from(got[k]) - want[k]).abs());
            }
        }
        worst
    }

    /// The pixels from the centre out along `degrees` from the right, and
    /// the same mirrored: through the disk.
    fn across(pixels: usize, degrees: f64) -> impl Iterator<Item = (usize, usize)> {
        let (c, (sin, cos)) = (pixels as f64 / 2.0, degrees.to_radians().sin_cos());
        (0..pixels / 2).flat_map(move |k| {
            let (dx, dy) = (k as f64 * cos, k as f64 * sin);
            [(c + dx, c + dy), (c - dx - 1.0, c - dy - 1.0)].map(|(x, y)| (x as usize, y as usize))
        })
    }

    #[test]
    fn a_pixel_off_the_axes_is_what_its_area_covers() {
        let look = Look::of(&theme::DARK, Media::Fit);
        // Each track another colour, so that one laid over its neighbour's
        // share of a pixel shows.
        let colours = [look.good, look.bad, look.flux, look.alone];
        let rows: Vec<Row> = (0..20)
            .map(|c| {
                let mut row = Row::new(colours[c % 4]);
                row.finish();
                row
            })
            .collect();
        for (media, room) in [(Media::Fit, 400.0), (Media::ThreeHalf, 300.0)] {
            let geometry = Geometry::new(media, 20, room);
            let d = Disk {
                span: 20,
                geometry,
                ..disk(0, media)
            };
            let canvas = Canvas::new(&d, &look, &rows, 1.0);
            assert_eq!(
                canvas.separate,
                media == Media::Fit,
                "lines between fitted ones"
            );
            for degrees in [45.0, 30.0, 0.0] {
                let worst = worst(&canvas, across(geometry.pixels, degrees));
                assert!(worst <= 2.0, "{media:?} at {degrees}°: {worst} levels off");
            }
        }
    }

    #[test]
    fn a_line_where_sectors_meet_is_what_its_area_covers_off_the_axes() {
        let look = Look::of(&theme::DARK, Media::Fit);
        // Two sectors meeting at 0.375 of a revolution, down the diagonal
        // to the lower right, and one ending a little after the other.
        let bad = Sector {
            data: Data::Bad,
            ..sector([0.375, 0.375, 0.5], None)
        };
        let row = row_of(&[sector([0.25, 0.25, 0.375], None), bad]);
        assert_eq!(row.meets, [0.375]);
        let span = 4;
        let geometry = Geometry::new(Media::Fit, span, 400.0);
        let d = Disk {
            span,
            geometry,
            ..disk(0, Media::Fit)
        };
        let rows = vec![row; span as usize];
        let canvas = Canvas::new(&d, &look, &rows, 1.0);
        // Pixels over a pixel from any track's edge.
        let clear = |&(x, y): &(usize, usize)| {
            let c = canvas.centre;
            let r = (x as f64 + 0.5 - c).hypot(y as f64 + 0.5 - c);
            (0..=span).all(|k| {
                let edge = canvas.outer - f64::from(k) * geometry.pitch;
                (r - edge).abs() > 1.0 && (r - edge + canvas.line).abs() > 1.0
            })
        };
        let n = geometry.pixels;
        let near: Vec<(usize, usize)> = (n / 2..n)
            .flat_map(|x| [(x, x), (x + 1, x), (x, x + 1)])
            .filter(|&(x, y)| x < n && y < n)
            .filter(clear)
            .collect();
        assert!(near.len() > 20, "{} pixels", near.len());
        let worst = worst(&canvas, near.into_iter());
        assert!(worst <= 2.0, "{worst} levels off");
    }

    #[test]
    fn a_pixels_shadow_is_a_box_on_the_axes_and_a_triangle_on_the_diagonals() {
        let square = Shadow::of(3.0, 0.0, 3.0);
        assert_eq!((square.below(0.0), square.within(-0.25, 0.25)), (0.5, 0.5));
        let diagonal = Shadow::of(2.0, 2.0, 8f64.sqrt());
        assert!((diagonal.half - 0.5f64.sqrt()).abs() < 1e-12 && diagonal.flat < 1e-12);
        // A band a pixel wide down the diagonal covers all but its corners.
        let band = diagonal.within(-0.5, 0.5);
        assert!((band - (1.0 - (0.5f64.sqrt() - 0.5).powi(2) * 2.0)).abs() < 1e-12);
        for shadow in [square, diagonal, Shadow::of(3.0, 1.0, 10f64.sqrt())] {
            assert_eq!(shadow.within(-1.0, 1.0), 1.0);
            let half = shadow.within(-1.0, 0.0);
            assert!((half - 0.5).abs() < 1e-12, "{shadow:?}: {half}");
        }
    }

    #[test]
    fn fitted_tracks_with_lines_between_cover_all_but_the_lines() {
        let g = Geometry::new(Media::Fit, 20, 400.0);
        assert!(g.separate(1.0) && !g.separate(g.pitch));
        assert_eq!(g.covered(1.0), ((g.pitch - 1.0) / g.pitch) as f32);
        assert_eq!(g.covered(g.pitch), 1.0, "too narrow for lines");
        let scale = Geometry::new(Media::ThreeHalf, 80, 858.0);
        assert!((scale.covered(1.0) - (0.115 / 0.1875) as f32).abs() < 1e-6);
    }

    /// Rows of `span` tracks of sectors end to end, each track's turned
    /// `turn` further round, every `bad`th's sector 3 bad; and with `flux`,
    /// flux shaded round each.
    fn tracks_of(span: u32, turn: f64, bad: u32, flux: bool) -> Vec<Row> {
        let look = Look::of(&theme::DARK, Media::Fit);
        (0..span)
            .map(|c| {
                if flux {
                    let shades = (0..1440).map(|i| (i * 7 + c + bad) % 13);
                    let mut row = Row::pieces(shades.map(|d| look.flux_at(0.5 + d as f32 / 10.0)));
                    row.finish();
                    return row;
                }
                let skew = f64::from(c) * turn;
                let sectors: Vec<Sector> = (0..11)
                    .map(|k| {
                        let start = (0.04 + skew + f64::from(k) * 0.0872) as f32;
                        Sector {
                            data: match c % bad == 0 && k == 3 {
                                true => Data::Bad,
                                false => Data::Good,
                            },
                            ..sector([start, start + 0.002, start + 0.0872], Some(start + 0.001))
                        }
                    })
                    .collect();
                row_of(&sectors)
            })
            .collect()
    }

    #[test]
    fn a_picture_painted_again_over_a_few_tracks_is_the_picture_painted_whole() {
        let look = Look::of(&theme::DARK, Media::Fit);
        for (media, room, line, flux) in [
            (Media::Fit, 700.0, 1.0, false),
            (Media::Fit, 1400.0, 2.0, false),
            (Media::Fit, 1050.0, 2.0, true),
            (Media::ThreeHalf, 1400.0, 2.0, false),
            (Media::Eight, 900.0, 1.0, false),
        ] {
            let span = 77;
            let geometry = Geometry::new(media, span, room);
            let d = Disk {
                span,
                geometry,
                ..disk(0, media)
            };
            let before = tracks_of(span, 0.0, 7, flux);
            let n = geometry.pixels;
            // Tracks 0, 17 and 18 and the last change, the rest as they were;
            // then two inner tracks alone.
            let inner = [40usize, 41];
            for changed in [&[0usize, 17, 18, span as usize - 1][..], &inner] {
                let mut after = tracks_of(span, 0.013, 3, flux);
                for c in 0..span as usize {
                    if !changed.contains(&c) {
                        after[c] = before[c].clone();
                    }
                }
                let mut dirty = vec![false; span as usize];
                changed.iter().for_each(|&c| dirty[c] = true);
                let size = [n; 2];
                let mut image = egui::ColorImage::filled(size, Color32::TRANSPARENT);
                Canvas::new(&d, &look, &before, line).paint(&mut image, None);
                let old = image.clone();
                let rows = Canvas::new(&d, &look, &after, line).paint(&mut image, Some(&dirty));
                let mut whole = egui::ColorImage::filled(size, Color32::TRANSPARENT);
                Canvas::new(&d, &look, &after, line).paint(&mut whole, None);
                let differ = |a: &egui::ColorImage| {
                    (a.pixels.iter().zip(&whole.pixels))
                        .filter(|(a, b)| a != b)
                        .count()
                };
                assert_eq!(differ(&image), 0, "{media:?} at {room} px");
                // The texture takes only the rows painted again, over the
                // picture it had: the picture painted whole.
                let mut taken = old;
                let painted = rows.start * n..rows.end * n;
                taken.pixels[painted.clone()].copy_from_slice(&image.pixels[painted]);
                assert_eq!(differ(&taken), 0, "{media:?} at {room} px, rows {rows:?}");
                if changed == inner {
                    assert!(rows.start > 0 && rows.end < n, "{rows:?} of {n}");
                }
            }
        }
    }

    #[test]
    fn a_picture_waits_for_its_size_to_hold_while_it_changes() {
        let stamps = vec![(None, None, true, None, false); 80];
        let painted = |room: f64, at: f64| Painted {
            key: key(room),
            stamps: stamps.clone(),
            at,
            asked: (key(room).geometry, at),
        };
        let all = Due::All(Vec::new());
        // Painted at 600 pixels a while ago: one change, painted at once.
        let mut p = painted(600.0, 0.0);
        assert_eq!(p.due(&key(600.0), &stamps, 1.0, false), Due::No);
        assert_eq!(p.due(&key(610.0), &stamps, 1.0, false), all);
        // Then a change after another: the picture waits for the size to hold.
        p = painted(610.0, 1.0);
        let wait = p.due(&key(620.0), &stamps, 1.02, false);
        assert_eq!(wait, Due::Later(SETTLE));
        assert_eq!(p.due(&key(630.0), &stamps, 1.04, false), Due::Later(SETTLE));
        assert_eq!(p.due(&key(630.0), &stamps, 1.05 + SETTLE, false), all);
        // Or for the pointer dragging it to let go.
        p = painted(630.0, 1.2);
        assert_eq!(p.due(&key(640.0), &stamps, 1.21, true), all);
        // Back to the size painted at: nothing to paint.
        p = painted(640.0, 2.0);
        assert!(matches!(
            p.due(&key(650.0), &stamps, 2.01, false),
            Due::Later(_)
        ));
        assert_eq!(p.due(&key(640.0), &stamps, 2.02, false), Due::No);
        // A track reported anew: painted again over it, once REPAINT is up.
        let mut reported = stamps.clone();
        reported[5] = (None, Some(9), true, None, false);
        p = painted(640.0, 3.0);
        assert!(matches!(
            p.due(&key(640.0), &reported, 3.05, false),
            Due::Later(_)
        ));
        assert_eq!(
            p.due(&key(640.0), &reported, 3.2, false),
            Due::Tracks(vec![5])
        );
        // Another way: all of it, every row again.
        let mut light = key(640.0);
        light.palette = &theme::LIGHT;
        let every = Due::All((0..80).collect());
        assert_eq!(p.due(&light, &stamps, 3.2, false), every);
    }

    #[test]
    fn equal_pieces_each_lie_their_share_of_the_revolution() {
        let look = Look::of(&theme::DARK, Media::Fit);
        let colours = [look.good, look.bad, look.flux, look.alone];
        let mut row = Row::pieces(colours.into_iter());
        row.finish();
        let w = 1.0 / 1024.0;
        let at = |share: f64| row.sample(share, Shadow::square(w), w, [9.0; 3]);
        for (share, i) in [(0.1, 0), (0.3, 1), (0.6, 2), (0.9, 3)] {
            assert_eq!(row.piece(share), i, "{share}");
            assert!(near(at(share), rgb(colours[i])), "{share}");
        }
        // At each edge, half of either piece; over the index, the last and
        // the first.
        for (edge, a, b) in [(0.25, 0, 1), (0.5, 1, 2), (0.75, 2, 3), (0.0, 3, 0)] {
            let half = mix(rgb(colours[a]), rgb(colours[b]), 0.5);
            assert!(near(at(edge), half), "{edge}: {:?}", at(edge));
        }
        assert_eq!([0.25, 0.5, 0.75].map(|t| row.piece(t)), [1, 2, 3]);
        assert_eq!((row.bounds(0), row.bounds(3)), ((0.0, 0.25), (0.75, 1.0)));
    }

    fn spin_of(revs: Vec<f64>) -> Spin {
        Spin {
            period: 0.2,
            revs,
            per_rev: 4.0,
            bins: vec![1.0; 4],
            intervals: None,
            ..Spin::default()
        }
    }

    #[test]
    fn a_tracks_flux_is_said_to_be_of_what_it_is_and_its_turns_as_measured() {
        assert_eq!(Origin::of(false, None).name(), None);
        assert_eq!(Origin::of(true, None).name(), Some("From the image"));
        let named = |source| Origin::of(false, Some(source)).name();
        assert_eq!(named(Source::Verify), Some("Read back to verify"));
        assert_eq!(named(Source::Written), Some("As written"));
        assert_eq!(named(Source::Image), Some("As written, from the image"));
        // Each revolution read whole, and the rate of their mean.
        let read = Spin {
            period: 0.20005,
            ..spin_of(vec![0.2, 0.2001])
        };
        assert_eq!(
            spin_line(&read, Origin::Read),
            "200.00, 200.10 ms · 299.93 rpm"
        );
        // None read whole: gw's measure of the drive; written, the format's.
        let none = spin_of(Vec::new());
        assert_eq!(
            spin_line(&none, Origin::Read),
            "Drive: 200.00 ms · 300.00 rpm"
        );
        assert_eq!(
            spin_line(&none, Origin::Verify),
            "Drive: 200.00 ms · 300.00 rpm"
        );
        assert_eq!(spin_line(&none, Origin::Image), "200.00 ms · 300.00 rpm");
        assert_eq!(
            spin_line(&spin_of(vec![0.2]), Origin::Written),
            "Format: 200.00 ms · 300.00 rpm"
        );
        // Times gw scaled, as with --adjust-speed, are not the drive's.
        let scaled = |revs| Spin {
            scaled: true,
            ..spin_of(revs)
        };
        assert_eq!(
            spin_line(&scaled(vec![0.2]), Origin::Read),
            "Scaled: 200.00 ms · 300.00 rpm"
        );
        assert_eq!(
            spin_line(&scaled(Vec::new()), Origin::Read),
            "Scaled: 200.00 ms · 300.00 rpm"
        );
    }

    #[test]
    fn the_pointer_is_timed_from_the_index_in_each_revolution_read() {
        assert_eq!(from_index(&spin_of(vec![0.2]), 0.25), "50.00");
        // The same in each to 0.01 ms; else from the least to the most.
        assert_eq!(from_index(&spin_of(vec![0.2, 0.200004]), 0.25), "50.00");
        assert_eq!(
            from_index(&spin_of(vec![0.2004, 0.2, 0.2002]), 0.25),
            "50.00–50.10"
        );
        // None read whole: by gw's measure of the drive.
        assert_eq!(from_index(&spin_of(Vec::new()), 0.5), "100.00");
        // The flux there, as Spin::relative has it.
        let uneven = Spin {
            per_rev: 10.0,
            bins: vec![1.0, 2.0, 3.0, 4.0],
            ..spin_of(vec![0.2])
        };
        let relative = uneven.relative();
        for (i, share) in [0.1, 0.3, 0.6, 0.9].into_iter().enumerate() {
            assert_eq!(relative_at(&uneven, share), relative[i]);
        }
        assert_eq!(relative_at(&uneven, 1.0), relative[3]);
        let none = Spin {
            per_rev: 0.0,
            ..uneven
        };
        assert_eq!(relative_at(&none, 0.5), 0.0);
    }

    #[test]
    fn bytes_are_dumped_sixteen_a_row_after_their_offset_and_then_as_ascii() {
        let bytes: Vec<u8> = (0x1e..0x32).collect();
        let hex = |b: &[u8]| b.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>();
        let rows = [
            format!(
                "0100  {:<47}  {}",
                hex(&bytes[..16]).join(" "),
                "..\u{20}!\"#$%&'()*+,-"
            ),
            format!("0110  {:<47}  {}", hex(&bytes[16..]).join(" "), "./01"),
        ];
        assert_eq!(dump(&bytes, usize::MAX, 0x100), rows.join("\n"));
        assert_eq!(dump(&bytes, 1, 0x100), rows[0]);
        // As many digits as the last offset needs.
        assert!(dump(&bytes, 1, 0xfff0).starts_with("0FFF0  1E 1F"));
        assert_eq!(dump(&[], usize::MAX, 0), "");
        // As the sector window keeps them: row by row, the widest's length.
        let dumped = Dumped::of(&bytes, 0x100);
        let kept: Vec<&str> = dumped
            .rows
            .iter()
            .map(|r| &dumped.text[r.clone()])
            .collect();
        assert_eq!(kept, rows);
        assert_eq!(dumped.widest, rows[0].chars().count());
        assert!(Dumped::of(&[], 0).rows.is_empty());
    }

    #[test]
    fn the_flux_charts_scale_and_its_count_of_longer_say_only_what_holds() {
        // Rounded down at what is shown: each counted as long or longer.
        assert_eq!(top_text(20.0188e-6), "20.01");
        assert_eq!(top_text(18.75e-6), "18.75");
        assert_eq!(top_text(20e-6), "20");
        // An axis to 20 µs, not 22, for bins whose end floating point puts a
        // hair past it.
        assert_eq!(axis(20.000000000000004), (20.0, 2.0));
        assert_eq!(axis(20.5), (22.0, 2.0));
        assert_eq!(axis(12.000000000000002), (12.0, 1.0));
        assert_eq!(axis(0.4), (1.0, 1.0));
    }

    /// The room `map`'s disks are laid out in, in a window `size` points.
    fn laid(map: &Map, size: egui::Vec2) -> Arc<Room> {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut room = None;
        ctx.run_ui(input, |ui| room = Room::of(ui, map))
            .drop_without_applying_deltas();
        room.expect("cylinders to show")
    }

    #[test]
    fn the_legend_names_the_index_only_where_a_disk_has_room_for_its_mark() {
        let mut progress = Progress::blank((0..80).collect(), vec![0]);
        let facts = Facts {
            summary: Some("IBM MFM (1/1 sectors)".into()),
            sectors: vec![sector([0.1, 0.12, 0.2], None)],
            ..Facts::default()
        };
        progress.facts.insert((0, 0), facts);
        let map = map_of(&progress, Shows::Sectors);
        let indexed = |room: &Room| {
            let entries = room.legend.entries.iter();
            entries
                .filter(|e| matches!(e.mark, Some(Mark::Index(_))))
                .count()
        };
        for (size, notched) in [(vec2(120.0, 200.0), false), (vec2(800.0, 800.0), true)] {
            let room = laid(&map, size);
            assert_eq!(room.geometry.notch(0, room.line).is_some(), notched);
            assert_eq!(indexed(&room), usize::from(notched), "{size:?}");
            // Its marks as the tracks show them at that size.
            assert_eq!(room.look.covered, room.geometry.covered(room.line));
            let good = room.legend.entries.first().and_then(|e| e.mark);
            let want = room.look.seen(room.look.good);
            assert!(matches!(good, Some(Mark::Swatch(c)) if c == want));
        }
    }

    #[test]
    fn thousands_are_set_apart() {
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(88_068), "88,068");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }
}

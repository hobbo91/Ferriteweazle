//! The Analyse drawer's view of the disk: each side as a round disk, a ring
//! per track, and on it each sector gw found, where it found it, round from
//! the index at the top, clockwise. Nothing is drawn that gw did not report.

use crate::diskmap;
use crate::form;
use crate::lines::Lines;
use crate::progress::{Progress, Status};
use crate::theme::{self, Palette};
use crate::track::{
    Before, Data, Facts, Header, Id, Intervals, Layout, Sector, Seen, Source, Spin, Turns,
};
use eframe::egui::{
    self, Align2, Color32, FontId, Galley, Pos2, Rect, RichText, Sense, Shape, Stroke,
    emath::GuiRounding, plugin::TypedPluginHandle, vec2,
};
use std::f64::consts::TAU;
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

/// What the drawer analyses: the disk, or the image the job makes or takes
/// its tracks from.
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
            // ECMA-78: 130.2 mm across, its hole 28.57 mm (3.3.1), the
            // recording area in to 31.3 mm (3.3.4), track n at 57.150 or
            // 55.033 mm less n/96 inch, 0.155 mm wide (5.1).
            Media::FiveQuarter96 => Some(Size {
                radius: 130.2 / 2.0,
                hub: None,
                hole: 28.57 / 2.0,
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
                hole: 28.57 / 2.0,
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
    let showing = match *shows {
        Shows::Flux if fluxed => Shows::Flux,
        _ => Shows::Sectors,
    };
    for (view, name, _) in SHOWS.into_iter().rev() {
        let why = (view == Shows::Flux && !fluxed).then_some("No flux reported.");
        let button = egui::Button::selectable(showing == view, name);
        let chosen = ui
            .add_enabled(why.is_none(), button)
            .on_disabled_hover_text(why.unwrap_or_default());
        if chosen.clicked() {
            *shows = view;
        }
    }
}

/// Room for a side's name above its disk.
pub(crate) const TITLE: f32 = 20.0;
/// The sector window's title bar, as tall as a macOS window's, its title in
/// the same 13-point type.
const TITLE_BAR: f32 = 28.0;
const TITLE_SIZE: f32 = 13.0;
/// The legend: the room above it and after each entry.
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
/// with a gap between their sectors, however short, have none that meet.
const EXACT: f64 = 1e-4;
const MEET: f64 = 0.003;
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
    pub(crate) body: Color32,
    pub(crate) hub: Color32,
    pub(crate) rim: Color32,
    /// Where no sector was found on a track gw decoded from flux.
    pub(crate) gap: Color32,
    /// A track gw is to work on and has not reported: in the image view, and
    /// on the disk, a shade off its surface toward the text's colour.
    pub(crate) pending: Color32,
    to_do: Color32,
    /// A track of which what the view shows was not reported: where its
    /// sectors lie, or its flux. Further toward the text's colour, no hue.
    unknown: Color32,
    pub(crate) good: Color32,
    pub(crate) empty: Color32,
    /// Data its mark calls deleted, its CRC holding.
    pub(crate) deleted: Color32,
    /// A CRC that fails: of the data; of the header, a shade further toward
    /// the ink.
    pub(crate) bad: Color32,
    pub(crate) bad_header: Color32,
    /// A header with no data after it, or data with no header.
    pub(crate) alone: Color32,
    pub(crate) flux: Color32,
    pub(crate) last: Color32,
    pub(crate) index: Color32,
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
/// them; and the most height they can use, as wide as the room lets them,
/// with the legend's height then.
#[derive(Clone)]
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
    most: f32,
    widest_legend: f32,
}

/// What a room was laid out for: the pass, the ui and the room it had
/// left, and what of the map it shows.
#[derive(Clone, PartialEq)]
struct RoomKey {
    pass: u64,
    ui: egui::Id,
    rect: Rect,
    revision: u64,
    disk: (u32, u32),
    media: Media,
    shows: Shows,
    current: Option<(u32, u32)>,
    verifying: bool,
}

impl Room {
    /// The disks' room in what `ui` has left for `map`, laid out once a pass.
    /// None with no cylinders to show.
    fn of(ui: &egui::Ui, map: &Map) -> Option<Room> {
        let ctx = ui.ctx();
        let kept = ctx.plugin_or_default::<Kept>();
        let key = RoomKey {
            pass: ctx.cumulative_pass_nr(),
            ui: ui.id(),
            rect: ui.available_rect_before_wrap(),
            revision: map.progress.revision,
            disk: map.disk,
            media: map.media,
            shows: map.shows,
            current: map.current,
            verifying: map.verifying,
        };
        if let Some((_, room)) = kept.lock().room.as_ref().filter(|(k, _)| *k == key) {
            return Some(room.clone());
        }
        let room = Room::lay(ui, map, &kept)?;
        let mut kept = kept.lock();
        // The most height the disks can use changes with their legend's:
        // the pass again, the drawer as tall as that, not a frame late.
        if kept.widest.is_some_and(|h| h != room.widest_legend) {
            ctx.request_discard("the disks' legend");
            if !ctx.will_discard() {
                ctx.request_repaint();
            }
        }
        kept.widest = Some(room.widest_legend);
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
        let holds = map
            .media
            .holds()
            .filter(|_| !fits)
            .map(|n| format!("{span} cylinders: a {} disk holds {n}.", map.media.name()));
        // The legend runs across the room under the disks, whatever their
        // size, so its height sets theirs and never the other way round.
        // While gw works its counts change: the room it has taken it keeps.
        let mut look = Look::of(p, map.media);
        let mut legend = Legend::of(ui, map, &drawn, &look, holds.clone());
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
        // Its colours as the tracks show them at that size.
        look.covered = geometry.covered(line);
        legend = Legend::of(ui, map, &drawn, &look, holds);
        legend.flow(ui, room.x);
        legend.height = height;
        let widest_legend = height;
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
            most,
            widest_legend,
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
    let mut look = Look::of(p, map.media);
    look.covered = room.geometry.covered(room.line);
    let (diameter, sides) = (room.diameter, room.sides);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), TITLE + diameter), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Disk map"));
    let painter = ui.painter_at(rect.expand(2.0));
    let left = rect.center().x - room.width / 2.0;
    let disks: Vec<Disk> = (0..sides)
        .map(|head| {
            let side = if map.swapped && sides == 2 {
                1 - head
            } else {
                head
            };
            let x = left + head as f32 * (diameter + SIDE_GAP);
            let min = egui::pos2(x, rect.top() + TITLE).round_to_pixels(ppp);
            let picture = Rect::from_min_size(min, vec2(diameter, diameter));
            Disk {
                side,
                head,
                span: room.span,
                rect: picture,
                centre: picture.center(),
                scale: 1.0 / ppp,
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
            FontId::proportional(13.0),
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
            d.draw(ui, &painter, map, &room, &look, &mut kept.pictures[head]);
        }
    }
    let pointer = response.hover_pos();
    let hovered = pointer.and_then(|at| {
        let d = disks
            .iter()
            .find(|d| (at - d.centre).length() <= diameter / 2.0)?;
        Some((d, d.track_at(at)?))
    });
    if let Some((d, (cyl, share))) = hovered.filter(|_| room.fits) {
        let key = (cyl, d.side);
        let found = progress.facts.get(&key);
        let least = d.line_at(pointer.unwrap_or_default());
        let at = found.and_then(|f| under(&f.sectors, share, least));
        let sector = at.zip(found).map(|(i, f)| &f.sectors[i]);
        let faint = Stroke::new(1.0, look.ink.gamma_multiply(0.45));
        d.outline(&painter, cyl, faint);
        if let Some((start, end)) = sector.and_then(|s| extent(s, least)) {
            let stroke = Stroke::new(1.5, look.ink);
            d.outline_arc(&painter, cyl, start, end, stroke);
        }
        if let Some((index, f)) = at.zip(found).filter(|_| response.clicked()) {
            ui.data_mut(|d| d.insert_temp(inspected(), (key, index, f.revision)));
        }
        response
            .clone()
            .on_hover_ui_at_pointer(|ui| tip(ui, map, key, share, least));
    }
    let over_title = pointer.and_then(|at| {
        disks
            .iter()
            .find(|d| at.y < d.rect.top() && (d.rect.left()..=d.rect.right()).contains(&at.x))
    });
    if let Some(d) = over_title {
        response
            .clone()
            .on_hover_ui_at_pointer(|ui| side_tip(ui, map, d.side, d.head, room.span));
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
            pixels: 2 * edge.ceil() as usize,
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
    /// The disk's side, as gw numbers it, and the drive head that reads it.
    side: u32,
    head: u32,
    /// The cylinders drawn.
    span: u32,
    /// The picture, on whole pixels.
    rect: Rect,
    centre: Pos2,
    /// Points per pixel.
    scale: f32,
    geometry: Geometry,
}

/// Where a share of a revolution from the index points, from the centre:
/// the index at the top, the track running on clockwise.
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
    /// Where a share of a revolution from the index lies, `r` pixels from the
    /// centre.
    fn at(&self, share: f64, r: f64) -> Pos2 {
        let (x, y) = heading(share);
        self.centre + self.scale * vec2((r * x) as f32, (r * y) as f32)
    }

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
        f64::from(1.0 / self.scale).round().max(1.0) / (TAU * r.max(1.0))
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
        look: &Look,
        slot: &mut Option<Picture>,
    ) {
        let ctx = ui.ctx();
        let p = theme::palette(ui);
        let (drawn, fits) = (&*room.drawn, room.fits);
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
        let row = |cyl: usize| row(ring(map, (cyl as u32, self.side), drawn, fits, p), look);
        // A pass to be laid out again shows nothing: it paints nothing.
        if !ctx.will_discard() {
            let stamps = stamps(map.progress, self.side, self.span, p);
            let (now, released) = ui.input(|i| (i.time, i.pointer.any_released()));
            let painted = |stamps| Painted {
                key,
                stamps,
                at: now,
                asked: (key.geometry, now),
            };
            match slot {
                None => *slot = Some(Picture::new(ctx, self, look, row, painted(stamps))),
                Some(picture) => match picture.painted.due(&key, &stamps, now, released) {
                    Due::No => {}
                    Due::Later(wait) => ctx.request_repaint_after(Duration::from_secs_f64(wait)),
                    Due::Tracks(changed) => {
                        picture.paint(self, look, row, &changed, false, painted(stamps))
                    }
                    Due::All(changed) => {
                        picture.paint(self, look, row, &changed, true, painted(stamps))
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
        let ctx = painter.ctx();
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

    /// The index's mark: a notch in the rim at the top, pointing in at track 0.
    fn index_mark(&self, painter: &egui::Painter, look: &Look) {
        let g = &self.geometry;
        let line = f64::from(1.0 / self.scale).round().max(1.0);
        let base = g.edge - line;
        let tip = (g.outer[self.head as usize] + line).max(base - 10.0 * line);
        if base - tip < 2.0 {
            return;
        }
        let half = (base - tip) * 0.7;
        let corner = |x: f64| self.centre + self.scale * vec2(x as f32, -base as f32);
        let points = vec![self.at(0.0, tip), corner(half), corner(-half)];
        painter.add(Shape::convex_polygon(points, look.index, Stroke::NONE));
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
        let half = f64::from(stroke.width / 2.0 / self.scale);
        let (outer, inner) = (outer + half, (inner - half).max(0.0));
        // Steps a point long round the outer edge, at least eight.
        let length = TAU * outer * (to - from) * f64::from(self.scale);
        let steps = (length.ceil() as usize).clamp(8, 2048);
        let along = |i: usize| from + (to - from) * i as f64 / steps as f64;
        let mut points: Vec<Pos2> = (0..=steps).map(|i| self.at(along(i), outer)).collect();
        points.extend((0..=steps).rev().map(|i| self.at(along(i), inner)));
        painter.add(Shape::closed_line(points, stroke));
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
    /// found none, the gap's colour.
    Sectors(&'a Facts),
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
        && map.running
        && map.current == Some(key)
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
            Ring::Sectors(f)
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
        Ring::Sectors(f) => {
            let background = match f.flux {
                Some(_) => look.gap,
                None => look.body,
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

/// Whether gw announced it would work on `key`.
fn planned(progress: &Progress, (cyl, head): (u32, u32)) -> bool {
    progress.cyls.contains(&cyl) && progress.heads.contains(&head)
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
    /// ID fields; sectors that meet; and where gw decoded flux, places with
    /// no sector.
    id_fields: bool,
    meet: bool,
    gaps: bool,
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
}

/// What a count of what the disk view draws was of: the job's revision,
/// the tracks gw was to work on, how many it spans and what it shows.
#[derive(Clone, PartialEq)]
struct DrawnKey {
    revision: u64,
    cyls: Vec<u32>,
    heads: Vec<u32>,
    span: u32,
    sides: u32,
    shows: Shows,
    fits: bool,
    palette: &'static Palette,
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
            cyls: progress.cyls.clone(),
            heads: progress.heads.clone(),
            span,
            sides,
            shows: map.shows,
            fits,
            palette: p,
        };
        if let Some((_, drawn)) = kept.lock().counted.as_ref().filter(|(k, _)| *k == key) {
            return drawn.clone();
        }
        let drawn = Arc::new(Drawn::of(map, span, sides, fits, p));
        kept.lock().counted = Some((key, drawn.clone()));
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
        let shows = match map.shows {
            Shows::Flux if fluxed => Shows::Flux,
            _ => Shows::Sectors,
        };
        let mut drawn = Drawn {
            shows,
            pure: !fluxed && !placed,
            ..Drawn::default()
        };
        for (cyl, side) in (0..span).flat_map(|c| (0..sides).map(move |s| (c, s))) {
            match ring(map, (cyl, side), &drawn, fits, p) {
                Ring::Sectors(f) => {
                    for s in f.sectors.iter().filter(|s| s.at.is_some()) {
                        let class = Class::of(s);
                        drawn.sectors[class as usize] += 1;
                        let alone = class == Class::Incomplete;
                        drawn.headers_alone |= alone && s.header != Header::None;
                        drawn.data_alone |= alone && s.header == Header::None;
                        drawn.id_fields |= s.header_end.is_some();
                    }
                    drawn.meet |= meet(&f.sectors);
                    drawn.gaps |= f.flux.is_some();
                }
                Ring::Flux => drawn.flux += 1,
                Ring::Unknown => drawn.unknown += 1,
                Ring::ToDo => drawn.to_do += 1,
                Ring::Status(colour) => drawn.statuses.push(colour),
                Ring::Bare | Ring::Spin(_) => {}
            }
            let facts = progress.facts.get(&(cyl, side)).filter(|_| fits);
            drawn.missing += facts.map_or(0, |f| f.missing.len());
            drawn.shared += facts.map_or(0, |f| {
                let sectors = &f.sectors;
                let shares =
                    |s: &Sector| sectors.iter().any(|t| !std::ptr::eq(s, t) && same_id(s, t));
                sectors.iter().filter(|s| shares(s)).count()
            });
        }
        drawn
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
    /// counts it, for what each was of; and the legend's height at the
    /// disks' widest, last laid out.
    room: Option<(RoomKey, Room)>,
    counted: Option<(DrawnKey, Arc<Drawn>)>,
    widest: Option<f32>,
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
    /// texture takes them in place where its size holds.
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
        canvas.paint(Arc::make_mut(&mut self.image), dirty.as_deref());
        let options = egui::TextureOptions::LINEAR;
        match resized {
            true => self.texture.set(self.image.clone(), options),
            false => self
                .texture
                .set_partial([0, 0], self.image.clone(), options),
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
/// it, the bridge's report, whether gw is to work on it, and its colour on
/// the grid.
type Stamp = (Option<Status>, Option<u64>, bool, Option<Color32>);

/// What is known of each of `side`'s tracks.
fn stamps(progress: &Progress, side: u32, span: u32, p: &Palette) -> Vec<Stamp> {
    (0..span)
        .map(|cyl| {
            let key = (cyl, side);
            (
                progress.tracks.get(&key).map(|t| t.status),
                progress.facts.get(&key).map(|f| f.revision),
                planned(progress, key),
                diskmap::fill(progress, key, p),
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
    /// the threads there are, and of a row, only where it crosses them.
    fn paint(&self, image: &mut egui::ColorImage, only: Option<&[bool]>) {
        let width = self.pixels;
        let bands = only.map(|dirty| self.bands(dirty));
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
        let rows = width.div_ceil(threads).max(1);
        std::thread::scope(|scope| {
            for (i, chunk) in image.pixels.chunks_mut(rows * width).enumerate() {
                let bands = bands.as_deref();
                scope.spawn(move || {
                    for (j, line) in chunk.chunks_mut(width).enumerate() {
                        let y = i * rows + j;
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
/// shadow on the line. A trapezoid, `half` long either side, its top `flat`
/// long: square to the line, a box; on the diagonal, a triangle.
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

    /// Pieces all as long as each other round the revolution, one per colour.
    fn pieces(colours: impl Iterator<Item = Color32>) -> Row {
        let colours: Vec<Color32> = colours.collect();
        if colours.is_empty() {
            return Row::new(Color32::BLACK);
        }
        Row {
            colours,
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
        let Some([start, data, end]) = s.at.map(|a| a.map(f64::from)) else {
            return;
        };
        let colour = look.status(s);
        self.lay(start, end, colour);
        if s.header != Header::None {
            let header_end = s.header_end.map_or(data, f64::from).min(data);
            self.lay(start, header_end, look.id(colour));
        }
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
        let (begins, ends): (Vec<f64>, Vec<f64>) =
            std::mem::take(&mut self.edges).into_iter().unzip();
        self.meets = meets(&begins, &ends);
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

/// What is known of the hovered track and sector.
fn tip(ui: &mut egui::Ui, map: &Map, (cyl, side): (u32, u32), share: f64, least: f64) {
    let progress = map.progress;
    let reported = progress.facts.get(&(cyl, side));
    let absent = reported.is_some_and(|f| f.absent);
    let facts = reported.filter(|f| !f.absent);
    let track = progress.tracks.get(&(cyl, side));
    ui.strong(format!("Cylinder {cyl} · side {side}"));
    match (facts.and_then(|f| f.summary.as_deref()), track) {
        _ if absent => {
            ui.label("Not in the image");
        }
        (Some(summary), _) => {
            ui.label(summary);
        }
        (None, Some(t)) => {
            ui.label(&t.text);
        }
        (None, None) => {
            ui.weak("Not reported");
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
    // Where the pointer is: by the revolution the track is drawn by, in time.
    let period = facts.and_then(|f| f.flux.as_ref()).map(|spin| spin.period);
    ui.weak(match period {
        Some(period) => format!(
            "At {:.1}° · {:.2} ms from the index",
            share * 360.0,
            share * period * 1e3
        ),
        None => format!("At {:.1}° from the index", share * 360.0),
    });
    let Some(f) = facts else {
        return;
    };
    if let Some(s) = under(&f.sectors, share, least).map(|i| &f.sectors[i]) {
        ui.separator();
        sector_tip(ui, s, &f.sectors);
        if s.bytes.len() > 64 {
            ui.weak(format!("Click for all {} bytes", s.bytes.len()));
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
            let relative = spin.relative();
            let at = ((share * relative.len() as f64) as usize).min(relative.len() - 1);
            let here = (relative[at] * 100.0).round();
            ui.label(format!("Flux here: {here}% of the track's average"));
        }
        spin_tip(ui, f, spin, Origin::of(map.image, f.source));
        if let Some(intervals) = &spin.intervals {
            intervals_chart(ui, intervals);
        }
    }
}

/// The IDs of a track's sectors in turn round it from the index, each R,
/// or number: of every sector with an ID, where all are placed and there
/// are two or more.
fn order(sectors: &[Sector]) -> Option<String> {
    let named: Vec<&Sector> = sectors.iter().filter(|s| s.id != Id::None).collect();
    if named.len() < 2 || named.iter().any(|s| s.at.is_none()) {
        return None;
    }
    let ids: Vec<String> = named
        .iter()
        .map(|s| match s.id {
            Id::Ibm([.., r, _]) => r.to_string(),
            Id::Number(n) => n.to_string(),
            Id::None => unreachable!(),
        })
        .collect();
    Some(ids.join(" "))
}

/// Each ID two or more of a track's sectors carry, from headers whose CRC
/// holds, and how many: as `R3 ×2`.
fn repeated(sectors: &[Sector]) -> Vec<String> {
    let mut seen: Vec<(Id, usize)> = Vec::new();
    for s in sectors.iter().filter(|s| sure_id(s)) {
        match seen.iter_mut().find(|(id, _)| *id == s.id) {
            Some((_, n)) => *n += 1,
            None => seen.push((s.id, 1)),
        }
    }
    seen.into_iter()
        .filter(|&(_, n)| n > 1)
        .map(|(id, n)| format!("{} ×{n}", short_id(&id)))
        .collect()
}

/// How far apart the track's flux transitions are: a count of each bin's,
/// from no time to the longest counted, and how many were longer.
fn intervals_chart(ui: &mut egui::Ui, i: &Intervals) {
    if i.counts.is_empty() && i.longer == 0 {
        return;
    }
    let p = theme::palette(ui);
    let width_us = i.width * 1e6;
    let span = (i.first as usize + i.counts.len()) as f64 * width_us;
    // Whole microseconds along the bottom, one or two apart.
    let step = if span > 12.0 { 2.0 } else { 1.0 };
    let end = ((span / step).ceil() * step).max(step);
    let bin = match i.width < 1e-6 {
        true => format!("{:.1} ns", i.width * 1e9),
        false => format!("{} µs", (i.width * 1e9).round() / 1e3),
    };
    ui.label(format!("Flux intervals in µs, bins of {bin}"));
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
    if i.longer > 0 {
        let top = i.top * 1e6;
        let top = match (top - top.round()).abs() < 1e-6 {
            true => format!("{}", top.round()),
            false => format!("{top:.2}"),
        };
        ui.weak(format!("{} of {top} µs or longer", grouped(i.longer)));
    }
}

/// The track's revolutions and flux.
fn spin_tip(ui: &mut egui::Ui, f: &Facts, spin: &Spin, from: Origin) {
    let rpm = |seconds: f64| 60.0 / seconds;
    let ms = |seconds: f64| format!("{:.2}", seconds * 1e3);
    let revs: Vec<String> = spin.revs.iter().map(|&r| ms(r)).collect();
    let mean = spin.revs.iter().sum::<f64>() / spin.revs.len().max(1) as f64;
    let line = match (from, revs.is_empty()) {
        (Origin::Written, _) => format!(
            "Format: {} ms · {:.2} rpm",
            ms(spin.period),
            rpm(spin.period)
        ),
        (_, false) => format!("{} ms · {:.2} rpm", revs.join(", "), rpm(mean)),
        (Origin::Read | Origin::Verify, true) => format!(
            "Drive: {} ms · {:.2} rpm",
            ms(spin.period),
            rpm(spin.period)
        ),
        (_, true) => format!("{} ms · {:.2} rpm", ms(spin.period), rpm(spin.period)),
    };
    ui.label(line);
    let mut flux = format!("{} flux/rev", grouped(spin.per_rev.round() as u64));
    if let Some(cell) = f.cell {
        flux += &format!(" · {:.3} µs cells", cell * 1e6);
    }
    ui.label(flux);
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

/// A side's sums: the sectors gw found by how they decoded, as the legend
/// names them, those missing, and the encodings gw decoded; of the side's
/// tracks the disk view spans, `span` cylinders.
fn side_tip(ui: &mut egui::Ui, map: &Map, side: u32, head: u32, span: u32) {
    match side == head {
        true => ui.strong(format!("Side {side}")),
        false => ui.strong(format!("Side {side} · head {head}")),
    };
    let (sums, encodings) = sums(map.progress, side, span);
    if !sums.is_empty() {
        ui.label(sums);
    }
    if !encodings.is_empty() {
        ui.label(encodings.join(", "));
    }
}

/// A side's sectors gw found by how they decoded and those missing, in a
/// line, and the encodings gw decoded, over the `span` cylinders drawn.
fn sums(progress: &Progress, side: u32, span: u32) -> (String, Vec<&str>) {
    let facts = progress.facts.iter();
    let facts = facts.filter(|((c, h), f)| *h == side && *c < span && !f.absent);
    let (mut counts, mut missing) = ([0usize; Class::ALL.len()], 0);
    let mut encodings: Vec<&str> = Vec::new();
    for (_, f) in facts {
        missing += f.missing.len();
        for s in &f.sectors {
            counts[Class::of(s) as usize] += 1;
        }
        let summary = f.summary.as_deref().unwrap_or_default();
        let encoding = summary.split(" (").next().unwrap_or_default();
        if !encoding.is_empty() && !encodings.contains(&encoding) {
            encodings.push(encoding);
        }
    }
    let mut sums: Vec<String> = (Class::ALL.map(Class::name).iter().zip(counts))
        .filter(|&(_, n)| n > 0)
        .map(|(name, n)| format!("{name} {n}"))
        .collect();
    if missing > 0 {
        sums.push(format!("{missing} missing"));
    }
    (sums.join(" · "), encodings)
}

/// The sector whose data is open: its track, its place among the track's
/// sectors, and the track's facts' revision.
type Inspected = ((u32, u32), usize, u64);

/// The id under which the sector whose data is open is kept.
fn inspected() -> egui::Id {
    egui::Id::new("disk sector")
}

/// The window a click on a sector opens: all gw decoded of it, and its
/// data in full.
fn inspector(ctx: &egui::Context, map: &Map) {
    let Some((key, index, revision)) = ctx.data(|d| d.get_temp::<Inspected>(inspected())) else {
        return;
    };
    // Gone once the track is reported again, or another job's shows.
    let facts = map
        .progress
        .facts
        .get(&key)
        .filter(|f| f.revision == revision);
    let Some(s) = facts.and_then(|f| f.sectors.get(index)) else {
        ctx.data_mut(|d| d.remove::<Inspected>(inspected()));
        return;
    };
    let (cyl, side) = key;
    let title = format!("{} · cylinder {cyl}, side {side}", id_text(&s.id));
    let id = egui::Id::new("disk sector window");
    let shown = Shown {
        title: &title,
        lines: &sector_lines(s, facts.map_or(&[][..], |f| &f.sectors)),
        bytes: &s.bytes,
        base: 0,
    };
    if !sector_window(ctx, id, &shown) {
        ctx.data_mut(|d| d.remove::<Inspected>(inspected()));
    }
}

/// What a sector window shows: its title, what is said of the sector, and
/// its data, the first byte numbered `base`.
pub(crate) struct Shown<'a> {
    pub title: &'a str,
    pub lines: &'a [(String, Tone)],
    pub bytes: &'a [u8],
    pub base: usize,
}

/// A sector's window, first in the middle of the app's: a title bar as tall
/// as a macOS window's, what is said of the sector, and its data in full.
/// Whether it is still open.
pub(crate) fn sector_window(ctx: &egui::Context, id: egui::Id, shown: &Shown) -> bool {
    let mut open = true;
    let title = shown.title;
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(0);
    egui::Window::new(title)
        .id(id)
        .title_bar(false)
        .frame(frame)
        .resizable(false)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let p = theme::palette(ui);
            let font = FontId::proportional(TITLE_SIZE);
            let galley = ui
                .painter()
                .layout_no_wrap(title.to_owned(), font, p.strong);
            // The title's room, between a button's each side, so it centres.
            let least = galley.size().x + 2.0 * TITLE_BAR + 2.0;
            let (bar, _) = ui.allocate_exact_size(vec2(least, TITLE_BAR), Sense::hover());
            let margin = egui::Margin::symmetric(12, 10);
            egui::Frame::new()
                .inner_margin(margin)
                .show(ui, |ui| sector_text(ui, shown, p));
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
            close
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close"));
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
            if close.clicked() {
                open = false;
            }
        });
    open
}

/// The sector window's text, to select and copy: what is said of the
/// sector, then its data, which scrolls.
fn sector_text(ui: &mut egui::Ui, shown: &Shown, p: &Palette) {
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
    // Lines as far apart as labels, none wrapped.
    let spaced = body.size + ui.spacing().item_spacing.y;
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _: f32| {
        let mut job = egui::text::LayoutJob::default();
        for (i, line) in buffer.as_str().split('\n').enumerate() {
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
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let said_width = layouter(ui, &text.as_str(), 0.0).size().x;
    let dumped = dump(shown.bytes, usize::MAX, shown.base);
    let rows: Vec<&str> = dumped.lines().collect();
    let mono = FontId::monospace(12.0);
    let advance = ui.fonts_mut(|f| f.glyph_width(&mono, '0'));
    let rows_width = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as f32 * advance;
    // A text view's bars, as the Log's: beside the rows, not over them.
    theme::solid_bars(ui, p);
    let bar = ui.spacing().scroll.allocated_width();
    ui.set_min_width(said_width.max(rows_width + bar).ceil() + 1.0);
    let mut shown = text.as_str();
    let said = egui::TextEdit::multiline(&mut shown)
        .layouter(&mut layouter)
        .frame(egui::Frame::NONE)
        .margin(0)
        .desired_rows(1)
        .desired_width(said_width.ceil() + 1.0);
    form::read_only_box(ui, ui.id().with("said"), said);
    if rows.is_empty() {
        return;
    }
    ui.separator();
    let line = |i: usize| (rows[i], plain);
    let lines = Lines {
        count: rows.len(),
        line: &line,
        font: mono,
        gap: 0.0,
    };
    let area = egui::ScrollArea::vertical().max_height(360.0);
    lines.show(ui, ui.id().with("bytes"), area);
}

/// Up to `rows` rows of 16 bytes: the offset, from `base`, the bytes in hex,
/// then as ASCII, others as dots.
fn dump(bytes: &[u8], rows: usize, base: usize) -> String {
    // As many hex digits as the last offset needs, and at least four.
    let last = base + bytes.len().saturating_sub(1);
    let digits = (usize::BITS - last.leading_zeros()).div_ceil(4).max(4) as usize;
    let lines = bytes.chunks(16).take(rows).enumerate().map(|(row, chunk)| {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
        let text: String = chunk
            .iter()
            .map(|&b| {
                if (32..127).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        format!(
            "{:0digits$X}  {:<47}  {text}",
            base + row * 16,
            hex.join(" ")
        )
    });
    lines.collect::<Vec<_>>().join("\n")
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
    if end - start >= least {
        return Some((start, end));
    }
    let middle = (start + end) / 2.0;
    Some((middle - least / 2.0, middle + least / 2.0))
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
    let size = match s.id {
        _ if !s.bytes.is_empty() => Some(s.bytes.len() as u32),
        // The data its header calls for, as gw's decoder reads it.
        Id::Ibm([.., n]) if n <= 7 => Some(128u32 << n),
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
    let mut checks = vec![
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
    ];
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
        _ => format!("{} {unit} {over} cells", grouped(whole as u64)),
    }
}

/// Where a sector lies in bytes: from the index, and after what gw found
/// before it.
fn layout_line(l: &Layout) -> String {
    let mut parts = vec![format!("{} from the index", bytes(l.from_index))];
    if let Some((cells, before)) = l.after {
        let what = match before {
            Before::IndexMark => "the index mark".to_owned(),
            Before::Sector(id, _) => short_id(&id),
            Before::Header(id, _) => format!("{}'s header", short_id(&id)),
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
        let rows = RichText::new(dump(&s.bytes, 4, 0)).monospace().small();
        ui.add(egui::Label::new(rows).extend());
    }
}

/// The disks' legend, laid out: its entries, each where it lies from the
/// legend's top left, and its height, the room above it with it; or to
/// scale, in place of the entries, why the disk holds none of the tracks.
#[derive(Clone)]
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
/// mark and its text, kept on one line, and what it says on a hover.
#[derive(Clone)]
struct Entry {
    lead: Option<Arc<Galley>>,
    mark: Option<Mark>,
    text: Arc<Galley>,
    tip: Option<String>,
    width: f32,
}

impl Legend {
    /// The key to what the disk shows, `drawn`: in the Sectors view, the
    /// sectors drawn by how they decoded, with how many, their ID fields, the
    /// lines where two meet and where none was found, and the tracks read as
    /// flux and not decoded; in the Flux view, its shading. Where no track
    /// shows more than gw's line, each status as the grid's legend counts it.
    /// Then the tracks not known, the index's mark, the tracks to do and the
    /// last reported, and the sectors missing or the retries. With `holds`,
    /// why there are none.
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
            }
        };
        let entries = &mut legend.entries;
        match drawn.shows {
            Shows::Sectors if drawn.pure => {
                let statuses = diskmap::entries(&drawn.statuses, progress, map.verifying, p);
                for (colour, skipped, name, tracks, tip) in statuses {
                    let mark = match skipped {
                        true => Mark::Hole(look.seen(colour), p.line_strong),
                        false => Mark::Swatch(look.seen(colour)),
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
                let (header, data) = (look.seen(look.id(look.alone)), look.seen(look.alone));
                let incomplete = match (drawn.headers_alone, drawn.data_alone) {
                    (true, true) => Mark::Split(header, data),
                    (true, false) => Mark::Swatch(header),
                    _ => Mark::Swatch(data),
                };
                for (class, n) in Class::ALL.into_iter().zip(drawn.sectors) {
                    let mark = match class {
                        Class::Good => Mark::Swatch(look.seen(look.good)),
                        Class::Empty => Mark::Swatch(look.seen(look.empty)),
                        Class::Deleted => Mark::Swatch(look.seen(look.deleted)),
                        Class::BadData => Mark::Swatch(look.seen(look.bad)),
                        Class::BadHeader => Mark::Swatch(look.seen(look.bad_header)),
                        Class::Incomplete => incomplete,
                    };
                    if n > 0 {
                        let text = format!("{} {n}", class.name());
                        entries.push(entry(Some(mark), text, class.tip()));
                    }
                }
                if drawn.id_fields {
                    let mark = Mark::Swatch(look.seen(look.id(look.good)));
                    entries.push(entry(Some(mark), "ID field".into(), None));
                }
                if drawn.meet {
                    let mark = Mark::Line(look.seen(look.good), look.body);
                    entries.push(entry(Some(mark), "Sectors meet".into(), None));
                }
                if drawn.gaps {
                    let mark = Mark::Swatch(look.seen(look.gap));
                    entries.push(entry(Some(mark), "No sector found".into(), None));
                }
                if drawn.flux > 0 {
                    let mark = Mark::Swatch(look.seen(look.flux));
                    let tip = Some("Read as flux, not decoded");
                    let text = format!("Flux {}", diskmap::tracks(drawn.flux));
                    entries.push(entry(Some(mark), text, tip));
                }
            }
            Shows::Flux => {
                let shades = [0.0, 0.5, 1.0, 1.5, 2.0].map(|d| look.seen(look.flux_at(d)));
                let tip = Some("Against the track's average");
                let mut scale = entry(Some(Mark::Shades(shades)), "More flux".into(), tip);
                let less = galley("Less".into(), strong);
                scale.width += less.size().x + gap;
                scale.lead = Some(less);
                entries.push(scale);
            }
        }
        if drawn.unknown > 0 {
            let tip = match drawn.shows {
                Shows::Sectors => "No sector places reported",
                Shows::Flux => "No flux reported",
            };
            let mark = Mark::Swatch(look.seen(look.unknown));
            entries.push(entry(
                Some(mark),
                format!("Not known {}", diskmap::tracks(drawn.unknown)),
                Some(tip),
            ));
        }
        entries.push(entry(Some(Mark::Index(look.index)), "Index".into(), None));
        if drawn.to_do > 0 {
            let mark = Mark::Swatch(look.seen(look.to_do));
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
        let retries = progress.tally().retries;
        if drawn.pure && retries > 0 {
            entries.push(entry(None, diskmap::retry_text(retries), None));
        } else if drawn.missing > 0 && drawn.shows == Shows::Sectors && !drawn.pure {
            let tip = Some("In the format, not found");
            entries.push(entry(None, format!("{} missing", drawn.missing), tip));
        }
        if drawn.shared > 0 && drawn.shows == Shows::Sectors && !drawn.pure {
            let tip = Some("Sectors of one track with the same C, H, R and N");
            entries.push(entry(None, format!("{} share an ID", drawn.shared), tip));
        }
        legend
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
        for e in &self.entries {
            if x > 0.0 && x + e.width > width {
                (x, y) = (0.0, y + row + spacing.y);
            }
            self.at.push(vec2(x, y));
            self.width = self.width.max(x + e.width);
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
                // A note, as a label in a row of them lays its text out:
                // from the row's start, after as much room as lies before it.
                None => {
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        e.text.text().to_owned(),
                        egui::TextStyle::Small.resolve(ui.style()),
                        ui.visuals().weak_text_color(),
                    );
                    job.first_row_min_height = row;
                    job.sections[0].leading_space = offset.x;
                    job.sections[0].format.valign = ui.text_valign();
                    let laid = ui.fonts_mut(|f| f.layout_job(job));
                    let start = egui::pos2(top.x, rect.top());
                    ui.painter().galley(start, laid, Color32::PLACEHOLDER);
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
            Class::Empty => Some("Every byte the same"),
            Class::Deleted => Some("Data mark F8, or F9 on a DEC RX02"),
            Class::BadData => Some("The data's CRC fails"),
            Class::BadHeader => Some("The header's CRC fails"),
            Class::Incomplete => Some("A header with no data, or data with no header"),
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
    let placed: Vec<[f64; 3]> = sectors
        .iter()
        .filter_map(|s| s.at.map(|a| a.map(f64::from)))
        .collect();
    let begins: Vec<f64> = placed.iter().map(|a| a[0].rem_euclid(1.0)).collect();
    let ends: Vec<f64> = placed.iter().map(|a| a[2].rem_euclid(1.0)).collect();
    !meets(&begins, &ends).is_empty()
}

/// Where, of sectors that start at `begins` and end at `ends` round a
/// track, one starts where another ends, as EXACT and MEET tell.
fn meets(begins: &[f64], ends: &[f64]) -> Vec<f64> {
    let within = |near: f64| {
        let starts = begins.iter().copied();
        starts.filter(move |&b| ends.iter().any(|&e| apart(b, e) < near))
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
            geometry,
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
            let index = d.at(0.0, 200.0);
            assert!((index.x - d.centre.x).abs() < 1e-3 && index.y < d.centre.y);
            // A quarter turn on, at the right.
            let quarter = d.at(0.25, 200.0);
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
                        let at = d.at(share, (outer + inner) / 2.0);
                        let (found, at_share) = d.track_at(at).unwrap();
                        assert_eq!(found, cyl, "{media:?}");
                        assert!((at_share - share).abs() < 1e-3, "{at_share} for {share}");
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
        let (begins, ends) = ([0.1, 0.2, 0.3], [0.1981, 0.2981, 0.3981]);
        assert!(meets(&begins, &ends).is_empty());
        // Sectors end to end, one pair across the seam between revolutions.
        let (begins, ends) = ([0.1, 0.2, 0.3], [0.2, 0.2981, 0.4]);
        assert_eq!(meets(&begins, &ends), [0.2, 0.3]);
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
        // Fitted, the tracks fill the disk: as drawn.
        let fit = Look::of(&theme::LIGHT, Media::Fit);
        assert_eq!(fit.seen(fit.good), fit.good);
        // To scale, ECMA-125's 0.115 mm tracks 0.1875 mm apart, erased between.
        let scale = Look::of(&theme::LIGHT, Media::ThreeHalf);
        let share = (0.115 / 0.1875) as f32;
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
            Ring::Sectors(_) => "sectors",
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
        assert_eq!(counted, (1, 2, 1, true), "not in the image: not to do");
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
    fn a_sides_sums_count_each_sector_as_the_legend_does() {
        // A data block found with no header, and a header with no data: two
        // incomplete sectors, the format's two missing.
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
        let mut progress = Progress::blank(vec![0], vec![0]);
        let facts = Facts {
            summary: Some("IBM MFM (0/2 sectors)".into()),
            sectors: vec![data, header],
            missing: vec![Id::Ibm([0, 0, 1, 2]), Id::Ibm([0, 0, 2, 2])],
            ..Facts::default()
        };
        progress.facts.insert((0, 0), facts);
        let (line, encodings) = sums(&progress, 0, 1);
        assert_eq!(line, "Incomplete 2 · 2 missing");
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
        assert_eq!(order(&sectors).as_deref(), Some("1 7 7 7"));
        assert_eq!(repeated(&sectors), ["R7 ×2"]);
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

    /// The pixel at `x`, `y` of `canvas`'s picture as the mean of `n` by `n`
    /// points over it, premultiplied: each the colour of what lies there,
    /// the rim, the hub, a fitted track's line, its row's piece there or a
    /// line where two sectors meet, else the bare disk; outside the disk,
    /// nothing.
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
        // Away from the tracks' edges, whose corners with the line a pixel
        // takes as each alone.
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
            let mut after = tracks_of(span, 0.013, 3, flux);
            // Tracks 0, 17 and 18 and the last change; the rest are as they were.
            let changed = [0usize, 17, 18, span as usize - 1];
            for c in 0..span as usize {
                if !changed.contains(&c) {
                    after[c] = before[c].clone();
                }
            }
            let mut dirty = vec![false; span as usize];
            changed.iter().for_each(|&c| dirty[c] = true);
            let size = [geometry.pixels; 2];
            let mut image = egui::ColorImage::filled(size, Color32::TRANSPARENT);
            Canvas::new(&d, &look, &before, line).paint(&mut image, None);
            Canvas::new(&d, &look, &after, line).paint(&mut image, Some(&dirty));
            let mut whole = egui::ColorImage::filled(size, Color32::TRANSPARENT);
            Canvas::new(&d, &look, &after, line).paint(&mut whole, None);
            let differ = (image.pixels.iter().zip(&whole.pixels)).filter(|(a, b)| a != b);
            assert_eq!(differ.count(), 0, "{media:?} at {room} px");
        }
    }

    #[test]
    fn a_picture_waits_for_its_size_to_hold_while_it_changes() {
        let key = |room: f64| Key {
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
        };
        let stamps = vec![(None, None, true, None); 80];
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
        reported[5] = (None, Some(9), true, None);
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
    fn thousands_are_set_apart() {
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(88_068), "88,068");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }
}

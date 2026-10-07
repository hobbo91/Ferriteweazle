//! The Analyse drawer's view of the disk: each side as a round disk, a ring
//! per track, and on it each sector gw found, where it found it, round from
//! the index at the top, clockwise. Nothing is drawn that gw did not report.

use crate::diskmap;
use crate::form;
use crate::lines::Lines;
use crate::progress::Progress;
use crate::theme::{self, Palette};
use crate::track::{Data, Facts, Header, Id, Sector, Source, Spin};
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, emath::GuiRounding,
    vec2,
};
use std::f64::consts::TAU;
use std::sync::{Arc, Mutex};

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
}

/// The cylinders the disk view spans for `progress` on a disk of `disk`'s.
pub fn span(progress: &Progress, disk: (u32, u32)) -> u32 {
    let (cyls, _) = progress.layout();
    disk.0.max(cyls.last().map_or(0, |c| c + 1))
}

/// Room for a side's name above its disk.
pub(crate) const TITLE: f32 = 20.0;
/// The sector window's title bar, as tall as a macOS window's, its title in
/// the same 13-point type.
const TITLE_BAR: f32 = 28.0;
const TITLE_SIZE: f32 = 13.0;
/// Room for a line of legend under the disks, until it is measured.
const LEGEND: f32 = 22.0;
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
/// Tracks this many pixels apart or more are a whole number of pixels
/// apart; fitted ones this many lines apart or more have a line between them.
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

/// The disk's colours, from the window's palette.
pub(crate) struct Look {
    /// The disk's surface, erased between tracks; and a 3½-inch disk's hub.
    pub(crate) body: Color32,
    pub(crate) hub: Color32,
    pub(crate) rim: Color32,
    /// Where no sector was found on a track gw read.
    pub(crate) gap: Color32,
    /// A track gw is to work on and has not reported.
    pub(crate) pending: Color32,
    pub(crate) good: Color32,
    pub(crate) empty: Color32,
    pub(crate) bad: Color32,
    /// A header with no data after it, or data with no header.
    pub(crate) alone: Color32,
    pub(crate) flux: Color32,
    pub(crate) last: Color32,
    pub(crate) index: Color32,
    /// The palette's strongest colour, furthest from the disk's: round what
    /// the pointer is over, and toward it, ID fields and the most flux.
    pub(crate) ink: Color32,
    /// How much of the disk the tracks cover, to scale: the rest is erased.
    pub(crate) covered: f32,
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
            good: theme::lerp(p.good, body, 0.15),
            empty: theme::lerp(p.good, body, 0.55),
            bad: p.bad,
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
        [self.good, self.empty, self.bad, self.alone][status(s)]
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

/// How the disks fit the room `ui` has left: how many cylinders and sides
/// they show, each disk's width at most, the room their legend takes, and
/// the most height they can use. None with no cylinders to show.
struct Fit {
    span: u32,
    sides: u32,
    legend: f32,
    most: f32,
    /// Each disk as drawn: its picture, its diameter on whole pixels, and
    /// the width of them all with the gaps between.
    geometry: Geometry,
    diameter: f32,
    width: f32,
}

impl Fit {
    fn of(ui: &egui::Ui, map: &Map) -> Option<Fit> {
        let progress = map.progress;
        let (_, heads) = progress.layout();
        let span = span(progress, map.disk);
        let sides = map
            .disk
            .1
            .max(heads.last().map_or(0, |h| h + 1))
            .clamp(1, 2);
        if span == 0 {
            return None;
        }
        let ppp = ui.ctx().pixels_per_point();
        let room = ui.available_size();
        let legend = ui.data(|d| d.get_temp(legend_id(ui))).unwrap_or(LEGEND);
        let n = sides as f32;
        let across = (room.x - SIDE_GAP * (n - 1.0)) / n;
        // The legend's room, and the space between it and the disks.
        let under = ui.spacing().item_spacing.y + legend;
        let widest = Geometry::new(map.media, span, f64::from(across.max(LEAST) * ppp));
        let most = TITLE + widest.pixels as f32 / ppp + under;
        let diameter = across.min(room.y - TITLE - under).max(LEAST);
        let geometry = Geometry::new(map.media, span, f64::from(diameter * ppp));
        let diameter = geometry.pixels as f32 / ppp;
        Some(Fit {
            span,
            sides,
            legend,
            most,
            geometry,
            diameter,
            width: n * diameter + SIDE_GAP * (n - 1.0),
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
    Fit::of(ui, map).map(|f| Place {
        sides: f.sides,
        diameter: f.diameter,
        width: f.width,
    })
}

/// Where the disks' legend keeps its height as measured.
fn legend_id(ui: &egui::Ui) -> egui::Id {
    ui.id().with("disk legend")
}

/// The most height the disks can use in the room `ui` has left, their
/// legend under them: as wide as the room lets them.
pub fn most(ui: &egui::Ui, map: &Map) -> Option<f32> {
    Fit::of(ui, map).map(|f| f.most)
}

/// Draws the disks side by side in the room the drawer gives them, with
/// their legend under them, and gives their response and the most height
/// they can use: as wide as the room lets them.
pub fn show(ui: &mut egui::Ui, map: &Map) -> Option<(egui::Response, f32)> {
    let progress = map.progress;
    let Fit {
        span,
        sides,
        legend,
        most,
        geometry,
        diameter,
        width,
        ..
    } = Fit::of(ui, map)?;
    let p = theme::palette(ui);
    let look = Look::of(p, map.media);
    let ppp = ui.ctx().pixels_per_point();
    let room = ui.available_size();
    let legend_id = legend_id(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(room.x, TITLE + diameter), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Disk map"));
    let painter = ui.painter_at(rect.expand(2.0));
    let left = rect.center().x - width / 2.0;
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
                span,
                rect: picture,
                centre: picture.center(),
                scale: 1.0 / ppp,
                geometry,
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
    let fits = map.media.holds().is_none_or(|n| span <= n);
    for d in &disks {
        d.draw(ui, &painter, map, &look, fits);
    }
    let pointer = response.hover_pos();
    let hovered = pointer.and_then(|at| {
        let d = disks
            .iter()
            .find(|d| (at - d.centre).length() <= diameter / 2.0)?;
        Some((d, d.track_at(at)?))
    });
    if let Some((d, (cyl, share))) = hovered.filter(|_| fits) {
        let key = (cyl, d.side);
        let found = progress.facts.get(&key);
        let least = d.line_at(pointer.unwrap_or_default());
        let at = found.and_then(|f| f.sectors.iter().position(|s| holds(s, share, least)));
        let sector = at.zip(found).map(|(i, f)| &f.sectors[i]);
        let faint = Stroke::new(1.0, look.ink.gamma_multiply(0.45));
        d.outline(&painter, cyl, faint);
        if let Some((start, end)) = sector.and_then(|s| drawn(s, least)) {
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
            .on_hover_ui_at_pointer(|ui| side_tip(ui, map, d.side, d.head));
    }
    inspector(ui.ctx(), map);
    let top = ui.cursor().top();
    let under = Rect::from_min_size(
        egui::pos2(left, top + 6.0),
        vec2(width.max(240.0).min(room.x), ui.available_height().max(0.0)),
    );
    let drawn = ui.scope_builder(egui::UiBuilder::new().max_rect(under), |ui| {
        match map.media.holds().filter(|_| !fits) {
            Some(n) => {
                let text = format!("{span} cylinders: a {} disk holds {n}.", map.media.name());
                ui.label(RichText::new(text).color(p.bad));
            }
            None => legend_rows(ui, map, &look),
        }
    });
    let height = drawn.response.rect.bottom() - top;
    if (height - legend).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(legend_id, height));
        ui.ctx().request_repaint();
    }
    Some((response, most))
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

    /// Draws the disk: its picture, painted again where what it shows has
    /// changed, the index's mark at the top, and round the last track gw
    /// reported, a ring.
    fn draw(&self, ui: &egui::Ui, painter: &egui::Painter, map: &Map, look: &Look, fits: bool) {
        let p = theme::palette(ui);
        let key = Key {
            media: map.media,
            shows: map.shows,
            span: self.span,
            side: self.side,
            head: self.head,
            geometry: self.geometry,
            palette: [
                p.card,
                p.line,
                p.line_strong,
                p.good,
                p.partial,
                p.bad,
                p.flux,
                p.written,
                p.erased,
                p.pending,
            ],
            fits,
        };
        let stamps = stamps(map.progress, self.side, self.span);
        let id = egui::Id::new(("disk picture", self.head));
        let picture = ui.data_mut(|d| d.get_temp_mut_or_default::<Shared>(id).clone());
        let mut picture = picture.lock().expect("one painter at a time");
        let now = ui.input(|i| i.time);
        let same = picture.key == Some(key);
        let resized = !same && picture.key.is_some_and(|k| k.same_but_size(&key));
        let changed: Vec<usize> = (0..self.span as usize)
            .filter(|&c| !same || picture.stamps.get(c) != stamps.get(c))
            .collect();
        let early = now - picture.at < REPAINT;
        if !changed.is_empty() && early && (same || resized) {
            // Painted moments ago: again once the time is up.
            let left = REPAINT - (now - picture.at);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(left));
        } else if !changed.is_empty() {
            let mut rows = std::mem::take(&mut picture.rows);
            rows.resize_with(self.span as usize, Row::default);
            for &cyl in &changed {
                rows[cyl] = row(map, (cyl as u32, self.side), look, p, fits);
            }
            let ppp = ui.ctx().pixels_per_point();
            let canvas = Canvas::new(self, look, &rows, ppp);
            let only = same.then(|| {
                let mut dirty = vec![false; rows.len()];
                changed.iter().for_each(|&c| dirty[c] = true);
                dirty
            });
            let size = [self.geometry.pixels; 2];
            if picture.image.size != size {
                picture.image = egui::ColorImage::filled(size, Color32::TRANSPARENT);
            }
            canvas.paint(&mut picture.image, only.as_deref());
            let image = picture.image.clone();
            match &mut picture.texture {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => {
                    let options = egui::TextureOptions::LINEAR;
                    picture.texture = Some(ui.ctx().load_texture("disk", image, options));
                }
            }
            picture.rows = rows;
            (picture.key, picture.stamps, picture.at) = (Some(key), stamps, now);
        }
        if let Some(texture) = &picture.texture {
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture.id(), self.rect, uv, Color32::WHITE);
        }
        drop(picture);
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

/// What a track shows round it. In the Sectors view, each sector where it
/// was found, its ID field shaded, where none was found the gap's colour,
/// and a line where two meet; in the Flux view, its flux against the track's
/// average. With no report, its status as gw printed it; else, if gw is to
/// work on it, pending; else nothing.
fn row(map: &Map, key: (u32, u32), look: &Look, p: &Palette, fits: bool) -> Row {
    let body = rgb(look.body);
    let mut row = Row::new(body);
    let facts = map.progress.facts.get(&key).filter(|_| fits);
    let spin = facts.and_then(|f| f.flux.as_ref());
    let sectors = facts.map(|f| &f.sectors[..]).unwrap_or_default();
    let placed = sectors.iter().any(|s| s.at.is_some());
    match (map.shows, spin) {
        (Shows::Flux, Some(spin)) => {
            let relative = spin.relative();
            row = Row::pieces(relative.iter().map(|&d| rgb(look.flux_at(d))));
        }
        (Shows::Sectors, Some(_)) => row = Row::new(rgb(look.gap)),
        _ => {}
    }
    if map.shows == Shows::Sectors && placed {
        for s in sectors {
            row.sector(s, look);
        }
    } else if spin.is_none() && !placed && fits {
        match map.progress.tracks.get(&key) {
            // All gw says of this track is its line.
            Some(t) => row = Row::new(rgb(diskmap::status_colour(t.status, p))),
            None if planned(map.progress, key) => row = Row::new(rgb(look.pending)),
            None => {}
        }
    }
    row.finish();
    row
}

/// Whether gw announced it would work on `key`.
fn planned(progress: &Progress, (cyl, head): (u32, u32)) -> bool {
    progress.cyls.contains(&cyl) && progress.heads.contains(&head)
}

/// What a disk's picture is of, beside its tracks.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Key {
    media: Media,
    shows: Shows,
    span: u32,
    side: u32,
    head: u32,
    geometry: Geometry,
    /// The palette's colours the picture shows, which a theme changes.
    palette: [Color32; 10],
    fits: bool,
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

/// A disk's picture, kept between frames.
type Shared = Arc<Mutex<Picture>>;

/// A disk's picture as last painted, its tracks as they were then, and when,
/// in egui's seconds.
#[derive(Default)]
struct Picture {
    key: Option<Key>,
    stamps: Vec<u64>,
    rows: Vec<Row>,
    image: egui::ColorImage,
    texture: Option<egui::TextureHandle>,
    at: f64,
}

/// A number for each of `side`'s tracks that changes as what is known of it
/// does: its status and retries as gw printed them, the bridge's report, and
/// whether gw is to work on it.
fn stamps(progress: &Progress, side: u32, span: u32) -> Vec<u64> {
    use std::hash::{Hash, Hasher};
    (0..span)
        .map(|cyl| {
            let key = (cyl, side);
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            let track = progress.tracks.get(&key).map(|t| (t.status, t.retries));
            let facts = progress.facts.get(&key).map(|f| f.revision);
            (track, facts, planned(progress, key)).hash(&mut hash);
            hash.finish()
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
    fn new(disk: &Disk, look: &Look, rows: &'a [Row], ppp: f32) -> Canvas<'a> {
        let g = disk.geometry;
        let line = f64::from(ppp).round().max(1.0);
        Canvas {
            pixels: g.pixels,
            centre: g.pixels as f64 / 2.0,
            geometry: g,
            outer: g.outer[disk.head as usize],
            rows,
            line,
            separate: g.width == g.pitch && g.pitch >= SEPARATE * line,
            body: rgb(look.body),
            hub: rgb(look.hub),
            rim: rgb(look.rim),
        }
    }

    /// Paints `image`, or where `only` is given, the pixels over those of its
    /// tracks: each pixel in a share of the threads there are.
    fn paint(&self, image: &mut egui::ColorImage, only: Option<&[bool]>) {
        let width = self.pixels;
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
        let band = width.div_ceil(threads).max(1);
        std::thread::scope(|scope| {
            for (i, rows) in image.pixels.chunks_mut(band * width).enumerate() {
                scope.spawn(move || {
                    for (j, pixel) in rows.iter_mut().enumerate() {
                        let (x, y) = (j % width, i * band + j / width);
                        if only.is_none_or(|dirty| self.over(x, y, dirty)) {
                            *pixel = self.pixel(x, y);
                        }
                    }
                });
            }
        });
    }

    /// The pixel's offset from the centre, to its middle, and its distance.
    fn offset(&self, x: usize, y: usize) -> (f64, f64, f64) {
        let (dx, dy) = (x as f64 + 0.5 - self.centre, y as f64 + 0.5 - self.centre);
        (dx, dy, (dx * dx + dy * dy).sqrt())
    }

    /// Whether the pixel lies over the room of any of the tracks marked in
    /// `dirty`, or the lines along its edges.
    fn over(&self, x: usize, y: usize, dirty: &[bool]) -> bool {
        let (_, _, r) = self.offset(x, y);
        let pitch = self.geometry.pitch;
        let reach = 0.5 + self.line;
        let first = ((self.outer - r - reach) / pitch).floor().max(0.0) as usize;
        let last = ((self.outer - r + reach) / pitch).floor();
        last >= 0.0 && dirty.iter().take(last as usize + 1).skip(first).any(|&d| d)
    }

    /// A pixel of the picture, premultiplied: the disk under it, its share
    /// of each track and each line, and its rim and hub.
    fn pixel(&self, x: usize, y: usize) -> Color32 {
        let g = &self.geometry;
        let (dx, dy, r) = self.offset(x, y);
        let (near, far) = (r - 0.5, r + 0.5);
        let cover = (g.edge - near).clamp(0.0, 1.0) * (far - g.hole).clamp(0.0, 1.0);
        if cover <= 0.0 {
            return Color32::TRANSPARENT;
        }
        let mut colour = self.disk(dx, dy, r);
        if let Some(hub) = g.hub {
            colour = mix(colour, self.hub, (hub - near).clamp(0.0, 1.0));
        }
        let rim = (far.min(g.edge) - near.max(g.edge - self.line)).max(0.0);
        colour = mix(colour, self.rim, rim);
        let [r, g, b] = colour.map(|c| (c * cover).round().clamp(0.0, 255.0) as u8);
        Color32::from_rgba_premultiplied(r, g, b, (cover * 255.0).round() as u8)
    }

    /// The disk across the pixel `r` pixels from the centre: each track's
    /// share of it, by how much of its reach across the radius each takes,
    /// the rest bare; then fitted, the lines between tracks.
    fn disk(&self, dx: f64, dy: f64, r: f64) -> [f64; 3] {
        let g = &self.geometry;
        let (near, far) = (r - 0.5, r + 0.5);
        let mut colour = [0.0; 3];
        let mut taken = 0.0;
        let first = ((self.outer - far) / g.pitch).floor().max(0.0) as usize;
        let last = ((self.outer - near) / g.pitch).floor();
        if last >= 0.0 && first < self.rows.len() {
            let last = (last as usize).min(self.rows.len() - 1);
            let share = share_at(dx, dy);
            let width = (1.0 / (TAU * r.max(0.5))).min(1.0);
            for cyl in first..=last {
                let middle = self.outer - (cyl as f64 + 0.5) * g.pitch;
                let (outer, inner) = (middle + g.width / 2.0, middle - g.width / 2.0);
                let part = (far.min(outer) - near.max(inner)).max(0.0);
                if part > 0.0 {
                    let c = self.rows[cyl].sample(share, width, self.line * width, self.body);
                    (0..3).for_each(|i| colour[i] += c[i] * part);
                    taken += part;
                }
            }
        }
        let bare = (1.0 - taken).max(0.0);
        (0..3).for_each(|i| colour[i] += self.body[i] * bare);
        if self.separate && taken > 0.0 {
            // A line in the disk's colour inside each track's outer edge.
            let span = self.rows.len() as f64;
            let from = ((self.outer - far - self.line) / g.pitch).ceil().max(0.0);
            let to = ((self.outer - near) / g.pitch).floor().min(span);
            let mut cover = 0.0;
            let mut k = from;
            while k <= to {
                let b = self.outer - k * g.pitch;
                cover += (far.min(b) - near.max(b - self.line)).max(0.0);
                k += 1.0;
            }
            colour = mix(colour, self.body, cover.min(1.0));
        }
        colour
    }
}

/// A track round a revolution, from the index: the pieces it is made of,
/// each from its start to the next one's, the last to 1, with their running
/// sums for averaging any part; and where two sectors meet.
#[derive(Clone, Default)]
struct Row {
    starts: Vec<f64>,
    colours: Vec<[f64; 3]>,
    sums: Vec<[f64; 3]>,
    /// Where sectors start and end, and where two meet.
    begins: Vec<f64>,
    ends: Vec<f64>,
    meets: Vec<f64>,
    /// Each sector's middle, length and colour, the shortest first: where
    /// one is shorter than a line, it is drawn a line wide.
    slivers: Vec<(f64, f64, [f64; 3])>,
}

impl Row {
    fn new(colour: [f64; 3]) -> Row {
        Row {
            starts: vec![0.0],
            colours: vec![colour],
            ..Row::default()
        }
    }

    /// Equal pieces round the revolution, one per colour.
    fn pieces(colours: impl ExactSizeIterator<Item = [f64; 3]>) -> Row {
        let n = colours.len();
        if n == 0 {
            return Row::new([0.0; 3]);
        }
        Row {
            starts: (0..n).map(|i| i as f64 / n as f64).collect(),
            colours: colours.collect(),
            ..Row::default()
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
    fn lay(&mut self, from: f64, to: f64, colour: [f64; 3]) {
        let from = from.rem_euclid(1.0);
        let to = from + (to - from).min(1.0);
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
        self.lay(start, end, rgb(colour));
        if s.header != Header::None {
            let header_end = s.header_end.map_or(data, f64::from).min(data);
            self.lay(start, header_end, rgb(look.id(colour)));
        }
        // A header alone is all ID field.
        let whole = match (s.header, s.data) {
            (Header::Good | Header::Bad, Data::None) => look.id(colour),
            _ => colour,
        };
        let middle = ((start + end) / 2.0).rem_euclid(1.0);
        self.slivers.push((middle, end - start, rgb(whole)));
        self.begins.push(start.rem_euclid(1.0));
        self.ends.push(end.rem_euclid(1.0));
    }

    /// Makes the running sums, and finds where two sectors meet, once the
    /// row is drawn.
    fn finish(&mut self) {
        let mut sum = [0.0; 3];
        self.sums = Vec::with_capacity(self.starts.len() + 1);
        self.sums.push(sum);
        for (i, c) in self.colours.iter().enumerate() {
            let to = self.starts.get(i + 1).copied().unwrap_or(1.0);
            let width = to - self.starts[i];
            (0..3).for_each(|k| sum[k] += c[k] * width);
            self.sums.push(sum);
        }
        self.meets = meets(&self.begins, &self.ends);
        self.slivers.sort_by(|a, b| a.1.total_cmp(&b.1));
    }

    /// The colour's integral from the index to share `t`, which may lie in
    /// the revolution before or after.
    fn integral(&self, t: f64) -> [f64; 3] {
        let turns = t.floor();
        let t = t - turns;
        let total = self.sums[self.sums.len() - 1];
        let i = self.starts.partition_point(|&s| s <= t).saturating_sub(1);
        let (sum, colour) = (self.sums[i], self.colours[i]);
        let into = t - self.starts[i];
        [0, 1, 2].map(|k| sum[k] + colour[k] * into + total[k] * turns)
    }

    /// The row's colour across `width` of a revolution centred on `share`,
    /// with a line `line` wide in `colour` where two sectors meet, and over
    /// any sector shorter than that, a line in its colour, each by its share
    /// of the width.
    fn sample(&self, share: f64, width: f64, line: f64, colour: [f64; 3]) -> [f64; 3] {
        let (a, b) = (share - width / 2.0, share + width / 2.0);
        let (from, to) = (self.integral(a), self.integral(b));
        let c = [0, 1, 2].map(|k| (to[k] - from[k]) / width);
        let c = mix(c, colour, covered(&self.meets, a, b, line));
        let short = self.slivers.iter().take_while(|s| s.1 < line);
        short.fold(c, |c, &(middle, _, colour)| {
            mix(c, colour, covered(&[middle], a, b, line))
        })
    }
}

/// How much of the span from `a` to `b` of a revolution lines `line` wide,
/// centred on `lines`, cover, round the index if need be.
fn covered(lines: &[f64], a: f64, b: f64, line: f64) -> f64 {
    let half = line / 2.0;
    let mut cover = 0.0;
    for turn in [-1.0, 0.0, 1.0] {
        let (lo, hi) = (a - half - turn, b + half - turn);
        if hi < 0.0 || lo > 1.0 {
            continue;
        }
        let first = lines.partition_point(|&x| x < lo);
        for &x in lines[first..].iter().take_while(|&&x| x <= hi) {
            let x = x + turn;
            cover += ((x + half).min(b) - (x - half).max(a)).max(0.0);
        }
    }
    (cover / (b - a)).min(1.0)
}

fn rgb(c: Color32) -> [f64; 3] {
    [c.r(), c.g(), c.b()].map(f64::from)
}

/// `b` laid over `a` at `alpha`.
fn mix(a: [f64; 3], b: [f64; 3], alpha: f64) -> [f64; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * alpha)
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
    let facts = progress.facts.get(&(cyl, side));
    let track = progress.tracks.get(&(cyl, side));
    ui.strong(format!("Cylinder {cyl} · side {side}"));
    match (facts.and_then(|f| f.summary.as_deref()), track) {
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
    let Some(f) = facts else {
        return;
    };
    if let Some(s) = f.sectors.iter().find(|s| holds(s, share, least)) {
        ui.separator();
        sector_tip(ui, s, true);
        if s.bytes.len() > 64 {
            ui.weak(format!("Click for all {} bytes", s.bytes.len()));
        }
    }
    let unplaced = f.sectors.iter().filter(|s| s.at.is_none()).count();
    if !f.missing.is_empty() || unplaced > 0 {
        ui.separator();
    }
    if !f.missing.is_empty() {
        let ids: Vec<String> = f.missing.iter().map(short_id).collect();
        ui.label(format!("Missing: {}", ids.join(", ")));
    }
    if unplaced > 0 {
        ui.label(format!("{unplaced} sectors not placed"));
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

/// A side's sums: the sectors found and those bad and missing, and the
/// encodings gw decoded.
fn side_tip(ui: &mut egui::Ui, map: &Map, side: u32, head: u32) {
    match side == head {
        true => ui.strong(format!("Side {side}")),
        false => ui.strong(format!("Side {side} · head {head}")),
    };
    let facts = map.progress.facts.iter().filter(|((_, h), _)| *h == side);
    let (mut found, mut bad, mut missing) = (0, 0, 0);
    let mut encodings: Vec<&str> = Vec::new();
    for (_, f) in facts {
        missing += f.missing.len();
        for s in f.sectors.iter().filter(|s| s.data != Data::None) {
            found += 1;
            let good = (s.header, s.data);
            if !matches!(good, (Header::Good, Data::Good | Data::Empty(_))) {
                bad += 1;
            }
        }
        let summary = f.summary.as_deref().unwrap_or_default();
        let encoding = summary.split(" (").next().unwrap_or_default();
        if !encoding.is_empty() && !encodings.contains(&encoding) {
            encodings.push(encoding);
        }
    }
    if found > 0 || missing > 0 {
        ui.label(format!("{found} sectors · {bad} bad · {missing} missing"));
    }
    if !encodings.is_empty() {
        ui.label(encodings.join(", "));
    }
}

/// The id under which the sector whose data is open is kept.
fn inspected() -> egui::Id {
    egui::Id::new("disk sector")
}

/// The window a click on a sector opens: all gw decoded of it, and its
/// data in full.
fn inspector(ctx: &egui::Context, map: &Map) {
    type Inspected = ((u32, u32), usize, u64);
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
        lines: &sector_lines(s),
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

fn holds(s: &Sector, share: f64, least: f64) -> bool {
    drawn(s, least).is_some_and(|(start, end)| {
        [share - 1.0, share, share + 1.0]
            .iter()
            .any(|x| (start..end).contains(x))
    })
}

/// Where a sector is drawn, in shares of a revolution: where it was found,
/// or one shorter than `least`, that much about its middle.
fn drawn(s: &Sector, least: f64) -> Option<(f64, f64)> {
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

/// What is said of a sector, line by line: its ID and size, its checks and
/// mark, its place, and notes.
fn sector_lines(s: &Sector) -> Vec<(String, Tone)> {
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
        let deleted = matches!(mark, 0xf8 | 0xf9);
        checks.push(format!(
            "Mark {mark:02X}{}",
            if deleted { " (deleted)" } else { "" }
        ));
    }
    lines.push((checks.join(" · "), Tone::Plain));
    if let Some([start, data, end]) = s.at {
        let deg = |x: f32| x * 360.0;
        let mut place = format!("{:.1}°–{:.1}°", deg(start), deg(end));
        if data > start {
            place += &format!(" · data {:.1}°", deg(data));
        }
        lines.push((place, Tone::Weak));
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

/// A sector: what is said of it, and with `preview`, its data's first rows.
fn sector_tip(ui: &mut egui::Ui, s: &Sector, preview: bool) {
    for (text, tone) in sector_lines(s) {
        match tone {
            Tone::Strong => ui.strong(text),
            Tone::Plain => ui.label(text),
            Tone::Weak => ui.weak(text),
        };
    }
    if preview && !s.bytes.is_empty() {
        let rows = RichText::new(dump(&s.bytes, 4, 0)).monospace().small();
        ui.add(egui::Label::new(rows).extend());
    }
}

/// The key to what the disk shows: in the Sectors view, the sectors drawn
/// by how they decoded, with how many, their ID fields, the lines where two
/// meet and where none was found; in the Flux view, its shading. Then the index's
/// mark, and the tracks to do and the last reported. With nothing decoded
/// or read, the tracks by their status, as the grid's legend counts them.
fn legend_rows(ui: &mut egui::Ui, map: &Map, look: &Look) {
    let progress = map.progress;
    let all = progress.facts.values().flat_map(|f| f.sectors.iter());
    let mut counts = [0usize; 4];
    for s in all.filter(|s| s.at.is_some()) {
        counts[status(s)] += 1;
    }
    let missing: usize = progress.facts.values().map(|f| f.missing.len()).sum();
    let fluxed = progress.facts.values().any(|f| f.flux.is_some());
    if counts.iter().all(|&n| n == 0) && !fluxed {
        diskmap::legend_for(ui, progress, map.verifying);
        return;
    }
    // Rows as tall as their text, not a field.
    ui.spacing_mut().interact_size.y = ui.text_style_height(&egui::TextStyle::Small).max(10.0);
    ui.horizontal_wrapped(|ui| {
        match map.shows {
            Shows::Sectors => {
                for ((colour, name, tip), n) in [
                    (look.good, "Good", ""),
                    (look.empty, "Empty", "Every byte the same"),
                    (look.bad, "Bad", "A CRC that fails"),
                    (
                        look.alone,
                        "Incomplete",
                        "A header with no data, or data with no header",
                    ),
                ]
                .into_iter()
                .zip(counts)
                {
                    if n > 0 {
                        let swatch = Mark::Swatch(look.seen(colour));
                        let entry = key(ui, swatch, &format!("{name} {n}"));
                        if !tip.is_empty() {
                            entry.on_hover_text(tip);
                        }
                    }
                }
                let placed = || {
                    let all = progress.facts.values().flat_map(|f| f.sectors.iter());
                    all.filter(|s| s.at.is_some())
                };
                if placed().any(|s| s.header_end.is_some()) {
                    key(ui, Mark::Swatch(look.seen(look.id(look.good))), "ID field");
                }
                if progress.facts.values().any(|f| meet(&f.sectors)) {
                    key(
                        ui,
                        Mark::Line(look.seen(look.good), look.body),
                        "Sectors meet",
                    );
                }
                if fluxed {
                    key(ui, Mark::Swatch(look.seen(look.gap)), "No sector found");
                }
            }
            Shows::Flux => {
                let shades = [0.0, 0.5, 1.0, 1.5, 2.0].map(|d| look.seen(look.flux_at(d)));
                ui.label(RichText::new("Less").small());
                key(ui, Mark::Shades(shades), "More flux")
                    .on_hover_text("Against the track's average");
            }
        }
        key(ui, Mark::Index(look.index), "Index");
        let (cyls, heads) = (&progress.cyls, &progress.heads);
        let to_do = cyls
            .iter()
            .flat_map(|&c| heads.iter().map(move |&h| (c, h)))
            .filter(|k| !progress.tracks.contains_key(k) && !progress.facts.contains_key(k))
            .count();
        if to_do > 0 {
            key(
                ui,
                Mark::Swatch(look.seen(look.pending)),
                &format!("To do {to_do}"),
            );
        }
        if map.current.is_some() {
            key(ui, Mark::Ring(look.last), "Last reported");
        }
        if missing > 0 && map.shows == Shows::Sectors {
            ui.label(RichText::new(format!("{missing} missing")).small().weak())
                .on_hover_text("In the format, not found");
        }
    });
}

/// How a sector decoded, as the legend lists them: good, empty, bad, or
/// incomplete.
fn status(s: &Sector) -> usize {
    match (s.header, s.data) {
        (Header::None, _) | (_, Data::None) => 3,
        (Header::Bad, _) | (_, Data::Bad) => 2,
        (_, Data::Empty(_)) => 1,
        (_, Data::Good | Data::Unread) => 0,
    }
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
    let apart = |a: f64, b: f64| {
        let d = (a - b).rem_euclid(1.0);
        d.min(1.0 - d)
    };
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
pub(crate) enum Mark {
    /// A square of the colour.
    Swatch(Color32),
    /// The colours side by side, as a scale.
    Shades([Color32; 5]),
    /// A line across a sector's colour, as where two sectors meet.
    Line(Color32, Color32),
    /// The index's notch.
    Index(Color32),
    /// A ring, as round the last track reported.
    Ring(Color32),
    /// A frame, as round the image's track last reported.
    Frame(Color32),
}

/// A legend entry: its mark and its text, kept on one line.
pub(crate) fn key(ui: &mut egui::Ui, mark: Mark, text: &str) -> egui::Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, ui.visuals().text_color());
    let wide = match mark {
        Mark::Shades(_) => 40.0,
        _ => 10.0,
    };
    let gap = ui.spacing().item_spacing.x;
    let size = vec2(wide + gap + galley.size().x, galley.size().y.max(10.0));
    if ui.available_width() < size.x {
        ui.end_row();
    }
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    let square = Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - 5.0),
        vec2(wide, 10.0),
    );
    let painter = ui.painter();
    match mark {
        Mark::Swatch(colour) => {
            painter.rect_filled(square, 2.0, colour);
        }
        Mark::Shades(colours) => {
            let step = wide / colours.len() as f32;
            for (i, colour) in colours.into_iter().enumerate() {
                let x = square.left() + step * i as f32;
                let part = Rect::from_min_size(egui::pos2(x, square.top()), vec2(step, 10.0));
                painter.rect_filled(part, 0.0, colour);
            }
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
    let at = egui::pos2(
        square.right() + gap,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(at, galley, ui.visuals().text_color());
    ui.add_space(6.0);
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
        }
    }

    fn row_of(sectors: &[Sector]) -> Row {
        let look = Look::of(&theme::DARK, Media::Fit);
        let mut row = Row::new(rgb(look.gap));
        for s in sectors {
            row.sector(s, &look);
        }
        row.finish();
        row
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
        assert!(near(row.sample(0.2578125, w, w, line), id));
        assert!(near(row.sample(0.2734375, w, w, line), good));
        assert!(near(row.sample(0.375, w, w, line), good));
        assert!(near(row.sample(0.75, w, w, line), gap));
        // Pixels straddling the ID field's end and the sector's: half each.
        let edge = row.sample(0.265625, w, w, line);
        assert!(near(edge, mix(id, good, 0.5)), "{edge:?}");
        let edge = row.sample(0.5, w, w, line);
        assert!(near(edge, mix(good, gap, 0.5)), "{edge:?}");
        // Over the index, the row wraps.
        let over = row_of(&[sector([0.9, 0.9, 1.1], None)]);
        assert!(near(over.sample(0.05, w, w, line), good));
        assert!(near(over.sample(0.15, w, w, line), gap));
        assert!(near(over.sample(0.0, 0.02, 0.0, line), good));
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
        assert!(near(row.sample(0.25, w, w, line), line), "where two meet");
        // Where a sector starts after a gap, no line.
        let look = Look::of(&theme::DARK, Media::Fit);
        let edge = row.sample(0.5, w, w, line);
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
        assert!(near(row.sample(middle, w, w, line), rgb(look.alone)));
        assert!(near(row.sample(middle + w, w, w, line), rgb(look.gap)));
        // Half a line either side of its middle, the pointer finds it.
        assert!(holds(&mark, middle + 0.4 * w, w));
        assert!(!holds(&mark, middle + 0.6 * w, w));
        // A sector a line long or more is found only where it lies.
        let whole = sector([0.25, 0.25, 0.5], None);
        assert!(holds(&whole, 0.25, w) && !holds(&whole, 0.25 - w / 4.0, w));
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
        let mut row = Row::new(rgb(look.good));
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

    #[test]
    fn thousands_are_set_apart() {
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(88_068), "88,068");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }
}

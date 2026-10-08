//! The Analyse drawer's view of the image a job makes or takes its tracks
//! from: its file as gw lays it out, a row for each track in the file's
//! order, each of its sectors' parts as wide as its bytes, coloured by what
//! it holds as the job goes. Nothing is drawn that gw did not report.

use crate::image::{self, Image, Part, Placed, Role, State};
use crate::progress::Progress;
use crate::surface::{self, Look, Mark, Media, Place, Shown, TITLE, Tone};
use crate::theme::{self, Palette};
use crate::track::Id;
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, emath::GuiRounding, vec2,
};

/// What the image view draws: the image, and the progress of the job that
/// has it, if one has; none before a job takes its tracks from it.
pub struct Map<'a> {
    pub progress: Option<&'a Progress>,
    pub image: &'a Image,
    pub running: bool,
    /// The job converts, rather than reads or writes.
    pub converts: bool,
}

/// A row's height at most, in points, and the least before the file runs
/// on in another column, where the width holds one.
const ROW_MOST: f32 = 12.0;
const ROW_LEAST: f32 = 4.0;
/// A column's rows' width at least, in points, and in pixels for each of
/// a track's parts.
const COLUMN_LEAST: f32 = 64.0;
const PART_LEAST: f32 = 4.0;
/// From a column's rows to the next column's offsets, and from an offset
/// to its row.
const COLUMN_GAP: f32 = 12.0;
const OFFSET_GAP: f32 = 8.0;
/// The offsets' type, and how far apart they are down a column at least.
const OFFSET_SIZE: f32 = 11.0;
const OFFSET_EVERY: f32 = 16.0;
/// Room for the legend under the rows until it is measured.
const LEGEND: f32 = 22.0;
/// How long the rows take to fade from one layout to another, in seconds.
const REFLOW: f64 = 0.2;

/// The image the view shows for `progress`'s job: the one it makes if gw
/// lays that out, else the one it takes its tracks from; or why there is
/// none to show.
pub fn shown(progress: &Progress) -> Result<&Image, String> {
    let images = [progress.made.as_ref(), progress.source.as_ref()];
    if let Some(image) = images.iter().flatten().find(|i| i.layout.is_some()) {
        return Ok(image);
    }
    match images.iter().flatten().next() {
        Some(image) => Err(not_laid_out(image)),
        None => Err("No image reported.".to_owned()),
    }
}

/// Why gw's report lays out no part of an image: its type, or a file of a
/// type gw lays out that is not laid out so.
pub(crate) fn not_laid_out(image: &Image) -> String {
    let file = image.file.as_deref().unwrap_or(&image.kind);
    match image.differs {
        true => {
            let name = extension(file).unwrap_or_else(|| file.to_owned());
            format!("Not mapped: the file is not as gw lays out {name}.")
        }
        false => not_mapped(file),
    }
}

/// Why no part of the image `file` is laid out, gw not laying it out as a
/// sector image: by its type, from its extension, else gw's name for it.
pub(crate) fn not_mapped(file: &str) -> String {
    let name = extension(file).unwrap_or_else(|| file.to_owned());
    match crate::form::track_holds(&name) {
        Some(holds) => format!("Not mapped: {name} holds {holds}, not sectors."),
        None => format!("Not mapped: gw lays out {name} its own way."),
    }
}

/// Whether the image `file` is of a type that holds tracks, as flux or
/// bitcells, not sectors.
pub(crate) fn holds_tracks(file: &str) -> bool {
    extension(file).is_some_and(|e| crate::form::track_holds(&e).is_some())
}

/// A file's extension as gw's image types name it, such as `.scp`.
fn extension(file: &str) -> Option<String> {
    let ext = std::path::Path::new(file).extension()?;
    Some(format!(".{}", ext.to_string_lossy().to_lowercase()))
}

/// Draws the image's file in the room the drawer gives it, where the disks
/// lie in the disk view, `disks`, with its legend under it.
pub fn show(ui: &mut egui::Ui, map: &Map, disks: Option<Place>) {
    let p = theme::palette(ui);
    let look = Look::of(p, Media::Fit);
    let work = job(map);
    let Some(placed) = map.image.placed(work.as_ref()) else {
        ui.label(RichText::new(header(map)).small().color(p.dim));
        let text = match map.image.tracks.values().any(|t| !t.laid) {
            true => "Not as laid out: gw puts the input's own tracks in it.",
            false => "Not as laid out: gw wrote the file another way.",
        };
        ui.label(RichText::new(text).color(p.partial));
        return;
    };
    if placed.is_empty() {
        ui.label(RichText::new(header(map)).small().color(p.dim));
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let legend_id = ui.id().with("image legend");
    let legend = ui.data(|d| d.get_temp(legend_id)).unwrap_or(LEGEND);
    let room = ui.available_size();
    let under = ui.spacing().item_spacing.y + legend;
    let digits = offset_digits(map.image, &placed);
    let font = FontId::monospace(OFFSET_SIZE);
    let label = ui.fonts_mut(|f| f.glyph_width(&font, '0')) * digits as f32 + OFFSET_GAP;
    // The disks' room, or with none, as wide and tall as there is.
    let sides = disks.map_or_else(|| sides_of(&placed), |d| d.sides as usize);
    let height = disks
        .map_or(f32::INFINITY, |d| d.diameter)
        .min(room.y - TITLE - under)
        .max(ROW_LEAST);
    let (area, _) = ui.allocate_exact_size(vec2(room.x, TITLE + height), Sense::hover());
    let picture = Rect::from_min_max(egui::pos2(area.left(), area.top() + TITLE), area.max);
    let widest = placed.iter().map(|t| t.len).max().unwrap_or(1).max(1);
    let plan = Plan {
        tracks: placed.len(),
        sides,
        parts: placed.iter().map(|t| t.parts.len()).max().unwrap_or(1),
        units: units(&placed, widest),
        column: disks.map(|d| d.diameter),
        label,
    };
    let grid = Grid::new(&plan, picture, ppp);
    let layers = flow(ui, grid, picture.min);
    let rows: Vec<Rect> = (0..placed.len()).map(|i| grid.row(i)).collect();
    let bounds = rows.iter().fold(Rect::NOTHING, |b, r| b.union(*r));
    let response = ui.interact(bounds, ui.id().with("image map"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Image map"));
    // The file's name, size and state over the rows, as the sides' names
    // are over the disks.
    let title = Rect::from_min_size(
        egui::pos2(grid.origin.x, area.top()),
        vec2(area.right() - grid.origin.x, TITLE),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(title)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| title_label(ui, map, title.width(), p),
    );
    // Rows flowing from a taller layout are cut at the box; marks round
    // them may reach a little past it.
    let painter = ui.painter_at(area);
    let marks = ui.painter_at(area.expand(2.0));
    // An offset's digits from the top of the row it names.
    let ink = offset_ink(ui, &font);
    for (layer, opacity) in &layers {
        for (i, track) in placed.iter().enumerate() {
            let row = layer.row(i);
            for (part, at, state) in &track.parts {
                let cell = drawn(row, track, *at, part.len, widest, ppp);
                let mut fill = colour(*state, &look, p);
                if !track.kept {
                    fill = fill.gamma_multiply(0.5);
                }
                painter.rect_filled(cell, 0.0, fill.gamma_multiply(*opacity));
            }
            if (i % layer.rows).is_multiple_of(layer.every) {
                let at = egui::pos2(row.left() - OFFSET_GAP, row.top() - ink);
                let text = format!("{:0digits$X}", track.start);
                let colour = p.dim.gamma_multiply(*opacity);
                painter.text(at, Align2::RIGHT_TOP, text, font.clone(), colour);
            }
        }
    }
    // Where gw is in the image while it works, as the disk view rings it.
    let current = work.as_ref().and_then(|j| j.current(map.image.role));
    let reported = placed
        .iter()
        .zip(&rows)
        .find(|(t, _)| Some(t.key) == current);
    if let Some((track, row)) = reported {
        let line = drawn(*row, track, track.start, track.len, widest, ppp);
        let stroke = egui::Stroke::new(1.5, look.last);
        marks.rect_stroke(line, 0.0, stroke, egui::StrokeKind::Outside);
    }
    let hovered = response
        .hover_pos()
        .and_then(|at| hit(&rows, &placed, widest, at));
    if let Some((t, k)) = hovered {
        let track = &placed[t];
        let (part, at, state) = &track.parts[k];
        let cell = drawn(rows[t], track, *at, part.len, widest, ppp);
        // Outlined round its edges as drawn, as the disk view outlines a sector.
        let stroke = egui::Stroke::new(1.0, look.ink);
        marks.rect_stroke(cell, 0.0, stroke, egui::StrokeKind::Outside);
        let lines = said(map, track, part, *at, *state, digits);
        response.clone().on_hover_ui_at_pointer(|ui| {
            for (text, tone) in &lines {
                tone.label(ui, text);
            }
        });
        if response.clicked() {
            // Another part in the window open keeps its place and size.
            let was = ui.data(|d| d.get_temp::<Opened>(opened_id()));
            let opened = Opened {
                key: track.key,
                part: k,
                at: *at,
                held: held(map, track, k),
                opened: was.map_or_else(|| surface::opening(ui.ctx()), |was| was.opened),
            };
            ui.data_mut(|d| d.insert_temp(opened_id(), opened));
        }
    }
    window(ui.ctx(), map, &placed, digits);
    // The legend under the first column, as wide as a column a side would
    // reach: its height sets the rows' room, so its width must not hang on
    // the columns that room gives.
    let top = area.bottom();
    let fewest = Grid::with(&plan, picture, ppp, plan.sides);
    let wide = area.right() - fewest.origin.x;
    let left = grid.origin.x.min(fewest.origin.x);
    let key = ui.scope_builder(
        egui::UiBuilder::new().max_rect(Rect::from_min_max(
            egui::pos2(left, top + 6.0),
            egui::pos2(left + wide, top + 6.0 + ui.available_height().max(0.0)),
        )),
        |ui| legend_rows(ui, map, &placed, reported.is_some(), &look, p),
    );
    let height = key.response.rect.bottom() - top;
    if (height - legend).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(legend_id, height));
        ui.ctx().request_repaint();
    }
}

/// How far the job has gone with the image, if one has it.
fn job<'a>(map: &Map<'a>) -> Option<image::Job<'a>> {
    map.progress.map(|progress| image::Job {
        progress,
        running: map.running,
    })
}

/// The line over the rows: the file, its size, and where gw is with it.
fn header(map: &Map) -> String {
    let (name, rest) = header_parts(map);
    name + &rest
}

/// The line over the rows in two: the file's name, and after it its size
/// and where gw is with it.
fn header_parts(map: &Map) -> (String, String) {
    let image = map.image;
    let name = image
        .file
        .as_deref()
        .map(|f| {
            std::path::Path::new(f)
                .file_name()
                .map_or(f.to_owned(), |n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| image.kind.clone());
    // gw empties a file it makes as it begins and fills it as it finishes,
    // or on an error deletes it: till then its size is only the layout's,
    // while gw works, and none once it stops.
    let unwritten = image.role == Role::Made && image.written.is_none();
    let size = image.bytes().and_then(|b| {
        let bytes = surface::grouped(b);
        match (unwritten, map.running) {
            (false, _) => Some(format!("{bytes} bytes")),
            (true, true) => Some(format!("{bytes} bytes as laid out")),
            (true, false) => None,
        }
    });
    let unread = image
        .unread()
        .map(|n| format!("{} past the layout", surface::grouped(n)));
    let state = match image.role {
        Role::Made if image.written.is_some() => "Written by gw",
        Role::Made if map.running => "Being made: gw writes it when it finishes",
        Role::Made => "Not written",
        Role::Source if map.progress.is_none() => "As gw reads it",
        Role::Source => "As gw read it",
    };
    let rest: String = [size, unread, Some(state.to_owned())]
        .into_iter()
        .flatten()
        .map(|part| format!(" · {part}"))
        .collect();
    (name, rest)
}

/// The line over the rows on one line `width` wide: a name too long for
/// it, with the rest after it, cut in the middle, as the status pane cuts
/// one, and whole on hover.
fn title_label(ui: &mut egui::Ui, map: &Map, width: f32, p: &Palette) {
    let (name, rest) = header_parts(map);
    let small = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(rest.clone(), small.clone(), Color32::PLACEHOLDER);
    let cut = crate::app::cut_middle(ui, &name, &small, width - galley.size().x);
    let label = ui.label(RichText::new(format!("{cut}{rest}")).small().color(p.dim));
    if cut != name.as_str() {
        label.on_hover_text(name);
    }
}

/// How many sides an image's tracks are on, with no disks to go by.
fn sides_of(placed: &[Placed]) -> usize {
    let heads = placed.iter().map(|t| t.key.1).max().unwrap_or(0);
    (heads as usize + 1).clamp(1, 2)
}

/// How far below the top of an offset's text its digits begin.
fn offset_ink(ui: &egui::Ui, font: &FontId) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap("0".into(), font.clone(), Color32::WHITE);
    galley
        .rows
        .first()
        .and_then(|r| {
            let glyph = r.row.glyphs.first()?;
            Some(r.pos.y + glyph.pos.y + glyph.uv_rect.offset.y)
        })
        .unwrap_or(0.0)
        .max(0.0)
}

/// What the rows are laid out for: how many tracks, the sides of the disk
/// and its tracks' parts at most; how many units the widest track's bytes
/// make, of the most bytes every part's length is a multiple of; the disks'
/// diameter where they are drawn, and the width of a column's offsets with
/// their gap.
struct Plan {
    tracks: usize,
    sides: usize,
    parts: usize,
    units: u64,
    column: Option<f32>,
    label: f32,
}

impl Plan {
    /// A column's rows' width at least: as wide as its parts need.
    fn least(&self, ppp: f32) -> f32 {
        COLUMN_LEAST.max(self.parts as f32 * PART_LEAST / ppp)
    }
}

/// The rows the file runs down, where the disks lie: in a column for each
/// side of the disk, or more where rows would be thinner than ROW_LEAST,
/// but no more than the room's width holds as wide as their parts need;
/// each row a whole number of pixels tall, the rows centred in the room's
/// height; each column as wide as a disk, or the room's width shared out
/// where no disks are drawn, centred, each with its offsets before it; and
/// each a whole number of pixels for each unit of the widest track's
/// bytes, so that parts alike are drawn alike.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Grid {
    rows: usize,
    columns: usize,
    row: f32,
    column: f32,
    /// The first row's top left.
    origin: Pos2,
    /// From one column's rows to the next's.
    pitch: f32,
    /// An offset every this many rows.
    every: usize,
}

impl Grid {
    fn new(plan: &Plan, room: Rect, ppp: f32) -> Grid {
        let fits = (room.height() / ROW_LEAST).floor().max(1.0) as usize;
        Grid::with(plan, room, ppp, plan.tracks.max(1).div_ceil(fits))
    }

    /// The rows in `columns` columns, or one for each side if that is
    /// more, or what the room's width holds if that is fewer.
    fn with(plan: &Plan, room: Rect, ppp: f32, columns: usize) -> Grid {
        let tracks = plan.tracks.max(1);
        let between = COLUMN_GAP + plan.label;
        let least = plan.least(ppp);
        let room_width = room.width() - plan.label;
        let most = ((room_width + between) / (least + between))
            .floor()
            .max(1.0) as usize;
        let columns = columns.max(plan.sides).min(tracks).min(most).max(1);
        let rows = tracks.div_ceil(columns);
        let height = room.height();
        let row = ((height / rows as f32).min(ROW_MOST) * ppp)
            .floor()
            .max(1.0)
            / ppp;
        let n = columns as f32;
        let shared = (room_width - (n - 1.0) * between) / n;
        let wide = plan.column.unwrap_or(shared).max(least);
        let column = uniform(wide.min(shared).max(1.0), (least, shared), plan.units, ppp);
        let width = n * column + (n - 1.0) * between;
        let left = (room.center().x - width / 2.0).max(room.left() + plan.label);
        // Rows that cannot be thinner run past the room's foot, not its top.
        let spare = (height - row * rows as f32).max(0.0);
        let top = room.top() + (spare / 2.0 * ppp).floor() / ppp;
        Grid {
            rows,
            columns,
            row,
            column,
            origin: egui::pos2((left * ppp).round() / ppp, top),
            pitch: column + between,
            every: (OFFSET_EVERY / row).ceil().max(1.0) as usize,
        }
    }

    /// Where track `i`'s row lies.
    fn row(&self, i: usize) -> Rect {
        let (column, row) = (i / self.rows, i % self.rows);
        let at = vec2(column as f32 * self.pitch, row as f32 * self.row);
        Rect::from_min_size(self.origin + at, vec2(self.column, self.row))
    }
}

/// `width` cut to whole pixels, a whole number for each of `units` where
/// there is a pixel for each, so parts alike are drawn alike; a unit's
/// worth more where the cut is under `least` and that fits `most`.
fn uniform(width: f32, (least, most): (f32, f32), units: u64, ppp: f32) -> f32 {
    let pixels = (width * ppp).floor().max(1.0);
    let units = units.max(1) as f32;
    if pixels < units {
        return pixels / ppp;
    }
    let fewer = (pixels / units).floor() * units;
    let more = fewer + units;
    match fewer < (least * ppp).ceil() && more <= (most * ppp).floor() {
        true => more / ppp,
        false => fewer / ppp,
    }
}

/// `widest` in units of the parts' lengths' greatest common divisor.
fn units(placed: &[Placed], widest: u64) -> u64 {
    let gcd = |mut a: u64, mut b: u64| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    let lens = placed.iter().flat_map(|t| &t.parts).map(|(p, _, _)| p.len);
    match lens.fold(0, gcd) {
        0 => 1,
        unit => (widest / unit).max(1),
    }
}

/// How the rows go from one layout to another: the one they were in when
/// the columns last changed, and since when; and the frame they were last
/// drawn in.
#[derive(Clone, Copy)]
struct Flow {
    grid: Grid,
    from: Option<Grid>,
    start: f64,
    frame: u64,
}

/// The layouts to draw the rows in this frame, each with its opacity:
/// `grid`'s, and for REFLOW after the columns change, the one before it
/// fading out as `grid`'s fades in, where it lay in the box at `corner`.
fn flow(ui: &egui::Ui, grid: Grid, corner: Pos2) -> Vec<(Grid, f32)> {
    // Kept from the box's corner, which moves as the drawer does.
    let grid = Grid {
        origin: grid.origin - corner.to_vec2(),
        ..grid
    };
    let placed = |g: Grid| Grid {
        origin: g.origin + corner.to_vec2(),
        ..g
    };
    let id = ui.id().with("image flow");
    let (now, frame) = (ui.input(|i| i.time), ui.ctx().cumulative_frame_nr());
    let was: Option<Flow> = ui.data(|d| d.get_temp(id));
    // Drawn the frame before, the rows go on from there; else they start here.
    let mut flow = was.filter(|f| f.frame + 1 >= frame).unwrap_or(Flow {
        grid,
        from: None,
        start: f64::NEG_INFINITY,
        frame,
    });
    if flow.grid.columns != grid.columns {
        flow.from = Some(flow.grid);
        flow.start = now;
    }
    flow.grid = grid;
    flow.frame = frame;
    let t = ((now - flow.start) / REFLOW).clamp(0.0, 1.0) as f32;
    let eased = t * t * (3.0 - 2.0 * t);
    ui.data_mut(|d| d.insert_temp(id, flow));
    match flow.from.filter(|_| t < 1.0) {
        Some(from) => {
            ui.ctx().request_repaint();
            vec![(placed(from), 1.0 - eased), (placed(grid), eased)]
        }
        None => vec![(placed(grid), 1.0)],
    }
}

/// A part `len` bytes long at `at` of `track` in its row, as drawn: on
/// whole pixels, a pixel short of the next part and row where there is
/// room for a gap.
fn drawn(row: Rect, track: &Placed, at: u64, len: u64, widest: u64, ppp: f32) -> Rect {
    let scale = row.width() as f64 / widest as f64;
    let x = |b: u64| row.left() + ((b - track.start) as f64 * scale) as f32;
    let cell = Rect::from_x_y_ranges(x(at)..=x(at + len), row.y_range()).round_to_pixels(ppp);
    let gap = 1.0 / ppp;
    Rect::from_min_max(
        cell.min,
        cell.max - vec2(gap_if(cell.width(), gap), gap_if(cell.height(), gap)),
    )
}

/// A gap of `gap` where `size` has room for one.
fn gap_if(size: f32, gap: f32) -> f32 {
    if size >= 4.0 * gap { gap } else { 0.0 }
}

/// The track and part under `at`, its row as drawn.
fn hit(rows: &[Rect], placed: &[Placed], widest: u64, at: Pos2) -> Option<(usize, usize)> {
    let t = rows.iter().position(|r| r.contains(at))?;
    let (row, track) = (rows[t], &placed[t]);
    let byte =
        track.start + ((at.x - row.left()) as f64 / row.width() as f64 * widest as f64) as u64;
    let k = track
        .parts
        .iter()
        .position(|(p, start, _)| (*start..start + p.len).contains(&byte))?;
    Some((t, k))
}

fn colour(state: State, look: &Look, p: &Palette) -> Color32 {
    match state {
        State::Data => look.good,
        State::Filler => look.bad,
        State::Unread => p.erased,
        State::ToDo => look.pending,
        State::PastEnd => look.alone,
    }
}

/// What a part holds, said in a line.
fn state_text(state: State, map: &Map) -> &'static str {
    let (source, converts) = (map.image.role == Role::Source, map.converts);
    match state {
        State::Data if source => "Data in the file",
        State::Data if converts => "Data from the input",
        State::Data => "Data from the disk",
        State::Filler if source => "gw's filler, in the file",
        State::Filler if converts => "gw's filler: the sector did not decode",
        State::Filler => "gw's filler: the sector did not read",
        State::Unread if converts => "gw's filler: the track was not converted",
        State::Unread => "gw's filler: the track was not read",
        State::ToDo if converts => "To convert",
        State::ToDo => "To read",
        State::PastEnd => "Past the file's end: gw takes zeros",
    }
}

/// What the tip and the window say of a part: its sector, where it lies in
/// the file, what it holds, and its track.
fn said(
    map: &Map,
    track: &Placed,
    part: &Part,
    at: u64,
    state: State,
    digits: usize,
) -> Vec<(String, Tone)> {
    let (cyl, side) = track.key;
    let mut lines = vec![
        (
            format!(
                "{} · {} bytes",
                sector_name(part),
                surface::grouped(part.len)
            ),
            Tone::Strong,
        ),
        (
            format!(
                "{at:0digits$X}–{:0digits$X} · cylinder {cyl}, side {side}",
                at + part.len - 1
            ),
            Tone::Plain,
        ),
        (state_text(state, map).to_owned(), Tone::Weak),
    ];
    // Where gw's track lists move a track, its track lines name it apart.
    let named: Vec<String> = job(map)
        .map(|j| j.named(map.image.role, track.key))
        .unwrap_or_default()
        .iter()
        .map(|(c, h)| format!("gw's T{c}.{h}"))
        .collect();
    if !named.is_empty() {
        lines.push((named.join(" · "), Tone::Weak));
    }
    lines
}

/// A part's sector by its ID, as the disk view names it.
fn id_text(part: &Part) -> String {
    surface::id_text(&match part.id {
        Some(id) => Id::Ibm(id),
        None => Id::Number(part.index as u32),
    })
}

fn sector_name(part: &Part) -> String {
    match part.id {
        Some(_) => format!("Sector {}", id_text(part)),
        None => id_text(part),
    }
}

/// The part a click opened a window on: its track, by key, its place in
/// the track, and where it lies in the file; what it held then; and which
/// opening of the window it is in.
#[derive(Clone, PartialEq)]
struct Opened {
    key: (u32, u32),
    part: usize,
    at: u64,
    held: Held,
    opened: u64,
}

/// What a part holds: the image it is part of, by what the job does with
/// it, its file and gw's type for it; and the part's state and bytes. The
/// window on a part shuts once these are another's, as for another file
/// or job, or the part holds other bytes.
#[derive(Clone, PartialEq)]
struct Held {
    role: Role,
    file: Option<String>,
    kind: String,
    state: State,
    bytes: Option<Vec<u8>>,
}

/// What `track`'s part `k` of `map`'s image holds.
fn held(map: &Map, track: &Placed, k: usize) -> Held {
    let image = map.image;
    Held {
        role: image.role,
        file: image.file.clone(),
        kind: image.kind.clone(),
        state: track.parts[k].2,
        bytes: image.part_bytes(track, k),
    }
}

fn opened_id() -> egui::Id {
    egui::Id::new("image part")
}

/// The window a click on a part opens: what it holds, and its bytes in full
/// where gw reported them, numbered from where they lie in the file.
fn window(ctx: &egui::Context, map: &Map, placed: &[Placed], digits: usize) {
    let Some(opened) = ctx.data(|d| d.get_temp::<Opened>(opened_id())) else {
        return;
    };
    let found = placed
        .iter()
        .find(|t| t.key == opened.key)
        .filter(|t| t.parts.get(opened.part).is_some_and(|p| p.1 == opened.at))
        .map(|t| (t, held(map, t, opened.part)))
        .filter(|(_, now)| *now == opened.held);
    let shut = || {
        ctx.data_mut(|d| d.remove::<Opened>(opened_id()));
        surface::forget(ctx);
    };
    let Some((track, now)) = found else {
        return shut();
    };
    let (part, at, state) = &track.parts[opened.part];
    let (cyl, side) = track.key;
    let title = format!("{} · cylinder {cyl}, side {side}", id_text(part));
    let lines = said(map, track, part, *at, *state, digits);
    let bytes = now.bytes.unwrap_or_default();
    let shown = Shown {
        title: &title,
        lines: &lines,
        bytes: &bytes,
        base: *at as usize,
        nav: None,
    };
    let id = egui::Id::new("image part window").with(opened.opened);
    if surface::sector_window(ctx, id, &shown).close {
        shut();
    }
}

/// How many hex digits an offset into a file of `size` bytes needs, and at least four.
fn hex_digits(size: u64) -> usize {
    let last = size.saturating_sub(1);
    ((u64::BITS - last.leading_zeros()).div_ceil(4).max(4)) as usize
}

/// How many hex digits the offsets into `image` need: to the end of its
/// file or of its tracks as `placed`, whichever is further, as a source
/// shorter than its layout has tracks past its end.
fn offset_digits(image: &Image, placed: &[Placed]) -> usize {
    let end = placed.iter().map(|t| t.start + t.len).max().unwrap_or(0);
    hex_digits(end.max(image.bytes().unwrap_or(0)))
}

/// The key to what the parts hold, with how many of each, and while gw
/// works, the mark on the track it last reported, where it is drawn.
fn legend_rows(
    ui: &mut egui::Ui,
    map: &Map,
    placed: &[Placed],
    reported: bool,
    look: &Look,
    p: &Palette,
) {
    let mut counts: Vec<(State, usize)> = Vec::new();
    for (_, _, state) in placed.iter().flat_map(|t| t.parts.iter()) {
        match counts.iter_mut().find(|(s, _)| s == state) {
            Some((_, n)) => *n += 1,
            None => counts.push((*state, 1)),
        }
    }
    let order = [
        State::Data,
        State::Filler,
        State::Unread,
        State::PastEnd,
        State::ToDo,
    ];
    ui.spacing_mut().interact_size.y = ui.text_style_height(&egui::TextStyle::Small).max(10.0);
    ui.horizontal_wrapped(|ui| {
        for state in order {
            let Some(&(_, n)) = counts.iter().find(|(s, _)| *s == state) else {
                continue;
            };
            let name = match state {
                State::Data => "Data",
                State::Filler => "Filler",
                State::Unread if map.converts => "Not converted",
                State::Unread => "Not read",
                State::PastEnd => "Past the end",
                State::ToDo => "To do",
            };
            let entry = surface::key(
                ui,
                Mark::Swatch(colour(state, look, p)),
                &format!("{name} {n}"),
            );
            // Its fillers found only once the tip shows.
            if state == State::Filler {
                entry.on_hover_ui(|ui| {
                    ui.set_max_width(ui.spacing().tooltip_width);
                    ui.label(filler_tip(map, placed));
                });
            }
        }
        if reported {
            surface::key(ui, Mark::Frame(look.last), "Last reported");
        }
        let least = map.image.layout.as_ref().and_then(|l| l.min_cyls);
        if let Some(least) = least.filter(|_| placed.iter().any(|t| !t.kept)) {
            let text = format!(
                "Faint: left out: gw writes cylinders past the first {least} only up to the last holding data"
            );
            ui.label(RichText::new(text).small().weak());
        }
    });
}

/// What the legend's Filler says on hover: the fillers the parts of gw's
/// filler hold, and why they hold them.
fn filler_tip(map: &Map, placed: &[Placed]) -> String {
    let text = fillers(map.image, placed);
    match (map.image.role, map.converts) {
        (Role::Made, false) => format!("gw's {text} in place of a sector it could not read"),
        (Role::Made, true) => format!("gw's {text} in place of a sector it could not decode"),
        (Role::Source, _) => format!("gw's {text}: gw takes it as the sector's data"),
    }
}

/// The fillers the image's parts of gw's filler hold, in turn, each as its
/// bytes repeat.
fn fillers(image: &Image, placed: &[Placed]) -> String {
    let mut held: Vec<usize> = Vec::new();
    for (part, _, state) in placed.iter().flat_map(|t| &t.parts) {
        if *state == State::Filler && !held.contains(&part.filler) {
            held.push(part.filler);
        }
    }
    let all = image.layout.as_ref().map_or(&[][..], |l| &l.fillers[..]);
    let texts: Vec<String> = held
        .iter()
        .filter_map(|&i| all.get(i))
        .map(|f| filler_text(f))
        .collect();
    texts.join(" or ")
}

/// A filler's bytes as they repeat, as ASCII where all of them print, else
/// in hex: the bytes that repeat and how many times, up to 16 of them; with
/// no such repeat, its first 16 and how many there are.
fn filler_text(bytes: &[u8]) -> String {
    let shown = |b: &[u8]| -> String {
        match b.iter().all(|c| (32..127).contains(c)) {
            true => b.iter().map(|&c| c as char).collect(),
            false => {
                let hex: Vec<String> = b.iter().map(|c| format!("{c:02X}")).collect();
                hex.join(" ")
            }
        }
    };
    let len = bytes.len();
    let repeats = (1..=len.min(16))
        .find(|&n| len.is_multiple_of(n) && bytes.chunks(n).all(|c| c == &bytes[..n]));
    match repeats {
        Some(n) if n < len => format!("{} × {}", shown(&bytes[..n]), len / n),
        Some(_) => shown(bytes),
        None if len == 0 => "no bytes".to_owned(),
        None => format!(
            "{}… of {} bytes",
            shown(&bytes[..16]),
            surface::grouped(len as u64)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 160 tracks of 11 parts alike on two sides, offsets 48 points wide.
    fn layout(column: Option<f32>) -> Plan {
        Plan {
            tracks: 160,
            sides: 2,
            parts: 11,
            units: 11,
            column,
            label: 48.0,
        }
    }

    #[test]
    fn the_rows_lie_where_the_disks_do_in_a_column_under_each() {
        // Disks 300 points across in a room 1,000 wide and 500 tall.
        let room = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0));
        let grid = Grid::new(&layout(Some(300.0)), room, 2.0);
        assert_eq!((grid.columns, grid.rows, grid.row), (2, 80, 6.0));
        // Each column a disk's width, to the pixel its 11 parts alike take
        // whole: 54 of its 600 each. The second's offsets between them,
        // centred; the rows centred in the room's height.
        let between = COLUMN_GAP + 48.0;
        assert_eq!(grid.column, 54.0 * 11.0 / 2.0);
        let width = 2.0 * grid.column + between;
        assert_eq!(grid.origin, egui::pos2(500.0 - width / 2.0, 10.0));
        let second = grid.row(80);
        assert_eq!(second.min.x, grid.origin.x + grid.column + between);
        assert_eq!(second.right(), 500.0 + width / 2.0);
    }

    #[test]
    fn a_file_runs_on_in_more_columns_rather_than_in_rows_thinner_than_the_least() {
        // 200 points: 50 rows of 4 at most, so four columns of 40, each
        // still as wide as a disk.
        let room = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 200.0));
        let grid = Grid::new(&layout(Some(150.0)), room, 2.0);
        assert_eq!((grid.columns, grid.rows, grid.row), (4, 40, 5.0));
        assert_eq!(grid.column, 27.0 * 11.0 / 2.0);
        // No narrower than its parts need, nor wider than the room: each
        // still a whole number of pixels for each part.
        let between = COLUMN_GAP + 48.0;
        let narrow = Grid::new(&layout(Some(20.0)), room, 2.0);
        assert_eq!(narrow.column, 12.0 * 11.0 / 2.0);
        assert!(narrow.column >= COLUMN_LEAST);
        assert_eq!(narrow.pitch, narrow.column + between);
        let wide = Grid::new(&layout(Some(400.0)), room, 2.0);
        let shared = (1000.0 - 48.0 - 3.0 * between) / 4.0;
        assert_eq!(wide.column, (shared * 2.0 / 11.0).floor() * 11.0 / 2.0);
        // Rows a whole number of pixels, and no taller than the most.
        let few = Plan {
            tracks: 10,
            ..layout(None)
        };
        let tall = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0));
        assert_eq!(Grid::new(&few, tall, 1.5).row, ROW_MOST);
        let odd = Grid::new(&layout(None), tall, 1.5);
        assert_eq!(odd.row * 1.5, (500.0 / 80.0 * 1.5f32).floor());
    }

    #[test]
    fn offsets_take_as_many_hex_digits_as_the_files_last_byte_and_at_least_four() {
        assert_eq!(hex_digits(0), 4);
        assert_eq!(hex_digits(0x10000), 4);
        assert_eq!(hex_digits(0x10001), 5);
        assert_eq!(hex_digits(901_120), 5);
        assert_eq!(hex_digits(1_638_400), 6);
    }

    #[test]
    fn a_tiny_room_keeps_its_columns_within_it_and_each_as_wide_as_its_parts_need() {
        // 20 points: rows 4 tall would want 32 columns.
        let room = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 20.0));
        for column in [None, Some(150.0)] {
            let plan = layout(column);
            let grid = Grid::new(&plan, room, 2.0);
            let last = grid.row(plan.tracks - 1);
            assert!(last.right() <= room.right(), "{grid:?}");
            assert!(grid.column >= plan.least(2.0), "{grid:?}");
            assert!(grid.row * 2.0 >= 1.0, "a pixel tall at least");
            // Thinner rows run on past the room's foot.
            assert_eq!(grid.origin.y, room.top());
        }
        // A room too narrow for one column of the least width has one.
        let narrow = Rect::from_min_size(Pos2::ZERO, vec2(80.0, 400.0));
        let grid = Grid::new(&layout(None), narrow, 2.0);
        assert_eq!(grid.columns, 1);
    }

    /// A track of `parts` sectors of 512 bytes, from the file's start.
    fn alike(parts: usize) -> Placed {
        let part = |i: usize| Part {
            index: i,
            id: None,
            len: 512,
            filler: 0,
        };
        Placed {
            key: (0, 0),
            start: 0,
            len: 512 * parts as u64,
            parts: (0..parts)
                .map(|i| (part(i), 512 * i as u64, State::Data))
                .collect(),
            kept: true,
        }
    }

    #[test]
    fn parts_alike_are_drawn_alike() {
        // A disk 660 pixels across: the column takes 648, 36 for each part.
        let plan = Plan {
            tracks: 160,
            sides: 2,
            parts: 18,
            units: 18,
            column: Some(330.0),
            label: 48.0,
        };
        let room = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0));
        let grid = Grid::new(&plan, room, 2.0);
        assert_eq!(grid.column, 36.0 * 18.0 / 2.0);
        let track = alike(18);
        for i in [0, 80] {
            let row = grid.row(i);
            let widths: Vec<f32> = track
                .parts
                .iter()
                .map(|(p, at, _)| drawn(row, &track, *at, p.len, track.len, 2.0).width())
                .collect();
            assert!(widths.iter().all(|&w| w == widths[0]), "{widths:?}");
        }
        // Parts of two lengths, each a whole number of the lesser's pixels.
        let mut mixed = alike(3);
        mixed.parts[2].0.len = 1024;
        mixed.len = 2048;
        assert_eq!(units(std::slice::from_ref(&mixed), mixed.len), 4);
    }

    #[test]
    fn a_fillers_bytes_are_said_as_they_repeat() {
        let adf = b"-=[BAD SECTOR]=-".repeat(32);
        assert_eq!(filler_text(&adf), "-=[BAD SECTOR]=- × 32");
        assert_eq!(filler_text(&[0; 512]), "00 × 512");
        assert_eq!(filler_text(&[0xE5, 0x00].repeat(128)), "E5 00 × 128");
        assert_eq!(filler_text(b"-=[BAD SECTOR]=-"), "-=[BAD SECTOR]=-");
        let odd: Vec<u8> = (0..40).collect();
        assert_eq!(
            filler_text(&odd),
            "00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F… of 40 bytes"
        );
    }

    #[test]
    fn the_legends_filler_names_each_filler_the_parts_hold() {
        // Sectors of three sizes, each its filler; the third's read, so its
        // filler goes unnamed.
        let hex = |b: &[u8]| -> String { b.iter().map(|b| format!("{b:02x}")).collect() };
        let bad = b"-=[BAD SECTOR]=-";
        let open = serde_json::json!({
            "event": "open", "role": "made", "file": "Disk.img", "type": "IMG",
            "layout": {"tracks": [
                {"c": 0, "h": 0, "sectors": [{"i": 0, "id": null, "len": 512, "fill": 0}]},
                {"c": 1, "h": 0, "sectors": [{"i": 0, "id": null, "len": 1024, "fill": 1}]},
                {"c": 2, "h": 0, "sectors": [{"i": 0, "id": null, "len": 256, "fill": 2}]}],
                "fillers": [hex(&bad.repeat(32)), hex(&[0xE5; 1024]), hex(&bad.repeat(16))],
                "min_cyls": null}
        });
        let mut image = Image::parse(&open).unwrap();
        for (c, has) in [(0, false), (1, false), (2, true)] {
            image.take(&serde_json::json!({"event": "track", "c": c, "h": 0, "has": [has]}));
        }
        let placed = image.placed(None).unwrap();
        assert_eq!(
            fillers(&image, &placed),
            "-=[BAD SECTOR]=- × 32 or E5 × 1024"
        );
    }

    /// An image gw makes, `tracks` tracks of 11 sectors on `sides` sides as
    /// gw lays out an ADF, past cylinder `least` written only up to the last
    /// holding data; its first 20 tracks read.
    fn made(tracks: u32, sides: u32, least: u32) -> Image {
        let sector = |i: usize| serde_json::json!({"i": i, "id": null, "len": 512, "fill": 0});
        let laid: Vec<serde_json::Value> = (0..tracks)
            .map(|t| {
                let sectors: Vec<_> = (0..11).map(sector).collect();
                serde_json::json!({"c": t / sides, "h": t % sides, "sectors": sectors})
            })
            .collect();
        let filler: String = b"-=[BAD SECTOR]=-"
            .repeat(32)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let open = serde_json::json!({
            "event": "open", "role": "made", "file": "Disk.adf", "type": "ADF",
            "layout": {"tracks": laid, "fillers": [filler], "min_cyls": least}
        });
        let mut image = Image::parse(&open).unwrap();
        for t in 0..20 {
            let mut has = vec![true; 11];
            has[3] = t % 5 != 0;
            let read =
                serde_json::json!({"event": "track", "c": t / sides, "h": t % sides, "has": has});
            image.take(&read);
        }
        image
    }

    /// A read of `image`'s cylinders and sides, gw last at `current`.
    fn reading(image: &Image, current: (u32, u32)) -> Progress {
        let mut progress = Progress::default();
        let last = image.layout.as_ref().unwrap().tracks.last().unwrap().key;
        progress.feed(&format!("Reading c=0-{}:h=0-{} revs=2", last.0, last.1));
        progress.current = Some(current);
        progress
    }

    /// The texts a frame drew, and where.
    type Texts = Vec<(String, Rect)>;

    /// The image view of `map` in a drawer's room `size`, the disks
    /// `disks` across, for `frames` frames a 60th of a second apart: the
    /// texts the last drew, and where, and those the one before it drew;
    /// and whether any of its last five asked for more.
    fn view(
        map: &Map,
        size: egui::Vec2,
        disks: Option<Place>,
        frames: usize,
    ) -> (Texts, Texts, bool) {
        let ctx = egui::Context::default();
        let mut busy = Vec::new();
        let (mut texts, mut before) = (Vec::new(), Vec::new());
        for frame in 0..frames {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(2000.0, 1200.0))),
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let room = Rect::from_min_size(Pos2::ZERO, size);
                ui.scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
                    show(ui, map, disks)
                });
            });
            out.textures_delta.clear();
            let root = &out.viewport_output[&egui::ViewportId::ROOT];
            busy.push(root.repaint_delay.is_zero());
            before = std::mem::take(&mut texts);
            texts = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => {
                        Some((t.galley.text().to_owned(), t.visual_bounding_rect()))
                    }
                    _ => None,
                })
                .collect();
        }
        (texts, before, busy.iter().rev().take(5).any(|&b| b))
    }

    #[test]
    fn the_columns_and_the_legend_settle_whatever_the_room() {
        // The legend's height sets the columns, which set where the legend
        // wraps: in these rooms they once changed each other every frame.
        // Single-sided files, as a D64 is.
        let rooms = [
            (40, 1, 30, 800.0, 160.0, 200.0),
            (40, 1, 30, 600.0, 330.0, 204.0),
            (40, 1, 30, 700.0, 100.0, 124.0),
            (35, 1, 30, 550.0, 120.0, 112.0),
            (35, 1, 30, 650.0, 160.0, 180.0),
            (160, 2, 40, 900.0, 330.0, 300.0),
            (160, 2, 40, 700.0, 150.0, 200.0),
        ];
        for (tracks, sides, least, width, diameter, height) in rooms {
            let image = made(tracks, sides, least);
            let progress = reading(&image, (5, 0));
            let map = Map {
                progress: Some(&progress),
                image: &image,
                running: true,
                converts: false,
            };
            let disks = Place {
                sides,
                diameter,
                width: sides as f32 * diameter + 32.0 * (sides as f32 - 1.0),
            };
            let size = vec2(width, height);
            let (texts, before, busy) = view(&map, size, Some(disks), 30);
            let room = format!("{tracks} tracks in {size:?}, disks {diameter} across");
            assert!(!busy, "{room}");
            assert_eq!(texts, before, "{room}");
        }
    }

    #[test]
    fn the_legend_names_the_mark_on_the_track_gw_last_reported_only_where_it_is_drawn() {
        let image = made(160, 2, 40);
        let size = vec2(900.0, 400.0);
        for (current, named) in [((21, 0), true), ((90, 0), false)] {
            let progress = reading(&image, current);
            let map = Map {
                progress: Some(&progress),
                image: &image,
                running: true,
                converts: false,
            };
            let (texts, _, _) = view(&map, size, None, 3);
            let named_it = texts.iter().any(|(t, _)| t == "Last reported");
            assert_eq!(named_it, named, "{texts:?}");
            // Cylinders 40 on hold no data yet: gw would leave them out.
            let faint = "Faint: left out: gw writes cylinders past the first 40 only up to the last holding data";
            assert!(texts.iter().any(|(t, _)| t == faint), "{texts:?}");
        }
    }

    #[test]
    fn a_name_too_long_for_the_line_over_the_rows_is_cut_in_the_middle() {
        let mut image = made(160, 2, 40);
        let name = format!("{}Disk 1.adf", "A Long Name for a Floppy ".repeat(6));
        image.file = Some(format!("/Users/you/Floppies/{name}"));
        let progress = reading(&image, (5, 0));
        let map = Map {
            progress: Some(&progress),
            image: &image,
            running: true,
            converts: false,
        };
        let size = vec2(700.0, 400.0);
        let (texts, _, _) = view(&map, size, None, 3);
        // 40 cylinders: those gw writes at least, holding data past none of them.
        let rest = " · 450,560 bytes as laid out · Being made: gw writes it when it finishes";
        let (line, at) = texts
            .iter()
            .find(|(t, _)| t.ends_with(rest))
            .expect("the line over the rows");
        let ends = line.starts_with("A Long Name") && line.contains("Disk 1.adf · ");
        assert!(ends && line.contains('…'), "{line}");
        assert!(at.right() <= size.x, "{at:?}");
        // A name that fits is whole.
        let mut short = image.clone();
        short.file = Some("/Users/you/Floppies/Workbench.adf".into());
        let map = Map {
            image: &short,
            ..map
        };
        let (texts, _, _) = view(&map, size, None, 3);
        let whole = format!("Workbench.adf{rest}");
        assert!(texts.iter().any(|(t, _)| *t == whole));
    }

    #[test]
    fn an_image_gw_does_not_lay_out_is_not_mapped_and_says_why() {
        let mut progress = Progress::default();
        assert_eq!(
            shown(&progress).err().as_deref(),
            Some("No image reported.")
        );
        let open = |file: &str, kind: &str| {
            format!(
                r#"{{"event":"open","role":"source","file":"{file}","type":"{kind}","layout":null,"size":null}}"#
            )
        };
        for (file, kind, said) in [
            (
                "/d/Game.DSK",
                "EDSK",
                "Not mapped: gw lays out .dsk its own way.",
            ),
            (
                "Disk.a2r",
                "A2R",
                "Not mapped: .a2r holds flux, not sectors.",
            ),
            (
                "Disk.ipf",
                "IPF",
                "Not mapped: .ipf holds bitcells, not sectors.",
            ),
        ] {
            progress.image(&open(file, kind));
            assert_eq!(shown(&progress).err().as_deref(), Some(said));
        }
        // A type gw lays out, its file not laid out so.
        let differs = r#"{"event":"open","role":"source","file":"Disk.adf","type":"ADF","layout":null,"size":900,"differs":true}"#;
        progress.image(differs);
        assert_eq!(
            shown(&progress).err().as_deref(),
            Some("Not mapped: the file is not as gw lays out .adf.")
        );
    }

    #[test]
    fn the_view_shows_the_image_a_job_makes_where_gw_lays_it_out_else_the_one_it_takes() {
        let open = |role: &str, file: &str, laid: bool| {
            let layout = laid.then(|| {
                let sector = serde_json::json!({"i": 0, "id": null, "len": 512, "fill": 0});
                let track = serde_json::json!({"c": 0, "h": 0, "sectors": [sector]});
                serde_json::json!({"tracks": [track], "fillers": ["e5"], "min_cyls": null})
            });
            let open = serde_json::json!({
                "event": "open", "role": role, "file": file, "type": "IMG", "layout": layout
            });
            open.to_string()
        };
        let file = |progress: &Progress| shown(progress).map(|i| i.file.clone().unwrap());
        // A conversion's two images, both laid out: the one it makes.
        let mut progress = Progress::default();
        progress.image(&open("source", "In.img", true));
        progress.image(&open("made", "Out.img", true));
        assert_eq!(file(&progress).as_deref(), Ok("Out.img"));
        // The one it makes kept as flux: the one it takes its tracks from.
        progress.image(&open("made", "Out.scp", false));
        assert_eq!(file(&progress).as_deref(), Ok("In.img"));
        // Neither laid out: why not, of the one it makes.
        progress.image(&open("source", "In.ipf", false));
        let why = "Not mapped: .scp holds flux, not sectors.";
        assert_eq!(file(&progress), Err(why.to_owned()));
        // A write's: only the one it takes them from.
        let mut progress = Progress::default();
        progress.image(&open("source", "In.img", true));
        assert_eq!(file(&progress).as_deref(), Ok("In.img"));
    }

    #[test]
    fn a_point_names_the_part_under_it_as_the_rows_are_drawn() {
        // A track of two sectors, then one of one, rows 100 points wide.
        let first = alike(2);
        let mut second = alike(1);
        second.key = (1, 0);
        second.start = 1024;
        second.parts[0].1 = 1024;
        let placed = [first, second];
        let rows = [
            Rect::from_min_size(egui::pos2(10.0, 0.0), vec2(100.0, 6.0)),
            Rect::from_min_size(egui::pos2(10.0, 6.0), vec2(100.0, 6.0)),
        ];
        let under = |x: f32, y: f32| hit(&rows, &placed, 1024, egui::pos2(x, y));
        assert_eq!(under(10.0, 3.0), Some((0, 0)));
        assert_eq!(under(59.9, 3.0), Some((0, 0)));
        assert_eq!(
            under(60.0, 3.0),
            Some((0, 1)),
            "the second sector's first byte"
        );
        assert_eq!(under(30.0, 9.0), Some((1, 0)));
        // Past the shorter track's bytes, before the rows, and below them.
        assert_eq!(under(70.0, 9.0), None);
        assert_eq!(under(5.0, 3.0), None);
        assert_eq!(under(30.0, 20.0), None);
        // In a file's rows as the view lays them out, each part's middle as
        // drawn names that part.
        let image = made(160, 2, 40);
        let placed = image.placed(None).unwrap();
        let widest = placed.iter().map(|t| t.len).max().unwrap();
        let plan = Plan {
            units: units(&placed, widest),
            ..layout(None)
        };
        let room = Rect::from_min_size(Pos2::ZERO, vec2(700.0, 400.0));
        let grid = Grid::new(&plan, room, 2.0);
        let rows: Vec<Rect> = (0..placed.len()).map(|i| grid.row(i)).collect();
        for (t, track) in placed.iter().enumerate() {
            for (k, (part, at, _)) in track.parts.iter().enumerate() {
                let cell = drawn(rows[t], track, *at, part.len, widest, 2.0);
                let named = hit(&rows, &placed, widest, cell.center());
                assert_eq!(named, Some((t, k)), "{cell:?}");
            }
        }
    }

    #[test]
    fn the_line_over_the_rows_gives_a_size_only_for_a_file_there_is() {
        let image = made(160, 2, 40);
        let progress = reading(&image, (5, 0));
        let running = Map {
            progress: Some(&progress),
            image: &image,
            running: true,
            converts: false,
        };
        // gw makes the file as it finishes: till then the size is the
        // layout's, and once it stops, none.
        assert_eq!(
            header(&running),
            "Disk.adf · 450,560 bytes as laid out · Being made: gw writes it when it finishes"
        );
        let stopped = Map {
            running: false,
            ..running
        };
        assert_eq!(header(&stopped), "Disk.adf · Not written");
        let mut written = image.clone();
        written.take(&serde_json::json!({"event": "written", "size": 450_560, "tracks": null}));
        let done = Map {
            image: &written,
            ..stopped
        };
        assert_eq!(header(&done), "Disk.adf · 450,560 bytes · Written by gw");
        // A source's is its file's.
        let mut source = image.clone();
        source.role = Role::Source;
        source.size = Some(901_120);
        let before = Map {
            progress: None,
            image: &source,
            running: false,
            converts: false,
        };
        assert_eq!(header(&before), "Disk.adf · 901,120 bytes · As gw reads it");
    }

    #[test]
    fn a_source_shorter_than_its_layout_has_offsets_as_long_as_the_layouts_end_needs() {
        // An ADF's 901,120 bytes laid out over a file of 61,440.
        let mut image = made(160, 2, 40);
        image.role = Role::Source;
        image.size = Some(61_440);
        let placed = image.placed(None).unwrap();
        assert_eq!(hex_digits(61_440), 4);
        assert_eq!(offset_digits(&image, &placed), 5);
        // Each drawn whole in the room, track 12's 10800 too.
        let map = Map {
            progress: None,
            image: &image,
            running: false,
            converts: false,
        };
        let (texts, _, _) = view(&map, vec2(500.0, 400.0), None, 3);
        let offsets: Vec<&(String, Rect)> = texts
            .iter()
            .filter(|(t, _)| t.len() == 5 && u64::from_str_radix(t, 16).is_ok())
            .collect();
        assert!(offsets.iter().any(|(t, _)| t == "10800"), "{texts:?}");
        for (text, at) in offsets {
            assert!(at.left() >= -0.5, "{text} at {at:?}");
        }
    }

    #[test]
    fn a_conversion_says_its_filler_is_for_what_it_did_not_decode_or_convert() {
        let image = made(160, 2, 40);
        let progress = reading(&image, (5, 0));
        let placed = image.placed(None).unwrap();
        let said = [
            (
                false,
                "did not read",
                "was not read",
                "could not read",
                "Not read ",
            ),
            (
                true,
                "did not decode",
                "was not converted",
                "could not decode",
                "Not converted ",
            ),
        ];
        for (converts, sector, track, could, legend) in said {
            let map = Map {
                progress: Some(&progress),
                image: &image,
                running: false,
                converts,
            };
            let filler = format!("gw's filler: the sector {sector}");
            assert_eq!(state_text(State::Filler, &map), filler);
            let unread = format!("gw's filler: the track {track}");
            assert_eq!(state_text(State::Unread, &map), unread);
            let tip = format!("gw's -=[BAD SECTOR]=- × 32 in place of a sector it {could}");
            assert_eq!(filler_tip(&map, &placed), tip);
            let (texts, _, _) = view(&map, vec2(900.0, 400.0), None, 3);
            assert!(
                texts.iter().any(|(t, _)| t.starts_with(legend)),
                "{texts:?}"
            );
        }
    }

    #[test]
    fn the_legends_filler_names_its_fillers_once_hovered() {
        let image = made(160, 2, 40);
        let progress = reading(&image, (5, 0));
        let map = Map {
            progress: Some(&progress),
            image: &image,
            running: false,
            converts: false,
        };
        let tip = "gw's -=[BAD SECTOR]=- × 32 in place of a sector it could not read";
        let ctx = egui::Context::default();
        let mut entry = None;
        for frame in 0..60 {
            let events = match (frame, entry) {
                (5, Some(at)) => vec![egui::Event::PointerMoved(at)],
                _ => vec![],
            };
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(2000.0, 1200.0))),
                time: Some(frame as f64 / 30.0),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let room = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 400.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
                    show(ui, &map, None)
                });
            });
            out.textures_delta.clear();
            let texts = out.shapes.iter().filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => {
                    Some((t.galley.text().to_owned(), t.visual_bounding_rect()))
                }
                _ => None,
            });
            let texts: Texts = texts.collect();
            let tipped = texts.iter().any(|(t, _)| t == tip);
            match frame {
                ..=5 => assert!(!tipped, "not before it is hovered"),
                59 => assert!(tipped, "{texts:?}"),
                _ => {}
            }
            // Where the entry lies once the legend has settled.
            let filler = texts.iter().find(|(t, _)| t.starts_with("Filler "));
            entry = filler.map(|(_, at)| at.center());
        }
    }
}

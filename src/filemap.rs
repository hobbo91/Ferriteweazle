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

/// A row's height, at most and at least, in points: past that, the file
/// runs on in another column.
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

/// Why gw's report lays out no part of an image.
fn not_laid_out(image: &Image) -> String {
    not_mapped(image.file.as_deref().unwrap_or(&image.kind))
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
    let job = job(map);
    let Some(placed) = map.image.placed(job.as_ref()) else {
        ui.label(RichText::new(header(map)).small().color(p.dim));
        let text = "Not as laid out: gw wrote the file another way.";
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
    let digits = hex_digits(map.image.bytes().unwrap_or(0));
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
    let parts = placed.iter().map(|t| t.parts.len()).max().unwrap_or(1);
    let layout = Layout {
        tracks: placed.len(),
        sides,
        parts,
        column: disks.map(|d| d.diameter),
        label,
    };
    let grid = Grid::new(&layout, picture, ppp);
    let layers = flow(ui, grid, picture.min);
    let rows: Vec<Rect> = (0..placed.len()).map(|i| grid.row(i)).collect();
    let bounds = rows.iter().fold(Rect::NOTHING, |b, r| b.union(*r));
    let response = ui.interact(bounds, ui.id().with("image map"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Image map"));
    // The file's name, size and state over the rows, as the sides' names are over the disks.
    let title = Rect::from_min_size(
        egui::pos2(grid.origin.x, area.top()),
        vec2(area.right() - grid.origin.x, TITLE),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(title)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| ui.label(RichText::new(header(map)).small().color(p.dim)),
    );
    // Rows flowing from a taller layout are cut at the box; marks round
    // them may reach a little past it.
    let painter = ui.painter_at(area);
    let marks = ui.painter_at(area.expand(2.0));
    let widest = placed.iter().map(|t| t.len).max().unwrap_or(1).max(1);
    // An offset's digits from the top of the row it names.
    let ink = offset_ink(ui, &font);
    for (grid, opacity) in &layers {
        for (i, track) in placed.iter().enumerate() {
            let row = grid.row(i);
            for (part, at, state) in &track.parts {
                let cell = drawn(row, track, *at, part.len, widest, ppp);
                let mut colour = colour(*state, &look, p);
                if !track.kept {
                    colour = colour.gamma_multiply(0.5);
                }
                painter.rect_filled(cell, 0.0, colour.gamma_multiply(*opacity));
            }
            if (i % grid.rows).is_multiple_of(grid.every) {
                let at = egui::pos2(row.left() - OFFSET_GAP, row.top() - ink);
                let text = format!("{:0digits$X}", track.start);
                let colour = p.dim.gamma_multiply(*opacity);
                painter.text(at, Align2::RIGHT_TOP, text, font.clone(), colour);
            }
        }
    }
    // Where gw is in the image while it works, as the disk view rings it.
    let current = job.as_ref().and_then(|j| j.current(map.image.role));
    if let Some((track, row)) = placed
        .iter()
        .zip(&rows)
        .find(|(t, _)| Some(t.key) == current)
    {
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
                match tone {
                    Tone::Strong => ui.strong(text),
                    Tone::Plain => ui.label(text),
                    Tone::Weak => ui.weak(text),
                };
            }
        });
        if response.clicked() {
            let opened = Opened {
                key: track.key,
                part: k,
                at: *at,
            };
            ui.data_mut(|d| d.insert_temp(opened_id(), opened));
        }
    }
    window(ui.ctx(), map, &placed, digits);
    // The legend where the disk view's is, under the rows' first column.
    let top = area.bottom();
    let drawn = ui.scope_builder(
        egui::UiBuilder::new().max_rect(Rect::from_min_max(
            egui::pos2(grid.origin.x, top + 6.0),
            egui::pos2(area.right(), top + 6.0 + ui.available_height().max(0.0)),
        )),
        |ui| legend_rows(ui, map, &placed, current.is_some(), &look, p),
    );
    let height = drawn.response.rect.bottom() - top;
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
        converts: map.converts,
    })
}

/// The line over the rows: the file, its size, and where gw is with it.
fn header(map: &Map) -> String {
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
    let size = image
        .bytes()
        .map(|b| format!("{} bytes", surface::grouped(b)));
    let state = match image.role {
        Role::Made if image.written.is_some() => "Written by gw",
        Role::Made if map.running => "Being made: gw writes it when it finishes",
        Role::Made => "Not written",
        Role::Source if map.progress.is_none() => "As gw reads it",
        Role::Source => "As gw read it",
    };
    let parts: Vec<String> = [Some(name), size, Some(state.to_owned())]
        .into_iter()
        .flatten()
        .collect();
    parts.join(" · ")
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
/// and its tracks' parts at most, the disks' diameter where they are drawn,
/// and the width of a column's offsets with their gap.
struct Layout {
    tracks: usize,
    sides: usize,
    parts: usize,
    column: Option<f32>,
    label: f32,
}

/// The rows the file runs down, where the disks lie: in a column for each
/// side of the disk, or more where rows would be thinner than ROW_LEAST;
/// each row a whole number of pixels tall, the rows centred in the room's
/// height; each column as wide as a disk, or the room's width shared out
/// where no disks are drawn, centred, each with its offsets before it.
#[derive(Clone, Copy, PartialEq)]
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
    fn new(layout: &Layout, room: Rect, ppp: f32) -> Grid {
        let tracks = layout.tracks.max(1);
        let height = room.height();
        let fits = (height / ROW_LEAST).floor().max(1.0) as usize;
        let columns = tracks.div_ceil(fits).max(layout.sides).min(tracks);
        let rows = tracks.div_ceil(columns);
        let row = ((height / rows as f32).min(ROW_MOST) * ppp)
            .floor()
            .max(1.0)
            / ppp;
        let n = columns as f32;
        let between = COLUMN_GAP + layout.label;
        let least = COLUMN_LEAST.max(layout.parts as f32 * PART_LEAST / ppp);
        let room_width = room.width() - layout.label;
        let shared = (room_width - (n - 1.0) * between) / n;
        let wide = layout.column.unwrap_or(shared).max(least);
        let width = (n * wide + (n - 1.0) * between).min(room_width);
        let column = ((width - (n - 1.0) * between) / n).max(1.0);
        let left = (room.center().x - width / 2.0).max(room.left() + layout.label);
        let spare = height - row * rows as f32;
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
        State::Filler => "gw's filler: the sector did not read",
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
/// the track, and where it lies in the file.
#[derive(Clone, Copy, PartialEq)]
struct Opened {
    key: (u32, u32),
    part: usize,
    at: u64,
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
        .and_then(|t| Some((t, t.parts.get(opened.part)?)))
        .filter(|(_, (_, at, _))| *at == opened.at);
    let Some((track, (part, at, state))) = found else {
        ctx.data_mut(|d| d.remove::<Opened>(opened_id()));
        return;
    };
    let (cyl, side) = track.key;
    let title = format!("{} · cylinder {cyl}, side {side}", id_text(part));
    let lines = said(map, track, part, *at, *state, digits);
    let bytes = map.image.part_bytes(track, opened.part).unwrap_or_default();
    let shown = Shown {
        title: &title,
        lines: &lines,
        bytes: &bytes,
        base: *at as usize,
    };
    if !surface::sector_window(ctx, egui::Id::new("image part window"), &shown) {
        ctx.data_mut(|d| d.remove::<Opened>(opened_id()));
    }
}

/// How many hex digits an offset into a file of `size` bytes needs, and at least four.
fn hex_digits(size: u64) -> usize {
    let last = size.saturating_sub(1);
    ((u64::BITS - last.leading_zeros()).div_ceil(4).max(4)) as usize
}

/// The key to what the parts hold, with how many of each, and while gw
/// works, the mark on the track it last reported.
fn legend_rows(
    ui: &mut egui::Ui,
    map: &Map,
    placed: &[Placed],
    current: bool,
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
                State::Unread => "Not read",
                State::PastEnd => "Past the end",
                State::ToDo => "To do",
            };
            let entry = surface::key(
                ui,
                Mark::Swatch(colour(state, look, p)),
                &format!("{name} {n}"),
            );
            if state == State::Filler {
                let filler = map.image.layout.as_ref().and_then(|l| l.fillers.first());
                let text: String = filler
                    .map(|f| {
                        f.iter()
                            .take(16)
                            .map(|&b| {
                                if (32..127).contains(&b) {
                                    b as char
                                } else {
                                    '.'
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                entry.on_hover_text(match map.image.role {
                    Role::Made => format!("gw's {text} in place of a sector it could not read"),
                    Role::Source => format!("gw's {text}: gw takes it as the sector's data"),
                });
            }
        }
        if current {
            surface::key(ui, Mark::Frame(look.last), "Last reported");
        }
        if placed.iter().any(|t| !t.kept) {
            ui.label(
                RichText::new("Faint: written up to the last cylinder holding data")
                    .small()
                    .weak(),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 160 tracks of 11 parts on two sides, offsets 48 points wide.
    fn layout(column: Option<f32>) -> Layout {
        Layout {
            tracks: 160,
            sides: 2,
            parts: 11,
            column,
            label: 48.0,
        }
    }

    #[test]
    fn the_rows_lie_where_the_disks_do_in_a_column_for_each_side() {
        // Disks 300 points across in a room 1,000 wide and 500 tall.
        let room = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0));
        let grid = Grid::new(&layout(Some(300.0)), room, 2.0);
        assert_eq!((grid.columns, grid.rows, grid.row), (2, 80, 6.0));
        // Each column a disk's width, the second's offsets between them,
        // centred; the rows centred in the room's height.
        let between = COLUMN_GAP + 48.0;
        let width = 600.0 + between;
        assert_eq!(grid.column, 300.0);
        assert_eq!(grid.origin, egui::pos2(500.0 - width / 2.0, 10.0));
        let second = grid.row(80);
        assert_eq!(second.min.x, grid.origin.x + 300.0 + between);
        assert_eq!(second.right(), 500.0 + width / 2.0);
    }

    #[test]
    fn a_file_runs_on_in_more_columns_rather_than_in_rows_thinner_than_the_least() {
        // 200 points: 50 rows of 4 at most, so four columns of 40, each
        // still as wide as a disk.
        let room = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 200.0));
        let grid = Grid::new(&layout(Some(150.0)), room, 2.0);
        assert_eq!((grid.columns, grid.rows, grid.row), (4, 40, 5.0));
        assert_eq!(grid.column, 150.0);
        // No narrower than its parts need, nor wider than the room.
        let between = COLUMN_GAP + 48.0;
        let narrow = Grid::new(&layout(Some(20.0)), room, 2.0);
        assert_eq!(narrow.column, COLUMN_LEAST);
        assert_eq!(narrow.pitch, COLUMN_LEAST + between);
        let wide = Grid::new(&layout(Some(400.0)), room, 2.0);
        assert_eq!(wide.column, (1000.0 - 48.0 - 3.0 * between) / 4.0);
        // Rows a whole number of pixels, and no taller than the most.
        let few = Layout {
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
    }
}

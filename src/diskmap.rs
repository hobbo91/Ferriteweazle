//! The disk map: a square per track, in a grid per side.

use crate::progress::{Progress, Status};
use crate::theme::{self, Palette};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, pos2,
    vec2,
};

/// Seconds a square takes to fade in.
const FILL_TIME: f32 = 0.4;

/// Cylinders to a row, so rows start at 0, 10, 20.
const ROW: u32 = 10;
const GAP: f32 = 3.0;
const MIN_CELL: f32 = 5.0;
/// Room for a legend of two lines: the squares keep one size as it fills in.
const LEGEND: f32 = 42.0;
/// The least square size wherever the pane has room for it.
const CELL: f32 = 18.0;
const MAX_CELL: f32 = 96.0;
/// Room for the row numbers left of a grid.
const LABEL: f32 = 24.0;
/// Row numbers' size, in points: rows shorter than it are numbered every other one.
const ROW_NUMBER: f32 = 11.0;
/// Room for a side's name above its grid.
const TITLE: f32 = 18.0;
/// Rows the squares are sized for, 90 cylinders, past any drive: every disk
/// gets one square size, which the Log at its least height does not shrink.
const SIZED_ROWS: u32 = 9;
const SIDE_GAP: f32 = 28.0;
const STACK_GAP: f32 = 8.0;

/// The most width a map of two sides across can use when `budget` points tall,
/// legend included: past it, the height limits the squares.
pub fn width_for(budget: f32) -> f32 {
    let rows = SIZED_ROWS as f32;
    let cell = ((budget - LEGEND - TITLE + GAP) / rows - GAP).clamp(MIN_CELL, MAX_CELL);
    // A point spare, so rounding to whole pixels cannot take a pixel off.
    2.0 * (LABEL + ROW as f32 * cell + (ROW - 1) as f32 * GAP) + SIDE_GAP + 1.0
}

/// Draws the map with squares of CELL points: smaller if `room`, the pane's height
/// below its top, lacks space; larger if a `budget`-point map (legend included) and
/// the width allow. `job` keys the fade-in, so a square fills once per job; while
/// it is `live`, a square that first shows lit fades in too.
pub fn show(
    ui: &mut egui::Ui,
    progress: &Progress,
    job: impl std::hash::Hash + std::fmt::Debug,
    live: bool,
    verifying: bool,
    budget: f32,
    room: f32,
) {
    let (cyls, heads) = progress.layout();
    let (Some(&first), Some(&last)) = (cyls.first(), cyls.last()) else {
        return;
    };
    let p = theme::palette(ui);
    // Whole pixels, so every square and every gap is the same size.
    let ppp = ui.ctx().pixels_per_point();
    let snap = |x: f32| (x * ppp).round() / ppp;
    let gap = snap(GAP);
    let sides = heads.len().max(1) as f32;
    let rows = last / ROW - first / ROW + 1;
    let width = ui.available_width();
    // The legend wraps in a narrow pane: last frame's height keeps it in room.
    let legend_id = ui.id().with("legend");
    let legend_height = ui.data(|d| d.get_temp(legend_id)).unwrap_or(LEGEND);
    let room = room - legend_height;
    let wanted = (budget - legend_height.max(LEGEND)).min(room);
    let cell_in = |across: bool, height: f32, rows: u32| {
        let (columns, stacked) = if across { (sides, 1.0) } else { (1.0, sides) };
        let each_width = (width - SIDE_GAP * (columns - 1.0)) / columns;
        let by_width = (each_width - LABEL - (ROW - 1) as f32 * gap) / ROW as f32;
        let each = (height - STACK_GAP * (stacked - 1.0)) / stacked;
        let by_height = (each - TITLE + gap) / rows as f32 - gap;
        by_width.min(by_height)
    };
    // The usual size where it fits, larger where the budget allows.
    let cell_for = |across: bool| {
        let fits = cell_in(across, room, rows);
        let usual = CELL.min(fits);
        cell_in(across, wanted, rows.max(SIZED_ROWS))
            .min(fits)
            .max(usual)
            .min(MAX_CELL)
    };
    // Sides across or stacked, whichever gives larger squares.
    let across = cell_for(true) >= cell_for(false);
    let cell = (cell_for(across).max(MIN_CELL) * ppp).floor() / ppp;
    let step = cell + gap;
    let grid = vec2(
        LABEL + ROW as f32 * cell + (ROW - 1) as f32 * gap,
        TITLE + rows as f32 * step - gap,
    );
    let size = match across {
        true => vec2(grid.x * sides + SIDE_GAP * (sides - 1.0), grid.y),
        false => vec2(grid.x, grid.y * sides + STACK_GAP * (sides - 1.0)),
    };
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect.expand(1.0));
    let radius = CornerRadius::same((cell / 5.0).round() as u8);
    let square = |i: usize, cyl: u32| {
        let origin = match across {
            true => rect.min + vec2(i as f32 * (grid.x + SIDE_GAP), 0.0),
            false => rect.min + vec2(0.0, i as f32 * (grid.y + STACK_GAP)),
        };
        let x = snap(origin.x + LABEL) + (cyl % ROW) as f32 * step;
        let y = snap(origin.y + TITLE) + (cyl / ROW - first / ROW) as f32 * step;
        Rect::from_min_size(pos2(x, y), vec2(cell, cell))
    };
    let mut shown = Vec::new();
    for (i, &head) in heads.iter().enumerate() {
        let corner = square(i, first - first % ROW);
        painter.text(
            pos2(corner.left(), corner.top() - TITLE + 2.0),
            Align2::LEFT_TOP,
            side_name(head),
            FontId::proportional(12.0),
            p.dim,
        );
        let every = if step < ROW_NUMBER { 2 } else { 1 };
        for row in (first / ROW..=last / ROW).filter(|r| r % every == 0) {
            let at = square(i, row * ROW);
            painter.text(
                pos2(at.left() - 7.0, at.center().y),
                Align2::RIGHT_CENTER,
                row * ROW,
                FontId::proportional(ROW_NUMBER),
                p.dim,
            );
        }
        for &cyl in &cyls {
            let key = (cyl, head);
            let filled = fill(progress, key, p);
            let id = egui::Id::new(("square", &job, key));
            let colour = shade(ui, id, filled, p.pending, live);
            let status = progress.tracks.get(&key).map(|t| t.status);
            let edge = edge(status == Some(Status::Skipped), p);
            painter.rect(square(i, cyl), radius, colour, edge, StrokeKind::Inside);
            shown.extend(filled);
        }
    }
    let hovered = response.hover_pos().and_then(|pos| {
        heads.iter().enumerate().find_map(|(i, &head)| {
            cyls.iter()
                .find(|&&c| square(i, c).expand(gap / 2.0).contains(pos))
                .map(|&c| (c, head))
        })
    });
    if let Some((cyl, head)) = hovered {
        response.on_hover_ui_at_pointer(|ui| {
            ui.strong(format!(
                "Cylinder {cyl}, {}",
                side_name(head).to_lowercase()
            ));
            match progress.tracks.get(&(cyl, head)) {
                Some(t) => {
                    ui.label(&t.text);
                    if t.retries > 0 {
                        ui.weak(retry_text(t.retries));
                    }
                }
                None => {
                    ui.weak("gw has not reported this track.");
                }
            }
            if let Some(rows) = missing(progress, (cyl, head)) {
                ui.label(format!("Missing in gw's sector map (S): {rows}"));
            }
        });
    }
    let top = ui.cursor().top();
    ui.add_space(6.0);
    legend(ui, &shown, progress, verifying, p);
    let height = ui.cursor().top() - top;
    if height != legend_height {
        ui.data_mut(|d| d.insert_temp(legend_id, height));
        ui.ctx().request_repaint();
    }
}

/// A square's fade from the colour it showed when its fill last changed.
#[derive(Clone, Copy)]
struct Shade {
    from: Color32,
    /// Its colour, `None` until gw reports the track.
    to: Option<Color32>,
    /// When it changed, in egui's seconds.
    at: f64,
}

impl Shade {
    fn colour(self, now: f64, pending: Color32) -> Color32 {
        let t = ((now - self.at) as f32 / FILL_TIME).clamp(0.0, 1.0);
        let to = self.to.unwrap_or(pending);
        theme::lerp(self.from, to, egui::emath::easing::cubic_in_out(t))
    }
}

/// A square's colour, fading over FILL_TIME from what it showed when `filled` last
/// changed. Timed from that frame, not by frame gaps, so a square lit after an idle
/// spell starts empty. One lit when first seen fades in while `live`, else shows at once.
fn shade(
    ui: &egui::Ui,
    id: egui::Id,
    filled: Option<Color32>,
    pending: Color32,
    live: bool,
) -> Color32 {
    let now = ui.input(|i| i.time);
    let fade_in = live && filled.is_some();
    let shade = ui.data_mut(|d| {
        let shade = d.get_temp_mut_or_insert_with(id, || Shade {
            from: if fade_in {
                pending
            } else {
                filled.unwrap_or(pending)
            },
            to: filled,
            at: if fade_in { now } else { f64::NEG_INFINITY },
        });
        if shade.to != filled {
            *shade = match filled {
                Some(_) => Shade {
                    from: shade.colour(now, pending),
                    to: filled,
                    at: now,
                },
                None => Shade {
                    from: pending,
                    to: None,
                    at: f64::NEG_INFINITY,
                },
            };
        }
        *shade
    });
    if now - shade.at < f64::from(FILL_TIME) {
        ui.ctx().request_repaint();
    }
    shade.colour(now, pending)
}

/// A track's colour, by its sectors once gw has mapped them; `None` until gw
/// reports the track.
fn fill(progress: &Progress, key: (u32, u32), p: &Palette) -> Option<Color32> {
    let Some(sectors) = progress.sector_map.get(&key) else {
        return progress.tracks.get(&key).map(|t| colour(t.status, p));
    };
    let good = sectors.contains(&Some(true));
    let bad = sectors.contains(&Some(false));
    Some(match (good, bad) {
        (false, false) => p.pending,
        (true, false) => p.good,
        (false, true) => p.bad,
        (true, true) => p.partial,
    })
}

/// The rows of gw's sector map missing on a track that has others, such as
/// `5, 9`. A row is a sector's place in the format's track, not its ID.
fn missing(progress: &Progress, key: (u32, u32)) -> Option<String> {
    let sectors = progress.sector_map.get(&key)?;
    let rows: Vec<String> = (0..sectors.len())
        .filter(|&s| sectors[s] == Some(false))
        .map(|s| s.to_string())
        .collect();
    // With none found, the track's text says so.
    (!rows.is_empty() && sectors.contains(&Some(true))).then(|| rows.join(", "))
}

fn side_name(head: u32) -> &'static str {
    match head {
        0 => "Side 0",
        1 => "Side 1",
        _ => "Side",
    }
}

fn colour(status: Status, p: &Palette) -> Color32 {
    match status {
        Status::Good => p.good,
        Status::Partial => p.partial,
        Status::Bad => p.bad,
        Status::Flux => p.flux,
        Status::Written => p.written,
        Status::Erased => p.erased,
        Status::Skipped => p.bg,
    }
}

/// A skipped track's square is a hole in the grid, outlined.
fn edge(skipped: bool, p: &Palette) -> Stroke {
    match skipped {
        true => Stroke::new(1.0, p.line_strong),
        false => Stroke::NONE,
    }
}

/// Each colour on the map with its track count, then the retries. `verifying`: a
/// write gw verifies is running, so its one written track is the one gw checks.
fn legend(ui: &mut egui::Ui, shown: &[Color32], progress: &Progress, verifying: bool, p: &Palette) {
    let written = match verifying {
        true => "The track gw is writing and checking.",
        false => progress
            .unverified
            .as_deref()
            .unwrap_or("Written, no verify reported."),
    };
    ui.horizontal_wrapped(|ui| {
        for (status, name, tip) in [
            (
                Status::Good,
                "Good",
                "Every sector found, or written and verified.",
            ),
            (Status::Partial, "Short", "Some sectors missing."),
            (Status::Bad, "Bad", "No sectors found, or the write failed."),
            (Status::Flux, "Flux", "Read as flux, not decoded."),
            (
                Status::Written,
                if verifying { "Verifying" } else { "Written" },
                written,
            ),
            (Status::Erased, "Erased", "Erased."),
            (
                Status::Skipped,
                "Skipped",
                "Outside the format, or not in the input.",
            ),
        ] {
            let swatch = colour(status, p);
            let tracks = shown.iter().filter(|&&c| c == swatch).count();
            if tracks == 0 {
                continue;
            }
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            let edge = edge(status == Status::Skipped, p);
            ui.painter()
                .rect(r, CornerRadius::same(2), swatch, edge, StrokeKind::Inside);
            let text = match status == Status::Written && verifying {
                true => name.to_owned(),
                false => format!("{name} {tracks}"),
            };
            ui.label(RichText::new(text).small()).on_hover_text(tip);
            ui.add_space(6.0);
        }
        let retries = progress.tally().retries;
        if retries > 0 {
            ui.label(RichText::new(retry_text(retries)).small().weak());
        }
    });
}

fn retry_text(n: u32) -> String {
    match n {
        1 => "1 retry".into(),
        n => format!("{n} retries"),
    }
}

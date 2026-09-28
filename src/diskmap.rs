//! The disk map: a square per track, in a grid per side.

use crate::progress::{Progress, Status};
use crate::theme::{self, Palette};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, pos2, vec2,
};

/// Seconds a square takes to fade in.
const FILL_TIME: f32 = 0.4;

/// Cylinders to a row, so rows start at 0, 10, 20.
const ROW: u32 = 10;
const GAP: f32 = 3.0;
const MIN_CELL: f32 = 5.0;
/// Room for the legend under the map.
const LEGEND: f32 = 28.0;
/// Square size wherever the pane has room.
const CELL: f32 = 18.0;
const MAX_CELL: f32 = 30.0;
/// Share of the pane's width the map fills where the height allows.
const WIDTH_SHARE: f32 = 0.9;
/// Room for the row numbers left of a grid.
const LABEL: f32 = 24.0;
/// Room for a side's name above its grid.
const TITLE: f32 = 20.0;
/// Rows the squares are sized for, 80 cylinders: the 82 gw erases add a row
/// below, not smaller squares, while the pane has room for it.
const SIZED_ROWS: u32 = 8;
const SIDE_GAP: f32 = 28.0;

/// Draws the map as tall as `budget` points (legend included) where the width
/// allows, and no taller than `room`, the pane's height below its top. `job`
/// keys the squares' fade-in, so each fills once per job.
pub fn show(
    ui: &mut egui::Ui,
    progress: &Progress,
    job: impl std::hash::Hash + std::fmt::Debug,
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
    let room = room - LEGEND;
    let wanted = (budget - LEGEND).min(room);
    let cell_in = |across: bool, height: f32, share: f32, rows: u32| {
        let (columns, stacked) = if across { (sides, 1.0) } else { (1.0, sides) };
        let each_width = (width * share - SIDE_GAP * (columns - 1.0)) / columns;
        let by_width = (each_width - LABEL - (ROW - 1) as f32 * gap) / ROW as f32;
        let each = (height - SIDE_GAP * 0.5 * (stacked - 1.0)) / stacked;
        let by_height = (each - TITLE + gap) / rows as f32 - gap;
        by_width.min(by_height)
    };
    // The usual size where it fits, larger where the budget allows.
    let cell_for = |across: bool| {
        let fits = cell_in(across, room, 1.0, rows);
        let usual = CELL.min(fits);
        cell_in(across, wanted, WIDTH_SHARE, rows.min(SIZED_ROWS))
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
        false => vec2(grid.x, grid.y * sides + SIDE_GAP * 0.5 * (sides - 1.0)),
    };
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect.expand(1.0));
    let radius = CornerRadius::same((cell / 5.0).round() as u8);
    let square = |i: usize, cyl: u32| {
        let origin = match across {
            true => rect.min + vec2(i as f32 * (grid.x + SIDE_GAP), 0.0),
            false => rect.min + vec2(0.0, i as f32 * (grid.y + SIDE_GAP * 0.5)),
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
        for row in first / ROW..=last / ROW {
            let at = square(i, row * ROW);
            painter.text(
                pos2(at.left() - 7.0, at.center().y),
                Align2::RIGHT_CENTER,
                row * ROW,
                FontId::proportional(11.0),
                p.dim,
            );
        }
        for &cyl in &cyls {
            let key = (cyl, head);
            let filled = fill(progress, key, p);
            let t = fade(ui, egui::Id::new(("square", &job, key)), filled.is_some());
            let colour = theme::lerp(p.pending, filled.unwrap_or(p.pending), t);
            painter.rect_filled(square(i, cyl), radius, colour);
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
                    ui.weak("Not read yet.");
                }
            }
        });
    }
    ui.add_space(6.0);
    legend(ui, &shown, progress.tally().retries, p);
}

/// How far a square has faded in, 0 to 1. Timed from the frame it lit up,
/// not by frame gaps, so one lit after an idle spell starts empty. A square
/// done when first seen shows at once.
fn fade(ui: &egui::Ui, id: egui::Id, done: bool) -> f32 {
    let now = ui.input(|i| i.time);
    // When it lit up: infinity while not done, minus infinity if done when first seen.
    let lit = ui.data_mut(|d| {
        let lit = d.get_temp_mut_or_insert_with(id, || f64::NEG_INFINITY);
        if !done {
            *lit = f64::INFINITY;
        } else if *lit == f64::INFINITY {
            *lit = now;
        }
        *lit
    });
    if !done {
        return 0.0;
    }
    let t = ((now - lit) as f32 / FILL_TIME).clamp(0.0, 1.0);
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    egui::emath::easing::cubic_in_out(t)
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
        Status::Skipped => p.pending,
    }
}

/// Each colour on the map with its track count, then the retries.
fn legend(ui: &mut egui::Ui, shown: &[Color32], retries: u32, p: &Palette) {
    ui.horizontal_wrapped(|ui| {
        for (status, name, tip) in [
            (Status::Good, "Good", "Every sector read."),
            (Status::Partial, "Short", "Some sectors missing."),
            (Status::Bad, "Bad", "No sectors read."),
            (Status::Flux, "Flux", "Read as flux, not decoded."),
            (Status::Written, "Written", "Written, not verified."),
            (Status::Erased, "Erased", "Erased."),
        ] {
            let swatch = colour(status, p);
            let tracks = shown.iter().filter(|&&c| c == swatch).count();
            if tracks == 0 {
                continue;
            }
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(2), swatch);
            ui.label(RichText::new(format!("{name} {tracks}")).small())
                .on_hover_text(tip);
            ui.add_space(6.0);
        }
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

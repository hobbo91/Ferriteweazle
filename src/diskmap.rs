//! The disk map: a square per track, in a grid per side.

use crate::progress::{Progress, Status};
use crate::theme::{self, Palette};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, pos2,
    vec2,
};

/// Seconds a square takes to fade in.
const FILL_TIME: f32 = 0.4;

/// Seconds the sides take to slide: to change places as their heads swap, to
/// follow the rows as they come and go, or to go from stacked to side by side.
const SLIDE_TIME: f32 = 0.25;

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

/// Draws the map: a grid of the disk's `(cylinders, sides)`, if known, and any tracks
/// past them, with a square for each track `progress` takes, fading in and out as they
/// come and go. Squares are CELL points: smaller if `room`, the pane's height below
/// its top, lacks space for SIZED_ROWS rows; larger if a `budget`-point map (legend
/// included) and the width allow. More rows run on, and the pane scrolls.
/// `swapped`: the heads swap, and side 1 takes side 0's place.
pub fn show(
    ui: &mut egui::Ui,
    progress: &Progress,
    disk: (u32, u32),
    swapped: bool,
    verifying: bool,
    budget: f32,
    room: f32,
) {
    let (cyls, heads) = progress.layout();
    let past = |set: &[u32]| set.last().map_or(0, |n| n + 1);
    let want = (disk.0.max(past(&cyls)), disk.1.max(past(&heads)).min(2));
    let (span, sides) = grid(ui, want);
    if span == 0 {
        return;
    }
    let p = theme::palette(ui);
    // Whole pixels, so every square and every gap is the same size.
    let ppp = ui.ctx().pixels_per_point();
    let snap = |x: f32| (x * ppp).round() / ppp;
    let gap = snap(GAP);
    let rows = span.div_ceil(ROW);
    let width = ui.available_width();
    // The legend wraps in a narrow pane: last frame's height keeps it in room.
    let legend_id = ui.id().with("legend");
    let legend_height = ui.data(|d| d.get_temp(legend_id)).unwrap_or(LEGEND);
    let room = room - legend_height;
    let wanted = (budget - legend_height.max(LEGEND)).min(room);
    let n = sides as f32;
    // The squares' size that fits `columns` sides side by side, a fraction of
    // one while side 1 slides.
    let by_width = |columns: f32| {
        let each = (width - SIDE_GAP * (columns - 1.0)) / columns;
        (each - LABEL - (ROW - 1) as f32 * gap) / ROW as f32
    };
    let cell_in = |across: bool, height: f32, rows: u32| {
        let (columns, stacked) = if across { (n, 1.0) } else { (1.0, n) };
        let each = (height - STACK_GAP * (stacked - 1.0)) / stacked;
        let by_height = (each - TITLE + gap) / rows as f32 - gap;
        by_width(columns).min(by_height)
    };
    // The usual size where it fits, larger where the budget allows.
    let cell_for = |across: bool| {
        let fits = cell_in(across, room, rows.min(SIZED_ROWS));
        let usual = CELL.min(fits);
        cell_in(across, wanted, SIZED_ROWS)
            .min(fits)
            .max(usual)
            .min(MAX_CELL)
    };
    // Sides across or stacked, whichever gives larger squares, but stacked where
    // the least squares do not fit across. One side keeps the way two went, so
    // it slides into place along it.
    let across_id = ui.id().with("across");
    let across = match sides {
        2 => {
            let across = by_width(2.0) >= MIN_CELL && cell_for(true) >= cell_for(false);
            ui.data_mut(|d| d.insert_temp(across_id, across));
            across
        }
        _ => ui.data(|d| d.get_temp(across_id)).unwrap_or(true),
    };
    // Side 1 and the legend slide as rows and sides come and go.
    let tall = slide(ui, egui::Id::new("map rows"), rows as f32);
    let shown_id = egui::Id::new("map sides");
    let mut shown = slide(ui, shown_id, n);
    // Side 1's place from side 0's: below it or beside it, sliding between as
    // the pane's shape changes. The squares are one size where the two meet.
    let (slid, to) = (egui::Id::new("map across"), f32::from(u8::from(across)));
    // A side not shown has no place to slide from: it comes back in its own.
    if shown <= 1.0 {
        settle(ui, slid, to);
    }
    let mut beside = slide(ui, slid, to);
    // While side 1 slides, part of it beside side 0, the squares shrink to keep the
    // map in the pane. With no room for the least of them, the sides take their
    // places at once.
    let mut columns = 1.0 + (shown - 1.0) * beside;
    if by_width(columns) < MIN_CELL {
        (shown, beside) = (settle(ui, shown_id, n), settle(ui, slid, to));
        columns = 1.0 + (shown - 1.0) * beside;
    }
    let cell = (cell_for(across).min(by_width(columns)).max(MIN_CELL) * ppp).floor() / ppp;
    let step = cell + gap;
    let grid = vec2(
        LABEL + ROW as f32 * cell + (ROW - 1) as f32 * gap,
        TITLE + tall * step - gap,
    );
    let apart = vec2(
        beside * (grid.x + SIDE_GAP),
        (1.0 - beside) * (grid.y + STACK_GAP),
    );
    let (rect, response) = ui.allocate_exact_size(grid + (shown - 1.0) * apart, Sense::hover());
    let painter = ui.painter_at(rect.expand(1.0));
    let radius = CornerRadius::same((cell / 5.0).round() as u8);
    let id = egui::Id::new("swap");
    let easing = egui::emath::easing::cubic_in_out;
    let t =
        ui.ctx()
            .animate_bool_with_time_and_easing(id, swapped && sides == 2, SLIDE_TIME, easing);
    // Each side's place, 0 first, sliding to the other's as the heads swap.
    let place = |head: u32| if head == 0 { t } else { 1.0 - t };
    let square = |head: u32, cyl: u32| {
        let origin = rect.min + place(head) * apart;
        let x = snap(origin.x + LABEL) + (cyl % ROW) as f32 * step;
        let y = snap(origin.y + TITLE) + (cyl / ROW) as f32 * step;
        Rect::from_min_size(pos2(x, y), vec2(cell, cell))
    };
    let taken = |(cyl, head): (u32, u32)| cyls.contains(&cyl) && heads.contains(&head);
    let mut shown = Vec::new();
    // A side's name and row numbers fade as squares do.
    let label = |id: egui::Id, on: bool| shade(ui, id, if on { p.dim } else { p.bg }, p.bg);
    for head in 0..sides {
        let corner = square(head, 0);
        painter.text(
            pos2(corner.left(), corner.top() - TITLE + 2.0),
            Align2::LEFT_TOP,
            side_name(head),
            FontId::proportional(12.0),
            label(egui::Id::new(("side", head)), head < want.1),
        );
        let every = if step < ROW_NUMBER { 2 } else { 1 };
        for row in 0..rows {
            let on = row % every == 0 && row * ROW < want.0 && head < want.1;
            let colour = label(egui::Id::new(("row", head, row)), on);
            if colour != p.bg {
                let at = square(head, row * ROW);
                painter.text(
                    pos2(at.left() - 7.0, at.center().y),
                    Align2::RIGHT_CENTER,
                    row * ROW,
                    FontId::proportional(ROW_NUMBER),
                    colour,
                );
            }
        }
        for cyl in 0..span {
            let key = (cyl, head);
            let filled = fill(progress, key, p);
            let to = match taken(key) {
                true => filled.unwrap_or(p.pending),
                false => p.bg,
            };
            let colour = shade(ui, egui::Id::new(("square", key)), to, p.bg);
            if taken(key) || colour != p.bg {
                let edge = edge(status(progress, key) == Some(Status::Skipped), p);
                painter.rect(square(head, cyl), radius, colour, edge, StrokeKind::Inside);
            }
            if taken(key) {
                shown.extend(filled);
            }
        }
    }
    let hovered = response.hover_pos().and_then(|pos| {
        heads.iter().find_map(|&head| {
            cyls.iter()
                .find(|&&c| square(head, c).expand(gap / 2.0).contains(pos))
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
                    if let Some(place) = &t.place {
                        ui.weak(place_text(place));
                    }
                    if t.retries > 0 {
                        ui.weak(retry_text(t.retries));
                    }
                }
                None if status(progress, (cyl, head)) == Some(Status::Skipped) => {
                    ui.label("Not in the image.");
                }
                None => {
                    ui.weak("Greaseweazle Tools has not reported this track.");
                }
            }
            if let Some(rows) = missing(progress, (cyl, head)) {
                ui.label(format!(
                    "Missing in Greaseweazle Tools' sector map (S): {rows}"
                ));
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

/// The cylinders and sides the grid spans for `want`: at once where it grows, but
/// where it shrinks only once the squares it loses have had FILL_TIME to fade out.
fn grid(ui: &egui::Ui, want: (u32, u32)) -> (u32, u32) {
    let now = ui.input(|i| i.time);
    let id = egui::Id::new("disk grid");
    let (drawn, wanted, since) = ui.data(|d| d.get_temp(id)).unwrap_or((want, want, now));
    let since = if want == wanted { since } else { now };
    let grows = want.0 >= drawn.0 && want.1 >= drawn.1;
    let drawn = match grows || now - since >= f64::from(FILL_TIME) {
        true => want,
        false => (drawn.0.max(want.0), drawn.1.max(want.1)),
    };
    if drawn != want {
        ui.ctx().request_repaint();
    }
    ui.data_mut(|d| d.insert_temp(id, (drawn, want, since)));
    drawn
}

/// A number's slide over SLIDE_TIME to `to` from where it was when `to` last
/// changed; `to` at once when first seen.
fn slide(ui: &egui::Ui, id: egui::Id, to: f32) -> f32 {
    let now = ui.input(|i| i.time);
    let at = |(from, to, since): (f32, f32, f64)| {
        let t = ((now - since) as f32 / SLIDE_TIME).clamp(0.0, 1.0);
        egui::lerp(from..=to, egui::emath::easing::cubic_in_out(t))
    };
    let slide = ui.data_mut(|d| {
        let slide = d.get_temp_mut_or_insert_with(id, || (to, to, now));
        if slide.1 != to {
            *slide = (at(*slide), to, now);
        }
        *slide
    });
    if now - slide.2 < f64::from(SLIDE_TIME) {
        ui.ctx().request_repaint();
    }
    at(slide)
}

/// Ends a slide at `to` at once, and gives `to`.
fn settle(ui: &egui::Ui, id: egui::Id, to: f32) -> f32 {
    ui.data_mut(|d| d.insert_temp(id, (to, to, f64::NEG_INFINITY)));
    to
}

/// A square's fade from the colour it showed when its target last changed.
#[derive(Clone, Copy)]
struct Shade {
    from: Color32,
    to: Color32,
    /// When it changed, in egui's seconds.
    at: f64,
    /// The background it was set against, which a theme change changes.
    bg: Color32,
}

impl Shade {
    fn colour(self, now: f64) -> Color32 {
        let t = ((now - self.at) as f32 / FILL_TIME).clamp(0.0, 1.0);
        theme::lerp(self.from, self.to, egui::emath::easing::cubic_in_out(t))
    }
}

/// A square's colour, fading over FILL_TIME to `to` from what it showed when that
/// last changed, or from the background `bg` when first seen; at once to a new
/// theme's colour. Timed from that frame, not by frame gaps, so a square lit
/// after an idle spell starts empty.
fn shade(ui: &egui::Ui, id: egui::Id, to: Color32, bg: Color32) -> Color32 {
    let now = ui.input(|i| i.time);
    let shade = ui.data_mut(|d| {
        let shade = d.get_temp_mut_or_insert_with(id, || Shade {
            from: bg,
            to,
            at: now,
            bg,
        });
        if shade.bg != bg {
            *shade = Shade {
                from: to,
                to,
                at: f64::NEG_INFINITY,
                bg,
            };
        } else if shade.to != to {
            *shade = Shade {
                from: shade.colour(now),
                to,
                at: now,
                bg,
            };
        }
        *shade
    });
    if now - shade.at < f64::from(FILL_TIME) {
        ui.ctx().request_repaint();
    }
    shade.colour(now)
}

/// A track's colour, by its sectors once gw has mapped them; `None` until gw
/// reports the track. One gw's image does not hold is skipped.
pub(crate) fn fill(progress: &Progress, key: (u32, u32), p: &Palette) -> Option<Color32> {
    let Some(sectors) = progress.sector_map.get(&key) else {
        return status(progress, key).map(|s| status_colour(s, p));
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

/// A track's status as gw's line gives it, or skipped where gw's image does
/// not hold it, which gw gives no line.
fn status(progress: &Progress, key: (u32, u32)) -> Option<Status> {
    let line = progress.tracks.get(&key).map(|t| t.status);
    let absent = progress.facts.get(&key).is_some_and(|f| f.absent);
    line.or(absent.then_some(Status::Skipped))
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

/// A track's colour on the map for its status.
pub(crate) fn status_colour(status: Status, p: &Palette) -> Color32 {
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

/// Each colour on the map with its track count, then the retries. `verifying`:
/// the one written track is the one gw is writing and checking.
fn legend(ui: &mut egui::Ui, shown: &[Color32], progress: &Progress, verifying: bool, p: &Palette) {
    // Rows a line of its text apart where it wraps, not a field's height:
    // a wrapping row takes its height as it is made.
    let row = ui.text_style_height(&egui::TextStyle::Small).max(10.0);
    let spacing = ui.spacing_mut();
    let kept = (spacing.interact_size.y, spacing.item_spacing.y);
    (spacing.interact_size.y, spacing.item_spacing.y) = (row, 4.0);
    ui.horizontal_wrapped(|ui| {
        let font = egui::TextStyle::Small.resolve(ui.style());
        let gap = ui.spacing().item_spacing.x;
        for (swatch, skipped, name, tracks, tip) in entries(shown, progress, verifying, p) {
            // The grid's squares are tracks: its counts need no word for them.
            let text = tracks.map_or(name.to_owned(), |n| format!("{name} {n}"));
            let colour = ui.visuals().text_color();
            let galley = ui
                .painter()
                .layout_no_wrap(text.clone(), font.clone(), colour);
            // Each entry whole on its row: its swatch and its name never apart.
            let size = vec2(10.0 + gap + galley.size().x, galley.size().y.max(10.0));
            let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
            let square = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.center().y - 5.0),
                vec2(10.0, 10.0),
            );
            let edge = edge(skipped, p);
            ui.painter().rect(
                square,
                CornerRadius::same(2),
                swatch,
                edge,
                StrokeKind::Inside,
            );
            let at = egui::pos2(
                square.right() + gap,
                rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(at, galley, colour);
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
            response.on_hover_text(tip);
            ui.add_space(6.0);
        }
        let retries = progress.tally().retries;
        if retries > 0 {
            ui.label(RichText::new(retry_text(retries)).small().weak());
        }
    });
    let spacing = ui.spacing_mut();
    (spacing.interact_size.y, spacing.item_spacing.y) = kept;
}

/// The legend's entries for tracks shown in `shown` colours, as the map
/// shows them: each status's colour, whether it is a skipped track's, its
/// name, how many tracks show it where that is told, and what it means.
pub(crate) fn entries<'a>(
    shown: &[Color32],
    progress: &'a Progress,
    verifying: bool,
    p: &Palette,
) -> Vec<(Color32, bool, &'static str, Option<usize>, &'a str)> {
    let written = match verifying {
        true => "The track Greaseweazle Tools is writing and checking.",
        false => progress
            .unverified
            .as_deref()
            .unwrap_or("Written, no verify reported."),
    };
    [
        (
            Status::Good,
            "Good",
            "Every sector found, or written and verified.",
        ),
        (
            Status::Partial,
            "Sectors missing",
            "Some of the track's sectors not read.",
        ),
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
    ]
    .into_iter()
    .filter_map(|(status, name, tip)| {
        let swatch = status_colour(status, p);
        let tracks = shown.iter().filter(|&&c| c == swatch).count();
        (tracks > 0).then(|| {
            // The one track gw is writing and checking.
            let count = (status != Status::Written || !verifying).then_some(tracks);
            (swatch, status == Status::Skipped, name, count, tip)
        })
    })
    .collect()
}

/// `n` tracks, as a legend counts them where it also counts sectors.
pub(crate) fn tracks(n: usize) -> String {
    match n {
        1 => "1 track".to_owned(),
        n => format!("{n} tracks"),
    }
}

/// Where gw read or wrote the track, from gw's `Drive 10.1` or `Image 10.1`:
/// the cylinder and head a step, a swap or an offset took it to.
pub(crate) fn place_text(place: &str) -> String {
    let parts = place
        .split_once(' ')
        .and_then(|(what, at)| Some((what, at.split_once('.')?)));
    match parts {
        Some((what, (c, h))) => format!("{what} cylinder {c}, head {h}."),
        None => place.to_owned(),
    }
}

pub(crate) fn retry_text(n: u32) -> String {
    match n {
        1 => "1 retry".into(),
        n => format!("{n} retries"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tracks_place_names_the_drive_or_image_cylinder_and_head() {
        assert_eq!(place_text("Drive 10.1"), "Drive cylinder 10, head 1.");
        assert_eq!(place_text("Image 0.0"), "Image cylinder 0, head 0.");
        assert_eq!(place_text("elsewhere"), "elsewhere", "kept as gw put it");
        assert_eq!(retry_text(1), "1 retry");
        assert_eq!(retry_text(3), "3 retries");
    }

    #[test]
    fn a_track_gws_image_does_not_hold_is_skipped() {
        let mut progress = Progress::default();
        progress.feed("Converting c=0-1:h=0 -> c=0-1:h=0");
        progress.report(r#"{"c":1,"h":0,"absent":true}"#);
        assert_eq!(status(&progress, (1, 0)), Some(Status::Skipped));
        assert_eq!(status(&progress, (0, 0)), None, "not yet reported");
        let p = &theme::DARK;
        assert_eq!(
            fill(&progress, (1, 0), p),
            Some(status_colour(Status::Skipped, p))
        );
    }
}

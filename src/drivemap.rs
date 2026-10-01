//! The drive: the disk seen from above with the heads on their rail at 12
//! o'clock, for the pages that move the heads or time the spindle.

use crate::form::LAST_USUAL_CYLINDER;
use crate::theme;
use eframe::egui::{self, Rect, RichText, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use std::f32::consts::{FRAC_PI_4, TAU};
use std::time::Duration;

/// The least size of the disk wherever the pane has room for it.
const MIN_SIZE: f32 = 120.0;
/// Seconds the heads take to step on screen.
const STEP_TIME: f32 = 0.12;
/// Turns a second while the motor runs: slow enough to follow, not the spindle's.
const TURNS: f32 = 0.5;

/// What the drive shows.
pub struct Drive {
    /// The cylinders a pass sweeps: Clean's.
    pub band: Option<(u32, u32)>,
    /// The cylinder Seek goes to.
    pub ring: Option<u32>,
    /// The heads' cylinder, and whether they are there or only would go there.
    pub head: (u32, bool),
    /// The motor runs, so the disk turns.
    pub spinning: bool,
    /// The reading over the disk, and a line under it.
    pub title: String,
    pub detail: String,
}

/// Draws `drive`'s lines, then the disk as wide as the pane and no taller than
/// `budget`, or than what `room`, the pane's height below its top, leaves, down
/// to MIN_SIZE.
pub fn show(ui: &mut egui::Ui, drive: &Drive, budget: f32, room: f32) {
    let p = theme::palette(ui);
    let top = ui.cursor().top();
    ui.label(RichText::new(&drive.title).size(15.0).strong());
    ui.label(RichText::new(&drive.detail).small().color(p.dim));
    ui.add_space(10.0);
    let left = room - (ui.cursor().top() - top);
    let width = ui.available_width();
    let size = width.min(budget).min(left.max(MIN_SIZE)).max(0.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, size), Sense::hover());
    let painter = ui.painter_at(rect);
    let centre = rect.center();
    let r = size / 2.0 - 1.0;
    // Cylinder 0 near the edge, LAST_USUAL_CYLINDER near the hub.
    let (outer, inner, hub) = (r * 0.9, r * 0.4, r * 0.22);
    let radius = |c: u32| {
        let c = c.min(LAST_USUAL_CYLINDER) as f32 / LAST_USUAL_CYLINDER as f32;
        outer - c * (outer - inner)
    };
    painter.circle(centre, r, p.pending, Stroke::new(1.0, p.line));
    // A ring of the disk's colour over a disc of the band's: egui draws a
    // circle's stroke outside it, so a wide one would not keep to the band.
    if let Some((first, last)) = drive.band {
        painter.circle_filled(centre, radius(first), p.flux.gamma_multiply(0.6));
        painter.circle_filled(centre, radius(last), p.pending);
    }
    if let Some(c) = drive.ring {
        painter.circle_stroke(centre, radius(c), Stroke::new(2.0, p.accent));
    }
    painter.circle(centre, hub, p.bg, Stroke::new(1.0, p.line_strong));
    // The index hole turns with the disk, from wherever it last stopped.
    let id = ui.id().with("index");
    let mut angle: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(-FRAC_PI_4);
    if drive.spinning {
        angle = (angle + ui.input(|i| i.stable_dt).min(0.1) * TURNS * TAU) % TAU;
        ui.data_mut(|d| d.insert_temp(id, angle));
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
    let index = centre + vec2(angle.cos(), angle.sin()) * (hub + r * 0.09);
    painter.circle(index, r * 0.035, p.bg, Stroke::new(1.0, p.line_strong));
    if drive.spinning {
        turning(&painter, centre, r * 0.95, Stroke::new(1.5, p.dim));
    }
    // The heads' rail runs from the hub to the edge.
    let rail = [
        pos2(centre.x, centre.y - hub - 3.0),
        pos2(centre.x, centre.y - r),
    ];
    painter.line_segment(rail, Stroke::new(1.5, p.line_strong));
    let (cyl, there) = drive.head;
    let from_centre =
        ui.ctx()
            .animate_value_with_time(ui.id().with("heads"), radius(cyl), STEP_TIME);
    let head = Rect::from_center_size(
        pos2(centre.x, centre.y - from_centre),
        vec2(r * 0.16, r * 0.1),
    );
    match there {
        true => painter.rect_filled(head, 2.0, p.partial),
        false => painter.rect_stroke(head, 2.0, Stroke::new(1.5, p.partial), StrokeKind::Inside),
    };
}

/// An arrow a quarter of the way round at 3 o'clock, clockwise as the disk turns.
fn turning(painter: &egui::Painter, centre: egui::Pos2, radius: f32, stroke: Stroke) {
    let at = |a: f32| centre + vec2(a.cos(), a.sin()) * radius;
    let (start, end) = (-FRAC_PI_4, FRAC_PI_4);
    let points: Vec<_> = (0..=16)
        .map(|i| at(start + (end - start) * i as f32 / 16.0))
        .collect();
    painter.add(Shape::line(points, stroke));
    // The head of the arrow, back along the circle from its tip.
    let tip = at(end);
    for side in [-1.0, 1.0] {
        let back = at(end - 0.12) + vec2(end.cos(), end.sin()) * side * 4.0;
        painter.line_segment([tip, back], stroke);
    }
}

/// Clean's pass, from 0, and the cylinder gw last sent the heads to, from its
/// lines such as `Pass 1: 9 0 19`, the one gw is still printing last.
pub fn clean_progress<'a>(lines: impl Iterator<Item = &'a str>) -> Option<(u32, Option<u32>)> {
    let mut found: Option<(u32, Option<u32>)> = None;
    for line in lines {
        let Some((pass, cyls)) = line.strip_prefix("Pass ").and_then(|l| l.split_once(':')) else {
            continue;
        };
        let Ok(pass) = pass.trim().parse() else {
            continue;
        };
        let at = cyls.split_whitespace().rev().find_map(|c| c.parse().ok());
        found = Some((pass, at.or(found.and_then(|(_, at)| at))));
    }
    found
}

/// gw rpm's reading: its mean once it gives one.
#[derive(Debug, PartialEq)]
pub struct Reading {
    pub rpm: f32,
    /// Milliseconds a revolution.
    pub period: f32,
    /// How many revolutions the reading takes in.
    pub of: usize,
}

/// The reading in gw rpm's lines, such as `Rate: 300.123 rpm ; Period: 199.918 ms`.
pub fn reading<'a>(lines: impl Iterator<Item = &'a str>) -> Option<Reading> {
    let (mut last, mut mean, mut of) = (None, None, 0);
    for line in lines {
        if let Some(rate) = rate(line) {
            (last, of) = (Some(rate), of + 1);
        } else if let Some(rate) = line.strip_prefix("Ar.Mean:").and_then(|l| rate(l.trim())) {
            mean = Some(rate);
        }
    }
    let ((rpm, period), of) = match mean {
        Some(mean) => (mean, of),
        None => (last?, 1),
    };
    Some(Reading { rpm, period, of })
}

fn rate(line: &str) -> Option<(f32, f32)> {
    let (rpm, period) = line.strip_prefix("Rate:")?.split_once(';')?;
    let rpm = rpm.trim().strip_suffix("rpm")?.trim().parse().ok()?;
    let period = period.trim().strip_prefix("Period:")?;
    let period = period.trim().strip_suffix("ms")?.trim().parse().ok()?;
    Some((rpm, period))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_pass_and_cylinder_come_from_its_lines_and_the_one_it_is_printing() {
        let lines = ["Pass 0: 9 0 19 10 29 20", "Pass 1: "];
        assert_eq!(clean_progress(lines.into_iter()), Some((1, Some(20))));
        let lines = ["Pass 0: 9 0", "Pass 1: 9 0 19"];
        assert_eq!(clean_progress(lines.into_iter()), Some((1, Some(19))));
        assert_eq!(clean_progress(["Command Failed: x"].into_iter()), None);
    }

    #[test]
    fn a_speed_reading_is_the_last_rate_or_gws_mean_of_them() {
        let one = ["Rate: 300.123 rpm ; Period: 199.918 ms"];
        let expected = Reading {
            rpm: 300.123,
            period: 199.918,
            of: 1,
        };
        assert_eq!(reading(one.into_iter()), Some(expected));
        let three = [
            "Rate: 300.1 rpm ; Period: 199.9 ms",
            "Rate: 299.9 rpm ; Period: 200.1 ms",
            "Rate: 300.0 rpm ; Period: 200.0 ms",
            "***",
            "FASTEST:  Rate: 300.1 rpm ; Period: 199.9 ms",
            "Ar.Mean:  Rate: 300.0 rpm ; Period: 200.0 ms",
            "Median:   Rate: 300.0 rpm ; Period: 200.0 ms",
        ];
        let mean = reading(three.into_iter()).unwrap();
        assert_eq!((mean.rpm, mean.of), (300.0, 3));
        assert_eq!(reading(["Command Failed: x"].into_iter()), None);
    }
}

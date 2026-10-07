//! A box of lines to read, such as gw's output or a sector's bytes: drawn as
//! they scroll into view, selected with the pointer, which scrolls the box on
//! while it is dragged past the box's edge, and copied with Copy on a right
//! click or the keyboard's.

use eframe::egui::{
    self, Color32, CursorIcon, FontId, Key, Modifiers, Pos2, Rect, Sense, Ui, Vec2, pos2, vec2,
};

/// How fast a selection dragged past the box's edge scrolls it, in points a
/// second for each point past, at least and at most.
const SCROLL: f32 = 10.0;
const SCROLL_LEAST: f32 = 60.0;
const SCROLL_MOST: f32 = 3000.0;

/// A place in the lines: a line, and a character in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct At {
    line: usize,
    char: usize,
}

/// Where a selection was begun and where it reaches, and whether the pointer
/// is still drawing it.
#[derive(Clone, Copy, Debug, Default)]
struct Selection {
    anchor: At,
    head: At,
    dragging: bool,
}

impl Selection {
    fn range(&self) -> (At, At) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// The lines a box shows.
pub struct Lines<'a> {
    pub count: usize,
    /// Line `i`, and the colour it is drawn in.
    pub line: &'a dyn Fn(usize) -> (&'a str, Color32),
    pub font: FontId,
    /// Room between one line and the next, beyond the font's own.
    pub gap: f32,
}

impl Lines<'_> {
    /// Shows the lines in `area`, their selection kept under `id`.
    pub fn show(&self, ui: &mut Ui, id: egui::Id, area: egui::ScrollArea) {
        let row = ui.fonts_mut(|f| f.row_height(&self.font));
        // The lines are in a fixed-width font: each character this wide.
        let advance = ui.fonts_mut(|f| f.glyph_width(&self.font, '0'));
        let widest = (0..self.count)
            .map(|i| (self.line)(i).0.chars().count())
            .max()
            .unwrap_or(0);
        let step = row + self.gap;
        let size = vec2(
            widest as f32 * advance + 1.0,
            (step * self.count as f32 - self.gap).max(row),
        );
        let shape = Shape { row, step, advance };
        area.show_viewport(ui, |ui, viewport| {
            self.contents(ui, id, viewport, size, shape);
        });
    }

    fn contents(&self, ui: &mut Ui, id: egui::Id, viewport: Rect, size: Vec2, shape: Shape) {
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        if self.count == 0 {
            return;
        }
        let mut selection: Option<Selection> = ui.data(|d| d.get_temp(id));
        let at = |p: Pos2| self.at(rect, shape, p);
        let (pressed, down, pointer, shift) = ui.input(|i| {
            let p = &i.pointer;
            (
                p.primary_pressed(),
                p.primary_down(),
                p.interact_pos(),
                i.modifiers.shift,
            )
        });
        if response.contains_pointer() || selection.is_some_and(|s| s.dragging) {
            ui.set_cursor_icon(CursorIcon::Text);
        }
        // A press over the lines begins a selection there, or with Shift,
        // carries the one there is on to there.
        if let Some(p) = pointer.filter(|_| pressed && response.contains_pointer()) {
            let anchor = match selection {
                Some(s) if shift => s.anchor,
                _ => at(p),
            };
            selection = Some(Selection {
                anchor,
                head: at(p),
                dragging: true,
            });
            response.request_focus();
        }
        if let Some(s) = selection.as_mut().filter(|s| s.dragging) {
            match pointer.filter(|_| down) {
                Some(p) => {
                    s.head = at(p);
                    scroll_toward(ui, p);
                }
                None => s.dragging = false,
            }
        }
        // The keyboard's Copy and Select All, and the right click's.
        let mut copy = false;
        let mut all = false;
        if response.has_focus() {
            ui.input_mut(|i| {
                copy |= i.events.iter().any(|e| matches!(e, egui::Event::Copy));
                all |= i.consume_key(Modifiers::COMMAND, Key::A);
                if i.consume_key(Modifiers::NONE, Key::Escape) {
                    selection = None;
                }
            });
        }
        let selected = selection.is_some_and(|s| !s.is_empty());
        response.context_menu(|ui| {
            let item = ui.add_enabled(selected, egui::Button::new("Copy"));
            if item.on_disabled_hover_text("Nothing selected.").clicked() {
                copy = true;
                ui.close();
            }
            if ui.button("Select All").clicked() {
                all = true;
                ui.close();
            }
        });
        if all {
            let last = self.count - 1;
            let end = (self.line)(last).0.chars().count();
            selection = Some(Selection {
                anchor: At::default(),
                head: At {
                    line: last,
                    char: end,
                },
                dragging: false,
            });
            response.request_focus();
        }
        if let Some(s) = selection.filter(|s| copy && !s.is_empty()) {
            ui.ctx().copy_text(self.text(s));
        }
        self.paint(ui, id, rect, viewport, shape, selection);
        ui.data_mut(|d| match selection {
            Some(s) => {
                d.insert_temp(id, s);
            }
            None => d.remove::<Selection>(id),
        });
    }

    /// The place `p` points at, on the lines laid out in `rect`: before the
    /// first line, its start; past the last, its end.
    fn at(&self, rect: Rect, shape: Shape, p: Pos2) -> At {
        let last = self.count - 1;
        if p.y < rect.top() {
            return At::default();
        }
        let line = ((p.y - rect.top()) / shape.step).floor() as usize;
        if line > last {
            let char = (self.line)(last).0.chars().count();
            return At { line: last, char };
        }
        let len = (self.line)(line).0.chars().count();
        let char = ((p.x - rect.left()) / shape.advance).round();
        At {
            line,
            char: (char.max(0.0) as usize).min(len),
        }
    }

    /// The text of `selection`, a line to a line.
    fn text(&self, selection: Selection) -> String {
        let (a, b) = selection.range();
        let lines: Vec<String> = (a.line..=b.line)
            .map(|i| {
                let text = (self.line)(i).0;
                let from = if i == a.line { a.char } else { 0 };
                let to = if i == b.line { b.char } else { usize::MAX };
                text.chars()
                    .skip(from)
                    .take(to.saturating_sub(from))
                    .collect()
            })
            .collect();
        lines.join("\n")
    }

    /// Draws the lines `viewport` shows, the selection behind them; and names
    /// each, as a label, to screen readers.
    fn paint(
        &self,
        ui: &Ui,
        id: egui::Id,
        rect: Rect,
        viewport: Rect,
        shape: Shape,
        selection: Option<Selection>,
    ) {
        let first = (viewport.min.y / shape.step).floor().max(0.0) as usize;
        let past = ((viewport.max.y / shape.step).ceil() as usize + 1).min(self.count);
        let range = selection.filter(|s| !s.is_empty()).map(|s| s.range());
        let painter = ui.painter();
        for i in first..past {
            let top = rect.top() + i as f32 * shape.step;
            let (text, colour) = (self.line)(i);
            if let Some((a, b)) = range.filter(|(a, b)| (a.line..=b.line).contains(&i)) {
                let len = text.chars().count();
                let from = if i == a.line { a.char } else { 0 };
                // A line selected to its end shows its end selected too.
                let to = if i == b.line { b.char } else { len + 1 };
                let x = |char: usize| rect.left() + char as f32 * shape.advance;
                let marked = Rect::from_min_max(pos2(x(from), top), pos2(x(to), top + shape.row));
                painter.rect_filled(marked, 0.0, ui.visuals().selection.bg_fill);
            }
            let galley = painter.layout_no_wrap(text.to_owned(), self.font.clone(), colour);
            painter.galley(pos2(rect.left(), top), galley, colour);
            let line = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), shape.row));
            ui.interact(line, id.with(("line", i)), Sense::hover())
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
        }
    }
}

/// A box's lines' measures: a line's height, from one line to the next, and
/// a character's width.
#[derive(Clone, Copy)]
struct Shape {
    row: f32,
    step: f32,
    advance: f32,
}

/// Scrolls the box toward `p` while it lies past the box's edge, the faster
/// the further past: a selection dragged there carries on into what was
/// hidden.
fn scroll_toward(ui: &Ui, p: Pos2) {
    let shown = ui.clip_rect();
    let past = |v: f32, low: f32, high: f32| {
        let d = if v < low {
            v - low
        } else if v > high {
            v - high
        } else {
            0.0
        };
        if d == 0.0 {
            0.0
        } else {
            d.signum() * (d.abs() * SCROLL).clamp(SCROLL_LEAST, SCROLL_MOST)
        }
    };
    let speed = vec2(
        past(p.x, shown.left(), shown.right()),
        past(p.y, shown.top(), shown.bottom()),
    );
    if speed != Vec2::ZERO {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let none = egui::style::ScrollAnimation::none();
        ui.scroll_with_delta_animation(-speed * dt, none);
        ui.ctx().request_repaint();
    }
}

//! A box of lines to read, such as gw's output or a sector's bytes: drawn as
//! they scroll into view, a long one only as far across as shows; selected
//! with the pointer as text is, a word at a time from a double click and a
//! line from a triple, scrolling the box on while dragged past its edge; and
//! copied with Copy on a right click or the keyboard's.

use eframe::egui::{
    self, Color32, CursorIcon, EventFilter, FontId, Galley, Key, Modifiers, Pos2, Rect, Sense, Ui,
    Vec2, Vec2b,
    emath::GuiRounding,
    pos2,
    text::{CCursor, LayoutJob},
    vec2,
};
use std::collections::HashMap;
use std::sync::Arc;

/// How fast a selection dragged past the box's edge scrolls it, in points a
/// second for each point past, at least and at most.
const SCROLL: f32 = 10.0;
const SCROLL_LEAST: f32 = 60.0;
const SCROLL_MOST: f32 = 3000.0;
/// A line up to this many characters is laid out whole; a longer one only
/// where it shows, in pieces this many characters long.
const LONG: usize = 1024;
const PIECE: usize = 256;
/// How many characters at each end of a selection it is known by: it is
/// let go once the lines no longer hold them there.
const MARK: usize = 64;

/// A place in the lines: a line, counted from the first the box has held,
/// and a character in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct At {
    line: usize,
    char: usize,
}

/// What a press selects, and a drag from it carries the selection on by:
/// characters from one press, words from two in a row, lines from three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Char,
    Word,
    Line,
}

/// Where a selection was begun and where it reaches; what it goes by, and
/// the word or line its press selected; whether the pointer is still
/// drawing it; and the text at its ends, which it is known by.
#[derive(Clone, Debug)]
struct Selection {
    anchor: At,
    head: At,
    unit: Unit,
    first: (At, At),
    dragging: bool,
    ends: (String, String),
}

impl Selection {
    /// A selection from `anchor` to `head`, going by `unit` from what its
    /// press selected, `first`.
    fn new(anchor: At, head: At, unit: Unit, first: (At, At)) -> Selection {
        Selection {
            anchor,
            head,
            unit,
            first,
            dragging: true,
            ends: Default::default(),
        }
    }

    fn range(&self) -> (At, At) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// What a box keeps from one frame to the next, under its id.
#[derive(Clone, Default)]
struct State {
    selection: Option<Selection>,
    /// The pass it was last shown in.
    pass: Option<u64>,
    /// The font, and the pixels to a point, it measured its lines in.
    metric: Option<(FontId, f32)>,
    /// The first line it held when it measured its lines, and the line past
    /// the last it measured, which may yet grow; the widest of them, and the
    /// last that wide; and how the first and that last began.
    first: usize,
    measured: usize,
    widest: f32,
    widest_line: usize,
    marks: (String, String),
    /// The widths of characters past printable ASCII, each laid out alone.
    widths: HashMap<char, f32>,
    /// Whether its lines ran past the box across and down when last shown:
    /// only then does it scroll that way.
    overflow: Vec2b,
    /// Its area's offset when last shown, and the most it could take.
    offset: Vec2,
    most: Vec2,
    /// Its last press: when, where, and how many it made in a row.
    press: Option<(f64, Pos2, u8)>,
}

/// The lines a box shows.
pub struct Lines<'a> {
    pub count: usize,
    /// Line `i`, and the colour it is drawn in.
    pub line: &'a dyn Fn(usize) -> (&'a str, Color32),
    /// A fixed-width font: its printable ASCII each as wide as '0'.
    pub font: FontId,
    /// Room between one line and the next, beyond the font's own.
    pub gap: f32,
}

/// A box past the lines it shows: its name to screen readers; how many
/// lines it has dropped before its first, as the session's log drops its
/// oldest, so that a selection keeps to the lines it was made on; and
/// whether it fills its area however few its lines, so that a press
/// anywhere in it reaches them.
#[derive(Clone, Copy, Default)]
pub struct Pane<'a> {
    pub name: &'a str,
    pub first: usize,
    pub fills: bool,
}

impl Lines<'_> {
    /// Shows the lines in `area` in the box `pane` says, their selection
    /// kept under `id`.
    pub fn show_as(&self, ui: &mut Ui, id: egui::Id, area: egui::ScrollArea, pane: Pane) {
        Held { lines: self, pane }.show(ui, id, area);
    }
}

/// A box's lines, each counted from the first it has held, and its pane.
struct Held<'l, 'a> {
    lines: &'l Lines<'a>,
    pane: Pane<'l>,
}

impl<'a> Held<'_, 'a> {
    /// Line `n`, and the colour it is drawn in.
    fn get(&self, n: usize) -> (&'a str, Color32) {
        (self.lines.line)(n - self.pane.first)
    }

    /// The line past the last.
    fn end(&self) -> usize {
        self.pane.first + self.lines.count
    }

    /// Shows the lines in `area`, their selection kept under `id`.
    fn show(&self, ui: &mut Ui, id: egui::Id, area: egui::ScrollArea) {
        let (lines, pane) = (self.lines, self.pane);
        let pass = ui.ctx().cumulative_pass_nr();
        let metric = (lines.font.clone(), ui.ctx().pixels_per_point());
        let mut state = ui.data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<State>(id)));
        // Not shown the pass before, the box may hold other lines now; and
        // in another font, they are another width.
        let gone = state.pass.is_none_or(|p| p + 1 < pass);
        if gone || state.metric.as_ref() != Some(&metric) {
            state = State {
                metric: Some(metric),
                ..State::default()
            };
        }
        state.pass = Some(pass);
        let row = ui.fonts_mut(|f| f.row_height(&lines.font));
        // The font is fixed-width: its printable ASCII characters are each
        // this wide.
        let advance = ui.fonts_mut(|f| f.glyph_width(&lines.font, '0'));
        let shape = Shape {
            row,
            step: row + lines.gap,
            advance,
        };
        self.measure(ui, &mut state, shape);
        let size = vec2(
            state.widest + 1.0,
            (shape.step * lines.count as f32 - lines.gap).max(row),
        );
        // A box that fills its area keeps all of its room, from where the
        // area begins, and takes a press anywhere in it.
        let area = match pane.fills {
            true => area.auto_shrink(false),
            false => area,
        };
        let corner = ui.available_rect_before_wrap().min;
        let mut drawn_at = Vec2::ZERO;
        let output = area.show_viewport(ui, |ui, viewport| {
            drawn_at = viewport.min.to_vec2();
            let room = pane
                .fills
                .then(|| Rect::from_min_size(corner, viewport.size()));
            self.contents(ui, id, size, shape, room, &mut state);
        });
        let (content, inner) = (output.content_size, output.inner_rect.size());
        state.overflow = Vec2b::new(content.x > inner.x + 0.5, content.y > inner.y + 0.5);
        state.offset = output.state.offset;
        state.most = (content - inner).max(Vec2::ZERO);
        // The area may move once the lines are drawn, to keep to the last or
        // within them: draw them again where it moved.
        if output.state.offset != drawn_at {
            ui.ctx().request_repaint();
        }
        ui.data_mut(|d| d.insert_temp(id, state));
    }

    /// Measures the lines new since the box last measured them, and the last
    /// it measured, which may have grown; or all of them again where the
    /// widest went with lines dropped from the start, where the first or
    /// that last no longer begins as it did, or where there are fewer. The
    /// widest sets how far the box scrolls across.
    fn measure(&self, ui: &Ui, state: &mut State, shape: Shape) {
        let (first, end) = (self.pane.first, self.end());
        // Of the lines it measured, those it still holds, as they began: the
        // first is gone where lines were dropped from the start.
        let held = (state.first..state.measured).contains(&first) && state.measured <= end;
        let same = held
            && (first > state.first || self.get(first).0.starts_with(&state.marks.0))
            && self.get(state.measured - 1).0.starts_with(&state.marks.1);
        let from = match same && state.widest_line >= first {
            true => state.measured - 1,
            false => {
                state.widest = 0.0;
                first
            }
        };
        for n in from..end {
            let width = self.width(ui, self.get(n).0, shape, &mut state.widths);
            if width >= state.widest {
                (state.widest, state.widest_line) = (width, n);
            }
        }
        state.first = first;
        state.measured = end;
        let mark = |n: usize| self.get(n).0.chars().take(MARK).collect::<String>();
        state.marks = match self.lines.count {
            0 => Default::default(),
            _ => (mark(first), mark(end - 1)),
        };
    }

    /// How wide `text` is laid out, in points: its printable ASCII
    /// characters each the font's width for them, any other as wide as it
    /// is laid out alone.
    fn width(&self, ui: &Ui, text: &str, shape: Shape, widths: &mut HashMap<char, f32>) -> f32 {
        if printable(text) {
            return text.len() as f32 * shape.advance;
        }
        let each = text.chars().map(|c| self.char_width(ui, c, shape, widths));
        each.sum()
    }

    fn char_width(&self, ui: &Ui, c: char, shape: Shape, widths: &mut HashMap<char, f32>) -> f32 {
        if (' '..='~').contains(&c) {
            return shape.advance;
        }
        *widths.entry(c).or_insert_with(|| {
            let placeholder = Color32::PLACEHOLDER;
            let font = self.lines.font.clone();
            let mut job = LayoutJob::simple(c.to_string(), font, placeholder, f32::INFINITY);
            job.round_output_to_gui = false;
            ui.fonts_mut(|f| f.layout_job(job)).intrinsic_size().x
        })
    }

    fn contents(
        &self,
        ui: &mut Ui,
        id: egui::Id,
        size: Vec2,
        shape: Shape,
        room: Option<Rect>,
        state: &mut State,
    ) {
        let (at, space) = ui.allocate_space(size);
        let rect = ui.layout().align_size_within_rect(size, space);
        // The lines take a press where they lie; a box that fills its area,
        // anywhere in the room it shows, past them too.
        let reach = room.map_or(space, |room| space.union(room));
        let mut response = ui.interact(reach, at, Sense::click_and_drag());
        response.set_intrinsic_size(size);
        let name = self.pane.name;
        if !name.is_empty() {
            let info = || egui::WidgetInfo::labeled(egui::WidgetType::Other, true, name);
            response.widget_info(info);
        }
        let laid = Laid { rect, shape };
        let widths = &mut state.widths;
        // Kept only while the lines hold what it selected.
        let mut selection = state.selection.take().and_then(|s| self.kept(s));
        if self.lines.count == 0 {
            return;
        }
        let (pressed, down, pointer, shift, time) = ui.input(|i| {
            let p = &i.pointer;
            (
                p.primary_pressed(),
                p.primary_down(),
                p.interact_pos(),
                i.modifiers.shift,
                i.time,
            )
        });
        if response.contains_pointer() || selection.as_ref().is_some_and(|s| s.dragging) {
            ui.set_cursor_icon(CursorIcon::Text);
        }
        // A press over the lines begins a selection there, or with Shift,
        // carries the one there is on to there; two or three presses in a
        // row select the word or the line there.
        if let Some(p) = pointer.filter(|_| pressed && response.contains_pointer()) {
            let presses = presses(ui, state.press, time, p);
            state.press = Some((time, p, presses));
            let was = selection.take().filter(|_| shift && presses == 1);
            selection = Some(self.press(ui, laid, widths, p, presses, was));
            response.request_focus();
        }
        if let Some(s) = selection.as_mut().filter(|s| s.dragging) {
            match pointer.filter(|_| down) {
                Some(p) => {
                    // Dragged away, the press is not a click to count on from.
                    let near = ui.ctx().options(|o| o.input_options.max_click_dist);
                    if state.press.is_some_and(|(_, at, _)| at.distance(p) > near) {
                        state.press = None;
                    }
                    self.drag(ui, laid, widths, s, p);
                    scroll_toward(ui, p, state.overflow, state.offset, state.most);
                }
                None => s.dragging = false,
            }
        }
        // The keyboard's Copy, Select All and Escape, and the right click's.
        let mut copy = false;
        let mut all = false;
        if response.has_focus() {
            // egui would take the focus from the box at Escape, which lets
            // go of the selection, and at the arrows, which have nothing to
            // move in it.
            let keys = EventFilter {
                escape: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                ..EventFilter::default()
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(response.id, keys));
            // Escape shuts the right click's menu first.
            let menu = response.context_menu_opened();
            let escape = ui.input_mut(|i| {
                copy |= i.events.iter().any(|e| matches!(e, egui::Event::Copy));
                all |= i.consume_key(Modifiers::COMMAND, Key::A);
                !menu && i.consume_key(Modifiers::NONE, Key::Escape)
            });
            if escape {
                // With nothing to let go, the box lets go of the keyboard.
                match selection.as_ref().is_some_and(|s| !s.is_empty()) {
                    true => selection = None,
                    false => response.surrender_focus(),
                }
            }
        }
        let selected = selection.as_ref().is_some_and(|s| !s.is_empty());
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
            let last = self.end() - 1;
            let end = At {
                line: last,
                char: self.chars(last),
            };
            let start = At {
                line: self.pane.first,
                char: 0,
            };
            let mut s = Selection::new(start, end, Unit::Char, (start, start));
            s.dragging = false;
            selection = Some(s);
            response.request_focus();
        }
        if let Some(s) = selection.as_ref().filter(|s| copy && !s.is_empty()) {
            ui.ctx().copy_text(self.text(s.range()));
        }
        // Known in the next frame by what it holds in this.
        if let Some(s) = selection.as_mut() {
            let (a, b) = s.range();
            s.ends = self.ends(a, b);
        }
        self.paint(ui, id, laid, selection.as_ref(), widths);
        state.selection = selection;
    }

    /// What a press at `p` selects, the `presses`th in a row: from one, the
    /// place there, or with Shift, `was` carried on to there; from two, the
    /// word there; from three, the line.
    fn press(
        &self,
        ui: &Ui,
        laid: Laid,
        widths: &mut HashMap<char, f32>,
        p: Pos2,
        presses: u8,
        was: Option<Selection>,
    ) -> Selection {
        let unit = match presses {
            1 => Unit::Char,
            2 => Unit::Word,
            _ => Unit::Line,
        };
        let (a, b) = match unit {
            Unit::Char => {
                let at = self.at(ui, laid, widths, p);
                (at, at)
            }
            unit => self.span(ui, laid, widths, p, unit),
        };
        let anchor = was.map_or(a, |s| s.anchor);
        Selection::new(anchor, b, unit, (a, b))
    }

    /// Carries `s` on to `p`, as the pointer drags it: to the place there,
    /// or the whole word or line there, from the far end of the one its
    /// press selected.
    fn drag(
        &self,
        ui: &Ui,
        laid: Laid,
        widths: &mut HashMap<char, f32>,
        s: &mut Selection,
        p: Pos2,
    ) {
        match s.unit {
            Unit::Char => s.head = self.at(ui, laid, widths, p),
            unit => {
                let (a, b) = self.span(ui, laid, widths, p, unit);
                let (first, last) = s.first;
                (s.anchor, s.head) = match a < first {
                    true => (last, a),
                    false => (first, b.max(last)),
                };
            }
        }
    }

    /// The place `p` points at, on the lines `laid` out: before the first
    /// line, its start; past the last, its end; on one, the place between
    /// its characters nearest `p`, as they are laid out.
    fn at(&self, ui: &Ui, laid: Laid, widths: &mut HashMap<char, f32>, p: Pos2) -> At {
        let line = match self.line_at(laid, p) {
            Ok(line) => line,
            Err(at) => return at,
        };
        let (text, colour) = self.get(line);
        let x = p.x - laid.rect.left();
        let piece = self.piece(ui, text, colour, (x, x), laid.shape, widths);
        At {
            line,
            char: piece.nearest(x),
        }
    }

    /// The line at `p`'s height; or where `p` is above or below them all,
    /// the start of the first or the end of the last.
    fn line_at(&self, laid: Laid, p: Pos2) -> Result<usize, At> {
        let first = self.pane.first;
        if p.y < laid.rect.top() {
            return Err(At {
                line: first,
                char: 0,
            });
        }
        let i = ((p.y - laid.rect.top()) / laid.shape.step).floor() as usize;
        if i >= self.lines.count {
            let last = self.end() - 1;
            let char = self.chars(last);
            return Err(At { line: last, char });
        }
        Ok(first + i)
    }

    /// The word or the line at `p`, `unit`, from its start to its end: a
    /// word, a run of spaces, or any other character alone; a line with
    /// its end, up to the next line. Above or below the lines, as a place:
    /// the first's start or the last's end, or for lines, the first or last.
    fn span(
        &self,
        ui: &Ui,
        laid: Laid,
        widths: &mut HashMap<char, f32>,
        p: Pos2,
        unit: Unit,
    ) -> (At, At) {
        let line = match self.line_at(laid, p) {
            Ok(line) => line,
            Err(at) if unit == Unit::Line => at.line,
            Err(at) => return (at, at),
        };
        let (text, colour) = self.get(line);
        let start = At { line, char: 0 };
        if unit == Unit::Line {
            let end = match line + 1 < self.end() {
                true => At {
                    line: line + 1,
                    char: 0,
                },
                false => At {
                    line,
                    char: count(text),
                },
            };
            return (start, end);
        }
        let x = p.x - laid.rect.left();
        let piece = self.piece(ui, text, colour, (x, x), laid.shape, widths);
        match piece.under(x) {
            Some(k) => {
                let (from, to) = word(text, k);
                (At { line, char: from }, At { line, char: to })
            }
            None => (start, start),
        }
    }

    /// How many characters line `n` has.
    fn chars(&self, n: usize) -> usize {
        count(self.get(n).0)
    }

    /// `s` where the lines still hold what it selected: none of it on lines
    /// dropped from the start, within them, and with the text at its ends
    /// as it was.
    fn kept(&self, mut s: Selection) -> Option<Selection> {
        let last = self.pane.first + self.lines.count.checked_sub(1)?;
        let places = [s.anchor, s.head, s.first.0, s.first.1];
        if places.iter().any(|at| at.line < self.pane.first) {
            return None;
        }
        let within = |at: At| {
            let line = at.line.min(last);
            let char = at.char.min(self.chars(line));
            At { line, char }
        };
        s.anchor = within(s.anchor);
        s.head = within(s.head);
        s.first = (within(s.first.0), within(s.first.1));
        let (a, b) = s.range();
        (self.ends(a, b) == s.ends).then_some(s)
    }

    /// What a selection from `a` to `b` is known by: the first MARK
    /// characters it holds on its first line, and the MARK before its end
    /// on its last; a line gw is still printing grows only past both.
    fn ends(&self, a: At, b: At) -> (String, String) {
        let to = if a.line == b.line { b.char } else { usize::MAX };
        let first = self.get(a.line).0.chars().skip(a.char);
        let start = first.take(to.saturating_sub(a.char).min(MARK)).collect();
        let from = b.char.saturating_sub(MARK);
        let last = self.get(b.line).0.chars().skip(from);
        (start, last.take(b.char - from).collect())
    }

    /// The text from `a` to `b`, a line to a line.
    fn text(&self, (a, b): (At, At)) -> String {
        let lines: Vec<String> = (a.line..=b.line)
            .map(|n| {
                let text = self.get(n).0;
                let from = if n == a.line { a.char } else { 0 };
                let to = if n == b.line { b.char } else { usize::MAX };
                text.chars()
                    .skip(from)
                    .take(to.saturating_sub(from))
                    .collect()
            })
            .collect();
        lines.join("\n")
    }

    /// Line `text` laid out in `colour` as far as needed to show it from
    /// `left` to `right`, in points from its start: all of it, up to LONG
    /// characters; of a longer line, the pieces of PIECE characters that
    /// reach across that, after room as wide as the characters before them,
    /// so that each lies where it would in the whole line.
    fn piece(
        &self,
        ui: &Ui,
        text: &str,
        colour: Color32,
        (left, right): (f32, f32),
        shape: Shape,
        widths: &mut HashMap<char, f32>,
    ) -> Piece {
        let chars = count(text);
        // The characters laid out, and their bytes; the room before them.
        let (from, to, lead) = if chars <= LONG {
            ((0, 0), (chars, text.len()), 0.0)
        } else if printable(text) {
            let at = |x: f32| ((x / shape.advance).max(0.0) as usize).min(chars);
            let from = at(left) / PIECE * PIECE;
            let to = ((at(right) / PIECE + 1) * PIECE).min(chars);
            ((from, from), (to, to), from as f32 * shape.advance)
        } else {
            let (mut from, mut to, mut x) = ((0, 0, 0.0), (chars, text.len()), 0.0);
            for (k, (byte, c)) in text.char_indices().enumerate() {
                if k.is_multiple_of(PIECE) {
                    if x <= left {
                        from = (k, byte, x);
                    } else if x > right {
                        to = (k, byte);
                        break;
                    }
                }
                x += self.char_width(ui, c, shape, widths);
            }
            ((from.0, from.1), to, from.2)
        };
        let laid = text[from.1..to.1].to_owned();
        let mut job = LayoutJob::simple(laid, self.lines.font.clone(), colour, f32::INFINITY);
        job.sections[0].leading_space = lead;
        Piece {
            from: from.0,
            len: to.0 - from.0,
            galley: ui.fonts_mut(|f| f.layout_job(job)),
        }
    }

    /// Draws the lines that show, as far across as they show, the selection
    /// behind them; and names each, as a label, to screen readers.
    fn paint(
        &self,
        ui: &Ui,
        id: egui::Id,
        laid: Laid,
        selection: Option<&Selection>,
        widths: &mut HashMap<char, f32>,
    ) {
        let Laid { rect, shape } = laid;
        let shown = ui.clip_rect();
        let start = ((shown.top() - rect.top()) / shape.step).floor().max(0.0) as usize;
        let past = ((shown.bottom() - rect.top()) / shape.step).ceil().max(0.0) as usize + 1;
        let across = (shown.left() - rect.left(), shown.right() - rect.left());
        let range = selection.filter(|s| !s.is_empty()).map(Selection::range);
        let ppp = ui.ctx().pixels_per_point();
        let painter = ui.painter();
        let marking = ui.visuals().selection;
        // On a solid fill, as the classic themes' is, a line's own colour may
        // not read: what is selected is drawn again in the selection's text
        // colour, as egui draws a selection. A tint, as the Log's is, keeps
        // the colours gw's lines are told apart by.
        let recolour = marking.bg_fill.is_opaque();
        for i in start..past.min(self.lines.count) {
            let n = self.pane.first + i;
            let at = pos2(rect.left(), rect.top() + i as f32 * shape.step);
            let (text, colour) = self.get(n);
            let piece = self.piece(ui, text, colour, across, shape, widths);
            let marked = range.filter(|(a, b)| (a.line..=b.line).contains(&n));
            let marked = marked.map(|(a, b)| {
                let from = if n == a.line { piece.x(a.char) } else { 0.0 };
                // A line selected to its end shows its end selected too, and
                // the room down to the next line.
                let (to, bottom) = match n == b.line {
                    true => (piece.x(b.char), shape.row),
                    false => (piece.end() + shape.advance, shape.step),
                };
                let min = at + vec2(from, 0.0);
                let max = at + vec2(to, bottom);
                Rect::from_min_max(min, max).round_to_pixels(ppp)
            });
            if let Some(marked) = marked {
                painter.rect_filled(marked, 0.0, marking.bg_fill);
            }
            let galley = piece.galley;
            let line = Rect::from_min_size(at, vec2(rect.width(), shape.row));
            ui.interact(line, id.with(("line", n)), Sense::hover())
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, galley.text())
                });
            painter.galley(at, galley.clone(), colour);
            if let Some(marked) = marked.filter(|_| recolour) {
                let ink = marking.stroke.color;
                let over = painter.with_clip_rect(marked);
                over.galley_with_override_text_color(at, galley, ink);
            }
        }
    }
}

/// Where a box's lines lie, and their measures.
#[derive(Clone, Copy)]
struct Laid {
    rect: Rect,
    shape: Shape,
}

/// A box's lines' measures: a line's height, from one line to the next, and
/// a printable ASCII character's width.
#[derive(Clone, Copy)]
struct Shape {
    row: f32,
    step: f32,
    advance: f32,
}

/// Part of a line, laid out: its characters from `from`, `len` of them,
/// where they lie in the whole line.
struct Piece {
    from: usize,
    len: usize,
    galley: Arc<Galley>,
}

impl Piece {
    /// Where the line's character `char` begins, in points from the line's
    /// start, as laid out: one before or past the part, at its start or end.
    fn x(&self, char: usize) -> f32 {
        let at = char.clamp(self.from, self.from + self.len) - self.from;
        self.galley.pos_from_cursor(CCursor::new(at)).min.x
    }

    /// Where the part ends.
    fn end(&self) -> f32 {
        self.x(self.from + self.len)
    }

    /// The place between characters nearest `x`.
    fn nearest(&self, x: f32) -> usize {
        let at = self.galley.cursor_from_pos(vec2(x, 0.0)).index.0;
        self.from + at.min(self.len)
    }

    /// The character at `x`, if there are any: before them all, the first,
    /// and past them, the last.
    fn under(&self, x: f32) -> Option<usize> {
        let last = self.len.checked_sub(1)?;
        let row = self.galley.rows.first()?;
        let x = x - row.pos.x;
        let at = row
            .glyphs
            .iter()
            .position(|g| x < g.pos.x + g.advance_width);
        Some(self.from + at.unwrap_or(last).min(last))
    }
}

/// Whether `text` is all printable ASCII, each character as wide as the
/// next in a fixed-width font.
fn printable(text: &str) -> bool {
    text.bytes().all(|b| (b' '..=b'~').contains(&b))
}

/// How many characters `text` has.
fn count(text: &str) -> usize {
    match text.is_ascii() {
        true => text.len(),
        false => text.chars().count(),
    }
}

/// What a double click selects a run of: a word's characters, or spaces;
/// any other character it selects alone.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Other,
}

fn class(c: char) -> Class {
    if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Other
    }
}

/// Whether `c` is a combining mark, which belongs with the character
/// before it: as in a name macOS keeps, whose é is an e and its accent.
fn combining(c: char) -> bool {
    matches!(
        c,
        '\u{300}'..='\u{36F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

/// Where the word, the run of spaces or the other character that `text`'s
/// character `k` is in begins and ends, in characters, each with the
/// combining marks after it. Found going out from `k`, keeping none of the
/// line: a line may run to millions of characters.
fn word(text: &str, k: usize) -> (usize, usize) {
    let byte = match text.is_ascii() {
        true => (k < text.len()).then_some(k),
        false => text.char_indices().nth(k).map(|(b, _)| b),
    };
    let Some(byte) = byte else {
        return (k, k);
    };
    let (before, after) = text.split_at(byte);
    let (mut back, mut ahead) = (before.chars().rev(), after.chars());
    // A mark goes by the character it follows, back past any marks between;
    // those the text begins with, by the first of them.
    let first = || text.chars().next().map_or(Class::Other, class);
    let c = ahead.next().unwrap_or_default();
    let (base, of) = match combining(c) {
        false => (k, class(c)),
        true => {
            let found = back.by_ref().enumerate().find(|&(_, b)| !combining(b));
            found.map_or_else(|| (0, first()), |(j, b)| (k - 1 - j, class(b)))
        }
    };
    if of == Class::Other {
        let marks = ahead.take_while(|&c| combining(c)).count();
        return (base, k + 1 + marks);
    }
    // On and back over each character of its class, with the marks after it.
    let on = ahead.take_while(|&c| combining(c) || class(c) == of);
    let to = k + 1 + on.count();
    let (mut from, mut at) = (base, base);
    let mut other = false;
    for b in back {
        at -= 1;
        if combining(b) {
            continue;
        }
        if class(b) != of {
            other = true;
            break;
        }
        from = at;
    }
    if !other && from > 0 && first() == of {
        from = 0;
    }
    (from, to)
}

/// How many presses in a row a press at `p` at `time` makes, the last
/// having been `last`: one more, up to three, where it comes as soon and as
/// near as a double click's second.
fn presses(ui: &Ui, last: Option<(f64, Pos2, u8)>, time: f64, p: Pos2) -> u8 {
    let options = ui.ctx().options(|o| o.input_options);
    match last {
        Some((then, at, n))
            if time - then < options.max_double_click_delay
                && at.distance(p) < options.max_click_dist =>
        {
            (n + 1).min(3)
        }
        _ => 1,
    }
}

/// Scrolls the box toward `p` while it lies past the box's edge, the faster
/// the further past, so a dragged selection carries on. Only the ways the
/// lines overflow, `overflow`: a scroll the box cannot take goes to the area
/// around it. Nor toward an end the area is at, its `offset` 0 or `most`:
/// egui would draw frame after frame for a scroll that goes nowhere.
fn scroll_toward(ui: &Ui, p: Pos2, overflow: Vec2b, offset: Vec2, most: Vec2) {
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
    ) * overflow.to_vec2();
    let open = |d: usize| match speed[d] {
        s if s < 0.0 && offset[d] > 0.0 => s,
        s if s > 0.0 && offset[d] < most[d] => s,
        _ => 0.0,
    };
    let speed = vec2(open(0), open(1));
    if speed != Vec2::ZERO {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let none = egui::style::ScrollAnimation::none();
        ui.scroll_with_delta_animation(-speed * dt, none);
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{self, Choice};
    use egui::{Event, FullOutput, PointerButton, RawInput, ScrollArea};
    use std::cell::Cell;

    /// The box's id in these tests.
    fn id() -> egui::Id {
        egui::Id::new("box")
    }

    /// The font these tests' lines are in.
    fn font() -> FontId {
        FontId::monospace(12.0)
    }

    /// A window 800 by 600 points showing `lines` in a box at most 360 tall,
    /// `area`, or none with `lines` None; each frame `events` at `time`.
    fn frame(
        ctx: &egui::Context,
        lines: Option<&[String]>,
        area: impl Fn() -> ScrollArea,
        events: Vec<Event>,
        time: f64,
    ) -> FullOutput {
        let input = RawInput {
            time: Some(time),
            events,
            ..Default::default()
        };
        run(ctx, lines.map(|l| (l, Pane::default())), area, input)
    }

    /// As `frame` shows them, `lines` in the box `pane` says, from `input`.
    fn run(
        ctx: &egui::Context,
        lines: Option<(&[String], Pane)>,
        area: impl Fn() -> ScrollArea,
        input: RawInput,
    ) -> FullOutput {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            ..input
        };
        let mut out = ctx.run_ui(input, |ui| {
            let Some((lines, pane)) = lines else { return };
            let line = |i: usize| (lines[i].as_str(), Color32::WHITE);
            let shown = Lines {
                count: lines.len(),
                line: &line,
                font: font(),
                gap: 0.0,
            };
            shown.show_as(ui, id(), area().max_height(360.0), pane);
        });
        // As a renderer takes them.
        out.textures_delta.clear();
        out
    }

    fn both() -> ScrollArea {
        ScrollArea::both()
    }

    /// A window's frames, a twentieth of a second apart, its box's first
    /// line the `first`th it has held.
    struct Window {
        ctx: egui::Context,
        time: f64,
        first: usize,
    }

    impl Window {
        fn new() -> Window {
            Window {
                ctx: egui::Context::default(),
                time: 0.0,
                first: 0,
            }
        }

        fn pane(&self) -> Pane<'static> {
            Pane {
                first: self.first,
                ..Pane::default()
            }
        }

        fn show(&mut self, lines: &[String], events: Vec<Event>) -> FullOutput {
            self.time += 0.05;
            let input = RawInput {
                time: Some(self.time),
                events,
                ..Default::default()
            };
            run(&self.ctx, Some((lines, self.pane())), both, input)
        }

        /// The selection the box keeps, as it would copy it.
        fn selected(&self, lines: &[String]) -> Option<String> {
            let state: State = self.ctx.data(|d| d.get_temp(id()))?;
            let s = state.selection.filter(|s| !s.is_empty())?;
            let line = |i: usize| (lines[i].as_str(), Color32::WHITE);
            let shown = Lines {
                count: lines.len(),
                line: &line,
                font: font(),
                gap: 0.0,
            };
            let held = Held {
                lines: &shown,
                pane: self.pane(),
            };
            Some(held.text(s.range()))
        }
    }

    fn press(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// What a frame put on the clipboard.
    fn copied(out: &FullOutput) -> Option<String> {
        out.platform_output.commands.iter().find_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
    }

    /// The lines a frame drew in their own colours: where each is, as laid
    /// out.
    fn drawn(out: &FullOutput) -> Vec<(Pos2, Arc<Galley>)> {
        let texts = out.shapes.iter().filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.override_text_color.is_none() => {
                Some((t.pos, t.galley.clone()))
            }
            _ => None,
        });
        texts.collect()
    }

    /// Where the line drawn at `(pos, galley)` has its character `k`, in its
    /// middle's height.
    fn on(drawn: &(Pos2, Arc<Galley>), k: usize) -> Pos2 {
        let (pos, galley) = drawn;
        let x = galley.pos_from_cursor(CCursor::new(k)).min.x;
        pos2(pos.x + x, pos.y + galley.size().y / 2.0)
    }

    /// Presses and lets go at `at` `n` times in a row, then holds it there.
    fn click(w: &mut Window, lines: &[String], at: Pos2, n: usize) {
        w.show(lines, vec![Event::PointerMoved(at)]);
        for _ in 0..n {
            w.show(lines, vec![press(at, true)]);
            w.show(lines, vec![press(at, false)]);
        }
    }

    fn rows(n: usize, bytes: &str) -> Vec<String> {
        (0..n).map(|i| format!("{:04X}  {bytes}", i * 16)).collect()
    }

    /// The fills a frame drew in `fill`, top first.
    fn fills(out: &FullOutput, fill: Color32) -> Vec<Rect> {
        let mut rects: Vec<Rect> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == fill => Some(r.rect),
                _ => None,
            })
            .collect();
        rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
        rects
    }

    #[test]
    fn a_selection_is_let_go_once_the_box_holds_other_lines() {
        // Select All in a 1,024-byte sector's 64 rows; then the same box,
        // as a sector window does, shows a 256-byte sector's 16.
        let big = rows(64, "00 11 22 33");
        let small = rows(16, "AA BB CC DD");
        let mut w = Window::new();
        let out = w.show(&big, vec![]);
        click(&mut w, &big, on(&drawn(&out)[0], 2), 1);
        w.show(&big, vec![key(Key::A, Modifiers::COMMAND)]);
        assert_eq!(w.selected(&big).map(|s| s.lines().count()), Some(64));
        // Copy once read the 64 rows, now gone, and panicked.
        let out = w.show(&small, vec![Event::Copy]);
        assert_eq!(copied(&out), None, "nothing selected");
        assert_eq!(w.selected(&small), None);

        // Nor is one kept by a box that went away: a window shut, an emptied
        // log, another page.
        let mut w = Window::new();
        let out = w.show(&big, vec![]);
        click(&mut w, &big, on(&drawn(&out)[0], 2), 1);
        w.show(&big, vec![key(Key::A, Modifiers::COMMAND)]);
        frame(&w.ctx, None, both, vec![], w.time + 0.05);
        w.time += 0.05;
        w.show(&big, vec![]);
        assert_eq!(w.selected(&big), None);

        // A box whose last lines are gone keeps none, nor copies them.
        let mut w = Window::new();
        w.show(&big, vec![]);
        let out = w.show(&big, vec![]);
        click(&mut w, &big, on(&drawn(&out)[0], 2), 1);
        w.show(&big, vec![key(Key::A, Modifiers::COMMAND)]);
        let out = w.show(&big[..16], vec![Event::Copy]);
        assert_eq!(copied(&out), None);
        assert_eq!(w.selected(&big[..16]), None);
    }

    #[test]
    fn a_selection_is_kept_as_lines_come_after_it_and_the_last_grows() {
        let mut lines = rows(16, "00 11 22 33");
        let mut w = Window::new();
        let out = w.show(&lines, vec![]);
        click(&mut w, &lines, on(&drawn(&out)[0], 2), 1);
        w.show(&lines, vec![key(Key::A, Modifiers::COMMAND)]);
        let all = w.selected(&lines).unwrap();
        // gw goes on printing its last line, then more.
        lines[15].push_str(" 44 55");
        lines.extend(rows(4, "66 77"));
        let out = w.show(&lines, vec![Event::Copy]);
        assert_eq!(copied(&out), Some(all));
    }

    #[test]
    fn a_log_dropping_its_oldest_lines_keeps_a_selection_on_the_lines_it_was_made_on() {
        // A log's lines from its `first`, numbered as the session's are.
        let log = |first: usize| -> Vec<String> {
            (first..first + 20)
                .map(|n| format!("T{n}.0: Read track"))
                .collect()
        };
        let mut w = Window::new();
        let lines = log(0);
        let out = w.show(&lines, vec![]);
        let drawn = drawn(&out);
        // From line 5's "5" to line 7's "T7.".
        let (from, to) = (on(&drawn[5], 1), on(&drawn[7], 3));
        w.show(&lines, vec![Event::PointerMoved(from)]);
        w.show(&lines, vec![press(from, true)]);
        w.show(&lines, vec![Event::PointerMoved(to)]);
        w.show(&lines, vec![press(to, false)]);
        let made = "5.0: Read track\nT6.0: Read track\nT7.";
        assert_eq!(w.selected(&lines).as_deref(), Some(made));
        // Full, the log drops its first three lines as three more come.
        w.first = 3;
        let lines = log(3);
        let out = w.show(&lines, vec![Event::Copy]);
        assert_eq!(copied(&out).as_deref(), Some(made));
        // Dragged on while it drops two more: from where it was begun, to
        // the line now under the pointer.
        let (from, to) = (on(&drawn[2], 1), on(&drawn[4], 3));
        w.time += 1.0;
        w.show(&lines, vec![Event::PointerMoved(from)]);
        w.show(&lines, vec![press(from, true)]);
        w.show(&lines, vec![Event::PointerMoved(to)]);
        assert_eq!(w.selected(&lines).as_deref(), Some(made));
        w.first = 5;
        let lines = log(5);
        w.show(&lines, vec![]);
        let on_to = "5.0: Read track\nT6.0: Read track\nT7.0: Read track\nT8.0: Read track\nT9.";
        assert_eq!(w.selected(&lines).as_deref(), Some(on_to));
        w.show(&lines, vec![press(to, false)]);
        assert_eq!(w.selected(&lines).as_deref(), Some(on_to));
        // Its first line dropped too, the selection goes with it.
        w.first = 6;
        let lines = log(6);
        let out = w.show(&lines, vec![Event::Copy]);
        assert_eq!(copied(&out), None);
        assert_eq!(w.selected(&lines), None);
    }

    #[test]
    fn escape_lets_go_of_the_selection_and_the_arrows_leave_the_box_its_keys() {
        let lines = rows(8, "00 11 22 33");
        let mut w = Window::new();
        let out = w.show(&lines, vec![]);
        click(&mut w, &lines, on(&drawn(&out)[0], 2), 1);
        w.show(&lines, vec![key(Key::A, Modifiers::COMMAND)]);
        w.show(&lines, vec![key(Key::ArrowDown, Modifiers::NONE)]);
        let out = w.show(&lines, vec![Event::Copy]);
        assert!(copied(&out).is_some(), "the box kept the keyboard");
        w.show(&lines, vec![key(Key::Escape, Modifiers::NONE)]);
        assert_eq!(w.selected(&lines), None);
        let out = w.show(&lines, vec![key(Key::A, Modifiers::COMMAND), Event::Copy]);
        assert!(copied(&out).is_some(), "the box still has the keyboard");
        // With nothing to let go, Escape lets go of the keyboard.
        w.show(&lines, vec![key(Key::Escape, Modifiers::NONE)]);
        w.show(&lines, vec![key(Key::Escape, Modifiers::NONE)]);
        assert_eq!(w.ctx.memory(|m| m.focused()), None);
    }

    #[test]
    fn a_shift_press_carries_the_selection_on_to_it() {
        let lines = rows(8, "00 11 22 33");
        let mut w = Window::new();
        let out = w.show(&lines, vec![]);
        let drawn = drawn(&out);
        click(&mut w, &lines, on(&drawn[1], 6), 1);
        assert_eq!(w.selected(&lines), None, "a place, nothing selected");
        // On to line 3's ninth character, then back before where it began.
        w.show(&lines, vec![Event::ModifiersChanged(Modifiers::SHIFT)]);
        for (to, said) in [
            (on(&drawn[3], 8), "00 11 22 33\n0020  00 11 22 33\n0030  00"),
            (on(&drawn[0], 2), "00  00 11 22 33\n0010  "),
        ] {
            w.time += 1.0;
            click(&mut w, &lines, to, 1);
            assert_eq!(w.selected(&lines).as_deref(), Some(said));
        }
        // Without Shift, a press begins another.
        w.show(&lines, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        w.time += 1.0;
        click(&mut w, &lines, on(&drawn[5], 3), 1);
        assert_eq!(w.selected(&lines), None);
    }

    #[test]
    fn two_presses_select_a_word_and_three_a_line_and_a_drag_goes_on_by_them() {
        let lines: Vec<String> = ["Write Bandwidth:  7.66", "Read Bandwidth:  8.15"]
            .map(String::from)
            .into();
        let mut w = Window::new();
        let out = w.show(&lines, vec![]);
        let drawn = drawn(&out);
        // In "Bandwidth"'s t, character 13, past its middle.
        let d = on(&drawn[0], 13) + vec2(4.0, 0.0);
        click(&mut w, &lines, d, 2);
        assert_eq!(w.selected(&lines).as_deref(), Some("Bandwidth"));
        // A run of spaces, and a character on its own.
        click(&mut w, &lines, on(&drawn[0], 16) + vec2(8.0, 0.0), 2);
        assert_eq!(w.selected(&lines).as_deref(), Some("  "));
        w.time += 1.0;
        click(&mut w, &lines, on(&drawn[0], 15) + vec2(2.0, 0.0), 2);
        assert_eq!(w.selected(&lines).as_deref(), Some(":"));
        w.time += 1.0;
        // Three presses: the line, with its end.
        click(&mut w, &lines, d, 3);
        assert_eq!(
            w.selected(&lines).as_deref(),
            Some("Write Bandwidth:  7.66\n")
        );
        // Held after two, the selection goes on a word at a time: back to
        // "Write", on to "Read".
        w.time += 1.0;
        let write = on(&drawn[0], 1);
        w.show(&lines, vec![Event::PointerMoved(d)]);
        w.show(&lines, vec![press(d, true)]);
        w.show(&lines, vec![press(d, false)]);
        w.show(&lines, vec![press(d, true)]);
        w.show(&lines, vec![Event::PointerMoved(write)]);
        assert_eq!(w.selected(&lines).as_deref(), Some("Write Bandwidth"));
        w.show(&lines, vec![Event::PointerMoved(on(&drawn[1], 2))]);
        w.show(&lines, vec![press(on(&drawn[1], 2), false)]);
        assert_eq!(
            w.selected(&lines).as_deref(),
            Some("Bandwidth:  7.66\nRead")
        );
    }

    #[test]
    fn held_after_three_presses_a_drag_goes_on_a_line_at_a_time() {
        let lines: Vec<String> = ["one", "two", "three", "four"].map(String::from).into();
        let mut w = Window::new();
        let out = w.show(&lines, vec![]);
        let drawn = drawn(&out);
        let two = on(&drawn[1], 1);
        w.show(&lines, vec![Event::PointerMoved(two)]);
        for _ in 0..2 {
            w.show(&lines, vec![press(two, true)]);
            w.show(&lines, vec![press(two, false)]);
        }
        w.show(&lines, vec![press(two, true)]);
        assert_eq!(w.selected(&lines).as_deref(), Some("two\n"));
        // Down to the last line, whole; then up to the first, with the line
        // pressed to its end.
        for (to, said) in [(3, "two\nthree\nfour"), (0, "one\ntwo\n")] {
            w.show(&lines, vec![Event::PointerMoved(on(&drawn[to], 2))]);
            assert_eq!(w.selected(&lines).as_deref(), Some(said));
        }
        w.show(&lines, vec![press(on(&drawn[0], 2), false)]);
        assert_eq!(w.selected(&lines).as_deref(), Some("one\ntwo\n"));
    }

    #[test]
    fn a_box_that_fills_its_area_takes_a_press_past_its_lines() {
        let lines: Vec<String> = ["one", "two", "three"].map(String::from).into();
        for fills in [true, false] {
            let ctx = egui::Context::default();
            let pane = Pane {
                fills,
                ..Pane::default()
            };
            let mut time = 0.0;
            let mut show = |events: Vec<Event>| {
                time += 0.05;
                let input = RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                };
                run(&ctx, Some((&lines, pane)), both, input)
            };
            let out = show(vec![]);
            let drawn = drawn(&out);
            // Below the last line and right of the widest, in the box's room.
            let past = pos2(400.0, 300.0);
            show(vec![Event::PointerMoved(past)]);
            show(vec![press(past, true)]);
            show(vec![Event::PointerMoved(on(&drawn[0], 1))]);
            show(vec![press(on(&drawn[0], 1), false)]);
            let out = show(vec![Event::Copy]);
            match fills {
                true => assert_eq!(copied(&out).as_deref(), Some("ne\ntwo\nthree")),
                // A box no bigger than its lines has no room past them.
                false => {
                    assert_eq!(copied(&out), None);
                    assert_eq!(ctx.memory(|m| m.focused()), None);
                }
            }
        }
    }

    #[test]
    fn a_line_is_selected_where_its_glyphs_lie_whatever_their_widths() {
        // A file name as macOS keeps it, its é an e and a mark that takes
        // no room, and a tab four spaces wide, before the text pressed on.
        let lines: Vec<String> =
            vec!["gw read Disque n\u{b0}1 e\u{301}te\u{301}.adf\t-> copy".into()];
        let mut w = Window::new();
        let out = w.show(&lines, vec![]);
        let line = &drawn(&out)[0];
        let glyphs = &line.1.rows[0].glyphs;
        let advance = w.ctx.fonts_mut(|f| f.glyph_width(&font(), '0'));
        let c = lines[0].chars().position(|c| c == 'c').unwrap();
        assert!(
            (glyphs[c].pos.x - c as f32 * advance).abs() > advance / 2.0,
            "glyphs other than the font's width"
        );
        // From "copy"'s o to the end.
        let o = on(line, c + 1);
        w.show(&lines, vec![Event::PointerMoved(o)]);
        w.show(&lines, vec![press(o, true)]);
        let end = on(line, c + 4) + vec2(40.0, 0.0);
        w.show(&lines, vec![Event::PointerMoved(end)]);
        let out = w.show(&lines, vec![press(end, false), Event::Copy]);
        assert_eq!(copied(&out).as_deref(), Some("opy"));
        // Marked from the o, as its glyph lies, to the pixel.
        let fill = egui::Visuals::dark().selection.bg_fill;
        let marked = fills(&out, fill);
        assert!(
            marked.len() == 1 && (marked[0].left() - o.x).abs() <= 0.5,
            "{marked:?} from {o:?}"
        );
        // Two presses on the name's t select its word, marks and all.
        w.time += 1.0;
        let t = lines[0].chars().position(|c| c == 't').unwrap();
        click(&mut w, &lines, on(line, t) + vec2(2.0, 0.0), 2);
        assert_eq!(w.selected(&lines).as_deref(), Some("e\u{301}te\u{301}"));
    }

    #[test]
    fn a_selection_down_lines_is_filled_through_the_room_between_them() {
        // Lines 3 points apart, as the Log's are farther.
        let lines = rows(6, "00 11 22 33");
        let ctx = egui::Context::default();
        let show = |events: Vec<Event>, time: f64| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let line = |i: usize| (lines[i].as_str(), Color32::WHITE);
                let shown = Lines {
                    count: lines.len(),
                    line: &line,
                    font: font(),
                    gap: 3.0,
                };
                shown.show_as(ui, id(), ScrollArea::both(), Pane::default());
            });
            out.textures_delta.clear();
            out
        };
        let out = show(vec![], 0.0);
        let at = on(&drawn(&out)[1], 6);
        show(vec![Event::PointerMoved(at)], 0.05);
        show(vec![press(at, true)], 0.1);
        let to = on(&drawn(&out)[4], 9);
        show(vec![Event::PointerMoved(to)], 0.15);
        let out = show(vec![press(to, false)], 0.2);
        let fill = egui::Visuals::dark().selection.bg_fill;
        let marked = fills(&out, fill);
        assert_eq!(marked.len(), 4, "{marked:?}");
        for pair in marked.windows(2) {
            assert_eq!(pair[0].bottom(), pair[1].top(), "{marked:?}");
        }
        // The last only as tall as its line.
        let row = ctx.fonts_mut(|f| f.row_height(&font()));
        assert!((marked[3].height() - row).abs() <= 1.0, "{marked:?}");
    }

    #[test]
    fn on_a_solid_fill_a_selection_is_drawn_in_its_text_colour_and_the_log_keeps_its_own() {
        let lines = rows(4, "00 11 22 33");
        let named = theme::CHOICES.map(|c| c.0);
        for choice in named.into_iter().filter(|&c| c != Choice::System) {
            // A sector window's bytes, and gw's output in the Log's box.
            for log in [false, true] {
                let ctx = egui::Context::default();
                theme::install(&ctx);
                theme::apply(&ctx, choice);
                let show = |events: Vec<Event>, time: f64| {
                    let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
                    let input = RawInput {
                        screen_rect: Some(screen),
                        time: Some(time),
                        events,
                        ..Default::default()
                    };
                    let mut out = ctx.run_ui(input, |ui| {
                        let line = |i: usize| (lines[i].as_str(), Color32::WHITE);
                        let shown = Lines {
                            count: lines.len(),
                            line: &line,
                            font: font(),
                            gap: 0.0,
                        };
                        let area = ScrollArea::both().max_height(360.0);
                        match log {
                            true => theme::terminal(ui, |ui, _| {
                                shown.show_as(ui, id(), area, Pane::default())
                            }),
                            false => shown.show_as(ui, id(), area, Pane::default()),
                        }
                    });
                    out.textures_delta.clear();
                    out
                };
                let out = show(vec![], 0.0);
                let at = on(&drawn(&out)[0], 2);
                show(vec![Event::PointerMoved(at)], 0.05);
                show(vec![press(at, true)], 0.1);
                show(vec![press(at, false)], 0.15);
                let out = show(vec![key(Key::A, Modifiers::COMMAND)], 0.2);
                let over: Vec<(Rect, Option<Color32>)> = out
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.override_text_color.is_some() => {
                            Some((s.clip_rect, t.override_text_color))
                        }
                        _ => None,
                    })
                    .collect();
                let selection = ctx.global_style().visuals.selection;
                let classic = matches!(choice, Choice::Classic | Choice::Blue | Choice::Vintage);
                let solid = classic && !log;
                let at = format!("{choice:?}, in the Log's box {log}");
                assert_eq!(over.len(), if solid { 4 } else { 0 }, "{at}");
                // Each line's selected part, where the fill lies.
                let marked = fills(&out, selection.bg_fill);
                for (clip, ink) in &over {
                    assert_eq!(*ink, Some(selection.stroke.color), "{at}");
                    assert!(marked.contains(clip), "{at}: {clip:?} in {marked:?}");
                }
            }
        }
    }

    #[test]
    fn a_long_line_is_laid_out_only_where_it_shows() {
        let long = "0123456789".repeat(200_000);
        let lines = vec![long.clone(), "Done.".to_owned()];
        let ctx = egui::Context::default();
        frame(&ctx, None, both, vec![], 0.0);
        let advance = ctx.fonts_mut(|f| f.glyph_width(&font(), '0'));
        // Scrolled to its middle.
        let middle = 1_000_000.0 * advance;
        let area = move || ScrollArea::both().horizontal_scroll_offset(middle);
        let mut out = frame(&ctx, Some(&lines), area, vec![], 0.0);
        for time in 1..3 {
            out = frame(&ctx, Some(&lines), area, vec![], time as f64 / 20.0);
        }
        let drawn = drawn(&out);
        let (pos, galley) = &drawn[0];
        let laid = galley.text();
        assert!(
            laid.len() <= 2 * PIECE,
            "{} characters laid out",
            laid.len()
        );
        // Those that show, where they lie in the whole line.
        let first = galley.rows[0].glyphs[0].pos.x;
        let from = (first / advance).round() as usize;
        assert!(from.is_multiple_of(PIECE));
        assert_eq!(laid, &long[from..from + laid.len()]);
        let shown = ctx.content_rect();
        assert!(pos.x + first <= shown.left());
        assert!(pos.x + galley.size().x >= shown.right());
        let shapes = std::mem::take(&mut out.shapes);
        let meshes = ctx.tessellate(shapes, out.pixels_per_point);
        let vertices: usize = meshes
            .iter()
            .map(|m| match &m.primitive {
                egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                egui::epaint::Primitive::Callback(_) => 0,
            })
            .sum();
        assert!(vertices < 20_000, "{vertices} vertices");
        // All of it is selected and copied, though it shows in part.
        let at = pos2(shown.center().x, pos.y + 4.0);
        frame(&ctx, Some(&lines), area, vec![Event::PointerMoved(at)], 1.0);
        frame(&ctx, Some(&lines), area, vec![press(at, true)], 1.05);
        frame(&ctx, Some(&lines), area, vec![press(at, false)], 1.1);
        let keys = vec![key(Key::A, Modifiers::COMMAND)];
        frame(&ctx, Some(&lines), area, keys, 1.15);
        let out = frame(&ctx, Some(&lines), area, vec![Event::Copy], 1.2);
        assert_eq!(copied(&out), Some(format!("{long}\nDone.")));
    }

    #[test]
    fn a_long_lines_pieces_lie_where_the_whole_line_lays_its_glyphs() {
        let mut long: String = "日本語 🙂 é ".repeat(40);
        long.push_str(&"abcdefghij".repeat(300));
        let lines = vec![long.clone()];
        let ctx = egui::Context::default();
        frame(&ctx, None, both, vec![], 0.0);
        let whole = ctx.fonts_mut(|f| f.layout_no_wrap(long.clone(), font(), Color32::WHITE));
        let pieces = |offset: f32| {
            let area = move || ScrollArea::both().horizontal_scroll_offset(offset);
            frame(&ctx, Some(&lines), area, vec![], 0.0);
            frame(&ctx, Some(&lines), area, vec![], 0.05);
            let out = frame(&ctx, Some(&lines), area, vec![], 0.1);
            drawn(&out).remove(0).1
        };
        for offset in [0.0, 5_000.0, 20_000.0] {
            let galley = pieces(offset);
            let chars: Vec<char> = long.chars().collect();
            let glyphs = &galley.rows[0].glyphs;
            // Only the pieces that show are laid out.
            assert!(glyphs.len() <= 2 * PIECE, "{offset}: {}", glyphs.len());
            // The piece's first character, as the whole line has it.
            let from = (0..chars.len())
                .find(|&k| {
                    (whole.pos_from_cursor(CCursor::new(k)).min.x - glyphs[0].pos.x).abs() < 1.0
                })
                .expect("where the piece begins");
            assert_eq!(from > 0, offset > 0.0, "{offset}: from {from}");
            for (i, g) in glyphs.iter().enumerate() {
                assert_eq!(g.chr, chars[from + i]);
                let x = whole.pos_from_cursor(CCursor::new(from + i)).min.x;
                assert!(
                    (g.pos.x - x).abs() <= 0.5,
                    "{offset}: {} at {} not {x}",
                    g.chr,
                    g.pos.x
                );
            }
        }
    }

    /// Shows `lines` as a box from its `first`th line, counting in `calls`
    /// the lines it asks for; gives the widest it measured.
    fn measured(
        ctx: &egui::Context,
        lines: &[String],
        first: usize,
        time: f64,
        calls: &Cell<usize>,
    ) -> f32 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(time),
            ..Default::default()
        };
        calls.set(0);
        let mut out = ctx.run_ui(input, |ui| {
            let line = |i: usize| {
                calls.set(calls.get() + 1);
                (lines[i].as_str(), Color32::WHITE)
            };
            let shown = Lines {
                count: lines.len(),
                line: &line,
                font: font(),
                gap: 0.0,
            };
            let pane = Pane {
                first,
                ..Pane::default()
            };
            shown.show_as(ui, id(), ScrollArea::both().max_height(360.0), pane);
        });
        out.textures_delta.clear();
        ctx.data(|d| d.get_temp::<State>(id())).unwrap().widest
    }

    #[test]
    fn only_new_lines_are_measured_and_all_again_once_they_move_up() {
        let mut lines: Vec<String> = (0..20_000).map(|i| format!("T{i}.0: read")).collect();
        lines[0] = "A line wider than any after it".repeat(4);
        let calls = Cell::new(0);
        let ctx = egui::Context::default();
        let widest = measured(&ctx, &lines, 0, 0.0, &calls);
        assert!(calls.get() >= 20_000);
        assert_eq!(measured(&ctx, &lines, 0, 0.05, &calls), widest);
        assert!(calls.get() < 100, "{} lines asked for", calls.get());
        // A line more, as a log takes one.
        lines.push("T20000.0: read".into());
        assert_eq!(measured(&ctx, &lines, 0, 0.1, &calls), widest);
        assert!(calls.get() < 100, "{} lines asked for", calls.get());
        // The widest gone, the lines after it moved up with no word of it.
        lines.remove(0);
        let narrower = measured(&ctx, &lines, 0, 0.15, &calls);
        assert!(calls.get() >= 20_000, "measured again");
        assert!(narrower < widest / 2.0, "{narrower} of {widest}");
    }

    #[test]
    fn a_log_dropping_its_oldest_measures_only_its_new_lines_until_the_widest_goes() {
        let log = |first: usize| -> Vec<String> {
            let line = |n: usize| match n {
                10 => "A line wider than any after it".repeat(4),
                n => format!("T{n}.0: read"),
            };
            (first..first + 20_000).map(line).collect()
        };
        let calls = Cell::new(0);
        let ctx = egui::Context::default();
        let widest = measured(&ctx, &log(0), 0, 0.0, &calls);
        assert!(calls.get() >= 20_000);
        // Five dropped and five more: the widest still held.
        assert_eq!(measured(&ctx, &log(5), 5, 0.05, &calls), widest);
        assert!(calls.get() < 100, "{} lines asked for", calls.get());
        // The widest dropped too: all of them again.
        let narrower = measured(&ctx, &log(11), 11, 0.1, &calls);
        assert!(calls.get() >= 20_000, "measured again");
        assert!(narrower < widest / 2.0, "{narrower} of {widest}");
    }

    #[test]
    fn a_selection_dragged_past_a_box_scrolls_only_the_ways_the_box_does() {
        // A box that scrolls down only, in a page that scrolls across: as a
        // sector's window's bytes do in the app.
        let lines = rows(64, "00 11 22 33");
        let ctx = egui::Context::default();
        let page = Cell::new(Vec2::ZERO);
        let run = |events: Vec<Event>, time: f64| -> FullOutput {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let output = ScrollArea::horizontal().id_salt("page").show(ui, |ui| {
                    ui.set_min_width(2000.0);
                    let line = |i: usize| (lines[i].as_str(), Color32::WHITE);
                    let shown = Lines {
                        count: lines.len(),
                        line: &line,
                        font: font(),
                        gap: 0.0,
                    };
                    shown.show_as(
                        ui,
                        id(),
                        ScrollArea::vertical().max_height(360.0),
                        Pane::default(),
                    );
                });
                page.set(output.state.offset);
            });
            out.textures_delta.clear();
            out
        };
        run(vec![], 0.0);
        let out = run(vec![], 0.05);
        let from = on(&drawn(&out)[0], 2);
        run(vec![Event::PointerMoved(from)], 0.1);
        run(vec![press(from, true)], 0.15);
        // Past the page's right edge, and below the box.
        let past = pos2(900.0, 500.0);
        for frame in 0..10 {
            run(vec![Event::PointerMoved(past)], 0.2 + frame as f64 / 20.0);
        }
        run(vec![press(past, false)], 1.0);
        assert_eq!(page.get().x, 0.0, "the page left where it was");
        let state = ctx.data(|d| d.get_temp::<State>(id())).unwrap();
        let s = state.selection.unwrap();
        assert_eq!(s.range().1.line, 63, "the box scrolled down");
    }

    #[test]
    fn held_past_its_end_a_dragged_selection_asks_for_no_more_frames() {
        let lines = rows(64, "00 11 22 33");
        let mut w = Window::new();
        let out = w.show(&lines, vec![]);
        let from = on(&drawn(&out)[0], 2);
        w.show(&lines, vec![Event::PointerMoved(from)]);
        w.show(&lines, vec![press(from, true)]);
        // Held below the box: it scrolls to its end, then waits.
        w.show(&lines, vec![Event::PointerMoved(pos2(100.0, 500.0))]);
        let waits = |out: &FullOutput| {
            let root = &out.viewport_output[&egui::ViewportId::ROOT];
            !root.repaint_delay.is_zero()
        };
        let mut frames = 0;
        while !waits(&w.show(&lines, vec![])) {
            frames += 1;
            assert!(frames < 200, "still scrolling");
        }
        let state = w.ctx.data(|d| d.get_temp::<State>(id())).unwrap();
        assert!(state.most.y > 0.0);
        assert_eq!(state.offset.y, state.most.y, "at its end");
        assert_eq!(state.selection.unwrap().range().1.line, 63);
        assert!(waits(&w.show(&lines, vec![])), "and stays");
    }

    #[test]
    fn a_named_box_is_named_to_screen_readers() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let lines = rows(3, "00 11 22 33");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(vec2(400.0, 200.0))
            .build_ui(|ui| {
                let line = |i: usize| (lines[i].as_str(), Color32::WHITE);
                let shown = Lines {
                    count: lines.len(),
                    line: &line,
                    font: font(),
                    gap: 0.0,
                };
                let pane = Pane {
                    name: "Sector bytes",
                    ..Pane::default()
                };
                shown.show_as(ui, id(), ScrollArea::both(), pane);
            });
        harness.run();
        let named = harness.get_by_label("Sector bytes");
        let role = named.accesskit_node().role();
        assert_eq!(role, egui::accesskit::Role::Unknown);
        // Its lines, each a label.
        harness.get_by_label("0010  00 11 22 33");
    }

    /// `word` as it was: from the line's characters and their classes, all
    /// of them collected.
    fn collected(text: &str, k: usize) -> (usize, usize) {
        let chars: Vec<char> = text.chars().collect();
        let mut classes = Vec::with_capacity(chars.len());
        for &c in &chars {
            let of = match classes.last() {
                Some(&before) if combining(c) => before,
                _ => class(c),
            };
            classes.push(of);
        }
        let Some(&of) = classes.get(k) else {
            return (k, k);
        };
        let marks =
            |from: usize| from + chars[from..].iter().take_while(|&&c| combining(c)).count();
        let base = chars[..=k]
            .iter()
            .rposition(|&c| !combining(c))
            .unwrap_or(0);
        if of == Class::Other {
            return (base, marks(base + 1));
        }
        let from = classes[..k].iter().rposition(|&c| c != of);
        let to = classes[k..].iter().position(|&c| c != of);
        (from.map_or(0, |i| i + 1), to.map_or(chars.len(), |i| k + i))
    }

    #[test]
    fn a_word_is_found_going_out_from_its_character_as_from_the_whole_line() {
        let texts = [
            "",
            "a",
            "Write Bandwidth:  7.66",
            "  x  ",
            "e\u{301}te\u{301}.adf",
            "\u{301}\u{301}ab cd",
            "\u{301}",
            "\u{301} \u{301}",
            "a\u{301}\u{301} b\u{20D0}",
            ":\u{301}:x_y",
            "日本語 🙂 é \u{FE20}x",
            "T0.0 <- Image 1.0: AmigaDOS (2/2 sectors)",
        ];
        for text in texts {
            for k in 0..=count(text) + 1 {
                assert_eq!(word(text, k), collected(text, k), "{text:?} at {k}");
            }
        }
    }
}

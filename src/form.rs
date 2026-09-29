//! A command's settings as a form. Some arguments get a hand-made field; the
//! rest, including any a newer gw adds, get one chosen by their type.

use crate::command::{ON, Values};
use crate::schema::{Arg, Command, FormatInfo, ImageOpt, Schema, extension};
use crate::service::{Load, Service};
use crate::theme;
use eframe::egui::{
    self, Color32, CornerRadius, PopupCloseBehavior, RichText, Sense, TextEdit, Ui, pos2, vec2,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Arguments set in the sidebar, for every command that has them.
pub const GLOBAL: [&str; 2] = ["device", "drive"];

/// Arguments a command shows first, in this order. The rest go under Advanced options.
pub const FIRST: &[(&str, &[&str])] = &[
    ("read", &["format", "file", "tracks", "revs"]),
    ("write", &["file", "format", "tracks", "no_verify"]),
    ("convert", &["in_file", "format", "out_file", "tracks"]),
    ("erase", &["tracks"]),
    ("align", &["tracks", "format", "reads"]),
];

/// Arguments whose file is written: a folder, a name and a type.
pub const OUTPUTS: &[(&str, &str)] = &[("read", "file"), ("convert", "out_file")];

/// The kind of file an open dialog shows for an argument that takes one kind.
const FILE_TYPES: &[(&str, &str, &str, &[&str])] =
    &[("update", "file", "Firmware updates", &["upd"])];

/// The update page's firmware source, kept with its settings. Not a gw argument.
pub const FIRMWARE: &str = "firmware";

/// A page's batch settings, kept with its values. Not gw arguments.
pub const BATCH: &str = "batch";
pub const BATCH_FOLDER: &str = "batch_folder";
/// The one image type a batch takes, empty for every type gw reads.
pub const BATCH_TYPE: &str = "batch_type";

/// Commands that take a folder of images one at a time, and the argument
/// each image goes to.
pub const BATCHES: &[(&str, &str)] = &[("write", "file"), ("convert", "in_file")];

/// Where gw update takes the firmware from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Firmware {
    /// The newest release, downloaded: gw's default.
    Latest,
    /// A release by its tag, downloaded.
    Release,
    /// An update file.
    File,
}

impl Firmware {
    const ALL: [Firmware; 3] = [Firmware::Latest, Firmware::Release, Firmware::File];

    /// The button's name, and the value kept under FIRMWARE.
    fn name(self) -> &'static str {
        match self {
            Firmware::Latest => "Latest",
            Firmware::Release => "Release",
            Firmware::File => "File",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Firmware::Latest => "Download the newest release.",
            Firmware::Release => "Download a release by its tag.",
            Firmware::File => "Install an update file.",
        }
    }

    /// The gw argument that names this source.
    fn dest(self) -> Option<&'static str> {
        match self {
            Firmware::Latest => None,
            Firmware::Release => Some("tag"),
            Firmware::File => Some("file"),
        }
    }

    /// The source chosen on the page, else the one its settings name, as
    /// when a command line is typed.
    pub fn of(values: &Values) -> Firmware {
        let chosen = Firmware::ALL
            .into_iter()
            .find(|f| f.name() == values.get(FIRMWARE));
        let named = || {
            Firmware::ALL
                .into_iter()
                .find(|f| f.dest().is_some_and(|d| values.on(d)))
        };
        chosen.or_else(named).unwrap_or(Firmware::Latest)
    }

    /// Whether `dest` is the setting of a source not chosen.
    fn unchosen(values: &Values, dest: &str) -> bool {
        let chosen = Firmware::of(values);
        Firmware::ALL
            .into_iter()
            .any(|f| f != chosen && f.dest() == Some(dest))
    }

    /// Clears the other sources' settings, so gw is given the chosen one alone.
    pub fn only(values: &mut Values) {
        let chosen = Firmware::of(values);
        let others = Firmware::ALL.into_iter().filter(|f| *f != chosen);
        for dest in others.filter_map(Firmware::dest) {
            values.set(dest, "");
        }
    }
}

/// Labels where gw's own argument name would not read well.
const LABELS: &[(&str, &str)] = &[
    ("adjust_speed", "Adjust speed"),
    ("cyls", "Cylinders"),
    ("densel", "Density select"),
    ("diskdefs", "Disk definitions"),
    ("erase_empty", "Erase empty tracks"),
    ("fake_index", "Fake index"),
    ("format", "Disk format"),
    ("gen_tg43", "TG43 signal"),
    ("hard_sectors", "Hard sectors"),
    ("hfreq", "High frequency"),
    ("in_file", "Input"),
    ("linger", "Time per step"),
    ("motor", "Motor delay"),
    ("motor_on", "Motor on"),
    ("no_clobber", "Keep existing files"),
    ("no_verify", "Skip verify"),
    ("nr", "Measurements"),
    ("out_file", "Output"),
    ("out_tracks", "Output tracks"),
    ("pll", "PLL"),
    ("post_write", "Post-write"),
    ("pre_erase", "Erase before writing"),
    ("pre_write", "Pre-write"),
    ("precomp", "Precompensation"),
    ("reverse", "Reverse (flippy)"),
    ("revs", "Revolutions"),
    ("seek_retries", "Seek retries"),
    ("select", "Select delay"),
    ("settle", "Settle time"),
    ("step", "Step delay"),
    ("tag", "Release tag"),
];

/// Common values for arguments that take any; Other… ends the list.
const SUGGESTIONS: &[(&str, &[&str])] = &[
    ("revs", &["1", "2", "3", "5"]),
    ("retries", &["0", "1", "3", "5", "10"]),
    ("seek_retries", &["0", "1", "2", "5"]),
    ("reads", &["5", "10", "20"]),
    ("fake_index", &["300rpm", "360rpm"]),
    ("adjust_speed", &["300rpm", "360rpm"]),
    ("cylinder", &["0", "40", "79"]),
];

const OTHER: &str = "Other…";

/// The format list's family for formats from a disk definitions file.
const CUSTOM: &str = "custom";
const CUSTOM_NAME: &str = "Custom disk definitions";

/// Image types that hold flux or bitcells: Detect can find a format from them.
const FLUX: &[&str] = &[".scp", ".hfe", ".raw", ".a2r", ".ipf", ".ctr"];

/// Names for gw's format families, which it names only by prefix.
const FAMILIES: &[(&str, &str)] = &[
    ("acorn", "Acorn"),
    ("akai", "Akai"),
    ("amiga", "Amiga"),
    ("apple2", "Apple II"),
    ("apricot", "Apricot"),
    ("atari", "Atari 8-bit"),
    ("atarist", "Atari ST"),
    ("coco", "Tandy CoCo"),
    ("commodore", "Commodore"),
    ("datageneral", "Data General"),
    ("dec", "DEC"),
    ("dragon", "Dragon"),
    ("eagle", "Eagle"),
    ("ensoniq", "Ensoniq"),
    ("epson", "Epson"),
    ("gem", "General Music"),
    ("hp", "HP"),
    ("ibm", "IBM PC"),
    ("kaypro", "Kaypro"),
    ("luxor", "Luxor ABC"),
    ("mac", "Macintosh"),
    ("micropolis", "Micropolis"),
    ("mm1", "MM/1"),
    ("msx", "MSX"),
    ("northstar", "North Star"),
    ("occ1", "Osborne 1"),
    ("olivetti", "Olivetti"),
    ("pc98", "NEC PC-98"),
    ("raw", "Raw bitcells"),
    ("rm", "Research Machines"),
    ("sci", "Sequential Circuits"),
    ("sega", "Sega"),
    ("sharp", "Sharp"),
    ("thomson", "Thomson"),
    ("tsc", "TSC FLEX"),
    ("xerox", "Xerox"),
    ("zx", "ZX Spectrum"),
];

/// The image type for formats that gw pairs with none, by prefix.
const TYPES: &[(&str, &str)] = &[
    ("ibm.", ".img"),
    ("atarist.", ".st"),
    ("amiga.", ".adf"),
    ("acorn.dfs.ss", ".ssd"),
    ("acorn.dfs.ds", ".dsd"),
    ("apple2.", ".do"),
    ("commodore.", ".d64"),
];

/// Most characters a typed name takes: an image's name, a disk label, a preset's name.
pub const NAME_LIMIT: usize = 48;
const LABEL_WIDTH: f32 = 112.0;
const MIN_FIELD: f32 = 160.0;
const MAX_FIELD: f32 = 400.0;
/// Short lists and values.
const SHORT_FIELD: f32 = 150.0;
/// A number of two or three digits.
const NUMBER_FIELD: f32 = 56.0;
/// A cylinder number's box in the track picker.
const NUMBER_BOX: f32 = 32.0;
/// Detect, beside the format.
const DETECT_BUTTON: f32 = 68.0;
/// The browse button, beside a path.
const BROWSE_BUTTON: f32 = 34.0;
const ROW_GAP: f32 = 12.0;
/// Room kept right of a form for its scroll bar.
const SCROLL_GUTTER: f32 = 14.0;

/// A field fills the room beside its label, up to a point.
fn field_width(ui: &Ui) -> f32 {
    ui.available_width().clamp(MIN_FIELD, MAX_FIELD)
}

/// The width of a form's rows, from the room the page has: notices and
/// headings end where the fields do.
pub fn form_width(ui: &Ui) -> f32 {
    let gap = ui.spacing().item_spacing.x;
    let room = ui.available_width() - SCROLL_GUTTER;
    let field = (room - LABEL_WIDTH - gap).clamp(MIN_FIELD, MAX_FIELD);
    LABEL_WIDTH + gap + field
}

/// A form's width when its fields are as wide as they get.
pub fn full_width(ui: &Ui) -> f32 {
    LABEL_WIDTH + ui.spacing().item_spacing.x + MAX_FIELD + SCROLL_GUTTER
}

/// A one-line text field as tall as the lists and buttons beside it.
pub fn edit(text: &mut String) -> TextEdit<'_> {
    TextEdit::singleline(text)
        .min_size(vec2(0.0, theme::FIELD_HEIGHT))
        .vertical_align(egui::Align::Center)
        .margin(egui::Margin::symmetric(10, 4))
}

/// The width left for a field with a button after it, so both end at the
/// field's right edge.
fn beside_button(ui: &Ui, button: f32) -> f32 {
    field_width(ui) - button - ui.spacing().item_spacing.x
}

/// Lays out a widget in exactly this width, so a list's long choice cannot
/// widen the row.
fn sized<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(vec2(width, theme::FIELD_HEIGHT), layout, |ui| {
        ui.set_max_width(width);
        add(ui)
    })
    .inner
}

/// The browse button: a folder with a magnifying glass, painted about 17 by
/// 13 points.
fn browse_button(ui: &mut Ui) -> egui::Response {
    let enabled = ui.is_enabled();
    let size = vec2(BROWSE_BUTTON, theme::FIELD_HEIGHT);
    let response = ui.add(egui::Button::new("").min_size(size));
    let colour = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let stroke = egui::Stroke::new(1.2, colour);
    let at = |x: f32, y: f32| response.rect.center() + vec2(x, y);
    let folder = vec![
        at(-8.5, -6.5),
        at(-3.5, -6.5),
        at(-2.0, -5.0),
        at(8.5, -5.0),
        at(8.5, 6.5),
        at(-8.5, 6.5),
    ];
    let painter = ui.painter();
    painter.add(egui::Shape::closed_line(folder, stroke));
    painter.line_segment([at(-8.5, -3.0), at(8.5, -3.0)], stroke);
    painter.circle_stroke(at(-0.6, 1.3), 1.9, stroke);
    painter.line_segment([at(0.8, 2.7), at(2.5, 4.4)], stroke);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, "Browse"));
    response
}

/// A row of the form: a label beside a field, level with its first line.
fn row<R>(ui: &mut Ui, label: &str, field: impl FnOnce(&mut Ui) -> R) -> (egui::Response, R) {
    ui.horizontal_top(|ui| {
        let size = vec2(LABEL_WIDTH, ui.spacing().interact_size.y);
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        // A long label wraps rather than push into its field.
        let name = ui.allocate_ui_with_layout(size, layout, |ui| {
            ui.set_min_size(size);
            ui.set_max_width(LABEL_WIDTH);
            ui.add(egui::Label::new(label).wrap())
        });
        let field = ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field(ui)
        });
        (name.inner, field.inner)
    })
    .inner
}

pub struct Form<'a> {
    pub schema: &'a Schema,
    pub cmd: &'a Command,
    pub values: &'a mut Values,
    pub outputs: &'a mut BTreeMap<String, Output>,
    pub service: &'a mut Service,
    /// Why Detect cannot start now, if it cannot.
    pub cannot_detect: Option<&'a str>,
}

/// Something the form asks of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Find the format of the disk, or of the input file.
    Detect,
}

impl<'a> Form<'a> {
    pub fn show(mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        let (first, rest) = sections(self.cmd);
        ui.spacing_mut().item_spacing.y = ROW_GAP;
        if self.cmd.name == "update" {
            self.firmware(ui);
        }
        for a in &first {
            action = action.or(self.arg(ui, a));
        }
        if self.cmd.name == "read" && self.cmd.arg("file").is_some() {
            ui.add_space(4.0);
            self.disks(ui);
        }
        if !rest.is_empty() {
            ui.add_space(4.0);
            let title = RichText::new(format!("Advanced options ({})", rest.len())).strong();
            egui::CollapsingHeader::new(title)
                .id_salt(("more", &self.cmd.name))
                .show_unindented(ui, |ui| {
                    ui.add_space(6.0);
                    for a in &rest {
                        action = action.or(self.arg(ui, a));
                    }
                });
        }
        action
    }

    fn arg(&mut self, ui: &mut Ui, a: &Arg) -> Option<Action> {
        if OUTPUTS.contains(&(self.cmd.name.as_str(), a.dest.as_str())) {
            self.output(ui, a);
            return None;
        }
        // Below the Firmware row, only the chosen source's field.
        if self.cmd.name == "update" && Firmware::unchosen(self.values, &a.dest) {
            return None;
        }
        let blocker = self.blocker(a);
        ui.data_mut(|d| d.remove_temp::<bool>(own_tip_id()));
        // "File" would name one of its own choices.
        let batchable = BATCHES.contains(&(self.cmd.name.as_str(), a.dest.as_str()));
        let text = match (batchable, a.dest.as_str()) {
            (true, "file") => "Image".to_owned(),
            _ => label(a),
        };
        let (name, (field, action)) = row(ui, &text, |ui| {
            let r = ui.add_enabled_ui(blocker.is_none(), |ui| self.field(ui, a));
            (r.response, r.inner)
        });
        let tip = tip(&self.cmd.name, a);
        name.on_hover_text(&tip);
        // Laid over the field, so it is hovered along with whatever is under
        // it, unless that has a tooltip of its own.
        let over = ui.interact(field.rect, field.id.with("tip"), Sense::hover());
        let quiet = ui.data_mut(|d| d.remove_temp::<bool>(own_tip_id()));
        if let Some(b) = blocker {
            over.on_hover_text(format!("Cannot be used with {}.", label(b)));
        } else if quiet.is_none() {
            over.on_hover_text(tip);
        }
        action
    }

    /// Another argument from the same exclusive group that is already set.
    fn blocker(&self, a: &Arg) -> Option<&'a Arg> {
        let group = a.group?;
        self.cmd
            .args
            .iter()
            .find(|b| b.group == Some(group) && b.dest != a.dest && self.values.on(&b.dest))
    }

    fn field(&mut self, ui: &mut Ui, a: &Arg) -> Option<Action> {
        match a.dest.as_str() {
            "format" => return self.format(ui, a),
            _ if a.is("TrackSet") => self.tracks(ui, a),
            "file" | "in_file" if a.positional() => self.input(ui, a),
            "diskdefs" => self.diskdefs(ui, a),
            // Shown once File is chosen, so it is needed.
            "file" if self.cmd.name == "update" => self.path(ui, a, "Required"),
            dest if dest.ends_with("file") => self.path(ui, a, "None"),
            _ if a.switch => {
                let mut on = self.values.on(&a.dest);
                if toggle(ui, &mut on).changed() {
                    self.values.set(&a.dest, if on { ON } else { "" });
                }
            }
            _ if !a.choices.is_empty() => {
                let options: Vec<(&str, &str)> =
                    a.choices.iter().map(|c| (c.as_str(), c.as_str())).collect();
                self.pick(ui, a, &options);
            }
            _ if a.is("level") => self.pick(ui, a, &[("H", "High"), ("L", "Low")]),
            dest => match SUGGESTIONS.iter().find(|(d, _)| *d == dest) {
                Some((_, options)) => self.suggest(ui, a, options),
                None => self.text(ui, a),
            },
        }
        None
    }

    /// Common values in a list, and Other… for anything else.
    fn suggest(&mut self, ui: &mut Ui, a: &Arg, options: &[&str]) {
        let current = self.values.get(&a.dest).to_owned();
        let other_id = ui.make_persistent_id(("other", &self.cmd.name, &a.dest));
        let listed = current.is_empty() || options.contains(&current.as_str());
        let mut other = !listed || ui.data(|d| d.get_temp(other_id)).unwrap_or(false);
        let default = a
            .default
            .as_deref()
            .map_or_else(|| "Default".to_owned(), |d| format!("Default ({d})"));
        let shown = match (other, current.as_str()) {
            (true, _) => OTHER,
            (false, "") => default.as_str(),
            (false, value) => value,
        };
        let mut chosen = None;
        ui.horizontal(|ui| {
            sized(ui, SHORT_FIELD, |ui| {
                egui::ComboBox::from_id_salt(("suggest", &self.cmd.name, &a.dest))
                    .selected_text(shown)
                    .truncate()
                    .width(SHORT_FIELD)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(!other && current.is_empty(), &default)
                            .clicked()
                        {
                            chosen = Some((false, ""));
                        }
                        for &o in options {
                            if ui.selectable_label(!other && current == o, o).clicked() {
                                chosen = Some((false, o));
                            }
                        }
                        if ui.selectable_label(other, OTHER).clicked() {
                            chosen = Some((true, ""));
                        }
                    })
            });
            if let Some((o, value)) = chosen {
                other = o;
                self.values.set(&a.dest, value);
            }
            if other {
                self.typed(ui, a, hint(a, self.schema), SHORT_FIELD);
            }
        });
        ui.data_mut(|d| d.insert_temp(other_id, other));
    }

    /// Choices as a row of buttons; choosing the chosen one again clears it.
    fn pick(&mut self, ui: &mut Ui, a: &Arg, options: &[(&str, &str)]) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let current = self.values.get(&a.dest).to_owned();
            let (unset, tip) = match a.dest.as_str() {
                "densel" => (
                    "Auto",
                    "Leave pin 2 as it is. Most drives sense density from the disk.",
                ),
                _ => ("Default", "gw's own choice."),
            };
            if a.default.is_none()
                && ui
                    .selectable_label(current.is_empty(), unset)
                    .own_tip(tip)
                    .clicked()
            {
                self.values.set(&a.dest, "");
            }
            for &(value, text) in options {
                if ui.selectable_label(current == value, text).clicked() {
                    self.values
                        .set(&a.dest, if current == value { "" } else { value });
                }
            }
        });
    }

    /// Where gw update gets the firmware. The chosen source's field follows.
    fn firmware(&mut self, ui: &mut Ui) {
        let chosen = Firmware::of(self.values);
        let cmd = self.cmd;
        let offered = Firmware::ALL
            .into_iter()
            .filter(|f| f.dest().is_none_or(|d| cmd.arg(d).is_some()));
        let (name, _) = row(ui, "Firmware", |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for f in offered {
                    let button = ui.selectable_label(f == chosen, f.name());
                    if button.on_hover_text(f.tip()).clicked() {
                        self.values.set(FIRMWARE, f.name());
                    }
                }
            });
        });
        name.on_hover_text("Where the firmware comes from.");
    }

    fn text(&mut self, ui: &mut Ui, a: &Arg) {
        let number = matches!(a.ty.as_deref(), Some("min_int" | "int" | "uint"));
        ui.horizontal(|ui| {
            let width = if number { SHORT_FIELD } else { field_width(ui) };
            self.typed(ui, a, hint(a, self.schema), width);
        });
    }

    /// A box to type the value in, and gw's objection to what is typed.
    fn typed(&mut self, ui: &mut Ui, a: &Arg, hint: String, width: f32) {
        let mut value = self.values.get(&a.dest).to_owned();
        let edit = edit(&mut value).hint_text(hint).desired_width(width);
        if ui.add(edit).changed() {
            self.values.set(&a.dest, value.as_str());
        }
        self.complaint(ui, a, &value);
    }

    /// gw's own objection to a value, checked with its parser for that argument.
    fn complaint(&mut self, ui: &mut Ui, a: &Arg, value: &str) {
        if value.is_empty() || a.ty.is_none() {
            return;
        }
        if let Some(e) = self.service.check(&self.cmd.name, &a.dest, value) {
            ui.label(
                RichText::new(sentence(e))
                    .color(theme::palette(ui).bad)
                    .small(),
            );
        }
    }

    /// A file for gw to read, typed or chosen in an open dialog.
    fn path(&mut self, ui: &mut Ui, a: &Arg, hint: &str) {
        let mut value = self.values.get(&a.dest).to_owned();
        ui.horizontal(|ui| {
            let edit = edit(&mut value)
                .hint_text(hint)
                .desired_width(beside_button(ui, BROWSE_BUTTON));
            if ui.add(edit).changed() {
                self.values.set(&a.dest, value.as_str());
            }
            if browse_button(ui).own_tip("Choose a file.").clicked()
                && let Some(path) = open_dialog(&self.cmd.name, &a.dest).pick_file()
            {
                self.values.set(&a.dest, path.to_string_lossy());
            }
        });
    }

    /// A disk definitions file, and what gw makes of it.
    fn diskdefs(&mut self, ui: &mut Ui, a: &Arg) {
        self.path(ui, a, "None");
        let path = self.values.get(&a.dest).to_owned();
        if path.is_empty() {
            return;
        }
        let p = theme::palette(ui);
        let file = Path::new(&path)
            .file_name()
            .map_or_else(|| path.clone(), |f| f.to_string_lossy().into_owned());
        let small = |text: String| RichText::new(text.replace(&path, &file)).small();
        match self.service.diskdefs(&path) {
            Load::Waiting(_) => {
                ui.label(small("Checking…".into()).weak());
            }
            Load::Failed(e) => {
                ui.label(small(sentence(e)).color(p.bad));
            }
            Load::Ready(d) if d.errors.is_empty() => {
                let n = d.formats.len();
                let formats = if n == 1 { "format" } else { "formats" };
                let text = format!("Adds {n} {formats} to the top of the format list.");
                ui.label(small(text).weak());
            }
            Load::Ready(d) => {
                for e in d.errors.iter().take(3) {
                    ui.label(small(sentence(e)).color(p.bad));
                }
                if d.errors.len() > 3 {
                    let more = format!("And {} more.", d.errors.len() - 3);
                    ui.label(small(more).color(p.bad));
                }
            }
        }
    }

    fn effective_format(&mut self) -> Option<String> {
        effective_format(self.service, self.schema, self.cmd, self.values)
    }

    /// gw's objection to the input, where it would find the format in the file.
    fn input_fault(&mut self) -> Option<String> {
        if !format_in_file(self.schema, self.cmd, self.values) {
            return None;
        }
        let path = input_file(self.cmd, self.values);
        let e = self.service.image_format(path).error()?;
        let file = Path::new(path).file_name()?.to_string_lossy();
        Some(sentence(&e.replace(path, &file)))
    }

    /// What gw says of a format, read with the disk definitions it needs.
    fn format_info(&mut self, format: &str) -> &Load<FormatInfo> {
        let diskdefs = diskdefs_for(self.service, self.values, format);
        self.service.format_info(&diskdefs, format)
    }

    fn format(&mut self, ui: &mut Ui, a: &Arg) -> Option<Action> {
        let current = self.values.get(&a.dest).to_owned();
        let diskdefs = self.values.get("diskdefs").to_owned();
        let effective = self.effective_format();
        let custom = self.service.custom_formats(&diskdefs).contains(&current);
        let dim = theme::palette(ui).dim;
        let shown = match (current.as_str(), &effective) {
            ("", Some(own)) => {
                // The input's own comes first, as in gw.
                let input = own_format(self.schema, input_file(self.cmd, self.values));
                let output = own_format(self.schema, output_file(self.cmd, self.values));
                let from = match (input, output) {
                    (None, Some(_)) => "the image type",
                    _ => "the input",
                };
                RichText::new(format!("{} (from {from})", format_name(own))).color(dim)
            }
            ("", None) => RichText::new("Choose disk format").color(dim),
            (chosen, _) if custom => RichText::new(format!("Custom · {chosen}")),
            (chosen, _) => RichText::new(format_name(chosen)),
        };
        let mut chosen = None;
        let mut action = None;
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let can_detect = flux_source(self.cmd, self.values);
                let width = if can_detect {
                    beside_button(ui, DETECT_BUTTON)
                } else {
                    field_width(ui)
                };
                sized(ui, width, |ui| {
                    egui::ComboBox::from_id_salt(("format", &self.cmd.name))
                        .selected_text(shown)
                        .truncate()
                        .width(width)
                        .height(400.0)
                        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                        .show_ui(ui, |ui| {
                            chosen = self.format_list(ui, &current, &diskdefs);
                            if chosen.is_some() {
                                ui.close();
                            }
                        })
                });
                if can_detect {
                    let detect = egui::Button::new("Detect")
                        .min_size(vec2(DETECT_BUTTON, theme::FIELD_HEIGHT));
                    if ui
                        .add_enabled(self.cannot_detect.is_none(), detect)
                        .own_tip("Attempt to find the disk format and image type.")
                        .on_disabled_hover_text(self.cannot_detect.unwrap_or_default())
                        .clicked()
                    {
                        action = Some(Action::Detect);
                    }
                }
            });
            if let Some(format) = effective {
                match self.format_info(&format) {
                    Load::Ready(info) => {
                        ui.label(RichText::new(describe(info)).small().weak());
                    }
                    Load::Failed(e) => {
                        ui.label(
                            RichText::new(sentence(e))
                                .small()
                                .color(theme::palette(ui).bad),
                        );
                    }
                    Load::Waiting(_) => {}
                }
            } else if let Some(e) = self.input_fault() {
                ui.label(RichText::new(e).small().color(theme::palette(ui).bad));
            }
        });
        if let Some(f) = chosen {
            choose_format(self.schema, self.cmd, self.values, self.outputs, &f);
        }
        action
    }

    /// The formats: families on the left, a family's formats on the right, or
    /// a search across all. Returns the format chosen.
    fn format_list(&mut self, ui: &mut Ui, current: &str, diskdefs: &str) -> Option<String> {
        let search_id = ui.make_persistent_id(("format-search", &self.cmd.name));
        let family_id = ui.make_persistent_id(("format-family", &self.cmd.name));
        let mut search: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
        ui.set_min_width(420.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        let edit = TextEdit::singleline(&mut search)
            .hint_text("Search, e.g. akai or 1440")
            .desired_width(f32::INFINITY);
        ui.add(edit).request_focus();
        let needle = search.trim().to_lowercase();
        ui.data_mut(|d| d.insert_temp(search_id, search));
        let custom = self.service.custom_formats(diskdefs).to_vec();
        let Some(formats) = self.service.formats() else {
            ui.spinner();
            return None;
        };
        // A file's formats come first, under their own heading, then gw's by family.
        let family_of = |f: &str| match custom.iter().any(|c| c == f) {
            true => CUSTOM.to_owned(),
            false => f.split('.').next().unwrap_or_default().to_owned(),
        };
        let heading = |family: &str| match family {
            CUSTOM => CUSTOM_NAME.to_owned(),
            f => family_name(f),
        };
        let all: Vec<&String> = custom
            .iter()
            .chain(formats.iter().filter(|f| !custom.contains(f)))
            .collect();
        let mut chosen = None;
        if !needle.is_empty() {
            // Search: every match, under its family.
            egui::ScrollArea::vertical()
                .max_height(340.0)
                .show(ui, |ui| {
                    let mut group = String::new();
                    let matches = all.iter().filter(|f| {
                        f.to_lowercase().contains(&needle)
                            || heading(&family_of(f)).to_lowercase().contains(&needle)
                    });
                    for f in matches {
                        let family = family_of(f);
                        if family != group {
                            ui.add_space(4.0);
                            ui.label(RichText::new(heading(&family)).strong());
                            group = family;
                        }
                        if ui.selectable_label(*f == current, f.as_str()).clicked() {
                            chosen = Some((*f).clone());
                        }
                    }
                });
        } else {
            if ui
                .selectable_label(current.is_empty(), "None")
                .on_hover_text("Leave the format to gw: the images' own, if they have one.")
                .clicked()
            {
                chosen = Some(String::new());
            }
            ui.separator();
            let mut families: Vec<String> = formats
                .iter()
                .filter_map(|f| f.split('.').next())
                .map(str::to_owned)
                .collect();
            families.dedup();
            families.sort_by_cached_key(|f| family_name(f));
            if !custom.is_empty() {
                families.insert(0, CUSTOM.to_owned());
            }
            let open: String = ui.data(|d| d.get_temp(family_id)).unwrap_or_else(|| {
                match (current.is_empty(), custom.is_empty()) {
                    (true, false) => CUSTOM.to_owned(),
                    (true, true) => "ibm".to_owned(),
                    (false, _) => family_of(current),
                }
            });
            let current_family = family_of(current);
            ui.horizontal_top(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt("families")
                    .max_height(320.0)
                    .show(ui, |ui| {
                        ui.set_width(170.0);
                        // A scroll area lays out as its parent does, and this row is horizontal.
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                            for family in &families {
                                let mut text = RichText::new(heading(family));
                                if !current.is_empty() && current_family == *family {
                                    text = text.strong();
                                }
                                if ui.selectable_label(*family == open, text).clicked() {
                                    ui.data_mut(|d| d.insert_temp(family_id, family.clone()));
                                }
                                if family == CUSTOM {
                                    ui.separator();
                                }
                            }
                        });
                    });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("in-family")
                    .max_height(320.0)
                    .show(ui, |ui| {
                        ui.set_width(210.0);
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                            for f in all.iter().filter(|f| family_of(f) == open) {
                                if ui.selectable_label(*f == current, f.as_str()).clicked() {
                                    chosen = Some((*f).clone());
                                }
                            }
                        });
                    });
            });
        }
        if chosen.is_some() {
            ui.data_mut(|d| d.remove_temp::<String>(search_id));
        }
        chosen
    }

    fn tracks(&mut self, ui: &mut Ui, a: &Arg) {
        let (cyls, heads) = self
            .effective_format()
            .and_then(|f| self.format_info(&f).ready().map(|i| (i.cyls, i.heads)))
            .unwrap_or(USUAL_DISK);
        let mut spec = TrackSpec::parse(self.values.get(&a.dest));
        let text_id = ui.make_persistent_id(("tracks-text", &self.cmd.name, &a.dest));
        let as_text = ui.data(|d| d.get_temp(text_id)).unwrap_or(false) || !spec.simple();
        ui.vertical(|ui| {
            if as_text {
                ui.horizontal(|ui| {
                    let hint = example(self.schema, "TSPEC").unwrap_or_default();
                    let width = field_width(ui);
                    self.typed(ui, a, hint, width);
                });
            } else {
                let (mut first, mut last) = spec.cylinders().unwrap_or((0, cyls.saturating_sub(1)));
                let mut changed = false;
                // Wraps rather than run past the field's edge in a narrow window.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    // Cylinder numbers have three digits at most.
                    ui.spacing_mut().interact_size.x = NUMBER_BOX;
                    ui.label("Cylinders");
                    changed |= ui
                        .add(egui::DragValue::new(&mut first).range(0..=last))
                        .changed();
                    ui.label("to");
                    changed |= ui
                        .add(egui::DragValue::new(&mut last).range(first..=254))
                        .changed();
                    if changed {
                        let default = first == 0 && last + 1 == cyls;
                        spec.c = (!default).then(|| format!("{first}-{last}"));
                    }
                    ui.label("Sides");
                    let fixed = !spec.sides_can_change(heads);
                    for head in 0..2u32 {
                        let on = spec.has_head(head, heads);
                        let r = ui
                            .add_enabled(!fixed, egui::Button::selectable(on, head.to_string()))
                            .on_disabled_hover_text("The format has one side.");
                        if fixed && r.contains_pointer() {
                            ui.data_mut(|d| d.insert_temp(own_tip_id(), true));
                        }
                        if r.clicked() {
                            spec.toggle_head(head, heads);
                            changed = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let mut double = spec.step.as_deref() == Some("2");
                    if checkbox(ui, &mut double, "Double step")
                        .own_tip("For a 40-track disk in an 80-track drive.")
                        .changed()
                    {
                        spec.step = double.then(|| "2".to_owned());
                        changed = true;
                    }
                    changed |= checkbox(ui, &mut spec.hswap, "Swap sides")
                        .own_tip("Read side 1 as side 0 and side 0 as side 1.")
                        .changed();
                });
                if changed {
                    self.values.set(&a.dest, spec.to_string());
                }
            }
            let flip = if as_text {
                "Use the track picker"
            } else {
                "Type a track list"
            };
            if ui
                .add_enabled(spec.simple(), egui::Link::new(RichText::new(flip).small()))
                .clicked()
            {
                ui.data_mut(|d| d.insert_temp(text_id, !as_text));
            }
        });
    }

    /// A file to read, with the image type it has and any options that type takes.
    /// The image a page takes; on Write and Convert, or a folder of them.
    fn input(&mut self, ui: &mut Ui, a: &Arg) {
        let batchable = BATCHES.contains(&(self.cmd.name.as_str(), a.dest.as_str()));
        ui.vertical(|ui| {
            if batchable {
                self.source(ui, a);
            }
            match batchable && self.values.on(BATCH) {
                true => self.folder(ui, a),
                false => self.image(ui, a),
            }
        });
    }

    fn source(&mut self, ui: &mut Ui, a: &Arg) {
        let batch = self.values.on(BATCH);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (on, name, tip) in [
                (false, "File", "One image."),
                (
                    true,
                    "Folder",
                    "Every image in a folder, one after another in name order.",
                ),
            ] {
                if ui
                    .selectable_label(batch == on, name)
                    .own_tip(tip)
                    .clicked()
                    && batch != on
                {
                    let file = split_opts(self.values.get(&a.dest)).0.to_owned();
                    if on && !self.values.on(BATCH_FOLDER) {
                        let parent = Path::new(&file).parent().map(Path::as_os_str);
                        self.values.set(BATCH_FOLDER, lossy(parent));
                    }
                    // The folder's first image is no choice of a file.
                    self.values.set(&a.dest, "");
                    self.values.set(BATCH, if on { ON } else { "" });
                }
            }
        });
    }

    /// A folder of images. Its first stands for them all on the page: the
    /// format, Detect and the command line go by it.
    fn folder(&mut self, ui: &mut Ui, a: &Arg) {
        let mut folder = self.values.get(BATCH_FOLDER).to_owned();
        ui.horizontal(|ui| {
            let edit = edit(&mut folder)
                .hint_text("Required")
                .desired_width(beside_button(ui, BROWSE_BUTTON));
            ui.add(edit);
            if browse_button(ui)
                .own_tip("Choose a folder of images.")
                .clicked()
                && let Some(f) = rfd::FileDialog::new().set_directory(&folder).pick_folder()
            {
                folder = f.to_string_lossy().into_owned();
            }
        });
        self.values.set(BATCH_FOLDER, folder.as_str());
        let files = self.service.folder(&folder).to_vec();
        let mut only = self.values.get(BATCH_TYPE).to_owned();
        let taken = batch_images(self.schema, &files, &only);
        if !folder.is_empty() {
            let schema = self.schema;
            let name =
                |e: &str| image_name(e, schema.images.get(e).map_or("", |i| i.name.as_str()));
            let types: BTreeSet<String> = batch_images(schema, &files, "")
                .iter()
                .filter_map(|p| extension(&p.to_string_lossy()))
                .collect();
            ui.horizontal(|ui| {
                let shown = match only.as_str() {
                    "" => "Every type".to_owned(),
                    e => name(e),
                };
                egui::ComboBox::from_id_salt(("batch type", &self.cmd.name))
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut only, String::new(), "Every type");
                        for e in &types {
                            ui.selectable_value(&mut only, e.clone(), name(e));
                        }
                    })
                    .response
                    .own_tip("Which of the folder's images to take.");
                let p = theme::palette(ui);
                let text = match taken.is_empty() {
                    true => RichText::new("No images gw can read.").color(p.bad),
                    false => RichText::new(listing(&taken)).weak(),
                };
                ui.add(egui::Label::new(text.small()).truncate());
            });
        }
        self.values.set(BATCH_TYPE, only);
        let first = taken
            .first()
            .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
        if self.values.get(&a.dest) != first {
            self.values.set(&a.dest, first);
            // The run button above was drawn with the old image.
            ui.ctx().request_repaint();
        }
    }

    fn image(&mut self, ui: &mut Ui, a: &Arg) {
        let (path, mut opts) = split_opts(self.values.get(&a.dest));
        let mut path = path.to_owned();
        let mut changed = false;
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let edit = edit(&mut path)
                    .hint_text("Required")
                    .desired_width(beside_button(ui, BROWSE_BUTTON));
                changed |= ui.add(edit).changed();
                if browse_button(ui).own_tip("Choose an image.").clicked() {
                    let exts: Vec<&str> = self
                        .schema
                        .images
                        .keys()
                        .map(|e| e.trim_start_matches('.'))
                        .collect();
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Disk images", &exts)
                        .pick_file()
                    {
                        path = p.to_string_lossy().into_owned();
                        changed = true;
                    }
                }
            });
            if !path.is_empty() {
                match self.schema.image(&path) {
                    Some((_, image)) => {
                        ui.label(RichText::new(image_name(&path, &image.name)).small().weak());
                        if !image.read_opts.is_empty() {
                            changed |= image_options(ui, &image.read_opts, &mut opts);
                        }
                    }
                    None => {
                        ui.label(
                            RichText::new("gw does not know this file type.")
                                .small()
                                .color(theme::palette(ui).bad),
                        );
                    }
                }
            }
        });
        if changed {
            self.values.set(&a.dest, join_opts(&path, &opts));
        }
    }

    /// Where a new image goes: its type, folder and name, and the file that makes.
    fn output(&mut self, ui: &mut Ui, a: &Arg) {
        let schema = self.schema;
        let input = input_file(self.cmd, self.values).to_owned();
        let has_input = self.cmd.arg("in_file").is_some();
        let batch = has_input && self.values.on(BATCH);
        let images = match batch {
            true => batch_images(
                schema,
                self.service.known_folder(self.values.get(BATCH_FOLDER)),
                self.values.get(BATCH_TYPE),
            ),
            false => Vec::new(),
        };
        let out = self
            .outputs
            .entry(output_key(&self.cmd.name, &a.dest))
            .or_default();
        let p = theme::palette(ui);

        let mut picked = out.ext.clone();
        let (name, _) = row(ui, "Image type", |ui| {
            let shown = match picked.as_str() {
                "" => RichText::new("Choose image type").color(p.dim),
                e => RichText::new(image_name(
                    e,
                    schema.images.get(e).map_or("", |i| i.name.as_str()),
                )),
            };
            let width = field_width(ui);
            sized(ui, width, |ui| {
                egui::ComboBox::from_id_salt(("type", &self.cmd.name))
                    .selected_text(shown)
                    .truncate()
                    .width(width)
                    .height(380.0)
                    .show_ui(ui, |ui| {
                        for (e, image) in schema.images.iter().filter(|(_, i)| i.writable) {
                            ui.selectable_value(&mut picked, e.clone(), image_name(e, &image.name));
                        }
                    })
                    .response
                    .on_hover_text(TYPE_TIP)
            });
        });
        name.on_hover_text(TYPE_TIP);
        if picked != out.ext {
            out.ext = picked;
            out.opts.clear();
        }
        if let Some(image) = schema
            .images
            .get(&out.ext)
            .filter(|i| !i.write_opts.is_empty())
        {
            row(ui, "Image options", |ui| {
                image_options(ui, &image.write_opts, &mut out.opts)
            })
            .0
            .on_hover_text("Settings of this image type.");
        }

        ui.add_space(2.0);
        ui.label(RichText::new("Save to").small().strong().color(p.dim));
        if has_input {
            // Beside the input, an image of the input's own type would replace it.
            let clash =
                !batch && extension(&input).is_some_and(|e| e.eq_ignore_ascii_case(&out.ext));
            out.beside_input &= !clash;
            let (text, tip) = match batch {
                true => (
                    "Next to each input",
                    "Save each image in its input's folder.",
                ),
                false => (
                    "Next to the input file",
                    "Save the image in the input's folder, under its name.",
                ),
            };
            row(ui, "Save", |ui| {
                ui.add_enabled_ui(!clash, |ui| checkbox(ui, &mut out.beside_input, text))
                    .response
                    .on_disabled_hover_text("It would have the input's name and replace it.")
            })
            .0
            .on_hover_text(tip);
        }
        let beside = has_input && out.beside_input;
        if beside && !batch && !input.is_empty() {
            let input = Path::new(&input);
            out.folder = lossy(input.parent().map(Path::as_os_str));
            out.name = lossy(input.file_stem());
        }
        if batch {
            if !beside {
                folder_row(ui, &mut out.folder);
            }
            for (label, text, tip) in [
                (
                    "Prefix",
                    &mut out.prefix,
                    "Text before each input's name, such as Backup_.",
                ),
                (
                    "Suffix",
                    &mut out.suffix,
                    "Text after each input's name, such as _copy.",
                ),
            ] {
                let (name, _) = row(ui, label, |ui| {
                    ui.add(
                        edit(text)
                            .char_limit(NAME_LIMIT)
                            .hint_text("None")
                            .desired_width(SHORT_FIELD),
                    )
                    .on_hover_text(tip);
                });
                name.on_hover_text(tip);
            }
        } else if !beside {
            folder_row(ui, &mut out.folder);
            let (name, _) = row(ui, "Name", |ui| {
                ui.add(
                    edit(&mut out.name)
                        .char_limit(NAME_LIMIT)
                        .hint_text("Required")
                        .desired_width(field_width(ui)),
                )
                .on_hover_text("The image's file name, without its type.");
            });
            name.on_hover_text("The image's file name, without its type.");
        }

        let value = match (batch, images.first()) {
            (true, Some(first)) => out.batch_value(first),
            (true, None) => String::new(),
            (false, _) => out.value(1),
        };
        if !value.is_empty() {
            row(ui, "", |ui| {
                let preview = match batch {
                    true => out.batch_preview(&images),
                    false => out.preview(),
                };
                ui.label(RichText::new(preview).monospace().small().color(p.dim));
                let replaces = match batch {
                    true => images.iter().any(|i| out.batch_path(i) == *i),
                    false => !input.is_empty() && Path::new(&input) == out.path(1),
                };
                if replaces {
                    let text = if batch {
                        REPLACES_INPUTS
                    } else {
                        REPLACES_INPUT
                    };
                    ui.label(RichText::new(text).small().color(p.bad));
                }
            });
        }
        if self.values.get(&a.dest) != value {
            self.values.set(&a.dest, value);
            // The run button above was drawn with the old path.
            ui.ctx().request_repaint();
        }
    }

    /// Several disks read one after another, each into a numbered file.
    fn disks(&mut self, ui: &mut Ui) {
        let out = self
            .outputs
            .entry(output_key(&self.cmd.name, "file"))
            .or_default();
        let title = match out.disks {
            0 | 1 => "Multiple disks".to_owned(),
            n => format!("Multiple disks ({n})"),
        };
        egui::CollapsingHeader::new(RichText::new(title).strong())
            .id_salt(("disks", &self.cmd.name))
            .show_unindented(ui, |ui| {
                ui.add_space(6.0);
                let tip = "How many disks to read, one after another.";
                let (name, _) = row(ui, "Disks", |ui| {
                    let size = vec2(NUMBER_FIELD, theme::FIELD_HEIGHT);
                    ui.add_sized(
                        size,
                        egui::DragValue::new(&mut out.disks).range(1..=MAX_DISKS),
                    )
                    .on_hover_text(tip)
                });
                name.on_hover_text(tip);
                let tip = "The word before each disk number, such as Disk in Game_Disk1.";
                let (name, _) = row(ui, "Label", |ui| {
                    ui.add_enabled(
                        out.disks > 1,
                        edit(&mut out.label)
                            .char_limit(NAME_LIMIT)
                            .hint_text("e.g. Disk")
                            .desired_width(SHORT_FIELD),
                    )
                    .on_hover_text(tip)
                    .on_disabled_hover_text("Needs more than one disk.");
                });
                name.on_hover_text(tip);
                let (name, _) = row(ui, "Number", |ui| {
                    ui.add_enabled_ui(out.disks > 1, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.selectable_value(&mut out.number_first, false, "After the name")
                                .on_hover_text("Game_Disk1, Game_Disk2…");
                            ui.selectable_value(&mut out.number_first, true, "Before the name")
                                .on_hover_text("Disk1_Game, Disk2_Game…");
                        });
                    })
                    .response
                    .on_disabled_hover_text("Needs more than one disk.");
                });
                name.on_hover_text("Where each file's disk number goes.");
                if out.disks > 1 {
                    row(ui, "", |ui| {
                        let text = format!("Asks for each disk in turn: {}", out.preview_names());
                        ui.label(RichText::new(text).small().color(theme::palette(ui).dim));
                    });
                }
            });
    }
}

const TYPE_TIP: &str = "The kind of file to make. A disk format picks one.";

const REPLACES_INPUT: &str = "This is the input file. Choose another type or name.";

const REPLACES_INPUTS: &str =
    "An image would replace its input. Choose another type, folder, prefix or suffix.";

/// The most disks one session reads.
pub const MAX_DISKS: u32 = 99;

/// gw's tracks when no format gives them: c=0-81:h=0-1.
pub const USUAL_DISK: (u32, u32) = (82, 2);

/// Why settings with every argument filled in still cannot run.
pub fn blocked(
    schema: &Schema,
    cmd: &Command,
    values: &Values,
    outputs: &BTreeMap<String, Output>,
    service: &Service,
) -> Option<&'static str> {
    if cmd.name == "update" {
        match Firmware::of(values) {
            Firmware::Release if !values.on("tag") => return Some("Type a release tag first."),
            Firmware::File if !values.on("file") => return Some("Choose an update file first."),
            _ => {}
        }
    }
    // gw would stop at the image before it opens the drive.
    if values.get("format").is_empty()
        && format_in_file(schema, cmd, values)
        && service.image_fault(input_file(cmd, values)).is_some()
    {
        return Some("gw cannot read this image. See Disk format.");
    }
    let batch = batch_input(cmd, values);
    if let Some(dest) = batch
        && values.get(dest).is_empty()
    {
        return Some(match values.get(BATCH_FOLDER) {
            "" => "Choose a folder of images first.",
            _ => "The folder has no images gw can read.",
        });
    }
    if let Some((_, dest)) = OUTPUTS.iter().find(|(c, _)| *c == cmd.name) {
        let out = outputs.get(&output_key(&cmd.name, dest));
        let Some(out) = out.filter(|o| !o.ext.is_empty()) else {
            return Some("Choose an image type first.");
        };
        if batch.is_none() && out.name.trim().is_empty() {
            return Some("Name the image first.");
        }
        if batch.is_some() {
            let files = service.known_folder(values.get(BATCH_FOLDER));
            let images = batch_images(schema, files, values.get(BATCH_TYPE));
            if images.iter().any(|i| out.batch_path(i) == *i) {
                return Some(REPLACES_INPUTS);
            }
        }
        let flux = flux_source(cmd, values);
        // Flux saved as flux needs no format. HFE holds bitcells, which gw
        // makes from flux only with a format or a bitrate.
        let bitcells = matches!(
            extension(input_file(cmd, values)).as_deref(),
            Some(".hfe" | ".ipf" | ".ctr")
        );
        let flux_out = FLUX.contains(&out.ext.as_str())
            && (out.ext != ".hfe" || out.opts.contains_key("bitrate") || bitcells);
        if !(flux && flux_out)
            && values.get("format").is_empty()
            && implied_format(schema, cmd, values, service).is_none()
        {
            return Some(if flux {
                "Choose a disk format first, or press Detect."
            } else {
                "Choose a disk format first."
            });
        }
    }
    // Paths, not strings, as in the page's warning: /a//b is /a/b.
    let input = input_file(cmd, values);
    let same = !input.is_empty() && Path::new(input) == Path::new(output_file(cmd, values));
    same.then_some(REPLACES_INPUT)
}

/// Where a new image is saved.
fn folder_row(ui: &mut Ui, folder: &mut String) {
    let (name, _) = row(ui, "Folder", |ui| {
        ui.horizontal(|ui| {
            let width = beside_button(ui, BROWSE_BUTTON);
            ui.add(edit(folder).desired_width(width))
                .on_hover_text("Where the image is saved.");
            if browse_button(ui)
                .on_hover_text("Choose a folder.")
                .clicked()
                && let Some(f) = rfd::FileDialog::new().set_directory(&*folder).pick_folder()
            {
                *folder = f.to_string_lossy().into_owned();
            }
        });
    });
    name.on_hover_text("Where the image is saved.");
}

/// The argument a page's folder of images goes to, one image a run, when
/// its batch is on.
pub fn batch_input(cmd: &Command, values: &Values) -> Option<&'static str> {
    let (_, dest) = BATCHES.iter().find(|(c, _)| *c == cmd.name)?;
    values.on(BATCH).then_some(*dest)
}

/// The images among `files` gw reads, or those of type `only`, in name order.
pub fn batch_images(schema: &Schema, files: &[PathBuf], only: &str) -> Vec<PathBuf> {
    let mut images: Vec<PathBuf> = files
        .iter()
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .filter(|p| {
            schema
                .image(&p.to_string_lossy())
                .is_some_and(|(e, _)| only.is_empty() || e == only)
        })
        .cloned()
        .collect();
    images.sort_by(|a, b| natural(&a.to_string_lossy(), &b.to_string_lossy()));
    images
}

/// Orders names with their numbers as numbers: Disk2 before Disk10.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::Equal;
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        let order = match (a.peek(), b.peek()) {
            (None, None) => return Equal,
            (x, y)
                if x.is_some_and(char::is_ascii_digit) && y.is_some_and(char::is_ascii_digit) =>
            {
                let number = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let digits: String =
                        std::iter::from_fn(|| it.next_if(char::is_ascii_digit)).collect();
                    digits.trim_start_matches('0').to_owned()
                };
                let (m, n) = (number(&mut a), number(&mut b));
                m.len().cmp(&n.len()).then_with(|| m.cmp(&n))
            }
            (x, y) => {
                let order = x
                    .map(|c| c.to_ascii_lowercase())
                    .cmp(&y.map(|c| c.to_ascii_lowercase()));
                a.next();
                b.next();
                order
            }
        };
        if order != Equal {
            return order;
        }
    }
}

/// "12 images: Disk1.scp, Disk2.scp … Disk12.scp"
fn listing(images: &[PathBuf]) -> String {
    let name = |p: &PathBuf| lossy(p.file_name());
    let names = match images {
        [a, b, .., z] if images.len() > 3 => format!("{}, {} … {}", name(a), name(b), name(z)),
        _ => images.iter().map(name).collect::<Vec<_>>().join(", "),
    };
    match images.len() {
        1 => format!("1 image: {names}"),
        n => format!("{n} images: {names}"),
    }
}

/// Chooses a disk format, and the image type that suits it.
pub fn choose_format(
    schema: &Schema,
    cmd: &Command,
    values: &mut Values,
    outputs: &mut BTreeMap<String, Output>,
    format: &str,
) {
    values.set("format", format);
    if format.is_empty() {
        return;
    }
    let ext = type_for(schema, format);
    for (_, dest) in OUTPUTS.iter().filter(|(c, _)| *c == cmd.name) {
        let out = outputs.entry(output_key(&cmd.name, dest)).or_default();
        if out.ext != ext {
            out.ext = ext.clone();
            out.opts.clear();
        }
    }
}

/// The disk definitions file a format needs: the one set if the format comes
/// from it, else none.
pub fn diskdefs_for(service: &mut Service, values: &Values, format: &str) -> String {
    let path = values.get("diskdefs");
    match service.custom_formats(path).iter().any(|f| f == format) {
        true => path.to_owned(),
        false => String::new(),
    }
}

/// Where an output argument's folder, name and type are kept.
pub fn output_key(command: &str, dest: &str) -> String {
    format!("{command}/{dest}")
}

/// The image file a command reads, if it reads one.
fn input_file<'v>(cmd: &Command, values: &'v Values) -> &'v str {
    let dest = match cmd.arg("in_file") {
        Some(_) => "in_file",
        None if OUTPUTS.contains(&(cmd.name.as_str(), "file")) => return "",
        None => "file",
    };
    split_opts(values.get(dest)).0
}

/// The image file a command writes, if it writes one.
fn output_file<'v>(cmd: &Command, values: &'v Values) -> &'v str {
    let output = OUTPUTS.iter().find(|(c, _)| *c == cmd.name);
    output.map_or("", |(_, dest)| split_opts(values.get(dest)).0)
}

/// Whether a command reads flux, from the drive or a flux image, so Detect
/// can find its format.
fn flux_source(cmd: &Command, values: &Values) -> bool {
    cmd.name == "read"
        || extension(input_file(cmd, values)).is_some_and(|e| FLUX.contains(&e.as_str()))
}

/// The format to decode with: the one chosen, or the one gw takes without.
/// Asks gw about an input it finds the format in, if it has not yet.
pub fn effective_format(
    service: &mut Service,
    schema: &Schema,
    cmd: &Command,
    values: &Values,
) -> Option<String> {
    match values.get("format") {
        "" => {
            if format_in_file(schema, cmd, values) {
                service.image_format(input_file(cmd, values));
            }
            implied_format(schema, cmd, values, service)
        }
        chosen => Some(chosen.to_owned()),
    }
}

/// The format gw takes when none is chosen, in gw's order: the input type's
/// own, such as an .adf's, else the output type's, else one gw has found in
/// the input file, such as an .nsi's.
fn implied_format(
    schema: &Schema,
    cmd: &Command,
    values: &Values,
    service: &Service,
) -> Option<String> {
    let path = input_file(cmd, values);
    let found = || match format_in_file(schema, cmd, values) {
        true => service.known_image_format(path).map(str::to_owned),
        false => None,
    };
    own_format(schema, path)
        .or_else(|| own_format(schema, output_file(cmd, values)))
        .or_else(found)
}

/// An image type's own format, such as an .adf's.
fn own_format(schema: &Schema, path: &str) -> Option<String> {
    schema.image(path)?.1.default_format.clone()
}

/// Whether gw looks in the input file for its format, as in an .nsi. The
/// output type's own format, such as an .adf's, comes first.
fn format_in_file(schema: &Schema, cmd: &Command, values: &Values) -> bool {
    let finds = schema
        .image(input_file(cmd, values))
        .is_some_and(|(_, i)| i.finds_format);
    finds && own_format(schema, output_file(cmd, values)).is_none()
}

/// A tooltip of a widget's own, in a row that has one: the row's then stays hidden.
trait OwnTip {
    fn own_tip(self, text: &str) -> Self;
}

impl OwnTip for egui::Response {
    fn own_tip(self, text: &str) -> Self {
        if self.contains_pointer() {
            self.ctx.data_mut(|d| d.insert_temp(own_tip_id(), true));
        }
        self.on_hover_text(text)
    }
}

fn own_tip_id() -> egui::Id {
    egui::Id::new("own-tip")
}

/// The arguments shown first, and the rest.
pub fn sections(cmd: &Command) -> (Vec<&Arg>, Vec<&Arg>) {
    let shown = |a: &&Arg| {
        !GLOBAL.contains(&a.dest.as_str())
            && !(a.dest == "no_clobber" && OUTPUTS.iter().any(|(c, _)| *c == cmd.name))
    };
    let args: Vec<&Arg> = cmd.args.iter().filter(shown).collect();
    match FIRST.iter().find(|(c, _)| *c == cmd.name) {
        Some((_, names)) => {
            let first = names
                .iter()
                .filter_map(|n| args.iter().find(|a| a.dest == *n).copied())
                .collect();
            let rest = args
                .into_iter()
                .filter(|a| !names.contains(&a.dest.as_str()))
                .collect();
            (first, rest)
        }
        None if args.len() <= 6 => (args, Vec::new()),
        None => args.into_iter().partition(|a| a.positional() || a.required),
    }
}

pub fn label(a: &Arg) -> String {
    plain(LABELS, &a.dest)
}

/// `name`'s entry in `names`, else `name` in sentence case with spaces for
/// underscores.
fn plain(names: &[(&str, &str)], name: &str) -> String {
    match names.iter().find(|(n, _)| *n == name) {
        Some((_, shown)) => (*shown).to_owned(),
        None => sentence_case(&name.replace('_', " ")),
    }
}

/// What an argument does, in one short sentence: from TIPS, else gw's help.
pub fn tip(command: &str, a: &Arg) -> String {
    let own = TIPS
        .iter()
        .find(|(c, d, _)| *c == command && *d == a.dest)
        .or_else(|| TIPS.iter().find(|(c, d, _)| c.is_empty() && *d == a.dest));
    match own {
        Some((_, _, tip)) => (*tip).to_owned(),
        None if a.help.is_empty() => label(a) + ".",
        None => sentence(&a.help),
    }
}

/// Tooltips by command and argument; an empty command is any command.
const TIPS: &[(&str, &str, &str)] = &[
    (
        "",
        "adjust_speed",
        "Scale the track data to this drive speed.",
    ),
    (
        "info",
        "bootloader",
        "Show the bootloader's details. F7 only.",
    ),
    (
        "update",
        "bootloader",
        "Update the bootloader. Use with caution.",
    ),
    ("seek", "cylinder", "The cylinder to move the heads to."),
    ("clean", "cyls", "How many cylinders the drive has."),
    ("reset", "delays", "Reset the delays as well."),
    ("", "densel", "Set the density select signal on pin 2."),
    (
        "",
        "diskdefs",
        "A file of disk formats to use instead of gw's own.",
    ),
    ("", "drive", "The drive, by bus unit."),
    ("", "device", "The Greaseweazle's port."),
    (
        "write",
        "erase_empty",
        "Erase tracks the image leaves empty. Otherwise they are skipped.",
    ),
    (
        "",
        "fake_index",
        "Fake index pulses at this rotation speed.",
    ),
    ("read", "file", "The image to make."),
    ("write", "file", "The image to write."),
    ("update", "file", "The update file to install."),
    ("seek", "force", "Allow extreme cylinders without asking."),
    ("update", "force", "Update even if the firmware is older."),
    (
        "read",
        "format",
        "The disk's format. The image is decoded with it unless Raw is on.",
    ),
    ("", "format", "The disk's format."),
    (
        "read",
        "gen_tg43",
        "Make the TG43 signal on pin 2 from track 60, for 8-inch drives.",
    ),
    (
        "",
        "gen_tg43",
        "Make the TG43 signal on pin 2, for 8-inch drives.",
    ),
    ("read", "hard_sectors", "Read a hard-sectored disk."),
    ("write", "hard_sectors", "Write a hard-sectored disk."),
    (
        "convert",
        "hard_sectors",
        "Turn index positions into hard sectors.",
    ),
    (
        "erase",
        "hfreq",
        "Erase by writing a high-frequency signal.",
    ),
    ("convert", "in_file", "The image to convert."),
    ("delays", "index_mask", "Index mask, in microseconds."),
    ("pin set", "level", "High or low."),
    ("clean", "linger", "Time on each step, in milliseconds."),
    ("delays", "motor", "Motor delay, in milliseconds."),
    ("seek", "motor_on", "Run the motor while seeking."),
    ("", "no_clobber", "Keep an existing file."),
    (
        "write",
        "no_verify",
        "Do not read each track back to check it.",
    ),
    ("rpm", "nr", "How many times to measure."),
    ("convert", "out_file", "The image to make."),
    (
        "convert",
        "out_tracks",
        "Which tracks to write out. Unset, the tracks read.",
    ),
    ("clean", "passes", "Passes across the cleaning disk."),
    ("", "pin", "The pin number."),
    ("", "pll", "Your own PLL settings for decoding flux."),
    ("delays", "post_write", "Post-write delay, in microseconds."),
    ("write", "pre_erase", "Erase each track before writing it."),
    ("delays", "pre_write", "Pre-write delay, in microseconds."),
    ("write", "precomp", "Write precompensation, by cylinder."),
    (
        "read",
        "raw",
        "Save the flux as read. A format only verifies it.",
    ),
    (
        "read",
        "retries",
        "Rereads of a track with missing sectors, before each seek retry.",
    ),
    (
        "write",
        "retries",
        "Retries of a track that fails to verify.",
    ),
    (
        "",
        "reverse",
        "Reverse the track data, for a flippy disk's other side.",
    ),
    ("read", "revs", "Revolutions to read per track."),
    ("erase", "revs", "Revolutions to erase per track."),
    (
        "read",
        "seek_retries",
        "Times to seek to cylinder 0 and back when a track reads short.",
    ),
    ("delays", "select", "Select delay, in microseconds."),
    ("delays", "settle", "Settle time, in milliseconds."),
    ("delays", "step", "Step delay, in microseconds."),
    ("update", "tag", "The GitHub release tag to update to."),
    ("read", "tracks", "Which tracks to read."),
    ("write", "tracks", "Which tracks to write."),
    ("convert", "tracks", "Which tracks to read and convert."),
    ("erase", "tracks", "Which tracks to erase."),
    ("delays", "watchdog", "Watchdog, in milliseconds."),
];

/// gw's own example for a kind of value, such as `e.g. c=0-7,9-12:h=0-1` for TSPEC.
fn example(schema: &Schema, metavar: &str) -> Option<String> {
    let line = schema
        .note(metavar)?
        .lines()
        .find(|l| l.contains("e.g. '"))?;
    let quoted = line.split('\'').nth(1)?;
    Some(format!("e.g. {quoted}"))
}

fn hint(a: &Arg, schema: &Schema) -> String {
    match a.ty.as_deref() {
        Some("period") => "e.g. 300rpm".into(),
        Some("PLL") => "e.g. period=5:phase=60".into(),
        Some("PrecompSpec") => example(schema, "PRECOMP").unwrap_or_default(),
        _ => match &a.default {
            Some(d) => format!("e.g. {d}"),
            None if a.required => "Required".into(),
            // As the firmware's releases are tagged.
            None if a.dest == "tag" => "e.g. v1.6".into(),
            None => String::new(),
        },
    }
}

/// gw's help text, which is lower case with no full stop, as a sentence.
pub fn sentence(text: &str) -> String {
    let mut s = sentence_case(text.trim());
    if !s.ends_with(['.', '?', '!']) {
        s.push('.');
    }
    s
}

fn sentence_case(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A family's name from any of its formats: `akai.800` is Akai.
pub fn family_name(format: &str) -> String {
    let family = format.split('.').next().unwrap_or(format);
    match FAMILIES.iter().find(|(f, _)| *f == family) {
        Some((_, name)) => (*name).to_owned(),
        None => sentence_case(family),
    }
}

/// `Akai · akai.800`.
fn format_name(format: &str) -> String {
    format!("{} · {format}", family_name(format))
}

/// The image type that suits a format: the one gw pairs with it (.adf for
/// Amiga, .d64 for the C64), else one for its family, else a sector image.
pub fn type_for(schema: &Schema, format: &str) -> String {
    let writable = |e: &str| schema.images.get(e).is_some_and(|i| i.writable);
    // gw pairs ibm.800 with .mgt, which is SAM Coupé's; PC disks want .img.
    let paired = schema.images.iter().find(|(e, i)| {
        !format.starts_with("ibm.")
            && i.writable
            && i.default_format.as_deref() == Some(format)
            && !e.is_empty()
    });
    let by_family = TYPES
        .iter()
        .find(|(p, e)| format.starts_with(p) && writable(e))
        .map(|(_, e)| *e);
    match (paired, by_family) {
        (Some((ext, _)), _) => ext.clone(),
        (None, Some(ext)) => ext.to_owned(),
        (None, None) => ".img".to_owned(),
    }
}

fn describe(info: &FormatInfo) -> String {
    let mut parts = Vec::new();
    parts.extend(info.encoding.clone());
    parts.push(format!("{} cylinders", info.cyls));
    parts.push(if info.heads == 1 {
        "1 side".into()
    } else {
        format!("{} sides", info.heads)
    });
    parts.extend(info.sectors.map(|s| format!("{s} sectors")));
    parts.extend(info.bytes.map(|b| format!("{} KB", b / 1024)));
    // A narrow field wraps between facts, never inside one: "1440 KB" stays whole.
    let whole: Vec<String> = parts.iter().map(|p| p.replace(' ', "\u{a0}")).collect();
    whole.join(" · ")
}

/// Plain names for image options, where gw's own would not read well.
const OPTION_NAMES: &[(&str, &str)] = &[
    ("disktype", "Disk type"),
    ("double_step", "Double step"),
    ("legacy_ss", "Legacy single-sided"),
    ("revs", "Revolutions"),
    ("sck", "Sample clock"),
];

/// `SuperCard Pro flux (.scp)` for a path or a bare suffix.
fn image_name(path: &str, name: &str) -> String {
    let Some(ext) = extension(&format!("x{path}")) else {
        return name.to_owned();
    };
    match KNOWN_IMAGES.iter().find(|(e, _)| *e == ext) {
        Some((e, what)) => format!("{what} ({e})"),
        None => format!("{name} ({ext})"),
    }
}

/// Plain names for the image types people meet most.
const KNOWN_IMAGES: &[(&str, &str)] = &[
    (".adf", "Amiga disk"),
    (".d64", "Commodore 1541"),
    (".d71", "Commodore 1571"),
    (".d81", "Commodore 1581"),
    (".dsk", "DSK"),
    (".edsk", "Extended DSK"),
    (".hfe", "HxC floppy emulator"),
    (".ima", "Sector image"),
    (".img", "Sector image"),
    (".imd", "ImageDisk"),
    (".ipf", "SPS IPF"),
    (".raw", "KryoFlux stream"),
    (".scp", "SuperCard Pro flux"),
    (".st", "Atari ST"),
    (".td0", "Teledisk"),
];

/// Splits gw's `path::name=value:flag` into the path and its options.
fn split_opts(value: &str) -> (&str, BTreeMap<String, String>) {
    let mut parts = value.split("::");
    let path = parts.next().unwrap_or_default();
    let mut opts = BTreeMap::new();
    for opt in parts.flat_map(|p| p.split(':')).filter(|o| !o.is_empty()) {
        let (k, v) = opt.split_once('=').unwrap_or((opt, ON));
        opts.insert(k.to_owned(), v.to_owned());
    }
    (path, opts)
}

fn join_opts(path: &str, opts: &BTreeMap<String, String>) -> String {
    let set: Vec<String> = opts
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| {
            if v == ON {
                k.clone()
            } else {
                format!("{k}={v}")
            }
        })
        .collect();
    if set.is_empty() {
        path.to_owned()
    } else {
        format!("{path}::{}", set.join(":"))
    }
}

/// An open dialog for a file argument, showing only its kind of file where
/// gw takes one kind.
fn open_dialog(command: &str, dest: &str) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new();
    match FILE_TYPES
        .iter()
        .find(|(c, d, _, _)| *c == command && *d == dest)
    {
        Some((_, _, name, exts)) => dialog.add_filter(*name, exts),
        None => dialog,
    }
}

fn lossy(s: Option<&OsStr>) -> String {
    s.map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// Fields for an image type's own options. Returns true if one changed.
fn image_options(ui: &mut Ui, options: &[ImageOpt], values: &mut BTreeMap<String, String>) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for opt in options {
            let name = plain(OPTION_NAMES, &opt.name);
            let value = values.entry(opt.name.clone()).or_default();
            if opt.flag() {
                let mut on = !value.is_empty();
                if checkbox(ui, &mut on, &name).changed() {
                    *value = if on { ON.to_owned() } else { String::new() };
                    changed = true;
                }
            } else {
                ui.label(RichText::new(name).small());
                let hint = opt
                    .default
                    .as_ref()
                    .filter(|d| !d.is_null())
                    .map(|d| d.to_string())
                    .unwrap_or_default();
                changed |= ui
                    .add(edit(value).hint_text(hint).desired_width(SHORT_FIELD / 2.0))
                    .changed();
            }
        }
    });
    values.retain(|_, v| !v.is_empty());
    changed
}

/// Documents/Ferriteweazle/Images: where new images go unless Settings says otherwise.
pub fn images_folder() -> PathBuf {
    crate::app_folder().join("Images")
}

/// Where a new image goes, and what it is called. An empty `ext` is no
/// type chosen yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Output {
    pub folder: String,
    pub name: String,
    /// Disks read one after another, each into its own numbered file.
    pub disks: u32,
    /// The word before each disk number: `Disk` in `Game_Disk1`.
    pub label: String,
    /// The disk number goes before the name, not after it.
    pub number_first: bool,
    pub ext: String,
    pub opts: BTreeMap<String, String>,
    /// Take the folder and name from the input file, when there is one.
    pub beside_input: bool,
    /// Around each input's name in a batch: `Backup_Disk1_copy`.
    pub prefix: String,
    pub suffix: String,
}

impl Default for Output {
    fn default() -> Self {
        Output {
            folder: images_folder().to_string_lossy().into_owned(),
            name: "Floppy".into(),
            disks: 1,
            label: "Disk".into(),
            number_first: false,
            ext: String::new(),
            opts: BTreeMap::new(),
            beside_input: false,
            prefix: String::new(),
            suffix: String::new(),
        }
    }
}

impl Output {
    /// An output from a path as gw takes it, such as a pasted command's.
    pub fn from_value(value: &str) -> Output {
        let (path, opts) = split_opts(value);
        let path = Path::new(path);
        Output {
            folder: lossy(path.parent().map(Path::as_os_str)),
            name: lossy(path.file_stem()),
            ext: path.extension().map_or_else(String::new, |e| {
                format!(".{}", e.to_string_lossy().to_lowercase())
            }),
            opts,
            ..Output::default()
        }
    }

    /// The file for one disk, counting from 1: `Game_Disk2.adf` of three.
    pub fn file_name(&self, disk: u32) -> String {
        let (name, ext) = (&self.name, &self.ext);
        if self.disks <= 1 {
            return format!("{name}{ext}");
        }
        let width = self.disks.to_string().len();
        let number = format!("{}{disk:0width$}", self.label.trim());
        match self.number_first {
            true => format!("{number}_{name}{ext}"),
            false => format!("{name}_{number}{ext}"),
        }
    }

    pub fn path(&self, disk: u32) -> PathBuf {
        PathBuf::from(&self.folder).join(self.file_name(disk))
    }

    /// Every file a session makes, in order.
    pub fn paths(&self) -> impl Iterator<Item = PathBuf> + '_ {
        (1..=self.disks.max(1)).map(|d| self.path(d))
    }

    /// The value gw takes for one disk: the path and any image options.
    /// Empty until there is a type and a name.
    pub fn value(&self, disk: u32) -> String {
        if self.ext.is_empty() || self.name.trim().is_empty() {
            return String::new();
        }
        join_opts(&self.path(disk).to_string_lossy(), &self.opts)
    }

    /// The file a batch makes from `input`: its name between the prefix and
    /// the suffix, in the input's folder when beside it.
    pub fn batch_path(&self, input: &Path) -> PathBuf {
        let folder = match self.beside_input {
            true => input.parent().unwrap_or(Path::new("")),
            false => Path::new(&self.folder),
        };
        let (prefix, suffix) = (self.prefix.trim(), self.suffix.trim());
        let stem = lossy(input.file_stem());
        folder.join(format!("{prefix}{stem}{suffix}{}", self.ext))
    }

    /// As `value`, for the image a batch makes from `input`.
    pub fn batch_value(&self, input: &Path) -> String {
        match self.ext.is_empty() {
            true => String::new(),
            false => join_opts(&self.batch_path(input).to_string_lossy(), &self.opts),
        }
    }

    /// The first file a batch makes, and the last.
    fn batch_preview(&self, images: &[PathBuf]) -> String {
        let path = |i: &PathBuf| self.batch_path(i);
        match images {
            [] => String::new(),
            [one] => path(one).to_string_lossy().into_owned(),
            [first, .., last] => format!(
                "{} … {}",
                path(first).to_string_lossy(),
                lossy(path(last).file_name())
            ),
        }
    }

    /// The first file, and the last when there are several.
    fn preview(&self) -> String {
        let first = self.path(1).to_string_lossy().into_owned();
        match self.disks {
            0 | 1 => first,
            n => format!("{first} … {}", self.file_name(n)),
        }
    }

    fn preview_names(&self) -> String {
        match self.disks {
            0..=3 => {
                let names: Vec<String> = (1..=self.disks).map(|d| self.file_name(d)).collect();
                names.join(", ")
            }
            n => format!(
                "{}, {} … {}",
                self.file_name(1),
                self.file_name(2),
                self.file_name(n)
            ),
        }
    }
}

/// A track list with double step added, unless it names a step already.
pub fn double_step(tracks: &str) -> String {
    let mut spec = TrackSpec::parse(tracks);
    spec.step.get_or_insert_with(|| "2".into());
    spec.to_string()
}

/// Which tracks, in gw's notation: `c=0-79:h=0:step=2:hswap`.
#[derive(Debug, Default, Clone, PartialEq)]
struct TrackSpec {
    c: Option<String>,
    h: Option<String>,
    step: Option<String>,
    hswap: bool,
    /// Parts the picker does not show, such as head offsets.
    other: Vec<String>,
}

impl TrackSpec {
    fn parse(s: &str) -> TrackSpec {
        let mut spec = TrackSpec::default();
        for part in s.split(':').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some(("c", v)) => spec.c = Some(v.to_owned()),
                Some(("h", v)) => spec.h = Some(v.to_owned()),
                Some(("step", v)) => spec.step = Some(v.to_owned()),
                None if part == "hswap" => spec.hswap = true,
                _ => spec.other.push(part.to_owned()),
            }
        }
        spec
    }

    /// True if the picker can show all of it.
    fn simple(&self) -> bool {
        self.other.is_empty()
            && (self.c.is_none() || self.cylinders().is_some())
            && matches!(self.h.as_deref(), None | Some("0" | "1" | "0-1" | "0,1"))
            && matches!(self.step.as_deref(), None | Some("1" | "2"))
    }

    fn cylinders(&self) -> Option<(u32, u32)> {
        let c = self.c.as_deref()?;
        let (a, b) = c.split_once('-').unwrap_or((c, c));
        Some((a.parse().ok()?, b.parse().ok()?))
    }

    fn has_head(&self, head: u32, heads: u32) -> bool {
        match self.h.as_deref() {
            Some("0") => head == 0,
            Some("1") => head == 1,
            Some("0-1" | "0,1") => head < 2,
            _ => head < heads,
        }
    }

    /// False where no click changes the sides: a one-sided format with side 0 alone.
    fn sides_can_change(&self, heads: u32) -> bool {
        heads > 1 || self.has_head(1, heads)
    }

    /// Turns a side off, or back on. At least one side stays on.
    fn toggle_head(&mut self, head: u32, heads: u32) {
        let mut on = [self.has_head(0, heads), self.has_head(1, heads)];
        on[head as usize] = !on[head as usize];
        self.h = match on {
            [true, false] if heads > 1 => Some("0".into()),
            [false, true] => Some("1".into()),
            _ => None,
        };
    }
}

impl std::fmt::Display for TrackSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        parts.extend(self.c.as_ref().map(|c| format!("c={c}")));
        parts.extend(self.h.as_ref().map(|h| format!("h={h}")));
        parts.extend(self.step.as_ref().map(|s| format!("step={s}")));
        if self.hswap {
            parts.push("hswap".into());
        }
        parts.extend(self.other.iter().cloned());
        f.write_str(&parts.join(":"))
    }
}

/// A checkbox with a clean tick, drawn here: egui's own is lopsided.
pub fn checkbox(ui: &mut Ui, on: &mut bool, text: &str) -> egui::Response {
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let size = vec2(
        CHECK + CHECK_GAP + galley.size().x,
        CHECK.max(galley.size().y),
    );
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let (enabled, state) = (ui.is_enabled(), *on);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, state, text)
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let p = theme::palette(ui);
    let square = egui::Rect::from_min_size(
        pos2(rect.left(), rect.center().y - CHECK / 2.0),
        vec2(CHECK, CHECK),
    );
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let painter = ui.painter();
    let round = CornerRadius::same(4);
    if *on {
        painter.rect_filled(square, round, fade(p.accent));
        // A short stroke down to the left, a long one up to the right.
        let at = |x: f32, y: f32| square.min + square.size() * vec2(x, y);
        let tick = vec![at(0.24, 0.52), at(0.42, 0.70), at(0.77, 0.32)];
        let stroke = egui::Stroke::new(2.0, fade(p.on_accent));
        painter.add(egui::Shape::line(tick, stroke));
    } else {
        let edge = if response.hovered() {
            p.dim
        } else {
            p.line_strong
        };
        painter.rect(
            square,
            round,
            p.card,
            egui::Stroke::new(1.0, fade(edge)),
            egui::StrokeKind::Inside,
        );
    }
    let text_at = pos2(
        square.right() + CHECK_GAP,
        rect.center().y - galley.size().y / 2.0,
    );
    painter.galley(text_at, galley, fade(ui.visuals().text_color()));
    response
}

/// A checkbox's square, and the gap before its text.
const CHECK: f32 = 16.0;
const CHECK_GAP: f32 = 7.0;

/// An on/off switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let (enabled, state) = (ui.is_enabled(), *on);
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, state, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, state);
        let p = theme::palette(ui);
        let fill = theme::lerp(p.line_strong, p.accent, t);
        ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
        let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
        ui.painter()
            .circle_filled(pos2(x, rect.center().y), 7.5, Color32::WHITE);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    fn schema() -> Schema {
        serde_json::from_str(include_str!("../tests/data/schema-1.23.json")).unwrap()
    }

    fn values(pairs: &[(&str, &str)]) -> Values {
        let mut v = Values::default();
        pairs.iter().for_each(|(k, x)| v.set(k, *x));
        v
    }

    /// An output of type `ext` in /f, named Floppy.
    fn output(ext: &str) -> Output {
        Output {
            folder: "/f".into(),
            ext: ext.into(),
            ..Output::default()
        }
    }

    type Page = (Values, BTreeMap<String, Output>);

    /// A command's form alone, drawn until it settles.
    fn page(
        command: &str,
        values: Values,
        outputs: BTreeMap<String, Output>,
    ) -> Harness<'static, Page> {
        let schema = schema();
        let cmd = schema.command(command).unwrap().clone();
        let mut service = Service::offline(Ok(schema.clone()));
        let mut h = Harness::new_ui_state(
            move |ui, (values, outputs): &mut Page| {
                let form = Form {
                    schema: &schema,
                    cmd: &cmd,
                    values,
                    outputs,
                    service: &mut service,
                    cannot_detect: None,
                };
                form.show(ui);
            },
            (values, outputs),
        );
        h.run();
        h
    }

    #[test]
    fn with_no_format_chosen_gw_takes_the_input_types_own_then_the_output_types() {
        let s = schema();
        let mut service = Service::offline(Ok(s.clone()));
        let mut implied = |command: &str, pairs: &[(&str, &str)]| {
            effective_format(
                &mut service,
                &s,
                s.command(command).unwrap(),
                &values(pairs),
            )
        };
        let read = |file| [("file", file)];
        assert_eq!(
            implied("read", &read("/f/x.adf")).as_deref(),
            Some("amiga.amigados")
        );
        assert_eq!(implied("read", &read("/f/x.scp")), None);
        let convert = |from, to| [("in_file", from), ("out_file", to)];
        assert_eq!(
            implied("convert", &convert("/f/x.scp", "/f/x.d64")).as_deref(),
            Some("commodore.1541")
        );
        assert_eq!(
            implied("convert", &convert("/f/x.adf", "/f/x.d64")).as_deref(),
            Some("amiga.amigados"),
            "the input's own comes first"
        );

        let outputs = BTreeMap::from([(output_key("read", "file"), output(".adf"))]);
        let v = values(&[("file", &outputs["read/file"].value(1))]);
        let read = s.command("read").unwrap();
        assert_eq!(blocked(&s, read, &v, &outputs, &service), None);
        let h = page("read", Values::default(), outputs);
        let format = h.get_all_by_role(Role::ComboBox).next().unwrap().value();
        assert_eq!(
            format.as_deref(),
            Some("Amiga · amiga.amigados (from the image type)")
        );
    }

    #[test]
    fn hfe_made_from_flux_needs_a_format_or_a_bitrate() {
        let s = schema();
        let service = Service::offline(Ok(s.clone()));
        let why = |command: &str, input: &str, out: Output| {
            let (_, dest) = OUTPUTS.iter().find(|(c, _)| *c == command).unwrap();
            let mut v = values(&[(dest, &out.value(1))]);
            if command == "convert" {
                v.set("in_file", input);
            }
            let outputs = BTreeMap::from([(output_key(command, dest), out)]);
            blocked(&s, s.command(command).unwrap(), &v, &outputs, &service)
        };
        let mut bitrate = output(".hfe");
        bitrate.opts.insert("bitrate".into(), "250".into());
        let needs = Some("Choose a disk format first, or press Detect.");
        assert_eq!(why("read", "", output(".hfe")), needs);
        assert_eq!(why("read", "", bitrate), None);
        assert_eq!(why("read", "", output(".scp")), None, "flux kept as flux");
        assert_eq!(why("convert", "/f/x.scp", output(".hfe")), needs);
        assert_eq!(
            why("convert", "/f/x.ipf", output(".hfe")),
            None,
            "an IPF's tracks have a bitrate"
        );
    }

    #[test]
    fn a_page_that_makes_an_image_says_whether_it_lacks_the_type_or_the_name() {
        let s = schema();
        let read = s.command("read").unwrap();
        let service = Service::offline(Ok(s.clone()));
        let v = values(&[("format", "ibm.1440")]);
        let mut outputs = BTreeMap::new();
        let why = |outputs: &_| blocked(&s, read, &v, outputs, &service);
        assert_eq!(why(&outputs), Some("Choose an image type first."));
        let unnamed = Output {
            name: " ".into(),
            ..output(".img")
        };
        outputs.insert(output_key("read", "file"), unnamed);
        assert_eq!(why(&outputs), Some("Name the image first."));
    }

    #[test]
    fn an_output_that_is_the_input_cannot_run_however_its_folder_is_typed() {
        let s = schema();
        let service = Service::offline(Ok(s.clone()));
        let out = Output {
            folder: "/f//g".into(),
            name: "x".into(),
            ..output(".img")
        };
        let v = values(&[
            ("in_file", "/f/g/x.img"),
            ("out_file", &out.value(1)),
            ("format", "ibm.1440"),
        ]);
        let outputs = BTreeMap::from([(output_key("convert", "out_file"), out)]);
        let convert = s.command("convert").unwrap();
        assert_eq!(
            blocked(&s, convert, &v, &outputs, &service),
            Some(REPLACES_INPUT)
        );
    }

    #[test]
    fn with_no_format_the_track_picker_offers_the_82_cylinders_gw_uses() {
        let h = page("erase", Values::default(), BTreeMap::new());
        let cylinders: Vec<_> = h
            .get_all_by_role(Role::SpinButton)
            .map(|c| c.accesskit_node().numeric_value())
            .collect();
        assert_eq!(cylinders, [Some(0.0), Some(81.0)]);
    }

    #[test]
    fn a_batch_takes_the_images_gw_reads_in_name_order() {
        let s = schema();
        let names = [
            "Disk10.adf",
            "Disk2.adf",
            "disk1.ADF",
            ".Disk0.adf",
            "notes.txt",
            "Disk3.scp",
        ];
        let files: Vec<PathBuf> = names.iter().map(|n| Path::new("/f").join(n)).collect();
        let taken = |only| -> Vec<String> {
            let images = batch_images(&s, &files, only);
            images.iter().map(|p| lossy(p.file_name())).collect()
        };
        assert_eq!(
            taken(""),
            ["disk1.ADF", "Disk2.adf", "Disk3.scp", "Disk10.adf"]
        );
        assert_eq!(taken(".adf"), ["disk1.ADF", "Disk2.adf", "Disk10.adf"]);
        let images = batch_images(&s, &files, "");
        assert_eq!(
            listing(&images),
            "4 images: disk1.ADF, Disk2.adf … Disk10.adf"
        );
        assert_eq!(listing(&images[..1]), "1 image: disk1.ADF");
    }

    #[test]
    fn a_batch_names_each_image_after_its_input() {
        let mut out = Output {
            folder: "/out".into(),
            ext: ".hfe".into(),
            ..Output::default()
        };
        let input = Path::new("/in/Game_Disk1.scp");
        assert_eq!(
            out.batch_path(input),
            Path::new("/out").join("Game_Disk1.hfe")
        );
        out.prefix = "Backup_".into();
        out.suffix = " _copy ".into();
        let named = "Backup_Game_Disk1_copy.hfe";
        assert_eq!(out.batch_path(input), Path::new("/out").join(named));
        out.beside_input = true;
        assert_eq!(out.batch_path(input), Path::new("/in").join(named));
    }

    #[test]
    fn a_batch_needs_a_folder_of_images_and_outputs_that_spare_them() {
        let s = schema();
        let convert = s.command("convert").unwrap();
        let mut service = Service::offline(Ok(s.clone()));
        let mut v = Values::default();
        v.set(BATCH, ON);
        let none = BTreeMap::new();
        let reason = |v: &Values, service: &Service, outputs: &BTreeMap<String, Output>| {
            blocked(&s, convert, v, outputs, service)
        };
        assert_eq!(
            reason(&v, &service, &none),
            Some("Choose a folder of images first.")
        );
        let dir = std::env::temp_dir().join(format!("ferriteweazle-batch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        v.set(BATCH_FOLDER, dir.to_string_lossy());
        service.folder(&dir.to_string_lossy());
        assert_eq!(
            reason(&v, &service, &none),
            Some("The folder has no images gw can read.")
        );

        std::fs::write(dir.join("Game.img"), [0u8; 512]).unwrap();
        service.folder(&dir.to_string_lossy());
        v.set("in_file", dir.join("Game.img").to_string_lossy());
        v.set("format", "ibm.360");
        let out = Output {
            folder: dir.to_string_lossy().into_owned(),
            ext: ".img".into(),
            ..Output::default()
        };
        let mut outputs = BTreeMap::from([(output_key("convert", "out_file"), out)]);
        assert_eq!(reason(&v, &service, &outputs), Some(REPLACES_INPUTS));
        outputs.get_mut("convert/out_file").unwrap().suffix = "_copy".into();
        assert_eq!(reason(&v, &service, &outputs), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_argument_of_every_command_has_a_place() {
        let s = schema();
        for cmd in &s.commands {
            let (first, rest) = sections(cmd);
            let shown: Vec<&str> = first.iter().chain(&rest).map(|a| a.dest.as_str()).collect();
            for a in &cmd.args {
                let elsewhere = GLOBAL.contains(&a.dest.as_str()) || a.dest == "no_clobber";
                assert!(
                    elsewhere || shown.contains(&a.dest.as_str()),
                    "gw {} {} has no field",
                    cmd.name,
                    a.dest
                );
            }
        }
    }

    #[test]
    fn hand_picked_arguments_still_exist_in_gw() {
        let s = schema();
        let outputs = OUTPUTS.iter().map(|(c, d)| (*c, std::slice::from_ref(d)));
        for (cmd, names) in FIRST.iter().copied().chain(outputs) {
            // A command from a newer gw may be missing from this one.
            let Some(cmd) = s.command(cmd) else { continue };
            for n in names {
                assert!(cmd.arg(n).is_some(), "gw {} has no {n}", cmd.name);
            }
        }
        for (dest, _) in LABELS {
            assert!(
                s.commands.iter().any(|c| c.arg(dest).is_some()),
                "no gw argument is called {dest}"
            );
        }
        assert!(
            s.commands.iter().all(|c| c.arg(FIRMWARE).is_none()),
            "the page's own {FIRMWARE} is a gw argument"
        );
    }

    #[test]
    fn update_waits_for_the_tag_or_file_its_source_needs_and_gives_gw_only_that() {
        let s = schema();
        let update = s.command("update").unwrap();
        let service = Service::offline(Err(String::new()));
        let outputs = BTreeMap::new();
        let why = |v: &Values| blocked(&s, update, v, &outputs, &service);
        let mut v = Values::default();
        assert_eq!(why(&v), None, "the latest needs nothing");
        v.set(FIRMWARE, "Release");
        assert_eq!(why(&v), Some("Type a release tag first."));
        v.set("tag", "v1.6");
        assert_eq!(why(&v), None);
        v.set(FIRMWARE, "File");
        assert_eq!(why(&v), Some("Choose an update file first."));
        v.set("file", "fw.upd");
        assert_eq!(why(&v), None);
        Firmware::only(&mut v);
        assert_eq!((v.get("tag"), v.get("file")), ("", "fw.upd"));
    }

    #[test]
    fn abbreviated_names_get_plain_labels() {
        let s = schema();
        let shown = |c: &str, d: &str| label(s.command(c).unwrap().arg(d).unwrap());
        assert_eq!(shown("rpm", "nr"), "Measurements");
        assert_eq!(shown("clean", "cyls"), "Cylinders");
        assert_eq!(shown("clean", "linger"), "Time per step");
        assert_eq!(shown("erase", "hfreq"), "High frequency");
        assert_eq!(shown("delays", "select"), "Select delay");
        assert_eq!(shown("delays", "pre_write"), "Pre-write");
        assert_eq!(
            shown("delays", "watchdog"),
            "Watchdog",
            "a plain name stays"
        );
    }

    #[test]
    fn track_specs_keep_what_the_picker_does_not_show() {
        let spec = TrackSpec::parse("c=0-39:h=1:step=2:hswap:h1.off=+1");
        assert_eq!(spec.cylinders(), Some((0, 39)));
        assert!(!spec.simple());
        assert_eq!(spec.to_string(), "c=0-39:h=1:step=2:hswap:h1.off=+1");
    }

    #[test]
    fn double_step_joins_a_track_list_but_keeps_a_step_already_there() {
        assert_eq!(double_step(""), "step=2");
        assert_eq!(double_step("c=0-39:h=0"), "c=0-39:h=0:step=2");
        assert_eq!(double_step("step=1"), "step=1");
    }

    #[test]
    fn turning_sides_off_leaves_at_least_one() {
        let mut spec = TrackSpec::default();
        spec.toggle_head(1, 2);
        assert_eq!(spec.to_string(), "h=0");
        spec.toggle_head(0, 2);
        assert_eq!(
            spec.to_string(),
            "",
            "turning the last side off turns both back on"
        );
        spec.toggle_head(0, 2);
        assert_eq!(spec.to_string(), "h=1");
    }

    #[test]
    fn the_sides_lit_are_those_gw_reads_and_grey_when_no_click_changes_them() {
        for heads in [1, 2] {
            for h in [None, Some("0"), Some("1"), Some("0-1"), Some("0,1")] {
                let spec = TrackSpec {
                    h: h.map(Into::into),
                    ..TrackSpec::default()
                };
                let sides = |s: &TrackSpec| [s.has_head(0, heads), s.has_head(1, heads)];
                // A list naming sides overrides the format's, in gw's TrackSet.
                let read = match h {
                    None => [true, heads > 1],
                    Some("0") => [true, false],
                    Some("1") => [false, true],
                    _ => [true, true],
                };
                assert_eq!(sides(&spec), read, "{heads} heads, h={h:?}");
                for head in 0..2 {
                    let mut next = spec.clone();
                    next.toggle_head(head, heads);
                    let changes = sides(&next) != sides(&spec);
                    assert_eq!(
                        spec.sides_can_change(heads),
                        changes,
                        "{heads} heads, h={h:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn several_disks_number_their_files_before_or_after_the_name() {
        let mut out = Output {
            folder: "/f".into(),
            name: "Game".into(),
            ext: ".adf".into(),
            ..Output::default()
        };
        assert_eq!(out.file_name(1), "Game.adf", "one disk has no number");
        out.disks = 3;
        let names: Vec<_> = out.paths().collect();
        assert_eq!(
            names,
            [
                "/f/Game_Disk1.adf",
                "/f/Game_Disk2.adf",
                "/f/Game_Disk3.adf"
            ]
            .map(PathBuf::from)
        );
        out.disks = 12;
        out.number_first = true;
        out.label = "Side".into();
        assert_eq!(out.file_name(2), "Side02_Game.adf");
        out.label.clear();
        assert_eq!(out.file_name(12), "12_Game.adf");
    }

    #[test]
    fn an_output_has_no_value_until_it_has_a_type_and_a_name() {
        let mut out = Output {
            folder: "/f".into(),
            ..Output::default()
        };
        assert_eq!(out.value(1), "");
        out.ext = ".img".into();
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(out.value(1), format!("/f{sep}Floppy.img"));
        out.name = " ".into();
        assert_eq!(out.value(1), "");
    }

    #[test]
    fn image_options_use_gws_notation() {
        let mut out = Output {
            folder: "/f".into(),
            name: "D".into(),
            ext: ".hfe".into(),
            ..Output::default()
        };
        out.opts.insert("version".into(), "3".into());
        out.opts.insert("double_step".into(), ON.into());
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(out.value(1), format!("/f{sep}D.hfe::double_step:version=3"));
        let value = out.value(1);
        let path = format!("/f{sep}D.hfe");
        assert_eq!(split_opts(&value), (path.as_str(), out.opts.clone()));
    }

    #[test]
    fn a_format_picks_the_image_type_people_use_for_it() {
        let s = schema();
        let t = |f| type_for(&s, f);
        assert_eq!(t("akai.800"), ".img");
        assert_eq!(t("amiga.amigados"), ".adf");
        assert_eq!(t("commodore.1541"), ".d64");
        assert_eq!(t("commodore.1581"), ".d81");
        assert_eq!(t("atarist.720"), ".st");
        assert_eq!(t("acorn.dfs.ss80"), ".ssd");
        assert_eq!(t("apple2.prodos.140"), ".po");
        assert_eq!(t("ibm.800"), ".img", "not SAM Coupé's .mgt");
    }

    #[test]
    fn families_have_names_even_new_ones() {
        assert_eq!(family_name("akai.800"), "Akai");
        assert_eq!(family_name("pc98.2hd"), "NEC PC-98");
        assert_eq!(family_name("newthing.1"), "Newthing");
    }

    #[test]
    fn every_argument_has_a_short_tooltip_and_every_tooltip_an_argument() {
        let s = schema();
        for cmd in &s.commands {
            for a in &cmd.args {
                let tip = tip(&cmd.name, a);
                assert!(tip.ends_with('.'), "gw {} {}: {tip}", cmd.name, a.dest);
                assert!(tip.len() <= 70, "gw {} {} is long: {tip}", cmd.name, a.dest);
                assert!(
                    TIPS.iter()
                        .any(|(c, d, _)| (c.is_empty() || *c == cmd.name) && *d == a.dest),
                    "gw {} {} has only gw's own help: {tip}",
                    cmd.name,
                    a.dest
                );
            }
        }
        for (c, d, _) in TIPS {
            let found = s
                .commands
                .iter()
                .any(|cmd| (c.is_empty() || cmd.name == *c) && cmd.arg(d).is_some());
            assert!(found, "no gw {c} has {d}");
        }
    }

    #[test]
    fn examples_come_from_gws_own_help() {
        let s = schema();
        assert_eq!(
            example(&s, "TSPEC").as_deref(),
            Some("e.g. c=0-7,9-12:h=0-1")
        );
        assert_eq!(example(&s, "PRECOMP").as_deref(), Some("e.g. 40=125"));
    }

    #[test]
    fn help_reads_as_sentences() {
        assert_eq!(sentence("number of revolutions"), "Number of revolutions.");
        assert_eq!(sentence("pin level (H,L)"), "Pin level (H,L).");
        assert_eq!(sentence("Done."), "Done.");
    }
}

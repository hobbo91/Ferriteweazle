//! A command's settings as a form. Some arguments get a hand-made field; the
//! rest, including any a newer gw adds, get one chosen by their type.

use crate::command::{ON, Values};
use crate::schema::{Arg, Command, FormatInfo, ImageOpt, Schema, extension};
use crate::service::{Load, Service};
use crate::theme;
use eframe::egui::{
    self, Color32, CornerRadius, PopupCloseBehavior, RichText, Sense, TextEdit, Ui,
    ViewportCommand, pos2, vec2,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Arguments set in the sidebar, for every command that has them.
pub const GLOBAL: [&str; 2] = ["device", "drive"];

/// Arguments a command shows first, in order; the rest go under Advanced options.
const FIRST: &[(&str, &[&str])] = &[
    ("read", &["format", "revs", "file", "tracks"]),
    ("write", &["file", "format", "tracks", "no_verify"]),
    ("convert", &["in_file", "format", "out_file", "tracks"]),
    ("erase", &["tracks"]),
    ("align", &["tracks", "format", "reads"]),
];

/// Groups of arguments, each under a heading, empty for none.
type Groups = &'static [(&'static str, &'static [&'static str])];

/// The drive's own options, the same on every page that reads or writes one.
const DRIVE: &[&str] = &[
    "densel",
    "gen_tg43",
    "fake_index",
    "hard_sectors",
    "reverse",
];

/// Arguments under Advanced options, in order, in groups under a heading
/// (none for an empty one); any others, such as a newer gw's, follow in gw's
/// own order with no heading.
const ADVANCED: &[(&str, Groups)] = &[
    (
        "read",
        &[
            ("", &["diskdefs"]),
            (
                "Reading",
                &["retries", "seek_retries", "pll", "adjust_speed"],
            ),
            ("Drive", DRIVE),
            ("Image", &["raw", "no_clobber"]),
        ],
    ),
    (
        "write",
        &[
            ("", &["diskdefs"]),
            (
                "Writing",
                &["pre_erase", "erase_empty", "retries", "precomp"],
            ),
            ("Drive", DRIVE),
        ],
    ),
    (
        "convert",
        &[
            ("", &["diskdefs"]),
            (
                "Conversion",
                &["pll", "adjust_speed", "hard_sectors", "reverse"],
            ),
            ("Image", &["no_clobber"]),
            // Under its own heading, Output track settings.
            ("", &["out_tracks"]),
        ],
    ),
    (
        "align",
        &[
            ("", &["diskdefs"]),
            ("Reading", &["revs", "pll", "adjust_speed"]),
            ("Drive", DRIVE),
            ("Image", &["raw"]),
        ],
    ),
];

/// Headings over a page's first rows: each groups the argument it names and those after it.
const HEADINGS: &[(&str, &str, &str)] = &[
    ("read", "format", "Disk settings"),
    ("read", "file", "Image settings"),
    ("write", "file", "Image settings"),
    ("write", "format", "Disk settings"),
    ("convert", "in_file", "Input settings"),
    // gw opens the input and the output with the format, so it is neither's.
    ("convert", "format", "Disk settings"),
    ("convert", "out_file", "Output settings"),
];

/// Arguments whose file is written: a folder, a name and a type.
pub const OUTPUTS: &[(&str, &str)] = &[("read", "file"), ("convert", "out_file")];

/// Whether a track list swaps the sides' heads.
pub fn swapped(tracks: &str) -> bool {
    TrackSpec::parse(tracks).hswap
}

/// Whether `command` makes an image the page names.
pub fn has_output(command: &str) -> bool {
    OUTPUTS.iter().any(|(c, _)| *c == command)
}

/// The update page's firmware source, kept with its settings. Not a gw argument.
const FIRMWARE: &str = "firmware";

/// A page's batch settings, kept with its values. Not gw arguments.
pub const BATCH: &str = "batch";
pub const BATCH_FOLDER: &str = "batch_folder";
/// The one image type a batch takes, empty for every type gw reads.
pub const BATCH_TYPE: &str = "batch_type";

/// Commands that take a folder of images one at a time, and the argument
/// each image goes to.
const BATCHES: &[(&str, &str)] = &[("write", "file"), ("convert", "in_file")];

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
            Firmware::Latest => "Download the latest release.",
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
    fn of(values: &Values) -> Firmware {
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
    ("cyls", "Cylinders"),
    ("densel", "Density select"),
    ("diskdefs", "Disk definitions"),
    ("erase_empty", "Erase empty tracks"),
    ("format", "Disk format"),
    ("gen_tg43", "TG43 signal"),
    ("hfreq", "High frequency"),
    ("in_file", "Input"),
    ("linger", "Time per step"),
    ("motor", "Motor delay"),
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

/// Image types that hold flux or bitcells: gw saves any track in them with
/// no format.
const FLUX: &[&str] = &[".scp", ".hfe", ".raw", ".a2r", ".ipf", ".ctr"];

/// Flux as read, which has no bitrate: gw makes HFE of it only at a set one.
const RAW_FLUX: &[&str] = &[".scp", ".raw", ".a2r"];

/// A KryoFlux stream: one file per track, `nameCC.H.raw`, which gw opens as a
/// set from any one of them.
const KRYOFLUX: &str = ".raw";

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

/// The image type for formats that gw pairs with none, by prefix; the first
/// match wins.
const TYPES: &[(&str, &str)] = &[
    ("atarist.", ".st"),
    ("amiga.", ".adf"),
    ("acorn.dfs.ss", ".ssd"),
    ("acorn.dfs.ds", ".dsd"),
    // Physical sector order, which DOS order would scramble.
    ("apple2.nofs", ".img"),
    ("apple2.", ".do"),
    ("commodore.", ".d64"),
    // A scan keeps each track's layout, as gw's release notes pair it.
    ("ibm.scan", ".edsk"),
    ("northstar.", ".nsi"),
    // Bitcells have no sectors for a sector image to hold.
    ("raw.", ".hfe"),
    // Side 0, then side 1, as Thomson emulators take them.
    ("thomson.", ".fd"),
];

/// Most characters a typed name takes: an image's name, a disk label, a preset's name.
pub const NAME_LIMIT: usize = 48;
const LABEL_WIDTH: f32 = 112.0;
const MIN_FIELD: f32 = 160.0;
const MAX_FIELD: f32 = 400.0;
/// The widest a field grows, in a page wider than `full_width`.
const WIDE_FIELD: f32 = 800.0;
/// Short lists and values: half of Disk format's list at the window's
/// default size, so Revolutions ends at its middle.
const SHORT_FIELD: f32 = 119.0;
/// A number of two or three digits.
const NUMBER_FIELD: f32 = 56.0;
/// An image option, wide enough for gw's names: Default (other-320k).
const OPTION_FIELD: f32 = 190.0;
/// A cylinder number's box in the track picker.
const NUMBER_BOX: f32 = 32.0;
/// Detect, beside the format.
const DETECT_BUTTON: f32 = 68.0;
/// The browse button, beside a path.
const BROWSE_BUTTON: f32 = 34.0;
const ROW_GAP: f32 = 9.0;

/// Space above a heading, beside the rows' own gap.
const GROUP_GAP: f32 = 6.0;

/// Space above the sections that open out, beside the rows' own gap.
const SECTIONS_GAP: f32 = 12.0;

/// A field fills the room beside its label, up to a point.
fn field_width(ui: &Ui) -> f32 {
    ui.available_width().clamp(MIN_FIELD, WIDE_FIELD)
}

/// The width of a form's rows, from the `room` the page has: notices and
/// headings end where the fields do.
pub fn form_width(ui: &Ui, room: f32) -> f32 {
    let gap = ui.spacing().item_spacing.x;
    let field = (room - LABEL_WIDTH - gap).clamp(MIN_FIELD, WIDE_FIELD);
    LABEL_WIDTH + gap + field
}

/// A form's width when its fields are as wide as a page usually has them.
pub fn full_width(ui: &Ui) -> f32 {
    LABEL_WIDTH + ui.spacing().item_spacing.x + MAX_FIELD
}

/// A form's width when its fields are as wide as they get.
pub fn widest(ui: &Ui) -> f32 {
    LABEL_WIDTH + ui.spacing().item_spacing.x + WIDE_FIELD
}

/// A one-line text field as tall as the lists and buttons beside it.
pub fn edit(text: &mut String) -> TextEdit<'_> {
    TextEdit::singleline(text)
        .min_size(vec2(0.0, theme::FIELD_HEIGHT))
        .vertical_align(egui::Align::Center)
        .margin(egui::Margin::symmetric(10, 4))
}

/// Adds a text box with the menu a system one has: Cut, Copy and Paste on a
/// right click, which keeps the selection the click would otherwise move.
pub fn text_box(ui: &mut Ui, edit: TextEdit<'_>) -> egui::Response {
    text_box_with(ui, ui.next_auto_id(), edit)
}

/// `text_box`, with the box under `id`.
pub fn text_box_with(ui: &mut Ui, id: egui::Id, edit: TextEdit<'_>) -> egui::Response {
    let ctx = ui.ctx().clone();
    let selection = || {
        egui::text_edit::TextEditState::load(&ctx, id)
            .and_then(|s| s.cursor.char_range())
            .filter(|r| !r.is_empty())
    };
    // egui puts the cursor where any button presses; a right-click keeps the
    // selection, as a system text box does, for the menu's Cut and Copy.
    let kept = ui
        .input(|i| i.pointer.secondary_pressed())
        .then(selection)
        .flatten();
    let response = ui.add(edit.id(id));
    if let Some(range) = kept
        && let Some(mut state) = egui::text_edit::TextEditState::load(&ctx, id)
    {
        state.cursor.set_char_range(Some(range));
        state.store(&ctx, id);
    }
    // A paste goes in at the cursor or over the selection.
    response.context_menu(|ui| {
        let selected = selection().is_some();
        for (name, can, command) in [
            ("Cut", selected, ViewportCommand::RequestCut),
            ("Copy", selected, ViewportCommand::RequestCopy),
            ("Paste", true, ViewportCommand::RequestPaste),
        ] {
            let item = ui.add_enabled(can, egui::Button::new(name));
            if item.on_disabled_hover_text("Nothing selected.").clicked() {
                // Back in the box for the cut or paste the next frame brings.
                ui.memory_mut(|m| m.request_focus(id));
                ui.ctx().send_viewport_cmd(command);
                ui.close();
            }
        }
    });
    response
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
    /// The device is an Adafruit RP2040: options its firmware cannot carry
    /// out are greyed and shown as off.
    pub adafruit: bool,
    /// gw runs as a standalone program, which cannot read in passes.
    pub standalone: bool,
    /// What the Greaseweazle last reported for the page's settings, greyed in
    /// their empty fields: gw delays's values.
    pub reported: Option<&'a BTreeMap<&'static str, String>>,
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
        for (i, a) in first.iter().filter(|a| !a.is("TrackSet")).enumerate() {
            let name = self.cmd.name.as_str();
            if let Some((.., text)) = HEADINGS.iter().find(|(c, d, _)| *c == name && *d == a.dest) {
                if i > 0 {
                    ui.add_space(GROUP_GAP);
                }
                heading(ui, text, |_| {});
            }
            action = action.or(self.arg(ui, a));
        }
        // A track list is a group of its own, after the page's other rows.
        for a in first.iter().filter(|a| a.is("TrackSet")) {
            if first.len() > 1 {
                ui.add_space(GROUP_GAP);
            }
            self.tracks(ui, a);
        }
        let read_file = self.cmd.name == "read" && self.cmd.arg("file").is_some();
        // The sections that open out sit apart from the rows above them.
        if read_file || !rest.is_empty() {
            ui.add_space(SECTIONS_GAP);
        }
        if read_file {
            self.disks(ui);
            self.passes(ui);
        }
        if !rest.is_empty() {
            // Set values go to gw while the header is shut; the Adafruit
            // RP2040's greyed ones do not.
            let impossible =
                |a: &Arg| self.adafruit && crate::device::adafruit::option(&self.cmd.name, &a.dest);
            let set = rest
                .iter()
                .filter(|a| self.values.on(&a.dest) && !impossible(a))
                .count();
            let title = match set {
                0 => format!("Advanced options ({})", rest.len()),
                n => format!("Advanced options ({}, {n} set)", rest.len()),
            };
            let title = RichText::new(title).strong();
            egui::CollapsingHeader::new(title)
                .id_salt(("more", &self.cmd.name))
                .show_unindented(ui, |ui| {
                    ui.add_space(6.0);
                    let groups = advanced_groups(&self.cmd.name, &rest);
                    for (i, (text, args)) in groups.into_iter().enumerate() {
                        if i > 0 {
                            ui.add_space(GROUP_GAP);
                        }
                        if !text.is_empty() {
                            heading(ui, text, |_| {});
                        }
                        for a in args {
                            match a.is("TrackSet") {
                                true => self.tracks(ui, a),
                                false => action = action.or(self.arg(ui, a)),
                            }
                        }
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
        let impossible = self.adafruit && crate::device::adafruit::option(&self.cmd.name, &a.dest);
        ui.data_mut(|d| d.remove_temp::<bool>(own_tip_id()));
        // "File" is one of the row's own choices.
        let batchable = BATCHES.contains(&(self.cmd.name.as_str(), a.dest.as_str()));
        let text = match (batchable, a.dest.as_str()) {
            (true, "file") => "Image".to_owned(),
            _ => label(a),
        };
        // Shown as off, as gw gets it, and kept for a Greaseweazle.
        let kept = impossible.then(|| {
            let kept = self.values.get(&a.dest).to_owned();
            self.values.set(&a.dest, "");
            kept
        });
        let (name, (field, action)) = row(ui, &text, |ui| {
            let enabled = blocker.is_none() && !impossible;
            let r = ui.add_enabled_ui(enabled, |ui| self.field(ui, a));
            (r.response, r.inner)
        });
        if let Some(kept) = kept {
            self.values.set(&a.dest, kept);
        }
        let tip = tip(&self.cmd.name, a);
        // gw's own help for values such as track lists, its columns kept.
        let grammar = grammar(self.schema, a);
        let help = |ui: &mut Ui| explain(ui, &tip, grammar);
        name.on_hover_ui(help);
        // Laid over the field, so it is hovered along with whatever is under
        // it, unless that has a tooltip of its own.
        let over = ui.interact(field.rect, field.id.with("tip"), Sense::hover());
        let quiet = ui.data_mut(|d| d.remove_temp::<bool>(own_tip_id()));
        if impossible {
            over.on_hover_text(crate::device::adafruit::OPTION);
        } else if let Some(b) = blocker {
            over.on_hover_text(format!("Cannot be used with {}.", label(b)));
        } else if quiet.is_none() {
            over.on_hover_ui(help);
        }
        action
    }

    /// Another argument of the same exclusive group that is set; one set itself is never
    /// blocked, so it can be cleared.
    fn blocker(&self, a: &Arg) -> Option<&'a Arg> {
        let group = a.group.filter(|_| !self.values.on(&a.dest))?;
        self.cmd
            .args
            .iter()
            .find(|b| b.group == Some(group) && b.dest != a.dest && self.values.on(&b.dest))
    }

    fn field(&mut self, ui: &mut Ui, a: &Arg) -> Option<Action> {
        match a.dest.as_str() {
            "format" => return self.format(ui, a),
            "file" | "in_file" if a.positional() => self.input(ui, a),
            "diskdefs" => self.diskdefs(ui, a),
            // Shown once File is chosen, so it is needed.
            "file" if self.cmd.name == "update" => {
                self.path(ui, a, "Required", Some(("Firmware updates", &["upd"])))
            }
            dest if dest.ends_with("file") => self.path(ui, a, "None", None),
            _ if a.switch => {
                let mut on = self.values.on(&a.dest);
                if toggle(ui, &mut on, &label(a)).changed() {
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
        let mut value = self.values.get(&a.dest).to_owned();
        let id = ui.make_persistent_id(("suggest", &self.cmd.name, &a.dest));
        let default = a.default.clone().or_else(|| self.implied_default(a));
        let unset = default_label(default.as_deref());
        let listed = options.iter().copied();
        ui.horizontal(|ui| {
            let (changed, other) =
                drop_down(ui, id, &mut value, &unset, a.required, SHORT_FIELD, listed);
            if changed {
                self.values.set(&a.dest, value);
            }
            if other {
                self.typed(ui, a, hint(a, self.schema), false, SHORT_FIELD);
            }
        });
    }

    /// gw's default for an argument its parser gives none for: the
    /// revolutions a read takes per track, which the format sets (a fraction
    /// past one is timed), else 3. Raw flux is read in whole revolutions, two.
    fn implied_default(&mut self, a: &Arg) -> Option<String> {
        if a.dest != "revs" || self.cmd.arg("format").is_none() {
            return None;
        }
        let revs = match self.effective_format() {
            Some(format) => self.format_info(&format).ready()?.revs?,
            None => 3.0,
        };
        // Raw flux is read in whole revolutions: a timed fraction becomes two.
        let revs = match revs.fract() != 0.0 && self.values.on("raw") {
            true => 2.0,
            false => revs,
        };
        Some(revs.to_string())
    }

    /// Choices as a row of buttons; choosing the chosen one again clears it.
    fn pick(&mut self, ui: &mut Ui, a: &Arg, options: &[(&str, &str)]) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let current = self.values.get(&a.dest).to_owned();
            let (unset, tip) = match a.dest.as_str() {
                "densel" => ("Auto", "Greaseweazle Tools leaves pin 2 as it is."),
                _ => ("Default", "Greaseweazle Tools' own choice."),
            };
            if a.default.is_none()
                && !a.required
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

    /// Where gw update gets the firmware; the chosen source's field follows.
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
        let reported = self.reported.and_then(|r| r.get(a.dest.as_str()));
        // gw takes its default for an empty field, so it shows as the value.
        let (shown, default) = match (reported, &a.default) {
            (Some(value), _) => (value.clone(), false),
            (None, Some(default)) => (default.clone(), true),
            (None, None) => (hint(a, self.schema), false),
        };
        ui.horizontal(|ui| {
            let width = if number { SHORT_FIELD } else { field_width(ui) };
            self.typed(ui, a, shown, default, width);
        });
    }

    /// A box to type the value in, and gw's objection to what is typed. Empty,
    /// it shows `hint` greyed, or as if typed where that is gw's `default`.
    fn typed(&mut self, ui: &mut Ui, a: &Arg, hint: String, default: bool, width: f32) {
        let mut value = self.values.get(&a.dest).to_owned();
        let edit = edit(&mut value).hint_text(hint).desired_width(width);
        let changed = ui
            .scope(|ui| {
                // egui draws a hint in the weak colour, whatever colour it is given.
                if default {
                    ui.visuals_mut().weak_text_color = Some(ui.visuals().text_color());
                }
                text_box(ui, edit).changed()
            })
            .inner;
        if changed {
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

    /// A file for gw to read, typed or chosen in an open dialog. `only` keeps
    /// the dialog to one kind of file: its name and suffixes.
    fn path(&mut self, ui: &mut Ui, a: &Arg, hint: &str, only: Option<(&str, &[&str])>) {
        let mut value = self.values.get(&a.dest).to_owned();
        ui.horizontal(|ui| {
            let width = beside_button(ui, BROWSE_BUTTON);
            if path_edit(ui, &mut value, hint, width).changed() {
                self.values.set(&a.dest, value.as_str());
            }
            if browse_button(ui).own_tip("Select a file.").clicked() {
                let mut dialog = file_dialog(Path::new(&value));
                if let Some((name, exts)) = only {
                    dialog = dialog.add_filter(name, &both_cases(exts.iter().copied()));
                }
                if let Some(path) = dialog.pick_file() {
                    self.values.set(&a.dest, path.to_string_lossy());
                }
            }
        });
    }

    /// A disk definitions file, and what gw makes of it.
    fn diskdefs(&mut self, ui: &mut Ui, a: &Arg) {
        self.path(ui, a, "None", None);
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
            Load::Ready(d) => {
                let n = d.formats.len();
                if n > 0 {
                    let formats = if n == 1 { "format" } else { "formats" };
                    let text = format!("Adds {n} {formats} to the top of the format list.");
                    ui.label(small(text).weak());
                }
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
            ("", None) => RichText::new("Select disk format").color(dim),
            (chosen, _) if custom => RichText::new(format!("Custom · {chosen}")),
            (chosen, _) => RichText::new(format_name(chosen)),
        };
        let mut chosen = None;
        let mut action = None;
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let can_detect = detectable(self.schema, self.cmd, self.values);
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
                    let tip = match has_output(&self.cmd.name) {
                        true => "Attempt to find the disk format and the image type that suits it.",
                        false => "Attempt to find the disk format.",
                    };
                    if ui
                        .add_enabled(self.cannot_detect.is_none(), detect)
                        .own_tip(tip)
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
        // No menu: egui keeps one popup open, so one would shut the list.
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
                .on_hover_text("Use the image's own format, if it has one.")
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
                        // A scroll area takes its parent's layout, here horizontal.
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                            for family in &families {
                                let mut text = RichText::new(heading(family));
                                // Selected, it takes the selection's colour.
                                if !current.is_empty()
                                    && current_family == *family
                                    && *family != open
                                {
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

    /// A track list under a heading of its own: the picker's rows, or gw's
    /// notation typed where they cannot show the list.
    fn tracks(&mut self, ui: &mut Ui, a: &Arg) {
        // Unset, gw's output tracks are the cylinders and sides read.
        let output = a.dest == "out_tracks";
        let title = match output {
            true => "Output track settings",
            false => "Track settings",
        };
        let format = self.effective_format();
        let known = format
            .as_ref()
            .and_then(|f| self.format_info(f).ready().map(|i| (i.cyls, i.heads)));
        let (cyls, heads) = known.unwrap_or(USUAL_DISK);
        let mut spec = TrackSpec::parse(self.values.get(&a.dest));
        let base = match output {
            true => TrackSpec::parse(self.values.get("tracks")),
            false => TrackSpec::default(),
        };
        // With no format gw takes 0-81 at any step; stepped further, the
        // picker keeps to those the drive reaches and names them.
        let free = !output && format.is_none();
        let step = spec.steps().unwrap_or(1);
        let short = free && step > 1;
        let cyls = if short { reach(step) } else { cyls };
        let whole = base.cylinders().unwrap_or((0, cyls.saturating_sub(1)));
        let text_id = ui.make_persistent_id(("tracks-text", &self.cmd.name, &a.dest));
        let simple = spec.simple();
        let as_text = ui.data(|d| d.get_temp(text_id)).unwrap_or(false) || !simple;
        let tip = tip(&self.cmd.name, a);
        let grammar = grammar(self.schema, a);
        let help = |ui: &mut Ui| explain(ui, &tip, grammar);
        heading(ui, title, |ui| {
            let flip = if as_text {
                "Use the track picker"
            } else {
                "Type a track list"
            };
            if ui
                .add_enabled(simple, egui::Link::new(RichText::new(flip).small()))
                .on_disabled_hover_text("The track picker cannot show this track list.")
                .clicked()
            {
                ui.data_mut(|d| d.insert_temp(text_id, !as_text));
            }
        })
        .on_hover_ui(help);
        if as_text {
            let (name, field) = row(ui, "Track list", |ui| {
                ui.horizontal(|ui| {
                    let hint = example(self.schema, "TSPEC").unwrap_or_default();
                    let width = field_width(ui);
                    self.typed(ui, a, hint, false, width);
                })
                .response
            });
            name.on_hover_ui(help);
            // Laid over the field, as for any other argument.
            let over = ui.interact(field.rect, field.id.with("tip"), Sense::hover());
            over.on_hover_ui(help);
        } else {
            let (mut first, mut last) = spec.cylinders().unwrap_or(whole);
            let mut changed = false;
            let (name, _) = row(ui, "Cylinders", |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    // Cylinder numbers have three digits at most.
                    ui.spacing_mut().interact_size.x = NUMBER_BOX;
                    changed |= ui
                        .add(egui::DragValue::new(&mut first).range(0..=last))
                        .changed();
                    ui.label("to");
                    changed |= ui
                        .add(egui::DragValue::new(&mut last).range(first..=254))
                        .changed();
                })
            });
            name.on_hover_text("The first and last cylinder.");
            if changed {
                let default = (first, last) == whole && !short;
                spec.c = match spec.half() {
                    true => Some(format!("{first}-{last}/2")),
                    false => (!default).then(|| format!("{first}-{last}")),
                };
            }
            let (name, _) = row(ui, "Sides", |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let mut sides = spec.sides(&base);
                    let fixed = !sides.sides_can_change(heads);
                    for head in 0..2u32 {
                        let on = sides.has_head(head, heads);
                        let r = ui
                            .add_enabled(!fixed, egui::Button::selectable(on, head.to_string()))
                            .on_disabled_hover_text("This format is single sided.");
                        if r.clicked() {
                            sides.toggle_head(head, heads);
                            spec.h = sides.h.clone();
                            changed = true;
                        }
                    }
                    ui.add_space(12.0);
                    changed |= checkbox(ui, &mut spec.hswap, "Swap sides")
                        .on_hover_text("Use head 1 for side 0 and head 0 for side 1.")
                        .changed();
                })
            });
            name.on_hover_text("The sides to use.");
            let other_id = ui.make_persistent_id(("step-other", &self.cmd.name, &a.dest));
            let (name, _) = row(ui, "Step", |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let current = spec.step.clone().unwrap_or_else(|| "1".into());
                    // Other… stays chosen, its box showing, while the step is the
                    // one it gave: a button, a preset or a typed list that changes
                    // the step shuts it.
                    let given: Option<String> = ui.data(|d| d.get_temp(other_id));
                    let other =
                        !STEPS.contains(&current.as_str()) || given == Some(current.clone());
                    for value in STEPS {
                        let (text, tip) = match value {
                            HALF => ("½", HALF_TIP),
                            _ => (value, STEP_TIP),
                        };
                        let on = !other && current == value;
                        let r = ui.add(egui::Button::selectable(on, text));
                        if r.on_hover_text(tip).clicked() && !on {
                            ui.data_mut(|d| d.remove_temp::<String>(other_id));
                            spec.pick_step(value, (first, last), whole, free);
                            changed = true;
                        }
                    }
                    let r = ui.add(egui::Button::selectable(other, OTHER));
                    if r.on_hover_text(OTHER_STEP_TIP).clicked() && !other {
                        ui.data_mut(|d| d.insert_temp(other_id, current.clone()));
                    }
                    if other {
                        // gw's step=[0-9]: a half step shows as 1 until changed.
                        let mut n = spec.steps().unwrap_or(1).min(9);
                        let box_ = egui::DragValue::new(&mut n).range(0..=9);
                        let r = ui.scope(|ui| {
                            ui.spacing_mut().interact_size.x = NUMBER_BOX;
                            ui.add(box_)
                        });
                        if r.inner.on_hover_text(OTHER_STEP_TIP).changed() {
                            spec.pick_step(&n.to_string(), (first, last), whole, free);
                            let given = spec.step.clone().unwrap_or_else(|| "1".into());
                            ui.data_mut(|d| d.insert_temp(other_id, given));
                            changed = true;
                        }
                    }
                })
            });
            name.on_hover_text(STEP_TIP);
            let (name, _) = row(ui, "Head offset", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.spacing_mut().interact_size.x = NUMBER_BOX;
                    let named = spec.sides(&base);
                    let sides = [
                        ("Side 0", "Side 0's offset, in cylinders.", "Needs side 0."),
                        ("Side 1", "Side 1's offset, in cylinders.", "Needs side 1."),
                    ];
                    for (head, (text, tip, why)) in sides.into_iter().enumerate() {
                        if head == 1 {
                            ui.add_space(6.0);
                        }
                        ui.label(text);
                        let value = egui::DragValue::new(&mut spec.off[head])
                            .range(-MAX_OFFSET..=MAX_OFFSET)
                            .custom_formatter(|n, _| match n as i32 {
                                0 => "0".to_owned(),
                                n => format!("{n:+}"),
                            })
                            .custom_parser(|t| t.trim().parse::<i32>().ok().map(f64::from));
                        changed |= ui
                            .add_enabled(named.has_head(head as u32, heads), value)
                            .on_hover_text(tip)
                            .on_disabled_hover_text(why)
                            .changed();
                    }
                })
            });
            name.on_hover_text(
                "Cylinders to move each side's head by, for a flippy-modded drive: \
                 side 1 by -8 for a Panasonic mod, side 0 by +8 for a Teac mod.",
            );
            if changed {
                self.values.set(&a.dest, spec.to_string());
            }
        }
        // The list as it stands, where it runs past the format's cylinders.
        let spec = match as_text {
            true => TrackSpec::parse(self.values.get(&a.dest)),
            false => spec,
        };
        // A sector image has no tracks past its format; a flux or track image may.
        let sectors = !self
            .schema
            .image(input_file(self.cmd, self.values))
            .is_some_and(|(_, i)| i.tracks);
        if let (Some((format_cyls, _)), Some(format), false) = (known, &format, output)
            && let Some(note) = past_format(
                &self.cmd.name,
                format,
                &spec,
                format_cyls,
                self.values.on("raw"),
                sectors,
            )
        {
            row(ui, "", |ui| {
                let text = RichText::new(note)
                    .small()
                    .color(theme::palette(ui).partial);
                ui.add(egui::Label::new(text).wrap())
            });
        }
    }

    /// The image to read, with its type and options; Write and Convert also take a folder.
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
                (false, "File", "Single image"),
                (
                    true,
                    "Folder",
                    "Every image in the folder, sequentially in order of name.",
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
                    // In a batch it holds the folder's first image, not a chosen file.
                    self.values.set(&a.dest, "");
                    self.values.set(BATCH, if on { ON } else { "" });
                }
            }
        });
    }

    /// A folder of images, whose first stands for them all: the format, Detect and the
    /// command line go by it.
    fn folder(&mut self, ui: &mut Ui, a: &Arg) {
        let mut folder = self.values.get(BATCH_FOLDER).to_owned();
        ui.horizontal(|ui| {
            let width = beside_button(ui, BROWSE_BUTTON);
            path_edit(ui, &mut folder, "Image folder (Required)", width);
            if browse_button(ui)
                .own_tip("Select a folder containing multiple images.")
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
                    true => RichText::new("No images Greaseweazle Tools can read.").color(p.bad),
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
                let width = beside_button(ui, BROWSE_BUTTON);
                changed |= path_edit(ui, &mut path, "Image file (Required)", width).changed();
                if browse_button(ui).own_tip("Select an image.").clicked()
                    && let Some(p) = image_dialog(self.schema, &path).pick_file()
                {
                    path = p.to_string_lossy().into_owned();
                    changed = true;
                }
            });
            if split_path(self.values.get(&a.dest)) {
                let bad = theme::palette(ui).bad;
                ui.label(RichText::new(COLONS_IN).small().color(bad));
            } else if !path.is_empty() {
                match self.schema.image(&path) {
                    Some((ext, image)) => {
                        ui.label(RichText::new(image_name(&path, &image.name)).small().weak());
                        if !image.read_opts.is_empty() {
                            changed |=
                                image_options(ui, self.service, ext, &image.read_opts, &mut opts);
                        }
                        let stray = foreign(&opts, &image.read_opts);
                        if !stray.is_empty() {
                            foreign_label(ui, &stray);
                        }
                    }
                    None => {
                        ui.label(
                            RichText::new("Greaseweazle Tools does not know this file type.")
                                .small()
                                .color(theme::palette(ui).bad),
                        );
                    }
                }
            }
        });
        if changed {
            // gw refuses an option the image's type does not take.
            if let Some((_, image)) = self.schema.image(&path) {
                opts.retain(|k, _| image.read_opts.iter().any(|o| o.name == *k));
            }
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
                "" => RichText::new("Select image type").color(p.dim),
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
        // Flux as read holds any format; Raw saves that alone.
        let tried = schema.images.get(&out.ext).is_some_and(|i| i.writable)
            && !RAW_FLUX.contains(&out.ext.as_str())
            && !self.values.on("raw");
        if tried && let Some(format) = effective_format(self.service, schema, self.cmd, self.values)
        {
            let diskdefs = diskdefs_for(self.service, self.values, &format);
            if let Load::Ready(Some(e)) = self.service.fits(&diskdefs, &format, &out.ext) {
                let text = RichText::new(sentence(e)).small().color(p.bad);
                row(ui, "", |ui| ui.label(text));
            }
        }
        if let Some(image) = schema
            .images
            .get(&out.ext)
            .filter(|i| !i.write_opts.is_empty())
        {
            row(ui, "Image options", |ui| {
                image_options(ui, self.service, &out.ext, &image.write_opts, &mut out.opts)
            })
            .0
            .on_hover_text("Settings of this image type.");
        }
        if let Some(image) = schema.images.get(&out.ext) {
            let stray = foreign(&out.opts, &image.write_opts);
            if !stray.is_empty() {
                row(ui, "", |ui| foreign_label(ui, &stray));
            }
        }

        if has_input {
            // Beside the input, an image of the input's own type would replace it.
            let clash =
                !batch && extension(&input).is_some_and(|e| e.eq_ignore_ascii_case(&out.ext));
            out.beside_input &= !clash;
            let (text, tip) = match batch {
                true => (
                    "Next to each input",
                    "Save each image in the same folder as the input image.",
                ),
                false => (
                    "Next to the input file",
                    "Save the image in the input's folder, under its name.",
                ),
            };
            row(ui, "Save", |ui| {
                ui.add_enabled_ui(!clash, |ui| row_checkbox(ui, &mut out.beside_input, text))
                    .response
                    .on_disabled_hover_text(
                        "Conflicting file names between the input and output image.",
                    )
            })
            .0
            .on_hover_text(tip);
        }
        let beside = has_input && out.beside_input;
        // An image is named after its input, as converters do, until renamed.
        if !batch && !input.is_empty() && (beside || out.named_for != input) {
            out.name = image_stem(Path::new(&input));
            out.named_for.clone_from(&input);
        }
        if beside && !batch && !input.is_empty() {
            out.folder = lossy(Path::new(&input).parent().map(Path::as_os_str));
        }
        // Beside the input the folder and name are the input's: shown greyed.
        let folder = if has_input { "Output folder" } else { "Folder" };
        folder_row(ui, &mut out.folder, folder, !beside);
        if batch {
            let tip = "Text added to each input's name, such as Backup in Backup_Game.";
            let (name, _) = row(ui, "Label", |ui| {
                text_box(
                    ui,
                    edit(&mut out.batch_label)
                        .char_limit(NAME_LIMIT)
                        .hint_text("None")
                        .desired_width(SHORT_FIELD),
                )
                .on_hover_text(tip);
            });
            name.on_hover_text(tip);
            let labelled = !out.batch_label.trim().is_empty();
            let (name, _) = row(ui, "Position", |ui| {
                ui.add_enabled_ui(labelled, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.selectable_value(&mut out.label_first, false, "After the name")
                            .on_hover_text("Game_Backup, Demo_Backup…");
                        ui.selectable_value(&mut out.label_first, true, "Before the name")
                            .on_hover_text("Backup_Game, Backup_Demo…");
                    });
                })
                .response
                .on_disabled_hover_text("Needs a label.");
            });
            name.on_hover_text("Where the label goes.");
        } else {
            let tip = "The image's file name, extensions are handled by Image type.";
            let (name, _) = row(ui, "Name", |ui| {
                let width = field_width(ui);
                ui.add_enabled_ui(!beside, |ui| {
                    text_box(
                        ui,
                        edit(&mut out.name)
                            .char_limit(NAME_LIMIT)
                            .hint_text("Required")
                            .desired_width(width),
                    )
                })
                .inner
                .on_hover_text(tip)
                .on_disabled_hover_text(BESIDE);
            });
            name.on_hover_text(tip);
        }

        let value = match (batch, images.first()) {
            (true, Some(first)) => out.batch_value(first),
            (true, None) => String::new(),
            (false, _) => out.value(out.first_disk()),
        };
        if !value.is_empty() {
            row(ui, "", |ui| {
                let preview = match batch {
                    true => out.batch_preview(&images),
                    false => out.preview(),
                };
                ui.label(
                    RichText::new(short_path(&preview))
                        .monospace()
                        .small()
                        .color(p.dim),
                );
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

    /// Reading the disk again while sectors are missing.
    fn passes(&mut self, ui: &mut Ui) {
        let cannot = match (self.standalone, self.effective_format()) {
            (true, _) => Some("Standalone Greaseweazle Tools cannot read in passes."),
            (false, None) => Some("Needs a disk format."),
            (false, Some(_)) => None,
        };
        let out = self
            .outputs
            .entry(output_key(&self.cmd.name, "file"))
            .or_default();
        let on = cannot.is_none() && out.passes > 1;
        let title = match on {
            true => format!("Read passes ({})", out.passes),
            false => "Read passes".to_owned(),
        };
        let more = "Needs more than one read pass.";
        egui::CollapsingHeader::new(RichText::new(title).strong())
            .id_salt(("passes", &self.cmd.name))
            .show_unindented(ui, |ui| {
                ui.add_space(6.0);
                let tip = "Maximum number of times to read a disk with missing or damaged sectors. \
                           Retries read a track again before moving on. A pass comes back to it \
                           after the rest of the disk.";
                let (name, _) = row(ui, "Passes", |ui| {
                    let size = vec2(NUMBER_FIELD, theme::FIELD_HEIGHT);
                    let passes = egui::DragValue::new(&mut out.passes).range(1..=MAX_PASSES);
                    ui.add_enabled_ui(cannot.is_none(), |ui| ui.add_sized(size, passes))
                        .inner
                        .on_hover_text(tip)
                        .on_disabled_hover_text(cannot.unwrap_or_default());
                });
                name.on_hover_text(tip);
                let (name, _) = row(ui, "Re-read", |ui| {
                    ui.add_enabled_ui(on, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.selectable_value(&mut out.whole_disk, false, "Incomplete tracks")
                                .on_hover_text(
                                    "Attempt to re-read only the tracks still missing sectors.",
                                );
                            ui.selectable_value(&mut out.whole_disk, true, "Whole disk")
                                .on_hover_text("Attempt to re-read the entire disk.");
                        });
                    })
                    .response
                    .on_disabled_hover_text(more);
                });
                name.on_hover_text(
                    "Whether the next pass reads the whole disk again, or only the tracks that failed.",
                );
                let tip = "Save each pass's flux to the Read passes folder.";
                let (name, _) = row(ui, "Keep", |ui| {
                    ui.add_enabled_ui(on, |ui| row_checkbox(ui, &mut out.keep_passes, "Each pass"))
                        .inner
                        .on_hover_text(tip)
                        .on_disabled_hover_text(more);
                });
                name.on_hover_text(tip);
                if on && out.keep_passes {
                    let stem = image_stem(&out.path(out.first_disk()));
                    row(ui, "", |ui| {
                        let text =
                            format!("{PASSES_FOLDER}/{stem} pass 1.scp, {stem} pass 2.scp…");
                        ui.label(RichText::new(text).small().color(theme::palette(ui).dim));
                    });
                }
            });
    }

    /// Several disks read one after another, each into a numbered file.
    fn disks(&mut self, ui: &mut Ui) {
        let out = self
            .outputs
            .entry(output_key(&self.cmd.name, "file"))
            .or_default();
        let title = match (out.first_disk(), out.disks) {
            (_, 0 | 1) => "Multiple disks".to_owned(),
            (1, n) => format!("Multiple disks ({n})"),
            (first, n) => format!("Multiple disks ({first} to {n})"),
        };
        egui::CollapsingHeader::new(RichText::new(title).strong())
            .id_salt(("disks", &self.cmd.name))
            .show_unindented(ui, |ui| {
                ui.add_space(6.0);
                let tip = "How many disks the set has, read sequentially.";
                let (name, _) = row(ui, "Disks", |ui| {
                    let size = vec2(NUMBER_FIELD, theme::FIELD_HEIGHT);
                    ui.add_sized(
                        size,
                        egui::DragValue::new(&mut out.disks).range(1..=MAX_DISKS),
                    )
                    .on_hover_text(tip)
                });
                name.on_hover_text(tip);
                let (name, _) = row(ui, "Names", |ui| {
                    ui.add_enabled_ui(out.disks > 1, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.selectable_value(&mut out.ask_names, false, "Numbered")
                                .on_hover_text("Name each image file with a numbered label.");
                            ui.selectable_value(&mut out.ask_names, true, "Ask for each")
                                .on_hover_text("Type each disk's name when asked for the disk.");
                        });
                    })
                    .response
                    .on_disabled_hover_text("Needs more than one disk.");
                });
                name.on_hover_text("How each disk's file is named.");
                let numbered = out.disks > 1 && !out.ask_names;
                let why = match out.disks > 1 {
                    true => "Needs numbered names.",
                    false => "Needs more than one disk.",
                };
                let tip = "The number of the first disk to read, to carry on a set.";
                let (name, _) = row(ui, "First disk", |ui| {
                    let size = vec2(NUMBER_FIELD, theme::FIELD_HEIGHT);
                    let width = out.first_digits as usize;
                    let digits = std::cell::Cell::new(None);
                    let first = egui::DragValue::new(&mut out.first)
                        .range(1..=out.disks.max(1))
                        .custom_formatter(|n, _| format!("{:0width$}", n as u32))
                        .custom_parser(|text| {
                            let (n, typed) = typed_number(text)?;
                            digits.set(Some(typed));
                            Some(f64::from(n))
                        });
                    ui.add_enabled_ui(numbered, |ui| ui.add_sized(size, first))
                        .inner
                        .on_hover_text(tip)
                        .on_disabled_hover_text(why);
                    if let Some(typed) = digits.get() {
                        out.first_digits = typed;
                    }
                });
                name.on_hover_text(tip);
                let tip = "The text before each disk number, such as Disk in Samples_Disk1.";
                let (name, _) = row(ui, "Label", |ui| {
                    ui.add_enabled_ui(numbered, |ui| {
                        text_box(
                            ui,
                            edit(&mut out.label)
                                .char_limit(NAME_LIMIT)
                                .hint_text("e.g. Disk")
                                .desired_width(SHORT_FIELD),
                        )
                    })
                    .inner
                    .on_hover_text(tip)
                    .on_disabled_hover_text(why);
                });
                name.on_hover_text(tip);
                let (name, _) = row(ui, "Number", |ui| {
                    ui.add_enabled_ui(numbered, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.selectable_value(&mut out.number_first, false, "After the name")
                                .on_hover_text("Game_Disk1, Game_Disk2…");
                            ui.selectable_value(&mut out.number_first, true, "Before the name")
                                .on_hover_text("Disk1_Game, Disk2_Game…");
                        });
                    })
                    .response
                    .on_disabled_hover_text(why);
                });
                name.on_hover_text("Where each file's disk number goes.");
                let tip = "Adds the set's size: Game_Disk01_of_12.";
                let (name, _) = row(ui, "Total", |ui| {
                    ui.add_enabled_ui(numbered, |ui| {
                        row_checkbox(ui, &mut out.total, "Add the total")
                    })
                    .inner
                    .on_hover_text(tip)
                    .on_disabled_hover_text(why);
                });
                name.on_hover_text(tip);
                if out.disks > 1 {
                    row(ui, "", |ui| {
                        let text = match out.ask_names {
                            true => "Asks for each disk's name before reading it.".to_owned(),
                            false => {
                                format!("Names each disk sequentially: {}", out.preview_names())
                            }
                        };
                        ui.label(RichText::new(text).small().color(theme::palette(ui).dim));
                    });
                }
            });
    }
}

const TYPE_TIP: &str = "The type of image to create. Disk format picks one.";

/// Why the folder and name are greyed beside the input.
const BESIDE: &str = "Not used when saving next to the input.";

const REPLACES_INPUT: &str = "This is the input file. Select another type or name.";

const COLONS_IN: &str =
    "Greaseweazle Tools reads :: in a path as options. Select another image or folder.";

const FOREIGN: &str = "An image has an option its type does not take.";

const REPLACES_INPUTS: &str =
    "An image would replace its input. Select another type, folder or label.";

/// The most disks one session reads.
const MAX_DISKS: u32 = 256;

/// A number as typed, and its digits: `08` is 8 in 2 digits.
fn typed_number(text: &str) -> Option<(u32, u32)> {
    let text = text.trim();
    let digits = text.chars().all(|c| c.is_ascii_digit());
    Some((text.parse().ok().filter(|_| digits)?, text.len() as u32))
}

/// The most passes a read makes.
pub const MAX_PASSES: u32 = 5;

/// Where a read keeps each pass's flux, beside its image.
pub const PASSES_FOLDER: &str = "Read passes";

/// The start of the name of each pass's flux file for `image`: `Read passes/Floppy pass`.
pub fn pass_prefix(image: &Path) -> PathBuf {
    let name = format!("{} pass", image_stem(image));
    image.with_file_name(PASSES_FOLDER).join(name)
}

/// gw's tracks when no format gives them: c=0-81:h=0-1.
pub const USUAL_DISK: (u32, u32) = (82, 2);

/// Cylinders of gw's 0-81 still within reach at `step` head steps each: 0-40 at 2.
fn reach(step: u32) -> u32 {
    USUAL_DISK.0.div_ceil(step)
}

/// The track picker's head steps per cylinder as buttons: 1 to 4, and a half
/// step. Other… takes the rest of gw's step=[0-9] in a number box.
const STEPS: [&str; 5] = ["1", "2", "3", "4", HALF];

/// gw's half step: each two of the list's cylinders on one of the drive's.
const HALF: &str = "1/2";

/// Whether the picker can show a step: the half step, or a digit, as gw
/// documents step=[0-9].
fn shown_step(step: &str) -> bool {
    step == HALF || (step.len() == 1 && step.as_bytes()[0].is_ascii_digit())
}

const STEP_TIP: &str = "Head steps per cylinder, 0 to 9.";

const OTHER_STEP_TIP: &str = "Any other step, 0 to 9. At 0 the heads stay on cylinder 0.";

const HALF_TIP: &str = "Half step: the list's cylinders 0, 2, 4… on the drive's 0, 1, \
                        2…, for an image numbered in half tracks.";

/// The track picker's largest head offset, in cylinders: gw's h0.off=[+-][0-9].
const MAX_OFFSET: i32 = 9;

/// The last cylinder gw seeks to without asking: it calls any further one extreme.
pub const LAST_USUAL_CYLINDER: u32 = 83;

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
            Firmware::File if !values.on("file") => return Some("Select an update file first."),
            _ => {}
        }
    }
    // gw would stop at the format, once the drive is open.
    let chosen = values.get("format");
    let fault = service.known_format_info(known_diskdefs(service, values, chosen), chosen);
    if fault.is_some_and(|f| f.error().is_some()) {
        return Some("Greaseweazle Tools cannot use this disk format. See Disk format.");
    }
    // gw would stop at the image before it opens the drive.
    if chosen.is_empty()
        && format_in_file(schema, cmd, values)
        && service.image_fault(input_file(cmd, values)).is_some()
    {
        return Some("Greaseweazle Tools cannot read this image. See Disk format.");
    }
    // gw opens a plain sector image only with a format; Read and Convert
    // wait for one below.
    if chosen.is_empty()
        && !has_output(&cmd.name)
        && schema
            .image(input_file(cmd, values))
            .is_some_and(|(_, i)| i.needs_format)
    {
        return Some("Select a disk format first.");
    }
    let batch = batch_input(cmd, values);
    if let Some(dest) = batch
        && values.get(dest).is_empty()
    {
        return Some(match values.get(BATCH_FOLDER) {
            "" => "Select a folder of images first.",
            _ => "The folder has no images Greaseweazle Tools can read.",
        });
    }
    if let Some((_, dest)) = BATCHES.iter().find(|(c, _)| *c == cmd.name) {
        let value = values.get(dest);
        if split_path(value) {
            return Some(COLONS_IN);
        }
        let (path, opts) = split_opts(value);
        if schema
            .image(path)
            .is_some_and(|(_, i)| !foreign(&opts, &i.read_opts).is_empty())
        {
            return Some(FOREIGN);
        }
    }
    if let Some((_, dest)) = OUTPUTS.iter().find(|(c, _)| *c == cmd.name) {
        let out = outputs.get(&output_key(&cmd.name, dest));
        let Some(out) = out.filter(|o| !o.ext.is_empty()) else {
            return Some("Select an image type first.");
        };
        // Only a pasted command line gives one of these.
        let image = match schema.images.get(&out.ext) {
            None => return Some("Greaseweazle Tools does not know this image type."),
            Some(i) if !i.writable => {
                return Some("Greaseweazle Tools cannot write this image type.");
            }
            Some(i) => i,
        };
        if !foreign(&out.opts, &image.write_opts).is_empty() {
            return Some(FOREIGN);
        }
        if batch.is_none() && !out.asks_names() && out.name.trim().is_empty() {
            return Some("Name the image first.");
        }
        let beside = cmd.arg("in_file").is_some() && out.beside_input;
        if !beside && out.folder.trim().is_empty() {
            return Some("Select a folder first.");
        }
        // gw would take it in its own working folder, which the app never sets.
        if !beside && !Path::new(out.folder.trim()).has_root() {
            return Some("Type the folder's full path.");
        }
        let named = match batch {
            None => out.name.contains("::"),
            Some(_) => out.batch_label.contains("::"),
        };
        if !beside && (named || out.folder.contains("::")) {
            return Some(
                "Greaseweazle Tools reads :: in a path as options. Select another folder or name.",
            );
        }
        if batch.is_some() {
            let files = service.known_folder(values.get(BATCH_FOLDER));
            let images = batch_images(schema, files, values.get(BATCH_TYPE));
            if images.iter().any(|i| out.batch_path(i) == *i) {
                return Some(REPLACES_INPUTS);
            }
        }
        let tracks = detectable(schema, cmd, values);
        // Tracks saved as flux need no format. HFE holds bitcells, which gw
        // makes from flux as read only with a format or a bitrate.
        let raw_flux = cmd.name == "read"
            || extension(input_file(cmd, values)).is_some_and(|e| RAW_FLUX.contains(&e.as_str()));
        let flux_out = FLUX.contains(&out.ext.as_str())
            && (out.ext != ".hfe" || out.opts.contains_key("bitrate") || !raw_flux);
        let format = match values.get("format") {
            "" => implied_format(schema, cmd, values, service),
            f => Some(f.to_owned()),
        };
        if !(tracks && flux_out) && format.is_none() {
            return Some(if tracks {
                "Select a disk format first, or press Detect."
            } else {
                "Select a disk format first."
            });
        }
        // Raw keeps the flux as read, with no format: gw's sector and track
        // types refuse it, and HFE takes it only at a set bitrate.
        if values.on("raw") && !FLUX.contains(&out.ext.as_str()) {
            return Some("With Raw on, select SCP, HFE or KryoFlux.");
        }
        if values.on("raw") && out.ext == ".hfe" && !out.opts.contains_key("bitrate") {
            return Some("With Raw on, HFE needs a bitrate. See Image options.");
        }
        // gw would stop at the first track, or once the whole disk is read.
        if let Some(format) = format.filter(|_| !values.on("raw")) {
            let diskdefs = known_diskdefs(service, values, &format);
            if service.known_fits(diskdefs, &format, &out.ext).is_some() {
                return Some("The image type cannot hold the disk format. See Image type.");
            }
        }
    }
    // Paths, not strings, as in the page's warning: /a//b is /a/b.
    let input = input_file(cmd, values);
    let same = !input.is_empty() && Path::new(input) == Path::new(output_file(cmd, values));
    same.then_some(REPLACES_INPUT)
}

/// A file dialog that opens where `current`, a file or a folder, is.
pub fn file_dialog(current: &Path) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new();
    let folder = match current.is_dir() {
        true => Some(current),
        false => current.parent().filter(|p| p.is_dir()),
    };
    match folder {
        Some(f) => dialog.set_directory(f),
        None => dialog,
    }
}

/// The dialog for an image: every type gw reads, then each alone, by name.
fn image_dialog(schema: &Schema, current: &str) -> rfd::FileDialog {
    let all = both_cases(schema.images.keys().map(|e| e.trim_start_matches('.')));
    let mut types: Vec<(String, Vec<String>)> = schema
        .images
        .iter()
        .map(|(e, i)| {
            let suffix = std::iter::once(e.trim_start_matches('.'));
            (image_name(e, &i.name), both_cases(suffix))
        })
        .collect();
    types.sort();
    let dialog = file_dialog(Path::new(current)).add_filter("Disk images", &all);
    types
        .into_iter()
        .fold(dialog, |d, (name, exts)| d.add_filter(name, &exts))
}

/// Suffixes for a dialog filter, in upper and lower case: GTK matches them by case.
fn both_cases<'e>(exts: impl Iterator<Item = &'e str>) -> Vec<String> {
    exts.flat_map(|e| [e.to_ascii_lowercase(), e.to_ascii_uppercase()])
        .collect()
}

/// Where a new image is saved.
fn folder_row(ui: &mut Ui, folder: &mut String, label: &str, enabled: bool) {
    let (name, _) = row(ui, label, |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                let width = beside_button(ui, BROWSE_BUTTON);
                path_edit(ui, folder, "Required", width)
                    .on_hover_text("Where the image is saved.")
                    .on_disabled_hover_text(BESIDE);
                if browse_button(ui)
                    .on_hover_text("Select a folder.")
                    .on_disabled_hover_text(BESIDE)
                    .clicked()
                    && let Some(f) = rfd::FileDialog::new().set_directory(&*folder).pick_folder()
                {
                    *folder = f.to_string_lossy().into_owned();
                }
            });
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
    // A KryoFlux stream is one image, taken by its first track's file; gw
    // refuses a .raw file named otherwise.
    let mut sets = BTreeSet::new();
    images.retain(|p| match extension(&p.to_string_lossy()).as_deref() {
        Some(KRYOFLUX) => {
            stream_set(&lossy(p.file_stem())).is_some_and(|set| sets.insert(p.with_file_name(set)))
        }
        _ => true,
    });
    images
}

/// A KryoFlux stream file's set, as gw takes it: `Game` for `Game00.0`.
fn stream_set(stem: &str) -> Option<&str> {
    let b = stem.as_bytes();
    let n = b.len();
    let track = n >= 4
        && b[n - 4].is_ascii_digit()
        && b[n - 3].is_ascii_digit()
        && b[n - 2] == b'.'
        && matches!(b[n - 1], b'0' | b'1');
    track.then(|| &stem[..n - 4])
}

/// The name for what is made from an input image: its own, less a KryoFlux
/// stream's track. A stream named by track alone, as KryoFlux's DTC names
/// it (track00.0.raw), takes its folder's name.
fn image_stem(input: &Path) -> String {
    let stem = lossy(input.file_stem());
    let stream = extension(&input.to_string_lossy()).as_deref() == Some(KRYOFLUX);
    let Some(set) = stream_set(&stem).filter(|_| stream) else {
        return stem;
    };
    let set = set.trim_end_matches(['_', '-', '.', ' ']);
    let folder = lossy(input.parent().and_then(Path::file_name));
    if (set.is_empty() || set.eq_ignore_ascii_case("track")) && !folder.is_empty() {
        folder
    } else if set.is_empty() {
        stem
    } else {
        set.to_owned()
    }
}

/// A file name of type `ext`. gw writes a KryoFlux stream as a set of files,
/// one per track, named from the first: `Game00.0.raw`.
fn typed_name(stem: &str, ext: &str) -> String {
    if ext != KRYOFLUX || stream_set(stem).is_some() {
        return format!("{stem}{ext}");
    }
    // Game_Disk1_00.0.raw, which does not read as disk 100.
    let gap = if stem.ends_with(|c: char| c.is_ascii_digit()) {
        "_"
    } else {
        ""
    };
    format!("{stem}{gap}00.0{ext}")
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
        // With Raw on the image keeps the flux, and the format only verifies it.
        let raw = values.on("raw") && FLUX.contains(&out.ext.as_str());
        if out.ext != ext && !raw {
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

/// As `diskdefs_for`, from what gw has already said of the file.
pub fn known_diskdefs<'v>(service: &Service, values: &'v Values, format: &str) -> &'v str {
    let path = values.get("diskdefs");
    match service
        .known_custom_formats(path)
        .iter()
        .any(|f| f == format)
    {
        true => path,
        false => "",
    }
}

/// Where an output argument's folder, name and type are kept.
pub fn output_key(command: &str, dest: &str) -> String {
    format!("{command}/{dest}")
}

/// The home folder as the system writes it in a path.
const HOME: &str = if cfg!(windows) { "%USERPROFILE%" } else { "~" };

/// `path` as shown: from HOME when in the home folder.
pub fn short_path(path: &str) -> Cow<'_, str> {
    let home = crate::home();
    match home
        .as_deref()
        .and_then(|h| Path::new(path).strip_prefix(h).ok())
    {
        Some(rest) if rest.as_os_str().is_empty() => HOME.into(),
        Some(rest) => format!("{HOME}{}{}", std::path::MAIN_SEPARATOR, rest.display()).into(),
        None => path.into(),
    }
}

/// A path as typed, made whole: HOME, in any case as Windows takes it, or
/// `~` is the home folder.
pub fn full_path(text: &str) -> String {
    let home = text
        .get(..HOME.len())
        .filter(|h| h.eq_ignore_ascii_case(HOME));
    match home.map(|h| &text[h.len()..]).or(text.strip_prefix('~')) {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => crate::home()
            .unwrap_or_default()
            .join(rest.trim_start_matches(['/', '\\']))
            .to_string_lossy()
            .into_owned(),
        _ => text.to_owned(),
    }
}

/// A box for a path, which `path` keeps whole. It shows the home folder as
/// HOME except while typed in, and takes it, or `~`, typed.
fn path_edit(ui: &mut Ui, path: &mut String, hint: &str, width: f32) -> egui::Response {
    let id = ui.next_auto_id();
    // What is typed, while it stands for the value, which a dropped file may change.
    let typed = ui.data(|d| d.get_temp::<String>(id));
    let typed = typed.filter(|t| ui.memory(|m| m.has_focus(id)) && full_path(t) == *path);
    let mut text = typed.unwrap_or_else(|| short_path(path).into_owned());
    let response = text_box_with(ui, id, edit(&mut text).hint_text(hint).desired_width(width));
    if response.changed() {
        *path = full_path(&text);
    }
    // Typing is left as typed until the box is left.
    let typing = response.has_focus();
    ui.data_mut(|d| {
        if typing {
            d.insert_temp(id, text);
        } else {
            d.remove_temp::<String>(id);
        }
    });
    response
}

/// Makes a pasted command's image paths whole, as a shell would: `~` is the
/// home folder. A relative path is taken in `base`, since gw's working
/// folder is whatever the app was started in.
pub fn anchor_images(command: &str, values: &mut Values, base: &Path) {
    let dests = OUTPUTS.iter().chain(BATCHES).filter(|(c, _)| *c == command);
    for (_, dest) in dests {
        let value = values.get(dest);
        if value.is_empty() {
            continue;
        }
        let path = PathBuf::from(full_path(value));
        let path = match path.has_root() {
            true => path,
            false => base.join(path),
        };
        values.set(dest, path.to_string_lossy());
    }
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

/// Whether Detect can find the format: of the disk in the drive, or of an
/// input that holds tracks, flux or decoded.
fn detectable(schema: &Schema, cmd: &Command, values: &Values) -> bool {
    cmd.name == "read"
        || schema
            .image(input_file(cmd, values))
            .is_some_and(|(_, i)| i.tracks)
}

/// Why the page's image cannot be written or converted: it is not there. gw
/// would stop before it opens the drive, but only after Write has asked.
pub fn missing_image(cmd: &Command, values: &Values) -> Option<&'static str> {
    let (_, dest) = BATCHES.iter().find(|(c, _)| *c == cmd.name)?;
    let value = values.get(dest);
    let (path, _) = split_opts(value);
    let missing = !path.is_empty() && !split_path(value) && !Path::new(path).is_file();
    missing.then_some("The image file does not exist.")
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

/// Whether gw verifies each track of a write with these arguments before it
/// goes on to the next: no --no-verify, and a format whose tracks gw checks,
/// as gw has already described it.
pub fn verifies(service: &mut Service, schema: &Schema, args: &[String]) -> bool {
    let arg = |flag: &str| args.iter().find_map(|a| a.strip_prefix(flag));
    let (Some(write), Some(file)) = (schema.command("write"), args.last()) else {
        return false;
    };
    if args.iter().any(|a| a == "--no-verify") {
        return false;
    }
    let mut values = Values::default();
    values.set("file", file);
    values.set("format", arg("--format=").unwrap_or_default());
    let Some(format) = effective_format(service, schema, write, &values) else {
        return false;
    };
    let info = service.known_format_info(arg("--diskdefs=").unwrap_or_default(), &format);
    info.and_then(Load::ready).is_some_and(|i| i.verifies)
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
fn sections(cmd: &Command) -> (Vec<&Arg>, Vec<&Arg>) {
    let shown = |a: &&Arg| !GLOBAL.contains(&a.dest.as_str());
    let args: Vec<&Arg> = cmd.args.iter().filter(shown).collect();
    match FIRST.iter().find(|(c, _)| *c == cmd.name) {
        Some((_, names)) => {
            let first = names
                .iter()
                .filter_map(|n| args.iter().find(|a| a.dest == *n).copied())
                .collect();
            let rest: Vec<&Arg> = args
                .into_iter()
                .filter(|a| !names.contains(&a.dest.as_str()))
                .collect();
            let rest = advanced_groups(&cmd.name, &rest)
                .into_iter()
                .flat_map(|(_, args)| args)
                .collect();
            (first, rest)
        }
        None if args.len() <= 6 => (args, Vec::new()),
        None => {
            let (first, rest): (Vec<&Arg>, Vec<&Arg>) =
                args.into_iter().partition(|a| a.positional() || a.required);
            // A page never opens on a shut header alone.
            match first.is_empty() {
                true => (rest, first),
                false => (first, rest),
            }
        }
    }
}

/// The Advanced options of `command` among `rest`, in ADVANCED's groups and
/// order: each heading, empty for none, with its arguments; the rest, such
/// as a newer gw's, last under no heading. Empty groups are left out.
fn advanced_groups<'a>(command: &str, rest: &[&'a Arg]) -> Vec<(&'static str, Vec<&'a Arg>)> {
    let groups = ADVANCED
        .iter()
        .find(|(c, _)| *c == command)
        .map_or(&[][..], |(_, g)| *g);
    let find = |n: &str| rest.iter().find(|a| a.dest == n).copied();
    let mut out: Vec<(&'static str, Vec<&Arg>)> = groups
        .iter()
        .map(|(heading, names)| (*heading, names.iter().filter_map(|n| find(n)).collect()))
        .filter(|(_, args): &(_, Vec<&Arg>)| !args.is_empty())
        .collect();
    let listed = |a: &&&Arg| {
        groups
            .iter()
            .any(|(_, names)| names.contains(&a.dest.as_str()))
    };
    let others: Vec<&Arg> = rest.iter().filter(|a| !listed(a)).copied().collect();
    if !others.is_empty() {
        out.push(("", others));
    }
    out
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
fn tip(command: &str, a: &Arg) -> String {
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
    ("", "diskdefs", "File containing custom disk definitions."),
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
    (
        "delays",
        "index_mask",
        "Index post-trigger mask time, in microseconds.",
    ),
    ("pin set", "level", "High or low."),
    ("clean", "linger", "Time on each step, in milliseconds."),
    (
        "delays",
        "motor",
        "Delay after turning on the spindle motor, in milliseconds.",
    ),
    ("seek", "motor_on", "Run the motor while seeking."),
    (
        "",
        "no_clobber",
        "Refuse a file that exists, as -n does. Off, Ferriteweazle asks first.",
    ),
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
    (
        "delays",
        "post_write",
        "Least time from a write's end to a track change, in microseconds.",
    ),
    ("write", "pre_erase", "Erase each track before writing it."),
    (
        "delays",
        "pre_write",
        "Least time from a track change to a write, in microseconds.",
    ),
    ("write", "precomp", "Write precompensation, by cylinder."),
    (
        "read",
        "raw",
        "Save the flux as read. A format only verifies it.",
    ),
    (
        "read",
        "retries",
        "Extra reads of a track with missing sectors, before each seek retry.",
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
    (
        "delays",
        "select",
        "Delay after asserting drive select, in microseconds.",
    ),
    (
        "delays",
        "settle",
        "Delay after a head seek completes, in milliseconds.",
    ),
    (
        "delays",
        "step",
        "Delay after each head-step command, in microseconds.",
    ),
    ("update", "tag", "The GitHub release tag to update to."),
    ("read", "tracks", "Which tracks to read."),
    ("write", "tracks", "Which tracks to write."),
    ("convert", "tracks", "Which tracks to read and convert."),
    ("erase", "tracks", "Which tracks to erase."),
    (
        "delays",
        "watchdog",
        "Idle time before drives deselect and motors stop, in milliseconds.",
    ),
];

/// An argument's tip, then gw's grammar for its value, as a hover.
fn explain(ui: &mut Ui, tip: &str, grammar: Option<&str>) {
    ui.label(tip);
    if let Some(g) = grammar {
        let text = RichText::new(g.trim_end()).monospace();
        ui.add(egui::Label::new(text).extend());
    }
}

/// gw's help for the kind of value an argument takes, such as TSPEC's.
fn grammar<'s>(schema: &'s Schema, a: &Arg) -> Option<&'s str> {
    let name = match a.ty.as_deref() {
        // gw 1.23 names this kind in its help but not on the argument.
        Some("PrecompSpec") => "PRECOMP",
        _ => a.metavar.as_deref()?,
    };
    schema.note(name)
}

/// gw's own example for a kind of value, such as `e.g. c=0-7,9-12:h=0-1` for TSPEC.
fn example(schema: &Schema, metavar: &str) -> Option<String> {
    let line = schema
        .note(metavar)?
        .lines()
        .find(|l| l.contains("e.g. '"))?;
    let quoted = line.split('\'').nth(1)?;
    Some(format!("e.g. {quoted}"))
}

/// An example of a value to type, or Required.
fn hint(a: &Arg, schema: &Schema) -> String {
    match a.ty.as_deref() {
        Some("period") => "e.g. 300rpm".into(),
        Some("PLL") => "e.g. period=5:phase=60".into(),
        Some("PrecompSpec") => example(schema, "PRECOMP").unwrap_or_default(),
        _ => match &a.default {
            Some(d) => format!("e.g. {d}"),
            // Density select's pin.
            None if a.dest == "pin" => "e.g. 2".into(),
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
pub(crate) fn type_for(schema: &Schema, format: &str) -> String {
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
    // A format with no fixed sectors, such as ibm.scan, gives 0 of each.
    let bytes = info.bytes.filter(|&b| b > 0);
    parts.extend(match info.sectors {
        Some((fewest, most)) if fewest == most && most > 0 => {
            Some(format!("{most} sectors per track"))
        }
        Some((fewest, most)) if most > 0 => Some(format!("{fewest} to {most} sectors per track")),
        _ => None,
    });
    parts.extend(bytes.map(|b| format!("{} KB", b / 1024)));
    // A narrow field wraps between facts, never inside one: "1440 KB" stays whole.
    let whole: Vec<String> = parts.iter().map(|p| p.replace(' ', "\u{a0}")).collect();
    whole.join(" · ")
}

/// Plain names for image options, where gw's own would not read well.
const OPTION_NAMES: &[(&str, &str)] = &[
    ("disktype", "Disk type"),
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

/// Plain names for gw's image types, by the machine or program they are
/// for. A type a newer gw adds shows gw's own name.
const KNOWN_IMAGES: &[(&str, &str)] = &[
    (".a2r", "Applesauce flux"),
    (".adf", "Amiga disk"),
    (".adl", "Acorn ADFS L"),
    (".adm", "Acorn ADFS M"),
    (".ads", "Acorn ADFS S"),
    (".ctr", "SPS CT Raw"),
    (".d1m", "CMD FD2000 DD"),
    (".d2m", "CMD FD2000 HD"),
    (".d4m", "CMD FD4000 ED"),
    (".d64", "Commodore 1541"),
    (".d71", "Commodore 1571"),
    (".d81", "Commodore 1581"),
    (".d88", "PC-98 D88"),
    (".dcp", "PC-98 DCP"),
    (".dim", "PC-98 DIM"),
    (".dmk", "TRS-80 DMK"),
    (".do", "Apple II DOS order"),
    (".dsd", "Acorn DFS double-sided"),
    // gw writes a plain sector image here, and reads a CPC one by its signature.
    (".dsk", "Sector image"),
    (".edsk", "Extended DSK"),
    (".fd", "Thomson"),
    (".fdi", "PC-98 FDI"),
    (".hdm", "PC-98 HDM"),
    (".hfe", "HxC floppy emulator"),
    (".ima", "Sector image"),
    (".img", "Sector image"),
    (".imd", "ImageDisk"),
    (".ipf", "SPS IPF"),
    (".mgt", "SAM Coupé or +D"),
    (".msa", "Atari ST MSA"),
    (".nfd", "PC-98 NFD"),
    (".nsi", "North Star"),
    (".po", "Apple II ProDOS order"),
    (".raw", "KryoFlux stream"),
    (".scp", "SuperCard Pro flux"),
    (".sf7", "Sega SF-7000"),
    (".ssd", "Acorn DFS single-sided"),
    (".st", "Atari ST"),
    (".td0", "Teledisk"),
    (".xdf", "PC-98 XDF"),
];

/// Whether gw would split a path in `value` at a `::` in it: an option never
/// holds a folder separator.
fn split_path(value: &str) -> bool {
    value
        .split_once("::")
        .is_some_and(|(_, opts)| opts.contains(['/', '\\']))
}

/// Options set that a type with `options` does not take: gw refuses them.
fn foreign<'o>(set: &'o BTreeMap<String, String>, options: &[ImageOpt]) -> Vec<&'o str> {
    let taken = |k: &&String| options.iter().any(|o| o.name == **k);
    set.keys()
        .filter(|k| !taken(k))
        .map(String::as_str)
        .collect()
}

/// Names the options set that the image type does not take.
fn foreign_label(ui: &mut Ui, names: &[&str]) {
    let text = format!("This image type takes no option {}.", names.join(", "));
    ui.label(RichText::new(text).small().color(theme::palette(ui).bad));
}

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

fn lossy(s: Option<&OsStr>) -> String {
    s.map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// A drop-down `width` wide of `options`, after `unset` unless the value is
/// required and before Other…, whose box the caller draws. Returns whether
/// the value changed, and whether Other… is chosen.
fn drop_down<'o>(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut String,
    unset: &str,
    required: bool,
    width: f32,
    options: impl Iterator<Item = &'o str> + Clone,
) -> (bool, bool) {
    let other_id = id.with("other");
    let listed = value.is_empty() || options.clone().any(|o| o == value);
    let mut other = !listed || ui.data(|d| d.get_temp(other_id)).unwrap_or(false);
    let shown = match (other, value.as_str()) {
        (true, _) => RichText::new(OTHER),
        (false, "") if required => RichText::new("Required").color(theme::palette(ui).dim),
        (false, "") => RichText::new(unset),
        (false, v) => RichText::new(v),
    };
    let mut chosen = None;
    sized(ui, width, |ui| {
        egui::ComboBox::from_id_salt(id)
            .selected_text(shown)
            .truncate()
            .width(width)
            .show_ui(ui, |ui| {
                if !required
                    && ui
                        .selectable_label(!other && value.is_empty(), unset)
                        .clicked()
                {
                    chosen = Some((false, ""));
                }
                for o in options {
                    if ui.selectable_label(!other && value == o, o).clicked() {
                        chosen = Some((false, o));
                    }
                }
                if ui.selectable_label(other, OTHER).clicked() {
                    chosen = Some((true, ""));
                }
            })
    });
    if let Some((o, v)) = chosen {
        other = o;
        *value = v.to_owned();
    }
    ui.data_mut(|d| d.insert_temp(other_id, other));
    (chosen.is_some(), other)
}

/// Common values for image options that gw names none for.
const OPTION_VALUES: &[(&str, &[&str])] = &[
    ("bitrate", &["125", "250", "300", "500"]),
    // gw takes these alone.
    ("version", &["1", "3"]),
];

/// What an image option does, by its name.
const OPTION_TIPS: &[(&str, &str)] = &[
    (
        "bitrate",
        "Bit rate, in kbit/s. Unset, the format gives it.",
    ),
    ("disktype", "Disk type in the SCP header."),
    (
        "double_step",
        "Mark the image double-stepped, for 40 tracks in an 80-track drive.",
    ),
    ("encoding", "Track encoding in the HFE header."),
    (
        "index",
        "Which disk of a multi-disk image, counting from 0.",
    ),
    ("interface", "Interface mode in the HFE header."),
    (
        "legacy_ss",
        "Lay out a single-sided image the old, wrong way, for older tools.",
    ),
    ("revs", "Revolutions to save per track."),
    (
        "sck",
        "Sample clock for flux timings, in Hz, or MHz with m: 72m.",
    ),
    (
        "uniform",
        "Keep one bit rate throughout, dropping variable-rate timings.",
    ),
    ("version", "HFE version: 1, or 3 for HFEv3."),
];

/// "Default (2)" for a default gw has, else "Default".
fn default_label(default: Option<&str>) -> String {
    default.map_or_else(|| "Default".to_owned(), |d| format!("Default ({d})"))
}

/// An image option's default as shown: gw's name for it, or a whole number.
fn option_default(opt: &ImageOpt) -> Option<String> {
    match opt.default.as_ref()? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(format!("{:.0}", n.as_f64()?)),
        _ => None,
    }
}

/// Fields for an image type's own options, and gw's complaint about each
/// value. Returns true if one changed.
fn image_options(
    ui: &mut Ui,
    service: &mut Service,
    ext: &str,
    options: &[ImageOpt],
    values: &mut BTreeMap<String, String>,
) -> bool {
    let tip = |name: &str| {
        OPTION_TIPS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, t)| *t)
    };
    let mut changed = false;
    egui::Grid::new(("image options", ext))
        .num_columns(2)
        .spacing(vec2(ROW_GAP, 6.0))
        .show(ui, |ui| {
            for opt in options.iter().filter(|o| !o.flag()) {
                let name = ui.label(plain(OPTION_NAMES, &opt.name));
                let value = values.entry(opt.name.clone()).or_default();
                let common = OPTION_VALUES.iter().find(|(n, _)| *n == opt.name);
                let common = common.map_or(&[][..], |(_, v)| *v);
                let listed = opt
                    .choices
                    .iter()
                    .map(String::as_str)
                    .chain(common.iter().copied());
                let field = ui.vertical(|ui| {
                    // Narrower in a small window, so it ends within the page.
                    let width = OPTION_FIELD.min(ui.available_width());
                    if opt.choices.is_empty() && common.is_empty() {
                        // gw saves every revolution read unless told how many.
                        let all = (opt.name == "revs").then_some("all".to_owned());
                        let default = option_default(opt).or(all);
                        let hint = default_label(default.as_deref());
                        let edit = edit(value).hint_text(hint).desired_width(width);
                        changed |= text_box(ui, edit).changed();
                        return;
                    }
                    let unset = default_label(option_default(opt).as_deref());
                    let id = ui.make_persistent_id(("image option", ext, &opt.name));
                    let (chose, other) = drop_down(ui, id, value, &unset, false, width, listed);
                    changed |= chose;
                    if other {
                        changed |= text_box(ui, edit(value).desired_width(width)).changed();
                    }
                });
                if let Some(tip) = tip(&opt.name) {
                    name.on_hover_text(tip);
                    field.response.on_hover_text(tip);
                }
                ui.end_row();
            }
        });
    ui.horizontal_wrapped(|ui| {
        for opt in options.iter().filter(|o| o.flag()) {
            let value = values.entry(opt.name.clone()).or_default();
            let mut on = !value.is_empty();
            let check = checkbox(ui, &mut on, &plain(OPTION_NAMES, &opt.name));
            if check.changed() {
                *value = if on { ON.to_owned() } else { String::new() };
                changed = true;
            }
            if let Some(tip) = tip(&opt.name) {
                check.on_hover_text(tip);
            }
        }
    });
    values.retain(|_, v| !v.is_empty());
    for (name, value) in values.iter() {
        if let Some(e) = service.check_opt(ext, name, value) {
            ui.label(
                RichText::new(sentence(e))
                    .small()
                    .color(theme::palette(ui).bad),
            );
        }
    }
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
    /// The number of the first disk a session reads, to carry on a set.
    pub first: u32,
    /// The digits `first` was typed with, 2 for 01: every disk number has at least as many.
    pub first_digits: u32,
    /// The word before each disk number: `Disk` in `Game_Disk1`.
    pub label: String,
    /// The disk number goes before the name, not after it.
    pub number_first: bool,
    /// Each disk number is followed by the set's size: `Disk01_of_12`.
    pub total: bool,
    /// Each disk's name is asked for with the disk, not numbered.
    pub ask_names: bool,
    pub ext: String,
    pub opts: BTreeMap<String, String>,
    /// Take the folder and name from the input file, when there is one.
    pub beside_input: bool,
    /// Added to each input's name in a batch: `Backup` in `Backup_Game`.
    pub batch_label: String,
    /// The batch label goes before the name, not after it.
    pub label_first: bool,
    /// The input the name was taken from: a new input names the image again.
    pub named_for: String,
    /// The most times to read a disk while sectors are missing: 1 to MAX_PASSES.
    pub passes: u32,
    /// Later passes read the whole disk, not only the tracks missing sectors.
    pub whole_disk: bool,
    /// Each pass's flux is saved in PASSES_FOLDER.
    pub keep_passes: bool,
}

impl Default for Output {
    fn default() -> Self {
        Output {
            folder: images_folder().to_string_lossy().into_owned(),
            name: "Floppy".into(),
            disks: 1,
            first: 1,
            first_digits: 1,
            label: String::new(),
            number_first: false,
            total: false,
            ask_names: false,
            ext: String::new(),
            opts: BTreeMap::new(),
            beside_input: false,
            batch_label: String::new(),
            label_first: false,
            named_for: String::new(),
            passes: 1,
            whole_disk: false,
            keep_passes: false,
        }
    }
}

impl Output {
    /// A set whose disks are named as they are asked for.
    pub fn asks_names(&self) -> bool {
        self.disks > 1 && self.ask_names
    }

    /// This output for one disk named `name`.
    pub fn named(&self, name: &str) -> Output {
        Output {
            name: name.to_owned(),
            disks: 1,
            ..self.clone()
        }
    }

    /// An output from a path as gw takes it, such as a pasted command's.
    pub fn from_value(value: &str) -> Output {
        let (path, opts) = split_opts(value);
        let path = Path::new(path);
        Output {
            folder: lossy(path.parent().map(Path::as_os_str)),
            name: lossy(path.file_stem()),
            ext: extension(value).unwrap_or_default(),
            opts,
            ..Output::default()
        }
    }

    /// The file for one disk, counting from 1: `Game_Disk2.adf` of three.
    fn file_name(&self, disk: u32) -> String {
        let (name, ext) = (&self.name, &self.ext);
        if self.disks <= 1 || self.ask_names {
            return typed_name(name, ext);
        }
        let width = self.first_digits as usize;
        let mut number = format!("{}{disk:0width$}", self.label.trim());
        if self.total {
            number += &format!("_of_{:0width$}", self.disks);
        }
        let stem = match self.number_first {
            true => format!("{number}_{name}"),
            false => format!("{name}_{number}"),
        };
        typed_name(&stem, ext)
    }

    pub fn path(&self, disk: u32) -> PathBuf {
        PathBuf::from(&self.folder).join(self.file_name(disk))
    }

    /// The first disk a session reads, within the set.
    pub fn first_disk(&self) -> u32 {
        match self.ask_names {
            true => 1,
            false => self.first.clamp(1, self.disks.max(1)),
        }
    }

    /// Every file a session makes, in order.
    pub fn paths(&self) -> impl Iterator<Item = PathBuf> + '_ {
        (self.first_disk()..=self.disks.max(1)).map(|d| self.path(d))
    }

    /// The value gw takes for one disk: the path and any image options.
    /// Empty until there is a type and a name.
    pub fn value(&self, disk: u32) -> String {
        if self.ext.is_empty() || self.name.trim().is_empty() {
            return String::new();
        }
        join_opts(&self.path(disk).to_string_lossy(), &self.opts)
    }

    /// The file a batch makes from `input`: its name with the label before or
    /// after it, in the input's folder when beside it.
    pub fn batch_path(&self, input: &Path) -> PathBuf {
        let folder = match self.beside_input {
            true => input.parent().unwrap_or(Path::new("")),
            false => Path::new(&self.folder),
        };
        let (label, stem) = (self.batch_label.trim(), image_stem(input));
        let stem = match (label.is_empty(), self.label_first) {
            (true, _) => stem,
            (false, true) => format!("{label}_{stem}"),
            (false, false) => format!("{stem}_{label}"),
        };
        folder.join(typed_name(&stem, &self.ext))
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
        let (first, last) = (self.first_disk(), self.disks.max(1));
        let path = self.path(first).to_string_lossy().into_owned();
        match first == last || self.ask_names {
            true => path,
            false => format!("{path} … {}", self.file_name(last)),
        }
    }

    fn preview_names(&self) -> String {
        let (first, last) = (self.first_disk(), self.disks.max(1));
        match last - first {
            0..=2 => {
                let names: Vec<String> = (first..=last).map(|d| self.file_name(d)).collect();
                names.join(", ")
            }
            _ => format!(
                "{}, {} … {}",
                self.file_name(first),
                self.file_name(first + 1),
                self.file_name(last)
            ),
        }
    }
}

/// A track list with the head step Detect measured in place of its own, or
/// None if it has that step already.
pub fn with_step(tracks: &str, step: u32) -> Option<String> {
    let mut spec = TrackSpec::parse(tracks);
    if !spec.half() && spec.steps() == Some(step) {
        return None;
    }
    // A half step's stride goes with it.
    if spec.half() {
        spec.c = spec
            .c
            .map(|c| c.strip_suffix("/2").unwrap_or(&c).to_owned());
    }
    spec.step = (step != 1).then(|| step.to_string());
    Some(spec.to_string())
}

/// The cylinders and sides a page takes of a disk of `cyls` and `heads`, as gw
/// works them out from its track lists: for Convert, those it writes.
pub fn page_tracks(values: &Values, (cyls, heads): (u32, u32)) -> (Vec<u32>, Vec<u32>) {
    let disk = ((0..cyls).collect(), (0..heads).collect());
    take(values.get("out_tracks"), take(values.get("tracks"), disk))
}

/// The cylinders and sides a track list names, else those it is given.
fn take(list: &str, (cyls, heads): (Vec<u32>, Vec<u32>)) -> (Vec<u32>, Vec<u32>) {
    let spec = TrackSpec::parse(list);
    let named = |part: Option<&str>, or: Vec<u32>| {
        let mut set = part.and_then(crate::progress::numbers).unwrap_or(or);
        set.sort_unstable();
        set.dedup();
        set
    };
    (
        named(spec.c.as_deref(), cyls),
        named(spec.h.as_deref(), heads),
    )
}

/// The format a page's settings fit, once the page is seen, and whether its
/// head step waits for gw to size that format.
#[derive(Debug, Default)]
pub struct FormatFit {
    format: Option<Option<String>>,
    sizing: bool,
}

/// Fits a page's settings to its format whenever that changes, by any route:
/// the cylinders and sides, and an HFE's bitrate, become the new format's,
/// as gw takes them unset. A head step belongs to the drive and stays, unless
/// the format so stepped would pass LAST_USUAL_CYLINDER. A page not yet
/// seen is taken as it is. True if the page changed.
pub fn fit_format(
    service: &mut Service,
    schema: &Schema,
    cmd: &Command,
    values: &mut Values,
    outputs: &mut BTreeMap<String, Output>,
    fit: &mut FormatFit,
) -> bool {
    if cmd.arg("format").is_none() || cmd.arg("tracks").is_none() {
        return false;
    }
    let format = effective_format(service, schema, cmd, values);
    // gw has yet to say which format the input file holds.
    if format.is_none()
        && format_in_file(schema, cmd, values)
        && matches!(
            service.image_format(input_file(cmd, values)),
            Load::Waiting(_)
        )
    {
        return false;
    }
    let mut changed = false;
    let put = |values: &mut Values, dest: &str, spec: &TrackSpec| {
        let spec = spec.to_string();
        let new = values.get(dest) != spec;
        values.set(dest, spec);
        new
    };
    match &fit.format {
        None => fit.format = Some(format),
        Some(was) if *was == format => {}
        Some(_) => {
            let mut spec = TrackSpec::parse(values.get("tracks"));
            let step = spec.steps().filter(|&s| s > 1);
            spec.refit();
            // With no format, as the picker keeps it to what the drive reaches.
            if let Some(step) = step
                && format.is_none()
            {
                spec.c = Some(format!("0-{}", reach(step) - 1));
            }
            changed |= put(values, "tracks", &spec);
            if cmd.arg("out_tracks").is_some() {
                let mut out = TrackSpec::parse(values.get("out_tracks"));
                out.refit();
                changed |= put(values, "out_tracks", &out);
            }
            // Flux as read has no bitrate of its own to give.
            if format.is_some() && !values.on("raw") {
                for (_, dest) in OUTPUTS.iter().filter(|(c, _)| *c == cmd.name) {
                    let out = outputs.get_mut(&output_key(&cmd.name, dest));
                    if let Some(out) = out.filter(|o| o.ext == ".hfe") {
                        changed |= out.opts.remove("bitrate").is_some();
                    }
                }
            }
            fit.sizing = step.is_some() && format.is_some();
            fit.format = Some(format);
        }
    }
    if fit.sizing
        && let Some(Some(name)) = &fit.format
    {
        let diskdefs = diskdefs_for(service, values, name);
        match service.format_info(&diskdefs, name) {
            Load::Waiting(_) => {}
            Load::Failed(_) => fit.sizing = false,
            Load::Ready(info) => {
                fit.sizing = false;
                let mut spec = TrackSpec::parse(values.get("tracks"));
                let step = spec.steps().unwrap_or(1);
                if info.cyls.saturating_sub(1).saturating_mul(step) > LAST_USUAL_CYLINDER {
                    spec.step = None;
                    changed |= put(values, "tracks", &spec);
                }
            }
        }
    }
    changed
}

/// The furthest physical cylinder of a track list over a format of `cyls`
/// cylinders, as gw steps and offsets to it: None for a list the picker cannot
/// read, or one that stays below cylinder 0.
pub fn last_cylinder(tracks: &str, cyls: Option<u32>) -> Option<u32> {
    let spec = TrackSpec::parse(tracks);
    if !spec.other.is_empty() {
        return None;
    }
    let last = match spec.c {
        Some(_) => spec.cylinders()?.1,
        None => cyls?.checked_sub(1)?,
    };
    let last = match spec.half() {
        true => i64::from(last / 2),
        false => i64::from(last.checked_mul(spec.steps()?)?),
    };
    // An offset counts only for a side the list reads.
    let off = (0..2)
        .filter(|&h| spec.has_head(h, 2))
        .map(|h| spec.off[h as usize]);
    u32::try_from(last + i64::from(off.max()?)).ok()
}

/// What gw does with the cylinders a track list names past a format's `cyls`,
/// if it names any: it decodes nothing there, and a sector image holds nothing
/// there. None for a read with --raw, which keeps their flux. `sectors`: the
/// image written is a sector image, which has no track there to skip.
fn past_format(
    command: &str,
    format: &str,
    spec: &TrackSpec,
    cyls: u32,
    raw: bool,
    sectors: bool,
) -> Option<String> {
    let mut past: Vec<u32> = spec
        .c
        .as_deref()
        .and_then(crate::progress::numbers)?
        .into_iter()
        .filter(|&c| c >= cyls)
        .collect();
    past.sort_unstable();
    past.dedup();
    let (&first, &last) = (past.first()?, past.last()?);
    if command == "read" && raw {
        return None;
    }
    let which = match past.len() {
        1 => format!("cylinder {first} is"),
        n if n == (last - first + 1) as usize => format!("cylinders {first} to {last} are"),
        _ => {
            let list: Vec<String> = past.iter().map(u32::to_string).collect();
            format!("cylinders {} are", list.join(", "))
        }
    };
    let fate = match (command, sectors) {
        ("read", _) => "read but not decoded or saved. Raw keeps the flux",
        ("write", true) => "skipped, or erased with Erase empty tracks",
        ("write", false) => {
            "skipped, or erased with Erase empty tracks where the image has no track"
        }
        _ => "skipped",
    };
    Some(format!("{format} has {cyls} cylinders, so {which} {fate}."))
}

/// Which tracks, in gw's notation: `c=0-79:h=0:step=2:hswap`.
#[derive(Debug, Default, Clone, PartialEq)]
struct TrackSpec {
    c: Option<String>,
    h: Option<String>,
    step: Option<String>,
    hswap: bool,
    /// Cylinders each side's head moves by: gw's h0.off and h1.off.
    off: [i32; 2],
    /// Parts the picker does not show, such as several cylinder ranges.
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
                Some((k @ ("h0.off" | "h1.off"), v)) => match offset(v) {
                    Some(n) => spec.off[usize::from(k == "h1.off")] = n,
                    None => spec.other.push(part.to_owned()),
                },
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
            && self.step.as_deref().is_none_or(shown_step)
            && (!self.half() || self.c.as_deref().is_some_and(|c| c.ends_with("/2")))
            && self.off.iter().all(|o| o.abs() <= MAX_OFFSET)
    }

    /// Leaves the cylinders and sides to a new format, as gw takes them unset. A
    /// half step pairs with the old format's cylinders, so goes with them.
    fn refit(&mut self) {
        (self.c, self.h) = (None, None);
        if self.half() {
            self.step = None;
        }
    }

    /// A half step, which the picker pairs with every other cylinder: `c=0-81/2`.
    fn half(&self) -> bool {
        self.step.as_deref() == Some(HALF)
    }

    /// Head steps per cylinder, if a whole number.
    fn steps(&self) -> Option<u32> {
        self.step.as_deref().map_or(Some(1), |s| s.parse().ok())
    }

    /// The sides alone: this list's, else `base`'s, as gw takes unset output tracks.
    fn sides(&self, base: &TrackSpec) -> TrackSpec {
        TrackSpec {
            h: self.h.clone().or_else(|| base.h.clone()),
            ..TrackSpec::default()
        }
    }

    /// Takes one of STEPS from the picker, which shows cylinders `shown` of
    /// `whole`. A half step takes every other one of them, and another step
    /// drops that stride. With no format (`free`), a list kept to the drive's
    /// reach at the old step takes the new step's.
    fn pick_step(&mut self, value: &str, shown: (u32, u32), whole: (u32, u32), free: bool) {
        let reached = |s: u32| (s > 1).then(|| format!("0-{}", reach(s) - 1));
        let kept = free && self.c == reached(self.steps().unwrap_or(1));
        if value == HALF {
            let (a, b) = if kept { (0, USUAL_DISK.0 - 1) } else { shown };
            self.c = Some(format!("{a}-{b}/2"));
            self.step = Some(HALF.into());
            return;
        }
        if self.half() {
            self.c = (shown != whole).then(|| format!("{}-{}", shown.0, shown.1));
        }
        let n: u32 = value.parse().expect("every step but HALF is a number");
        if kept || free && self.c.is_none() {
            self.c = reached(n);
        }
        self.step = (n != 1).then(|| value.to_owned());
    }

    fn cylinders(&self) -> Option<(u32, u32)> {
        let c = self.c.as_deref()?;
        let c = c.strip_suffix("/2").filter(|_| self.half()).unwrap_or(c);
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
        for (head, off) in self.off.iter().enumerate().filter(|(_, o)| **o != 0) {
            parts.push(format!("h{head}.off={off:+}"));
        }
        parts.extend(self.other.iter().cloned());
        f.write_str(&parts.join(":"))
    }
}

/// A head offset as gw takes it, signed: `+8` or `-8`.
fn offset(v: &str) -> Option<i32> {
    let digits = v.strip_prefix(['+', '-'])?;
    match !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        true => v.parse().ok(),
        false => None,
    }
}

/// A group's heading, with a rule to the fields' right edge, or to what `end`
/// lays out there.
fn heading(ui: &mut Ui, text: &str, end: impl FnOnce(&mut Ui)) -> egui::Response {
    let p = theme::palette(ui);
    ui.scope(|ui| {
        // As tall as its words, not a row of fields.
        ui.spacing_mut().interact_size.y = 0.0;
        ui.horizontal(|ui| {
            let label = ui.label(RichText::new(text).small().strong().color(p.dim));
            let right = egui::Layout::right_to_left(egui::Align::Center);
            let end = ui.with_layout(right, end).response.rect;
            let stop = match end.width() > 0.0 {
                true => end.left() - 10.0,
                false => end.right(),
            };
            let y = label.rect.center().y;
            let rule = label.rect.right() + 10.0..=stop;
            ui.painter().hline(rule, y, egui::Stroke::new(1.0, p.line));
            label
        })
        .inner
    })
    .inner
}

/// A checkbox as a row's field, centred on the row's label.
fn row_checkbox(ui: &mut Ui, on: &mut bool, text: &str) -> egui::Response {
    ui.horizontal(|ui| checkbox(ui, on, text)).inner
}

/// A checkbox with a clean tick, drawn here: egui's own is lopsided.
fn checkbox(ui: &mut Ui, on: &mut bool, text: &str) -> egui::Response {
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

/// An on/off switch, named `label` for screen readers.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let (enabled, state) = (ui.is_enabled(), *on);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, state, label)
    });
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, state);
        let p = theme::palette(ui);
        let fill = theme::lerp(p.line_strong, p.accent, t);
        ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
        let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
        // Round by its corners, not a circle, so Classic squares it.
        let knob = egui::Rect::from_center_size(pos2(x, rect.center().y), vec2(14.0, 14.0));
        ui.painter().rect_filled(knob, 7, Color32::WHITE);
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
        serde_json::from_str(include_str!("gw-1.23.json")).unwrap()
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
        page_with(command, values, outputs, false)
    }

    /// As `page`, with gw standalone or not.
    fn page_with(
        command: &str,
        values: Values,
        outputs: BTreeMap<String, Output>,
        standalone: bool,
    ) -> Harness<'static, Page> {
        page_set(command, values, outputs, standalone, |_| {})
    }

    /// As `page_with`, with `setup` done to the service first, such as describing a format.
    fn page_set(
        command: &str,
        values: Values,
        outputs: BTreeMap<String, Output>,
        standalone: bool,
        setup: impl FnOnce(&mut Service),
    ) -> Harness<'static, Page> {
        let schema = schema();
        let cmd = schema.command(command).unwrap().clone();
        let mut service = Service::offline(Ok(schema.clone()));
        setup(&mut service);
        // Tall enough for the read page with Advanced options open.
        let mut h = Harness::builder()
            .with_size(vec2(800.0, 900.0))
            .build_ui_state(
                move |ui, (values, outputs): &mut Page| {
                    let form = Form {
                        schema: &schema,
                        cmd: &cmd,
                        values,
                        outputs,
                        service: &mut service,
                        cannot_detect: None,
                        adafruit: false,
                        standalone,
                        reported: None,
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
        let needs = Some("Select a disk format first, or press Detect.");
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
    fn raw_waits_for_a_type_that_holds_flux_and_a_format_keeps_it() {
        let s = schema();
        let read = s.command("read").unwrap();
        let service = Service::offline(Ok(s.clone()));
        let why = |out: Output| {
            let v = values(&[("format", "ibm.1440"), ("raw", ON)]);
            let outputs = BTreeMap::from([(output_key("read", "file"), out)]);
            blocked(&s, read, &v, &outputs, &service)
        };
        let flux = Some("With Raw on, select SCP, HFE or KryoFlux.");
        assert_eq!(why(output(".img")), flux);
        assert_eq!(why(output(".imd")), flux);
        assert_eq!(why(output(".scp")), None);
        assert_eq!(why(output(".raw")), None);
        let bitrate = Some("With Raw on, HFE needs a bitrate. See Image options.");
        assert_eq!(why(output(".hfe")), bitrate, "the format does not give it");
        let mut hfe = output(".hfe");
        hfe.opts.insert("bitrate".into(), "250".into());
        assert_eq!(why(hfe), None);

        let mut v = values(&[("raw", ON)]);
        let mut outputs = BTreeMap::from([(output_key("read", "file"), output(".scp"))]);
        choose_format(&s, read, &mut v, &mut outputs, "amiga.amigados");
        assert_eq!(outputs["read/file"].ext, ".scp", "the format only verifies");
        v.set("raw", "");
        choose_format(&s, read, &mut v, &mut outputs, "amiga.amigados");
        assert_eq!(outputs["read/file"].ext, ".adf");
    }

    #[test]
    fn a_track_image_converts_to_flux_with_no_format_and_offers_detect() {
        let s = schema();
        let convert = s.command("convert").unwrap();
        let service = Service::offline(Ok(s.clone()));
        let why = |input: &str, out: Output| {
            let v = values(&[("in_file", input)]);
            let outputs = BTreeMap::from([(output_key("convert", "out_file"), out)]);
            blocked(&s, convert, &v, &outputs, &service)
        };
        assert_eq!(
            why("/f/a.imd", output(".hfe")),
            None,
            "its tracks have a bitrate"
        );
        assert_eq!(why("/f/a.edsk", output(".scp")), None);
        let detect = Some("Select a disk format first, or press Detect.");
        assert_eq!(why("/f/a.imd", output(".img")), detect);
        let sectors = Some("Select a disk format first.");
        assert_eq!(
            why("/f/a.dsk", output(".scp")),
            sectors,
            "a .dsk may hold sectors"
        );
        let h = page(
            "convert",
            values(&[("in_file", "/f/a.imd")]),
            BTreeMap::new(),
        );
        h.get_by_label("Detect");
    }

    #[test]
    fn a_plain_sector_image_waits_for_a_format_before_a_write() {
        let s = schema();
        let write = s.command("write").unwrap();
        let service = Service::offline(Ok(s.clone()));
        let none = BTreeMap::new();
        let why = |pairs: &[(&str, &str)]| blocked(&s, write, &values(pairs), &none, &service);
        let needs = Some("Select a disk format first.");
        assert_eq!(why(&[("file", "/f/a.img")]), needs);
        assert_eq!(why(&[("file", "/f/a.st")]), needs);
        assert_eq!(why(&[("file", "/f/a.img"), ("format", "ibm.1440")]), None);
        for own in ["/f/a.adf", "/f/a.dsk", "/f/a.msa", "/f/a.scp"] {
            assert_eq!(why(&[("file", own)]), None, "{own}");
        }
    }

    #[test]
    fn a_missing_image_stops_write_and_convert_before_they_ask() {
        let s = schema();
        let dir =
            std::env::temp_dir().join(format!("ferriteweazle-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let game = dir.join("Game.adf");
        let file = |dest: &str| values(&[(dest, &game.to_string_lossy())]);
        let missing = |command: &str, v: &Values| missing_image(s.command(command).unwrap(), v);
        let gone = Some("The image file does not exist.");
        assert_eq!(missing("write", &file("file")), gone);
        assert_eq!(missing("convert", &file("in_file")), gone);
        assert_eq!(
            missing("write", &Values::default()),
            None,
            "none chosen yet"
        );
        std::fs::write(&game, [0u8; 512]).unwrap();
        assert_eq!(missing("write", &file("file")), None);
        assert_eq!(missing("convert", &file("in_file")), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_option_an_images_type_does_not_take_is_shown_and_stops_the_page() {
        let s = schema();
        let service = Service::offline(Ok(s.clone()));
        let none = BTreeMap::new();
        let v = values(&[("file", "/f/disk.hfe::bitrate=250")]);
        let write = s.command("write").unwrap();
        assert_eq!(blocked(&s, write, &v, &none, &service), Some(FOREIGN));
        let h = page("write", v, BTreeMap::new());
        h.get_by_label("This image type takes no option bitrate.");

        let read = s.command("read").unwrap();
        let v = values(&[("format", "ibm.1440")]);
        let pasted = Output::from_value("/f/x.img::index=1");
        let outputs = BTreeMap::from([(output_key("read", "file"), pasted)]);
        assert_eq!(blocked(&s, read, &v, &outputs, &service), Some(FOREIGN));
        let h = page("read", v, outputs);
        h.get_by_label("This image type takes no option index.");
    }

    #[test]
    fn a_page_that_makes_an_image_says_whether_it_lacks_the_type_or_the_name() {
        let s = schema();
        let read = s.command("read").unwrap();
        let service = Service::offline(Ok(s.clone()));
        let v = values(&[("format", "ibm.1440")]);
        let mut outputs = BTreeMap::new();
        let why = |outputs: &_| blocked(&s, read, &v, outputs, &service);
        assert_eq!(why(&outputs), Some("Select an image type first."));
        let unnamed = Output {
            name: " ".into(),
            ..output(".img")
        };
        outputs.insert(output_key("read", "file"), unnamed);
        assert_eq!(why(&outputs), Some("Name the image first."));
    }

    #[test]
    fn a_conversion_is_named_after_its_input_until_renamed() {
        let key = output_key("convert", "out_file");
        let v = values(&[("in_file", "/d/Game.scp")]);
        let outputs = BTreeMap::from([(key.clone(), output(".adf"))]);
        let mut h = page("convert", v, outputs);
        assert_eq!(h.state().1[&key].name, "Game");
        h.state_mut().1.get_mut(&key).unwrap().name = "Mine".into();
        h.run();
        assert_eq!(h.state().1[&key].name, "Mine", "a name typed holds");
        h.state_mut().0.set("in_file", "/d/Other.scp");
        h.run();
        assert_eq!(h.state().1[&key].name, "Other");
    }

    #[test]
    fn a_pasted_output_path_becomes_folder_name_and_type() {
        let out = Output::from_value("/disks/Game Disk.HFE::version=3");
        assert_eq!(
            (out.folder.as_str(), out.name.as_str(), out.ext.as_str()),
            ("/disks", "Game Disk", ".hfe")
        );
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(out.value(1), format!("/disks{sep}Game Disk.hfe::version=3"));
    }

    #[test]
    fn an_output_needs_a_type_gw_writes_and_a_whole_folder() {
        let s = schema();
        let service = Service::offline(Ok(s.clone()));
        let why = |command: &str, out: Output| {
            let (_, dest) = OUTPUTS.iter().find(|(c, _)| *c == command).unwrap();
            let v = values(&[("format", "ibm.1440"), ("in_file", "/f/a.scp")]);
            let outputs = BTreeMap::from([(output_key(command, dest), out)]);
            blocked(&s, s.command(command).unwrap(), &v, &outputs, &service)
        };
        let ipf = Output::from_value("/f/b.ipf");
        assert_eq!(
            why("convert", ipf),
            Some("Greaseweazle Tools cannot write this image type.")
        );
        let unknown = Output::from_value("/f/b.xyz");
        assert_eq!(
            why("convert", unknown),
            Some("Greaseweazle Tools does not know this image type.")
        );
        let folder = |f: &str| Output {
            folder: f.into(),
            ..output(".img")
        };
        assert_eq!(why("read", folder(" ")), Some("Select a folder first."));
        for relative in ["Images", "~/Disks"] {
            let full = Some("Type the folder's full path.");
            assert_eq!(why("read", folder(relative)), full, "{relative}");
        }
        assert_eq!(why("read", output(".img")), None);
    }

    #[test]
    fn a_path_gw_would_split_at_its_double_colon_cannot_run() {
        let s = schema();
        let service = Service::offline(Ok(s.clone()));
        let read = s.command("read").unwrap();
        let v = values(&[("format", "ibm.1440")]);
        let why = |out: Output| {
            let outputs = BTreeMap::from([(output_key("read", "file"), out)]);
            blocked(&s, read, &v, &outputs, &service)
        };
        let colons = Some(
            "Greaseweazle Tools reads :: in a path as options. Select another folder or name.",
        );
        let folder = Output {
            folder: "/Volumes/x::y".into(),
            ..output(".img")
        };
        assert_eq!(why(folder), colons);
        let name = Output {
            name: "a::b".into(),
            ..output(".img")
        };
        assert_eq!(why(name), colons);
        let write = s.command("write").unwrap();
        let v = values(&[("file", "/Volumes/x::y/Game.adf")]);
        let none = BTreeMap::new();
        assert_eq!(blocked(&s, write, &v, &none, &service), Some(COLONS_IN));
        let v = values(&[("file", "/f/Game.d88::index=1")]);
        assert_eq!(blocked(&s, write, &v, &none, &service), None, "options");
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
    fn of_two_exclusive_options_set_at_once_either_can_be_cleared() {
        let s = schema();
        let read = s.command("read").unwrap();
        let mut service = Service::offline(Ok(s.clone()));
        let mut outputs = BTreeMap::new();
        let mut blocker = |v: &mut Values, dest: &str| {
            let form = Form {
                schema: &s,
                cmd: read,
                values: v,
                outputs: &mut outputs,
                service: &mut service,
                cannot_detect: None,
                adafruit: false,
                standalone: false,
                reported: None,
            };
            form.blocker(read.arg(dest).unwrap())
                .map(|b| b.dest.clone())
        };
        let mut v = values(&[("hard_sectors", ON)]);
        assert_eq!(
            blocker(&mut v, "fake_index").as_deref(),
            Some("hard_sectors")
        );
        assert_eq!(blocker(&mut v, "hard_sectors"), None);
        v.set("fake_index", "300rpm");
        assert_eq!(blocker(&mut v, "fake_index"), None);
        assert_eq!(blocker(&mut v, "hard_sectors"), None);
    }

    #[test]
    fn an_image_of_another_type_drops_the_options_its_type_does_not_take() {
        let v = values(&[("file", "/f/game.d88::index=1")]);
        let mut h = page("write", v, BTreeMap::new());
        let mut retype = |path: &str| {
            h.get_all_by_role(Role::TextInput).next().unwrap().click();
            h.run();
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.run();
            h.event(egui::Event::Text(path.to_owned()));
            h.run();
            h.state().0.get("file").to_owned()
        };
        assert_eq!(retype("/f/disk.d88"), "/f/disk.d88::index=1");
        assert_eq!(retype("/f/game.scp"), "/f/game.scp");
    }

    #[test]
    fn a_required_argument_offers_no_default() {
        let h = page("pin set", Values::default(), BTreeMap::new());
        h.get_by_label("High");
        assert!(h.query_by_label("Default").is_none());

        let mut h = page("seek", Values::default(), BTreeMap::new());
        let cylinder = h.get_by_role(Role::ComboBox);
        assert_eq!(cylinder.value().as_deref(), Some("Required"));
        cylinder.click();
        h.run();
        h.get_by_label("40");
        assert!(h.query_by_label_contains("Default").is_none());
    }

    #[test]
    fn an_empty_field_shows_gws_default_as_its_value() {
        let h = page("clean", Values::default(), BTreeMap::new());
        let shown: Vec<_> = h
            .get_all_by_role(Role::TextInput)
            .filter_map(|t| t.accesskit_node().placeholder().map(str::to_owned))
            .collect();
        assert_eq!(shown, ["80", "3", "100"], "cylinders, passes, linger");
    }

    #[test]
    fn other_shows_gws_default_as_an_example() {
        // A format, so Revolutions waits for gw's word and Retries alone says 3.
        let v = values(&[("format", "ibm.1440")]);
        let mut h = page("read", v, BTreeMap::new());
        h.get_by_label_contains("Advanced options").click();
        h.run();
        let retries = h
            .get_all_by_role(Role::ComboBox)
            .find(|c| c.value().as_deref() == Some("Default (3)"))
            .expect("Retries' list");
        retries.click();
        h.run();
        // The list's Other…, in its popup after the page, not the step's.
        h.get_all_by_label(OTHER).last().unwrap().click();
        h.run();
        let shown: Vec<_> = h
            .get_all_by_role(Role::TextInput)
            .filter_map(|t| t.accesskit_node().placeholder().map(str::to_owned))
            .collect();
        assert!(shown.iter().any(|s| s == "e.g. 3"), "{shown:?}");
    }

    #[test]
    fn the_pin_field_shows_pin_2_as_its_example() {
        let s = schema();
        for cmd in ["pin get", "pin set"] {
            let pin = s.command(cmd).unwrap().arg("pin").unwrap();
            assert_eq!(hint(pin, &s), "e.g. 2", "gw {cmd}");
        }
    }

    #[test]
    fn a_switch_is_named_by_its_row() {
        let h = page("write", Values::default(), BTreeMap::new());
        h.get_by_role_and_label(Role::CheckBox, "Skip verify");
    }

    #[test]
    fn detect_promises_an_image_type_only_where_the_page_makes_one() {
        let mut h = page("write", values(&[("file", "/f/x.scp")]), BTreeMap::new());
        h.get_by_label("Detect").hover();
        h.run();
        h.get_by_label("Attempt to find the disk format.");
    }

    #[test]
    fn with_no_format_the_track_picker_offers_the_82_cylinders_gw_uses() {
        let h = page("erase", Values::default(), BTreeMap::new());
        let cylinders: Vec<_> = h
            .get_all_by_role(Role::SpinButton)
            .take(2)
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
    fn the_last_cylinder_a_track_list_steps_to_is_physical() {
        assert_eq!(last_cylinder("", Some(80)), Some(79), "the format's own");
        assert_eq!(last_cylinder("", Some(82)), Some(81));
        assert_eq!(
            last_cylinder("c=0-81", Some(80)),
            Some(81),
            "the list's own"
        );
        assert_eq!(last_cylinder("c=0-39:step=2", Some(40)), Some(78));
        assert_eq!(last_cylinder("h=0:step=2", Some(42)), Some(82));
        assert_eq!(last_cylinder("c=0-26:step=3", None), Some(78));
        assert_eq!(last_cylinder("c=0-83/2:step=1/2", None), Some(41));
        assert_eq!(last_cylinder("c=0-83/2:step=1/2:h0.off=+2", None), Some(43));
        assert_eq!(last_cylinder("c=5", None), Some(5));
        assert_eq!(last_cylinder("c=0-79:h0.off=+8", Some(80)), Some(87));
        assert_eq!(
            last_cylinder("c=0-79:h1.off=-8", Some(80)),
            Some(79),
            "side 0 goes furthest"
        );
        assert_eq!(
            last_cylinder("c=0-79:h=0:h1.off=+8", Some(80)),
            Some(79),
            "side 1 is not read"
        );
        // Only gw reads these: the bridge checks each seek instead.
        assert_eq!(last_cylinder("c=0-9,20-29", Some(80)), None);
        assert_eq!(last_cylinder("c=0-79:h1.off=8", Some(80)), None, "unsigned");
        assert_eq!(last_cylinder("c=0-4:h=1:h1.off=-8", None), None, "below 0");
        assert_eq!(last_cylinder("", None), None, "no format known");
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
        out.batch_label = " copy ".into();
        let after = "Game_Disk1_copy.hfe";
        assert_eq!(out.batch_path(input), Path::new("/out").join(after));
        out.label_first = true;
        let before = "copy_Game_Disk1.hfe";
        assert_eq!(out.batch_path(input), Path::new("/out").join(before));
        out.beside_input = true;
        assert_eq!(out.batch_path(input), Path::new("/in").join(before));
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
            Some("Select a folder of images first.")
        );
        let dir = std::env::temp_dir().join(format!("ferriteweazle-batch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        v.set(BATCH_FOLDER, dir.to_string_lossy());
        service.folder(&dir.to_string_lossy());
        assert_eq!(
            reason(&v, &service, &none),
            Some("The folder has no images Greaseweazle Tools can read.")
        );

        std::fs::write(dir.join("Game.img"), [0u8; 512]).unwrap();
        // A fresh service: one that listed the folder may keep that listing for
        // FOLDER_EVERY where the folder's modified time is coarse.
        let mut service = Service::offline(Ok(s.clone()));
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
        outputs.get_mut("convert/out_file").unwrap().batch_label = "copy".into();
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
                let elsewhere = GLOBAL.contains(&a.dest.as_str());
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
    fn advanced_options_keep_a_set_order_under_headings_then_any_a_newer_gw_adds() {
        let s = schema();
        let dests = |cmd: &str| -> Vec<String> {
            let (_, rest) = sections(s.command(cmd).unwrap());
            rest.iter().map(|a| a.dest.clone()).collect()
        };
        assert_eq!(
            dests("read"),
            [
                "diskdefs",
                "retries",
                "seek_retries",
                "pll",
                "adjust_speed",
                "densel",
                "gen_tg43",
                "fake_index",
                "hard_sectors",
                "reverse",
                "raw",
                "no_clobber"
            ]
        );
        assert_eq!(
            dests("write"),
            [
                "diskdefs",
                "pre_erase",
                "erase_empty",
                "retries",
                "precomp",
                "densel",
                "gen_tg43",
                "fake_index",
                "hard_sectors",
                "reverse"
            ]
        );
        assert_eq!(
            dests("convert"),
            [
                "diskdefs",
                "pll",
                "adjust_speed",
                "hard_sectors",
                "reverse",
                "no_clobber",
                "out_tracks"
            ]
        );
        for (cmd, groups) in ADVANCED {
            // A command from a newer gw may be missing from this one.
            let Some(cmd) = s.command(cmd) else { continue };
            for (_, names) in *groups {
                for n in *names {
                    assert!(cmd.arg(n).is_some(), "gw {} has no {n}", cmd.name);
                }
            }
        }
        let mut read = s.command("read").unwrap().clone();
        let new = Arg {
            flags: vec!["--new".into()],
            dest: "new_thing".into(),
            switch: true,
            ty: None,
            default: None,
            choices: Vec::new(),
            required: false,
            group: None,
            metavar: None,
            help: String::new(),
        };
        read.args.insert(3, new);
        let (_, rest) = sections(&read);
        let groups = advanced_groups("read", &rest);
        let headings: Vec<&str> = groups.iter().map(|(h, _)| *h).collect();
        assert_eq!(headings, ["", "Reading", "Drive", "Image", ""]);
        assert_eq!(
            groups.last().map(|(_, a)| a[0].dest.as_str()),
            Some("new_thing"),
            "a newer gw's option goes last"
        );
        let mut h = page("read", Values::default(), BTreeMap::new());
        h.get_by_label_contains("Advanced options").click();
        h.run();
        for heading in ["Reading", "Drive", "Image"] {
            h.get_by_label(heading);
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
        assert_eq!(why(&v), Some("Select an update file first."));
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
        let spec = TrackSpec::parse("c=0-39:h=1:step=1/2:hswap:h1.off=+1");
        assert_eq!(spec.cylinders(), Some((0, 39)));
        assert!(!spec.simple(), "a half step without its stride");
        assert_eq!(spec.to_string(), "c=0-39:h=1:step=1/2:hswap:h1.off=+1");
    }

    #[test]
    fn the_picker_shows_head_offsets_steps_and_a_paired_half_step_as_gw_takes_them() {
        let spec = TrackSpec::parse("c=0-39:h1.off=-8:step=4:h0.off=+0");
        assert_eq!((spec.off, spec.steps()), ([0, -8], Some(4)));
        assert!(spec.simple());
        assert_eq!(spec.to_string(), "c=0-39:step=4:h1.off=-8");
        let half = TrackSpec::parse("c=0-81/2:step=1/2");
        assert!(half.simple() && half.half());
        assert_eq!(half.cylinders(), Some((0, 81)));
        for step in ["step=0", "step=5", "step=9", "c=0-8:step=9:h1.off=-8"] {
            assert!(TrackSpec::parse(step).simple(), "{step}: gw's step=[0-9]");
        }
        for typed in ["h1.off=8", "h1.off=-12", "step=12", "step=1/2", "c=0-81/2"] {
            let spec = TrackSpec::parse(typed);
            assert!(!spec.simple(), "{typed}");
            assert_eq!(spec.to_string(), typed, "kept as typed");
        }
    }

    #[test]
    fn detects_step_takes_the_place_of_the_track_lists() {
        let with = |tracks: &str, step| with_step(tracks, step);
        assert_eq!(with("", 2).as_deref(), Some("step=2"));
        assert_eq!(with("c=0-39:h=0", 2).as_deref(), Some("c=0-39:h=0:step=2"));
        assert_eq!(with("step=1", 2).as_deref(), Some("step=2"), "measured");
        assert_eq!(with("c=0-26:step=3", 2).as_deref(), Some("c=0-26:step=2"));
        assert_eq!(
            with("c=0-81/2:step=1/2", 2).as_deref(),
            Some("c=0-81:step=2"),
            "a half step's stride goes with it"
        );
        assert_eq!(with("c=0-39:step=2", 1).as_deref(), Some("c=0-39"));
        assert_eq!(with("c=0-27:step=3", 1).as_deref(), Some("c=0-27"));
        assert_eq!(with("h=0:c=0-39", 1), None, "kept as typed");
        assert_eq!(with("h1.off=-8:step=2", 2), None);
    }

    #[test]
    fn a_step_with_no_format_keeps_to_the_cylinders_the_drive_reaches() {
        let mut h = page("erase", Values::default(), BTreeMap::new());
        let mut step = |n: &str| {
            h.get_all_by_label(n).last().unwrap().click();
            h.run();
            let last = h.get_all_by_role(Role::SpinButton).nth(1).unwrap();
            let last = last.accesskit_node().numeric_value();
            (h.state().0.get("tracks").to_owned(), last)
        };
        assert_eq!(step("2"), ("c=0-40:step=2".into(), Some(40.0)));
        assert_eq!(step("3"), ("c=0-27:step=3".into(), Some(27.0)));
        assert_eq!(step("½"), ("c=0-81/2:step=1/2".into(), Some(81.0)));
        assert_eq!(step("2"), ("c=0-40:step=2".into(), Some(40.0)));
        assert_eq!(step("1"), (String::new(), Some(81.0)));
    }

    #[test]
    fn a_half_step_takes_every_other_cylinder_of_the_range_shown() {
        let mut h = page("erase", Values::default(), BTreeMap::new());
        h.get_by_label("½").click();
        h.run();
        let last = h.get_all_by_role(Role::SpinButton).nth(1).unwrap();
        last.click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("39".into()));
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(h.state().0.get("tracks"), "c=0-39/2:step=1/2");
        h.get_all_by_label("1").last().unwrap().click();
        h.run();
        assert_eq!(
            h.state().0.get("tracks"),
            "c=0-39",
            "the range, not its stride"
        );
    }

    #[test]
    fn head_offsets_are_set_per_side_and_greyed_for_a_side_not_read() {
        let v = values(&[("tracks", "c=0-39:h1.off=-8")]);
        let mut h = page("erase", v, BTreeMap::new());
        let offsets = |h: &Harness<'_, Page>| {
            let spins: Vec<_> = h.get_all_by_role(Role::SpinButton).collect();
            let n = spins.len();
            spins[n - 2..]
                .iter()
                .map(|s| s.accesskit_node().numeric_value())
                .collect::<Vec<_>>()
        };
        assert_eq!(offsets(&h), [Some(0.0), Some(-8.0)], "shown, not typed");
        let side0 = h.get_all_by_role(Role::SpinButton).nth(2).unwrap();
        side0.click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("+8".into()));
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(h.state().0.get("tracks"), "c=0-39:h0.off=+8:h1.off=-8");

        let v = values(&[("tracks", "c=0-39:h=0:h1.off=-8")]);
        let mut h = page("erase", v, BTreeMap::new());
        h.get_all_by_role(Role::SpinButton).last().unwrap().hover();
        h.run();
        h.get_by_label("Needs side 1.");
    }

    #[test]
    fn a_text_box_has_cut_copy_and_paste_on_a_right_click_and_keeps_its_selection() {
        let mut h = page("read", Values::default(), BTreeMap::new());
        // The first box: the image's folder.
        h.get_all_by_role(Role::TextInput).next().unwrap().click();
        h.run();
        h.event(egui::Event::Text("abc".into()));
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run();
        h.get_all_by_role(Role::TextInput)
            .next()
            .unwrap()
            .click_secondary();
        h.run();
        assert!(
            !h.get_by_label("Cut").accesskit_node().is_disabled(),
            "the selection is kept"
        );
        h.get_by_label("Paste").click();
        h.step();
        let pasted = h
            .output()
            .viewport_output
            .values()
            .flat_map(|v| &v.commands)
            .any(|c| *c == ViewportCommand::RequestPaste);
        assert!(pasted, "the system pastes");
    }

    #[test]
    fn revolutions_show_the_default_gw_resolves_for_the_page() {
        let value = |command: &str, text: &str| {
            let mut h = page(command, Values::default(), BTreeMap::new());
            // Erase keeps its revolutions under Advanced options.
            if let Some(more) = h.query_by_label_contains("Advanced options") {
                more.click();
            }
            h.run();
            h.query_all_by_role(Role::ComboBox)
                .any(|c| c.value().as_deref() == Some(text))
        };
        assert!(value("read", "Default (3)"), "with no format, gw reads 3");
        assert!(value("erase", "Default (1)"), "gw erase's own default");
    }

    #[test]
    fn revolutions_default_is_the_formats_and_raw_rounds_a_fraction_to_two() {
        let shown = |raw: bool, revs: f64| {
            let mut v = values(&[("format", "amiga.amigados")]);
            if raw {
                v.set("raw", ON);
            }
            let mut h = page_set("read", v, BTreeMap::new(), false, |s| {
                s.describe("amiga.amigados", 80, 2);
                s.describe_revs("amiga.amigados", revs);
            });
            h.run();
            h.query_all_by_role(Role::ComboBox)
                .filter_map(|c| c.value())
                .find(|v| v.starts_with("Default ("))
                .unwrap_or_default()
        };
        assert_eq!(
            shown(false, 1.1),
            "Default (1.1)",
            "a timed fraction past one"
        );
        assert_eq!(
            shown(true, 1.1),
            "Default (2)",
            "raw reads whole revolutions"
        );
        assert_eq!(shown(false, 2.0), "Default (2)");
    }

    #[test]
    fn other_takes_the_rest_of_gws_steps_in_a_number_box() {
        let mut h = page("erase", Values::default(), BTreeMap::new());
        let spins = |h: &Harness<'_, Page>| h.get_all_by_role(Role::SpinButton).count();
        let before = spins(&h);
        h.get_by_label("Other…").click();
        h.run();
        assert_eq!(spins(&h), before + 1, "the step's box");
        let set = |h: &mut Harness<'_, Page>, text: &str| {
            h.get_all_by_role(Role::SpinButton).nth(2).unwrap().click();
            h.run();
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.event(egui::Event::Text(text.into()));
            h.key_press(egui::Key::Enter);
            h.run();
            h.state().0.get("tracks").to_owned()
        };
        assert_eq!(set(&mut h, "7"), "c=0-11:step=7", "the drive's reach at 7");
        assert_eq!(
            set(&mut h, "0"),
            "step=0",
            "gw's step=0: every cylinder from the drive's 0"
        );
        h.get_all_by_label("2").last().unwrap().click();
        h.run();
        assert_eq!(h.state().0.get("tracks"), "c=0-40:step=2");
        assert_eq!(spins(&h), before, "a button shuts the box");
        h.get_by_label("Other…").click();
        h.run();
        assert_eq!(spins(&h), before + 1);
        h.state_mut().0.set("tracks", "c=0-26:step=3");
        h.run();
        assert_eq!(
            spins(&h),
            before,
            "a preset or a typed list changing the step shuts it"
        );
        let v = values(&[("tracks", "c=0-8:step=9")]);
        let h = page("erase", v, BTreeMap::new());
        assert_eq!(spins(&h), before + 1, "a step past the buttons opens it");
    }

    #[test]
    fn a_track_list_past_the_format_says_what_gw_does_there() {
        let spec = TrackSpec::parse;
        let past = |cmd, list, raw| past_format(cmd, "ibm.1440", &spec(list), 80, raw, true);
        assert_eq!(past("read", "c=0-79", false), None, "within the format");
        assert_eq!(past("read", "h=0", false), None, "the format's own");
        assert_eq!(
            past("read", "c=0-83", false).as_deref(),
            Some(
                "ibm.1440 has 80 cylinders, so cylinders 80 to 83 are read but not decoded \
                 or saved. Raw keeps the flux."
            )
        );
        assert_eq!(past("read", "c=0-83", true), None, "raw keeps the flux");
        assert_eq!(
            past("write", "c=0-80", false).as_deref(),
            Some(
                "ibm.1440 has 80 cylinders, so cylinder 80 is skipped, or erased with \
                 Erase empty tracks."
            )
        );
        assert_eq!(
            past("convert", "c=0-83/2:step=1/2", false).as_deref(),
            Some("ibm.1440 has 80 cylinders, so cylinders 80, 82 are skipped.")
        );
        // A flux image may hold the track: gw then finds it, cannot decode it, and skips it.
        assert_eq!(
            past_format("write", "ibm.1440", &spec("c=0-80"), 80, false, false).as_deref(),
            Some(
                "ibm.1440 has 80 cylinders, so cylinder 80 is skipped, or erased with \
                 Erase empty tracks where the image has no track."
            )
        );
    }

    #[test]
    fn read_passes_need_a_format_and_their_options_more_than_one_pass() {
        let read = |format: &str, passes: u32, standalone: bool| {
            let out = Output {
                passes,
                keep_passes: true,
                ..output(".img")
            };
            let outputs = BTreeMap::from([(output_key("read", "file"), out)]);
            let mut h = page_with("read", values(&[("format", format)]), outputs, standalone);
            h.get_by_label_contains("Read passes").click();
            h.run();
            h
        };
        let hover_passes = |h: &mut Harness<Page>| {
            h.get_all_by_role(Role::SpinButton).last().unwrap().hover();
            h.run();
        };
        let mut h = read("", 3, false);
        hover_passes(&mut h);
        h.get_by_label("Needs a disk format.");
        let mut h = read("ibm.1440", 3, true);
        hover_passes(&mut h);
        h.get_by_label("Standalone Greaseweazle Tools cannot read in passes.");
        let mut h = read("ibm.1440", 1, false);
        h.get_by_label("Each pass").hover();
        h.run();
        h.get_by_label("Needs more than one read pass.");

        let h = read("ibm.1440", 3, false);
        h.get_by_label("Read passes (3)");
        h.get_by_label("Read passes/Floppy pass 1.scp, Floppy pass 2.scp…");
    }

    #[test]
    fn a_page_takes_the_tracks_its_lists_name_of_the_disk() {
        let tracks = |pairs: &[(&str, &str)]| page_tracks(&values(pairs), (80, 2));
        let cyls = |n: u32| (0..n).collect::<Vec<_>>();
        assert_eq!(tracks(&[]), (cyls(80), vec![0, 1]));
        assert_eq!(
            tracks(&[("tracks", "c=0-39:h=1:step=2")]),
            (cyls(40), vec![1])
        );
        assert_eq!(
            tracks(&[("tracks", "c=9,0-4/2,2")]),
            (vec![0, 2, 4, 9], vec![0, 1])
        );
        let convert = [("tracks", "c=0-9"), ("out_tracks", "h=0")];
        assert_eq!(tracks(&convert), (cyls(10), vec![0]));
        assert_eq!(tracks(&[("tracks", "c=x")]), (cyls(80), vec![0, 1]));
    }

    #[test]
    fn a_new_format_brings_its_own_cylinders_and_sides_by_any_route() {
        let s = schema();
        let read = s.command("read").unwrap();
        let mut service = Service::offline(Ok(s.clone()));
        let mut fit = |command: &str, v: &mut Values, f: &mut FormatFit| {
            let cmd = s.command(command).unwrap();
            fit_format(&mut service, &s, cmd, v, &mut BTreeMap::new(), f)
        };
        // The format list.
        let (mut v, mut f) = (values(&[("format", "ibm.1440")]), FormatFit::default());
        fit("read", &mut v, &mut f);
        v.set("tracks", "c=0-39:h=0");
        assert!(
            !fit("read", &mut v, &mut f),
            "a track list set by hand stays"
        );
        choose_format(&s, read, &mut v, &mut BTreeMap::new(), "ibm.1440");
        fit("read", &mut v, &mut f);
        assert_eq!(v.get("tracks"), "c=0-39:h=0", "the same format again");
        choose_format(&s, read, &mut v, &mut BTreeMap::new(), "ibm.360");
        assert!(fit("read", &mut v, &mut f));
        assert_eq!(v.get("tracks"), "");

        // The image type, with no format chosen.
        let (mut v, mut f) = (values(&[("file", "/f/x.adf")]), FormatFit::default());
        fit("read", &mut v, &mut f);
        v.set("tracks", "c=0-9");
        v.set("file", "/f/x.d64");
        fit("read", &mut v, &mut f);
        assert_eq!(v.get("tracks"), "");

        // The image to write, whose swapped sides are the drive's.
        let (mut v, mut f) = (values(&[("file", "/f/a.adf")]), FormatFit::default());
        fit("write", &mut v, &mut f);
        v.set("tracks", "c=0-9:hswap");
        v.set("file", "/f/b.adf");
        fit("write", &mut v, &mut f);
        assert_eq!(
            v.get("tracks"),
            "c=0-9:hswap",
            "another image of the same format"
        );
        v.set("file", "/f/c.d64");
        fit("write", &mut v, &mut f);
        assert_eq!(v.get("tracks"), "hswap");
        // An image that holds its format waits for gw to read it.
        v.set("tracks", "c=0-9");
        v.set("file", "/f/d.nsi");
        fit("write", &mut v, &mut f);
        assert_eq!(v.get("tracks"), "c=0-9");

        // A conversion's output tracks too.
        let pairs = [
            ("in_file", "/f/a.adf"),
            ("tracks", "c=0-9"),
            ("out_tracks", "c=0-9:h=0:step=2"),
        ];
        let (mut v, mut f) = (values(&pairs), FormatFit::default());
        fit("convert", &mut v, &mut f);
        v.set("in_file", "/f/a.d64");
        fit("convert", &mut v, &mut f);
        assert_eq!((v.get("tracks"), v.get("out_tracks")), ("", "step=2"));

        // An HFE's bitrate, which a format gives and flux as read cannot.
        let convert = s.command("convert").unwrap();
        let mut hfe = output(".hfe");
        hfe.opts.insert("bitrate".into(), "500".into());
        let mut o = BTreeMap::from([(output_key("convert", "out_file"), hfe.clone())]);
        let pairs = [("in_file", "/f/a.scp"), ("out_file", &hfe.value(1))];
        let (mut v, mut f) = (values(&pairs), FormatFit::default());
        fit_format(&mut service, &s, convert, &mut v, &mut o, &mut f);
        v.set("in_file", "/f/a.adf");
        fit_format(&mut service, &s, convert, &mut v, &mut o, &mut f);
        assert_eq!(o["convert/out_file"].opts.get("bitrate"), None);
        o.insert(output_key("convert", "out_file"), hfe.clone());
        v.set("in_file", "/f/b.scp");
        fit_format(&mut service, &s, convert, &mut v, &mut o, &mut f);
        assert!(
            o["convert/out_file"].opts.contains_key("bitrate"),
            "no format"
        );
        let mut o = BTreeMap::from([(output_key("read", "file"), hfe)]);
        let pairs = [("format", "ibm.720"), ("raw", ON)];
        let (mut v, mut f) = (values(&pairs), FormatFit::default());
        fit_format(&mut service, &s, read, &mut v, &mut o, &mut f);
        choose_format(&s, read, &mut v, &mut o, "ibm.1440");
        fit_format(&mut service, &s, read, &mut v, &mut o, &mut f);
        assert!(
            o["read/file"].opts.contains_key("bitrate"),
            "Raw keeps the flux"
        );
    }

    #[test]
    fn a_step_stays_with_the_drive_unless_the_new_format_would_pass_cylinder_83() {
        let s = schema();
        let read = s.command("read").unwrap();
        let mut service = Service::offline(Ok(s.clone()));
        service.describe("ibm.1440", 80, 2);
        service.describe("c64.28", 28, 1);
        service.describe("c64.42", 42, 1);
        service.describe("c64.43", 43, 1);
        // Detect's double step for a 40-track disk, or a step of 3, then another format.
        let mut refit = |step: &str, to: &str| {
            let tracks = format!("c=0-26:step={step}");
            let mut v = values(&[("format", "ibm.360"), ("tracks", tracks.as_str())]);
            let (mut o, mut f) = (BTreeMap::new(), FormatFit::default());
            fit_format(&mut service, &s, read, &mut v, &mut o, &mut f);
            v.set("format", to);
            fit_format(&mut service, &s, read, &mut v, &mut o, &mut f);
            v.get("tracks").to_owned()
        };
        assert_eq!(refit("2", "ibm.1440"), "", "double stepped, 80 reach 158");
        assert_eq!(refit("2", "c64.42"), "step=2", "42 reach 82");
        assert_eq!(refit("2", "c64.43"), "", "43 reach 84");
        assert_eq!(refit("3", "c64.28"), "step=3", "28 reach 81");
        assert_eq!(refit("3", "c64.42"), "", "42 reach 123");
        assert_eq!(
            refit("1/2", "c64.42"),
            "",
            "a half step goes with the format"
        );
        assert_eq!(
            refit("2", ""),
            "c=0-40:step=2",
            "no format: what the drive reaches"
        );
        assert_eq!(refit("3", ""), "c=0-27:step=3");
        assert_eq!(refit("60000000", "ibm.1440"), "", "no overflow");

        // A format gw has yet to size, as a standalone gw never does, keeps it.
        let mut v = values(&[("format", "ibm.360"), ("tracks", "step=2")]);
        let (mut o, mut f) = (BTreeMap::new(), FormatFit::default());
        fit_format(&mut service, &s, read, &mut v, &mut o, &mut f);
        v.set("format", "ibm.720");
        fit_format(&mut service, &s, read, &mut v, &mut o, &mut f);
        assert_eq!(v.get("tracks"), "step=2");
        service.describe("ibm.720", 80, 2);
        fit_format(&mut service, &s, read, &mut v, &mut o, &mut f);
        assert_eq!(v.get("tracks"), "");
    }

    #[test]
    fn a_new_format_takes_a_half_step_from_both_of_converts_track_lists() {
        let s = schema();
        let convert = s.command("convert").unwrap();
        let mut service = Service::offline(Ok(s.clone()));
        service.describe("ibm.720", 80, 2);
        let half = "c=0-39/2:step=1/2";
        let mut v = values(&[
            ("format", "ibm.360"),
            ("tracks", half),
            ("out_tracks", half),
        ]);
        let (mut o, mut f) = (BTreeMap::new(), FormatFit::default());
        fit_format(&mut service, &s, convert, &mut v, &mut o, &mut f);
        v.set("format", "ibm.720");
        fit_format(&mut service, &s, convert, &mut v, &mut o, &mut f);
        assert_eq!((v.get("tracks"), v.get("out_tracks")), ("", ""));
    }

    #[test]
    fn output_tracks_unset_show_the_cylinders_and_sides_read() {
        let v = values(&[("in_file", "/f/a.scp"), ("tracks", "c=0-39:h=0")]);
        let mut h = page("convert", v, BTreeMap::new());
        h.get_by_label_contains("Advanced options").click();
        h.run();
        // Each picker's cylinders, then its head offsets.
        let ends: Vec<_> = h
            .get_all_by_role(Role::SpinButton)
            .map(|c| c.accesskit_node().numeric_value())
            .collect();
        assert_eq!(ends, [0.0, 39.0, 0.0, 0.0, 0.0, 39.0, 0.0, 0.0].map(Some));
        // The output picker's, after the input picker's side 1 and step 1.
        let side1 = h
            .get_all_by_role_and_label(Role::Button, "1")
            .nth(2)
            .unwrap();
        let lit = side1.accesskit_node().toggled();
        assert_eq!(lit, Some(egui::accesskit::Toggled::False));
    }

    #[test]
    fn a_track_list_is_a_group_of_its_own_after_the_pages_other_rows() {
        let h = page("read", Values::default(), BTreeMap::new());
        let top = |label: &str| h.get_by_label_contains(label).rect().top();
        assert!(top("Image settings") < top("Track settings"));
        assert!(
            top("Track settings") < top("Cylinders"),
            "shown, not shut away"
        );
        assert!(top("Cylinders") < top("Multiple disks"));
        let h = page("erase", Values::default(), BTreeMap::new());
        h.get_by_label("Track settings");
        h.get_by_label("Cylinders");
    }

    #[test]
    fn headings_group_the_first_rows_by_what_gw_does_with_them() {
        let groups: [(&str, &[&str]); 3] = [
            (
                "read",
                &[
                    "Disk settings",
                    "Disk format",
                    "Revolutions",
                    "Image settings",
                    "Image type",
                ],
            ),
            (
                "write",
                &[
                    "Image settings",
                    "Image",
                    "Disk settings",
                    "Disk format",
                    "Skip verify",
                ],
            ),
            (
                "convert",
                &[
                    "Input settings",
                    "Input",
                    "Disk settings",
                    "Output settings",
                    "Image type",
                ],
            ),
        ];
        for (name, rows) in groups {
            let h = page(name, Values::default(), BTreeMap::new());
            let top = |r: &&str| h.get_by_role_and_label(Role::Label, r).rect().top();
            let tops: Vec<f32> = rows.iter().map(top).collect();
            assert!(tops.is_sorted(), "gw {name}: {rows:?} at {tops:?}");
        }
    }

    #[test]
    fn a_track_list_the_picker_cannot_show_says_so_on_its_link() {
        let v = values(&[("tracks", "c=0-7,9-12")]);
        let mut h = page("read", v, BTreeMap::new());
        h.get_by_label("Use the track picker").hover();
        h.run();
        h.get_by_label("The track picker cannot show this track list.");
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
            label: "Disk".into(),
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
        (out.label, out.first_digits) = ("Side".into(), 2);
        assert_eq!(out.file_name(2), "Side02_Game.adf");
        out.label.clear();
        assert_eq!(out.file_name(12), "12_Game.adf");
    }

    #[test]
    fn a_kryoflux_stream_is_named_as_gw_takes_it() {
        let mut out = output(".raw");
        assert_eq!(out.file_name(1), "Floppy00.0.raw");
        out.name = "Disk7".into();
        assert_eq!(out.file_name(1), "Disk7_00.0.raw", "not disk 700");
        (out.name, out.label, out.disks) = ("Game".into(), "Disk".into(), 2);
        assert_eq!(out.file_name(2), "Game_Disk2_00.0.raw");
        let pasted = Output::from_value("/f/Game00.0.raw");
        assert_eq!(
            pasted.file_name(1),
            "Game00.0.raw",
            "a set's own name stays"
        );
        out.folder = "/out".into();
        let made = out.batch_path(Path::new("/in/Game.scp"));
        assert_eq!(made, Path::new("/out").join("Game00.0.raw"));
    }

    #[test]
    fn a_kryoflux_stream_is_one_image_named_for_its_disk() {
        let s = schema();
        let names = [
            "track00.1.raw",
            "track00.0.raw",
            "track81.1.raw",
            "dump.raw",
            "Disk2.scp",
        ];
        let folder = Path::new("/dumps/Lemmings");
        let files: Vec<PathBuf> = names.iter().map(|n| folder.join(n)).collect();
        let images = batch_images(&s, &files, "");
        let taken: Vec<String> = images.iter().map(|p| lossy(p.file_name())).collect();
        assert_eq!(taken, ["Disk2.scp", "track00.0.raw"]);
        let out = Output {
            folder: "/out".into(),
            ..output(".scp")
        };
        let made = |input: &Path| out.batch_path(input);
        assert_eq!(made(&images[1]), Path::new("/out").join("Lemmings.scp"));
        let named = Path::new("/d/Disk1_00.0.raw");
        assert_eq!(made(named), Path::new("/out").join("Disk1.scp"));

        let v = values(&[("in_file", &images[1].to_string_lossy())]);
        let beside = Output {
            beside_input: true,
            ..output(".scp")
        };
        let key = output_key("convert", "out_file");
        let h = page("convert", v, BTreeMap::from([(key.clone(), beside)]));
        assert_eq!(h.state().1[&key].name, "Lemmings");
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
    fn named_image_options_are_chosen_from_gws_names() {
        let v = values(&[("format", "ibm.1440")]);
        let key = output_key("read", "file");
        let pick = |ext: &str, shown: &str, name: &str| {
            let outputs = BTreeMap::from([(key.clone(), output(ext))]);
            let mut h = page("read", v.clone(), outputs);
            // The option's list, below Revolutions.
            h.get_all_by_role(Role::ComboBox)
                .rev()
                .find(|c| c.value().as_deref() == Some(shown))
                .expect("the option's list")
                .click();
            h.run();
            h.get_by_label(name).click();
            h.run();
            h.state().1[&key].value(1)
        };
        let scp = pick(".scp", "Default (other-320k)", "amiga");
        assert!(scp.ends_with(".scp::disktype=amiga"), "{scp}");
        let hfe = pick(".hfe", "Default", "250");
        assert!(hfe.ends_with(".hfe::bitrate=250"), "{hfe}");
    }

    #[test]
    fn every_image_option_has_a_short_tip_and_a_default_as_gw_names_it() {
        let s = schema();
        for (ext, image) in &s.images {
            for opt in image.read_opts.iter().chain(&image.write_opts) {
                let tip = OPTION_TIPS.iter().find(|(n, _)| *n == opt.name);
                let (_, tip) = tip.unwrap_or_else(|| panic!("{ext} {} has no tip", opt.name));
                assert!(tip.len() <= 70 && tip.ends_with('.'), "{tip}");
            }
        }
        let opt = |ext: &str, name: &str| {
            let image = &s.images[ext];
            let opts = image.read_opts.iter().chain(&image.write_opts);
            option_default(opts.clone().find(|o| o.name == name).unwrap())
        };
        assert_eq!(opt(".raw", "sck").as_deref(), Some("24027429"));
        assert_eq!(opt(".scp", "disktype").as_deref(), Some("other-320k"));
        assert_eq!(opt(".hfe", "bitrate"), None);
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
    fn every_image_type_gw_knows_has_a_plain_name() {
        let s = schema();
        for ext in s.images.keys() {
            assert!(KNOWN_IMAGES.iter().any(|(e, _)| e == ext), "{ext}");
        }
        assert_eq!(image_name(".dsk", "DSK"), "Sector image (.dsk)");
        assert_eq!(image_name(".2d", "TwoD"), "TwoD (.2d)", "a newer gw's");
    }

    #[test]
    fn a_file_dialog_opens_where_the_file_chosen_is() {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-dialog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.scp");
        std::fs::write(&file, b"").unwrap();
        let start = |p: &Path| format!("{:?}", file_dialog(p));
        let folder = format!("starting_directory: Some({dir:?})");
        assert!(start(&file).contains(&folder), "{}", start(&file));
        assert!(start(&dir).contains(&folder));
        assert!(start(Path::new("")).contains("starting_directory: None"));
        let gone = dir.join("gone/a.scp");
        assert!(start(&gone).contains("starting_directory: None"));

        let dialog = format!("{:?}", image_dialog(&schema(), &file.to_string_lossy()));
        assert!(dialog.contains(&folder));
        let all = dialog.find("\"Disk images\"").expect("every type first");
        let raw = r#""KryoFlux stream (.raw)", extensions: ["raw", "RAW"]"#;
        assert!(dialog.find(raw).is_some_and(|at| at > all), "{dialog}");
        assert!(dialog.contains(r#""adf", "ADF""#), "GTK matches by case");
        std::fs::remove_dir_all(&dir).ok();
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
        assert_eq!(t("apple2.nofs.140"), ".img", "not DOS order");
        assert_eq!(t("ibm.800"), ".img", "not SAM Coupé's .mgt");
        assert_eq!(t("ibm.scan"), ".edsk");
        assert_eq!(t("northstar.mfm.ds"), ".nsi");
        assert_eq!(t("raw.250"), ".hfe");
        assert_eq!(t("thomson.2s320"), ".fd");
    }

    #[test]
    fn a_format_with_no_fixed_sectors_shows_no_count_or_size() {
        let info = |sectors, bytes| FormatInfo {
            cyls: 80,
            heads: 2,
            encoding: Some("IBM MFM".into()),
            sectors: Some(sectors),
            bytes: Some(bytes),
            verifies: true,
            revs: None,
        };
        let shown = |i| describe(&i).replace('\u{a0}', " ");
        assert_eq!(
            shown(info((18, 18), 1_474_560)),
            "IBM MFM · 80 cylinders · 2 sides · 18 sectors per track · 1440 KB"
        );
        assert_eq!(shown(info((0, 0), 0)), "IBM MFM · 80 cylinders · 2 sides");
        let zoned = shown(info((17, 21), 196_608));
        assert!(zoned.contains("17 to 21 sectors per track"), "{zoned}");
    }

    #[test]
    fn families_have_names_even_new_ones() {
        assert_eq!(family_name("akai.800"), "Akai");
        assert_eq!(family_name("pc98.2hd"), "NEC PC-98");
        assert_eq!(family_name("newthing.1"), "Newthing");
    }

    #[test]
    fn every_field_has_a_short_tooltip_and_every_tooltip_a_field() {
        fn shown(cmd: &Command) -> Vec<&Arg> {
            let (first, rest) = sections(cmd);
            first.into_iter().chain(rest).collect()
        }
        let s = schema();
        for cmd in &s.commands {
            for a in shown(cmd) {
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
            let found = s.commands.iter().any(|cmd| {
                (c.is_empty() || cmd.name == *c) && shown(cmd).iter().any(|a| a.dest == *d)
            });
            assert!(found, "no gw {c} shows {d}");
        }
    }

    #[test]
    fn a_value_gw_has_a_grammar_for_shows_it_on_hover() {
        let typed = "c=0-7,9-12";
        let mut h = page("read", values(&[("tracks", typed)]), BTreeMap::new());
        h.get_by_label("Track settings").hover();
        h.run();
        h.get_by_label("Which tracks to read.");
        h.get_by_label_contains("h[01].off");
        h.event(egui::Event::PointerGone);
        h.run();
        h.get_all_by_role(Role::TextInput)
            .find(|n| n.value().as_deref() == Some(typed))
            .expect("the typed list")
            .hover();
        h.run();
        h.get_by_label_contains("h[01].off");

        h.get_by_label_contains("Advanced options").click();
        h.run();
        for (row, grammar) in [("PLL", "lowpass=USEC"), ("Fake index", "<N>scp")] {
            h.event(egui::Event::PointerGone);
            h.run();
            h.get_by_label(row).hover();
            h.run();
            h.get_by_label_contains(grammar);
        }
    }

    #[test]
    fn delay_tips_say_what_gws_help_defines() {
        let s = schema();
        let delays = s.command("delays").unwrap();
        let t = |dest: &str| tip("delays", delays.arg(dest).unwrap());
        assert!(t("watchdog").contains("motors stop"), "{}", t("watchdog"));
        assert!(t("pre_write").contains("track change"));
        assert!(t("post_write").contains("track change"));
        assert!(t("index_mask").contains("post-trigger"));
        for a in delays.args.iter().filter(|a| a.dest != "device") {
            assert!(
                !t(&a.dest).starts_with(&label(a)),
                "{} restates its label",
                a.dest
            );
        }
    }

    #[test]
    fn no_page_opens_with_all_its_fields_under_a_shut_header() {
        let s = schema();
        for cmd in &s.commands {
            let (first, rest) = sections(cmd);
            assert!(rest.is_empty() || !first.is_empty(), "gw {}", cmd.name);
        }
    }

    #[test]
    fn advanced_options_say_how_many_of_them_are_set() {
        let header = |v: Values| {
            let h = page("read", v, BTreeMap::new());
            let node = h.get_by_label_contains("Advanced options");
            node.accesskit_node()
                .label()
                .unwrap_or_default()
                .to_string()
        };
        let plain = header(Values::default());
        assert!(!plain.contains("set"), "{plain}");
        let set = header(values(&[("retries", "5"), ("raw", ON)]));
        assert!(set.ends_with(", 2 set)"), "{set}");
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

    #[test]
    fn disk_numbers_have_the_digits_the_first_was_typed_with() {
        assert_eq!(typed_number("08"), Some((8, 2)));
        assert_eq!(typed_number("1"), Some((1, 1)));
        assert_eq!(typed_number(" 001 "), Some((1, 3)));
        assert_eq!(typed_number("1a"), None);
        let out = Output {
            name: "Game".into(),
            label: "Disk".into(),
            disks: 100,
            first_digits: 2,
            ..output(".adf")
        };
        let names = [1, 10, 100].map(|d| out.file_name(d));
        let typed = ["Game_Disk01.adf", "Game_Disk10.adf", "Game_Disk100.adf"];
        assert_eq!(names, typed, "not padded to the 3 digits of 100");

        let out = Output { disks: 12, ..out };
        let outputs = BTreeMap::from([(output_key("read", "file"), out)]);
        let mut h = page("read", Values::default(), outputs);
        h.get_by_label("Multiple disks (12)").click();
        h.run();
        for (first, names) in [
            ("1", "Game_Disk1.adf, Game_Disk2.adf … Game_Disk12.adf"),
            (
                "001",
                "Game_Disk001.adf, Game_Disk002.adf … Game_Disk012.adf",
            ),
        ] {
            h.get_all_by_role(Role::SpinButton).last().unwrap().click();
            h.run();
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.event(egui::Event::Text(first.into()));
            h.key_press(egui::Key::Enter);
            h.run();
            h.get_by_label(&format!("Names each disk sequentially: {names}"));
        }
    }

    #[test]
    fn a_path_in_the_home_folder_shows_from_the_systems_home_and_takes_it_typed() {
        let home = crate::home().unwrap();
        let disks = home.join("Disks").to_string_lossy().into_owned();
        let shown = match cfg!(windows) {
            true => r"%USERPROFILE%\Disks",
            false => "~/Disks",
        };
        assert_eq!(short_path(&disks), shown);
        assert_eq!(full_path(shown), disks);
        if cfg!(windows) {
            assert_eq!(full_path(r"%userprofile%\Disks"), disks, "any case");
        }
        assert_eq!(short_path(&home.to_string_lossy()), HOME);
        assert_eq!(short_path("/elsewhere/Disks"), "/elsewhere/Disks");
        assert_eq!(full_path("~/Disks"), disks);
        assert_eq!(full_path("/elsewhere/Disks"), "/elsewhere/Disks");
        assert_eq!(full_path("~user/Disks"), "~user/Disks", "another account's");
    }

    #[test]
    fn a_path_box_keeps_what_is_typed_and_its_value_whole() {
        let home = crate::home().unwrap();
        let out = Output {
            folder: home.join("Images").to_string_lossy().into_owned(),
            ext: ".adf".into(),
            ..Output::default()
        };
        let outputs = BTreeMap::from([(output_key("read", "file"), out)]);
        let mut h = page("read", Values::default(), outputs);
        let shown = format!("{HOME}{}Images", std::path::MAIN_SEPARATOR);
        let folder = |h: &Harness<'_, Page>, text: &str| {
            let boxes = h.get_all_by_role(Role::TextInput);
            boxes.filter(|n| n.value().as_deref() == Some(text)).count()
        };
        assert_eq!(folder(&h, &shown), 1, "from home");
        h.get_all_by_role(Role::TextInput)
            .find(|n| n.value().as_deref() == Some(shown.as_str()))
            .unwrap()
            .click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("~/Disks".into()));
        h.run();
        let kept = &h.state().1[&output_key("read", "file")].folder;
        assert_eq!(*kept, home.join("Disks").to_string_lossy());
        assert_eq!(folder(&h, "~/Disks"), 1, "as typed");
        // Changed elsewhere, as by a dropped file, it shows the new value.
        let key = output_key("read", "file");
        h.state_mut().1.get_mut(&key).unwrap().folder = "/elsewhere".into();
        h.run();
        assert_eq!(folder(&h, "/elsewhere"), 1);
    }

    #[test]
    fn a_numbered_set_can_add_its_total_to_each_name() {
        let mut out = Output {
            name: "Game".into(),
            label: "Disk".into(),
            disks: 12,
            first_digits: 2,
            total: true,
            ..output(".adf")
        };
        assert_eq!(out.file_name(1), "Game_Disk01_of_12.adf");
        (out.first_digits, out.number_first) = (1, true);
        assert_eq!(out.file_name(3), "Disk3_of_12_Game.adf");
        out.ask_names = true;
        assert_eq!(
            out.file_name(3),
            "Game.adf",
            "a name asked for has no number"
        );

        let outputs = BTreeMap::from([(output_key("read", "file"), out)]);
        let mut h = page("read", Values::default(), outputs);
        h.get_by_label("Multiple disks (12)").click();
        h.run();
        h.get_by_label("Add the total").hover();
        h.run();
        h.get_by_label("Needs numbered names.");
    }

    #[test]
    fn a_set_reads_up_to_256_disks() {
        let out = Output {
            name: "Game".into(),
            disks: 300,
            ..output(".adf")
        };
        let outputs = BTreeMap::from([(output_key("read", "file"), out)]);
        let mut h = page("read", Values::default(), outputs);
        h.get_by_label_contains("Multiple disks").click();
        h.run();
        let out = &h.state().1[&output_key("read", "file")];
        assert_eq!(out.disks, 256);
        let names = "Game_1.adf, Game_2.adf … Game_256.adf";
        assert_eq!(out.preview_names(), names);
    }

    #[test]
    fn a_set_numbers_its_files_unless_it_asks_each_disks_name() {
        let mut out = Output {
            folder: "/f".into(),
            name: "Game".into(),
            ext: ".adf".into(),
            disks: 3,
            ..Output::default()
        };
        assert_eq!(out.preview_names(), "Game_1.adf, Game_2.adf, Game_3.adf");
        out.first = 8;
        out.ask_names = true;
        assert_eq!(out.first_disk(), 1, "First disk is for numbered sets");
        assert_eq!(out.path(2), PathBuf::from("/f/Game.adf"));
        assert_eq!(
            out.preview(),
            Path::new("/f").join("Game.adf").to_string_lossy()
        );
        let named = out.named("Lemmings 2").path(1);
        assert_eq!(named, PathBuf::from("/f/Lemmings 2.adf"));

        let s = schema();
        let service = Service::offline(Ok(s.clone()));
        let why = |out: &Output| {
            let outputs = BTreeMap::from([(output_key("read", "file"), out.clone())]);
            let v = values(&[("format", "amiga.amigados")]);
            blocked(&s, s.command("read").unwrap(), &v, &outputs, &service)
        };
        out.name.clear();
        assert_eq!(why(&out), None, "each disk's name is asked for");
        out.ask_names = false;
        assert_eq!(why(&out), Some("Name the image first."));

        out.ask_names = true;
        let outputs = BTreeMap::from([(output_key("read", "file"), out)]);
        let mut h = page("read", values(&[("format", "amiga.amigados")]), outputs);
        h.get_by_label("Multiple disks (3)").click();
        h.run();
        h.get_by_label("Asks for each disk's name before reading it.");
        h.get_all_by_role(Role::TextInput).last().unwrap().hover();
        h.run();
        h.get_by_label("Needs numbered names.");
    }

    #[test]
    fn a_set_can_start_at_any_disk_and_keeps_the_sets_numbering() {
        let mut out = Output {
            folder: "/f".into(),
            name: "Game".into(),
            label: "Disk".into(),
            ext: ".adf".into(),
            disks: 12,
            first: 4,
            ..Output::default()
        };
        let paths: Vec<_> = out.paths().collect();
        let names = (4..=12).map(|d| PathBuf::from(format!("/f/Game_Disk{d}.adf")));
        assert_eq!(paths, names.collect::<Vec<_>>());
        assert_eq!(
            out.preview_names(),
            "Game_Disk4.adf, Game_Disk5.adf … Game_Disk12.adf"
        );
        out.first = 20;
        assert_eq!(out.first_disk(), 12, "no further than the set's last");
        let saved: Output = serde_json::from_str(r#"{"disks": 3}"#).unwrap();
        assert_eq!(saved.first, 1, "a preset saved without it");
    }
}

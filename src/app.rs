//! The window: a sidebar, a page per gw command, and the disk's status beside it.

use crate::command::{self, Values};
use crate::device::{self, DeviceInfo};
use crate::diskmap;
use crate::engine::{Engine, Origin};
use crate::form::{self, Form, Output};
use crate::job::{DETECT, Job, Outcome};
use crate::presets::{self, Preset};
use crate::progress::Progress;
use crate::schema::{Command, Port, Schema};
use crate::service::{Load, Repaint, Service};
use crate::theme::{self, Palette};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Layout, Margin, RichText, Sense,
    Stroke, TextEdit, TextStyle, ThemePreference, Ui, UserAttentionType, Vec2, ViewportCommand,
    pos2, vec2,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// How long the device card waits for `gw info`.
const INFO_TIMEOUT: Duration = Duration::from_secs(12);

/// Commands in the sidebar, by section. Commands a newer gw adds go under Other.
const SECTIONS: &[(&str, &[&str])] = &[
    ("Disk", &["read", "write", "convert", "erase"]),
    ("Drive", &["clean", "seek", "rpm", "align"]),
    (
        "Device",
        &[
            "info",
            "update",
            "delays",
            "pin get",
            "pin set",
            "reset",
            "bandwidth",
        ],
    ),
];

/// Page titles and run buttons.
const NAMES: &[(&str, &str, &str)] = &[
    ("read", "Read disk", "Read disk"),
    ("write", "Write disk", "Write disk"),
    ("convert", "Convert image", "Convert"),
    ("erase", "Erase disk", "Erase disk"),
    ("clean", "Clean heads", "Clean"),
    ("seek", "Seek", "Seek"),
    ("rpm", "Drive speed", "Measure"),
    ("align", "Align heads", "Start"),
    ("info", "Device info", "Get info"),
    ("update", "Update firmware", "Update"),
    ("delays", "Delays", "Run"),
    ("pin get", "Read pin", "Read pin"),
    ("pin set", "Set pin", "Set pin"),
    ("reset", "Reset", "Reset"),
    ("bandwidth", "USB bandwidth", "Measure"),
    (DETECT, "Detect disk format", "Detect"),
];

/// Commands that ask first, and what they do to the disk.
const DESTRUCTIVE: &[(&str, &str)] = &[
    ("write", "Everything on the disk will be replaced."),
    ("erase", "Everything on the disk will be lost."),
];

/// Commands the status pane shows. Others show their results under their page.
const DISK_COMMANDS: &[&str] = &["read", "write", "convert", "erase", "align", DETECT];

const REPO: &str = "https://github.com/hobbo91/ferriteweazle";

/// The page's minimum width: room for a label beside its field.
const PAGE_MIN: f32 = 420.0;
const STATUS_MIN: f32 = 320.0;
/// The sidebar logo's side.
const LOGO_SIZE: f32 = 40.0;
/// How far the logo reaches past the sidebar's margin, up and to the left.
const LOGO_TUCK: f32 = 6.0;
/// Space above the device card, so its top lines up with the page's description.
const CARD_DROP: f32 = 2.0;
/// A sidebar entry's height.
const NAV_ROW: f32 = 26.0;
/// The share of the status pane's height the map and its heading fill.
const MAP_SHARE: f32 = 0.8;
/// The log's height apart from its lines: its heading row and frame.
const LOG_HEADING: f32 = 58.0;
/// One line of the log.
const LOG_LINE: f32 = 18.0;
/// The drawer's height, margins included: the command line's, and the log's at first.
const DRAWER: f32 = 124.0;
/// Height the log leaves the page above it, however far it is dragged.
const LOG_ROOM: f32 = 260.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Page {
    Command(String),
    Settings,
}

impl Default for Page {
    fn default() -> Self {
        Page::Command("read".into())
    }
}

/// Everything remembered between runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub page: Page,
    pub theme: ThemePreference,
    /// A Python or `gw` to use instead of the one found automatically.
    pub engine: Option<PathBuf>,
    /// Empty for gw's own choice.
    pub device: String,
    pub drive: String,
    /// Passes gw's `--bt` for Python tracebacks on errors.
    pub backtrace: bool,
    /// Saves gw's output beside each image a job makes, as `name.ext.log`.
    pub save_logs: bool,
    /// Plays a sound when a job ends.
    pub sound: bool,
    pub values: BTreeMap<String, Values>,
    pub outputs: BTreeMap<String, Output>,
    /// Where presets are saved and listed from; unset, Documents/Ferriteweazle.
    pub presets_folder: Option<PathBuf>,
    /// The drawer open under the page and the status pane, if one is.
    pub drawer: Option<Drawer>,
}

/// What the drawer under the page and the status pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Drawer {
    /// The page's gw command line.
    Cli,
    /// gw's output from the latest job.
    Log,
}

enum Dialog {
    Confirm {
        command: String,
        args: Vec<String>,
    },
    /// Files a job would replace, and the job's runs, one per disk.
    Overwrite {
        files: Vec<PathBuf>,
        command: String,
        runs: Vec<Vec<String>>,
    },
    /// Between the disks of a session.
    NextDisk {
        disk: usize,
        total: usize,
        /// The disk before failed.
        failed: bool,
    },
    SavePreset {
        command: String,
        name: String,
    },
    Quit,
}

/// What a dialog's button does, once the dialog has let go of the app.
type Action = Box<dyn FnOnce(&mut App)>;

/// Disks read one after another: gw's arguments for each, and the next.
struct Session {
    runs: Vec<Vec<String>>,
    next: usize,
}

/// The command line drawer's text, and why it does not parse.
#[derive(Default)]
struct Cli {
    text: String,
    error: Option<String>,
}

pub struct App {
    pub settings: Settings,
    engine: Option<Engine>,
    service: Service,
    schema: Option<Arc<Schema>>,
    /// The last job about a disk, which the status pane shows.
    pub disk: Option<Job>,
    /// The last job of any other command, shown under its page.
    pub tool: Option<Job>,
    /// The page a running detect job chooses the format on.
    detect_for: Option<String>,
    session: Option<Session>,
    cli: Cli,
    dialog: Option<Dialog>,
    /// A note above the page, such as the formats detection found.
    pub notice: Option<String>,
    /// Close the window once the stopped job has ended.
    quitting: bool,
    /// What the last `gw info` said about the device.
    device: Option<DeviceInfo>,
    /// `gw info` run for the device card, apart from the jobs.
    probe: Option<Job>,
    /// Why the last probe learned nothing.
    probe_failed: Option<String>,
    /// The port the card last asked about.
    probed: Option<String>,
    /// Ask each Greaseweazle that appears what it is.
    auto_info: bool,
    logo: Option<egui::TextureHandle>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        let mut app = App::with_settings(&cc.egui_ctx, settings);
        app.auto_info = true;
        app
    }

    pub fn with_settings(ctx: &egui::Context, settings: Settings) -> App {
        let mut app = App::offline(ctx, settings, Err(String::new()));
        app.connect(ctx);
        app
    }

    /// A window over a known schema, with no engine to run anything.
    pub fn offline(ctx: &egui::Context, settings: Settings, schema: Result<Schema, String>) -> App {
        theme::install(ctx);
        ctx.set_theme(settings.theme);
        App {
            settings,
            engine: None,
            schema: schema.as_ref().ok().cloned().map(Arc::new),
            service: Service::offline(schema),
            disk: None,
            tool: None,
            detect_for: None,
            session: None,
            cli: Cli::default(),
            dialog: None,
            notice: None,
            quitting: false,
            device: None,
            probe: None,
            probe_failed: None,
            probed: None,
            auto_info: false,
            logo: None,
        }
    }

    fn connect(&mut self, ctx: &egui::Context) {
        self.schema = None;
        self.engine = Engine::find(self.settings.engine.as_deref());
        self.service = match (&self.engine, &self.settings.engine) {
            (Some(engine), _) => Service::start(engine, repaint(ctx)),
            (None, Some(path)) => Service::offline(Err(format!(
                "{} is neither a Python nor a gw launcher.",
                path.display()
            ))),
            (None, None) => Service::offline(Err(
                "Ferriteweazle could not find Greaseweazle. Choose your gw in Settings.".into(),
            )),
        };
    }

    /// gw's command line, once the engine has described it.
    pub fn schema(&self) -> Option<&Schema> {
        self.schema.as_deref()
    }

    /// The job that is running, if one is.
    pub fn running(&self) -> Option<&Job> {
        [&self.disk, &self.tool]
            .into_iter()
            .flatten()
            .find(|j| j.running())
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.guard_close(&ctx);
        self.take_dropped_files(&ctx);
        let p = theme::palette(ui);
        egui::Panel::left("nav")
            .exact_size(240.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.sidebar)
                    .inner_margin(Margin::symmetric(14, 16)),
            )
            .show(ui, |ui| self.nav(ui));
        let page = Frame::new().fill(p.bg).inner_margin(Margin {
            left: 28,
            right: 28,
            top: 22,
            bottom: 14,
        });
        match self.settings.page.clone() {
            Page::Settings => {
                egui::CentralPanel::default()
                    .frame(page)
                    .show(ui, |ui| self.settings_page(ui));
            }
            Page::Command(name) => {
                // Added first, so it spans the page and the status pane.
                if let Some(drawer) = self.settings.drawer {
                    self.drawer(ui, &name, drawer);
                }
                // The page takes up to its form's full width and the status
                // pane the rest, down to STATUS_MIN.
                let widest = (ui.available_width() - PAGE_MIN).max(STATUS_MIN);
                let page_wants = form::full_width(ui) + page.inner_margin.sum().x;
                let status = (ui.available_width() - page_wants).clamp(STATUS_MIN, widest);
                egui::Panel::right("status")
                    .resizable(false)
                    .exact_size(status.min(1100.0))
                    .frame(Frame::new().fill(p.bg).inner_margin(Margin::same(18)))
                    .show(ui, |ui| self.status(ui, &name));
                egui::CentralPanel::default()
                    .frame(page)
                    .show(ui, |ui| self.page(ui, &name));
            }
        }
        self.dialogs(&ctx);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.service.poll();
        if self.schema.is_none() {
            self.schema = self.service.schema.ready().cloned().map(Arc::new);
        }
        let mut ended = Vec::new();
        for (disk, job) in [(true, &mut self.disk), (false, &mut self.tool)] {
            let Some(job) = job else { continue };
            let was_running = job.running();
            job.poll();
            if let Some(wait) = job.wake_in() {
                ctx.request_repaint_after(wait);
            }
            if was_running && !job.running() {
                ended.push(disk);
            }
        }
        for disk in ended {
            self.ended(ctx, disk);
        }
        self.poll_probe(ctx);
        if self.quitting && self.running().is_none() && self.probe.is_none() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// Keeps the device card current, and gives up on a device that does not answer.
    fn poll_probe(&mut self, ctx: &egui::Context) {
        if let Some(probe) = &mut self.probe {
            probe.poll();
            if probe.running() && probe.elapsed() > INFO_TIMEOUT {
                probe.stop();
            }
            if let Some(wait) = probe.wake_in() {
                ctx.request_repaint_after(wait);
            }
            // The device's fields come before gw looks online for firmware.
            if let Some(info) = device::parse(&probe.log) {
                self.device = Some(info);
            }
            if !probe.running() {
                self.probe_failed = match (&self.device, probe.outcome()) {
                    (Some(_), _) => None,
                    (None, Some(Outcome::Stopped)) => Some("No answer.".into()),
                    (None, _) => Some(
                        probe
                            .progress
                            .error
                            .clone()
                            .unwrap_or_else(|| "It did not say what it is.".into()),
                    ),
                };
                self.probe = None;
            }
        }
        let port = self.found_port().map(|p| p.device.clone());
        if port.is_none() {
            self.probed = None;
            self.device = None;
            self.probe_failed = None;
        } else if self.auto_info && port != self.probed {
            self.ask_device(ctx);
        }
    }

    /// The Greaseweazle the sidebar shows.
    fn found_port(&mut self) -> Option<&Port> {
        self.service.ports();
        chosen_port(self.service.known_ports(), &self.settings.device)
    }

    /// Runs `gw info` for the device card, when nothing else is using the device.
    fn ask_device(&mut self, ctx: &egui::Context) {
        if self.running().is_some() || self.probe.is_some() {
            return;
        }
        let Some(schema) = self.schema.clone() else {
            return;
        };
        let Some(cmd) = schema.command("info") else {
            return;
        };
        self.probed = self.found_port().map(|p| p.device.clone());
        // None of the Device info page's options: --bootloader would switch
        // the device's mode every time.
        let args = self.argv(cmd, &self.device_only(cmd));
        let Some(engine) = &self.engine else { return };
        match Job::start(engine, "info", args, repaint(ctx)) {
            Ok(job) => {
                self.probe = Some(job);
                self.probe_failed = None;
            }
            Err(e) => self.probe_failed = Some(format!("Could not start gw: {e}")),
        }
    }

    /// A job has just ended: save the log, and do whatever was waiting on it.
    fn ended(&mut self, ctx: &egui::Context, disk: bool) {
        let slot = if disk { &mut self.disk } else { &mut self.tool };
        let Some(job) = slot.as_mut() else { return };
        let outcome = job.outcome();
        if self.settings.save_logs
            && let Some(image) = job.output.as_ref().filter(|p| p.exists())
        {
            let mut log = image.clone().into_os_string();
            log.push(".log");
            if let Err(e) = std::fs::write(&log, job.log.join("\n") + "\n") {
                job.log
                    .push(format!("Could not save this output beside the image: {e}"));
            }
        }
        let command = job.command.clone();
        let detected = std::mem::take(&mut job.detected);
        let step = job.step;
        match command.as_str() {
            "info" => self.device = device::parse(&job.log),
            // New firmware changes what the device says about itself.
            "update" => self.probed = None,
            _ => {}
        }
        if command == "read"
            && let Some(session) = &self.session
        {
            let total = session.runs.len();
            match outcome {
                Some(Outcome::Stopped) => self.session = None,
                _ if session.next < total => {
                    self.dialog = Some(Dialog::NextDisk {
                        disk: session.next + 1,
                        total,
                        failed: outcome == Some(Outcome::Failed),
                    });
                }
                _ => self.session = None,
            }
        }
        if self.settings.sound {
            chime(outcome);
        }
        if !ctx.input(|i| i.viewport().focused.unwrap_or(true)) {
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                UserAttentionType::Informational,
            ));
        }
        if command == DETECT {
            self.found(detected, step);
        }
    }

    /// A detect job found these formats, best first, and the disk's head
    /// step: its page takes the best.
    fn found(&mut self, formats: Vec<String>, step: u32) {
        let (Some(page), Some(best)) = (self.detect_for.take(), formats.first()) else {
            return; // the status pane says why
        };
        let Some(schema) = self.schema.clone() else {
            return;
        };
        let Some(cmd) = schema.command(&page) else {
            return;
        };
        let values = self.settings.values.entry(page).or_default();
        form::choose_format(&schema, cmd, values, &mut self.settings.outputs, best);
        if step > 1 {
            values.set("tracks", form::double_step(values.get("tracks")));
        }
        self.notice = Some(found_note(&formats, step));
    }

    /// Closing the window while gw works asks first: gw stops the drive before
    /// the window closes.
    fn guard_close(&mut self, ctx: &egui::Context) {
        if self.running().is_some() && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if !self.quitting {
                self.dialog = Some(Dialog::Quit);
            }
        }
    }

    /// A dropped file becomes the input of the Write page if it is open, else of Convert.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        let Some(path) = dropped.filter(|p| !p.as_os_str().is_empty()) else {
            return;
        };
        let (page, dest) = match &self.settings.page {
            Page::Command(name) if name == "write" => ("write", "file"),
            _ => ("convert", "in_file"),
        };
        self.settings
            .values
            .entry(page.to_owned())
            .or_default()
            .set(dest, path.to_string_lossy());
        self.settings.page = Page::Command(page.to_owned());
    }

    fn nav(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("nav-foot")
            .frame(Frame::NONE)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.add_space(6.0);
                let version = self.schema.as_ref().map(|s| format!("gw {}", s.version));
                let settings = self.settings.page == Page::Settings;
                if nav_item(ui, "Settings", version.as_deref(), settings).clicked() {
                    self.settings.page = Page::Settings;
                }
            });
        ui.horizontal(|ui| {
            logo(ui, &mut self.logo);
            ui.add_space(6.0);
            ui.label(RichText::new("Ferriteweazle").size(17.0).strong());
        });
        ui.add_space(CARD_DROP);
        self.device_card(ui);
        ui.add_space(12.0);
        let list = egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                // Rows touch, as in a source list.
                ui.spacing_mut().item_spacing.y = 0.0;
                for (section, names) in sections(self.schema.as_deref()) {
                    ui.add_space(4.0);
                    ui.label(RichText::new(section).small().weak());
                    ui.add_space(1.0);
                    for name in names {
                        let here = matches!(&self.settings.page, Page::Command(n) if n == name);
                        if nav_item(ui, &title(name), None, here).clicked() {
                            self.settings.page = Page::Command(name.to_owned());
                        }
                    }
                    ui.add_space(4.0);
                }
            });
        // More below: fade out, so no row shows cut in half.
        if list.content_size.y - list.state.offset.y - list.inner_rect.height() > 1.0 {
            fade_out(ui.painter(), list.inner_rect, theme::palette(ui).sidebar);
        }
    }

    fn device_card(&mut self, ui: &mut Ui) {
        let p = theme::palette(ui);
        let drives = self.drives();
        let default_drive = self.default_drive();
        let found = self.found_port().cloned();
        let ports = self.service.known_ports();
        let asking = self.probe.is_some();
        let idle = self.running().is_none() && !asking;
        // gw info's report, while it is about the device still connected.
        let info = self.device.as_ref().filter(|i| {
            found
                .as_ref()
                .is_some_and(|port| i.get("Port").is_none_or(|p| p == port.device))
        });
        let mut ask = false;
        Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.line))
            .corner_radius(10)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 5.0;
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    let colour = if found.is_some() { p.good } else { p.bad };
                    ui.painter().circle_filled(r.center(), 4.5, colour);
                    let name = match (&found, info.and_then(|i| i.get("Model"))) {
                        (None, _) => "Disconnected",
                        (Some(_), Some(model)) => model,
                        (Some(port), None) => port.name.as_deref().unwrap_or("Greaseweazle"),
                    };
                    ui.add(egui::Label::new(RichText::new(name).strong()).truncate());
                    right(ui, |ui| {
                        ask |= ui
                            .add_enabled(idle, refresh_button(p))
                            .on_hover_text("Look for the Greaseweazle again.")
                            .on_disabled_hover_text(match asking {
                                true => "Asking the device…",
                                false => "Wait for the job that is running.",
                            })
                            .clicked();
                    });
                });
                if found.is_some() {
                    // The firmware and any newer one share a line, so the card keeps its height.
                    if let Some(firmware) = info.and_then(|i| i.get("Firmware")) {
                        let update = info.and_then(|i| i.update.as_deref());
                        text_row(ui, |ui| {
                            ui.label(RichText::new(format!("Firmware {firmware}")).small().weak());
                            if let Some(update) = update {
                                let text = RichText::new(format!("· {update} available")).small();
                                if ui
                                    .link(text)
                                    .on_hover_text("Open Update firmware.")
                                    .clicked()
                                {
                                    self.settings.page = Page::Command("update".into());
                                }
                            }
                        });
                    }
                    if info.is_none() && asking {
                        text_row(ui, |ui| {
                            ui.add(egui::Spinner::new().size(10.0));
                            ui.label(RichText::new("Asking the device…").small().weak());
                        });
                    } else if info.is_none() {
                        // On the line the firmware takes once the device answers.
                        if let Some(why) = &self.probe_failed {
                            ui.label(RichText::new(why).small().color(p.partial));
                        }
                        let link = egui::Link::new(RichText::new("Get info").small());
                        ask |= ui
                            .add_enabled(idle, link)
                            .on_hover_text("Ask the Greaseweazle what it is.")
                            .on_disabled_hover_text("Wait for the job that is running.")
                            .clicked();
                    }
                }
                ui.add_space(4.0);
                let shown = match &found {
                    Some(port) => RichText::new(short_port(&port.device)),
                    None => RichText::new("Select device").color(p.dim),
                };
                egui::ComboBox::from_id_salt("device")
                    .selected_text(shown)
                    .truncate()
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        if ports.is_empty() {
                            ui.label(RichText::new("No ports found.").weak());
                        }
                        for port in ports {
                            let text = match (&port.name, port.score > 0) {
                                (Some(name), true) => {
                                    format!("{} · {name}", short_port(&port.device))
                                }
                                _ => short_port(&port.device).to_owned(),
                            };
                            let here = found.as_ref().is_some_and(|f| f.device == port.device);
                            if ui
                                .selectable_label(here, text)
                                .on_hover_text(port.device.as_str())
                                .clicked()
                            {
                                self.settings.device = port.device.clone();
                            }
                        }
                    })
                    .response
                    .on_hover_text("Which port the Greaseweazle is on.");
                ui.add_space(2.0);
                ui.label(RichText::new("Identifier").small().weak())
                    .on_hover_text("The drive, by bus unit.");
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let current = if self.settings.drive.is_empty() {
                        default_drive.clone()
                    } else {
                        self.settings.drive.clone()
                    };
                    for (id, about) in &drives {
                        let button = egui::Button::selectable(current == *id, id.as_str())
                            .min_size(vec2(26.0, 24.0));
                        if ui.add(button).on_hover_text(about.as_str()).clicked() {
                            self.settings.drive = if *id == default_drive {
                                String::new()
                            } else {
                                id.clone()
                            };
                        }
                    }
                });
            });
        if ask {
            self.service.refresh_ports();
            if found.is_some() {
                self.ask_device(ui.ctx());
            }
        }
    }

    /// Drive letters and bus units, read from gw's own help for DRIVE.
    fn drives(&self) -> Vec<(String, String)> {
        let note = self
            .schema
            .as_ref()
            .and_then(|s| s.note("DRIVE"))
            .unwrap_or("0 | 1 | 2 :: Shugart bus unit\nA | B :: IBM/PC bus unit");
        let mut drives: Vec<(String, String)> = note
            .lines()
            .filter_map(|l| l.split_once("::"))
            .flat_map(|(ids, about)| {
                let about = form::sentence(about);
                ids.split('|')
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(move |id| (id.to_owned(), about.clone()))
            })
            .collect();
        drives.sort_by_key(|(id, _)| id.starts_with(|c: char| c.is_ascii_digit()));
        drives
    }

    fn default_drive(&self) -> String {
        self.schema
            .as_ref()
            .and_then(|s| {
                s.commands
                    .iter()
                    .flat_map(|c| &c.args)
                    .find(|a| a.dest == "drive")?
                    .default
                    .clone()
            })
            .unwrap_or_else(|| "A".into())
    }

    /// A command's settings from its page, with the sidebar's device and drive.
    fn values_for(&self, cmd: &Command) -> Values {
        let mut values = self
            .settings
            .values
            .get(&cmd.name)
            .cloned()
            .unwrap_or_default();
        // A chosen port that has gone is left to gw, which finds the Greaseweazle.
        let ports = self.service.known_ports();
        let device = match ports.iter().any(|p| p.device == self.settings.device) {
            true => self.settings.device.as_str(),
            false => "",
        };
        for (dest, value) in [("device", device), ("drive", self.settings.drive.as_str())] {
            if cmd.arg(dest).is_some() {
                values.set(dest, value);
            }
        }
        // A definitions file goes to gw only with one of its own formats.
        let custom = self.service.known_custom_formats(values.get("diskdefs"));
        if !custom.iter().any(|f| f == values.get("format")) {
            values.set("diskdefs", "");
        }
        values
    }

    /// A command's settings with nothing set but the sidebar's port.
    fn device_only(&self, cmd: &Command) -> Values {
        let mut values = Values::default();
        values.set("device", self.values_for(cmd).get("device"));
        values
    }

    /// gw's arguments for settings, with gw's own options from Settings.
    fn argv(&self, cmd: &Command, values: &Values) -> Vec<String> {
        let mut args = command::argv(cmd, values);
        if self.settings.backtrace {
            args.insert(0, "--bt".into());
        }
        args
    }

    fn args(&self, cmd: &Command) -> Vec<String> {
        self.argv(cmd, &self.values_for(cmd))
    }

    /// What a detect job needs: the drive when reading, else the input file.
    fn detect_args(&self, cmd: &Command) -> Vec<String> {
        let mut values = self.values_for(cmd);
        // Detection tries a definitions file's formats beside gw's own.
        if let Some(page) = self.settings.values.get(&cmd.name) {
            values.set("diskdefs", page.get("diskdefs"));
        }
        let mut args: Vec<String> = ["device", "drive", "diskdefs"]
            .into_iter()
            .filter(|d| cmd.name == "read" || *d == "diskdefs")
            .filter(|d| !values.get(d).is_empty())
            .map(|d| format!("--{d}={}", values.get(d)))
            .collect();
        if cmd.name != "read" {
            let dest = if cmd.arg("in_file").is_some() {
                "in_file"
            } else {
                "file"
            };
            args.push(
                values
                    .get(dest)
                    .split("::")
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
        args
    }

    fn page(&mut self, ui: &mut Ui, name: &str) {
        let Some(schema) = self.schema.clone() else {
            return self.not_ready(ui);
        };
        let Some(cmd) = schema.command(name) else {
            ui.heading(title(name));
            ui.label(format!("gw {} has no {name} command.", schema.version));
            return;
        };
        // Everything above and in the form ends where its fields do.
        let width = form::form_width(ui);
        ui.scope(|ui| {
            ui.set_max_width(width);
            ui.horizontal(|ui| {
                ui.heading(title(name));
                right(ui, |ui| self.presets_menu(ui, name));
            });
            ui.label(RichText::new(form::sentence(&cmd.about)).weak());
            self.notice_bar(ui);
        });
        ui.add_space(14.0);
        egui::Panel::bottom("run-bar")
            .frame(Frame::new().inner_margin(Margin {
                left: 0,
                right: 0,
                top: 12,
                bottom: 4,
            }))
            .show_separator_line(false)
            .show(ui, |ui| self.run_bar(ui, cmd));
        let busy = self.running().is_some();
        let action = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // A column of its own: the scroll area fills the page, the form does not.
                let column = Layout::top_down(Align::Min);
                ui.allocate_ui_with_layout(vec2(width, 0.0), column, |ui| {
                    ui.set_max_width(width);
                    let values = self.settings.values.entry(name.to_owned()).or_default();
                    let action = Form {
                        schema: &schema,
                        cmd,
                        values,
                        outputs: &mut self.settings.outputs,
                        service: &mut self.service,
                        busy,
                    }
                    .show(ui);
                    if let Some(job) = self.tool.as_ref().filter(|j| j.command == name) {
                        ui.add_space(18.0);
                        result(ui, job);
                    }
                    ui.add_space(12.0);
                    action
                })
                .inner
            })
            .inner;
        if action == Some(form::Action::Detect) {
            self.detect_for = Some(name.to_owned());
            let args = self.detect_args(cmd);
            self.run(ui.ctx(), DETECT, args);
        }
    }

    fn notice_bar(&mut self, ui: &mut Ui) {
        let Some(notice) = self.notice.clone() else {
            return;
        };
        ui.add_space(8.0);
        let p = theme::palette(ui);
        Frame::new()
            .fill(p.accent.gamma_multiply(0.12))
            .corner_radius(8)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.allocate_ui(vec2(ui.available_width() - 70.0, 0.0), |ui| {
                        ui.add(egui::Label::new(notice).wrap());
                    });
                    right(ui, |ui| {
                        if ui
                            .small_button("Dismiss")
                            .on_hover_text("Hide this.")
                            .clicked()
                        {
                            self.notice = None;
                        }
                    });
                });
            });
    }

    fn not_ready(&mut self, ui: &mut Ui) {
        ui.add_space(60.0);
        ui.vertical_centered(|ui| match self.service.schema.error() {
            None => {
                ui.spinner();
                ui.label(RichText::new("Starting Greaseweazle…").weak());
            }
            Some(e) => {
                ui.label(
                    RichText::new("Greaseweazle is not ready")
                        .size(18.0)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(RichText::new(e).weak());
                ui.add_space(10.0);
                if ui.button("Open Settings").clicked() {
                    self.settings.page = Page::Settings;
                }
            }
        });
    }

    fn run_bar(&mut self, ui: &mut Ui, cmd: &Command) {
        let p = theme::palette(ui);
        let why = self.why_not(cmd);
        ui.horizontal(|ui| {
            // The job this page started, or its format being found.
            let here = self.running().filter(|j| {
                j.command == cmd.name
                    || (j.command == DETECT && self.detect_for.as_deref() == Some(&cmd.name))
            });
            match here {
                Some(job) => {
                    let label = if job.stopping() {
                        "Stopping…"
                    } else {
                        "Stop"
                    };
                    let stop = ui.add_enabled(!job.stopping(), big_button(label, p.bad, p));
                    if stop
                        .on_hover_text("Stop gw. The drive motor turns off.")
                        .clicked()
                    {
                        self.stop();
                    }
                }
                None => {
                    let several = cmd.name == "read"
                        && self
                            .settings
                            .outputs
                            .get(&form::output_key("read", "file"))
                            .is_some_and(|o| o.disks > 1);
                    let label = if several {
                        "Read disks"
                    } else {
                        run_label(&cmd.name)
                    };
                    let run = ui.add_enabled(why.is_none(), big_button(label, p.accent, p));
                    match &why {
                        Some(why) => {
                            run.on_disabled_hover_text(why);
                        }
                        None if run.clicked() => self.start(ui.ctx(), cmd),
                        None => {}
                    }
                }
            }
            for (drawer, text, show, hide) in [
                (
                    Drawer::Cli,
                    "CLI",
                    "Show this page as a gw command line.",
                    "Hide the command line.",
                ),
                (Drawer::Log, "Log", "Show gw's output.", "Hide gw's output."),
            ] {
                ui.add_space(6.0);
                let open = self.settings.drawer == Some(drawer);
                let button = egui::Button::new(RichText::new(text).strong())
                    .selected(open)
                    .min_size(vec2(70.0, 40.0))
                    .corner_radius(8);
                if ui
                    .add(button)
                    .on_hover_text(if open { hide } else { show })
                    .clicked()
                {
                    self.settings.drawer = (!open).then_some(drawer);
                }
            }
        });
    }

    /// Why this page cannot run now, if it cannot.
    fn why_not(&self, cmd: &Command) -> Option<String> {
        let empty = Values::default();
        let values = self.settings.values.get(&cmd.name).unwrap_or(&empty);
        let output = |dest: &str| form::OUTPUTS.contains(&(cmd.name.as_str(), dest));
        let missing: Vec<String> = command::missing(cmd, values)
            .filter_map(|d| cmd.arg(d))
            .filter(|a| !form::GLOBAL.contains(&a.dest.as_str()) && !output(&a.dest))
            .map(|a| form::label(a).to_lowercase())
            .collect();
        let Some(schema) = self.schema.as_deref() else {
            return Some("Starting Greaseweazle…".to_owned());
        };
        if self.engine.is_none() {
            Some("Greaseweazle is not set up. See Settings.".to_owned())
        } else if self.running().is_some() {
            Some("Wait for the job that is running.".to_owned())
        } else if self.probe.is_some() {
            Some("Wait while the device says what it is.".to_owned())
        } else if !missing.is_empty() {
            Some(format!("Choose the {} first.", missing.join(" and ")))
        } else {
            self.diskdefs_fault(values)
                .or_else(|| form::blocked(schema, cmd, values, &self.settings.outputs))
                .map(str::to_owned)
        }
    }

    /// Why a disk definitions file stops the page: gw would refuse it too.
    fn diskdefs_fault(&self, values: &Values) -> Option<&'static str> {
        let path = values.get("diskdefs");
        if path.is_empty() {
            return None;
        }
        match self.service.known_diskdefs(path) {
            None | Some(Load::Waiting(_)) => Some("Checking the disk definitions file…"),
            Some(Load::Failed(_)) => Some("The disk definitions file cannot be read."),
            Some(Load::Ready(d)) if !d.errors.is_empty() => {
                Some("The disk definitions file has errors. See Advanced options.")
            }
            Some(Load::Ready(_)) => None,
        }
    }

    fn stop(&mut self) {
        self.detect_for = None;
        self.session = None;
        for job in [&mut self.disk, &mut self.tool].into_iter().flatten() {
            job.stop();
        }
    }

    /// Runs the page, naming first any files it would replace.
    fn start(&mut self, ctx: &egui::Context, cmd: &Command) {
        let values = self.values_for(cmd);
        let (runs, files) = runs(cmd, values, &self.settings.outputs, |v| self.argv(cmd, v));
        let files: Vec<PathBuf> = files.into_iter().filter(|f| f.exists()).collect();
        if files.is_empty() {
            self.begin(ctx, &cmd.name, runs);
        } else {
            self.dialog = Some(Dialog::Overwrite {
                files,
                command: cmd.name.clone(),
                runs,
            });
        }
    }

    fn begin(&mut self, ctx: &egui::Context, command: &str, mut runs: Vec<Vec<String>>) {
        if runs.len() > 1 {
            self.session = Some(Session { runs, next: 0 });
            self.next_disk(ctx);
        } else if let Some(args) = runs.pop() {
            self.confirm_or_run(ctx, command, args);
        }
    }

    /// Reads the session's next disk.
    fn next_disk(&mut self, ctx: &egui::Context) {
        let Some(session) = &mut self.session else {
            return;
        };
        let Some(args) = session.runs.get(session.next).cloned() else {
            self.session = None;
            return;
        };
        session.next += 1;
        let part = (session.next, session.runs.len());
        self.run(ctx, "read", args);
        if let Some(job) = &mut self.disk {
            job.part = Some(part);
        }
    }

    fn confirm_or_run(&mut self, ctx: &egui::Context, command: &str, args: Vec<String>) {
        if DESTRUCTIVE.iter().any(|(c, _)| *c == command) {
            self.dialog = Some(Dialog::Confirm {
                command: command.to_owned(),
                args,
            });
        } else {
            self.run(ctx, command, args);
        }
    }

    fn run(&mut self, ctx: &egui::Context, command: &str, args: Vec<String>) {
        let Some(engine) = &self.engine else { return };
        // The image the job writes. A read's is its last argument, one per disk.
        let output = match command {
            "read" => args
                .last()
                .map(|a| PathBuf::from(a.split("::").next().unwrap_or(a))),
            _ => form::OUTPUTS
                .iter()
                .filter(|(c, _)| *c == command)
                .find_map(|(_, dest)| {
                    self.settings
                        .values
                        .get(command)?
                        .get(dest)
                        .split("::")
                        .next()
                        .map(PathBuf::from)
                })
                .filter(|p| !p.as_os_str().is_empty()),
        };
        if let Some(folder) = output.as_ref().and_then(|p| p.parent()) {
            let _ = std::fs::create_dir_all(folder);
        }
        match Job::start(engine, command, args, repaint(ctx)) {
            Ok(mut job) => {
                job.output = output;
                job.format = job
                    .args
                    .iter()
                    .find_map(|a| a.strip_prefix("--format="))
                    .map(String::from);
                let disk = DISK_COMMANDS.contains(&command);
                *(if disk { &mut self.disk } else { &mut self.tool }) = Some(job);
                self.notice = None;
            }
            Err(e) => self.notice = Some(format!("Could not start gw: {e}")),
        }
    }

    /// The disk's status: what is happening now, or last happened, to a disk.
    fn status(&mut self, ui: &mut Ui, page: &str) {
        let p = theme::palette(ui);
        let blank = self.blank_map(page);
        let (top, full) = (ui.cursor().top(), ui.available_height());
        let budget = |ui: &Ui| full * MAP_SHARE - (ui.cursor().top() - top);
        // The job's state keeps to the top right, leaving the rows below to its name.
        egui::Sides::new().shrink_left().truncate().show(
            ui,
            |ui| ui.label(RichText::new("Disk status").size(16.0).strong()),
            |ui| {
                if let Some(job) = &self.disk {
                    let (text, colour) = state(job, p);
                    pill(ui, &format!("{text} · {}", clock(job.elapsed())), colour);
                }
            },
        );
        ui.add_space(4.0);
        let Some(job) = &self.disk else {
            ui.label(RichText::new(idle_status(page)).weak());
            ui.add_space(10.0);
            diskmap::show(ui, &blank, "blank", budget(ui));
            return;
        };
        // These rows wrap, so the job shows in full.
        let name = match job.part {
            Some((disk, total)) => format!("{} {disk} of {total}", title(&job.command)),
            None => title(&job.command),
        };
        ui.add(egui::Label::new(RichText::new(name).size(15.0).strong()).wrap());
        let file = job
            .output
            .as_ref()
            .and_then(|o| o.file_name())
            .map(|f| f.to_string_lossy().into_owned());
        let format = job
            .format
            .as_ref()
            .map(|f| format!("{} {f}", form::family_name(f).replace(' ', "\u{a0}")));
        let about: Vec<String> = format.into_iter().chain(file).collect();
        if !about.is_empty() {
            let text = RichText::new(about.join("  ·  ")).small().color(p.dim);
            ui.add(egui::Label::new(text).wrap());
        }
        ui.add_space(6.0);
        progress_bar(ui, job, p);
        if let Some(e) = &job.progress.error {
            ui.add_space(6.0);
            Frame::new()
                .fill(p.bad.gamma_multiply(0.14))
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(e).color(p.bad));
                });
        }
        ui.add_space(8.0);
        match job.progress.cyls.is_empty() && job.progress.tracks.is_empty() {
            true => diskmap::show(ui, &blank, "blank", budget(ui)),
            false => diskmap::show(ui, &job.progress, job.started, budget(ui)),
        }
    }

    /// An empty map the size of the page's format, or of a common disk.
    fn blank_map(&mut self, page: &str) -> Progress {
        let empty = Values::default();
        let values = self.settings.values.get(page).unwrap_or(&empty);
        let format = values.get("format");
        let diskdefs = form::diskdefs_for(&mut self.service, values, format);
        let info = match format.is_empty() {
            true => None,
            false => self.service.format_info(&diskdefs, format).ready(),
        };
        let (cyls, heads) = info.map_or(form::USUAL_DISK, |i| (i.cyls, i.heads));
        Progress::blank(cyls, heads)
    }

    /// The drawer under the page and the status pane: the command line or the log.
    fn drawer(&mut self, ui: &mut Ui, page: &str, drawer: Drawer) {
        let p = theme::palette(ui);
        let frame = Frame::new().fill(p.bg).inner_margin(Margin {
            left: 28,
            right: 18,
            top: 12,
            bottom: 14,
        });
        match drawer {
            Drawer::Cli => {
                egui::Panel::bottom("cli")
                    .frame(frame)
                    .resizable(false)
                    .exact_size(DRAWER)
                    .show(ui, |ui| self.cli(ui, page));
            }
            Drawer::Log => {
                // As tall as the command line at first; drag its edge for more.
                let tallest = (ui.available_height() - LOG_ROOM).max(DRAWER);
                egui::Panel::bottom("log")
                    .frame(frame)
                    .resizable(true)
                    .default_size(DRAWER)
                    .size_range(DRAWER..=tallest)
                    .show(ui, |ui| {
                        // The latest job, of either kind.
                        let latest = [&self.disk, &self.tool]
                            .into_iter()
                            .flatten()
                            .max_by_key(|j| j.started);
                        let log = latest.map_or(&[][..], |j| j.log.as_slice());
                        // Exactly the room there is, or the drawer grows to fit.
                        let height = (ui.available_height() - LOG_HEADING).max(LOG_LINE);
                        output(ui, "Log", log, p, height);
                    });
            }
        }
    }

    /// The page's gw command, which follows the page. Typing one fills in the
    /// page and runs nothing.
    fn cli(&mut self, ui: &mut Ui, page: &str) {
        let p = theme::palette(ui);
        let Some(schema) = self.schema.clone() else {
            return;
        };
        let Some(cmd) = schema.command(page) else {
            return;
        };
        let line = command::line(&self.args(cmd));
        let cli = &mut self.cli;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Command line").strong());
            right(ui, |ui| {
                if ui
                    .button("Copy")
                    .on_hover_text("Copy this command.")
                    .clicked()
                {
                    ui.ctx().copy_text(cli.text.clone());
                }
            });
        });
        ui.add_space(4.0);
        let id = ui.make_persistent_id("cli-text");
        // While it is being typed in, the text is the person's own.
        if !ui.memory(|m| m.has_focus(id)) {
            cli.text = line;
            cli.error = None;
        }
        let edit = ui
            .add(
                TextEdit::multiline(&mut cli.text)
                    .id(id)
                    .font(TextStyle::Monospace)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            )
            .on_hover_text("Type or paste options. The page follows.");
        let mut apply = None;
        if edit.changed() {
            match command::parse(&schema, &cli.text) {
                Ok(parsed) => {
                    cli.error = None;
                    apply = Some(parsed);
                }
                Err(e) => cli.error = Some(e),
            }
        }
        if let Some(e) = &cli.error {
            ui.label(RichText::new(e).small().color(p.bad));
        }
        if let Some((name, values)) = apply {
            self.fill_in(name, values);
        }
    }

    fn presets_menu(&mut self, ui: &mut Ui, command: &str) {
        let folder = self.presets_folder();
        let mut load = None;
        let mut save = false;
        let mut pick = false;
        ui.menu_button("Presets", |ui| {
            ui.set_min_width(220.0);
            let saved = presets::list(&folder, command);
            if saved.is_empty() {
                ui.label(RichText::new("No presets saved yet.").weak());
            }
            for (name, path) in saved {
                if ui
                    .button(name)
                    .on_hover_text("Use these settings.")
                    .clicked()
                {
                    load = Some(path);
                    ui.close();
                }
            }
            ui.separator();
            save = ui
                .button("Save…")
                .on_hover_text("Save this page's settings as a preset.")
                .clicked();
            pick = ui
                .button("Load…")
                .on_hover_text("Load a preset from a file.")
                .clicked();
            if save || pick {
                ui.close();
            }
        });
        if save {
            self.dialog = Some(Dialog::SavePreset {
                command: command.to_owned(),
                name: String::new(),
            });
        }
        if pick {
            load = rfd::FileDialog::new()
                .add_filter("Presets", &["json"])
                .set_directory(&folder)
                .pick_file();
        }
        if let Some(path) = load {
            self.load_preset(&path);
        }
    }

    fn settings_page(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| self.settings_inner(ui));
    }

    fn settings_inner(&mut self, ui: &mut Ui) {
        ui.heading("Settings");
        ui.add_space(18.0);
        let p = theme::palette(ui);
        section(ui, "Theme", |ui| {
            ui.horizontal(|ui| {
                for (pref, text, tip) in [
                    (ThemePreference::System, "System", "Follow the system."),
                    (ThemePreference::Light, "Light", "Always light."),
                    (ThemePreference::Dark, "Dark", "Always dark."),
                ] {
                    let r = ui.selectable_label(self.settings.theme == pref, text);
                    if r.on_hover_text(tip).clicked() {
                        self.settings.theme = pref;
                        ui.ctx().set_theme(pref);
                    }
                }
            });
        });
        section(ui, "Greaseweazle", |ui| {
            match (&self.engine, &self.service.schema) {
                (Some(engine), Load::Ready(schema)) => {
                    let origin = match engine.origin {
                        Origin::Bundled => "built in",
                        Origin::Installed => "installed on this computer",
                        Origin::Custom => "chosen here",
                    };
                    ui.label(format!("Greaseweazle {}, {origin}.", schema.version));
                    let path = RichText::new(engine.python.to_string_lossy())
                        .monospace()
                        .small()
                        .weak();
                    ui.label(path).on_hover_text("The Python that runs gw.");
                }
                (Some(_), Load::Waiting(_)) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Starting…");
                    });
                }
                (_, load) => {
                    ui.label(RichText::new(load.error().unwrap_or("Not found.")).color(p.bad));
                }
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let choose = ui
                    .button("Use another gw…")
                    .on_hover_text("Choose a gw, or a Python with greaseweazle.");
                if choose.clicked()
                    && let Some(path) = rfd::FileDialog::new().pick_file()
                {
                    self.settings.engine = Some(path);
                    self.connect(ui.ctx());
                }
                if self.settings.engine.is_some()
                    && ui
                        .button("Use the built-in gw")
                        .on_hover_text("Go back to the gw Ferriteweazle ships.")
                        .clicked()
                {
                    self.settings.engine = None;
                    self.connect(ui.ctx());
                }
                if ui
                    .button("Restart")
                    .on_hover_text("Start gw again.")
                    .clicked()
                {
                    self.connect(ui.ctx());
                }
            });
        });
        section(ui, "Presets", |ui| {
            let folder = self.presets_folder();
            ui.label("Presets folder:");
            ui.label(
                RichText::new(folder.to_string_lossy())
                    .monospace()
                    .small()
                    .weak(),
            );
            ui.horizontal(|ui| {
                if ui
                    .button("Choose…")
                    .on_hover_text("Choose the presets folder.")
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new().set_directory(&folder).pick_folder()
                {
                    self.settings.presets_folder = Some(path);
                }
                if self.settings.presets_folder.is_some()
                    && ui
                        .button("Use the default")
                        .on_hover_text("Documents/Ferriteweazle.")
                        .clicked()
                {
                    self.settings.presets_folder = None;
                }
            });
        });
        section(ui, "Jobs", |ui| {
            setting(
                ui,
                &mut self.settings.save_logs,
                "Save gw's output beside each image",
                "Writes name.ext.log next to the image.",
            );
            if cfg!(target_os = "macos") {
                setting(
                    ui,
                    &mut self.settings.sound,
                    "Play a sound when a job ends",
                    "Glass when it works, Basso when it fails.",
                );
            }
        });
        section(ui, "Troubleshooting", |ui| {
            setting(
                ui,
                &mut self.settings.backtrace,
                "Show Python tracebacks",
                "Passes --bt, so gw's errors say where they came from.",
            );
        });
        section(ui, "About", |ui| {
            ui.label(format!(
                "Ferriteweazle {}, under the MIT licence.",
                env!("CARGO_PKG_VERSION")
            ));
            ui.hyperlink_to("Source code and issues", REPO)
                .on_hover_text(REPO);
            ui.horizontal(|ui| {
                ui.label("Greaseweazle is by Keir Fraser:");
                ui.hyperlink_to(
                    "github.com/keirf/greaseweazle",
                    "https://github.com/keirf/greaseweazle",
                );
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let quitting = matches!(self.dialog, Some(Dialog::Quit));
        let asking = [&mut self.disk, &mut self.tool]
            .into_iter()
            .flatten()
            .find(|j| j.question.is_some());
        if let Some(job) = asking.filter(|_| !quitting) {
            return ask(ctx, job);
        }
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        // The job ended while Quit asked: nothing is left to stop.
        if quitting && self.running().is_none() {
            self.quitting = true;
            return;
        }
        let mut close = false;
        let mut action: Option<Action> = None;
        let response = egui::Modal::new(Id::new("dialog")).show(ctx, |ui| {
            ui.set_width(420.0);
            match &mut dialog {
                Dialog::Confirm { command, args } => {
                    let drive = if self.settings.drive.is_empty() {
                        "A"
                    } else {
                        self.settings.drive.as_str()
                    };
                    let verb = title(command);
                    let verb = verb.split(' ').next().unwrap_or_default();
                    dialog_heading(ui, &format!("{verb} the disk in drive {drive}?"));
                    let why = DESTRUCTIVE
                        .iter()
                        .find(|(c, _)| c == command)
                        .map_or("", |(_, w)| *w);
                    ui.label(why);
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        let p = theme::palette(ui);
                        if ui
                            .add(dialog_button(run_label(command), p.bad, p))
                            .clicked()
                        {
                            let (ctx, command, args) = (ctx.clone(), command.clone(), args.clone());
                            action =
                                Some(Box::new(move |app: &mut App| app.run(&ctx, &command, args)));
                            close = true;
                        }
                        if ui.add(dialog_plain("Cancel")).clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Overwrite {
                    files,
                    command,
                    runs,
                } => {
                    let name = |f: &PathBuf| {
                        f.file_name()
                            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                    };
                    let heading = match files.as_slice() {
                        [one] => format!("Overwrite \"{}\"?", name(one)),
                        many => format!("Overwrite {} files?", many.len()),
                    };
                    dialog_heading(ui, &heading);
                    if files.len() > 1 {
                        let mut names: Vec<String> = files.iter().take(4).map(name).collect();
                        if files.len() > 4 {
                            names.push(format!("and {} more", files.len() - 4));
                        }
                        ui.label(names.join(", "));
                    }
                    ui.label("This cannot be undone.");
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        let p = theme::palette(ui);
                        if ui.add(dialog_button("Overwrite", p.bad, p)).clicked() {
                            let (ctx, command, runs) = (ctx.clone(), command.clone(), runs.clone());
                            action = Some(Box::new(move |app: &mut App| {
                                app.begin(&ctx, &command, runs)
                            }));
                            close = true;
                        }
                        if ui.add(dialog_plain("Cancel")).clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::NextDisk {
                    disk,
                    total,
                    failed,
                } => {
                    dialog_heading(ui, &format!("Insert disk {disk} of {total}"));
                    if *failed {
                        let p = theme::palette(ui);
                        let text = format!("Disk {} failed. Its output says why.", *disk - 1);
                        ui.label(RichText::new(text).color(p.bad));
                    }
                    ui.label("Put the next disk in the drive, then read it.");
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        let p = theme::palette(ui);
                        let read = format!("Read disk {disk}");
                        if ui.add(dialog_button(&read, p.accent, p)).clicked() {
                            let ctx = ctx.clone();
                            action = Some(Box::new(move |app: &mut App| app.next_disk(&ctx)));
                            close = true;
                        }
                        if ui
                            .add(dialog_plain("Stop here"))
                            .on_hover_text("End the session. The disks read so far are kept.")
                            .clicked()
                        {
                            action = Some(Box::new(|app: &mut App| app.session = None));
                            close = true;
                        }
                    });
                }
                Dialog::SavePreset { command, name } => {
                    dialog_heading(ui, "Save a preset");
                    ui.add(
                        form::edit(name)
                            .hint_text("e.g. Amiga DD")
                            .desired_width(f32::INFINITY),
                    )
                    .request_focus();
                    let exists = presets::path(&self.presets_folder(), name).exists();
                    if exists {
                        let p = theme::palette(ui);
                        let text = "A preset of this name exists. Saving replaces it.";
                        ui.label(RichText::new(text).small().color(p.partial));
                    }
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        let name = name.trim().to_owned();
                        let p = theme::palette(ui);
                        let text = if exists { "Replace" } else { "Save" };
                        if ui
                            .add_enabled(!name.is_empty(), dialog_button(text, p.accent, p))
                            .clicked()
                        {
                            let command = command.clone();
                            action = Some(Box::new(move |app: &mut App| {
                                app.save_preset(&command, &name)
                            }));
                            close = true;
                        }
                        if ui.add(dialog_plain("Cancel")).clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Quit => {
                    let job = self
                        .running()
                        .map_or_else(String::new, |j| title(&j.command));
                    dialog_heading(ui, &format!("Stop {job} and quit?"));
                    ui.label("gw stops the drive first, then the window closes.");
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        let p = theme::palette(ui);
                        if ui.add(dialog_button("Stop and quit", p.bad, p)).clicked() {
                            action = Some(Box::new(|app: &mut App| {
                                app.quitting = true;
                                app.stop();
                            }));
                            close = true;
                        }
                        if ui.add(dialog_plain("Keep running")).clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
        if close || response.should_close() {
            // A session waits on its dialog: closing that any other way ends it.
            if action.is_none() && matches!(dialog, Dialog::NextDisk { .. }) {
                self.session = None;
            }
        } else {
            self.dialog = Some(dialog);
        }
        if let Some(action) = action {
            action(self);
        }
    }

    /// Opens a pasted command's page with its settings.
    fn fill_in(&mut self, name: String, mut values: Values) {
        for dest in form::GLOBAL {
            let value = values.get(dest).to_owned();
            if !value.is_empty() {
                let global = if dest == "device" {
                    &mut self.settings.device
                } else {
                    &mut self.settings.drive
                };
                *global = value;
                values.set(dest, "");
            }
        }
        for (cmd, dest) in form::OUTPUTS.iter().filter(|(c, _)| *c == name) {
            let file = values.get(dest);
            let key = form::output_key(cmd, dest);
            // The file the page names already, such as a set's first disk, keeps its settings.
            let same = self
                .settings
                .outputs
                .get(&key)
                .is_some_and(|o| o.value(1) == file);
            if !file.is_empty() && !same {
                self.settings.outputs.insert(key, Output::from_value(file));
            }
        }
        // The line leaves out a definitions file the chosen format does not use.
        if values.get("diskdefs").is_empty()
            && let Some(kept) = self.settings.values.get(&name).map(|v| v.get("diskdefs"))
            && !kept.is_empty()
        {
            values.set("diskdefs", kept);
        }
        self.settings.values.insert(name.clone(), values);
        self.settings.page = Page::Command(name);
    }

    fn presets_folder(&self) -> PathBuf {
        self.settings
            .presets_folder
            .clone()
            .unwrap_or_else(presets::default_folder)
    }

    fn save_preset(&mut self, command: &str, name: &str) {
        let values = self
            .settings
            .values
            .get(command)
            .cloned()
            .unwrap_or_default();
        let prefix = format!("{command}/");
        let outputs = self
            .settings
            .outputs
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let preset = Preset {
            command: command.to_owned(),
            values,
            outputs,
        };
        if let Err(e) = presets::save(&self.presets_folder(), name, &preset) {
            self.notice = Some(format!("Could not save the preset: {e}"));
        }
    }

    /// Applies a preset file's settings and opens its page.
    fn load_preset(&mut self, path: &Path) {
        match presets::load(path) {
            Ok(preset) => {
                self.settings
                    .values
                    .insert(preset.command.clone(), preset.values);
                self.settings.outputs.extend(preset.outputs);
                self.settings.page = Page::Command(preset.command);
            }
            Err(e) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.notice = Some(format!("Could not load {name}. {e}"));
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
}

/// A question from gw, such as whether to seek past the last cylinder.
fn ask(ctx: &egui::Context, job: &mut Job) {
    let question = job.question.clone().unwrap_or_default();
    let id = Id::new("answer");
    egui::Modal::new(Id::new("question")).show(ctx, |ui| {
        ui.set_width(400.0);
        dialog_heading(ui, "gw asks");
        ui.label(question.trim());
        ui.add_space(10.0);
        if question.contains("Yes/No") {
            right(ui, |ui| {
                if ui.add(dialog_plain("Yes")).clicked() {
                    job.answer("Yes");
                }
                if ui.add(dialog_plain("No")).clicked() {
                    job.answer("No");
                }
            });
        } else {
            let mut text: String = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
            ui.add(form::edit(&mut text).desired_width(f32::INFINITY));
            ui.data_mut(|d| d.insert_temp(id, text.clone()));
            if ui.add(dialog_plain("Answer")).clicked() {
                job.answer(&text);
                ui.data_mut(|d| d.remove_temp::<String>(id));
            }
        }
    });
}

/// How far a disk job has got, and the share done if known: sectors found
/// once gw has counted them, else tracks done.
fn progress_text(job: &Job) -> Option<(String, Option<f32>)> {
    let progress = &job.progress;
    let done = progress.tally().done;
    let planned = progress.cyls.len() * progress.heads.len();
    let share = |a: u32, b: usize| (b > 0).then(|| a as f32 / b as f32);
    match progress.total {
        Some((good, all)) => Some((format!("{good} / {all} sectors"), share(good, all as usize))),
        None if planned > 0 => Some((format!("{done} / {planned} tracks"), share(done, planned))),
        None if done == 1 => Some(("1 track".into(), None)),
        None if done > 1 => Some((format!("{done} tracks"), None)),
        None => None,
    }
}

/// How far a job has got: a slim bar, with the count at its end. A job
/// with no known end shows the count alone.
fn progress_bar(ui: &mut Ui, job: &Job, p: &Palette) {
    let Some((text, share)) = progress_text(job) else {
        return;
    };
    let tally = job.progress.tally();
    let colour = match job.outcome() {
        None => p.accent,
        Some(Outcome::Failed) => p.bad,
        Some(Outcome::Stopped) => p.dim,
        Some(Outcome::Succeeded) if tally.partial + tally.bad > 0 => p.partial,
        Some(Outcome::Succeeded) => p.good,
    };
    text_row(ui, |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(text).small().color(p.dim));
            let Some(share) = share else { return };
            ui.add_space(6.0);
            let size = vec2(ui.available_width(), PROGRESS_BAR);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let round = CornerRadius::same((PROGRESS_BAR / 2.0) as u8);
            ui.painter().rect_filled(rect, round, p.pending);
            let done = rect.with_max_x(rect.left() + rect.width() * share.clamp(0.0, 1.0));
            ui.painter().rect_filled(done, round, colour);
        });
    });
}

/// The progress bar's thickness.
const PROGRESS_BAR: f32 = 6.0;

fn log_line(line: &str, p: &Palette) -> RichText {
    let text = RichText::new(line).monospace();
    if line.starts_with("** FATAL")
        || line.contains(": error:")
        || line.starts_with("Command Failed")
    {
        text.color(p.bad)
    } else if line.contains("WARNING") || line.contains("Giving up") || line.contains("(Retry #") {
        text.color(p.partial)
    } else {
        text
    }
}

fn title(command: &str) -> String {
    match NAMES.iter().find(|(c, _, _)| *c == command) {
        Some((_, title, _)) => (*title).to_owned(),
        None => form::sentence(command).trim_end_matches('.').to_owned(),
    }
}

fn run_label(command: &str) -> &str {
    NAMES
        .iter()
        .find(|(c, _, _)| *c == command)
        .map_or("Run", |(_, _, run)| run)
}

/// gw's arguments for each run of a command, and the files they make: a
/// read of several disks runs once per disk.
fn runs(
    cmd: &Command,
    mut values: Values,
    outputs: &BTreeMap<String, Output>,
    argv: impl Fn(&Values) -> Vec<String>,
) -> (Vec<Vec<String>>, Vec<PathBuf>) {
    let out = form::OUTPUTS
        .iter()
        .find(|(c, _)| *c == cmd.name)
        .and_then(|(c, dest)| Some((*dest, outputs.get(&form::output_key(c, dest))?)));
    match out {
        Some((dest, out)) if cmd.name == "read" => {
            let runs = (1..=out.disks.max(1))
                .map(|d| {
                    values.set(dest, out.value(d));
                    argv(&values)
                })
                .collect();
            (runs, out.paths().collect())
        }
        Some((_, out)) => (vec![argv(&values)], vec![out.path(1)]),
        None => (vec![argv(&values)], Vec::new()),
    }
}

/// What the status pane says before any disk job, for this page.
fn idle_status(page: &str) -> &'static str {
    match page {
        "write" => "No disk written yet",
        "erase" => "No disk erased yet",
        "convert" => "No image converted yet",
        _ => "No disk read yet",
    }
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

fn repaint(ctx: &egui::Context) -> Repaint {
    let ctx = ctx.clone();
    Box::new(move || ctx.request_repaint())
}

fn big_button<'a>(text: &'a str, fill: Color32, p: &Palette) -> egui::Button<'a> {
    egui::Button::new(RichText::new(text).color(p.on_accent).strong().size(15.0))
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(8)
        .min_size(vec2(170.0, 40.0))
}

/// Every dialog button's height.
const DIALOG_BUTTON: f32 = 32.0;

fn dialog_heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(16.0).strong());
    ui.add_space(6.0);
}

fn dialog_button<'a>(text: &'a str, fill: Color32, p: &Palette) -> egui::Button<'a> {
    egui::Button::new(RichText::new(text).color(p.on_accent).strong())
        .fill(fill)
        .stroke(Stroke::NONE)
        .min_size(vec2(120.0, DIALOG_BUTTON))
}

fn dialog_plain(text: &str) -> egui::Button<'_> {
    egui::Button::new(text).min_size(vec2(90.0, DIALOG_BUTTON))
}

/// gw info's report on the device, as a table.
fn device_table(ui: &mut Ui, info: &DeviceInfo, p: &Palette) {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(8)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("device-info")
                .num_columns(2)
                .spacing([16.0, 5.0])
                .show(ui, |ui| {
                    for (key, value) in &info.fields {
                        ui.label(RichText::new(key).weak());
                        ui.label(value);
                        ui.end_row();
                    }
                });
            if let Some(update) = &info.update {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("Firmware {update} is available.")).color(p.accent));
            }
        });
}

/// The sidebar's sections and the commands this gw has in each.
fn sections(schema: Option<&Schema>) -> Vec<(&'static str, Vec<&str>)> {
    let names: Vec<&str> = match schema {
        Some(s) => s.commands.iter().map(|c| c.name.as_str()).collect(),
        None => SECTIONS
            .iter()
            .flat_map(|(_, names)| names.iter().copied())
            .collect(),
    };
    let mut out: Vec<(&str, Vec<&str>)> = SECTIONS
        .iter()
        .map(|(section, known)| {
            (
                *section,
                known
                    .iter()
                    .copied()
                    .filter(|k| names.contains(k))
                    .collect(),
            )
        })
        .collect();
    let other: Vec<&str> = names
        .into_iter()
        .filter(|n| !SECTIONS.iter().any(|(_, known)| known.contains(n)))
        .collect();
    if !other.is_empty() {
        out.push(("Other", other));
    }
    out
}

/// Plays a system sound for how a job ended, on macOS only.
fn chime(outcome: Option<Outcome>) {
    let sound = match outcome {
        Some(Outcome::Succeeded) => "Glass",
        Some(Outcome::Failed) => "Basso",
        _ => return,
    };
    if cfg!(target_os = "macos") {
        let file = format!("/System/Library/Sounds/{sound}.aiff");
        // A thread waits for afplay, so it is reaped when it ends.
        std::thread::spawn(move || {
            std::process::Command::new("/usr/bin/afplay")
                .arg(file)
                .status()
        });
    }
}

/// A row laid out from the right: the first thing added sits rightmost.
fn right<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), add)
            .inner
    })
    .inner
}

/// A switch with its label, and help on both.
fn setting(ui: &mut Ui, on: &mut bool, label: &str, tip: &str) {
    ui.horizontal(|ui| {
        form::toggle(ui, on).on_hover_text(tip);
        ui.label(label).on_hover_text(tip);
    });
}

fn state(job: &Job, p: &Palette) -> (&'static str, Color32) {
    match job.outcome() {
        None if job.stopping() => ("Stopping", p.partial),
        None => ("Running", p.accent),
        Some(Outcome::Succeeded) => ("Done", p.good),
        Some(Outcome::Failed) => ("Failed", p.bad),
        Some(Outcome::Stopped) => ("Stopped", p.dim),
    }
}

/// What a command other than a disk job did, under its page.
fn result(ui: &mut Ui, job: &Job) {
    let p = theme::palette(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Result").strong());
        let (text, colour) = state(job, p);
        pill(ui, text, colour);
        right(ui, |ui| {
            ui.label(RichText::new(clock(job.elapsed())).monospace().weak());
        });
    });
    if let Some(e) = &job.progress.error {
        ui.label(RichText::new(e).color(p.bad));
    }
    ui.add_space(4.0);
    // The device's report reads as a table; gw's raw words stay in the Log.
    if job.command == "info"
        && !job.running()
        && let Some(info) = device::parse(&job.log)
    {
        device_table(ui, &info, p);
    } else {
        output(ui, "Output", &job.log, p, 260.0);
    }
}

/// gw's output: a scrolling log of this height, with Copy and Save.
fn output(ui: &mut Ui, heading: &str, log: &[String], p: &Palette, height: f32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(heading).strong());
        right(ui, |ui| {
            let save = ui.add_enabled(!log.is_empty(), egui::Button::new("Save…"));
            if save.on_hover_text("Save gw's output to a file.").clicked()
                && let Some(path) = rfd::FileDialog::new().set_file_name("gw.log").save_file()
            {
                let _ = std::fs::write(&path, log.join("\n") + "\n");
            }
            let copy = ui.add_enabled(!log.is_empty(), egui::Button::new("Copy"));
            if copy.on_hover_text("Copy gw's output.").clicked() {
                ui.ctx().copy_text(log.join("\n"));
            }
        });
    });
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(8)
        .inner_margin(8)
        .show(ui, |ui| {
            if log.is_empty() {
                ui.set_min_size(vec2(ui.available_width(), height.min(80.0)));
                ui.label(RichText::new("gw's output appears here.").weak());
                return;
            }
            let row = ui.text_style_height(&TextStyle::Monospace);
            egui::ScrollArea::both()
                .id_salt("log")
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .max_height(height)
                .min_scrolled_height(height)
                .show_rows(ui, row, log.len(), |ui, rows| {
                    for line in &log[rows] {
                        ui.add(egui::Label::new(log_line(line, p)).extend());
                    }
                });
        });
}

/// The port chosen while it is connected, else the best Greaseweazle.
fn chosen_port<'p>(ports: &'p [Port], chosen: &str) -> Option<&'p Port> {
    ports
        .iter()
        .find(|p| !chosen.is_empty() && p.device == chosen)
        .or_else(|| ports.iter().find(|p| p.score > 0))
}

/// A port as people know it: COM3, ttyACM0, cu.usbmodem14201.
fn short_port(device: &str) -> &str {
    device.strip_prefix("/dev/").unwrap_or(device)
}

/// "Found akai.800. It also matches eagle.dsqd.800 and zx.quorum.ds80."
fn found_note(formats: &[String], step: u32) -> String {
    let mut note = format!("Found {}.", formats[0]);
    if step > 1 {
        note += " It is a 40-track disk in an 80-track drive, so Double step is on.";
    }
    match &formats[1..] {
        [] => {}
        [one] => note += &format!(" It also matches {one}."),
        more => {
            let (last, rest) = more[..more.len().min(4)]
                .split_last()
                .expect("more has some");
            note += &format!(" It also matches {} and {last}.", rest.join(", "));
        }
    }
    note
}

fn pill(ui: &mut Ui, text: &str, colour: Color32) {
    Frame::new()
        .fill(colour.gamma_multiply(0.16))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(colour).strong())
        });
}

fn section(ui: &mut Ui, heading: &str, add: impl FnOnce(&mut Ui)) {
    let p = theme::palette(ui);
    ui.label(RichText::new(heading).strong());
    ui.add_space(4.0);
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(10)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(640.0));
            add(ui);
        });
    ui.add_space(18.0);
}

/// A full-width sidebar entry, with a quiet note at its right such as gw's version.
fn nav_item(ui: &mut Ui, text: &str, note: Option<&str>, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), NAV_ROW), Sense::click());
    let p = theme::palette(ui);
    let fill = if selected {
        p.accent.gamma_multiply(0.16)
    } else if response.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    let colour = if selected { p.accent } else { p.text };
    ui.painter().text(
        pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(14.0),
        colour,
    );
    if let Some(note) = note {
        ui.painter().text(
            pos2(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            note,
            FontId::proportional(12.0),
            p.dim,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, text)
    });
    response
}

/// Fades the bottom of `rect` into `colour`.
fn fade_out(painter: &egui::Painter, rect: egui::Rect, colour: Color32) {
    let top = rect.bottom() - FADE;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(pos2(rect.left(), top), Color32::TRANSPARENT);
    mesh.colored_vertex(pos2(rect.right(), top), Color32::TRANSPARENT);
    mesh.colored_vertex(rect.left_bottom(), colour);
    mesh.colored_vertex(rect.right_bottom(), colour);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(egui::Shape::mesh(mesh));
}

/// How far a scrolled list fades out at its end.
const FADE: f32 = 26.0;

/// A row as tall as a line of text, where a plain row would be as tall as a button.
fn text_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 0.0;
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.horizontal(add);
    });
}

/// A small button with a painted circular arrow.
fn refresh_button(p: &'static Palette) -> impl egui::Widget {
    move |ui: &mut Ui| {
        let response = ui.add(
            egui::Button::new("")
                .min_size(vec2(22.0, 22.0))
                .frame_when_inactive(false),
        );
        let colour = if ui.is_enabled() {
            p.dim
        } else {
            p.line_strong
        };
        let c = response.rect.center();
        let r = 5.5;
        let at = |deg: f32| {
            let a = deg.to_radians();
            c + vec2(a.cos(), -a.sin()) * r
        };
        let arc = (0..=20).map(|i| at(60.0 + i as f32 * 13.5)).collect();
        ui.painter()
            .add(egui::Shape::line(arc, Stroke::new(1.6, colour)));
        // The head points along the arc, clockwise.
        let a = 60f32.to_radians();
        let (tip, along, out) = (at(60.0), vec2(a.sin(), a.cos()), vec2(a.cos(), -a.sin()));
        let head = vec![
            tip + along * 3.0,
            tip - along + out * 3.0,
            tip - along - out * 3.0,
        ];
        ui.painter()
            .add(egui::Shape::convex_polygon(head, colour, Stroke::NONE));
        response
    }
}

/// The logo, as a texture made once.
fn logo(ui: &mut Ui, texture: &mut Option<egui::TextureHandle>) {
    let texture = texture.get_or_insert_with(|| {
        let icon = eframe::icon_data::from_png_bytes(theme::LOGO).expect("the logo is a PNG");
        let size = [icon.width as usize, icon.height as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, &icon.rgba);
        // Mipmaps keep the drawing clean at a fraction of its size.
        let options = egui::TextureOptions {
            mipmap_mode: Some(egui::TextureFilter::Linear),
            ..egui::TextureOptions::LINEAR
        };
        ui.ctx().load_texture("logo", image, options)
    });
    // It reaches into the sidebar's margin, so it takes less room than its size.
    let size = Vec2::splat(LOGO_SIZE);
    let tuck = Vec2::splat(LOGO_TUCK);
    let (rect, _) = ui.allocate_exact_size(size - tuck, Sense::hover());
    let at = egui::Rect::from_min_size(rect.min - tuck, size);
    egui::Image::new((texture.id(), size)).paint_at(ui, at);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_save_and_load() {
        let mut s = Settings {
            page: Page::Command("convert".into()),
            theme: ThemePreference::Dark,
            drive: "B".into(),
            ..Settings::default()
        };
        s.values.entry("read".into()).or_default().set("revs", "5");
        let out = Output {
            disks: 7,
            ..Output::default()
        };
        s.outputs.insert("read/file".into(), out);
        s.presets_folder = Some("/presets".into());
        let saved = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(serde_json::to_string(&loaded).unwrap(), saved);
    }

    #[test]
    fn settings_missing_newer_fields_still_load() {
        let loaded: Settings = serde_json::from_str(r#"{"drive": "B"}"#).unwrap();
        assert_eq!(loaded.drive, "B");
        assert_eq!(loaded.page, Page::Command("read".into()));
    }

    #[test]
    fn a_read_of_three_disks_runs_gw_once_for_each() {
        let schema: Schema =
            serde_json::from_str(include_str!("../tests/data/schema-1.23.json")).unwrap();
        let read = schema.command("read").unwrap();
        let mut values = Values::default();
        values.set("format", "amiga.amigados");
        let out = Output {
            folder: "/f".into(),
            name: "Game".into(),
            ext: ".adf".into(),
            disks: 3,
            ..Output::default()
        };
        let outputs = BTreeMap::from([("read/file".to_owned(), out)]);
        let (runs, files) = runs(read, values, &outputs, |v| command::argv(read, v));
        let last: Vec<&str> = runs.iter().map(|r| r.last().unwrap().as_str()).collect();
        assert_eq!(
            last,
            [
                "/f/Game_Disk1.adf",
                "/f/Game_Disk2.adf",
                "/f/Game_Disk3.adf"
            ]
        );
        assert!(
            runs.iter()
                .all(|r| r.contains(&"--format=amiga.amigados".into()))
        );
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn the_device_card_asks_gw_info_with_no_page_options() {
        let schema: Schema =
            serde_json::from_str(include_str!("../tests/data/schema-1.23.json")).unwrap();
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        settings
            .values
            .entry("info".into())
            .or_default()
            .set("bootloader", command::ON);
        let app = App::offline(&ctx, settings, Ok(schema.clone()));
        let info = schema.command("info").unwrap();
        assert!(
            app.args(info).contains(&"--bootloader".to_owned()),
            "the page's own run"
        );
        assert_eq!(app.argv(info, &app.device_only(info)), ["info"]);
    }

    #[test]
    fn a_pasted_output_path_becomes_folder_name_and_type() {
        let out = Output::from_value("/disks/Game Disk.HFE::version=3");
        assert_eq!(
            (out.folder.as_str(), out.name.as_str(), out.ext.as_str()),
            ("/disks", "Game Disk", ".hfe")
        );
        assert_eq!(out.value(1), "/disks/Game Disk.hfe::version=3");
    }
}

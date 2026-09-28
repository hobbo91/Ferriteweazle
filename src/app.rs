//! The window: a sidebar, a page per gw command, and the disk's status beside it.

use crate::command::{self, Values};
use crate::device::{self, DeviceInfo};
use crate::diskmap;
use crate::engine::{self, Engine, Origin};
use crate::form::{self, Form, Output};
use crate::job::{DETECT, Job, Outcome, SessionLog};
use crate::presets::{self, Preset};
use crate::progress::Progress;
use crate::schema::{Command, Port, Schema};
use crate::service::{Load, Repaint, Service};
use crate::theme::{self, Palette};
use crate::update::{self, Install, Update};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Layout, Margin, RichText, Sense,
    Stroke, TextEdit, TextStyle, Theme, ThemePreference, Ui, UserAttentionType, Vec2,
    ViewportCommand, pos2, vec2,
};
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

/// Why a command that uses the device cannot run.
const NO_DEVICE: &str = "Connect a Greaseweazle.";

/// Commands the status pane shows. Others show their results under their page.
const DISK_COMMANDS: &[&str] = &["read", "write", "convert", "erase", "align", DETECT];

const REPO: &str = "https://github.com/hobbo91/ferriteweazle";
const GW_REPO: &str = "https://github.com/keirf/greaseweazle";

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
/// How long a theme chosen in Settings takes to fade in, in seconds.
const FADE_TIME: f32 = 0.25;
/// The most of a frame the fade counts, in seconds, so a stall cannot skip it.
const FADE_STEP: f32 = 1.0 / 30.0;
/// Frames to wait for the old theme's screenshot before changing at once.
const FADE_WAIT: u32 = 8;
/// One line of the log.
const LOG_LINE: f32 = 18.0;
/// The drawer's height, margins included: the command line's, and the log's at first.
const DRAWER: f32 = 124.0;
/// Height the log leaves the page above it, however far it is dragged.
const LOG_ROOM: f32 = 260.0;
/// How long a drawer takes to slide open or shut, in seconds.
const DRAWER_TIME: f32 = 0.2;
/// How far past its least height the log must be dragged to shut, in points.
const LOG_BUMP: f32 = 40.0;
/// The status pane's strip for its scroll bar, taken from its right margin.
const STATUS_BAR: i8 = 10;

#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    Command(String),
    Settings,
}

impl Default for Page {
    fn default() -> Self {
        Page::Command("read".into())
    }
}

/// The choices made in the window. Nothing is saved: every run starts afresh.
#[derive(Debug, Clone, Default)]
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
    /// Where new images go; unset, Documents/Ferriteweazle/Images.
    pub images_folder: Option<PathBuf>,
    /// Where presets are saved and listed from; unset, Documents/Ferriteweazle/Presets.
    pub presets_folder: Option<PathBuf>,
    /// The drawer open under the page and the status pane, if one is.
    pub drawer: Option<Drawer>,
}

/// What the drawer under the page and the status pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawer {
    /// The page's gw command line.
    Cli,
    /// gw's output from every job of the session.
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

/// A theme chosen in Settings: the old theme's last frame fades out over the new.
#[derive(Default)]
struct Fade {
    /// The theme to change to once the screenshot comes, and frames waited for it.
    asked: Option<(ThemePreference, u32)>,
    /// The old theme's last frame, and how long it has faded, in seconds.
    shown: Option<(egui::TextureHandle, f32)>,
}

/// Marks the screenshot a theme change asks for.
struct FadeShot;

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
    /// The latest gw's description, none once a gw fails to start. The sidebar
    /// lists its commands, and keeps them while gw restarts.
    listed: Option<Arc<Schema>>,
    /// The last job about a disk, which the status pane shows.
    pub disk: Option<Job>,
    /// The last job of any other command, shown under its page.
    pub tool: Option<Job>,
    /// Every job's output since the app opened: the Log drawer.
    pub log: SessionLog,
    /// The page a running detect job chooses the format on.
    detect_for: Option<String>,
    session: Option<Session>,
    cli: Cli,
    dialog: Option<Dialog>,
    /// Notes above pages, such as the formats detection found, by the page
    /// each came from. Each stays until dismissed or replaced there.
    pub notices: BTreeMap<String, String>,
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
    /// The real app, not a test window: it asks each Greaseweazle that
    /// appears what it is, and GitHub for newer gw releases.
    live: bool,
    gw_update: Update,
    app_update: Update,
    logo: Option<egui::TextureHandle>,
    fade: Fade,
    /// The drawer open when the drawers were last drawn.
    drawn: Option<Drawer>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let mut app = App::with_settings(&cc.egui_ctx, Settings::default());
        app.live = true;
        update::tidy();
        app.look_for_updates(&cc.egui_ctx);
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
        let known = schema.as_ref().ok().cloned().map(Arc::new);
        App {
            settings,
            engine: None,
            schema: known.clone(),
            listed: known,
            service: Service::offline(schema),
            disk: None,
            tool: None,
            log: SessionLog::default(),
            detect_for: None,
            session: None,
            cli: Cli::default(),
            dialog: None,
            notices: BTreeMap::new(),
            quitting: false,
            device: None,
            probe: None,
            probe_failed: None,
            probed: None,
            live: false,
            gw_update: Update::default(),
            app_update: Update::default(),
            logo: None,
            fade: Fade::default(),
            drawn: None,
        }
    }

    fn connect(&mut self, ctx: &egui::Context) {
        self.schema = None;
        self.engine = Engine::find(self.settings.engine.as_deref());
        // A new gw finds the same device, so the window keeps it meanwhile.
        let ports = self.service.known_ports().to_vec();
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
        self.service.seed_ports(ports);
        self.look_for_updates(ctx);
    }

    /// Asks GitHub for newer releases of the built-in gw and of this app.
    fn look_for_updates(&mut self, ctx: &egui::Context) {
        let Some(engine) = self.engine.as_ref().filter(|_| self.live) else {
            return;
        };
        if engine.origin == Origin::Bundled {
            self.gw_update = Update::check(engine, None, repaint(ctx));
        }
        if Install::this().is_some() {
            self.app_update = Update::check(engine, Some(update::APP_REPO), repaint(ctx));
        }
    }

    /// Shows these ports as the connected devices, whatever gw finds, until
    /// gw restarts: for tests and pictures of the window.
    pub fn pin_ports(&mut self, ports: Vec<Port>) {
        self.service.pin_ports(ports);
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
        self.fade_theme(&ctx);
        self.poll(&ctx);
        if self
            .gw_update
            .poll(self.schema.as_deref().map(|s| s.version.as_str()))
        {
            self.connect(&ctx);
        }
        if self.app_update.poll(Some(env!("CARGO_PKG_VERSION")))
            && let (Update::Latest(tag), Some(install)) = (&self.app_update, Install::this())
        {
            install.relaunch(tag.trim_start_matches('v'));
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
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
                let status_frame = Frame::new().fill(p.bg).inner_margin(Margin {
                    right: 18 - STATUS_BAR,
                    ..Margin::same(18)
                });
                // The status pane's height with no drawer open.
                let tall = ui.available_height() - status_frame.total_margin().sum().y;
                // Added first, so they span the page and the status pane.
                self.drawers(ui, &name);
                // The page takes up to its form's full width and the status
                // pane the rest, down to STATUS_MIN.
                let widest = (ui.available_width() - PAGE_MIN).max(STATUS_MIN);
                let page_wants = form::full_width(ui) + page.inner_margin.sum().x;
                let status = (ui.available_width() - page_wants).clamp(STATUS_MIN, widest);
                egui::Panel::right("status")
                    .resizable(false)
                    .exact_size(status.min(1100.0))
                    .frame(status_frame)
                    .show(ui, |ui| self.status(ui, &name, tall));
                egui::CentralPanel::default()
                    .frame(page)
                    .show(ui, |ui| self.page(ui, &name));
            }
        }
        self.dialogs(&ctx);
    }

    /// Changes the theme, cross-fading when the window will look different.
    fn choose_theme(&mut self, ctx: &egui::Context, pref: ThemePreference) {
        self.settings.theme = pref;
        let next = match pref {
            ThemePreference::Dark => Theme::Dark,
            ThemePreference::Light => Theme::Light,
            ThemePreference::System => ctx
                .system_theme()
                .unwrap_or_else(|| ctx.options(|o| o.fallback_theme)),
        };
        if next == ctx.theme() {
            ctx.set_theme(pref);
            self.fade.asked = None;
        } else {
            let shot = egui::UserData::new(FadeShot);
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(shot));
            self.fade.asked = Some((pref, 0));
        }
    }

    /// Takes a theme change's screenshot, then fades it out over the new theme.
    /// With no screenshot the theme changes at once.
    fn fade_theme(&mut self, ctx: &egui::Context) {
        if let Some((_, faded)) = &mut self.fade.shown {
            *faded += ctx.input(|i| i.stable_dt).min(FADE_STEP);
        }
        if let Some((pref, waited)) = &mut self.fade.asked {
            let (shot, side) = ctx.input(|i| {
                let shot = i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot {
                        user_data, image, ..
                    } if user_data.data.as_ref().is_some_and(|d| d.is::<FadeShot>()) => {
                        Some(image.clone())
                    }
                    _ => None,
                });
                (shot, i.max_texture_side)
            });
            let got = shot.is_some();
            // A window too big for one texture changes at once.
            if let Some(image) = shot.filter(|s| s.width().max(s.height()) <= side) {
                let old = ctx.load_texture("theme-fade", image, egui::TextureOptions::NEAREST);
                self.fade.shown = Some((old, 0.0));
            }
            if got || *waited >= FADE_WAIT {
                ctx.set_theme(*pref);
                self.fade.asked = None;
            } else {
                *waited += 1;
                ctx.request_repaint();
            }
        }
        let Some((old, faded)) = &self.fade.shown else {
            return;
        };
        let t = faded / FADE_TIME;
        if t >= 1.0 {
            self.fade.shown = None;
            return;
        }
        // The screenshot is in pixels, from the window's top left.
        let rect =
            egui::Rect::from_min_size(pos2(0.0, 0.0), old.size_vec2() / ctx.pixels_per_point());
        let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let tint = Color32::WHITE.gamma_multiply(1.0 - egui::emath::easing::quadratic_out(t));
        ctx.layer_painter(egui::LayerId::new(egui::Order::TOP, Id::new("theme-fade")))
            .image(old.id(), rect, uv, tint);
        ctx.request_repaint();
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.service.poll();
        if self.schema.is_none() {
            match &self.service.schema {
                Load::Ready(schema) => {
                    self.schema = Some(Arc::new(schema.clone()));
                    self.listed = self.schema.clone();
                }
                // No gw runs, so the sidebar lists none of its commands.
                Load::Failed(_) => self.listed = None,
                Load::Waiting(_) => {}
            }
        }
        let mut ended = Vec::new();
        for (disk, job) in [(true, &mut self.disk), (false, &mut self.tool)] {
            let Some(job) = job else { continue };
            let was_running = job.running();
            job.poll();
            self.log.follow(job);
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
            self.log.follow(probe);
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
                self.log.end(probe, ending(probe));
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
        } else if self.live && port != self.probed {
            self.ask_device(ctx);
        }
    }

    /// The Greaseweazle the sidebar shows.
    fn found_port(&mut self) -> Option<&Port> {
        self.service.ports();
        chosen_port(self.service.known_ports(), &self.settings.device)
    }

    /// Whether the sidebar shows a Greaseweazle, as last listed.
    fn connected(&self) -> bool {
        chosen_port(self.service.known_ports(), &self.settings.device).is_some()
    }

    /// Why Detect cannot run on `page` now. On Read it reads the disk in the drive.
    fn cannot_detect(&self, page: &str) -> Option<&'static str> {
        if self.running().is_some() {
            Some("Wait for the job that is running.")
        } else if page != "read" {
            None
        } else if self.probe.is_some() {
            Some("Wait while the device says what it is.")
        } else if !self.connected() {
            Some(NO_DEVICE)
        } else {
            None
        }
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
            Ok(mut job) => {
                self.log.begin(heading(&job), &mut job);
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
        self.log.end(job, ending(job));
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
        let values = self.settings.values.entry(page.clone()).or_default();
        form::choose_format(&schema, cmd, values, &mut self.settings.outputs, best);
        if step > 1 {
            values.set("tracks", form::double_step(values.get("tracks")));
        }
        self.notices.insert(page, found_note(&formats, step));
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
                let version = self.listed.as_ref().map(|s| format!("gw {}", s.version));
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
                for (section, names) in sections(self.listed.as_deref()) {
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
        if cmd.name == "update" {
            form::Firmware::only(&mut values);
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
            self.notice_bar(ui, name);
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
        let cannot_detect = self.cannot_detect(name);
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
                        cannot_detect,
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
            // The page's earlier answer goes while it looks again.
            self.notices.remove(name);
            self.detect_for = Some(name.to_owned());
            let args = self.detect_args(cmd);
            self.run(ui.ctx(), DETECT, args);
        }
    }

    fn notice_bar(&mut self, ui: &mut Ui, page: &str) {
        let Some(notice) = self.notices.get(page).cloned() else {
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
                            self.notices.remove(page);
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
                    let drive = job.command != DETECT || self.detect_for.as_deref() == Some("read");
                    if stop.on_hover_text(stop_tip(&job.command, drive)).clicked() {
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
            // The page's own settings first: they can be made ready with no device.
            let no_device = uses_device(schema, &cmd.name) && !self.connected();
            let outputs = &self.settings.outputs;
            self.diskdefs_fault(values)
                .or_else(|| form::blocked(schema, cmd, values, outputs, &self.service))
                .or(no_device.then_some(NO_DEVICE))
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
                self.log.begin(heading(&job), &mut job);
                let disk = DISK_COMMANDS.contains(&command);
                *(if disk { &mut self.disk } else { &mut self.tool }) = Some(job);
            }
            Err(e) => {
                // A detect job's page is the one it chooses the format on.
                let page = match command {
                    DETECT => self.detect_for.clone(),
                    _ => None,
                };
                let page = page.unwrap_or_else(|| command.to_owned());
                self.notices
                    .insert(page, format!("Could not start gw: {e}"));
            }
        }
    }

    /// The disk's status: what is happening now, or last happened, to a disk.
    /// The job and its map. `tall` is the pane's height with no drawer open:
    /// the map keeps the size that gives it while a drawer leaves it room.
    fn status(&mut self, ui: &mut Ui, page: &str, tall: f32) {
        let full = ui.available_height();
        // The rows keep one width, clear of the strip the scroll bar floats in.
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width() - f32::from(STATUS_BAR));
                self.status_rows(ui, page, tall, full)
            });
    }

    fn status_rows(&mut self, ui: &mut Ui, page: &str, tall: f32, full: f32) {
        let p = theme::palette(ui);
        let blank = self.blank_map(page);
        let top = ui.cursor().top();
        // Below the cursor: the map's share of the pane with no drawer open,
        // so opening one leaves the map be while it fits, and the room there is.
        let room = |ui: &Ui| {
            let used = ui.cursor().top() - top;
            (tall * MAP_SHARE - used, full - used)
        };
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
            let (budget, room) = room(ui);
            diskmap::show(ui, &blank, "blank", budget, room);
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
        if let Some(left) = cancelled(job) {
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(left).color(p.partial)).wrap());
        }
        ui.add_space(8.0);
        let (budget, room) = room(ui);
        match job.progress.cyls.is_empty() && job.progress.tracks.is_empty() {
            true => diskmap::show(ui, &blank, "blank", budget, room),
            false => diskmap::show(ui, &job.progress, job.started, budget, room),
        }
    }

    /// An empty map the size of the page's format, or of a common disk.
    fn blank_map(&mut self, page: &str) -> Progress {
        let empty = Values::default();
        let values = self.settings.values.get(page).unwrap_or(&empty);
        let format = self
            .schema
            .as_deref()
            .and_then(|s| form::effective_format(&mut self.service, s, s.command(page)?, values));
        let info = format.and_then(|format| {
            let diskdefs = form::diskdefs_for(&mut self.service, values, &format);
            self.service.format_info(&diskdefs, &format).ready()
        });
        let (cyls, heads) = info.map_or(form::USUAL_DISK, |i| (i.cyls, i.heads));
        Progress::blank(cyls, heads)
    }

    /// The drawers under the page and the status pane: the command line and
    /// the log. A drawer slides open and shut; going from one to the other does not.
    fn drawers(&mut self, ui: &mut Ui, page: &str) {
        let p = theme::palette(ui);
        let frame = Frame::new().fill(p.bg).inner_margin(Margin {
            left: 28,
            right: 18,
            top: 12,
            bottom: 14,
        });
        let open = self.settings.drawer;
        let switched = self.drawn.is_some() && open.is_some() && self.drawn != open;
        self.drawn = open;
        for (drawer, id) in [(Drawer::Cli, "cli"), (Drawer::Log, "log")] {
            // Runs the slide egui's Panel keys by this id, so it takes DRAWER_TIME,
            // or no time going straight from one drawer to the other.
            let slide = Id::new(id).with("animation");
            let time = if switched { 0.0 } else { DRAWER_TIME };
            ui.ctx()
                .animate_bool_with_time(slide, open == Some(drawer), time);
        }
        let mut cli = open == Some(Drawer::Cli);
        egui::Panel::bottom("cli")
            .frame(frame)
            .resizable(false)
            .exact_size(DRAWER)
            .show_collapsible(ui, &mut cli, |ui| self.cli(ui, page));
        // As tall as the command line at first; drag its edge for more.
        let tallest = (ui.available_height() - LOG_ROOM).max(DRAWER);
        let bottom = ui.max_rect().bottom();
        let mut log = open == Some(Drawer::Log);
        let mut clear = false;
        egui::Panel::bottom("log")
            .frame(frame)
            .resizable(true)
            .drag_to_open(false)
            .default_size(DRAWER)
            .size_range(DRAWER..=tallest)
            .show_collapsible(ui, &mut log, |ui| {
                let log = &self.log;
                let note = log.trimmed().then_some("Older lines were dropped.");
                let lines = log.lines();
                let heads = |i| log.is_head(i);
                clear = output(ui, "Log", note, lines, heads, p, None, true);
            });
        if clear {
            self.log.clear();
        }
        // Dragged below its least height, the log holds there until pulled
        // LOG_BUMP further; a double-click on its edge shuts it at once.
        let (pointer, double) = ui.input(|i| {
            let double = i
                .pointer
                .button_double_clicked(egui::PointerButton::Primary);
            (i.pointer.interact_pos(), double)
        });
        let past = pointer.map_or(f32::INFINITY, |p| p.y - (bottom - DRAWER));
        if open == Some(Drawer::Log) && !log && (double || past > LOG_BUMP) {
            self.settings.drawer = None;
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
            self.load_preset(command, &path);
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
                        self.choose_theme(ui.ctx(), pref);
                    }
                }
            });
        });
        section(ui, "Greaseweazle", |ui| {
            match (&self.engine, &self.service.schema) {
                (Some(engine), Load::Ready(schema)) => {
                    let origin = match engine.origin {
                        Origin::Bundled if engine.update_in(&engine::updates()).is_some() => {
                            "updated from GitHub"
                        }
                        Origin::Bundled => "built in",
                        Origin::Installed => "installed on this computer",
                        Origin::Custom => "chosen here",
                    };
                    ui.label(format!("Greaseweazle {}, {origin}.", schema.version));
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
                if ui
                    .button("Restart")
                    .on_hover_text("Start gw again, and check GitHub for a newer release.")
                    .clicked()
                {
                    self.connect(ui.ctx());
                }
                let Some(engine) = self.engine.as_ref().filter(|e| e.origin == Origin::Bundled)
                else {
                    return;
                };
                let (can, tip) = self.gw_update.button("Greaseweazle Tools");
                let update = ui.add_enabled(can, egui::Button::new("Update"));
                if update
                    .on_hover_text(&tip)
                    .on_disabled_hover_text(&tip)
                    .clicked()
                    && let Update::Newer(tag) = &self.gw_update
                {
                    self.gw_update = Update::gw(engine, tag, repaint(ui.ctx()));
                }
            });
        });
        section(ui, "Paths", |ui| {
            let default = ("Use the default", "Documents/Ferriteweazle/Images.");
            let images = self.images_folder();
            let back = self.settings.images_folder.is_some().then_some(default);
            match path_row(
                ui,
                "Images folder",
                &images,
                "Choose where new images go.",
                back,
            ) {
                Some(PathClick::Choose) => {
                    let chosen = rfd::FileDialog::new().set_directory(&images).pick_folder();
                    if let Some(folder) = chosen {
                        self.set_images_folder(Some(folder));
                    }
                }
                Some(PathClick::Default) => self.set_images_folder(None),
                None => {}
            }
            ui.add_space(8.0);
            let default = ("Use the default", "Documents/Ferriteweazle/Presets.");
            let presets = self.presets_folder();
            let back = self.settings.presets_folder.is_some().then_some(default);
            match path_row(
                ui,
                "Presets folder",
                &presets,
                "Choose the presets folder.",
                back,
            ) {
                Some(PathClick::Choose) => {
                    let chosen = rfd::FileDialog::new().set_directory(&presets).pick_folder();
                    if chosen.is_some() {
                        self.settings.presets_folder = chosen;
                    }
                }
                Some(PathClick::Default) => self.settings.presets_folder = None,
                None => {}
            }
            ui.add_space(8.0);
            let default = (
                "Use the built-in gw",
                "Go back to the gw Ferriteweazle ships.",
            );
            let python = self.engine.as_ref().map(|e| e.python.clone());
            let gw = python
                .or_else(|| self.settings.engine.clone())
                .unwrap_or_default();
            let back = self.settings.engine.is_some().then_some(default);
            let tip = "Choose a gw, or a Python with greaseweazle.";
            match path_row(ui, "Greaseweazle Tools (gw cli)", &gw, tip, back) {
                Some(PathClick::Choose) => {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        self.settings.engine = Some(path);
                        self.connect(ui.ctx());
                    }
                }
                Some(PathClick::Default) => {
                    self.settings.engine = None;
                    self.connect(ui.ctx());
                }
                None => {}
            }
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
            ui.label(concat!(
                "Ferriteweazle ",
                env!("CARGO_PKG_VERSION"),
                " written with \u{2661} by Lee Hobson (@hobbo91), under the MIT license."
            ));
            ui.horizontal(|ui| {
                if let Some(install) = Install::this() {
                    self.app_update_button(ui, install);
                }
                ui.hyperlink_to("Source code and issues", REPO)
                    .on_hover_text(REPO);
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.hyperlink_to("Greaseweazle Tools", GW_REPO)
                    .on_hover_text(GW_REPO);
                ui.label(
                    " is the brains of the operation, all credit goes to Keir Fraser. \
                     This is merely a fancy GUI front end.",
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

    fn app_update_button(&mut self, ui: &mut Ui, install: Install) {
        let (can, tip) = match install.stuck() {
            Some(why) => (false, why.to_owned()),
            None if self.running().is_some() => (false, "Wait for the job that is running.".into()),
            None => self.app_update.button("Ferriteweazle"),
        };
        let update = ui.add_enabled(can, egui::Button::new("Update"));
        if update
            .on_hover_text(&tip)
            .on_disabled_hover_text(&tip)
            .clicked()
            && let (Update::Newer(tag), Some(engine)) = (&self.app_update, &self.engine)
        {
            if cfg!(windows) && matches!(install, Install::Folder(_)) {
                // Windows will not move the data folder while gw runs from it.
                self.service = Service::offline(Err("Updating Ferriteweazle\u{2026}".into()));
            }
            self.app_update = Update::app(engine, install, tag, repaint(ui.ctx()));
        }
    }

    fn presets_folder(&self) -> PathBuf {
        self.settings
            .presets_folder
            .clone()
            .unwrap_or_else(presets::default_folder)
    }

    fn images_folder(&self) -> PathBuf {
        self.settings
            .images_folder
            .clone()
            .unwrap_or_else(form::images_folder)
    }

    /// Changes where new images go, and moves every page still using the old
    /// folder along with it.
    fn set_images_folder(&mut self, folder: Option<PathBuf>) {
        let old = self.images_folder();
        self.settings.images_folder = folder;
        let new = self.images_folder().to_string_lossy().into_owned();
        for (command, dest) in form::OUTPUTS {
            let key = form::output_key(command, dest);
            let out = self.settings.outputs.entry(key).or_default();
            if Path::new(&out.folder) == old {
                out.folder = new.clone();
            }
        }
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
            let text = format!("Could not save the preset: {e}");
            self.notices.insert(command.to_owned(), text);
        }
    }

    /// Applies a preset file's settings and opens its page. A fault shows on `page`.
    fn load_preset(&mut self, page: &str, path: &Path) {
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
                self.notices
                    .insert(page.to_owned(), format!("Could not load {name}. {e}"));
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
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

/// A job's heading in the log: its command line, as the CLI shows it.
/// Detection is the bridge's own, so it goes by its title.
fn heading(job: &Job) -> String {
    match job.command.as_str() {
        DETECT => job
            .args
            .iter()
            .fold(title(DETECT), |out, a| out + " " + &command::quote(a)),
        _ => command::line(&job.args),
    }
}

/// A job's last line in the log: how it ended, and when.
fn ending(job: &Job) -> String {
    let how = match job.outcome() {
        Some(Outcome::Succeeded) => "Done in",
        Some(Outcome::Failed) => "Failed after",
        _ => "Cancelled after",
    };
    format!("{how} {}.", clock(job.elapsed()))
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

/// Whether a command acts on the Greaseweazle: gw gives each such one --device.
fn uses_device(schema: &Schema, command: &str) -> bool {
    schema
        .command(command)
        .is_some_and(|c| c.arg("device").is_some())
}

/// The sidebar's sections and the commands this gw has in each: none
/// until a gw has described itself.
fn sections(schema: Option<&Schema>) -> Vec<(&'static str, Vec<&str>)> {
    let names: Vec<&str> = schema
        .iter()
        .flat_map(|s| &s.commands)
        .map(|c| c.name.as_str())
        .collect();
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
    out.retain(|(_, names)| !names.is_empty());
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
        Some(Outcome::Stopped) => ("Cancelled", p.partial),
    }
}

/// The Stop button's tip. Stopping a job that runs the drive turns its motor off.
fn stop_tip(command: &str, drive: bool) -> &'static str {
    match command {
        "read" => "Stop read, this will also stop the drive's motor.",
        "write" => "Stop write, this will also stop the drive's motor.",
        "erase" => "Stop erase, this will also stop the drive's motor.",
        "clean" | "seek" | "rpm" | "align" => "Stop, this will also stop the drive's motor.",
        DETECT if drive => "Stop detect, this will also stop the drive's motor.",
        _ => "Stop gw.",
    }
}

/// What a disk job cancelled part way leaves behind: gw keeps the tracks a
/// read has done, and deletes a conversion's image.
fn cancelled(job: &Job) -> Option<&'static str> {
    if job.outcome() != Some(Outcome::Stopped) {
        return None;
    }
    match job.command.as_str() {
        "read" => Some("Cancelled: incomplete image."),
        "write" => Some("Cancelled: disk partly written."),
        "erase" => Some("Cancelled: disk partly erased."),
        "convert" => Some("Cancelled: no image made."),
        _ => None,
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
        output(
            ui,
            "Output",
            None,
            &job.log,
            |_| false,
            p,
            Some(260.0),
            false,
        );
    }
}

/// gw's output under `heading`, with Copy and Save, and Clear if `clearable`,
/// `height` tall or, with none, as tall as the room left. `head` picks the
/// lines that head a job. True when Clear was pressed.
#[allow(clippy::too_many_arguments)]
fn output(
    ui: &mut Ui,
    heading: &str,
    note: Option<&str>,
    log: &[String],
    head: impl Fn(usize) -> bool,
    p: &Palette,
    height: Option<f32>,
    clearable: bool,
) -> bool {
    let mut clear = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(heading).strong());
        if let Some(note) = note {
            ui.label(RichText::new(note).small().weak());
        }
        right(ui, |ui| {
            let save = ui.add_enabled(!log.is_empty(), egui::Button::new("Save…"));
            if save.on_hover_text("Save gw's output to a file.").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_directory(crate::app_folder())
                    .set_file_name("gw.log")
                    .save_file()
            {
                let _ = std::fs::write(&path, log.join("\n") + "\n");
            }
            let copy = ui.add_enabled(!log.is_empty(), egui::Button::new("Copy"));
            if copy.on_hover_text("Copy gw's output.").clicked() {
                ui.ctx().copy_text(log.join("\n"));
            }
            if clearable {
                let button = ui.add_enabled(!log.is_empty(), egui::Button::new("Clear"));
                clear = button.on_hover_text("Clear the log.").clicked();
            }
        });
    });
    let frame = Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(8)
        .inner_margin(8);
    // Exactly the room left: a drawer a little taller than its contents
    // would shrink to them, frame by frame.
    let room = || ui.available_height() - frame.total_margin().sum().y;
    let fill = height.is_none();
    let height = height.unwrap_or_else(room).max(LOG_LINE);
    frame.show(ui, |ui| {
        if log.is_empty() {
            let least = if fill { height } else { height.min(80.0) };
            ui.set_min_size(vec2(ui.available_width(), least));
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
                for i in rows {
                    let text = match head(i) {
                        true => RichText::new(&log[i]).monospace().color(p.accent),
                        false => log_line(&log[i], p),
                    };
                    ui.add(egui::Label::new(text).extend().selectable(true));
                }
            });
    });
    clear
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

/// "Found akai.800. Disk also matches eagle.dsqd.800 and zx.quorum.ds80."
fn found_note(formats: &[String], step: u32) -> String {
    let mut note = format!("Found {}.", formats[0]);
    if step > 1 {
        note += " It is a 40-track disk in an 80-track drive, so Double step is on.";
    }
    match &formats[1..] {
        [] => {}
        [one] => note += &format!(" Disk also matches {one}."),
        more => {
            let (last, rest) = more[..more.len().min(4)]
                .split_last()
                .expect("more has some");
            note += &format!(" Disk also matches {} and {last}.", rest.join(", "));
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

enum PathClick {
    Choose,
    Default,
}

/// A path in Settings: its name, where it is, and Choose… with, where `back`
/// is given, its button and tip for going back to the default.
fn path_row(
    ui: &mut Ui,
    name: &str,
    path: &Path,
    tip: &str,
    back: Option<(&str, &str)>,
) -> Option<PathClick> {
    ui.label(name);
    let shown = match path.as_os_str().is_empty() {
        true => RichText::new("Not found.").weak(),
        false => RichText::new(path.to_string_lossy())
            .monospace()
            .small()
            .weak(),
    };
    ui.label(shown);
    ui.horizontal(|ui| {
        if ui.button("Choose…").on_hover_text(tip).clicked() {
            return Some(PathClick::Choose);
        }
        let (text, tip) = back?;
        ui.button(text)
            .on_hover_text(tip)
            .clicked()
            .then_some(PathClick::Default)
    })
    .inner
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
    let enabled = ui.is_enabled();
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, selected, text)
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

    fn offline() -> App {
        let schema = serde_json::from_str(include_str!("../tests/data/schema-1.23.json")).unwrap();
        App::offline(&egui::Context::default(), Settings::default(), Ok(schema))
    }

    #[test]
    fn detect_on_the_read_page_waits_while_the_device_card_asks_gw() {
        let mut app = offline();
        app.pin_ports(vec![Port {
            device: "/dev/cu.usbmodem14201".into(),
            name: Some("Greaseweazle".into()),
            serial: None,
            score: 20,
        }]);
        assert_eq!(app.cannot_detect("read"), None);
        let mut probe = Job::replay("info", "");
        probe.ended = None;
        app.probe = Some(probe);
        let wait = Some("Wait while the device says what it is.");
        assert_eq!(app.cannot_detect("read"), wait);
        assert_eq!(app.cannot_detect("convert"), None, "it reads a file");
    }

    #[test]
    fn a_detected_format_names_the_others_the_disk_also_matches() {
        let formats = ["akai.800", "eagle.dsqd.800", "epson.qx10.400"].map(String::from);
        assert_eq!(found_note(&formats[..1], 1), "Found akai.800.");
        assert_eq!(
            found_note(&formats[..2], 1),
            "Found akai.800. Disk also matches eagle.dsqd.800."
        );
        assert_eq!(
            found_note(&formats, 1),
            "Found akai.800. Disk also matches eagle.dsqd.800 and epson.qx10.400."
        );
    }

    #[test]
    fn images_and_presets_go_in_the_apps_own_folder_by_default() {
        let folder = crate::app_folder();
        assert!(folder.ends_with("Documents/Ferriteweazle"), "{folder:?}");
        assert_eq!(Path::new(&Output::default().folder), folder.join("Images"));
        assert_eq!(presets::default_folder(), folder.join("Presets"));
    }

    #[test]
    fn pages_on_the_default_images_folder_follow_a_new_one() {
        let mut app = offline();
        let mine = Output {
            folder: "/mine".into(),
            ..Output::default()
        };
        let convert = form::output_key("convert", "out_file");
        app.settings.outputs.insert(convert.clone(), mine);
        app.set_images_folder(Some("/new".into()));
        let read = form::output_key("read", "file");
        assert_eq!(app.settings.outputs[&read].folder, "/new");
        assert_eq!(app.settings.outputs[&convert].folder, "/mine");
        app.set_images_folder(None);
        let default = Path::new(&app.settings.outputs[&read].folder);
        assert_eq!(default, form::images_folder());
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
        let sep = std::path::MAIN_SEPARATOR;
        let expected = (1..=3).map(|d| format!("/f{sep}Game_Disk{d}.adf"));
        assert_eq!(last, expected.collect::<Vec<_>>());
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
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(out.value(1), format!("/disks{sep}Game Disk.hfe::version=3"));
    }

    #[test]
    fn a_fault_shows_on_the_page_it_happened_on() {
        let ctx = egui::Context::default();
        let settings = Settings {
            presets_folder: Some("/dev/null/presets".into()),
            ..Settings::default()
        };
        let mut app = App::offline(&ctx, settings, Err(String::new()));
        app.engine = Some(Engine {
            python: "/no/such/python".into(),
            origin: Origin::Custom,
        });
        app.run(&ctx, "erase", Vec::new());
        app.detect_for = Some("convert".into());
        app.run(&ctx, DETECT, Vec::new());
        app.save_preset("seek", "Mine");
        app.load_preset("write", Path::new("/no/such/Mine.json"));
        let pages: Vec<&str> = app.notices.keys().map(String::as_str).collect();
        assert_eq!(pages, ["convert", "erase", "seek", "write"]);
        assert!(app.notices["erase"].starts_with("Could not start gw: "));
    }

    #[test]
    fn a_gw_that_cannot_be_found_leaves_no_greaseweazle_or_command() {
        let ctx = egui::Context::default();
        let schema = serde_json::from_str(include_str!("../tests/data/schema-1.23.json")).unwrap();
        let mut app = App::offline(&ctx, Settings::default(), Ok(schema));
        app.pin_ports(vec![Port {
            device: "/dev/cu.usbmodem14201".into(),
            name: Some("Greaseweazle".into()),
            serial: None,
            score: 20,
        }]);
        assert!(app.connected());
        assert!(!sections(app.listed.as_deref()).is_empty());
        app.settings.engine = Some("/no/such/gw".into());
        app.connect(&ctx);
        app.poll(&ctx);
        assert!(app.engine.is_none());
        assert!(!app.connected(), "no gw will look for it");
        assert!(sections(app.listed.as_deref()).is_empty(), "no gw runs");
    }
}

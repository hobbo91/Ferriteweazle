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
use crate::udev;
use crate::update::{self, Install, Update};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Layout, Margin, RichText, Sense,
    Stroke, TextEdit, TextStyle, Theme, ThemePreference, Ui, UserAttentionType, Vec2,
    ViewportCommand, pos2, vec2,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
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

/// Page descriptions where gw's does not fit: gw gives pin get pin set's.
const ABOUTS: &[(&str, &str)] = &[("pin get", "Read the level of a floppy interface pin.")];

/// Commands that ask first, and what they do to the disk.
const DESTRUCTIVE: &[(&str, &str)] = &[
    ("write", "The tracks written lose what they hold."),
    ("erase", "The tracks erased lose what they hold."),
];

/// Why a command that uses the device cannot run.
const NO_DEVICE: &str = "Connect a Greaseweazle.";
/// Why nothing new can start while a job runs.
const BUSY: &str = "Wait for the job that is running.";
/// Why nothing new can start while gw or this app installs an update.
const INSTALLING: &str = "Wait for the update to install.";
/// Why a command that uses the device waits while the card runs gw info.
const ASKING: &str = "Wait for gw info to finish.";
/// Why a page cannot run while its command line shows what gw cannot take.
const CLI_FAULT: &str = "Fix the command line or Reset it.";
/// Settings' gw line while a Windows folder copy replaces itself.
const UPDATING: &str = "Updating Ferriteweazle\u{2026}";

/// Commands the status pane shows. Others show their results under their page.
const DISK_COMMANDS: &[&str] = &["read", "write", "convert", "erase", "align", DETECT];

const REPO: &str = "https://github.com/hobbo91/ferriteweazle";
const GW_REPO: &str = "https://github.com/keirf/greaseweazle";
/// gw's guide to setting up a Greaseweazle, its drives and its cables.
const GW_GUIDE: &str = "https://github.com/keirf/greaseweazle/wiki/Getting-Started";

/// The page's minimum width: room for a label beside its field.
const PAGE_MIN: f32 = 420.0;
const STATUS_MIN: f32 = 320.0;
/// The sidebar logo's side.
const LOGO_SIZE: f32 = 40.0;
/// How far the logo reaches above the sidebar's margin.
const LOGO_TUCK: f32 = 6.0;
/// The clear strip at the logo's left, as a share of its side: the drawing
/// lines up with the device card's edge.
const LOGO_CLEAR: f32 = 16.0 / 256.0;
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
    /// Saves the gw command and its output where a job puts its image, as
    /// `name.ext.log`.
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
    /// A disk job, or the first of a session of `disks`.
    Confirm {
        command: String,
        args: Vec<String>,
        disks: usize,
    },
    /// Files a job would replace, and the job's runs.
    Overwrite {
        files: Vec<PathBuf>,
        command: String,
        runs: Runs,
    },
    /// Between the disks of a session. Disks count from 1.
    NextDisk {
        command: String,
        /// The disk to insert next: none when the one that failed was the last.
        disk: Option<usize>,
        total: usize,
        /// The disk that failed, which can be tried again.
        failed: Option<usize>,
        /// The image the next disk gets, when writing.
        image: Option<String>,
    },
    SavePreset {
        command: String,
        name: String,
    },
    DeletePreset {
        /// The page that shows why the file could not be deleted.
        command: String,
        name: String,
        path: PathBuf,
    },
    /// How to give this account the port Linux refused it.
    Access {
        port: String,
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

/// Installing gw's udev rule through pkexec, and how it went.
#[derive(Default)]
enum RuleInstall {
    #[default]
    Idle,
    Running(Receiver<Result<(), String>>),
    Done(Result<(), String>),
}

/// What a dialog's button does, once the dialog has let go of the app.
type Action = Box<dyn FnOnce(&mut App)>;

/// Runs of one command one after another: disks read or written, which wait
/// for the next disk, or images converted.
struct Session {
    command: String,
    runs: Runs,
    next: usize,
    /// The runs that failed, from 0.
    failed: Vec<usize>,
}

/// A page's gw runs, one per disk or image: the arguments, each run's
/// image for a batch, and the files they make.
#[derive(Clone, Default)]
struct Runs {
    args: Vec<Vec<String>>,
    images: Vec<String>,
    makes: Vec<PathBuf>,
}

/// A page's Presets menu while it is open, so the folder is read once, not
/// every frame.
struct PresetsMenu {
    page: String,
    /// The page's presets, by name.
    saved: Vec<(String, PathBuf)>,
    /// Whether the page differs from gw's defaults.
    changed: bool,
}

/// The command line drawer's text, and why it does not parse.
#[derive(Default)]
struct Cli {
    text: String,
    error: Option<String>,
    /// The page the text was typed on.
    page: String,
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
    /// The port the card last asked about, and whether Linux denied it then.
    probed: Option<(String, bool)>,
    /// The real app, not a test window: it keeps the drive in drive_file(),
    /// runs gw info on each Greaseweazle that appears, and asks GitHub for
    /// newer releases of gw and of this app.
    live: bool,
    /// The drive as last kept in drive_file().
    kept_drive: String,
    gw_update: Update,
    app_update: Update,
    /// The Presets menu, while it is open.
    presets: Option<PresetsMenu>,
    /// gw's bridge is stopped while a Windows folder copy replaces its data
    /// folder: the ports it had listed.
    gw_paused: Option<Vec<Port>>,
    logo: Option<egui::TextureHandle>,
    fade: Fade,
    /// The desktop's light or dark preference, where winit reports none.
    desktop_theme: Option<Receiver<Theme>>,
    /// The theme last given the window's frame.
    framed: Option<Theme>,
    /// The drawer open when the drawers were last drawn.
    drawn: Option<Drawer>,
    /// gw's udev rule, where a Linux package ships it.
    pub udev_rule: Option<PathBuf>,
    install: RuleInstall,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let drive = kept_drive(&drive_file());
        let settings = Settings {
            drive: drive.clone(),
            ..Settings::default()
        };
        let mut app = App::with_settings(&cc.egui_ctx, settings);
        app.live = true;
        app.kept_drive = drive;
        update::tidy();
        app.look_for_updates(&cc.egui_ctx);
        #[cfg(target_os = "linux")]
        {
            app.desktop_theme = Some(crate::portal::watch(repaint(&cc.egui_ctx)));
        }
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
            kept_drive: String::new(),
            gw_update: Update::default(),
            app_update: Update::default(),
            presets: None,
            gw_paused: None,
            logo: None,
            fade: Fade::default(),
            desktop_theme: None,
            framed: None,
            drawn: None,
            udev_rule: engine::udev_rule(),
            install: RuleInstall::Idle,
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
                "Ferriteweazle could not find gw. Choose one in Settings.".into(),
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
        // An install under way keeps its answer.
        if engine.origin == Origin::Bundled && !installing(&self.gw_update) {
            self.gw_update = Update::check(engine, None, repaint(ctx));
        }
        if Install::this().is_some() && !installing(&self.app_update) {
            self.app_update = Update::check(engine, Some(update::APP_REPO), repaint(ctx));
        }
    }

    /// Takes the updates' answers. A new gw restarts gw, and a new
    /// Ferriteweazle opens in place of this window.
    fn poll_updates(&mut self, ctx: &egui::Context) {
        if self
            .gw_update
            .poll(self.schema.as_deref().map(|s| s.version.as_str()))
        {
            self.connect(ctx);
        }
        if self.app_update.poll(Some(env!("CARGO_PKG_VERSION")))
            && let (Update::Latest(tag), Some(install)) = (&self.app_update, Install::this())
        {
            install.relaunch(tag.trim_start_matches('v'));
            ctx.send_viewport_cmd(ViewportCommand::Close);
        } else if matches!(self.app_update, Update::Failed(_))
            && let Some(engine) = &self.engine
            && let Some(ports) = self.gw_paused.take()
        {
            // The update stopped gw. Not connect(): its check for updates
            // would drop the reason the update failed.
            self.service = Service::start(engine, repaint(ctx));
            self.service.seed_ports(ports);
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
        if self.live && self.settings.drive != self.kept_drive {
            self.kept_drive.clone_from(&self.settings.drive);
            keep_drive(&drive_file(), &self.kept_drive);
        }
        self.follow_desktop(&ctx);
        self.fade_theme(&ctx);
        self.poll(&ctx);
        self.poll_updates(&ctx);
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

    /// Takes the desktop's preference as the theme System falls back on where
    /// winit reports none. On Linux the window's frame follows the theme shown:
    /// egui's own sync gives it winit's default for System, which is light.
    fn follow_desktop(&mut self, ctx: &egui::Context) {
        if let Some(theme) = self
            .desktop_theme
            .as_ref()
            .and_then(|d| d.try_iter().last())
        {
            ctx.options_mut(|o| o.fallback_theme = theme);
        }
        if cfg!(target_os = "linux") && self.framed != Some(ctx.theme()) {
            self.framed = Some(ctx.theme());
            ctx.options_mut(|o| o.sync_window_theme = false);
            let frame = match ctx.theme() {
                Theme::Dark => egui::SystemTheme::Dark,
                Theme::Light => egui::SystemTheme::Light,
            };
            ctx.send_viewport_cmd(ViewportCommand::SetTheme(frame));
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
        if let RuleInstall::Running(answer) = &self.install
            && let Ok(done) = answer.try_recv()
        {
            // The port list says so once udev has granted access.
            self.service.refresh_ports();
            self.install = RuleInstall::Done(done);
        }
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
                let port = chosen_port(self.service.known_ports(), &self.settings.device);
                if let Some(port) = refused_port(probe, port) {
                    probe
                        .log
                        .extend(udev::advice(&port, self.udev_rule.as_deref()));
                }
                // Only INFO_TIMEOUT stops the card's gw info.
                let end = match probe.outcome() {
                    Some(Outcome::Stopped) => {
                        format!("Timed out after {}.", clock(probe.elapsed()))
                    }
                    _ => ending(probe),
                };
                self.log.end(probe, end);
                self.probe_failed = match (device::parse(&probe.log), probe.outcome()) {
                    (Some(_), _) => None,
                    (None, Some(Outcome::Stopped)) => Some("No answer.".into()),
                    (None, _) => Some(
                        probe
                            .progress
                            .error
                            .clone()
                            .unwrap_or_else(|| "Unable to retrieve firmware.".into()),
                    ),
                };
                self.probe = None;
            }
        }
        let port = self.found_port().map(|p| (p.device.clone(), p.denied));
        if port.is_none() {
            self.probed = None;
            self.device = None;
            self.probe_failed = None;
        } else if self.live && !self.quitting && port != self.probed {
            self.ask_device(ctx);
        }
    }

    /// The Greaseweazle the sidebar shows.
    fn found_port(&mut self) -> Option<&Port> {
        self.service.ports();
        chosen_port(self.service.known_ports(), &self.settings.device)
    }

    /// Whether the chosen port is there, open to this account, and did not
    /// fail its last gw info: the device card's dot is green.
    fn answering(&self) -> bool {
        let port = chosen_port(self.service.known_ports(), &self.settings.device);
        port.is_some_and(|p| !p.denied) && self.probe_failed.is_none()
    }

    /// Whether the sidebar shows a Greaseweazle, as last listed.
    fn connected(&self) -> bool {
        chosen_port(self.service.known_ports(), &self.settings.device).is_some()
    }

    /// Why nothing new can start now: a job runs, or gw or this app installs
    /// an update.
    fn busy(&self) -> Option<&'static str> {
        if self.running().is_some() {
            Some(BUSY)
        } else if installing(&self.gw_update) || installing(&self.app_update) {
            Some(INSTALLING)
        } else {
            None
        }
    }

    /// Why Detect cannot run on `page` now. On Read it reads the disk in the drive.
    fn cannot_detect(&self, page: &str) -> Option<&'static str> {
        self.busy().or(match page {
            "read" if self.probe.is_some() => Some(ASKING),
            "read" if !self.connected() => Some(NO_DEVICE),
            _ => None,
        })
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
        self.probed = self.found_port().map(|p| (p.device.clone(), p.denied));
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
        let port = chosen_port(self.service.known_ports(), &self.settings.device).cloned();
        let slot = if disk { &mut self.disk } else { &mut self.tool };
        let Some(job) = slot.as_mut() else { return };
        let outcome = job.outcome();
        // gw deletes the image of a read or conversion that fails.
        job.no_image =
            outcome == Some(Outcome::Failed) && job.output.as_ref().is_some_and(|p| !p.exists());
        if self.settings.save_logs
            && let Some(image) = &job.output
        {
            let mut log = image.clone().into_os_string();
            log.push(".log");
            // As the Log has it, from the command line to how it ended.
            let lines: Vec<String> = std::iter::once(heading(job))
                .chain(job.log.iter().cloned())
                .chain(std::iter::once(ending(job)))
                .collect();
            if let Some(why) = save_log(Path::new(&log), &lines) {
                job.log.push(why);
            }
        }
        if let Some(port) = refused_port(job, port.as_ref()) {
            job.log
                .extend(udev::advice(&port, self.udev_rule.as_deref()));
        }
        self.log.end(job, ending(job));
        let command = job.command.clone();
        let detected = std::mem::take(&mut job.detected);
        let step = job.step;
        match command.as_str() {
            // With --bootloader, gw reports the bootloader's firmware.
            "info" if !job.args.iter().any(|a| a == "--bootloader") => {
                self.device = device::parse(&job.log);
            }
            // New firmware changes what the device says about itself.
            "update" => self.probed = None,
            _ => {}
        }
        if let Some(session) = self.session.as_mut().filter(|s| s.command == command) {
            let failed = outcome == Some(Outcome::Failed);
            if failed {
                session.failed.push(session.next - 1);
            }
            let total = session.runs.args.len();
            let next = (session.next < total).then_some(session.next + 1);
            // A batch of images goes on alone; a disk can be tried again.
            let again = (failed && command != "convert").then_some(session.next);
            // Quit is asking: the window closes once this run has ended.
            if matches!(self.dialog, Some(Dialog::Quit)) || (next.is_none() && again.is_none()) {
                self.end_session();
            } else if command == "convert" {
                self.next_disk(ctx);
            } else {
                self.dialog = Some(Dialog::NextDisk {
                    disk: next,
                    total,
                    failed: again,
                    image: session.runs.images.get(session.next).cloned(),
                    command: command.clone(),
                });
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
            match outcome {
                Some(Outcome::Stopped) => self.detect_for = None,
                _ => self.found(detected, step),
            }
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

    /// Whether `job` runs the drive's motor, which gw turns off as it stops.
    /// gw seek runs it only with --motor-on.
    fn runs_motor(&self, job: &Job) -> bool {
        match job.command.as_str() {
            "read" | "write" | "erase" | "clean" | "rpm" | "align" => true,
            DETECT => self.detect_for.as_deref() == Some("read"),
            _ => false,
        }
    }

    /// Closing the window while gw works asks first.
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
        // A dialog's action holds the page's values from when it opened.
        if self.dialog.is_some() {
            return;
        }
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
        let answering = self.answering();
        let mut ask = false;
        let mut access = None;
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
                    let colour = if answering { p.good } else { p.bad };
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
                            .on_hover_text("List the ports again and run gw info.")
                            .on_disabled_hover_text(match asking {
                                true => "Running gw info…",
                                false => BUSY,
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
                    let denied = found.as_ref().filter(|p| p.denied);
                    if info.is_none() && asking {
                        text_row(ui, |ui| {
                            ui.add(egui::Spinner::new().size(10.0));
                            ui.label(RichText::new("Running gw info…").small().weak());
                        });
                    } else if let Some(port) = denied.filter(|_| info.is_none()) {
                        // gw info says only that it found none; the port list says why.
                        let text = format!("No access to {}.", short_port(&port.device));
                        ui.label(RichText::new(text).small().color(p.bad));
                        let link = egui::Link::new(RichText::new("Grant access…").small());
                        if ui
                            .add(link)
                            .on_hover_text("How to give this account access.")
                            .clicked()
                        {
                            access = Some(port.device.clone());
                        }
                    } else if info.is_none() {
                        // On the line the firmware takes once the device answers.
                        if let Some(why) = &self.probe_failed {
                            ui.label(RichText::new(why).small().color(p.bad));
                        }
                        let link = egui::Link::new(RichText::new("Get info").small());
                        ask |= ui
                            .add_enabled(idle, link)
                            .on_hover_text("Run gw info.")
                            .on_disabled_hover_text(BUSY)
                            .clicked();
                    }
                }
                ui.add_space(4.0);
                let shown = match &found {
                    Some(port) => RichText::new(short_port(&port.device)),
                    None => RichText::new("Choose port").color(p.dim),
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
        if let Some(port) = access {
            self.dialog = Some(Dialog::Access { port });
        }
    }

    /// Drive letters and bus units, read from gw's own help for DRIVE.
    fn drives(&self) -> Vec<(String, String)> {
        let note = self
            .schema
            .as_ref()
            .and_then(|s| s.note("DRIVE"))
            .unwrap_or("0 | 1 | 2 | 3 :: Shugart bus unit\nA | B :: IBM/PC bus unit");
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
        // The card's port, so gw opens the Greaseweazle the card names.
        let device = chosen_port(self.service.known_ports(), &self.settings.device)
            .map_or("", |p| p.device.as_str());
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
        let width = form::form_width(ui);
        egui::Panel::bottom("run-bar")
            .frame(Frame::new().inner_margin(Margin {
                left: 0,
                right: 0,
                top: 12,
                bottom: 4,
            }))
            .show_separator_line(false)
            .show(ui, |ui| self.run_bar(ui, &schema, cmd));
        let cannot_detect = self.cannot_detect(name);
        let mut install = false;
        let mut unsaved = None;
        // Everything above the run bar scrolls, in no more than the room left,
        // so a tall drawer never pushes the page over the bar.
        let action = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .min_scrolled_height(0.0)
            .show(ui, |ui| {
                // A column of its own: the scroll area fills the page, the form does not.
                let column = Layout::top_down(Align::Min);
                ui.allocate_ui_with_layout(vec2(width, 0.0), column, |ui| {
                    ui.set_max_width(width);
                    ui.horizontal(|ui| {
                        ui.heading(title(name));
                        right(ui, |ui| self.presets_menu(ui, name));
                    });
                    let about = match ABOUTS.iter().find(|(c, _)| *c == name) {
                        Some((_, about)) => (*about).to_owned(),
                        None => form::sentence(&cmd.about),
                    };
                    ui.label(RichText::new(about).weak());
                    self.notice_bar(ui, name);
                    ui.add_space(14.0);
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
                        (install, unsaved) = result(ui, job, self.refused(job));
                    }
                    ui.add_space(12.0);
                    action
                })
                .inner
            })
            .inner;
        if install {
            self.install_rule(ui.ctx());
        }
        if let Some(why) = unsaved {
            self.notices.insert(name.to_owned(), why);
        }
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
                ui.label(RichText::new("Starting gw…").weak());
            }
            Some(e) => {
                ui.label(RichText::new("gw is not ready").size(18.0).strong());
                ui.add_space(4.0);
                ui.label(RichText::new(e).weak());
                ui.add_space(10.0);
                if ui.button("Open Settings").clicked() {
                    self.settings.page = Page::Settings;
                }
            }
        });
    }

    fn run_bar(&mut self, ui: &mut Ui, schema: &Schema, cmd: &Command) {
        let p = theme::palette(ui);
        let why = self.why_not(schema, cmd);
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
                    let tip = match self.runs_motor(job) {
                        true => "Stop gw and the drive's motor.",
                        false => "Stop gw.",
                    };
                    let warning = flash_warning(job);
                    let stop = stop.on_hover_ui(|ui| {
                        ui.label(tip);
                        if let Some(warning) = warning {
                            ui.label(warning);
                        }
                    });
                    if stop.clicked() {
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
                    let batch = self
                        .settings
                        .values
                        .get(&cmd.name)
                        .is_some_and(|v| form::batch_input(cmd, v).is_some());
                    let label = match (several, batch, cmd.name.as_str()) {
                        (true, _, _) => "Read disks",
                        (_, true, "write") => "Write disks",
                        (_, true, _) => "Convert images",
                        _ => run_label(&cmd.name),
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
    fn why_not(&self, schema: &Schema, cmd: &Command) -> Option<String> {
        let empty = Values::default();
        let values = self.settings.values.get(&cmd.name).unwrap_or(&empty);
        let output = |dest: &str| form::OUTPUTS.contains(&(cmd.name.as_str(), dest));
        let missing: Vec<String> = command::missing(cmd, values)
            .filter_map(|d| cmd.arg(d))
            .filter(|a| !form::GLOBAL.contains(&a.dest.as_str()) && !output(&a.dest))
            .filter(|a| form::batch_input(cmd, values) != Some(a.dest.as_str()))
            .map(|a| form::label(a).to_lowercase())
            .collect();
        let device = uses_device(schema, &cmd.name);
        if self.engine.is_none() {
            Some("gw is not set up. See Settings.".to_owned())
        } else if let Some(why) = self.busy() {
            Some(why.to_owned())
        } else if self.probe.is_some() && device {
            Some(ASKING.to_owned())
        } else if self.settings.drawer == Some(Drawer::Cli)
            && self.cli.page == cmd.name
            && self.cli.error.is_some()
        {
            // The page holds the last line that parsed, not the one shown.
            Some(CLI_FAULT.to_owned())
        } else if !missing.is_empty() {
            Some(format!("Choose the {} first.", missing.join(" and ")))
        } else {
            // The page's own settings first: they can be made ready with no device.
            let no_device = device && !self.connected();
            let outputs = &self.settings.outputs;
            self.diskdefs_fault(values)
                .or_else(|| form::blocked(schema, cmd, values, outputs, &self.service))
                .or(no_device.then_some(NO_DEVICE))
                .map(str::to_owned)
        }
    }

    /// Why the page's disk definitions file stops it, whatever the format.
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
        self.session = None;
        for job in [&mut self.disk, &mut self.tool].into_iter().flatten() {
            job.stop();
        }
    }

    /// Runs the page, naming first any files it would replace.
    fn start(&mut self, ctx: &egui::Context, cmd: &Command) {
        let values = self.values_for(cmd);
        let images = match (form::batch_input(cmd, &values), self.schema.as_deref()) {
            (Some(_), Some(schema)) => {
                let files = self.service.known_folder(values.get(form::BATCH_FOLDER));
                form::batch_images(schema, files, values.get(form::BATCH_TYPE))
            }
            _ => Vec::new(),
        };
        let outputs = &self.settings.outputs;
        let runs = runs(cmd, values, outputs, &images, |v| self.argv(cmd, v));
        let files: Vec<PathBuf> = runs.makes.iter().filter(|f| f.exists()).cloned().collect();
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

    fn begin(&mut self, ctx: &egui::Context, command: &str, mut runs: Runs) {
        let disks = runs.args.len();
        if disks > 1 {
            let first = runs.args[0].clone();
            self.session = Some(Session {
                command: command.to_owned(),
                runs,
                next: 0,
                failed: Vec::new(),
            });
            match destructive(command) {
                true => {
                    self.dialog = Some(Dialog::Confirm {
                        command: command.to_owned(),
                        args: first,
                        disks,
                    });
                }
                false => self.next_disk(ctx),
            }
        } else if let Some(args) = runs.args.pop() {
            self.confirm_or_run(ctx, command, args);
        }
    }

    /// Starts the session's next run.
    fn next_disk(&mut self, ctx: &egui::Context) {
        let Some(session) = &mut self.session else {
            return;
        };
        let Some(args) = session.runs.args.get(session.next).cloned() else {
            self.session = None;
            return;
        };
        session.next += 1;
        let part = (session.next, session.runs.args.len());
        let command = session.command.clone();
        if !self.run(ctx, &command, args) {
            self.session = None;
        } else if let Some(job) = &mut self.disk {
            job.part = Some(part);
        }
    }

    /// Runs the session's disk that failed again.
    fn disk_again(&mut self, ctx: &egui::Context) {
        if let Some(session) = &mut self.session {
            session.next -= 1;
            session.failed.pop();
        }
        self.next_disk(ctx);
    }

    /// Ends the session, saying on its page how it went.
    fn end_session(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        let total = session.runs.args.len();
        let (verb, noun) = match session.command.as_str() {
            "convert" => ("Converted", "images"),
            "write" => ("Wrote", "disks"),
            _ => ("Read", "disks"),
        };
        let done = session.next - session.failed.len();
        let mut note = format!("{verb} {done} of {total} {noun}.");
        if !session.failed.is_empty() {
            let name = |&i: &usize| {
                session
                    .runs
                    .images
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("disk {}", i + 1))
            };
            let mut names: Vec<String> = session.failed.iter().take(4).map(name).collect();
            if session.failed.len() > 4 {
                names.push(format!("and {} more", session.failed.len() - 4));
            }
            note += &format!(" Failed: {}. The Log says why.", names.join(", "));
        }
        self.notices.insert(session.command, note);
    }

    fn confirm_or_run(&mut self, ctx: &egui::Context, command: &str, args: Vec<String>) {
        if destructive(command) || flashes_bootloader(command, &args) {
            self.dialog = Some(Dialog::Confirm {
                command: command.to_owned(),
                args,
                disks: 1,
            });
        } else {
            self.run(ctx, command, args);
        }
    }

    /// Starts gw; false if it did not start.
    fn run(&mut self, ctx: &egui::Context, command: &str, args: Vec<String>) -> bool {
        let Some(engine) = &self.engine else {
            return false;
        };
        // The image the job writes: gw's last argument, one per disk or image.
        let output = args
            .last()
            .filter(|_| form::OUTPUTS.iter().any(|(c, _)| *c == command))
            .map(|a| PathBuf::from(a.split("::").next().unwrap_or(a)));
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
                true
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
                false
            }
        }
    }

    /// The disk job, running or last run, and its map, or with no job the
    /// page's idle status. `tall` is the pane's height with no drawer open.
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
        let mut install = false;
        if let Some(e) = &job.progress.error {
            ui.add_space(6.0);
            let refused = self.refused(job);
            error_box(ui, p.bad, |ui| match &refused {
                Some(refused) => install = access(ui, refused),
                None => {
                    ui.label(RichText::new(e).color(p.bad));
                }
            });
        }
        let warnings = job.progress.warnings.iter().map(String::as_str);
        for note in warnings.chain(left_behind(job)) {
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(note).color(p.partial)).wrap());
        }
        ui.add_space(8.0);
        let (budget, room) = room(ui);
        match job.progress.cyls.is_empty() && job.progress.tracks.is_empty() {
            true => diskmap::show(ui, &blank, "blank", budget, room),
            false => diskmap::show(ui, &job.progress, job.started, budget, room),
        }
        if install {
            self.install_rule(ui.ctx());
        }
    }

    /// The port Linux refused `job`, if it was refused one, and what can grant access.
    fn refused(&self, job: &Job) -> Option<Refused<'_>> {
        let port = chosen_port(self.service.known_ports(), &self.settings.device);
        Some(Refused {
            port: refused_port(job, port)?,
            rule: self.udev_rule.as_deref(),
            install: &self.install,
        })
    }

    /// Installs gw's udev rule through pkexec, which asks for a password.
    fn install_rule(&mut self, ctx: &egui::Context) {
        if let Some(rule) = &self.udev_rule
            && !matches!(self.install, RuleInstall::Running(_))
        {
            self.install = RuleInstall::Running(udev::install(rule, repaint(ctx)));
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
            // egui's Panel keys its slide by this id. Setting it here first
            // makes it take DRAWER_TIME, or none from one drawer to the
            // other: the Panel's own call this frame then sees no time pass.
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
        let (mut clear, mut unsaved) = (false, None);
        egui::Panel::bottom("log")
            .frame(frame)
            .resizable(true)
            .drag_to_open(false)
            .default_size(DRAWER)
            .size_range(DRAWER..=tallest)
            .show_collapsible(ui, &mut log, |ui| {
                let jobs = [&self.disk, &self.tool].into_iter().flatten();
                let tail = jobs.map(|j| self.log.tail(j)).find(|t| !t.is_empty());
                let shown = Shown::Log(&self.log, tail.unwrap_or_default());
                (clear, unsaved) = output(ui, shown);
            });
        if clear {
            self.log.clear();
        }
        if let Some(why) = unsaved {
            self.notices.insert(page.to_owned(), why);
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
        let id = ui.make_persistent_id("cli-text");
        // The text is the person's own while they type, and after if gw
        // cannot take it, until Reset; otherwise it follows the page.
        if ui.memory(|m| m.has_focus(id)) {
            cli.page = page.to_owned();
        } else if cli.error.is_none() || cli.page != page {
            cli.text.clone_from(&line);
            cli.error = None;
        }
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
                let reset = ui.add_enabled(cli.text != line, egui::Button::new("Reset"));
                if reset
                    .on_hover_text("Put back the page's command line.")
                    .on_disabled_hover_text("No changes.")
                    .clicked()
                {
                    cli.text.clone_from(&line);
                    cli.error = None;
                    ui.memory_mut(|m| m.surrender_focus(id));
                }
            });
        });
        ui.add_space(4.0);
        // Two rows, then it scrolls inside its frame.
        let rows = 2.0 * ui.text_style_height(&TextStyle::Monospace);
        let visuals = ui.visuals();
        let stroke = match ui.memory(|m| m.has_focus(id)) {
            true => visuals.selection.stroke,
            false => visuals.widgets.inactive.bg_stroke,
        };
        let edit = Frame::new()
            .fill(visuals.text_edit_bg_color())
            .stroke(stroke)
            .corner_radius(visuals.widgets.inactive.corner_radius)
            .inner_margin(Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.spacing_mut().scroll = egui::style::ScrollStyle {
                    bar_width: 4.0,
                    ..egui::style::ScrollStyle::solid()
                };
                ui.visuals_mut().widgets.inactive.bg_fill = p.line;
                egui::ScrollArea::vertical()
                    .id_salt("cli")
                    .max_height(rows)
                    .min_scrolled_height(rows)
                    .show(ui, |ui| {
                        ui.add(
                            TextEdit::multiline(&mut cli.text)
                                .id(id)
                                .font(TextStyle::Monospace)
                                .frame(Frame::NONE)
                                .desired_rows(2)
                                .desired_width(f32::INFINITY),
                        )
                    })
                    .inner
            })
            .inner
            .on_hover_text("Type or paste a gw command line. The page follows it.");
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
        let mut delete = None;
        let (mut save, mut pick, mut defaults) = (false, false, false);
        let menu = ui.menu_button("Presets", |ui| {
            ui.set_min_width(220.0);
            if self.presets.as_ref().is_none_or(|m| m.page != command) {
                self.presets = Some(PresetsMenu {
                    page: command.to_owned(),
                    saved: presets::list(&folder, command),
                    changed: self.changed(command),
                });
            }
            let Some(menu) = &self.presets else { return };
            if menu.saved.is_empty() {
                ui.label(RichText::new("No presets saved yet.").weak());
            }
            for (name, path) in &menu.saved {
                if ui
                    .button(name.as_str())
                    .on_hover_text("Use these settings.")
                    .clicked()
                {
                    load = Some(path.clone());
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
            if !menu.saved.is_empty() {
                ui.menu_button("Delete", |ui| {
                    for (name, path) in &menu.saved {
                        if ui
                            .button(name.as_str())
                            .on_hover_text("Delete this preset.")
                            .clicked()
                        {
                            delete = Some((name.clone(), path.clone()));
                            ui.close();
                        }
                    }
                });
            }
            ui.separator();
            defaults = ui
                .add_enabled(menu.changed, egui::Button::new("Restore defaults"))
                .on_hover_text("Put this page's options back to gw's defaults.")
                .on_disabled_hover_text("No changes.")
                .clicked();
            if save || pick || defaults {
                ui.close();
            }
        });
        if menu.inner.is_none() {
            self.presets = None;
        }
        if defaults {
            self.restore_defaults(command);
        }
        if let Some((name, path)) = delete {
            self.dialog = Some(Dialog::DeletePreset {
                command: command.to_owned(),
                name,
                path,
            });
        }
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
        section(ui, "Greaseweazle Tools", |ui| {
            match (&self.engine, &self.service.schema) {
                _ if self.gw_paused.is_some() => {
                    ui.label(RichText::new(UPDATING).weak());
                }
                (Some(engine), Load::Ready(schema)) => {
                    let origin = match engine.origin {
                        Origin::Bundled if engine.update_in(&engine::updates()).is_some() => {
                            "updated from GitHub"
                        }
                        Origin::Bundled => "built in",
                        Origin::Installed => "installed on this computer",
                        Origin::Custom => "chosen here",
                    };
                    ui.label(format!("gw {}, {origin}.", schema.version));
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
            let busy = self.busy();
            ui.horizontal(|ui| {
                let restart = ui.add_enabled(busy.is_none(), egui::Button::new("Restart"));
                if restart
                    .on_hover_text("Start gw again, and check GitHub for a newer release.")
                    .on_disabled_hover_text(busy.unwrap_or_default())
                    .clicked()
                {
                    self.connect(ui.ctx());
                }
                let Some(engine) = self.engine.as_ref().filter(|e| e.origin == Origin::Bundled)
                else {
                    return;
                };
                let (can, tip) = self.update_button(&self.gw_update, "Greaseweazle Tools");
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
                None,
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
                None,
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
            let busy = self.busy();
            match path_row(ui, "Greaseweazle Tools (gw cli)", &gw, tip, back, busy) {
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
                "Save gw's output beside each image it makes",
                "Writes the gw command and its output to name.ext.log where gw read or \
                 gw convert puts its image.",
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
        section(ui, "Update", |ui| self.app_update(ui));
        section(ui, "About", |ui| {
            ui.label(concat!(
                "Ferriteweazle ",
                env!("CARGO_PKG_VERSION"),
                " written with \u{2661} by Lee Hobson (@hobbo91), under the MIT license."
            ));
            ui.hyperlink_to("Source code and issues", REPO)
                .on_hover_text(REPO);
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
            ui.hyperlink_to("Getting started with Greaseweazle", GW_GUIDE)
                .on_hover_text(GW_GUIDE);
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
        // Room for the commands that grant a port.
        let width = match dialog {
            Dialog::Access { .. } => 560.0,
            _ => 420.0,
        };
        let response = egui::Modal::new(Id::new("dialog")).show(ctx, |ui| {
            ui.set_width(width);
            match &mut dialog {
                Dialog::Confirm {
                    command,
                    args,
                    disks,
                } => {
                    let disks = *disks;
                    if flashes_bootloader(command, args) {
                        dialog_heading(ui, "Update the bootloader?");
                        ui.label(
                            "If the flash fails, the Greaseweazle may need reflashing with a \
                             programming adapter.",
                        );
                    } else {
                        let drive = match self.settings.drive.as_str() {
                            "" => self.default_drive(),
                            drive => drive.to_owned(),
                        };
                        let verb = title(command);
                        let verb = verb.split(' ').next().unwrap_or_default();
                        let heading = match disks {
                            1 => format!("{verb} the disk in drive {drive}?"),
                            n => format!("{verb} {n} disks in drive {drive}?"),
                        };
                        dialog_heading(ui, &heading);
                        let why = DESTRUCTIVE
                            .iter()
                            .find(|(c, _)| c == command)
                            .map_or("", |(_, w)| *w);
                        ui.label(why);
                        if disks > 1 {
                            ui.label("It asks for each disk in turn.");
                        }
                    }
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        let p = theme::palette(ui);
                        let text = match disks {
                            1 => run_label(command).to_owned(),
                            _ => format!("{} 1", run_label(command)),
                        };
                        if ui.add(dialog_button(&text, p.bad, p)).clicked() {
                            let (ctx, command, args) = (ctx.clone(), command.clone(), args.clone());
                            action = Some(match disks {
                                1 => Box::new(move |app: &mut App| {
                                    app.run(&ctx, &command, args);
                                }),
                                _ => Box::new(move |app: &mut App| app.next_disk(&ctx)),
                            });
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
                    command,
                    disk,
                    total,
                    failed,
                    image,
                } => {
                    let (disk, failed) = (*disk, *failed);
                    let (verb, p) = (run_label(command), theme::palette(ui));
                    let heading = match disk {
                        Some(disk) => format!("Insert disk {disk} of {total}"),
                        None => format!("{verb} {total} again?"),
                    };
                    dialog_heading(ui, &heading);
                    if let Some(failed) = failed {
                        let text = format!("Disk {failed} failed. The Log says why.");
                        ui.label(RichText::new(text).color(p.bad));
                    }
                    if disk.is_some() {
                        ui.label("Eject, then insert the next disk in the drive.");
                    }
                    if let Some(image) = image {
                        ui.label(RichText::new(format!("Next image: {image}")).color(p.dim));
                    }
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        if let Some(disk) = disk {
                            let next = format!("{verb} {disk}");
                            if ui.add(dialog_button(&next, p.accent, p)).clicked() {
                                let ctx = ctx.clone();
                                action = Some(Box::new(move |app: &mut App| app.next_disk(&ctx)));
                                close = true;
                            }
                        }
                        if let Some(failed) = failed {
                            let again = format!("{verb} {failed} again");
                            let button = match disk {
                                Some(_) => dialog_plain(&again),
                                None => dialog_button(&again, p.accent, p),
                            };
                            if ui
                                .add(button)
                                .on_hover_text("Try the disk that failed again.")
                                .clicked()
                            {
                                let ctx = ctx.clone();
                                action = Some(Box::new(move |app: &mut App| app.disk_again(&ctx)));
                                close = true;
                            }
                        }
                        let tip = match command.as_str() {
                            "read" => "End the session. The disks read so far are kept.",
                            _ => "End the session.",
                        };
                        if ui
                            .add(dialog_plain("Stop here"))
                            .on_hover_text(tip)
                            .clicked()
                        {
                            action = Some(Box::new(|app: &mut App| app.end_session()));
                            close = true;
                        }
                    });
                }
                Dialog::SavePreset { command, name } => {
                    dialog_heading(ui, "Save a preset");
                    ui.add(
                        form::edit(name)
                            .char_limit(form::NAME_LIMIT)
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
                            .on_disabled_hover_text("Type a name.")
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
                Dialog::DeletePreset {
                    command,
                    name,
                    path,
                } => {
                    dialog_heading(ui, &format!("Delete \"{name}\"?"));
                    ui.label("This cannot be undone.");
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        let p = theme::palette(ui);
                        if ui.add(dialog_button("Delete", p.bad, p)).clicked() {
                            let (command, path) = (command.clone(), path.clone());
                            action = Some(Box::new(move |app: &mut App| {
                                app.delete_preset(&command, &path)
                            }));
                            close = true;
                        }
                        if ui.add(dialog_plain("Cancel")).clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Access { port } => {
                    let refused = Refused {
                        port: port.clone(),
                        rule: self.udev_rule.as_deref(),
                        install: &self.install,
                    };
                    if access(ui, &refused) {
                        let ctx = ctx.clone();
                        action = Some(Box::new(move |app: &mut App| app.install_rule(&ctx)));
                    }
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        if ui.add(dialog_plain("Close")).clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Quit => {
                    let job = self.running();
                    let name = job.map_or_else(String::new, |j| title(&j.command));
                    dialog_heading(ui, &format!("Stop {name} and quit?"));
                    ui.label(match job.is_some_and(|j| self.runs_motor(j)) {
                        true => "gw stops the drive first, then the window closes.",
                        false => "gw stops, then the window closes.",
                    });
                    if let Some(warning) = job.and_then(flash_warning) {
                        ui.label(warning);
                    }
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
            let waits = matches!(
                dialog,
                Dialog::NextDisk { .. } | Dialog::Confirm { disks: 2.., .. }
            );
            if action.is_none() && waits {
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

    /// What GitHub has for this copy, with Update at the right.
    fn app_update(&mut self, ui: &mut Ui) {
        let install = Install::this();
        let (line, why) = match install {
            Some(_) => self.app_update.summary(env!("CARGO_PKG_VERSION")),
            None => ("This copy was built from source.".into(), None),
        };
        let stuck = match &install {
            None => Some("Needs a copy installed from a release."),
            Some(install) => install.stuck(),
        };
        let (can, tip) = match stuck {
            Some(why) => (false, why.to_owned()),
            None => self.update_button(&self.app_update, "Ferriteweazle"),
        };
        let spin = matches!(
            self.app_update,
            Update::Checking(_) | Update::Installing(..)
        );
        ui.horizontal(|ui| {
            if spin {
                ui.spinner();
            }
            let status = ui.label(line);
            if let Some(why) = why {
                status.on_hover_text(why);
            }
            right(ui, |ui| {
                let update = ui.add_enabled(can, egui::Button::new("Update"));
                if update
                    .on_hover_text(&tip)
                    .on_disabled_hover_text(&tip)
                    .clicked()
                    && let (Update::Newer(tag), Some(engine), Some(install)) =
                        (&self.app_update, &self.engine, install.clone())
                {
                    if cfg!(windows) && matches!(install, Install::Folder(_)) {
                        // Windows will not move the data folder while gw runs from it.
                        self.gw_paused = Some(self.service.known_ports().to_vec());
                        self.service = Service::offline(Err(UPDATING.into()));
                    }
                    self.app_update = Update::app(engine, install, tag, repaint(ui.ctx()));
                }
            });
        });
        if !can && matches!(self.app_update, Update::Newer(_)) {
            ui.label(RichText::new(tip).small().weak());
        }
    }

    /// Whether an Update button can run, and its tip: an update that could
    /// install waits while a job runs or the other update installs.
    fn update_button(&self, update: &Update, what: &str) -> (bool, String) {
        match (update.button(what), self.busy()) {
            ((true, _), Some(why)) => (false, why.to_owned()),
            (button, _) => button,
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

    /// Deletes a preset's file. A fault shows on `page`.
    fn delete_preset(&mut self, page: &str, path: &Path) {
        if let Err(e) = std::fs::remove_file(path) {
            let text = format!("Could not delete the preset: {e}");
            self.notices.insert(page.to_owned(), text);
        }
    }

    /// Output settings as a new page has them: gw's defaults, and the
    /// images folder.
    fn fresh_output(&self) -> Output {
        Output {
            folder: self.images_folder().to_string_lossy().into_owned(),
            ..Output::default()
        }
    }

    /// Whether a page differs from what Restore defaults leaves.
    fn changed(&self, command: &str) -> bool {
        let fresh = self.fresh_output();
        let mut keys = form::OUTPUTS
            .iter()
            .filter(|(c, _)| *c == command)
            .map(|(c, dest)| form::output_key(c, dest));
        let options = self.settings.values.get(command);
        options.is_some_and(|v| *v != Values::default())
            || keys.any(|k| self.settings.outputs.get(&k).is_some_and(|o| *o != fresh))
    }

    /// Puts a page's options and output settings back to gw's defaults. The
    /// device and drive are the sidebar's, and stay.
    fn restore_defaults(&mut self, command: &str) {
        self.settings.values.remove(command);
        for (c, dest) in form::OUTPUTS.iter().filter(|(c, _)| *c == command) {
            let fresh = self.fresh_output();
            self.settings
                .outputs
                .insert(form::output_key(c, dest), fresh);
        }
        // Such as the format detection found, which the page no longer has.
        self.notices.remove(command);
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
    let mut answer = None;
    egui::Modal::new(Id::new("question")).show(ctx, |ui| {
        ui.set_width(400.0);
        dialog_heading(ui, "gw asks");
        ui.label(question.trim());
        ui.add_space(10.0);
        if question.contains("Yes/No") {
            right(ui, |ui| {
                for choice in ["Yes", "No"] {
                    if ui.add(dialog_plain(choice)).clicked() {
                        answer = Some(choice.to_owned());
                    }
                }
            });
        } else {
            let mut text: String = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
            ui.add(form::edit(&mut text).desired_width(f32::INFINITY));
            ui.data_mut(|d| d.insert_temp(id, text.clone()));
            if ui.add(dialog_plain("Answer")).clicked() {
                answer = Some(text);
                ui.data_mut(|d| d.remove_temp::<String>(id));
            }
        }
    });
    if let Some(answer) = answer {
        // After the question, as a terminal shows it.
        job.log.push(format!("{question}{answer}"));
        job.answer(&answer);
    }
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

/// How gw's errors begin, which the log shows in red.
const ERRORS: [&str; 5] = [
    "** FATAL ERROR:",
    "** UPDATE FAILED",
    "ERROR: ",
    "Command Failed",
    "Traceback (most recent call last):",
];

/// A log line's colour: red for gw's errors and a fatal error's first
/// line, which follows the line `before`; orange for warnings, retries
/// and what an update leaves to do.
fn log_colour(line: &str, before: Option<&str>, p: &Palette) -> Option<Color32> {
    if ERRORS.iter().any(|e| line.starts_with(e))
        || line.contains(": error:")
        || before == Some("** FATAL ERROR:")
    {
        Some(p.bad)
    } else if ["WARNING", "Giving up", "Retry #"]
        .iter()
        .any(|w| line.contains(w))
        || ["** SKIPPING UPDATE", "** Unplug device"]
            .iter()
            .any(|u| line.starts_with(u))
    {
        Some(p.partial)
    } else {
        None
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

/// gw's runs of a command: once per disk for a read of several, once per
/// image of a batch's `images`, else once.
fn runs(
    cmd: &Command,
    mut values: Values,
    outputs: &BTreeMap<String, Output>,
    images: &[PathBuf],
    argv: impl Fn(&Values) -> Vec<String>,
) -> Runs {
    let out = form::OUTPUTS
        .iter()
        .find(|(c, _)| *c == cmd.name)
        .and_then(|(c, dest)| Some((*dest, outputs.get(&form::output_key(c, dest))?)));
    if let Some(dest) = form::batch_input(cmd, &values) {
        let mut runs = Runs::default();
        for image in images {
            values.set(dest, image.to_string_lossy());
            if let Some((out_dest, out)) = out {
                values.set(out_dest, out.batch_value(image));
                runs.makes.push(out.batch_path(image));
            }
            runs.args.push(argv(&values));
            let name = image.file_name().map(|n| n.to_string_lossy().into_owned());
            runs.images.push(name.unwrap_or_default());
        }
        return runs;
    }
    let (args, makes) = match out {
        Some((dest, out)) if cmd.name == "read" => {
            let args = (1..=out.disks.max(1))
                .map(|d| {
                    values.set(dest, out.value(d));
                    argv(&values)
                })
                .collect();
            (args, out.paths().collect())
        }
        Some((_, out)) => (vec![argv(&values)], vec![out.path(1)]),
        None => (vec![argv(&values)], Vec::new()),
    };
    Runs {
        args,
        makes,
        ..Runs::default()
    }
}

fn destructive(command: &str) -> bool {
    DESTRUCTIVE.iter().any(|(c, _)| *c == command)
}

/// Whether gw update flashes the bootloader, which asks first.
fn flashes_bootloader(command: &str, args: &[String]) -> bool {
    command == "update" && args.iter().any(|a| a == "--bootloader")
}

/// What an update stopped part way through its flash leaves to put right.
fn flash_warning(job: &Job) -> Option<&'static str> {
    if job.command != "update" {
        None
    } else if flashes_bootloader(&job.command, &job.args) {
        Some("A bootloader flash stopped part way may need reflashing with a programming adapter.")
    } else {
        Some("A flash stopped part way leaves the firmware erased until Update runs again.")
    }
}

fn installing(update: &Update) -> bool {
    matches!(update, Update::Installing(..))
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
        _ => "Stopped after",
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
                // gw's own steps: an F1 needs its Update Jumper fitted first.
                for step in &info.steps {
                    ui.label(RichText::new(step).small().weak());
                }
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
        Some(Outcome::Stopped) => ("Stopped", p.partial),
    }
}

/// What a disk job that did not finish leaves behind: gw keeps the tracks a
/// stopped read has done, and deletes a stopped conversion's image and
/// that of a job that failed.
fn left_behind(job: &Job) -> Option<&'static str> {
    match (job.outcome()?, job.command.as_str()) {
        (Outcome::Failed, _) if job.no_image => Some("Failed: no image kept."),
        (Outcome::Stopped, "read") => Some("Stopped: incomplete image."),
        (Outcome::Stopped, "write") => Some("Stopped: disk partly written."),
        (Outcome::Stopped, "erase") => Some("Stopped: disk partly erased."),
        (Outcome::Stopped, "convert") => Some("Stopped: no image made."),
        _ => None,
    }
}

/// What a command other than a disk job did, under its page. Gives whether
/// Install udev rule was pressed, and why saving the output failed.
fn result(ui: &mut Ui, job: &Job, refused: Option<Refused>) -> (bool, Option<String>) {
    let p = theme::palette(ui);
    let mut install = false;
    let mut unsaved = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Result").strong());
        let (text, colour) = state(job, p);
        pill(ui, text, colour);
        right(ui, |ui| {
            ui.label(RichText::new(clock(job.elapsed())).monospace().weak());
        });
    });
    match (&refused, &job.progress.error) {
        (Some(refused), _) => error_box(ui, p.bad, |ui| install = access(ui, refused)),
        (None, Some(e)) => {
            // Orange for a job that worked all the same, as gw info does
            // when only its check for newer firmware fails.
            let colour = match job.outcome() {
                Some(Outcome::Succeeded) => p.partial,
                _ => p.bad,
            };
            error_box(ui, colour, |ui| {
                ui.label(RichText::new(e).color(colour));
            });
        }
        (None, None) => {}
    }
    ui.add_space(4.0);
    // The device's report reads as a table; gw's raw words stay in the Log.
    if job.command == "info"
        && !job.running()
        && let Some(info) = device::parse(&job.log)
    {
        device_table(ui, &info, p);
    } else {
        (_, unsaved) = output(ui, Shown::Job(job));
    }
    (install, unsaved)
}

/// A port Linux refused gw, and what can grant this account access to it.
struct Refused<'a> {
    port: String,
    rule: Option<&'a Path>,
    install: &'a RuleInstall,
}

/// The port a job was refused for want of permission: the one gw's error
/// names, else `port` when Linux denies it and `gw info` found no device.
fn refused_port(job: &Job, port: Option<&Port>) -> Option<String> {
    if let Some(port) = job.progress.error.as_deref().and_then(udev::denied_port) {
        return Some(port.to_owned());
    }
    let port = port.filter(|p| p.denied)?;
    let unanswered = job.command == "info" && !job.running() && device::parse(&job.log).is_none();
    unanswered.then(|| port.device.clone())
}

const NO_ACCESS: &str = "This account has no permission to open the port. gw's udev rule \
                         gives the user logged in at this computer access to a Greaseweazle, \
                         and tells ModemManager to leave it alone.";

/// Why gw was refused the port, and gw's udev rule: a button that installs
/// it, and the commands that do the same. True when the button is pressed.
fn access(ui: &mut Ui, refused: &Refused) -> bool {
    let p = theme::palette(ui);
    let heading = format!("No access to {}", refused.port);
    ui.label(RichText::new(heading).strong().color(p.bad));
    ui.add(egui::Label::new(NO_ACCESS).wrap());
    ui.add_space(4.0);
    let running = matches!(refused.install, RuleInstall::Running(_));
    let pressed = ui
        .add_enabled(
            refused.rule.is_some() && !running,
            egui::Button::new("Install udev rule"),
        )
        .on_hover_text("Copy it to /etc/udev/rules.d and reload udev, as root, through pkexec.")
        .on_disabled_hover_text(match running {
            true => "Waiting for pkexec…",
            false => "No copy of the rule ships with this build.",
        })
        .clicked();
    match refused.install {
        RuleInstall::Idle => {}
        RuleInstall::Running(_) => {
            text_row(ui, |ui| {
                ui.add(egui::Spinner::new().size(10.0));
                ui.label(RichText::new("Waiting for pkexec…").small().weak());
            });
        }
        RuleInstall::Done(Ok(())) => {
            ui.label(RichText::new("Installed, and udev has applied it.").color(p.good));
        }
        RuleInstall::Done(Err(e)) => {
            ui.add(egui::Label::new(RichText::new(e).color(p.partial)).wrap());
        }
    }
    ui.add_space(4.0);
    let commands = udev::commands(refused.rule);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Or in a terminal:").weak());
        right(ui, |ui| {
            if ui
                .small_button("Copy")
                .on_hover_text("Copy these commands.")
                .clicked()
            {
                ui.ctx().copy_text(commands.join("\n"));
            }
        });
    });
    // A command to a line, never broken: the box scrolls sideways instead,
    // with a bar that shows there is more.
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(6)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().scroll = egui::style::ScrollStyle {
                foreground_color: true,
                dormant_handle_opacity: 0.35,
                ..egui::style::ScrollStyle::thin()
            };
            egui::ScrollArea::horizontal()
                .id_salt("udev-commands")
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for command in &commands {
                        let text = RichText::new(command).monospace();
                        ui.add(egui::Label::new(text).extend().selectable(true));
                    }
                });
        });
    ui.hyperlink_to("gw's Linux instructions", udev::WIKI)
        .on_hover_text(udev::WIKI);
    pressed
}

/// A box tinted `colour`, for what went wrong.
fn error_box(ui: &mut Ui, colour: Color32, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(colour.gamma_multiply(0.14))
        .corner_radius(8)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// What an output box shows.
#[derive(Clone, Copy)]
enum Shown<'a> {
    /// Every job's output this session, and the line the last is still printing.
    Log(&'a SessionLog, &'a str),
    /// A job's output under its page.
    Job(&'a Job),
}

/// A job's output box, in points.
const OUTPUT_HEIGHT: f32 = 260.0;

/// gw's output with Copy and Save: the Log as tall as the room left, with
/// Clear, or a job's in a box of its own. Gives whether Clear was pressed,
/// and why a save failed.
fn output(ui: &mut Ui, shown: Shown) -> (bool, Option<String>) {
    let p = theme::palette(ui);
    let (heading, log, tail) = match shown {
        Shown::Log(log, tail) => ("Log", log.lines(), tail),
        Shown::Job(job) => ("Output", job.log.as_slice(), job.partial.as_str()),
    };
    let drawer = matches!(shown, Shown::Log(..));
    let (mut clear, mut unsaved) = (false, None);
    ui.horizontal(|ui| {
        ui.label(RichText::new(heading).strong());
        if let Shown::Log(log, _) = shown
            && log.trimmed()
        {
            ui.label(RichText::new("Older lines were dropped.").small().weak());
        }
        right(ui, |ui| {
            let save = ui.add_enabled(!log.is_empty(), egui::Button::new("Save…"));
            if save
                .on_hover_text("Save gw's output to a file.")
                .on_disabled_hover_text("No output.")
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_directory(crate::app_folder())
                    .set_file_name("gw.log")
                    .save_file()
            {
                unsaved = save_log(&path, log);
            }
            let copy = ui.add_enabled(!log.is_empty(), egui::Button::new("Copy"));
            if copy
                .on_hover_text("Copy gw's output.")
                .on_disabled_hover_text("No output.")
                .clicked()
            {
                ui.ctx().copy_text(log.join("\n"));
            }
            if drawer {
                let button = ui.add_enabled(!log.is_empty(), egui::Button::new("Clear"));
                clear = button
                    .on_hover_text("Clear the log.")
                    .on_disabled_hover_text("No output.")
                    .clicked();
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
    let height = match drawer {
        true => room(),
        false => OUTPUT_HEIGHT,
    }
    .max(LOG_LINE);
    frame.show(ui, |ui| {
        if log.is_empty() && tail.is_empty() {
            let least = if drawer { height } else { height.min(80.0) };
            ui.set_min_size(vec2(ui.available_width(), least));
            let empty = match shown {
                Shown::Job(job) if !job.running() => "gw printed nothing.",
                _ => "gw's output appears here.",
            };
            ui.label(RichText::new(empty).weak());
            return;
        }
        let row = ui.text_style_height(&TextStyle::Monospace);
        // A line gw has not ended yet comes last.
        let lines = log.len() + usize::from(!tail.is_empty());
        // Bars drawn whenever there is more to see, as a text view's: a
        // floating one hides until hovered, and a wheel does not scroll
        // sideways. The theme paints an idle handle in the card's colour.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        ui.visuals_mut().widgets.inactive.bg_fill = p.line;
        egui::ScrollArea::both()
            .id_salt("log")
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height)
            .show_rows(ui, row, lines, |ui, rows| {
                for i in rows {
                    let line = log.get(i).map_or(tail, String::as_str);
                    let before = i.checked_sub(1).map(|b| log[b].as_str());
                    let colour = match shown {
                        Shown::Log(log, _) if log.is_head(i) => Some(p.accent),
                        _ => log_colour(line, before, p),
                    };
                    let mut text = RichText::new(line).monospace();
                    if let Some(colour) = colour {
                        text = text.color(colour);
                    }
                    ui.add(egui::Label::new(text).extend().selectable(true));
                }
            });
    });
    (clear, unsaved)
}

/// Writes `log` to `path`, and says why if it cannot.
fn save_log(path: &Path, log: &[String]) -> Option<String> {
    let text = log.join("\n") + "\n";
    let failed = std::fs::write(path, text).err()?;
    Some(format!("Could not save {}: {failed}", path.display()))
}

/// Where the drive identifier is kept between runs: the one setting kept.
fn drive_file() -> PathBuf {
    crate::data_folder().join("drive.txt")
}

/// The drive kept in `file`, empty for gw's default.
fn kept_drive(file: &Path) -> String {
    let drive = std::fs::read_to_string(file).unwrap_or_default();
    let drive = drive.trim();
    match drive.len() == 1 && drive.chars().all(|c| c.is_ascii_alphanumeric()) {
        true => drive.to_owned(),
        false => String::new(),
    }
}

/// Keeps `drive` in `file`, or removes the file for gw's default.
fn keep_drive(file: &Path, drive: &str) {
    let _ = match drive {
        "" => std::fs::remove_file(file),
        _ => file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(file, drive)),
    };
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
            let mut names: Vec<String> = more.iter().take(4).cloned().collect();
            if more.len() > 4 {
                names.push(format!("{} more", more.len() - 4));
            }
            let (last, rest) = names.split_last().expect("more has some");
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

/// A path in Settings: its name, where it is, Choose…, and with `back` a
/// button and tip that restore the default. Both wait while `busy` says why.
fn path_row(
    ui: &mut Ui,
    name: &str,
    path: &Path,
    tip: &str,
    back: Option<(&str, &str)>,
    busy: Option<&str>,
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
        let why = busy.unwrap_or_default();
        let choose = ui.add_enabled(busy.is_none(), egui::Button::new("Choose…"));
        if choose
            .on_hover_text(tip)
            .on_disabled_hover_text(why)
            .clicked()
        {
            return Some(PathClick::Choose);
        }
        let (text, tip) = back?;
        ui.add_enabled(busy.is_none(), egui::Button::new(text))
            .on_hover_text(tip)
            .on_disabled_hover_text(why)
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
    let tuck = vec2(LOGO_SIZE * LOGO_CLEAR, LOGO_TUCK);
    let (rect, _) = ui.allocate_exact_size(size - tuck, Sense::hover());
    let at = egui::Rect::from_min_size(rect.min - tuck, size);
    egui::Image::new((texture.id(), size)).paint_at(ui, at);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    fn schema() -> Schema {
        serde_json::from_str(include_str!("../tests/data/schema-1.23.json")).unwrap()
    }

    fn offline() -> App {
        App::offline(&egui::Context::default(), Settings::default(), Ok(schema()))
    }

    /// A Greaseweazle as gw lists it, on a made-up port.
    fn greaseweazle(device: &str, denied: bool) -> Port {
        Port {
            device: device.into(),
            name: Some("Greaseweazle".into()),
            serial: None,
            score: 20,
            denied,
        }
    }

    /// An engine with no Python behind it: no job starts.
    fn no_gw() -> Engine {
        Engine {
            python: "/no/such/python".into(),
            origin: Origin::Custom,
        }
    }

    /// A job that runs until the test ends it.
    fn running(command: &str) -> Job {
        let mut job = Job::replay(command, "");
        job.ended = None;
        job
    }

    /// An install that has not answered yet.
    fn installing() -> Update {
        Update::Installing(std::sync::mpsc::channel().1, "v1.24".into())
    }

    /// The window after a couple of frames. Stepped, not run: a running job
    /// keeps it repainting.
    fn window(app: App) -> Harness<'static, App> {
        let mut w = Harness::builder()
            .with_size(vec2(1240.0, 780.0))
            .build_ui_state(|ui, app: &mut App| app.show(ui), app);
        w.run_steps(2);
        w
    }

    #[test]
    fn a_port_that_does_not_answer_says_so_after_a_greaseweazle_did() {
        let mut app = offline();
        let lines = |text: &str| text.lines().map(String::from).collect::<Vec<_>>();
        app.device = device::parse(&lines(
            "Host Tools: 1.23\nDevice:\n  Port:     /dev/cu.usbmodem14201\n  Model:    Greaseweazle V4.1",
        ));
        app.pin_ports(vec![Port {
            device: "/dev/cu.debug-console".into(),
            name: None,
            serial: None,
            score: 0,
            denied: false,
        }]);
        app.settings.device = "/dev/cu.debug-console".into();
        let failed = "Host Tools: 1.23\nDevice:\n** FATAL ERROR:\nThe Greaseweazle did not answer.";
        app.probe = Some(Job::replay("info", failed));
        app.poll_probe(&egui::Context::default());
        assert_eq!(
            app.probe_failed.as_deref(),
            Some("The Greaseweazle did not answer.")
        );
        assert!(!app.answering(), "the dot stays green");
    }

    #[test]
    fn the_drive_is_kept_between_runs_and_gws_default_leaves_no_file() {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-drive-{}", std::process::id()));
        let file = dir.join("drive.txt");
        assert_eq!(kept_drive(&file), "", "nothing kept yet");
        keep_drive(&file, "B");
        assert_eq!(kept_drive(&file), "B");
        keep_drive(&file, "");
        assert!(!file.exists());
        std::fs::write(&file, "not a drive").unwrap();
        assert_eq!(kept_drive(&file), "", "only an identifier is taken");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_on_the_read_page_waits_while_the_device_card_asks_gw() {
        let mut app = offline();
        app.pin_ports(vec![greaseweazle("/dev/cu.usbmodem14201", false)]);
        assert_eq!(app.cannot_detect("read"), None);
        let mut probe = Job::replay("info", "");
        probe.ended = None;
        app.probe = Some(probe);
        assert_eq!(app.cannot_detect("read"), Some(ASKING));
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
        let atari = [
            "atarist.720",
            "ibm.360",
            "ibm.720",
            "msx.2d",
            "msx.2dd",
            "zx.3dos.ds80",
            "zx.d80.ds80",
        ]
        .map(String::from);
        assert_eq!(
            found_note(&atari, 1),
            "Found atarist.720. Disk also matches ibm.360, ibm.720, msx.2d, msx.2dd and 2 more."
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
    fn a_batch_convert_runs_gw_once_for_each_image_and_names_what_it_makes() {
        let schema = schema();
        let convert = schema.command("convert").unwrap();
        let mut values = Values::default();
        values.set(form::BATCH, command::ON);
        values.set("format", "ibm.1440");
        let out = Output {
            folder: "/out".into(),
            ext: ".img".into(),
            suffix: "_pc".into(),
            ..Output::default()
        };
        let outputs = BTreeMap::from([("convert/out_file".to_owned(), out)]);
        let images = [PathBuf::from("/in/A.scp"), PathBuf::from("/in/B.scp")];
        let runs = runs(convert, values, &outputs, &images, |v| {
            command::argv(convert, v)
        });
        assert_eq!(runs.images, ["A.scp", "B.scp"]);
        let made = ["A_pc.img", "B_pc.img"].map(|n| Path::new("/out").join(n));
        assert_eq!(runs.makes, made);
        for ((args, image), made) in runs.args.iter().zip(&images).zip(&made) {
            let files = &args[args.len() - 2..];
            assert_eq!(files, [image.to_string_lossy(), made.to_string_lossy()]);
            assert!(args.contains(&"--format=ibm.1440".to_owned()));
        }
    }

    #[test]
    fn a_batch_write_confirms_once_asks_for_each_disk_by_its_image_and_says_how_it_went() {
        let mut app = offline();
        let ctx = egui::Context::default();
        let runs = Runs {
            args: vec![
                vec!["write".into(), "a.adf".into()],
                vec!["write".into(), "b.adf".into()],
            ],
            images: vec!["a.adf".into(), "b.adf".into()],
            makes: Vec::new(),
        };
        app.begin(&ctx, "write", runs);
        assert!(matches!(app.dialog, Some(Dialog::Confirm { disks: 2, .. })));

        app.dialog = None;
        app.session.as_mut().unwrap().next = 1;
        app.disk = Some(Job::replay("write", "T0.0: Writing Track"));
        app.ended(&ctx, true);
        let next = match &app.dialog {
            Some(Dialog::NextDisk {
                command,
                disk,
                image,
                ..
            }) => (command.as_str(), *disk, image.as_deref()),
            _ => panic!("no next disk"),
        };
        assert_eq!(next, ("write", Some(2), Some("b.adf")));

        app.dialog = None;
        app.session.as_mut().unwrap().next = 2;
        app.disk = Some(Job::replay(
            "write",
            "Command Failed: GetFluxStatus: No Index",
        ));
        app.ended(&ctx, true);
        let again = matches!(
            app.dialog,
            Some(Dialog::NextDisk {
                disk: None,
                failed: Some(2),
                ..
            })
        );
        assert!(again, "it can be tried again");
        // Stop here.
        app.end_session();
        let note = "Wrote 1 of 2 disks. Failed: b.adf. The Log says why.";
        assert_eq!(app.notices["write"], note);
    }

    #[test]
    fn a_disk_that_fails_can_be_tried_again_the_last_one_too() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.session = Some(Session {
            command: "read".into(),
            runs: reads(&["a.adf", "b.adf", "c.adf"]),
            next: 2,
            failed: Vec::new(),
        });
        let no_index = || {
            Some(Job::replay(
                "read",
                "Command Failed: GetFluxStatus: No Index",
            ))
        };
        app.disk = no_index();
        app.ended(&ctx, true);
        let mut w = window(app);
        w.get_by_label("Insert disk 3 of 3");
        w.get_by_label("Disk 2 failed. The Log says why.");
        w.get_by_label("Read disk 3");
        w.get_by_label("Read disk 2 again");

        let app = w.state_mut();
        app.dialog = None;
        app.session.as_mut().unwrap().next = 3;
        app.disk = no_index();
        app.ended(&ctx, true);
        assert!(app.session.is_some(), "the session ended");
        w.run_steps(2);
        w.get_by_label("Read disk 3 again?");
        w.get_by_label("Disk 3 failed. The Log says why.");
        w.get_by_role_and_label(egui::accesskit::Role::Button, "Read disk 3 again");
        let next = "Eject, then insert the next disk in the drive.";
        assert!(w.query_by_label(next).is_none(), "no disk is left");
    }

    #[test]
    fn a_read_of_three_disks_runs_gw_once_for_each() {
        let schema = schema();
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
        let runs = runs(read, values, &outputs, &[], |v| command::argv(read, v));
        let (files, runs) = (runs.makes, runs.args);
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
        let schema = schema();
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
        // A folder inside a file cannot be made on any system.
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml/presets");
        let settings = Settings {
            presets_folder: Some(folder),
            ..Settings::default()
        };
        let mut app = App::offline(&ctx, settings, Err(String::new()));
        app.engine = Some(no_gw());
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
        let mut app = offline();
        app.pin_ports(vec![greaseweazle("/dev/cu.usbmodem14201", false)]);
        assert!(app.connected());
        assert!(!sections(app.listed.as_deref()).is_empty());
        app.settings.engine = Some("/no/such/gw".into());
        app.connect(&ctx);
        app.poll(&ctx);
        assert!(app.engine.is_none());
        assert!(!app.connected(), "no gw will look for it");
        assert!(sections(app.listed.as_deref()).is_empty(), "no gw runs");
    }

    #[test]
    fn system_follows_the_desktops_preference_and_a_chosen_theme_wins() {
        let ctx = egui::Context::default();
        let mut app = offline();
        let (desktop, answers) = std::sync::mpsc::channel();
        app.desktop_theme = Some(answers);
        for theme in [Theme::Light, Theme::Dark, Theme::Light] {
            desktop.send(theme).unwrap();
            app.follow_desktop(&ctx);
            assert_eq!(ctx.theme(), theme);
        }
        ctx.set_theme(ThemePreference::Dark);
        desktop.send(Theme::Light).unwrap();
        app.follow_desktop(&ctx);
        assert_eq!(ctx.theme(), Theme::Dark, "a theme chosen in Settings wins");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_the_window_frame_follows_the_theme_shown() {
        let ctx = egui::Context::default();
        let mut app = offline();
        let (desktop, answers) = std::sync::mpsc::channel();
        app.desktop_theme = Some(answers);
        let frames = |app: &mut App| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.follow_desktop(ui.ctx()));
            out.textures_delta.clear(); // no painter here
            let commands = &out.viewport_output[&egui::ViewportId::ROOT].commands;
            commands
                .iter()
                .filter(|c| matches!(c, ViewportCommand::SetTheme(_)))
                .cloned()
                .collect::<Vec<_>>()
        };
        let light = || ViewportCommand::SetTheme(egui::SystemTheme::Light);
        let dark = ViewportCommand::SetTheme(egui::SystemTheme::Dark);
        desktop.send(Theme::Light).unwrap();
        assert_eq!(frames(&mut app), [light()]);
        assert_eq!(frames(&mut app), [], "only when the theme changes");
        ctx.set_theme(ThemePreference::Dark);
        assert_eq!(frames(&mut app), [dark], "a theme chosen in Settings too");
        ctx.set_theme(ThemePreference::System);
        assert_eq!(frames(&mut app), [light()]);
    }

    /// What gw prints when Linux refuses it the port: pyserial's EACCES error.
    const REFUSED: &str = "** FATAL ERROR:\n[Errno 13] could not open port /dev/ttyACM0: \
                           [Errno 13] Permission denied: '/dev/ttyACM0'";

    const RULE: &str = "/opt/Ferriteweazle/ferriteweazle-data/49-greaseweazle.rules";

    fn has_the_fix(log: &[String]) -> bool {
        let log = log.join("\n");
        [
            "No access to /dev/ttyACM0: this account has no permission to open it.",
            &format!("  sudo cp {RULE} /etc/udev/rules.d/"),
            "  sudo udevadm control --reload-rules && sudo udevadm trigger",
            udev::WIKI,
        ]
        .iter()
        .all(|line| log.contains(line))
    }

    #[test]
    fn a_port_refused_for_want_of_permission_puts_the_fix_in_the_log() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.udev_rule = Some(RULE.into());
        app.disk = Some(Job::replay("read", REFUSED));
        app.ended(&ctx, true);
        assert!(has_the_fix(app.log.lines()), "{:#?}", app.log.lines());
    }

    #[test]
    fn gw_info_on_a_port_linux_denies_puts_the_fix_in_the_log() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.udev_rule = Some(RULE.into());
        app.pin_ports(vec![greaseweazle("/dev/ttyACM0", true)]);
        // gw info prints "Not found" for a port pyserial may not open.
        app.probe = Some(Job::replay(
            "info",
            "Host Tools: 1.23\nDevice:\n  Not found",
        ));
        app.poll_probe(&ctx);
        assert!(has_the_fix(app.log.lines()), "{:#?}", app.log.lines());
    }

    #[test]
    fn no_job_starts_while_gw_or_this_app_installs_an_update() {
        let schema = schema();
        let info = schema.command("info").unwrap();
        let mut app = offline();
        app.engine = Some(no_gw());
        app.pin_ports(vec![greaseweazle("/dev/cu.usbmodem14201", false)]);
        assert_eq!(app.why_not(&schema, info), None);
        app.gw_update = installing();
        assert_eq!(app.why_not(&schema, info).as_deref(), Some(INSTALLING));
        assert_eq!(app.cannot_detect("convert"), Some(INSTALLING));
        app.gw_update = Update::Idle;
        app.app_update = installing();
        assert_eq!(app.why_not(&schema, info).as_deref(), Some(INSTALLING));
    }

    #[test]
    fn a_command_line_gw_cannot_take_holds_up_its_page_until_reset() {
        use egui::accesskit::Role;
        let mut app = offline();
        app.engine = Some(no_gw());
        app.pin_ports(vec![greaseweazle("/dev/cu.usbmodem14201", false)]);
        app.settings.page = Page::Command("info".into());
        app.settings.drawer = Some(Drawer::Cli);
        let mut w = window(app);
        let greyed = |w: &Harness<'_, App>| {
            let run = w.get_by_role_and_label(Role::Button, "Get info");
            run.accesskit_node().is_disabled()
        };
        assert!(!greyed(&w));
        w.get_by_role(Role::MultilineTextInput).click();
        w.run_steps(2);
        w.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        w.event(egui::Event::Text("gw info --bogus".into()));
        w.run_steps(2);
        w.get_by_label("gw info has no option --bogus.");
        assert!(greyed(&w), "Get info runs a line other than the one shown");
        w.get_by_role_and_label(Role::Button, "Get info").hover();
        w.run_steps(4);
        w.get_by_label(CLI_FAULT);
        let heading = w.get_by_label("Command line").rect().top();
        let reset = w
            .get_all_by_label("Reset")
            .find(|b| b.rect().top() > heading - 10.0);
        reset.expect("the command line's Reset").click();
        w.run_steps(2);
        assert!(!greyed(&w));
    }

    #[test]
    fn gw_info_on_the_card_holds_up_only_the_pages_that_use_the_device() {
        let schema = schema();
        let mut app = offline();
        app.engine = Some(no_gw());
        app.pin_ports(vec![greaseweazle("/dev/cu.usbmodem14201", false)]);
        app.probe = Some(running("info"));
        let why = |name| app.why_not(&schema, schema.command(name).unwrap());
        assert_eq!(why("info").as_deref(), Some(ASKING));
        assert_ne!(why("convert").as_deref(), Some(ASKING), "it reads a file");
    }

    #[test]
    fn restart_and_the_gw_path_wait_while_a_job_runs_or_an_update_installs() {
        let mut app = offline();
        app.settings.page = Page::Settings;
        app.settings.engine = Some("/no/such/gw".into());
        app.tool = Some(running("info"));
        let mut w = window(app);
        let greyed = |w: &Harness<'_, App>| {
            let choose = w.get_all_by_label("Choose…").last().expect("the gw row's");
            [
                w.get_by_label("Restart"),
                w.get_by_label("Use the built-in gw"),
                choose,
            ]
            .map(|b| b.accesskit_node().is_disabled())
        };
        assert_eq!(greyed(&w), [true; 3], "a job runs");
        w.state_mut().tool = None;
        w.run_steps(2);
        assert_eq!(greyed(&w), [false; 3]);
        w.state_mut().gw_update = installing();
        w.run_steps(2);
        assert_eq!(greyed(&w), [true; 3], "gw installs an update");
    }

    #[test]
    fn a_check_for_updates_leaves_an_install_under_way() {
        let mut app = offline();
        app.live = true;
        app.engine = Some(Engine {
            origin: Origin::Bundled,
            ..no_gw()
        });
        app.gw_update = installing();
        app.look_for_updates(&egui::Context::default());
        assert!(matches!(app.gw_update, Update::Installing(..)));
    }

    #[test]
    fn a_failed_windows_folder_update_starts_gw_again_and_keeps_its_reason() {
        let mut app = offline();
        app.engine = Some(no_gw());
        let port = greaseweazle("COM3", false);
        app.service = Service::offline(Err(UPDATING.into()));
        app.gw_paused = Some(vec![port.clone()]);
        let (send, answer) = std::sync::mpsc::channel();
        let why = "GitHub did not answer in time.";
        send.send(Err(why.to_owned())).unwrap();
        app.app_update = Update::Installing(answer, "v0.9.1".into());
        app.poll_updates(&egui::Context::default());
        assert!(matches!(&app.app_update, Update::Failed(w) if w == why));
        assert!(app.gw_paused.is_none());
        assert_eq!(app.service.known_ports(), [port], "kept while gw starts");
    }

    fn reads(images: &[&str]) -> Runs {
        let args = images.iter().map(|i| vec!["read".into(), (*i).into()]);
        Runs {
            args: args.collect(),
            ..Runs::default()
        }
    }

    #[test]
    fn a_session_whose_next_run_cannot_start_ends_and_names_no_other_job() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.engine = Some(no_gw());
        app.disk = Some(Job::replay("erase", ""));
        app.begin(&ctx, "read", reads(&["a.adf", "b.adf"]));
        assert!(app.session.is_none());
        assert_eq!(
            app.disk.as_ref().unwrap().part,
            None,
            "the erase is no disk 1"
        );
        assert!(app.notices["read"].starts_with("Could not start gw: "));
    }

    #[test]
    fn a_disk_that_ends_while_quit_asks_ends_its_session_and_the_window_closes() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.session = Some(Session {
            command: "read".into(),
            runs: reads(&["a.adf", "b.adf"]),
            next: 1,
            failed: Vec::new(),
        });
        app.dialog = Some(Dialog::Quit);
        app.disk = Some(Job::replay("read", ""));
        app.ended(&ctx, true);
        assert!(app.session.is_none());
        app.dialogs(&ctx);
        assert!(app.quitting, "no Insert disk 2 of 2");
    }

    /// A file dropped on the window.
    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn a_file_dropped_while_a_dialog_asks_changes_nothing() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.settings.page = Page::Command("write".into());
        let values = app.settings.values.entry("write".into()).or_default();
        values.set("file", "a.img");
        app.dialog = Some(Dialog::Confirm {
            command: "write".into(),
            args: vec!["write".into(), "a.img".into()],
            disks: 1,
        });
        let drop = |app: &mut App| {
            let input = egui::RawInput {
                dropped_files: vec![Arc::new(Dropped("b.img".into()))],
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| app.take_dropped_files(ui.ctx()));
            out.textures_delta.clear(); // no painter here
            app.settings.values["write"].get("file").to_owned()
        };
        assert_eq!(drop(&mut app), "a.img", "the dialog writes a.img");
        app.dialog = None;
        assert_eq!(drop(&mut app), "b.img");
    }

    #[test]
    fn a_stopped_detect_keeps_its_page_until_it_ends_and_chooses_nothing() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.detect_for = Some("read".into());
        app.disk = Some(running(DETECT));
        app.stop();
        assert_eq!(
            app.detect_for.as_deref(),
            Some("read"),
            "its page shows Stopping"
        );
        let found = r#"@ferriteweazle result {"formats": ["ibm.720"], "step": 1}"#;
        let mut job = Job::replay(DETECT, found);
        job.ended = Some((job.started, Outcome::Stopped));
        app.disk = Some(job);
        app.ended(&ctx, true);
        assert_eq!(app.detect_for, None);
        assert!(app.notices.is_empty(), "{:?}", app.notices);
        assert!(!app.settings.values.contains_key("read"));
    }

    #[test]
    fn gw_is_given_the_port_the_card_names() {
        let schema = schema();
        let read = schema.command("read").unwrap();
        let mut app = offline();
        app.pin_ports(vec![
            greaseweazle("COM10", false),
            greaseweazle("COM3", false),
        ]);
        assert_eq!(app.values_for(read).get("device"), "COM10");
        app.settings.device = "COM7".into();
        assert_eq!(app.values_for(read).get("device"), "COM10", "COM7 has gone");
        app.settings.device = "COM3".into();
        assert_eq!(app.values_for(read).get("device"), "COM3");
    }

    #[test]
    fn the_card_runs_no_gw_info_once_the_window_is_closing() {
        let mut app = offline();
        app.live = true;
        app.engine = Some(no_gw());
        app.pin_ports(vec![greaseweazle("/dev/cu.usbmodem14201", false)]);
        app.quitting = true;
        app.poll_probe(&egui::Context::default());
        assert_eq!(app.probed, None);
        assert_eq!(app.probe_failed, None, "gw info did not try to start");
    }

    #[test]
    fn device_info_with_bootloader_leaves_the_card_as_it_was() {
        let ctx = egui::Context::default();
        let mut app = offline();
        let info = |firmware: &str, args: &[&str]| {
            let log = format!("Host Tools: 1.23\nDevice:\n  Firmware: {firmware}");
            let mut job = Job::replay("info", &log);
            job.args = args.iter().map(|a| a.to_string()).collect();
            Some(job)
        };
        app.tool = info("1.6", &["info"]);
        app.ended(&ctx, false);
        app.tool = info("1.0 (Bootloader)", &["info", "--bootloader"]);
        app.ended(&ctx, false);
        let card = app.device.as_ref().and_then(|d| d.get("Firmware"));
        assert_eq!(card, Some("1.6"));
    }

    #[test]
    fn gw_info_the_card_gives_up_on_is_logged_as_timed_out() {
        let mut app = offline();
        app.pin_ports(vec![greaseweazle("/dev/cu.usbmodem14201", false)]);
        let mut probe = Job::replay("info", "Host Tools: 1.23\nDevice:");
        probe.ended = Some((probe.started, Outcome::Stopped));
        app.probe = Some(probe);
        app.poll_probe(&egui::Context::default());
        let last = app.log.lines().last().map(String::as_str);
        assert_eq!(last, Some("Timed out after 0:00."));
        assert_eq!(app.probe_failed.as_deref(), Some("No answer."));
    }

    #[test]
    fn before_gw_describes_itself_the_card_offers_the_drives_gw_does() {
        let ids = |app: &App| {
            app.drives()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };
        let ctx = egui::Context::default();
        let starting = App::offline(&ctx, Settings::default(), Err(String::new()));
        assert_eq!(ids(&starting), ids(&offline()));
        assert_eq!(ids(&starting), ["A", "B", "0", "1", "2", "3"]);
    }

    #[test]
    fn quit_names_the_drive_only_for_a_job_that_runs_it() {
        let mut app = offline();
        app.disk = Some(running("convert"));
        app.dialog = Some(Dialog::Quit);
        let mut w = window(app);
        w.get_by_label("Stop Convert image and quit?");
        w.get_by_label("gw stops, then the window closes.");
        w.state_mut().disk = Some(running("read"));
        w.run_steps(2);
        w.get_by_label("gw stops the drive first, then the window closes.");
    }

    #[test]
    fn an_update_of_the_bootloader_asks_first() {
        let schema = schema();
        let update = schema.command("update").unwrap();
        let ctx = egui::Context::default();
        let mut app = offline();
        app.start(&ctx, update);
        assert!(app.dialog.is_none(), "the main firmware updates at once");
        let values = app.settings.values.entry("update".into()).or_default();
        values.set("bootloader", command::ON);
        app.start(&ctx, update);
        let asks = |args: &Vec<String>| args.contains(&"--bootloader".to_owned());
        assert!(matches!(&app.dialog, Some(Dialog::Confirm { args, .. }) if asks(args)));
        let w = window(app);
        w.get_by_label("Update the bootloader?");
        w.get_by_label(
            "If the flash fails, the Greaseweazle may need reflashing with a programming adapter.",
        );
        w.get_by_role_and_label(egui::accesskit::Role::Button, "Update");
    }

    #[test]
    fn stopping_an_update_warns_what_a_flash_stopped_part_way_leaves() {
        let firmware =
            "A flash stopped part way leaves the firmware erased until Update runs again.";
        let bootloader =
            "A bootloader flash stopped part way may need reflashing with a programming adapter.";
        for (args, warning) in [
            (&["update"][..], firmware),
            (&["update", "--bootloader"], bootloader),
        ] {
            let mut app = offline();
            app.settings.page = Page::Command("update".into());
            let mut job = running("update");
            job.args = args.iter().map(|a| a.to_string()).collect();
            app.tool = Some(job);
            let mut w = window(app);
            w.get_by_role_and_label(egui::accesskit::Role::Button, "Stop")
                .hover();
            // Past the tooltip's delay.
            w.run_steps(4);
            w.get_by_label("Stop gw.");
            w.get_by_label(warning);
            w.event(egui::Event::PointerGone);
            w.state_mut().dialog = Some(Dialog::Quit);
            w.run_steps(2);
            w.get_by_label("Stop Update firmware and quit?");
            w.get_by_label("gw stops, then the window closes.");
            w.get_by_label(warning);
        }
        let mut app = offline();
        app.disk = Some(running("read"));
        app.dialog = Some(Dialog::Quit);
        let w = window(app);
        assert!(
            w.query_by_label(firmware).is_none(),
            "a read flashes nothing"
        );
    }

    #[test]
    fn write_and_erase_ask_about_the_drive_the_card_shows() {
        let mut schema = schema();
        let drives = schema.commands.iter_mut().flat_map(|c| &mut c.args);
        for arg in drives.filter(|a| a.dest == "drive") {
            arg.default = Some("B".into());
        }
        let ctx = egui::Context::default();
        let mut app = App::offline(&ctx, Settings::default(), Ok(schema));
        app.dialog = Some(Dialog::Confirm {
            command: "erase".into(),
            args: Vec::new(),
            disks: 1,
        });
        window(app).get_by_label("Erase the disk in drive B?");
    }

    #[test]
    fn a_log_that_cannot_be_saved_says_why() {
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml/gw.log");
        let why = save_log(&file, &["Done in 0:01.".into()]).expect("a file holds no folder");
        let start = format!("Could not save {}: ", file.display());
        assert!(why.starts_with(&start), "{why}");
    }

    #[test]
    fn the_presets_menu_reads_its_folder_once_while_it_is_open() {
        let folder = std::env::temp_dir().join(format!("fw-menu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let preset = Preset {
            command: "read".into(),
            ..Preset::default()
        };
        presets::save(&folder, "First", &preset).unwrap();
        let mut app = offline();
        app.settings.presets_folder = Some(folder.clone());
        let mut w = window(app);
        w.get_by_label("Presets").click();
        w.run();
        w.get_by_label("First");
        presets::save(&folder, "Second", &preset).unwrap();
        w.run();
        assert!(w.query_by_label("Second").is_none(), "read again");
        w.get_by_label("Presets").click();
        w.run();
        w.get_by_label("Presets").click();
        w.run();
        w.get_by_label("Second");
        std::fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn a_failed_read_says_it_kept_no_image_and_leaves_its_log_where_the_image_would_be() {
        let dir = std::env::temp_dir().join(format!("fw-failed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("Game.img");
        let mut app = offline();
        app.settings.save_logs = true;
        let mut job = Job::replay(
            "read",
            "Reading c=0-81:h=0-1 revs=3\n\
             T0.0: IBM MFM (18/18 sectors) from Raw Flux (1 flux in 200.00ms)\n\
             Command Failed: GetFluxStatus: No Index",
        );
        job.args = vec!["read".into(), "--revs=3".into(), path(&image)];
        job.output = Some(image.clone());
        app.disk = Some(job);
        app.ended(&egui::Context::default(), true);
        let note = left_behind(app.disk.as_ref().unwrap());
        assert_eq!(note, Some("Failed: no image kept."), "gw deleted it");
        let log = std::fs::read_to_string(dir.join("Game.img.log")).expect("the log");
        let lines: Vec<&str> = log.lines().collect();
        let command = format!("gw read --revs=3 {}", command::quote(&path(&image)));
        assert_eq!(lines.first(), Some(&command.as_str()), "{log}");
        assert!(lines.contains(&"Command Failed: GetFluxStatus: No Index"));
        assert_eq!(lines.last(), Some(&"Failed after 0:00."));
        std::fs::remove_dir_all(&dir).ok();
    }

    fn path(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn the_log_shows_gws_errors_in_red_and_its_retries_and_warnings_in_orange() {
        let p = &theme::DARK;
        for line in [
            "ERROR: Device is in Firmware Update Mode",
            "ERROR: USB write data garbled (Host -> Device)",
            "** UPDATE FAILED: Please retry!",
            "Traceback (most recent call last):",
            "Command Failed: GetFluxStatus: No Index",
        ] {
            assert_eq!(log_colour(line, None, p), Some(p.bad), "{line}");
        }
        let message = log_colour("Failed to verify Track 3.0", Some("** FATAL ERROR:"), p);
        assert_eq!(message, Some(p.bad), "the fatal error's message");
        for line in [
            "T0.1: Writing Track (Verify Failure: Retry #1)",
            "T1.0: IBM MFM (17/18 sectors) from Raw Flux (1 flux in 200.00ms) (Retry #1.1)",
            "** SKIPPING UPDATE:",
            "** Unplug device and remove the Update Jumper",
        ] {
            assert_eq!(log_colour(line, None, p), Some(p.partial), "{line}");
        }
        let advice = " - The only available action is \"gw update\"";
        let before = Some("ERROR: Device is in Firmware Update Mode");
        assert_eq!(log_colour(advice, before, p), None);
    }
}

//! The window: a sidebar, a page per gw command, and the disk's status beside it.

use crate::command::{self, Values};
use crate::device::{self, DeviceInfo, Kind, adafruit};
use crate::diskmap;
use crate::form::{self, Form, Output};
use crate::job::{DETECT, Job, Outcome, SessionLog};
use crate::presets::{self, Preset};
use crate::progress::Progress;
use crate::schema::{Command, Port, Schema};
use crate::service::{Load, Repaint, Service};
use crate::theme::{self, Palette};
use crate::tools::{self, Origin, Tools};
use crate::udev;
use crate::update::{self, Install, Update};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Layout, Margin, RichText, Sense,
    Stroke, TextEdit, TextStyle, Theme, Ui, UserAttentionType, Vec2, ViewportCommand, pos2, vec2,
};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// How long the device card waits for `gw info`.
const INFO_TIMEOUT: Duration = Duration::from_secs(12);

/// Commands in the sidebar, by section; any a newer gw adds go under Other.
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
    ("delays", "Delays", "Get delays"),
    ("pin get", "Read pin", "Read pin"),
    ("pin set", "Set pin", "Set pin"),
    ("reset", "Reset", "Reset"),
    ("bandwidth", "USB bandwidth", "Measure"),
    (DETECT, "Detect disk format", "Detect"),
];

/// Page descriptions in place of gw's: gw describes pin get in pin set's words.
const ABOUTS: &[(&str, &str)] = &[("pin get", "Read the level of a floppy interface pin.")];

/// Commands that ask first, and what they do to the disk.
const DESTRUCTIVE: &[(&str, &str)] = &[
    ("write", "Tracks on the disk will be erased and replaced."),
    ("erase", "All tracks will be erased from the disk."),
];

/// Why a command that uses the device cannot run.
const NO_DEVICE: &str = "Connect a Greaseweazle.";
/// Why a copy built from source cannot update itself.
const FROM_SOURCE: &str = "Needs a copy installed from a release.";
/// Why Detect greys for a gw with no Python the bridge can run in.
const STANDALONE_DETECT: &str = "Standalone Greaseweazle Tools cannot run Detect.";
/// Why Restart and Update grey when no gw is found.
const NOT_FOUND: &str = "Unable to load Greaseweazle Tools.";
/// When no gw is found, built in, installed or chosen.
const NO_GW: &str = "Unable to load Greaseweazle Tools, update the path in Settings.";
/// Why nothing that uses an Adafruit RP2040 can start, by whether a port is chosen.
const NO_ADAFRUIT: &str = "Select the Adafruit RP2040's serial port.";
const GONE_ADAFRUIT: &str = "Connect the Adafruit RP2040.";
/// Why nothing new can start while a job runs.
const BUSY: &str = "Wait for the running job to complete.";
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

const REPO: &str = "https://github.com/hobbo91/Ferriteweazle";
const GW_REPO: &str = "https://github.com/keirf/greaseweazle";
/// gw's guide to setting up a Greaseweazle, its drives and its cables.
const GW_GUIDE: &str = "https://github.com/keirf/greaseweazle/wiki/Getting-Started";
const COFFEE: &str = "https://buymeacoffee.com/hobbo91";

/// The window as it opens, in points: wide enough for Disk format to show
/// "Sequential Circuits · sci.prophet" whole, and as tall as 21-point squares
/// need, which fits a 1920x1080 screen at 125% on Windows 11.
pub const WINDOW: egui::Vec2 = egui::vec2(1050.0, 773.0);
/// The smallest window, in points: fits a 1024 by 600 screen, or 1366 by 768 at 125%,
/// beside a taskbar.
pub const SMALLEST: egui::Vec2 = egui::vec2(880.0, 520.0);
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
/// The Presets menu's width, in points: longer names are cut in the middle.
const PRESETS_MENU: f32 = 240.0;
/// How long a theme chosen in Settings takes to fade in, in seconds.
const FADE_TIME: f32 = 0.25;
/// The most of a frame the fade counts, in seconds, so a stall cannot skip it.
const FADE_STEP: f32 = 1.0 / 30.0;
/// Frames to wait for the old theme's screenshot before changing at once.
const FADE_WAIT: u32 = 8;
/// A log line's height, in points.
const LOG_LINE: f32 = 18.0;
/// The drawer's height, margins included: the command line's, and the log's at first.
const DRAWER: f32 = 124.0;
/// A job's rows above the map, each on one line. The map's budget counts
/// them with no job too, so its squares keep their size as a job starts.
const JOB_ROWS: f32 = 101.0;
/// Height the log leaves the page above it, however far it is dragged.
const LOG_ROOM: f32 = 260.0;
/// How long a drawer takes to slide open or shut, in seconds.
const DRAWER_TIME: f32 = 0.2;
/// How far past its least height the log must be dragged to shut, in points.
const LOG_BUMP: f32 = 40.0;
/// The status pane's strip for its scroll bar, taken from its right margin.
const STATUS_BAR: i8 = 10;
/// The page's strip for its scroll bar, taken from its right margin: the page
/// ends as far from the status pane as from the sidebar.
const PAGE_BAR: i8 = 20;

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

/// The choices made in the window; the drive, device, gw and theme are kept between runs.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub page: Page,
    pub theme: theme::Choice,
    /// A Python or `gw` to use instead of the one found automatically.
    pub tools: Option<PathBuf>,
    /// Empty for gw's own choice.
    pub device: String,
    /// The type of device the card drives.
    pub kind: Kind,
    pub drive: String,
    /// Passes gw's `--bt` for Python tracebacks on errors.
    pub backtrace: bool,
    /// Saves the gw command and its output where a job puts its image, as `name.ext.log`.
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

impl Settings {
    /// The device's port among `ports`, as chosen_port picks it.
    fn port<'p>(&self, ports: &'p [Port]) -> Option<&'p Port> {
        chosen_port(ports, &self.device, self.kind)
    }
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
    /// A job that asks first, or the first of a session of `disks`.
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
        /// The name typed for the next disk, when the set asks for names.
        name: Option<String>,
        /// The next disk's name if none is typed: the page's, for the first.
        default: String,
    },
    SavePreset {
        command: String,
        name: String,
        description: String,
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
    asked: Option<(theme::Choice, u32)>,
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
    /// Disks of the set read before these, when a read carries a set on.
    before: usize,
    /// Each disk's name is asked for with it; `images` gathers them.
    ask_names: bool,
}

impl Runs {
    /// Run `run`'s disk number, counting from 1, and the set's last.
    fn number(&self, run: usize) -> (usize, usize) {
        (self.before + run + 1, self.before + self.args.len())
    }
}

/// The longest preset description, in characters.
const DESCRIPTION_LIMIT: usize = 120;

/// A page's Presets menu while it is open, so the folder is read once, not every frame.
struct PresetsMenu {
    page: String,
    /// The page's presets, by name: each one's name, file and description.
    saved: Vec<(String, PathBuf, String)>,
}

/// A preset as its page last loaded or saved it, and whether Reset goes back to it.
struct Applied {
    path: PathBuf,
    preset: Preset,
    /// Reset puts the page back to this preset, not to gw's defaults.
    reset_to: bool,
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
    tools: Option<Tools>,
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
    /// The real app, not a test window: it keeps the drive, device, gw and theme in their files,
    /// runs gw info on each Greaseweazle that appears, and checks GitHub for updates.
    live: bool,
    /// The drive as last kept in drive_file().
    kept_drive: String,
    /// The device type and port as last kept in device_file().
    kept_device: (Kind, String),
    /// The gw chosen in Settings as last kept in tools_file().
    kept_tools: Option<PathBuf>,
    /// The theme as last kept in theme_file().
    kept_theme: theme::Choice,
    /// Classic in the accent last chosen for it while the app runs: Classic
    /// itself (teal) or Blue.
    classic: theme::Choice,
    /// The delays gw delays last reported, and the port of the Greaseweazle
    /// they are of: kept while another run of it goes on.
    delays: Option<(String, BTreeMap<&'static str, String>)>,
    /// Detect's note on its page, and the format it chose: the note goes once
    /// the page takes another.
    found_note: Option<(String, String, String)>,
    /// By page, the format its settings fit.
    format_fits: BTreeMap<String, form::FormatFit>,
    /// Where the window's size is kept: size_file() in the real app.
    pub size_file: Option<PathBuf>,
    /// The window's size as last kept, or as it opened.
    kept_size: egui::Vec2,
    /// A new size of the window, and when it was first seen: kept once it
    /// has stayed SIZE_SETTLE, so a drag writes the file once.
    new_size: Option<(egui::Vec2, Instant)>,
    /// Whether a kept size too large for the screen has been fitted to it.
    fitted: bool,
    gw_update: Update,
    app_update: Update,
    /// How this copy was installed, and why it cannot update itself if it cannot.
    copy: Option<Install>,
    stuck: Option<&'static str>,
    /// The release whose banner was dismissed, as kept in dismissed_file().
    dismissed: Option<String>,
    /// The Presets menu, while it is open.
    presets: Option<PresetsMenu>,
    /// By page, the preset it last loaded or saved.
    applied: BTreeMap<String, Applied>,
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
        let (kind, port) = kept_device(&device_file());
        let tools = kept_tools(&tools_file());
        let theme = kept_theme(&theme_file());
        let settings = Settings {
            drive: drive.clone(),
            kind,
            device: port.clone(),
            tools: tools.clone(),
            theme,
            ..Settings::default()
        };
        let mut app = App::with_settings(&cc.egui_ctx, settings);
        app.live = true;
        app.kept_drive = drive;
        app.kept_device = (kind, port);
        app.kept_tools = tools;
        app.kept_theme = theme;
        app.copy = Install::this();
        app.stuck = app.copy.as_ref().map_or(Some(FROM_SOURCE), Install::stuck);
        app.dismissed = kept_dismissed(&dismissed_file());
        app.kept_size = opening_size();
        app.size_file = Some(size_file());
        if let Some(copy) = &app.copy {
            update::tidy(copy);
        }
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

    /// A window over a known schema, with no Greaseweazle Tools to run anything.
    pub fn offline(ctx: &egui::Context, settings: Settings, schema: Result<Schema, String>) -> App {
        theme::install(ctx);
        theme::apply(ctx, settings.theme);
        let known = schema.as_ref().ok().cloned().map(Arc::new);
        let classic = match settings.theme {
            theme::Choice::Blue => theme::Choice::Blue,
            _ => theme::Choice::Classic,
        };
        App {
            settings,
            tools: None,
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
            kept_device: (Kind::Greaseweazle, String::new()),
            kept_tools: None,
            kept_theme: theme::Choice::System,
            classic,
            delays: None,
            found_note: None,
            format_fits: BTreeMap::new(),
            size_file: None,
            kept_size: WINDOW,
            new_size: None,
            fitted: false,
            gw_update: Update::default(),
            app_update: Update::default(),
            copy: None,
            stuck: Some(FROM_SOURCE),
            dismissed: None,
            presets: None,
            applied: BTreeMap::new(),
            gw_paused: None,
            logo: None,
            fade: Fade::default(),
            desktop_theme: None,
            framed: None,
            drawn: None,
            udev_rule: tools::udev_rule(),
            install: RuleInstall::Idle,
        }
    }

    fn connect(&mut self, ctx: &egui::Context) {
        self.schema = None;
        self.tools = Tools::find(self.settings.tools.as_deref());
        // A new gw finds the same device, so the window keeps it meanwhile.
        let ports = self.service.known_ports().to_vec();
        self.service = match (&self.tools, &self.settings.tools) {
            (Some(tools), _) => Service::start(tools, repaint(ctx)),
            (None, Some(path)) if path.exists() => Service::offline(Err(format!(
                "{} is not Greaseweazle Tools.",
                path.display()
            ))),
            (None, _) => Service::offline(Err(NO_GW.into())),
        };
        self.service.seed_ports(ports);
        self.look_for_updates(ctx);
    }

    /// Asks GitHub for newer releases of the built-in gw and of this app.
    fn look_for_updates(&mut self, ctx: &egui::Context) {
        let Some(tools) = self.tools.as_ref().filter(|_| self.live) else {
            return;
        };
        // An install under way keeps its answer.
        if tools.origin == Origin::Bundled && !installing(&self.gw_update) {
            self.gw_update = Update::check(tools, None, repaint(ctx));
        }
        // A standalone gw has no Python to ask GitHub with; the built-in one may.
        if let Some(python) = tools.with_python().filter(|_| self.copy.is_some())
            && !installing(&self.app_update)
        {
            self.app_update = Update::check(&python, Some(update::APP_REPO), repaint(ctx));
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
        let was_installing = installing(&self.app_update);
        if self.app_update.poll(Some(env!("CARGO_PKG_VERSION")))
            && let (Update::Latest(tag), Some(install)) = (&self.app_update, &self.copy)
        {
            install.relaunch(update::bare(tag));
            ctx.send_viewport_cmd(ViewportCommand::Close);
        } else if matches!(self.app_update, Update::Failed(_))
            && let Some(tools) = &self.tools
            && let Some(ports) = self.gw_paused.take()
        {
            // The update stopped gw. Not connect(): its check for updates
            // would drop the reason the update failed.
            self.service = Service::start(tools, repaint(ctx));
            self.service.seed_ports(ports);
        }
        // The page shown says why an install failed.
        if was_installing
            && let (Update::Failed(why), Page::Command(page)) =
                (&self.app_update, &self.settings.page)
        {
            let text = format!("Unable to install the update: {why}");
            self.notices.insert(page.clone(), text);
        }
    }

    /// Starts installing the newer Ferriteweazle that GitHub has.
    fn install_app(&mut self, ctx: &egui::Context) {
        let (Update::Newer(tag), Some(tools), Some(install)) = (
            &self.app_update,
            self.tools.as_ref().and_then(Tools::with_python),
            self.copy.clone(),
        ) else {
            return;
        };
        let tag = tag.clone();
        if cfg!(windows) && matches!(install, Install::Folder(_)) {
            // Windows will not move the data folder while gw runs from it.
            self.gw_paused = Some(self.service.known_ports().to_vec());
            self.service = Service::offline(Err(UPDATING.into()));
        }
        self.app_update = Update::app(&tools, install, &tag, repaint(ctx));
    }

    /// Offers the newer Ferriteweazle GitHub has, on every page, where this
    /// copy can update itself: Update installs it, Dismiss hides it until a
    /// newer one.
    fn update_banner(&mut self, ui: &mut Ui) {
        let (text, offered) = match &self.app_update {
            Update::Newer(tag) if self.stuck.is_none() && self.dismissed.as_ref() != Some(tag) => {
                let text = format!("Ferriteweazle {} is available.", update::bare(tag));
                (text, Some(tag.clone()))
            }
            Update::Installing(_, tag) => {
                let text = format!("Installing Ferriteweazle {}\u{2026}", update::bare(tag));
                (text, None)
            }
            _ => return,
        };
        let (can, tip) = self.update_button(&self.app_update, "Ferriteweazle");
        let (mut install, mut dismiss) = (false, false);
        let room = if offered.is_some() { 150.0 } else { 0.0 };
        banner(ui, &text, room, |ui| {
            if offered.is_some() {
                dismiss = ui
                    .small_button("Dismiss")
                    .on_hover_text("Hide this until a newer release.")
                    .clicked();
                install = ui
                    .add_enabled(can, egui::Button::new("Update").small())
                    .on_hover_text(&tip)
                    .on_disabled_hover_text(&tip)
                    .clicked();
            }
        });
        if install {
            self.install_app(ui.ctx());
        }
        if dismiss && let Some(tag) = offered {
            if self.live {
                keep(&dismissed_file(), Some(tag.clone()));
            }
            self.dismissed = Some(tag);
        }
    }

    /// Shows these ports as the connected devices, whatever gw finds, until
    /// gw restarts: for tests and pictures of the window.
    pub fn pin_ports(&mut self, ports: Vec<Port>) {
        self.service.pin_ports(ports);
    }

    /// gw's command line, once gw has described it.
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
        let (kind, port) = &self.kept_device;
        if self.live && (self.settings.kind != *kind || self.settings.device != *port) {
            self.kept_device = (self.settings.kind, self.settings.device.clone());
            keep_device(&device_file(), self.settings.kind, &self.settings.device);
        }
        if self.live && self.settings.tools != self.kept_tools {
            self.kept_tools.clone_from(&self.settings.tools);
            keep_tools(&tools_file(), self.kept_tools.as_deref());
        }
        if self.live && self.settings.theme != self.kept_theme {
            self.kept_theme = self.settings.theme;
            keep_theme(&theme_file(), self.kept_theme);
        }
        self.drop_found_note();
        self.follow_desktop(&ctx);
        self.fade_theme(&ctx);
        self.poll(&ctx);
        self.poll_updates(&ctx);
        self.guard_close(&ctx);
        self.keep_size(&ctx);
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
            right: 28 - PAGE_BAR,
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
                // The page takes up to its form's full width and the status pane the rest,
                // at least STATUS_MIN unless the page would drop below PAGE_MIN. Past the
                // map's widest, the page takes the rest up to its widest form.
                let room = ui.available_width();
                let margins = page.inner_margin.sum().x + f32::from(PAGE_BAR);
                let map = diskmap::width_for(tall - DRAWER - JOB_ROWS)
                    + status_frame.total_margin().sum().x
                    + f32::from(STATUS_BAR);
                let status = (room - form::full_width(ui) - margins)
                    .min(map.max(room - form::widest(ui) - margins))
                    .max(STATUS_MIN)
                    .min(room - PAGE_MIN);
                egui::Panel::right("status")
                    .resizable(false)
                    .exact_size(status)
                    .frame(status_frame)
                    .show(ui, |ui| self.status(ui, &name, tall));
                egui::CentralPanel::default()
                    .frame(page)
                    .show(ui, |ui| self.page(ui, &name));
            }
        }
        self.dialogs(&ctx);
        size_corner(&ctx);
        if p.classic {
            theme::square(&ctx);
        }
    }

    /// Keeps the window's size for the next run once it settles, and fits a
    /// kept size to a smaller screen than it was kept on.
    fn keep_size(&mut self, ctx: &egui::Context) {
        let Some(file) = self.size_file.clone() else {
            return;
        };
        let (whole, monitor) = ctx.input(|i| {
            let v = i.viewport();
            let whole = [v.maximized, v.fullscreen, v.minimized].contains(&Some(true));
            (whole, v.monitor_size)
        });
        let size = ctx.content_rect().size();
        if !self.fitted
            && let Some(monitor) = monitor
        {
            self.fitted = true;
            // Room for a title bar and a taskbar or menu bar.
            let fit = size.min(monitor - vec2(0.0, SCREEN_BARS)).max(SMALLEST);
            if fit != size {
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(fit));
                return;
            }
        }
        if whole || same_size(size, self.kept_size) {
            self.new_size = None;
            return;
        }
        match self.new_size {
            Some((seen, at)) if same_size(seen, size) => {
                let left = SIZE_SETTLE.saturating_sub(at.elapsed());
                if left.is_zero() {
                    save_size(&file, size);
                    self.kept_size = size;
                    self.new_size = None;
                } else {
                    ctx.request_repaint_after(left);
                }
            }
            _ => {
                self.new_size = Some((size, Instant::now()));
                ctx.request_repaint_after(SIZE_SETTLE);
            }
        }
    }

    /// Changes the theme, cross-fading when the window will look different.
    fn choose_theme(&mut self, ctx: &egui::Context, choice: theme::Choice) {
        let looks = |choice| match choice {
            theme::Choice::System => (ctx.system_theme())
                .unwrap_or_else(|| ctx.options(|o| o.fallback_theme))
                .into(),
            choice => choice,
        };
        let same = looks(self.settings.theme) == looks(choice);
        self.settings.theme = choice;
        if same {
            theme::apply(ctx, choice);
            self.fade.asked = None;
        } else {
            let shot = egui::UserData::new(FadeShot);
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(shot));
            self.fade.asked = Some((choice, 0));
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
        if let Some((choice, waited)) = &mut self.fade.asked {
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
                theme::apply(ctx, *choice);
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
            // Relist the ports to show the access udev has granted.
            self.service.refresh_ports();
            self.install = RuleInstall::Done(done);
        }
        if self.quitting && self.running().is_none() && self.probe.is_none() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// Keeps the device card current, and gives up on a device that does not answer.
    fn poll_probe(&mut self, ctx: &egui::Context) {
        let mut named = None;
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
            let info = device::parse(&probe.log);
            if info.is_some() {
                self.device.clone_from(&info);
            }
            if !probe.running() {
                let port = self.settings.port(self.service.known_ports());
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
                named = info.as_ref().and_then(named_device);
                self.probe_failed = match (info, probe.outcome()) {
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
        if let Some((port, kind)) = named {
            self.follow(&port, kind);
        }
        let port = self.found_port().map(|p| (p.device.clone(), p.denied));
        if port.is_none() {
            self.probed = None;
            self.device = None;
            self.probe_failed = None;
        } else if self.live && !self.quitting && port != self.probed {
            self.ask_device(ctx);
        }
        // Unplugged, a Greaseweazle loses the delays it was given.
        if let Some((device, _)) = &self.delays
            && !self
                .service
                .known_ports()
                .iter()
                .any(|p| p.device == *device)
        {
            self.delays = None;
        }
    }

    /// The Greaseweazle the sidebar shows.
    fn found_port(&mut self) -> Option<&Port> {
        self.service.ports();
        self.settings.port(self.service.known_ports())
    }

    /// Whether the chosen port is there, open to this account, and did not
    /// fail its last gw info: the device card's dot is green.
    fn answering(&self) -> bool {
        let port = self.settings.port(self.service.known_ports());
        port.is_some_and(|p| !p.denied) && self.probe_failed.is_none()
    }

    /// Whether the sidebar shows a Greaseweazle, as last listed.
    fn connected(&self) -> bool {
        self.settings.port(self.service.known_ports()).is_some()
    }

    /// Why nothing new can start now: a job runs, or gw or this app installs an update.
    fn busy(&self) -> Option<&'static str> {
        if self.running().is_some() {
            Some(BUSY)
        } else if installing(&self.gw_update) || installing(&self.app_update) {
            Some(INSTALLING)
        } else {
            None
        }
    }

    /// The delays the last Get or Set delays reported, if that was of the
    /// card's Greaseweazle: each delay's argument and value.
    fn reported_delays(&self) -> Option<BTreeMap<&'static str, String>> {
        let (device, found) = self.delays.as_ref()?;
        let port = self.settings.port(self.service.known_ports())?;
        (port.device == *device).then(|| found.clone())
    }

    /// Why Detect cannot run on `page` now. On Read it reads the disk in the drive.
    fn cannot_detect(&self, page: &str) -> Option<Cow<'static, str>> {
        if self.tools.as_ref().is_some_and(|e| e.standalone) {
            return Some(STANDALONE_DETECT.into());
        }
        if let Some(why) = self.busy() {
            return Some(why.into());
        }
        match page {
            "read" if self.probe.is_some() => Some(ASKING.into()),
            "read" if !self.connected() => Some(self.no_device()),
            _ => None,
        }
    }

    /// Why the sidebar shows no device: gw's reason when it could not list the
    /// ports, else NO_DEVICE or an Adafruit RP2040's.
    fn no_device(&self) -> Cow<'static, str> {
        match (self.service.ports_error(), self.settings.kind) {
            (Some(why), _) => why.to_owned().into(),
            (None, Kind::Greaseweazle) => NO_DEVICE.into(),
            (None, Kind::Adafruit) if self.settings.device.is_empty() => NO_ADAFRUIT.into(),
            (None, Kind::Adafruit) => GONE_ADAFRUIT.into(),
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
        self.probed = self.found_port().map(|p| (p.device.clone(), p.denied));
        // None of the Device info page's options: --bootloader would switch
        // the device's mode every time.
        let args = self.argv(cmd, &self.device_only(cmd));
        let Some(tools) = &self.tools else { return };
        match Job::start(
            tools,
            self.settings.kind.name(),
            "info",
            args,
            &[],
            repaint(ctx),
        ) {
            Ok(mut job) => {
                self.log.begin(heading(&job), &mut job);
                self.probe = Some(job);
                self.probe_failed = None;
            }
            Err(e) => self.probe_failed = Some(format!("Could not start Greaseweazle Tools: {e}")),
        }
    }

    /// A job has just ended: save the log, and do whatever was waiting on it.
    pub fn ended(&mut self, ctx: &egui::Context, disk: bool) {
        let port = self.settings.port(self.service.known_ports()).cloned();
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
        // gw's message cites its wiki, whose steps are for gw's own install.
        if let Some(error) = &mut job.progress.error
            && error.contains("Could not find SPS/CAPS library")
        {
            let advice = caps_advice(self.tools.as_ref());
            error.push('\n');
            error.push_str(&advice);
            job.log.push(advice);
        }
        self.log.end(job, ending(job));
        let command = job.command.clone();
        let detected = std::mem::take(&mut job.detected);
        let step = job.step;
        match command.as_str() {
            // With --bootloader, gw reports the bootloader's firmware.
            "info" if !job.args.iter().any(|a| a == "--bootloader") => {
                self.device = device::parse(&job.log);
                if let Some((port, kind)) = self.device.as_ref().and_then(named_device) {
                    self.follow(&port, kind);
                }
            }
            // New firmware changes what the device says about itself, and
            // restarts it with its default delays.
            "update" => (self.probed, self.delays) = (None, None),
            "reset" if job.args.iter().any(|a| a == "--delays") => self.delays = None,
            // The drive has the delays typed now, and their fields show them greyed.
            "delays" => {
                if let Some(found) = device::delays(&job.log) {
                    let values = self.settings.values.entry(command.clone()).or_default();
                    for dest in found.keys() {
                        values.set(dest, "");
                    }
                    let device = job.args.iter().find_map(|a| a.strip_prefix("--device="));
                    self.delays = device.map(|d| (d.to_owned(), found));
                }
            }
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
                // As the set numbers its disks, which may start past 1.
                let number = |run: usize| session.runs.number(run);
                self.dialog = Some(Dialog::NextDisk {
                    disk: next.map(|_| number(session.next).0),
                    total: number(0).1,
                    failed: again.map(|_| number(session.next - 1).0),
                    image: session.runs.images.get(session.next).cloned(),
                    command: command.clone(),
                    name: (next.is_some() && session.runs.ask_names).then(String::new),
                    default: String::new(),
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
        if let Some(job) = self.disk.as_mut().filter(|j| j.command == DETECT) {
            job.format = Some(best.clone());
        }
        let values = self.settings.values.entry(page.clone()).or_default();
        form::choose_format(&schema, cmd, values, &mut self.settings.outputs, best);
        // On Write, Detect reads the image, whose step is not the drive's.
        let step = (page != "write").then_some(step);
        let tracks = step.and_then(|step| form::with_step(values.get("tracks"), step));
        let changed = tracks.is_some();
        if let Some(tracks) = tracks {
            values.set("tracks", tracks);
        }
        let note = found_note(&formats, step.unwrap_or(1), changed);
        self.found_note = Some((page.clone(), best.clone(), note.clone()));
        self.notices.insert(page, note);
    }

    /// Detect's note gives way once its page takes another format than the one Detect chose.
    fn drop_found_note(&mut self) {
        let Some((page, format, note)) = &self.found_note else {
            return;
        };
        let shown = self.notices.get(page) == Some(note);
        let chosen = self
            .settings
            .values
            .get(page)
            .map_or("", |v| v.get("format"));
        if shown && chosen != format {
            self.notices.remove(page);
        }
        if !shown || chosen != format {
            self.found_note = None;
        }
    }

    /// gw info found a `kind` on `port`. While that is the port chosen, the
    /// tick follows it, and with it what the pages allow.
    fn follow(&mut self, port: &str, kind: Kind) {
        let chosen = self.settings.port(self.service.known_ports());
        if chosen.is_some_and(|p| p.device == port) && kind != self.settings.kind {
            // An Adafruit RP2040 is only ever the port chosen for it.
            self.settings.device = port.to_owned();
            self.set_kind(kind);
        }
    }

    /// The device type ticked by hand. A port gw info found the other type on
    /// is not this one's, so it gives way rather than undo the tick.
    fn choose_kind(&mut self, kind: Kind) {
        let found = self.device.as_ref().and_then(named_device);
        if found.is_some_and(|(port, k)| k != kind && port == self.settings.device) {
            self.settings.device.clear();
        }
        self.set_kind(kind);
        // gw info again, in the type's name.
        self.probed = None;
    }

    /// Drives `kind` from now on. An Adafruit RP2040 has unit 0 alone, so a
    /// drive it cannot select gives way to gw's default, A.
    fn set_kind(&mut self, kind: Kind) {
        self.settings.kind = kind;
        let drive = self.drive();
        if kind == Kind::Adafruit && !adafruit::DRIVES.contains(&drive.as_str()) {
            self.settings.drive = String::new();
        }
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
                let version = self.listed.as_ref().map(|s| s.gw());
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
        let kind = self.settings.kind;
        let mut ask = false;
        let mut access = None;
        let mut chose = None;
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
                        (Some(port), None) => match self.settings.kind {
                            Kind::Greaseweazle => port.name.as_deref().unwrap_or("Greaseweazle"),
                            Kind::Adafruit => {
                                let room =
                                    ui.available_width() - REFRESH - ui.spacing().item_spacing.x;
                                let font = egui::TextStyle::Body.resolve(ui.style());
                                let full = ui.painter().layout_no_wrap(
                                    adafruit::NAME.into(),
                                    font,
                                    Color32::PLACEHOLDER,
                                );
                                match full.size().x <= room {
                                    true => adafruit::NAME,
                                    false => adafruit::SHORT,
                                }
                            }
                        },
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
                } else if let Some(why) = self.service.ports_error() {
                    ui.label(RichText::new(why).small().color(p.bad));
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
                        for (option, tip) in [
                            (
                                Kind::Greaseweazle,
                                "A Greaseweazle: Greaseweazle Tools finds its port.",
                            ),
                            (
                                Kind::Adafruit,
                                "Adafruit's Greaseweazle-compatible firmware: select its port.",
                            ),
                        ] {
                            if ticked(ui, kind == option, option.name())
                                .on_hover_text(tip)
                                .clicked()
                            {
                                chose = Some(option);
                            }
                        }
                        ui.add_space(4.0);
                        ui.label(RichText::new("Serial Port").small().weak());
                        if ports.is_empty() {
                            ui.label(RichText::new("No ports found.").weak());
                        }
                        for port in ports {
                            // gw names only a Greaseweazle; an Adafruit RP2040
                            // is known by the name its USB gives.
                            let named = port.score > 0 || kind == Kind::Adafruit;
                            let text = match (&port.name, named) {
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
                    .on_hover_text("The device, and the port it is on.");
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
                        let possible =
                            kind == Kind::Greaseweazle || adafruit::DRIVES.contains(&id.as_str());
                        if ui
                            .add_enabled(possible, button)
                            .on_hover_text(about.as_str())
                            .on_disabled_hover_text(adafruit::OPTION)
                            .clicked()
                        {
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
        if let Some(kind) = chose {
            self.choose_kind(kind);
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

    /// The drive gw uses: the one chosen, else gw's default.
    fn drive(&self) -> String {
        match self.settings.drive.as_str() {
            "" => self.default_drive(),
            drive => drive.to_owned(),
        }
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
        let device = self.settings.port(self.service.known_ports());
        let device = device.map_or("", |p| p.device.as_str());
        for (dest, value) in [("device", device), ("drive", self.settings.drive.as_str())] {
            if cmd.arg(dest).is_some() {
                values.set(dest, value);
            }
        }
        // Options its firmware cannot carry out stay off, as the page shows them.
        if self.settings.kind == Kind::Adafruit {
            for a in cmd
                .args
                .iter()
                .filter(|a| adafruit::option(&cmd.name, &a.dest))
            {
                values.set(&a.dest, "");
            }
        }
        // A definitions file goes to gw only with one of its own formats.
        let custom = self.service.known_custom_formats(values.get("diskdefs"));
        if !custom.iter().any(|f| f == values.get("format")) {
            values.set("diskdefs", "");
        }
        // The Overwrite question stands in for gw's -n, which would refuse once it is answered.
        if form::has_output(&cmd.name) {
            values.set("no_clobber", "");
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

    /// Fits the page's settings to its format: see `form::fit_format`.
    fn fit_format(&mut self, schema: &Schema, cmd: &Command) -> bool {
        let values = self.settings.values.entry(cmd.name.clone()).or_default();
        let outputs = &mut self.settings.outputs;
        let fit = self.format_fits.entry(cmd.name.clone()).or_default();
        form::fit_format(&mut self.service, schema, cmd, values, outputs, fit)
    }

    /// What a detect job needs: the drive and how the page reads it, or the
    /// image and how the page takes it in. A write's drive settings are for
    /// the disk it writes, not its image.
    fn detect_args(&self, cmd: &Command) -> Vec<String> {
        let mut values = self.values_for(cmd);
        // Detection tries a definitions file's formats beside gw's own.
        if let Some(page) = self.settings.values.get(&cmd.name) {
            values.set("diskdefs", page.get("diskdefs"));
        }
        let dests: &[&str] = match cmd.name.as_str() {
            "write" => &["diskdefs"],
            _ => &[
                "device",
                "drive",
                "diskdefs",
                "tracks",
                "densel",
                "gen_tg43",
                "fake_index",
                "hard_sectors",
                "reverse",
                "adjust_speed",
            ],
        };
        let mut with = Values::default();
        for dest in dests {
            with.set(dest, values.get(dest));
        }
        if cmd.name != "read" {
            let dest = if cmd.arg("in_file").is_some() {
                "in_file"
            } else {
                "file"
            };
            with.set(dest, image_path(values.get(dest)));
        }
        // As gw spells them, less the command's name.
        command::argv(cmd, &with).split_off(1)
    }

    fn page(&mut self, ui: &mut Ui, name: &str) {
        let Some(schema) = self.schema.clone() else {
            return self.not_ready(ui);
        };
        let Some(cmd) = schema.command(name) else {
            ui.heading(title(name));
            ui.label(format!("{} has no {name} command.", schema.tools()));
            return;
        };
        let width = form::form_width(ui, ui.available_width() - f32::from(PAGE_BAR));
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
                        right(ui, |ui| {
                            self.presets_menu(ui, name);
                            self.reset_button(ui, name);
                        });
                    });
                    let mut about = match ABOUTS.iter().find(|(c, _)| *c == name) {
                        Some((_, about)) => (*about).to_owned(),
                        None => form::sentence(&cmd.about),
                    };
                    // gw's own words name the Greaseweazle.
                    if self.settings.kind == Kind::Adafruit {
                        about = about.replace("Greaseweazle", adafruit::NAME);
                    }
                    ui.label(RichText::new(about).weak());
                    self.notice_bar(ui, name);
                    self.update_banner(ui);
                    ui.add_space(10.0);
                    let reported = (name == "delays").then(|| self.reported_delays()).flatten();
                    let values = self.settings.values.entry(name.to_owned()).or_default();
                    let form = Form {
                        schema: &schema,
                        cmd,
                        values,
                        outputs: &mut self.settings.outputs,
                        service: &mut self.service,
                        cannot_detect: cannot_detect.as_deref(),
                        adafruit: self.settings.kind == Kind::Adafruit,
                        standalone: self.tools.as_ref().is_some_and(|t| t.standalone),
                        reported: reported.as_ref(),
                    };
                    // Under ids of its own, so a banner coming above it leaves a box focused.
                    let ids = egui::UiBuilder::new().id(Id::new(("form", name)));
                    let action = ui.scope_builder(ids, |ui| form.show(ui)).inner;
                    // After the form, which names the page's images.
                    if self.fit_format(&schema, cmd) {
                        ui.ctx().request_repaint();
                    }
                    // Delays the drive reports show in their fields, so only
                    // a failure needs a Result.
                    let quiet = |j: &Job| {
                        j.command == "delays" && (j.running() || device::delays(&j.log).is_some())
                    };
                    if let Some(job) = self
                        .tool
                        .as_ref()
                        .filter(|j| j.command == name && !quiet(j))
                    {
                        ui.add_space(18.0);
                        (install, unsaved) = result(ui, job, self.refused(job));
                    }
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
            // The page's notice goes while Detect looks again.
            self.notices.remove(name);
            self.detect_for = Some(name.to_owned());
            let args = self.detect_args(cmd);
            self.run(ui.ctx(), DETECT, args);
        }
    }

    fn notice_bar(&mut self, ui: &mut Ui, page: &str) {
        let Some(notice) = self.notices.get(page) else {
            return;
        };
        let mut dismiss = false;
        banner(ui, notice, 70.0, |ui| {
            dismiss = ui.small_button("Dismiss").clicked();
        });
        if dismiss {
            self.notices.remove(page);
        }
    }

    fn not_ready(&mut self, ui: &mut Ui) {
        ui.add_space(60.0);
        ui.vertical_centered(|ui| match self.service.schema.error() {
            None => {
                ui.spinner();
                ui.label(RichText::new("Starting Greaseweazle…").weak());
            }
            Some(e) => {
                ui.label(RichText::new("Device not ready").size(18.0).strong());
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
                        true => "Stop Greaseweazle Tools and the drive's motor.",
                        false => "Stop Greaseweazle Tools.",
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
                            .is_some_and(|o| o.first_disk() < o.disks);
                    let batch = self
                        .settings
                        .values
                        .get(&cmd.name)
                        .is_some_and(|v| form::batch_input(cmd, v).is_some());
                    // gw delays shows the drive's delays, after setting any typed.
                    let sets = cmd.name == "delays"
                        && self.settings.values.get(&cmd.name).is_some_and(|v| {
                            let own = |dest: &str| !form::GLOBAL.contains(&dest);
                            cmd.args.iter().any(|a| own(&a.dest) && v.on(&a.dest))
                        });
                    let label = match (several, batch, cmd.name.as_str()) {
                        (true, _, _) => "Read disks",
                        (_, true, "write") => "Write disks",
                        (_, true, _) => "Convert images",
                        _ if sets => "Set delays",
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
                    "Show command line.",
                    "Hide command line.",
                ),
                (
                    Drawer::Log,
                    "Log",
                    "Show Greaseweazle Tools' output.",
                    "Hide Greaseweazle Tools' output.",
                ),
            ] {
                ui.add_space(6.0);
                let open = self.settings.drawer == Some(drawer);
                let button = egui::Button::new(text)
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
            .filter(|a| !form::GLOBAL.contains(&a.dest.as_str()) && !output(&a.dest))
            .filter(|a| form::batch_input(cmd, values) != Some(a.dest.as_str()))
            .map(|a| form::label(a).to_lowercase())
            .collect();
        let device = uses_device(schema, &cmd.name);
        if self.tools.is_none() {
            Some(NO_GW.to_owned())
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
        } else if let Some(why) = (device && self.settings.kind == Kind::Adafruit)
            .then(|| adafruit::command(&cmd.name))
            .flatten()
        {
            // The board never runs these, connected or not.
            Some(why.to_owned())
        } else if device && !self.connected() {
            Some(self.no_device().into_owned())
        } else if let Some(why) = device.then(|| self.adafruit_fault(cmd, values)).flatten() {
            Some(why.to_owned())
        } else if !missing.is_empty() {
            Some(format!("Select the {} first.", missing.join(" and ")))
        } else {
            let outputs = &self.settings.outputs;
            self.diskdefs_fault(values)
                .or_else(|| form::missing_image(cmd, values))
                .or_else(|| form::blocked(schema, cmd, values, outputs, &self.service))
                .map(str::to_owned)
        }
    }

    /// Why the Adafruit RP2040 cannot run the page as it is set, when it is the device.
    fn adafruit_fault(&self, cmd: &Command, values: &Values) -> Option<&'static str> {
        if self.settings.kind != Kind::Adafruit {
            return None;
        }
        let drive = self.drive();
        if cmd.arg("drive").is_some() && !adafruit::DRIVES.contains(&drive.as_str()) {
            return Some("The Adafruit RP2040 has one drive: select A or 0.");
        }
        let pin = values.get("pin").trim().parse::<u32>().ok();
        match cmd.name.as_str() {
            "pin get" if pin.is_some_and(|p| p != adafruit::GET_PIN) => {
                Some("The Adafruit RP2040 reads pin 26 alone.")
            }
            "pin set" if pin.is_some_and(|p| p != adafruit::SET_PIN) => {
                Some("The Adafruit RP2040 sets pin 2 alone.")
            }
            _ => self
                .last_cylinder(cmd, values)
                .filter(|&c| c > adafruit::LAST_CYLINDER)
                .map(|_| "The Adafruit RP2040 reaches cylinders 0 to 79."),
        }
    }

    /// The furthest cylinder the page would seek to, where it says so plainly:
    /// Seek's cylinder, Clean's last, or a simple track list over the chosen
    /// format. The bridge refuses any other past the Adafruit RP2040's last.
    fn last_cylinder(&self, cmd: &Command, values: &Values) -> Option<u32> {
        let number = |dest: &str| {
            let value = match values.get(dest) {
                "" => cmd.arg(dest)?.default.as_deref()?,
                value => value,
            };
            value.trim().parse::<u32>().ok()
        };
        match cmd.name.as_str() {
            "seek" => number("cylinder"),
            // gw clean goes as far as cyls - 1.
            "clean" => number("cyls").map(|c| c.saturating_sub(1)),
            _ if cmd.arg("tracks").is_some() => {
                let format = values.get("format");
                let diskdefs = form::known_diskdefs(&self.service, values, format);
                let cyls = (!format.is_empty())
                    .then(|| self.service.known_format_info(diskdefs, format)?.ready())
                    .flatten()
                    .map(|i| i.cyls);
                form::last_cylinder(values.get("tracks"), cyls)
            }
            _ => None,
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
            // gw reads the file only for one of its own formats.
            Some(Load::Ready(d)) if d.failed.iter().any(|f| f == values.get("format")) => {
                Some("The format's definition has errors. See Advanced options.")
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
            let (disk, total) = runs.number(0);
            let ask = runs.ask_names;
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
                false if ask => {
                    let out = self
                        .settings
                        .outputs
                        .get(&form::output_key(command, "file"));
                    self.dialog = Some(Dialog::NextDisk {
                        command: command.to_owned(),
                        disk: Some(disk),
                        total,
                        failed: None,
                        image: None,
                        name: Some(String::new()),
                        default: out.map(|o| o.name.trim().to_owned()).unwrap_or_default(),
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
        let part = session.runs.number(session.next);
        session.next += 1;
        let command = session.command.clone();
        if !self.run(ctx, &command, args) {
            self.session = None;
        } else if let Some(job) = &mut self.disk {
            job.part = Some(part);
        }
    }

    /// Reads the session's next disk into `name`, as its prompt asked.
    fn name_next(&mut self, name: &str) {
        let Some(session) = &mut self.session else {
            return;
        };
        let key = form::output_key(&session.command, "file");
        let Some(out) = self.settings.outputs.get(&key) else {
            return;
        };
        if let Some(file) = session
            .runs
            .args
            .get_mut(session.next)
            .and_then(|a| a.last_mut())
        {
            *file = out.named(name).value(1);
            session.runs.images.push(name.to_owned());
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
                    .unwrap_or_else(|| format!("disk {}", session.runs.number(i).0))
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
        if destructive(command) || command == "update" {
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
        let Some(tools) = &self.tools else {
            return false;
        };
        // The image the job makes, one per disk or image.
        let output = image_arg(&args)
            .filter(|_| form::has_output(command))
            .map(Path::to_path_buf);
        if let Some(folder) = output.as_ref().and_then(|p| p.parent()) {
            let _ = std::fs::create_dir_all(folder);
        }
        let env = match command {
            "read" => pass_env(self.settings.outputs.get("read/file"), output.as_deref()),
            _ => Vec::new(),
        };
        let page = match command {
            DETECT => self.detect_for.clone(),
            _ => None,
        }
        .unwrap_or_else(|| command.to_owned());
        match Job::start(
            tools,
            self.settings.kind.name(),
            command,
            args,
            &env,
            repaint(ctx),
        ) {
            Ok(mut job) => {
                job.output = output;
                let (_, _, planned) = self.blank_map(&page);
                job.planned = Some((planned.cyls, planned.heads));
                job.page = page;
                job.format = job
                    .args
                    .iter()
                    .find_map(|a| a.strip_prefix("--format="))
                    .map(String::from);
                let schema = self.schema.as_deref();
                job.progress.verifies = command == "write"
                    && schema.is_some_and(|s| form::verifies(&mut self.service, s, &job.args));
                self.log.begin(heading(&job), &mut job);
                let disk = DISK_COMMANDS.contains(&command);
                *(if disk { &mut self.disk } else { &mut self.tool }) = Some(job);
                true
            }
            Err(e) => {
                self.notices
                    .insert(page, format!("Could not start Greaseweazle Tools: {e}"));
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
        let (format, disk, blank) = self.blank_map(page);
        let tracks = self.settings.values.get(page).map(|v| v.get("tracks"));
        let swapped = tracks.is_some_and(form::swapped);
        // A running job shows on every page. A finished one shows on its own page
        // until that page takes other tracks, or for Detect another format.
        let preview = (&blank.cyls, &blank.heads);
        let shown = self.disk.as_ref().filter(|j| {
            j.running()
                || j.page == page
                    && match j.command.as_str() {
                        DETECT => j.format == format,
                        _ => j.planned.as_ref().is_none_or(|(c, h)| (c, h) == preview),
                    }
        });
        let top = ui.cursor().top();
        // The map's height above a drawer at its least height and below a job's rows, even
        // with no job, so its squares keep one size; and the room below those rows. Rows
        // past them (a warning, a wrapped line) scroll the pane instead of shrinking the squares.
        let room = |ui: &Ui| {
            let used = ui.cursor().top() - top;
            (tall - DRAWER - JOB_ROWS, full - used.min(JOB_ROWS))
        };
        // The job's state keeps to the top right, leaving the rows below to its name.
        egui::Sides::new().shrink_left().truncate().show(
            ui,
            |ui| ui.label(RichText::new("Disk status").size(16.0).strong()),
            |ui| {
                if let Some(job) = shown {
                    let (text, colour) = state(job, p);
                    pill(ui, &format!("{text} · {}", clock(job.elapsed())), colour);
                }
            },
        );
        ui.add_space(4.0);
        let Some(job) = shown else {
            // Only the pages that work on a disk's tracks have a map of their own.
            if !DISK_COMMANDS.contains(&page) {
                ui.label(RichText::new("No disk job running").weak());
                return;
            }
            ui.label(RichText::new(idle_status(page)).weak());
            ui.add_space(10.0);
            let (budget, room) = room(ui);
            diskmap::show(ui, &blank, disk, swapped, false, budget, room);
            return;
        };
        // The sides as the job took them, whatever its page says now.
        let tracks = job.args.iter().find_map(|a| a.strip_prefix("--tracks="));
        let swapped = tracks.is_some_and(form::swapped);
        // These rows wrap, so the job shows in full.
        let mut name = match job.part {
            Some((disk, total)) => format!("{} {disk} of {total}", title(&job.command)),
            None => title(&job.command),
        };
        if let Some((pass, of)) = job.progress.pass {
            name += &format!(", pass {pass} of {of}");
        }
        ui.add(egui::Label::new(RichText::new(name).size(15.0).strong()).wrap());
        // The image a read or a conversion makes, or a write takes.
        let image = match job.command.as_str() {
            "write" => image_arg(&job.args),
            _ => job.output.as_deref(),
        };
        let file = image
            .and_then(|i| i.file_name())
            .map(|f| f.to_string_lossy());
        let format = job
            .format
            .as_ref()
            .map(|f| format!("{} {f}", form::family_name(f).replace(' ', "\u{a0}")));
        let mut show = false;
        if format.is_some() || file.is_some() {
            // As tall as its text, and the link runs on after it.
            ui.scope(|ui| {
                ui.spacing_mut().interact_size.y = 0.0;
                ui.horizontal_wrapped(|ui| {
                    let mut about = format.unwrap_or_default();
                    if !about.is_empty() && file.is_some() {
                        about += "  ·  ";
                    }
                    // The row wraps; a name too long for a line of its own is cut
                    // in the middle, so the row keeps to two.
                    let small = TextStyle::Small.resolve(ui.style());
                    let wide = |text: &str| {
                        let text = text.to_owned();
                        let galley = ui.painter().layout_no_wrap(text, small.clone(), p.dim);
                        galley.size().x + ui.spacing().item_spacing.x
                    };
                    let link = if image_kept(job) { wide(REVEAL) } else { 0.0 };
                    let room = ui.available_width() - link;
                    let name = file.as_deref().map(|f| cut_middle(ui, f, &small, room));
                    let cut = matches!(name, Some(Cow::Owned(_)));
                    about += name.as_deref().unwrap_or_default();
                    let text = RichText::new(about).small().color(p.dim);
                    let about = ui.add(egui::Label::new(text).wrap());
                    if cut {
                        about.on_hover_text(file.as_deref().unwrap_or_default());
                    }
                    if image_kept(job) {
                        show = ui
                            .link(RichText::new(REVEAL).small())
                            .on_hover_text("Show the image in its folder.")
                            .clicked();
                    }
                });
            });
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
            true => diskmap::show(ui, &blank, disk, swapped, false, budget, room),
            false => {
                let verifying = job.running() && job.progress.verifies;
                diskmap::show(ui, &job.progress, disk, swapped, verifying, budget, room);
            }
        }
        if install {
            self.install_rule(ui.ctx());
        }
        if show {
            self.show_image(page);
        }
    }

    /// Shows the disk job's image in the file manager. A fault shows on `page`.
    fn show_image(&mut self, page: &str) {
        let Some(file) = self.disk.as_ref().and_then(|j| j.output.clone()) else {
            return;
        };
        let failed = match file.exists() {
            true => reveal(&file)
                .err()
                .map(|e| format!("Unable to show {}: {e}", file.display())),
            false => Some(format!("{} has been moved or deleted.", file.display())),
        };
        if let Some(why) = failed {
            self.notices.insert(page.to_owned(), why);
        }
    }

    /// The port Linux refused `job`, if it was refused one, and what can grant access.
    fn refused(&self, job: &Job) -> Option<Refused<'_>> {
        let port = self.settings.port(self.service.known_ports());
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

    /// The page's format, its disk's cylinders and sides, (0, 0) until gw describes
    /// a format, and an empty map of the tracks the page takes.
    fn blank_map(&mut self, page: &str) -> (Option<String>, (u32, u32), Progress) {
        let empty = Values::default();
        let values = self.settings.values.get(page).unwrap_or(&empty);
        let format = self
            .schema
            .as_deref()
            .and_then(|s| form::effective_format(&mut self.service, s, s.command(page)?, values));
        let info = format.as_ref().and_then(|format| {
            let diskdefs = form::diskdefs_for(&mut self.service, values, format);
            self.service.format_info(&diskdefs, format).ready()
        });
        let disk = info.map(|i| (i.cyls, i.heads));
        let (cyls, heads) = form::page_tracks(values, disk.unwrap_or(form::USUAL_DISK));
        let blank = Progress::blank(cyls, heads);
        (format, disk.unwrap_or_default(), blank)
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
                    .on_hover_text("Reset the page's command line arguments.")
                    .on_disabled_hover_text("No changes.")
                    .clicked()
                {
                    cli.text.clone_from(&line);
                    cli.error = None;
                    ui.memory_mut(|m| m.surrender_focus(id));
                }
                if let Some(e) = &cli.error {
                    let e = RichText::new(e).small().color(p.bad);
                    ui.add(egui::Label::new(e).truncate());
                }
            });
        });
        // egui puts the cursor where any button presses; a right-click keeps
        // the selection, as a system text box does, for its menu's Paste.
        let ctx = ui.ctx().clone();
        let selection = || {
            egui::text_edit::TextEditState::load(&ctx, id)
                .and_then(|s| s.cursor.char_range())
                .filter(|r| !r.is_empty())
        };
        let kept = selection().filter(|_| ui.input(|i| i.pointer.secondary_pressed()));
        // The Log's box, as tall as the drawer leaves; the command scrolls in it.
        let edit = theme::terminal(ui, |ui, p| {
            let edge = match ui.memory(|m| m.has_focus(id)) {
                true => p.accent,
                false => p.line,
            };
            let frame = console_frame(p, edge);
            let height = ui.available_height() - frame.total_margin().sum().y;
            frame
                .show(ui, |ui| {
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    ui.visuals_mut().widgets.inactive.bg_fill = p.line;
                    egui::ScrollArea::vertical()
                        .id_salt("cli")
                        .max_height(height)
                        .min_scrolled_height(height)
                        .show(ui, |ui| {
                            ui.add(
                                TextEdit::multiline(&mut cli.text)
                                    .id(id)
                                    .font(TextStyle::Monospace)
                                    .frame(Frame::NONE)
                                    .margin(Margin::ZERO)
                                    .desired_rows(1)
                                    .desired_width(f32::INFINITY)
                                    .min_size(vec2(0.0, height)),
                            )
                        })
                        .inner
                })
                .inner
        })
        .on_hover_text("Type or paste a gw command line. The page follows it.");
        if let Some(range) = kept
            && let Some(mut state) = egui::text_edit::TextEditState::load(&ctx, id)
        {
            state.cursor.set_char_range(Some(range));
            state.store(&ctx, id);
        }
        // Its own menu, as a system text box has: a paste goes in at the
        // cursor or over the selection.
        edit.context_menu(|ui| {
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
        let mut apply = None;
        if edit.changed() {
            match command::parse(&schema, &cli.text) {
                Ok(parsed) => {
                    cli.error = None;
                    apply = Some(parsed);
                }
                Err(e) => cli.error = Some(e),
            }
            // The heading, drawn already, shows what is wrong next frame.
            ui.ctx().request_repaint();
        }
        if let Some((name, values, backtrace)) = apply {
            // As with --device: given, it sets the setting; left out, it leaves it.
            self.settings.backtrace |= backtrace;
            self.fill_in(name, values);
        }
    }

    fn presets_menu(&mut self, ui: &mut Ui, command: &str) {
        let folder = self.presets_folder();
        let mut load = None;
        let mut delete = None;
        let (mut save, mut pick) = (false, false);
        let menu = ui.menu_button("Presets", |ui| {
            if self.presets.as_ref().is_none_or(|m| m.page != command) {
                self.presets = Some(PresetsMenu {
                    page: command.to_owned(),
                    saved: presets::list(&folder, command),
                });
            }
            let Some(menu) = &self.presets else { return };
            // Its rows take the menu's width, which is otherwise the window's;
            // a name too long for it is cut in the middle.
            ui.set_width(PRESETS_MENU);
            if menu.saved.is_empty() {
                ui.label(RichText::new("No presets saved yet.").weak());
            }
            let loaded = self.applied.get(command).map(|a| &a.path);
            for (name, path, description) in &menu.saved {
                let tip = match description.as_str() {
                    "" => "Use these settings.",
                    description => description,
                };
                let row = ticked(ui, loaded == Some(path), name);
                if row.on_hover_text(tip).clicked() {
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
                    ui.set_width(PRESETS_MENU);
                    for (name, path, _) in &menu.saved {
                        let row = ticked(ui, false, name);
                        if row.on_hover_text("Delete this preset.").clicked() {
                            delete = Some((name.clone(), path.clone()));
                            ui.close();
                        }
                    }
                });
            }
            if save || pick {
                ui.close();
            }
        });
        if menu.inner.is_none() {
            self.presets = None;
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
                description: String::new(),
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
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width() - f32::from(PAGE_BAR));
                self.settings_inner(ui)
            });
    }

    fn settings_inner(&mut self, ui: &mut Ui) {
        ui.heading("Settings");
        ui.add_space(18.0);
        let p = theme::palette(ui);
        section(ui, "Theme", |ui| {
            ui.horizontal(|ui| {
                for (choice, text, tip, _) in theme::CHOICES {
                    // Blue is Classic's other accent, in Classic's right-click menu.
                    if choice == theme::Choice::Blue {
                        continue;
                    }
                    let classic = choice == theme::Choice::Classic;
                    let choice = if classic { self.classic } else { choice };
                    let r = ui
                        .selectable_label(self.settings.theme == choice, text)
                        .on_hover_text(tip);
                    if r.clicked() {
                        self.choose_theme(ui.ctx(), choice);
                    }
                    if classic {
                        r.context_menu(|ui| {
                            ui.set_width(100.0);
                            for (accent, name) in [
                                (theme::Choice::Classic, "Teal"),
                                (theme::Choice::Blue, "Blue"),
                            ] {
                                if ticked(ui, self.classic == accent, name).clicked() {
                                    self.classic = accent;
                                    self.choose_theme(ui.ctx(), accent);
                                    ui.close();
                                }
                            }
                        });
                    }
                }
            });
        });
        section(ui, "Greaseweazle Tools", |ui| {
            match (&self.tools, &self.service.schema) {
                _ if self.gw_paused.is_some() => {
                    ui.label(RichText::new(UPDATING).weak());
                }
                (Some(tools), Load::Ready(schema)) => {
                    let bundled = match tools.origin {
                        Origin::Bundled => " (bundled)",
                        _ => "",
                    };
                    ui.label(format!("{}{bundled}", schema.tools()));
                }
                (Some(_), Load::Waiting(_)) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Starting…");
                    });
                }
                (None, _) => {
                    ui.label(RichText::new(NOT_FOUND).color(p.bad));
                }
                (_, load) => {
                    ui.label(RichText::new(load.error().unwrap_or(NOT_FOUND)).color(p.bad));
                }
            }
            ui.add_space(4.0);
            let origin = self.tools.as_ref().map(|e| e.origin);
            let busy = match origin {
                None => Some(NOT_FOUND),
                Some(_) => self.busy(),
            };
            ui.horizontal(|ui| {
                let restart = ui.add_enabled(busy.is_none(), egui::Button::new("Restart"));
                if restart
                    .on_hover_text(
                        "Start Greaseweazle Tools again, and check GitHub for a newer release.",
                    )
                    .on_disabled_hover_text(busy.unwrap_or_default())
                    .clicked()
                {
                    self.connect(ui.ctx());
                }
                let (can, tip) = match origin {
                    None => (false, NOT_FOUND.to_owned()),
                    Some(Origin::Bundled) => {
                        self.update_button(&self.gw_update, "Greaseweazle Tools")
                    }
                    Some(_) => (
                        false,
                        "Updates only the built-in Greaseweazle Tools.".into(),
                    ),
                };
                let update = ui.add_enabled(can, egui::Button::new("Update"));
                if update
                    .on_hover_text(&tip)
                    .on_disabled_hover_text(&tip)
                    .clicked()
                    && let Update::Newer(tag) = &self.gw_update
                    && let Some(tools) = &self.tools
                {
                    self.gw_update = Update::gw(tools, tag, repaint(ui.ctx()));
                }
            });
        });
        section(ui, "Paths", |ui| {
            static IMAGES: OnceLock<String> = OnceLock::new();
            static PRESETS: OnceLock<String> = OnceLock::new();
            let default = (
                "Use the default",
                back_to(&IMAGES, form::images_folder),
                None,
            );
            let images = self.images_folder();
            let back = self.settings.images_folder.is_some().then_some(default);
            match path_row(
                ui,
                "Images folder",
                &images,
                "Select where new images go.",
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
            let default = (
                "Use the default",
                back_to(&PRESETS, presets::default_folder),
                None,
            );
            let presets = self.presets_folder();
            let back = self.settings.presets_folder.is_some().then_some(default);
            match path_row(
                ui,
                "Presets folder",
                &presets,
                "Select the presets folder.",
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
            let missing = (!tools::has_bundled()).then_some("No bundled version found.");
            let default = (
                "Use bundled version",
                "Go back to the Greaseweazle Tools bundled with Ferriteweazle.",
                missing,
            );
            let python = self.tools.as_ref().map(|e| e.python.clone());
            let gw = python
                .or_else(|| self.settings.tools.clone())
                .unwrap_or_default();
            let back = self.settings.tools.is_some().then_some(default);
            let tip = "Select gw, gw.exe or a Python with Greaseweazle Tools installed.";
            let busy = self.busy();
            match path_row(ui, "Greaseweazle Tools (gw cli)", &gw, tip, back, busy) {
                Some(PathClick::Choose) => {
                    if let Some(path) = form::file_dialog(&gw).pick_file() {
                        self.settings.tools = Some(path);
                        self.connect(ui.ctx());
                    }
                }
                Some(PathClick::Default) => {
                    self.settings.tools = None;
                    self.connect(ui.ctx());
                }
                None => {}
            }
        });
        section(ui, "Jobs", |ui| {
            setting(
                ui,
                &mut self.settings.save_logs,
                "Save Greaseweazle Tools' output beside each image it makes",
                "Writes the gw command and its output to name.ext.log where gw read or \
                 gw convert puts its image.",
            );
            setting(
                ui,
                &mut self.settings.sound,
                "Play a sound when a job ends",
                SOUND_TIP,
            );
        });
        section(ui, "Troubleshooting", |ui| {
            setting(
                ui,
                &mut self.settings.backtrace,
                "Show Python tracebacks",
                "Passes --bt, so Greaseweazle Tools' errors say where they came from.",
            );
        });
        section(ui, "Update", |ui| self.app_update(ui));
        section(ui, "About", |ui| {
            ui.label(RichText::new(concat!("Ferriteweazle ", env!("CARGO_PKG_VERSION"))).strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.hyperlink_to("Greaseweazle Tools", GW_REPO)
                    .on_hover_text(GW_REPO);
                ui.label(
                    " is the brains of the operation, all credit goes to Keir Fraser and \
                     anyone who contributed to the Greaseweazle project. This is merely a \
                     fancy GUI front end.",
                );
            });
            ui.hyperlink_to("Getting started with Greaseweazle", GW_GUIDE)
                .on_hover_text(GW_GUIDE);
            ui.separator();
            ui.label(
                "Ferriteweazle is made with \u{2661} by Lee Hobson (@hobbo91), under the MIT \
                 license. It is free of charge and provided as is, without warranty.",
            );
            ui.hyperlink_to("Source code and issues", REPO)
                .on_hover_text(REPO);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label("If you find it useful, you can ");
                ui.hyperlink_to("buy me a coffee", COFFEE)
                    .on_hover_text(COFFEE);
                ui.label(".");
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
                            "Warning! If the flash fails, the Greaseweazle may need to be \
                             reflashed with a programming adapter.",
                        );
                    } else if command == "update" {
                        dialog_heading(ui, "Update the firmware?");
                        ui.label("Are you sure?");
                    } else {
                        let drive = self.drive();
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
                        let first = self.session.as_ref().and_then(|s| s.runs.images.first());
                        if let Some(image) = first.filter(|_| disks > 1) {
                            let p = theme::palette(ui);
                            ui.label(RichText::new(format!("First image: {image}")).color(p.dim));
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
                    name,
                    default,
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
                        let first = self.session.as_ref().is_some_and(|s| s.next == 0);
                        ui.label(match first {
                            true => "Insert the first disk in the drive.",
                            false => "Eject, then insert the next disk in the drive.",
                        });
                    }
                    if let Some(image) = image {
                        ui.label(RichText::new(format!("Next image: {image}")).color(p.dim));
                    }
                    // The name the next disk is read under, when the set asks.
                    let mut named = None;
                    if let Some(typed) = name {
                        ui.add_space(6.0);
                        let hint = match default.is_empty() {
                            true => "Required",
                            false => default.as_str(),
                        };
                        let edit = form::edit(typed)
                            .char_limit(form::NAME_LIMIT)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY);
                        ui.horizontal(|ui| {
                            ui.label("Name");
                            ui.add(edit).request_focus();
                        });
                        let chosen = Some(typed.trim()).filter(|t| !t.is_empty());
                        let chosen = chosen.unwrap_or(default.as_str()).to_owned();
                        let out = self
                            .settings
                            .outputs
                            .get(&form::output_key(command, "file"));
                        let path = out.map(|o| o.named(&chosen).path(1));
                        if let Some(path) = path.filter(|p| !chosen.is_empty() && p.exists()) {
                            let file = path.file_name().unwrap_or_default().to_string_lossy();
                            let text = format!("{file} exists. Reading replaces it.");
                            ui.label(RichText::new(text).small().color(p.partial));
                        }
                        named = Some(chosen);
                    }
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        if let Some(disk) = disk {
                            let next = format!("{verb} {disk}");
                            let ready = named.as_ref().is_none_or(|n| !n.is_empty());
                            let enter = ready && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            let button = ui
                                .add_enabled(ready, dialog_button(&next, p.accent, p))
                                .on_disabled_hover_text("Type a name.");
                            if button.clicked() || (enter && named.is_some()) {
                                let ctx = ctx.clone();
                                let named = named.clone();
                                action = Some(Box::new(move |app: &mut App| {
                                    if let Some(name) = named {
                                        app.name_next(&name);
                                    }
                                    app.next_disk(&ctx)
                                }));
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
                Dialog::SavePreset {
                    command,
                    name,
                    description,
                } => {
                    dialog_heading(ui, "Save a preset");
                    let named = ui.add(
                        form::edit(name)
                            .char_limit(form::NAME_LIMIT)
                            .hint_text("e.g. Amiga DD")
                            .desired_width(f32::INFINITY),
                    );
                    // The name keeps the focus unless the description has it.
                    let about = ui.make_persistent_id("preset-description");
                    if !ui.memory(|m| m.has_focus(about)) {
                        named.request_focus();
                    }
                    let exists = presets::path(&self.presets_folder(), name).exists();
                    if exists {
                        let p = theme::palette(ui);
                        let text = "A preset of this name exists. Saving replaces it.";
                        ui.label(RichText::new(text).small().color(p.partial));
                    }
                    ui.add_space(6.0);
                    ui.add(
                        form::edit(description)
                            .id(about)
                            .char_limit(DESCRIPTION_LIMIT)
                            .hint_text(match exists {
                                true => "Description, empty keeps the old one",
                                false => "Description, optional",
                            })
                            .desired_width(f32::INFINITY),
                    )
                    .on_hover_text("Shown when you hover over the preset.");
                    ui.add_space(10.0);
                    right(ui, |ui| {
                        let name = name.trim().to_owned();
                        let description = description.trim().to_owned();
                        let p = theme::palette(ui);
                        let text = if exists { "Replace" } else { "Save" };
                        if ui
                            .add_enabled(!name.is_empty(), dialog_button(text, p.accent, p))
                            .on_disabled_hover_text("Type a name.")
                            .clicked()
                        {
                            let command = command.clone();
                            action = Some(Box::new(move |app: &mut App| {
                                app.save_preset(&command, &name, &description)
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
                        true => "Greaseweazle Tools stops the drive first, then the window closes.",
                        false => "Greaseweazle Tools stops, then the window closes.",
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
        form::anchor_images(&name, &mut values, &self.images_folder());
        for (cmd, dest) in form::OUTPUTS.iter().filter(|(c, _)| *c == name) {
            let file = values.get(dest);
            let key = form::output_key(cmd, dest);
            // The file the page names already, such as a set's first disk, keeps its settings.
            let same = self
                .settings
                .outputs
                .get(&key)
                .is_some_and(|o| o.value(o.first_disk()) == file);
            if !file.is_empty() && !same {
                let mut out = Output::from_value(file);
                // The name given holds while the input is the one given with it.
                let input = image_path(values.get("in_file"));
                out.named_for = input.to_owned();
                self.settings.outputs.insert(key, out);
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
        // Its settings suit its own format.
        self.format_fits.remove(&name);
        self.settings.page = Page::Command(name);
    }

    /// What GitHub has for this copy, with Update at the right.
    fn app_update(&mut self, ui: &mut Ui) {
        let (line, why) = match self.copy {
            Some(_) => self.app_update.summary(env!("CARGO_PKG_VERSION")),
            None => ("This copy was built from source.".into(), None),
        };
        let (can, tip) = match self.stuck {
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
                {
                    self.install_app(ui.ctx());
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

    fn save_preset(&mut self, command: &str, name: &str, description: &str) {
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
        let folder = self.presets_folder();
        // Replaced with no description, a preset keeps its own.
        let description = match description {
            "" => presets::load(&presets::path(&folder, name))
                .map(|p| p.description)
                .unwrap_or_default(),
            typed => typed.to_owned(),
        };
        let preset = Preset {
            command: command.to_owned(),
            values,
            outputs,
            description,
        };
        match presets::save(&folder, name, &preset) {
            Ok(path) => self.applied_now(path, preset),
            Err(e) => {
                let text = format!("Could not save the preset: {e}");
                self.notices.insert(command.to_owned(), text);
            }
        }
    }

    /// Records `preset`, saved to or loaded from `path`, as its page's, keeping
    /// what the page's Reset goes back to.
    fn applied_now(&mut self, path: PathBuf, preset: Preset) {
        let reset_to = self
            .applied
            .get(&preset.command)
            .is_some_and(|a| a.reset_to);
        let page = preset.command.clone();
        let applied = Applied {
            path,
            preset,
            reset_to,
        };
        self.applied.insert(page, applied);
    }

    /// Reset, beside Presets: puts the page back to gw's defaults, or to the
    /// preset it last loaded or saved where its right-click menu ticks that.
    fn reset_button(&mut self, ui: &mut Ui, page: &str) {
        let applied = self.applied.get(page);
        let (loaded, to_preset) = (applied.is_some(), applied.is_some_and(|a| a.reset_to));
        let can = match applied.filter(|a| a.reset_to) {
            Some(a) => !self.holds(page, &a.preset),
            None => self.changed(page),
        };
        let button = ui.add_enabled(can, egui::Button::new("Reset"));
        // Greyed, it still takes the right-click that chooses what it does.
        let (menu, tip) = match can {
            true => (
                button,
                "Reset this page. Right-click to choose default or preset.",
            ),
            false => (
                ui.interact(button.rect, button.id.with("menu"), Sense::CLICK),
                "No changes. Right-click to choose default or preset.",
            ),
        };
        let reset = can && menu.clicked();
        let mut choice = None;
        menu.on_hover_text(tip).context_menu(|ui| {
            // Its rows take the menu's width, which is otherwise the window's.
            ui.set_width(160.0);
            if ticked(ui, !to_preset, "Page to default").clicked() {
                choice = Some(false);
            }
            let current = ui.add_enabled_ui(loaded, |ui| ticked(ui, to_preset, "Current preset"));
            if current
                .inner
                .on_disabled_hover_text("No preset loaded.")
                .clicked()
            {
                choice = Some(true);
            }
            if choice.is_some() {
                ui.close();
            }
        });
        if let (Some(choice), Some(applied)) = (choice, self.applied.get_mut(page)) {
            applied.reset_to = choice;
        }
        if reset {
            match self.applied.get(page).filter(|a| a.reset_to) {
                Some(applied) => self.apply(applied.preset.clone()),
                None => self.restore_defaults(page),
            }
        }
    }

    /// Whether `page` has `preset`'s settings. An output's file comes from its
    /// output settings, so they stand for it.
    fn holds(&self, page: &str, preset: &Preset) -> bool {
        let empty = Values::default();
        let option = |(dest, _): &(&str, &str)| !form::OUTPUTS.contains(&(page, *dest));
        let values = self.settings.values.get(page).unwrap_or(&empty);
        values
            .iter()
            .filter(option)
            .eq(preset.values.iter().filter(option))
            && (preset.outputs.iter()).all(|(k, o)| self.settings.outputs.get(k) == Some(o))
    }

    /// Deletes a preset's file. A fault shows on `page`.
    fn delete_preset(&mut self, page: &str, path: &Path) {
        match std::fs::remove_file(path) {
            Ok(()) => self.applied.retain(|_, a| a.path != path),
            Err(e) => {
                let text = format!("Could not delete the preset: {e}");
                self.notices.insert(page.to_owned(), text);
            }
        }
    }

    /// Output settings as a new page has them: gw's defaults, and the images folder.
    fn fresh_output(&self) -> Output {
        Output {
            folder: self.images_folder().to_string_lossy().into_owned(),
            ..Output::default()
        }
    }

    /// Whether a page differs from gw's defaults, as Reset leaves it.
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

    /// Puts a preset's settings on its page.
    fn apply(&mut self, preset: Preset) {
        // Its settings suit its own format, which a notice may not name.
        self.format_fits.remove(&preset.command);
        self.notices.remove(&preset.command);
        self.settings.outputs.extend(preset.outputs);
        self.settings.values.insert(preset.command, preset.values);
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
        // A notice, such as Detect's, may name a format the page no longer has.
        self.notices.remove(command);
    }

    /// Applies a preset file's settings and opens its page. A fault shows on `page`.
    fn load_preset(&mut self, page: &str, path: &Path) {
        match presets::load(path) {
            Ok(preset) => {
                self.settings.page = Page::Command(preset.command.clone());
                self.apply(preset.clone());
                self.applied_now(path.to_owned(), preset);
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
        dialog_heading(ui, "Greaseweazle Tools asks");
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

/// What the bridge needs to read in passes, which must match bridge.py's gw.
fn pass_env(out: Option<&Output>, image: Option<&Path>) -> Vec<(&'static str, String)> {
    let Some(out) = out.filter(|o| o.passes > 1) else {
        return Vec::new();
    };
    let mut env = vec![("FERRITEWEAZLE_PASSES", out.passes.to_string())];
    if out.whole_disk {
        env.push(("FERRITEWEAZLE_REREAD", "disk".into()));
    }
    if let Some(image) = image.filter(|_| out.keep_passes) {
        let prefix = form::pass_prefix(image);
        env.push(("FERRITEWEAZLE_KEEP", prefix.to_string_lossy().into_owned()));
    }
    env
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
            let args = (out.first_disk()..=out.disks.max(1))
                .map(|d| {
                    values.set(dest, out.value(d));
                    argv(&values)
                })
                .collect();
            // Names asked for later are checked as they are typed.
            let makes = match out.asks_names() {
                true => Vec::new(),
                false => out.paths().collect(),
            };
            (args, makes)
        }
        Some((_, out)) => (vec![argv(&values)], vec![out.path(1)]),
        None => (vec![argv(&values)], Vec::new()),
    };
    Runs {
        args,
        makes,
        before: out.map_or(0, |(_, o)| o.first_disk() as usize - 1),
        ask_names: out.is_some_and(|(_, o)| cmd.name == "read" && o.asks_names()),
        ..Runs::default()
    }
}

fn destructive(command: &str) -> bool {
    DESTRUCTIVE.iter().any(|(c, _)| *c == command)
}

/// Whether the command is gw update --bootloader.
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

/// Where Greaseweazle Tools looks for the SPS/CAPS library, which reads IPF and CTRaw
/// images and which gw does not ship.
fn caps_advice(tools: Option<&Tools>) -> String {
    if cfg!(target_os = "macos") {
        "Greaseweazle Tools looks for CAPSImage.framework or CAPSImg.framework in /Library/Frameworks.".into()
    } else if cfg!(windows) {
        // Python loads a DLL by name from python.exe's folder or System32.
        let bundled = tools.filter(|e| e.origin == Origin::Bundled);
        match bundled.and_then(|e| e.python.parent()) {
            Some(folder) => format!(
                "Greaseweazle Tools looks for CAPSImg_x64.dll or CAPSImg.dll in {} and in System32.",
                folder.display()
            ),
            None => "Greaseweazle Tools looks for CAPSImg_x64.dll or CAPSImg.dll beside its python.exe and in \
                     System32."
                .into(),
        }
    } else {
        "Greaseweazle Tools looks for libcapsimage.so.5 or libcapsimage.so.4 in the system's library folders, \
         such as /usr/lib."
            .into()
    }
}

/// The tip of Play a sound when a job ends: the sounds this system plays.
const SOUND_TIP: &str = if cfg!(target_os = "macos") {
    "Glass when it works, Basso when it fails."
} else if cfg!(windows) {
    "The Notification sound when it works, Critical Stop when it fails."
} else {
    "The sound theme's complete sound when it works, dialog-error when it fails."
};

/// Plays the system's sound for how a job ended.
fn chime(outcome: Option<Outcome>) {
    let players = players(outcome);
    if players.is_empty() {
        return;
    }
    // A thread waits for the player, so it is reaped when it ends. The next
    // is tried only if this one is not installed.
    std::thread::spawn(move || {
        for mut player in players {
            let null = std::process::Stdio::null;
            if player.stdout(null()).stderr(null()).status().is_ok() {
                break;
            }
        }
    });
}

/// The commands that play the system's sound for how a job ended, best
/// first: none for a job that was stopped.
fn players(outcome: Option<Outcome>) -> Vec<std::process::Command> {
    use std::process::Command;
    let worked = match outcome {
        Some(Outcome::Succeeded) => true,
        Some(Outcome::Failed) => false,
        _ => return Vec::new(),
    };
    if cfg!(target_os = "macos") {
        let sound = if worked { "Glass" } else { "Basso" };
        let mut afplay = Command::new("/usr/bin/afplay");
        afplay.arg(format!("/System/Library/Sounds/{sound}.aiff"));
        vec![afplay]
    } else if cfg!(windows) {
        // The file the sound scheme gives the event: none if turned off.
        let event = if worked {
            "Notification.Default"
        } else {
            "SystemHand"
        };
        let script = format!(
            "$f = [Environment]::ExpandEnvironmentVariables((Get-ItemProperty \
             'HKCU:\\AppEvents\\Schemes\\Apps\\.Default\\{event}\\.Current').'(default)'); \
             if ($f) {{ (New-Object Media.SoundPlayer $f).PlaySync() }}"
        );
        let mut powershell = Command::new("powershell");
        powershell.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            powershell.creation_flags(CREATE_NO_WINDOW);
        }
        vec![powershell]
    } else {
        // The desktop's sound theme, which heeds its event sound setting,
        // else the freedesktop theme's own file.
        let id = if worked { "complete" } else { "dialog-error" };
        let mut canberra = Command::new("canberra-gtk-play");
        canberra.args(["--id", id, "--description", "Ferriteweazle"]);
        let file = format!("/usr/share/sounds/freedesktop/stereo/{id}.oga");
        let files = ["pw-play", "paplay"].map(|player| {
            let mut player = Command::new(player);
            player.arg(&file);
            player
        });
        std::iter::once(canberra).chain(files).collect()
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
        form::toggle(ui, on, label).on_hover_text(tip);
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

/// What an unfinished disk job leaves behind: gw keeps a stopped read's tracks, and
/// deletes a stopped conversion's image and a failed job's.
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

/// Whether a disk job has ended and left its image: gw deletes a failed
/// job's, and a stopped conversion's.
fn image_kept(job: &Job) -> bool {
    job.output.is_some()
        && match job.outcome() {
            Some(Outcome::Succeeded) => true,
            Some(Outcome::Failed) => !job.no_image,
            Some(Outcome::Stopped) => job.command == "read",
            None => false,
        }
}

/// The file manager's own words for showing a file in it.
const REVEAL: &str = if cfg!(target_os = "macos") {
    "Show in Finder"
} else if cfg!(windows) {
    "Show in Explorer"
} else {
    "Show in folder"
};

/// Shows `file` in the system's file manager: selected in Finder or
/// Explorer, elsewhere by opening its folder.
fn reveal(file: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("/usr/bin/open");
        cmd.arg("-R").arg(file);
        cmd
    };
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("explorer");
        // explorer reads /select, and the quoted path as one argument.
        cmd.raw_arg(format!("/select,\"{}\"", file.display()));
        cmd
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut cmd = {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(file.parent().unwrap_or(file));
        cmd
    };
    let mut child = cmd.spawn()?;
    // Waited on, so it is reaped when it ends.
    std::thread::spawn(move || child.wait());
    Ok(())
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

const NO_ACCESS: &str = "This account has no permission to open the port. Greaseweazle Tools' udev rule \
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
    theme::terminal(ui, |ui, p| {
        console_frame(p, p.line).show(ui, |ui| {
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
    });
    ui.hyperlink_to("Greaseweazle Tools' Linux instructions", udev::WIKI)
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

/// The height of a job's output box, in points.
const OUTPUT_HEIGHT: f32 = 260.0;

/// gw's output with Copy and Save: the Log as tall as the room left, with
/// Clear, or a job's in a box of its own. Gives whether Clear was pressed,
/// and why a save failed.
fn output(ui: &mut Ui, shown: Shown) -> (bool, Option<String>) {
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
                .on_hover_text("Save Greaseweazle Tools' output to a file.")
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
                .on_hover_text("Copy Greaseweazle Tools' output.")
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
    theme::terminal(ui, |ui, p| {
        let frame = console_frame(p, p.line);
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
                    Shown::Job(job) if !job.running() => "Greaseweazle Tools printed no output.",
                    _ => "Greaseweazle Tools' output appears here.",
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
    });
    (clear, unsaved)
}

/// The box of gw's command line or output, edged in `edge`.
fn console_frame(p: &Palette, edge: Color32) -> Frame {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, edge))
        .corner_radius(8)
        .inner_margin(8)
}

/// Writes `log` to `path`, and says why if it cannot.
fn save_log(path: &Path, log: &[String]) -> Option<String> {
    let text = log.join("\n") + "\n";
    let failed = std::fs::write(path, text).err()?;
    Some(format!("Could not save {}: {failed}", path.display()))
}

/// Where the gw chosen in Settings is kept between runs; the built-in or an
/// installed gw keeps no file.
fn tools_file() -> PathBuf {
    crate::data_folder().join("gw.txt")
}

fn kept_tools(file: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(file).ok()?;
    Some(PathBuf::from(text.trim())).filter(|p| !p.as_os_str().is_empty())
}

fn keep_tools(file: &Path, tools: Option<&Path>) {
    keep(file, tools.map(|p| p.to_string_lossy().into_owned()));
}

/// Where the release whose update banner was dismissed is kept, such as v1.2.1.
fn dismissed_file() -> PathBuf {
    crate::data_folder().join("dismissed.txt")
}

fn kept_dismissed(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    Some(text.trim().to_owned()).filter(|t| !t.is_empty())
}

/// A banner at the top of a page: `text`, wrapped clear of `room` points at its
/// right, where `actions` adds its buttons.
fn banner(ui: &mut Ui, text: &str, room: f32, actions: impl FnOnce(&mut Ui)) {
    ui.add_space(8.0);
    let p = theme::palette(ui);
    Frame::new()
        .fill(p.accent.gamma_multiply(0.12))
        .corner_radius(8)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.allocate_ui(vec2(ui.available_width() - room, 0.0), |ui| {
                    ui.add(egui::Label::new(text).wrap());
                });
                right(ui, actions);
            });
        });
}

/// Where the theme chosen in Settings is kept between runs; System keeps no file.
fn theme_file() -> PathBuf {
    crate::data_folder().join("theme.txt")
}

fn kept_theme(file: &Path) -> theme::Choice {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let kept = theme::CHOICES.iter().find(|c| c.3 == text.trim());
    kept.map_or(theme::Choice::System, |c| c.0)
}

fn keep_theme(file: &Path, choice: theme::Choice) {
    let word = theme::CHOICES.iter().find(|c| c.0 == choice).map(|c| c.3);
    keep(file, word.filter(|w| !w.is_empty()).map(String::from));
}

/// Where the drive identifier is kept between runs.
fn drive_file() -> PathBuf {
    crate::data_folder().join("drive.txt")
}

/// The port gw info reports and the device type its Model names.
fn named_device(info: &DeviceInfo) -> Option<(String, Kind)> {
    let kind = Kind::of_model(info.get("Model")?)?;
    Some((info.get("Port")?.to_owned(), kind))
}

/// Where the device type is kept between runs, with an Adafruit RP2040's
/// port, which gw cannot find by itself. A Greaseweazle keeps no file.
fn device_file() -> PathBuf {
    crate::data_folder().join("device.txt")
}

/// The device type and port kept in `file`.
fn kept_device(file: &Path) -> (Kind, String) {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let mut lines = text.lines().map(str::trim);
    match lines.next() {
        Some("adafruit") => (Kind::Adafruit, lines.next().unwrap_or_default().to_owned()),
        _ => (Kind::Greaseweazle, String::new()),
    }
}

/// Keeps the device type in `file` and an Adafruit RP2040's `port`, or
/// removes the file for a Greaseweazle.
fn keep_device(file: &Path, kind: Kind, port: &str) {
    keep(
        file,
        (kind == Kind::Adafruit).then(|| format!("adafruit\n{port}\n")),
    );
}

/// A menu row with a tick when it is the one chosen of its group.
fn ticked(ui: &mut Ui, on: bool, text: &str) -> egui::Response {
    let size = vec2(ui.available_width(), ui.spacing().interact_size.y);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, on, text)
    });
    if ui.is_rect_visible(rect) {
        let p = theme::palette(ui);
        let painter = ui.painter();
        if response.hovered() {
            painter.rect_filled(rect, 4, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        if on {
            let tick = egui::Rect::from_center_size(
                pos2(rect.left() + 11.0, rect.center().y),
                vec2(12.0, 12.0),
            );
            let at = |x: f32, y: f32| tick.min + tick.size() * vec2(x, y);
            let line = vec![at(0.1, 0.55), at(0.4, 0.85), at(0.95, 0.2)];
            painter.add(egui::Shape::line(line, Stroke::new(2.0, p.accent)));
        }
        let font = TextStyle::Button.resolve(ui.style());
        let shown = cut_middle(ui, text, &font, rect.width() - 24.0);
        // Cut, the whole text shows on hover, above any of the caller's.
        let cut = matches!(shown, Cow::Owned(_));
        let at = pos2(rect.left() + 24.0, rect.center().y);
        let colour = ui.visuals().text_color();
        painter.text(at, Align2::LEFT_CENTER, shown, font, colour);
        if cut {
            response = response.on_hover_text(text);
        }
    }
    response
}

/// `text`, or where it is wider than `width` in `font`, its two ends about
/// an ellipsis, so that texts differing at either end stay apart.
fn cut_middle<'a>(ui: &Ui, text: &'a str, font: &FontId, width: f32) -> Cow<'a, str> {
    let wide = |t: &str| {
        let galley = ui
            .painter()
            .layout_no_wrap(t.to_owned(), font.clone(), Color32::PLACEHOLDER);
        galley.size().x
    };
    let whole = wide(text);
    if whole <= width {
        return text.into();
    }
    let chars: Vec<char> = text.chars().collect();
    // From a guess in proportion to the width, a character fewer at a time.
    let mut keep = (chars.len() as f32 * width / whole) as usize;
    loop {
        let (head, tail) = (keep - keep / 2, keep / 2);
        let ends = chars[..head]
            .iter()
            .chain(&['…'])
            .chain(&chars[chars.len() - tail..]);
        let cut: String = ends.collect();
        if keep == 0 || wide(&cut) <= width {
            return cut.into();
        }
        keep -= 1;
    }
}

/// An image argument's path, without gw's `::` options after it.
fn image_path(value: &str) -> &str {
    value.split_once("::").map_or(value, |(path, _)| path)
}

/// The image a gw command line ends with.
fn image_arg(args: &[String]) -> Option<&Path> {
    args.last().map(|a| Path::new(image_path(a)))
}

/// Where the window's size is kept between runs.
fn size_file() -> PathBuf {
    crate::data_folder().join("window.txt")
}

/// How long a new window size must stay before it is kept.
const SIZE_SETTLE: Duration = Duration::from_millis(500);
/// Height a screen keeps for a title bar and a taskbar or menu bar, in points.
const SCREEN_BARS: f32 = 80.0;
/// The corner that resets the window's size, in points.
const SIZE_CORNER: f32 = 16.0;

/// The window's size as it opens: as last kept, else WINDOW.
pub fn opening_size() -> egui::Vec2 {
    kept_size(&size_file()).unwrap_or(WINDOW)
}

/// The size kept in `file`, "width height" in points, no smaller than SMALLEST.
fn kept_size(file: &Path) -> Option<egui::Vec2> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut numbers = text.split_whitespace().map(|n| n.parse::<f32>().ok());
    let (width, height) = (numbers.next()??, numbers.next()??);
    let size = vec2(width, height);
    size.is_finite().then(|| size.max(SMALLEST))
}

/// Keeps `size` in `file`, or removes the file for WINDOW.
fn save_size(file: &Path, size: egui::Vec2) {
    let text = format!("{} {}", size.x.round(), size.y.round());
    keep(file, (!same_size(size, WINDOW)).then_some(text));
}

/// Sizes within half a point, as a window reports its size in pixels.
fn same_size(a: egui::Vec2, b: egui::Vec2) -> bool {
    (a - b).abs().max_elem() < 0.5
}

/// The window's bottom right corner: right-click it to put back WINDOW. It
/// shows only when hovered, as it cannot start a resize on macOS.
fn size_corner(ctx: &egui::Context) {
    let size = ctx.content_rect().size();
    let maximized = ctx.input(|i| i.viewport().maximized == Some(true));
    let changed = maximized || !same_size(size, WINDOW);
    egui::Area::new(Id::new("size-corner"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let (rect, corner) =
                ui.allocate_exact_size(vec2(SIZE_CORNER, SIZE_CORNER), Sense::click());
            corner.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Window size")
            });
            if corner.hovered() {
                let stroke = Stroke::new(1.0, theme::palette(ui).dim);
                for inset in [4.0, 8.0] {
                    let (a, b) = (
                        rect.right_top() + vec2(0.0, inset),
                        rect.left_bottom() + vec2(inset, 0.0),
                    );
                    ui.painter().line_segment([a, b], stroke);
                }
            }
            corner
                .on_hover_text("Right-click to reset the window's size.")
                .context_menu(|ui| {
                    let reset = ui.add_enabled(changed, egui::Button::new("Reset window size"));
                    if reset
                        .on_hover_text("Put back the size the window first opens at.")
                        .on_disabled_hover_text("The window is its default size.")
                        .clicked()
                    {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
                        ctx.send_viewport_cmd(ViewportCommand::InnerSize(WINDOW));
                        ui.close();
                    }
                });
        });
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
    keep(file, (!drive.is_empty()).then(|| drive.to_owned()));
}

/// Writes `text` to `file`, making its folder, or removes the file for None.
fn keep(file: &Path, text: Option<String>) {
    let _ = match text {
        None => std::fs::remove_file(file),
        Some(text) => file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(file, text)),
    };
}

/// The port chosen while it is connected, else the best Greaseweazle. An
/// Adafruit RP2040 has the one chosen alone: gw cannot pick it out.
fn chosen_port<'p>(ports: &'p [Port], chosen: &str, kind: Kind) -> Option<&'p Port> {
    let named = ports
        .iter()
        .find(|p| !chosen.is_empty() && p.device == chosen);
    match kind {
        Kind::Greaseweazle => named.or_else(|| ports.iter().find(|p| p.score > 0)),
        Kind::Adafruit => named,
    }
}

/// A port as people know it: COM3, ttyACM0, cu.usbmodem14201.
fn short_port(device: &str) -> &str {
    device.strip_prefix("/dev/").unwrap_or(device)
}

/// "Found akai.800. Disk also matches eagle.dsqd.800 and zx.quorum.ds80."
/// `changed`: Detect's step replaced the track list's.
fn found_note(formats: &[String], step: u32, changed: bool) -> String {
    let mut note = format!("Found {}.", formats[0]);
    match (step > 1, changed) {
        (true, true) => note += " 40-track disk in an 80-track drive, setting Step to 2.",
        (true, false) => note += " 40-track disk in an 80-track drive.",
        (false, true) => note += " The disk's tracks match the drive's, setting Step to 1.",
        (false, false) => {}
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

/// "Go back to" a default folder, made once: the folder is fixed while the app runs.
fn back_to(tip: &'static OnceLock<String>, folder: fn() -> PathBuf) -> &'static str {
    tip.get_or_init(|| format!("Go back to {}.", folder().display()))
}

/// A path in Settings: its name, where it is, Browse…, and with `back` a
/// button, its tip and why it greys, to restore the default. Both wait while
/// `busy` says why.
fn path_row(
    ui: &mut Ui,
    name: &str,
    path: &Path,
    tip: &str,
    back: Option<(&str, &str, Option<&str>)>,
    busy: Option<&str>,
) -> Option<PathClick> {
    ui.label(name);
    let shown = match path.as_os_str().is_empty() {
        true => RichText::new("Not found.").weak(),
        false => RichText::new(form::short_path(&path.to_string_lossy()))
            .monospace()
            .small()
            .weak(),
    };
    ui.label(shown);
    ui.horizontal(|ui| {
        let why = busy.unwrap_or_default();
        let choose = ui.add_enabled(busy.is_none(), egui::Button::new("Browse…"));
        if choose
            .on_hover_text(tip)
            .on_disabled_hover_text(why)
            .clicked()
        {
            return Some(PathClick::Choose);
        }
        let (text, tip, missing) = back?;
        let why = missing.or(busy);
        ui.add_enabled(why.is_none(), egui::Button::new(text))
            .on_hover_text(tip)
            .on_disabled_hover_text(why.unwrap_or_default())
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
    let (fill, colour) = match (selected, p.classic) {
        (true, true) => (p.accent, p.on_accent),
        (true, false) => (p.accent.gamma_multiply(0.16), p.accent),
        (false, _) if response.hovered() => (p.hover, p.text),
        (false, _) => (Color32::TRANSPARENT, p.text),
    };
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
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
            if selected && p.classic { colour } else { p.dim },
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

/// The refresh button's side.
const REFRESH: f32 = 22.0;

/// A small button with a painted circular arrow.
fn refresh_button(p: &'static Palette) -> impl egui::Widget {
    move |ui: &mut Ui| {
        let response = ui.add(
            egui::Button::new("")
                .min_size(vec2(REFRESH, REFRESH))
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
        serde_json::from_str(include_str!("gw-1.23.json")).unwrap()
    }

    fn offline() -> App {
        App::offline(&egui::Context::default(), Settings::default(), Ok(schema()))
    }

    /// A Greaseweazle as gw lists it, on a made-up port.
    fn greaseweazle(device: &str, denied: bool) -> Port {
        Port {
            device: device.into(),
            name: Some("Greaseweazle".into()),
            score: 20,
            denied,
        }
    }

    /// Greaseweazle Tools with no Python behind them: no job starts.
    fn no_gw() -> Tools {
        Tools {
            python: "/no/such/python".into(),
            origin: Origin::Custom,
            standalone: false,
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

    /// The window after two frames, stepped, not run: a running job keeps it repainting.
    fn window(app: App) -> Harness<'static, App> {
        let mut w = Harness::builder()
            .with_size(vec2(1240.0, 780.0))
            .build_ui_state(|ui, app: &mut App| app.show(ui), app);
        w.run_steps(2);
        w
    }

    #[test]
    fn the_window_size_is_kept_no_smaller_than_the_smallest_and_the_default_keeps_no_file() {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-size-{}", std::process::id()));
        let file = dir.join("window.txt");
        assert_eq!(kept_size(&file), None, "no file");
        save_size(&file, vec2(1400.4, 900.0));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "1400 900");
        assert_eq!(kept_size(&file), Some(vec2(1400.0, 900.0)));
        std::fs::write(&file, "10 10").unwrap();
        assert_eq!(kept_size(&file), Some(SMALLEST));
        for bad in ["", "wide", "1200", "NaN 800", "inf 800"] {
            std::fs::write(&file, bad).unwrap();
            assert_eq!(kept_size(&file), None, "{bad:?}");
        }
        save_size(&file, WINDOW);
        assert!(!file.exists(), "the default is no file");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every option the Adafruit RP2040 greys, by gw 1.23's own names.
    const ADAFRUIT_OPTIONS: &[(&str, &str)] = &[
        ("read", "densel"),
        ("read", "gen_tg43"),
        ("read", "hard_sectors"),
        ("write", "densel"),
        ("write", "gen_tg43"),
        ("write", "hard_sectors"),
        ("write", "pre_erase"),
        ("write", "erase_empty"),
        ("info", "bootloader"),
    ];

    /// An offline app driving an Adafruit RP2040 on COM9, able to run.
    fn adafruit() -> App {
        let mut app = offline();
        app.tools = Some(no_gw());
        app.settings.kind = Kind::Adafruit;
        app.settings.device = "COM9".into();
        app.pin_ports(vec![Port {
            device: "COM9".into(),
            name: Some("Feather RP2040".into()),
            score: 0,
            denied: false,
        }]);
        app
    }

    #[test]
    fn every_option_the_adafruit_rp2040_greys_is_one_gw_has() {
        let schema = schema();
        for (command, dest) in ADAFRUIT_OPTIONS {
            let cmd = schema.command(command).unwrap();
            assert!(cmd.arg(dest).is_some(), "gw {command} has no {dest}");
            assert!(adafruit::option(command, dest), "{command} {dest}");
        }
        for command in ["erase", "update", "delays", "reset", "pin get", "pin set"] {
            assert!(schema.command(command).is_some(), "gw has no {command}");
        }
        for drive in adafruit::DRIVES {
            assert!(
                App::offline(
                    &egui::Context::default(),
                    Settings::default(),
                    Ok(schema.clone())
                )
                .drives()
                .iter()
                .any(|(id, _)| id == drive)
            );
        }
    }

    #[test]
    fn an_adafruit_rp2040_is_only_ever_the_port_chosen_for_it() {
        let ports = [
            greaseweazle("COM3", false),
            Port {
                score: 0,
                ..greaseweazle("COM9", false)
            },
        ];
        let chosen =
            |chosen: &str, kind| chosen_port(&ports, chosen, kind).map(|p| p.device.as_str());
        assert_eq!(chosen("", Kind::Greaseweazle), Some("COM3"), "gw's best");
        assert_eq!(chosen("", Kind::Adafruit), None, "gw cannot pick one out");
        assert_eq!(chosen("COM9", Kind::Adafruit), Some("COM9"));
        assert_eq!(chosen("COM4", Kind::Adafruit), None, "not connected");
        assert_eq!(chosen("COM4", Kind::Greaseweazle), Some("COM3"));
    }

    #[test]
    fn the_adafruit_rp2040_and_its_port_are_kept_and_a_greaseweazle_keeps_no_file() {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-device-{}", std::process::id()));
        let file = dir.join("device.txt");
        assert_eq!(kept_device(&file), (Kind::Greaseweazle, String::new()));
        keep_device(&file, Kind::Adafruit, "/dev/cu.usbmodem1101");
        assert_eq!(
            kept_device(&file),
            (Kind::Adafruit, "/dev/cu.usbmodem1101".to_owned())
        );
        keep_device(&file, Kind::Greaseweazle, "COM3");
        assert!(!file.exists(), "gw finds a Greaseweazle by itself");
        std::fs::write(&file, "something else\nCOM3").unwrap();
        assert_eq!(kept_device(&file), (Kind::Greaseweazle, String::new()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_newer_release_is_offered_until_dismissed_and_a_later_one_again() {
        let mut app = offline();
        (app.copy, app.stuck) = (Some(Install::Folder(PathBuf::from("/opt/F"))), None);
        app.app_update = Update::Newer("v9.9.0".into());
        let mut w = window(app);
        w.get_by_label("Ferriteweazle 9.9.0 is available.");
        w.get_by_label("Update");
        w.get_by_label("Dismiss").click();
        w.run_steps(2);
        assert!(w.query_by_label_contains("is available").is_none());
        assert_eq!(w.state().dismissed.as_deref(), Some("v9.9.0"));
        w.state_mut().app_update = Update::Newer("v9.9.1".into());
        w.run_steps(2);
        w.get_by_label("Ferriteweazle 9.9.1 is available.");
        // A copy that cannot replace itself is offered nothing.
        w.state_mut().stuck = Some("Read-only.");
        w.run_steps(2);
        assert!(w.query_by_label_contains("is available").is_none());
    }

    #[test]
    fn a_failed_install_says_why_on_the_page() {
        let mut app = offline();
        let (tx, rx) = std::sync::mpsc::channel();
        app.app_update = Update::Installing(rx, "v9.9.0".into());
        tx.send(Err("No space left on device".into())).unwrap();
        app.poll_updates(&egui::Context::default());
        let why = "Unable to install the update: No space left on device";
        assert_eq!(app.notices["read"], why);
    }

    #[test]
    fn the_theme_is_kept_and_system_keeps_no_file() {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-theme-{}", std::process::id()));
        let file = dir.join("theme.txt");
        assert_eq!(kept_theme(&file), theme::Choice::System);
        for theme in [
            theme::Choice::Light,
            theme::Choice::Dark,
            theme::Choice::Classic,
            theme::Choice::Blue,
            theme::Choice::Greaseweazle,
        ] {
            keep_theme(&file, theme);
            assert_eq!(kept_theme(&file), theme);
        }
        keep_theme(&file, theme::Choice::System);
        assert!(!file.exists(), "System is the default");
        std::fs::write(&file, "purple").unwrap();
        assert_eq!(kept_theme(&file), theme::Choice::System);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_adafruit_rp2040_runs_without_the_options_it_greys_and_they_come_back_after() {
        let schema = schema();
        let mut app = adafruit();
        for (command, dest) in ADAFRUIT_OPTIONS {
            let arg = schema.command(command).unwrap().arg(dest).unwrap();
            let value = if arg.switch { command::ON } else { "H" };
            app.settings
                .values
                .entry((*command).to_owned())
                .or_default()
                .set(dest, value);
        }
        let flags = |app: &App, command: &str| {
            let cmd = schema.command(command).unwrap();
            let args = app.args(cmd);
            ADAFRUIT_OPTIONS
                .iter()
                .filter(|(c, _)| *c == command)
                .filter(|(_, d)| {
                    args.iter()
                        .any(|a| a.starts_with(&format!("--{}", d.replace('_', "-"))))
                })
                .count()
        };
        for command in ["read", "write", "info"] {
            assert_eq!(flags(&app, command), 0, "{command}");
        }
        assert!(
            app.args(schema.command("read").unwrap())
                .contains(&"--device=COM9".to_owned())
        );
        app.settings.kind = Kind::Greaseweazle;
        assert_eq!(flags(&app, "read"), 3);
        assert_eq!(flags(&app, "write"), 5);
        assert_eq!(flags(&app, "info"), 1);
    }

    #[test]
    fn the_adafruit_rp2040_says_why_it_cannot_run_a_page() {
        let schema = schema();
        let mut app = adafruit();
        let why = |app: &App, command: &str| {
            let cmd = schema.command(command).unwrap();
            app.why_not(&schema, cmd)
        };
        let set = |app: &mut App, command: &str, dest: &str, value: &str| {
            app.settings
                .values
                .entry(command.to_owned())
                .or_default()
                .set(dest, value);
        };
        for command in ["erase", "update", "delays", "reset"] {
            assert_eq!(
                why(&app, command).as_deref(),
                adafruit::command(command),
                "{command}"
            );
        }
        assert_eq!(why(&app, "rpm"), None);
        assert_eq!(why(&app, "bandwidth"), None);

        set(&mut app, "pin get", "pin", "25");
        assert_eq!(
            why(&app, "pin get").as_deref(),
            Some("The Adafruit RP2040 reads pin 26 alone.")
        );
        set(&mut app, "pin get", "pin", "26");
        assert_eq!(why(&app, "pin get"), None);
        set(&mut app, "pin set", "pin", "4");
        set(&mut app, "pin set", "level", "H");
        assert_eq!(
            why(&app, "pin set").as_deref(),
            Some("The Adafruit RP2040 sets pin 2 alone.")
        );
        set(&mut app, "pin set", "pin", "2");
        assert_eq!(why(&app, "pin set"), None);

        let far = Some("The Adafruit RP2040 reaches cylinders 0 to 79.");
        set(&mut app, "seek", "cylinder", "80");
        assert_eq!(why(&app, "seek").as_deref(), far);
        set(&mut app, "seek", "cylinder", "79");
        assert_eq!(why(&app, "seek"), None);
        assert_eq!(why(&app, "clean"), None, "gw's 80 cylinders end at 79");
        set(&mut app, "clean", "cyls", "81");
        assert_eq!(why(&app, "clean").as_deref(), far);
        set(&mut app, "read", "format", "ibm.1440");
        set(&mut app, "read", "tracks", "c=0-81");
        assert_eq!(why(&app, "read").as_deref(), far);
        set(&mut app, "read", "tracks", "c=0-79");
        assert_ne!(why(&app, "read").as_deref(), far);

        app.settings.drive = "B".into();
        assert_eq!(
            why(&app, "rpm").as_deref(),
            Some("The Adafruit RP2040 has one drive: select A or 0.")
        );
        app.settings.drive = "0".into();
        assert_eq!(why(&app, "rpm"), None);

        app.settings.kind = Kind::Greaseweazle;
        app.settings.drive = "B".into();
        for command in ["erase", "seek", "clean", "pin get", "pin set", "rpm"] {
            assert_eq!(why(&app, command), None, "{command} on a Greaseweazle");
        }
    }

    #[test]
    fn choosing_the_adafruit_rp2040_puts_a_drive_it_cannot_select_back_to_a() {
        let mut app = offline();
        app.settings.drive = "B".into();
        app.set_kind(Kind::Adafruit);
        assert_eq!(app.settings.drive, "", "gw's default, A");
        app.settings.drive = "0".into();
        app.set_kind(Kind::Greaseweazle);
        app.set_kind(Kind::Adafruit);
        assert_eq!(app.settings.drive, "0");
    }

    /// gw info's report of a device with `model` on `port`.
    fn info_job(port: &str, model: &str) -> Job {
        let log = format!(
            "Host Tools: 1.23\nDevice:\n  Port:     {port}\n  Model:    {model}\n  Firmware: 1.6"
        );
        Job::replay("info", &log)
    }

    #[test]
    fn replacing_a_preset_keeps_its_description_unless_another_is_typed() {
        let folder =
            std::env::temp_dir().join(format!("ferriteweazle-replace-{}", std::process::id()));
        let settings = Settings {
            presets_folder: Some(folder.clone()),
            ..Settings::default()
        };
        let mut app = App::offline(&egui::Context::default(), settings, Err(String::new()));
        let mut saved = |typed: &str| {
            app.save_preset("read", "Mine", typed);
            presets::load(&presets::path(&folder, "Mine"))
                .unwrap()
                .description
        };
        assert_eq!(saved("Both sides"), "Both sides");
        assert_eq!(saved(""), "Both sides", "kept");
        assert_eq!(saved("Side 0"), "Side 0");
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn the_drives_delays_go_once_it_resets_them_takes_new_firmware_or_is_unplugged() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.pin_ports(vec![greaseweazle("COM3", false)]);
        let job = |args: &[&str], log: &str| {
            let mut job = Job::replay(args[0], log);
            job.args = args.iter().map(|a| a.to_string()).collect();
            job
        };
        let report = || job(&["delays", "--device=COM3"], "Select Delay: 10us");
        let run = |app: &mut App, job: Job| {
            app.tool = Some(job);
            app.ended(&ctx, false);
            app.reported_delays().is_some()
        };
        assert!(run(&mut app, report()));
        let reset = job(&["reset", "--device=COM3"], "");
        assert!(run(&mut app, reset), "a reset without --delays keeps them");
        assert!(!run(
            &mut app,
            job(&["reset", "--device=COM3", "--delays"], "")
        ));
        assert!(run(&mut app, report()));
        assert!(!run(&mut app, job(&["update", "--device=COM3"], "")));
        assert!(run(&mut app, report()));
        app.pin_ports(Vec::new());
        app.poll_probe(&ctx);
        app.pin_ports(vec![greaseweazle("COM3", false)]);
        assert!(app.reported_delays().is_none(), "unplugged");
    }

    #[test]
    fn delays_typed_clear_once_the_drive_reports_them() {
        let ctx = egui::Context::default();
        let mut app = offline();
        let mut typed = Values::default();
        typed.set("step", "3000");
        app.settings.values.insert("delays".into(), typed);
        // gw prints a refusal and ends as if it worked.
        app.tool = Some(Job::replay("delays", "Command Failed: Bad Command"));
        app.ended(&ctx, false);
        assert_eq!(
            app.settings.values["delays"].get("step"),
            "3000",
            "kept to try again"
        );
        let report = "Select Delay: 10us\nStep Delay:   3000us";
        app.tool = Some(Job::replay("delays", report));
        app.ended(&ctx, false);
        assert_eq!(
            app.settings.values["delays"].get("step"),
            "",
            "the drive has it"
        );
    }

    #[test]
    fn the_tick_follows_the_device_gw_info_finds_on_the_chosen_port() {
        let ctx = egui::Context::default();
        let mut app = adafruit();
        app.tool = Some(info_job("COM9", "Greaseweazle V4.1"));
        app.ended(&ctx, false);
        assert_eq!(app.settings.kind, Kind::Greaseweazle);
        assert_eq!(app.settings.device, "COM9", "the port chosen stays");

        // The card's own gw info.
        app.probe = Some(info_job("COM9", "Adafruit Floppy Generic"));
        app.poll_probe(&ctx);
        assert_eq!(app.settings.kind, Kind::Adafruit);

        app.tool = Some(info_job("COM3", "Greaseweazle V4.1"));
        app.ended(&ctx, false);
        assert_eq!(app.settings.kind, Kind::Adafruit, "another port's device");
        app.tool = Some(info_job("COM9", "Unknown (0x0900)"));
        app.ended(&ctx, false);
        assert_eq!(
            app.settings.kind,
            Kind::Adafruit,
            "a model gw does not know"
        );

        // gw's own pick becomes the port chosen, as an Adafruit RP2040 needs.
        let mut app = offline();
        app.pin_ports(vec![greaseweazle("COM3", false)]);
        app.tool = Some(info_job("COM3", "Adafruit Floppy Generic"));
        app.ended(&ctx, false);
        assert_eq!(app.settings.kind, Kind::Adafruit);
        assert_eq!(app.settings.device, "COM3");
    }

    #[test]
    fn a_type_ticked_by_hand_drops_a_port_gw_info_found_the_other_type_on() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.pin_ports(vec![greaseweazle("COM3", false)]);
        app.settings.device = "COM3".into();
        app.tool = Some(info_job("COM3", "Greaseweazle V4.1"));
        app.ended(&ctx, false);
        app.choose_kind(Kind::Adafruit);
        assert_eq!(app.settings.kind, Kind::Adafruit);
        assert_eq!(app.settings.device, "", "its port is still to select");
        app.probe = Some(info_job("COM3", "Greaseweazle V4.1"));
        app.poll_probe(&ctx);
        assert_eq!(
            app.settings.kind,
            Kind::Adafruit,
            "the next gw info keeps the tick"
        );

        // A port gw info has not named stays.
        let mut app = adafruit();
        app.choose_kind(Kind::Greaseweazle);
        assert_eq!(app.settings.device, "COM9");
    }

    #[test]
    fn detects_note_goes_once_its_page_takes_another_format() {
        let mut app = offline();
        app.detect_for = Some("read".into());
        let formats = vec!["akai.800".to_owned(), "eagle.dsqd.800".to_owned()];
        app.found(formats.clone(), 1);
        assert!(
            app.notices["read"].starts_with("Found akai.800. Disk also matches eagle.dsqd.800")
        );
        app.drop_found_note();
        assert!(
            app.notices.contains_key("read"),
            "the format is Detect's own"
        );
        let values = app.settings.values.get_mut("read").unwrap();
        values.set("format", "ibm.1440");
        app.drop_found_note();
        assert!(!app.notices.contains_key("read"));

        // Another notice in its place stays.
        app.detect_for = Some("read".into());
        app.found(formats, 1);
        app.notices
            .insert("read".into(), "Read 2 of 2 disks.".into());
        app.settings
            .values
            .get_mut("read")
            .unwrap()
            .set("format", "ibm.720");
        app.drop_found_note();
        assert_eq!(app.notices["read"], "Read 2 of 2 disks.");
    }

    #[test]
    fn a_tool_that_printed_nothing_shows_how_it_ended_as_the_log_does() {
        let mut app = offline();
        app.settings.page = Page::Command("seek".into());
        let mut job = running("seek");
        app.log.begin(heading(&job), &mut job);
        job.ended = Some((std::time::Instant::now(), Outcome::Succeeded));
        app.tool = Some(job);
        app.ended(&egui::Context::default(), false);
        let job = app.tool.as_ref().unwrap();
        assert_eq!(job.log, ["Done in 0:00."]);
        assert_eq!(
            app.log.lines().last().map(String::as_str),
            Some("Done in 0:00.")
        );
        let w = window(app);
        w.get_by_label("Done in 0:00.");
        assert!(
            w.query_by_label("Greaseweazle Tools printed no output.")
                .is_none()
        );
    }

    #[test]
    fn a_verified_write_calls_its_purple_track_verifying_until_it_stops() {
        let mut job = running("write");
        job.progress.verifies = true;
        for line in [
            "Writing c=0-1:h=0",
            "T0.0: Writing Track (Flux: 1)",
            "T1.0: Writing Track (Flux: 1)",
        ] {
            job.progress.feed(line);
        }
        let mut app = offline();
        app.settings.page = Page::Command("write".into());
        app.disk = Some(job);
        let mut w = window(app);
        w.get_by_label("Good 1");
        w.get_by_label("Verifying");
        // Stopped part way: its last track was written but never checked.
        let job = w.state_mut().disk.as_mut().unwrap();
        job.ended = Some((std::time::Instant::now(), Outcome::Stopped));
        w.run_steps(2);
        w.get_by_label("Written 1");
        assert!(w.query_by_label("Verifying").is_none());
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
            score: 0,
            denied: false,
        }]);
        app.settings.device = "/dev/cu.debug-console".into();
        let failed =
            "Host Tools: 1.23\nDevice:\n** FATAL ERROR:\nGreaseweazle interface did not answer.";
        app.probe = Some(Job::replay("info", failed));
        app.poll_probe(&egui::Context::default());
        assert_eq!(
            app.probe_failed.as_deref(),
            Some("Greaseweazle interface did not answer.")
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
        assert_eq!(app.cannot_detect("read").as_deref(), Some(ASKING));
        assert_eq!(app.cannot_detect("convert"), None, "it reads a file");
    }

    #[test]
    fn a_detected_format_names_the_others_the_disk_also_matches() {
        let formats = ["akai.800", "eagle.dsqd.800", "epson.qx10.400"].map(String::from);
        assert_eq!(found_note(&formats[..1], 1, false), "Found akai.800.");
        let forty = "Found akai.800. 40-track disk in an 80-track drive";
        assert_eq!(
            found_note(&formats[..1], 2, true),
            format!("{forty}, setting Step to 2.")
        );
        assert_eq!(
            found_note(&formats[..1], 2, false),
            format!("{forty}."),
            "a list that keeps its own step"
        );
        assert_eq!(
            found_note(&formats[..2], 1, false),
            "Found akai.800. Disk also matches eagle.dsqd.800."
        );
        assert_eq!(
            found_note(&formats, 1, false),
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
            found_note(&atari, 1, false),
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
            batch_label: "pc".into(),
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
    fn a_batch_writes_confirmation_names_its_first_image() {
        let mut app = offline();
        let runs = Runs {
            args: vec![vec!["write".into(), "a.adf".into()]; 2],
            images: vec!["a.adf".into(), "b.adf".into()],
            ..Runs::default()
        };
        app.begin(&egui::Context::default(), "write", runs);
        let w = window(app);
        w.get_by_label("First image: a.adf");
        assert!(w.query_by_label_contains("sequentially").is_none());
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
            ..Runs::default()
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
            label: "Disk".into(),
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
    fn a_fault_shows_on_the_page_it_happened_on() {
        let ctx = egui::Context::default();
        // A folder inside a file cannot be made on any system.
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml/presets");
        let settings = Settings {
            presets_folder: Some(folder),
            ..Settings::default()
        };
        let mut app = App::offline(&ctx, settings, Err(String::new()));
        app.tools = Some(no_gw());
        app.run(&ctx, "erase", Vec::new());
        app.detect_for = Some("convert".into());
        app.run(&ctx, DETECT, Vec::new());
        app.save_preset("seek", "Mine", "");
        app.load_preset("write", Path::new("/no/such/Mine.json"));
        let pages: Vec<&str> = app.notices.keys().map(String::as_str).collect();
        assert_eq!(pages, ["convert", "erase", "seek", "write"]);
        assert!(app.notices["erase"].starts_with("Could not start Greaseweazle Tools: "));
    }

    #[test]
    fn a_gw_that_cannot_be_found_leaves_no_greaseweazle_or_command() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.pin_ports(vec![greaseweazle("/dev/cu.usbmodem14201", false)]);
        assert!(app.connected());
        assert!(!sections(app.listed.as_deref()).is_empty());
        app.settings.tools = Some("/no/such/gw".into());
        app.connect(&ctx);
        app.poll(&ctx);
        assert!(app.tools.is_none());
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
        ctx.set_theme(egui::ThemePreference::Dark);
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
        ctx.set_theme(egui::ThemePreference::Dark);
        assert_eq!(frames(&mut app), [dark], "a theme chosen in Settings too");
        ctx.set_theme(egui::ThemePreference::System);
        assert_eq!(frames(&mut app), [light()]);
    }

    /// What gw prints when Linux refuses it the port: pyserial's EACCES error.
    const REFUSED: &str = "** FATAL ERROR:\n[Errno 13] could not open port /dev/ttyACM0: \
                           [Errno 13] Permission denied: '/dev/ttyACM0'";

    const RULE: &str = "/opt/Ferriteweazle/greaseweazle/49-greaseweazle.rules";

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
    fn an_image_that_needs_the_caps_library_says_where_this_gw_looks_for_it() {
        let ctx = egui::Context::default();
        let mut app = offline();
        let missing = "** FATAL ERROR:\nCould not find SPS/CAPS library\n\
                       For installation instructions please read the wiki:\n\
                       <https://github.com/keirf/greaseweazle/wiki/IPF-Images>";
        app.disk = Some(Job::replay("convert", missing));
        app.ended(&ctx, true);
        let place = match () {
            _ if cfg!(target_os = "macos") => "/Library/Frameworks.",
            _ if cfg!(windows) => "System32.",
            _ => "/usr/lib.",
        };
        let advice = app.log.lines().iter().rev().nth(1).unwrap();
        assert!(advice.ends_with(place), "{:#?}", app.log.lines());
        let error = app.disk.unwrap().progress.error.unwrap_or_default();
        assert!(
            error.ends_with(advice.as_str()),
            "the status pane lacks it: {error}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_caps_library_goes_beside_the_built_in_gws_python() {
        let tools = Tools {
            python: r"C:\Program Files\Ferriteweazle\greaseweazle\python.exe".into(),
            origin: Origin::Bundled,
            standalone: false,
        };
        assert_eq!(
            caps_advice(Some(&tools)),
            r"Greaseweazle Tools looks for CAPSImg_x64.dll or CAPSImg.dll in C:\Program Files\Ferriteweazle\greaseweazle and in System32."
        );
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
        app.tools = Some(no_gw());
        app.pin_ports(vec![greaseweazle("/dev/cu.usbmodem14201", false)]);
        assert_eq!(app.why_not(&schema, info), None);
        app.gw_update = installing();
        assert_eq!(app.why_not(&schema, info).as_deref(), Some(INSTALLING));
        assert_eq!(app.cannot_detect("convert").as_deref(), Some(INSTALLING));
        app.gw_update = Update::Idle;
        app.app_update = installing();
        assert_eq!(app.why_not(&schema, info).as_deref(), Some(INSTALLING));
    }

    #[test]
    fn a_port_list_gw_could_not_make_gives_its_reason_on_the_card_and_the_buttons() {
        use egui::accesskit::Role;
        let mut app = offline();
        app.tools = Some(no_gw());
        app.service = Service::start(&no_gw(), Box::new(|| {}));
        let asked = std::time::Instant::now();
        let why = loop {
            app.service.poll();
            if let Some(why) = app.service.ports_error() {
                break why.to_owned();
            }
            assert!(asked.elapsed() < Duration::from_secs(10), "no reason came");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(app.cannot_detect("read").as_deref(), Some(why.as_str()));
        app.settings.page = Page::Command("info".into());
        let mut w = window(app);
        w.get_by_label("Disconnected");
        w.get_by_label(&why);
        w.get_by_role_and_label(Role::Button, "Get info").hover();
        w.run_steps(4);
        assert_eq!(
            w.get_all_by_label(&why).count(),
            2,
            "not the run button's reason"
        );
        assert!(w.query_by_label(NO_DEVICE).is_none());
    }

    #[test]
    fn a_command_line_gw_cannot_take_holds_up_its_page_until_reset() {
        use egui::accesskit::Role;
        let mut app = offline();
        app.tools = Some(no_gw());
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
        app.tools = Some(no_gw());
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
        app.settings.tools = Some("/no/such/gw".into());
        app.tools = Some(no_gw());
        app.tool = Some(running("info"));
        let mut w = window(app);
        let greyed = |w: &Harness<'_, App>| {
            let choose = w.get_all_by_label("Browse…").last().expect("the gw row's");
            [w.get_by_label("Restart"), choose].map(|b| b.accesskit_node().is_disabled())
        };
        assert_eq!(greyed(&w), [true; 2], "a job runs");
        w.state_mut().tool = None;
        w.run_steps(2);
        assert_eq!(greyed(&w), [false; 2]);
        w.state_mut().gw_update = installing();
        w.run_steps(2);
        assert_eq!(greyed(&w), [true; 2], "gw installs an update");
    }

    #[test]
    fn use_bundled_version_greys_where_there_is_none() {
        // A test program has no greaseweazle folder beside it.
        assert!(!tools::has_bundled());
        let mut app = offline();
        app.settings.page = Page::Settings;
        app.settings.tools = Some("/no/such/gw".into());
        app.tools = Some(no_gw());
        let w = window(app);
        let button = w.get_by_label("Use bundled version");
        assert!(button.accesskit_node().is_disabled());
        w.get_by_label("Greaseweazle Tools 1.23");
    }

    #[test]
    fn a_standalone_gw_greys_detect_and_says_why() {
        let mut app = offline();
        app.tools = Some(Tools {
            standalone: true,
            ..no_gw()
        });
        app.pin_ports(vec![greaseweazle("COM3", false)]);
        assert_eq!(
            app.cannot_detect("read").as_deref(),
            Some(STANDALONE_DETECT)
        );
        assert_eq!(
            app.cannot_detect("convert").as_deref(),
            Some(STANDALONE_DETECT)
        );
        app.tools = Some(no_gw());
        assert_eq!(app.cannot_detect("convert"), None);
    }

    #[test]
    fn with_no_gw_restart_and_update_grey_and_say_why() {
        let settings = Settings {
            page: Page::Settings,
            tools: Some("/no/such/gw".into()),
            ..Settings::default()
        };
        let app = App::with_settings(&egui::Context::default(), settings);
        let mut w = window(app);
        w.get_by_label(NOT_FOUND);
        // The first Update is gw's; the app's own comes later.
        let greyed = |w: &Harness<'_, App>, name| {
            w.get_all_by_label(name)
                .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button)
                .expect(name)
                .accesskit_node()
                .is_disabled()
        };
        assert!(greyed(&w, "Restart") && greyed(&w, "Update"));
        w.state_mut().settings.page = Page::Command("read".into());
        w.run_steps(2);
        w.get_by_label(NO_GW);
        w.state_mut().settings.page = Page::Settings;
        w.run_steps(2);
        // A gw of the person's own runs, but only the built-in one updates.
        w.state_mut().tools = Some(no_gw());
        w.run_steps(2);
        assert!(!greyed(&w, "Restart"));
        assert!(greyed(&w, "Update"));
    }

    #[test]
    fn the_gw_chosen_in_settings_is_kept_and_the_built_in_one_keeps_no_file() {
        let dir = std::env::temp_dir().join(format!("ferriteweazle-gw-{}", std::process::id()));
        let file = dir.join("gw.txt");
        assert_eq!(kept_tools(&file), None);
        keep_tools(&file, Some(Path::new("C:\\Tools\\gw\\gw.exe")));
        assert_eq!(
            kept_tools(&file),
            Some(PathBuf::from("C:\\Tools\\gw\\gw.exe"))
        );
        keep_tools(&file, None);
        assert!(!file.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_check_for_updates_leaves_an_install_under_way() {
        let mut app = offline();
        app.live = true;
        app.tools = Some(Tools {
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
        app.tools = Some(no_gw());
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
    fn a_set_that_asks_names_takes_each_disks_name_as_it_asks_for_the_disk() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join(format!("ferriteweazle-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Lemmings 2.adf"), b"").unwrap();
        let mut app = offline();
        let out = Output {
            folder: dir.to_string_lossy().into(),
            name: "Lemmings 1".into(),
            ext: ".adf".into(),
            disks: 3,
            ask_names: true,
            ..Output::default()
        };
        app.settings.outputs.insert("read/file".into(), out);
        let runs = Runs {
            ask_names: true,
            ..reads(&["", "", ""])
        };
        app.begin(&ctx, "read", runs);
        let first = match &app.dialog {
            Some(Dialog::NextDisk { disk, default, .. }) => (*disk, default.as_str()),
            _ => panic!("no first disk"),
        };
        assert_eq!(
            first,
            (Some(1), "Lemmings 1"),
            "the page's name, if none is typed"
        );
        let mut w = window(app);
        w.get_by_label("Insert the first disk in the drive.");
        let app = w.state_mut();
        app.name_next("Lemmings 1");
        let session = app.session.as_ref().unwrap();
        let file = dir.join("Lemmings 1.adf").to_string_lossy().into_owned();
        assert_eq!(session.runs.args[0].last(), Some(&file));

        app.dialog = None;
        app.session.as_mut().unwrap().next = 1;
        app.disk = Some(Job::replay(
            "read",
            "Command Failed: GetFluxStatus: No Index",
        ));
        app.ended(&ctx, true);
        w.run_steps(2);
        let read = |w: &Harness<App>| {
            let button = w.get_by_role_and_label(Role::Button, "Read disk 2");
            button.accesskit_node().is_disabled()
        };
        assert!(read(&w), "a name first");
        w.event(egui::Event::Text("Lemmings 2".into()));
        w.run_steps(2);
        assert!(!read(&w));
        w.get_by_label("Lemmings 2.adf exists. Reading replaces it.");

        w.state_mut().end_session();
        let note = "Read 0 of 3 disks. Failed: Lemmings 1. The Log says why.";
        assert_eq!(w.state().notices["read"], note);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_session_whose_next_run_cannot_start_ends_and_names_no_other_job() {
        let ctx = egui::Context::default();
        let mut app = offline();
        app.tools = Some(no_gw());
        app.disk = Some(Job::replay("erase", ""));
        app.begin(&ctx, "read", reads(&["a.adf", "b.adf"]));
        assert!(app.session.is_none());
        assert_eq!(
            app.disk.as_ref().unwrap().part,
            None,
            "the erase is no disk 1"
        );
        assert!(app.notices["read"].starts_with("Could not start Greaseweazle Tools: "));
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
        app.tools = Some(no_gw());
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
        w.get_by_label("Greaseweazle Tools stops, then the window closes.");
        w.state_mut().disk = Some(running("read"));
        w.run_steps(2);
        w.get_by_label("Greaseweazle Tools stops the drive first, then the window closes.");
    }

    #[test]
    fn every_update_asks_first_and_the_bootloader_warns() {
        let schema = schema();
        let update = schema.command("update").unwrap();
        let ctx = egui::Context::default();
        let mut app = offline();
        app.start(&ctx, update);
        let firmware = |args: &Vec<String>| !args.contains(&"--bootloader".to_owned());
        assert!(matches!(&app.dialog, Some(Dialog::Confirm { args, .. }) if firmware(args)));
        let w = window(app);
        w.get_by_label("Update the firmware?");
        w.get_by_label("Are you sure?");
        let mut app = offline();
        let values = app.settings.values.entry("update".into()).or_default();
        values.set("bootloader", command::ON);
        app.start(&ctx, update);
        let asks = |args: &Vec<String>| args.contains(&"--bootloader".to_owned());
        assert!(matches!(&app.dialog, Some(Dialog::Confirm { args, .. }) if asks(args)));
        let w = window(app);
        w.get_by_label("Update the bootloader?");
        w.get_by_label(
            "Warning! If the flash fails, the Greaseweazle may need to be reflashed with a \
             programming adapter.",
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
            w.get_by_label("Stop Greaseweazle Tools.");
            w.get_by_label(warning);
            w.event(egui::Event::PointerGone);
            w.state_mut().dialog = Some(Dialog::Quit);
            w.run_steps(2);
            w.get_by_label("Stop Update firmware and quit?");
            w.get_by_label("Greaseweazle Tools stops, then the window closes.");
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
    fn a_job_that_works_or_fails_has_a_sound_on_every_system_and_a_stopped_one_none() {
        let first = |outcome| {
            let players = players(Some(outcome));
            let args = players.first().map(|p| p.get_args().collect::<Vec<_>>());
            args.map(|a| a.join(std::ffi::OsStr::new(" ")))
        };
        let (worked, failed) = (first(Outcome::Succeeded), first(Outcome::Failed));
        assert!(worked.is_some() && failed.is_some());
        assert_ne!(worked, failed, "one sound for both");
        assert!(players(Some(Outcome::Stopped)).is_empty());
        assert!(players(None).is_empty());
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

    #[test]
    fn write_waits_for_an_image_that_is_there() {
        let schema = schema();
        let write = schema.command("write").unwrap();
        let mut app = offline();
        app.tools = Some(no_gw());
        app.pin_ports(vec![greaseweazle("COM3", false)]);
        let values = app.settings.values.entry("write".into()).or_default();
        values.set("file", "/no/such/Game.adf");
        let why = app.why_not(&schema, write);
        assert_eq!(why.as_deref(), Some("The image file does not exist."));
    }

    #[test]
    fn a_page_that_needs_a_device_asks_for_one_before_its_own_settings() {
        let schema = schema();
        let mut app = offline();
        app.tools = Some(no_gw());
        for name in ["read", "write", "seek", "pin set"] {
            let why = app.why_not(&schema, schema.command(name).unwrap());
            assert_eq!(why.as_deref(), Some(NO_DEVICE), "{name}");
        }
        // What the Adafruit RP2040 never runs says so first.
        app.settings.kind = Kind::Adafruit;
        let why = app.why_not(&schema, schema.command("erase").unwrap());
        assert_eq!(why.as_deref(), adafruit::command("erase"));
        assert!(why.is_some());
    }

    #[test]
    fn a_banner_coming_above_the_form_leaves_its_box_focused() {
        let mut app = offline();
        app.stuck = None;
        let mut w = window(app);
        w.get_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .expect("a box")
            .click();
        w.run_steps(2);
        let focused = w.ctx.memory(|m| m.focused());
        assert!(focused.is_some());
        w.state_mut().app_update = Update::Newer("v9.9.0".into());
        w.run_steps(2);
        w.get_by_label("Ferriteweazle 9.9.0 is available.");
        assert_eq!(w.ctx.memory(|m| m.focused()), focused);
    }

    #[test]
    fn detect_takes_double_step_away_from_a_disk_that_needs_none() {
        let mut app = offline();
        app.detect_for = Some("read".into());
        app.found(vec!["ibm.360".into()], 2);
        assert_eq!(app.settings.values["read"].get("tracks"), "step=2");
        app.detect_for = Some("read".into());
        app.found(vec!["ibm.1440".into()], 1);
        assert_eq!(app.settings.values["read"].get("tracks"), "");
        let note = &app.notices["read"];
        assert!(note.contains("setting Step to 1"), "{note}");
    }

    #[test]
    fn detect_on_write_leaves_the_drives_step_alone() {
        let mut app = offline();
        let values = app.settings.values.entry("write".into()).or_default();
        values.set("tracks", "step=2");
        app.detect_for = Some("write".into());
        // Its step is the image's.
        app.found(vec!["ibm.360".into()], 1);
        assert_eq!(app.settings.values["write"].get("tracks"), "step=2");
        assert_eq!(app.notices["write"], "Found ibm.360.");
    }

    #[test]
    fn a_read_in_passes_tells_the_bridge_how_many_what_to_reread_and_where_to_keep_them() {
        let out = |passes, whole_disk, keep_passes| Output {
            passes,
            whole_disk,
            keep_passes,
            ..Output::default()
        };
        let env = |out: Output, image: &str| pass_env(Some(&out), Some(Path::new(image)));
        let kept = |name: &str| {
            let prefix = Path::new("/f").join("Read passes").join(name);
            prefix.to_string_lossy().into_owned()
        };
        assert!(env(out(1, true, true), "/f/Game.img").is_empty());
        assert_eq!(
            env(out(2, false, false), "/f/Game.img"),
            [("FERRITEWEAZLE_PASSES", "2".to_owned())]
        );
        assert_eq!(
            env(out(3, true, true), "/f/Game_Disk1.img"),
            [
                ("FERRITEWEAZLE_PASSES", "3".to_owned()),
                ("FERRITEWEAZLE_REREAD", "disk".to_owned()),
                ("FERRITEWEAZLE_KEEP", kept("Game_Disk1 pass")),
            ]
        );
        let stream = env(out(2, false, true), "/f/Floppy00.0.raw");
        assert_eq!(stream[1], ("FERRITEWEAZLE_KEEP", kept("Floppy pass")));
    }

    #[test]
    fn a_read_in_passes_names_its_pass() {
        let mut app = offline();
        app.disk = Some(Job::replay(
            "read",
            "Reading c=0-1:h=0-1 revs=2\nPass 2 of 3: 1 track",
        ));
        window(app).get_by_label("Read disk, pass 2 of 3");
    }

    #[test]
    fn a_page_takes_the_cylinders_of_a_format_from_its_image_type_or_from_detect() {
        use egui::accesskit::Role;
        let mut app = offline();
        app.settings.page = Page::Command("read".into());
        let out = Output {
            folder: "/f".into(),
            ext: ".adf".into(),
            ..Output::default()
        };
        app.settings.outputs.insert("read/file".into(), out);
        let read = app.settings.values.entry("read".into()).or_default();
        read.set("tracks", "c=0-9:h=0");
        let mut w = window(app);
        let tracks = |w: &Harness<App>| w.state().settings.values["read"].get("tracks").to_owned();
        assert_eq!(tracks(&w), "c=0-9:h=0", "set by hand");
        w.get_all_by_role(Role::ComboBox)
            .find(|c| c.value().is_some_and(|v| v.contains("(.adf)")))
            .unwrap()
            .click();
        w.run_steps(2);
        w.get_by_label_contains("(.d64)").click();
        w.run_steps(2);
        assert_eq!(tracks(&w), "", "a D64's format");

        let read = w.state_mut().settings.values.get_mut("read").unwrap();
        read.set("tracks", "c=0-9:h=0");
        w.run_steps(2);
        let app = w.state_mut();
        app.detect_for = Some("read".into());
        app.found(vec!["ibm.360".into()], 2);
        w.run_steps(2);
        assert_eq!(tracks(&w), "step=2", "Detect's format, and its double step");
    }

    #[test]
    fn the_maps_grid_is_the_formats_disk_and_with_no_format_the_tracks_read() {
        let mut app = offline();
        let read = app.settings.values.entry("read".into()).or_default();
        read.set("tracks", "c=0-39");
        let (_, disk, blank) = app.blank_map("read");
        assert_eq!((disk, blank.cyls.len()), ((0, 0), 40));
        app.service.describe("ibm.1440", 80, 2);
        let read = app.settings.values.get_mut("read").unwrap();
        read.set("format", "ibm.1440");
        assert_eq!(app.blank_map("read").1, (80, 2));
    }

    #[test]
    fn a_preset_or_a_pasted_command_keeps_the_track_list_it_brings() {
        let schema = schema();
        let mut app = offline();
        app.settings.page = Page::Command("read".into());
        let read = app.settings.values.entry("read".into()).or_default();
        read.set("format", "ibm.1440");
        let mut w = window(app);
        let tracks = |w: &Harness<App>| w.state().settings.values["read"].get("tracks").to_owned();
        let line = "gw read --format=ibm.720 --tracks=c=0-9 /d/x.img";
        let (name, values, _) = command::parse(&schema, line).unwrap();
        w.state_mut().fill_in(name, values);
        w.run_steps(2);
        assert_eq!(tracks(&w), "c=0-9");
        let preset = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("presets")
            .join("Read Amiga 880 KB, 84 cyl.json");
        w.state_mut().load_preset("read", &preset);
        w.run_steps(2);
        assert_eq!(tracks(&w), "c=0-83");
    }

    #[test]
    fn a_pasted_no_clobber_is_left_to_the_overwrite_question() {
        let schema = schema();
        let read = schema.command("read").unwrap();
        let mut app = offline();
        let line = "gw read --format=ibm.1440 -n /d/x.img";
        let (name, values, _) = command::parse(&schema, line).unwrap();
        app.fill_in(name, values);
        let args = app.args(read);
        assert!(args.iter().all(|a| a != "-n"), "{args:?}");
        assert!(args.iter().any(|a| a == "--format=ibm.1440"), "{args:?}");
    }

    #[test]
    fn a_pasted_conversion_keeps_the_name_it_gives_its_image() {
        let schema = schema();
        let mut app = offline();
        let (name, values, _) = command::parse(&schema, "gw convert /d/a.scp /o/b.adf").unwrap();
        app.fill_in(name, values);
        let w = window(app);
        assert_eq!(w.state().settings.outputs["convert/out_file"].name, "b");
    }

    #[test]
    fn detect_reads_as_its_page_does_but_takes_only_a_writes_image() {
        let schema = schema();
        let mut settings = Settings {
            drive: "B".into(),
            ..Settings::default()
        };
        let pages = [
            (
                "read",
                &[
                    ("tracks", "c=0-39:h0.off=+2:hswap"),
                    ("fake_index", "300rpm"),
                    ("adjust_speed", "360rpm"),
                    ("densel", "H"),
                    ("reverse", command::ON),
                    ("revs", "5"),
                ][..],
            ),
            (
                "convert",
                &[
                    ("in_file", "/d/x.scp"),
                    ("tracks", "hswap"),
                    ("hard_sectors", command::ON),
                    ("reverse", command::ON),
                ],
            ),
            (
                "write",
                &[
                    ("file", "/d/y.scp"),
                    ("tracks", "hswap"),
                    ("densel", "H"),
                    ("reverse", command::ON),
                ],
            ),
        ];
        for (page, pairs) in pages {
            let values = settings.values.entry(page.into()).or_default();
            pairs
                .iter()
                .for_each(|(dest, value)| values.set(dest, *value));
        }
        let app = App::offline(&egui::Context::default(), settings, Ok(schema.clone()));
        let args = |page| app.detect_args(schema.command(page).unwrap());
        assert_eq!(
            args("read"),
            [
                "--drive=B",
                "--tracks=c=0-39:h0.off=+2:hswap",
                "--fake-index=300rpm",
                "--adjust-speed=360rpm",
                "--densel=H",
                "--reverse"
            ]
        );
        assert_eq!(
            args("convert"),
            ["--tracks=hswap", "--hard-sectors", "--reverse", "/d/x.scp"]
        );
        assert_eq!(args("write"), ["/d/y.scp"], "they are for the disk written");
    }

    #[test]
    fn a_read_that_carries_on_a_set_counts_its_disks_from_the_first() {
        let schema = schema();
        let read = schema.command("read").unwrap();
        let out = Output {
            folder: "/f".into(),
            name: "Game".into(),
            label: "Disk".into(),
            ext: ".adf".into(),
            disks: 7,
            first: 4,
            ..Output::default()
        };
        let outputs = BTreeMap::from([("read/file".to_owned(), out)]);
        let runs = runs(read, Values::default(), &outputs, &[], |v| {
            command::argv(read, v)
        });
        let last: Vec<&str> = runs
            .args
            .iter()
            .map(|r| r.last().unwrap().as_str())
            .collect();
        let sep = std::path::MAIN_SEPARATOR;
        let expected: Vec<String> = (4..=7)
            .map(|d| format!("/f{sep}Game_Disk{d}.adf"))
            .collect();
        assert_eq!(last, expected);
        assert_eq!(runs.makes.len(), 4);
        assert_eq!(runs.number(0), (4, 7), "Read disk 4 of 7");

        let ctx = egui::Context::default();
        let mut app = offline();
        app.session = Some(Session {
            command: "read".into(),
            runs,
            next: 1,
            failed: vec![0],
        });
        app.disk = Some(Job::replay("read", ""));
        app.ended(&ctx, true);
        let next = match &app.dialog {
            Some(Dialog::NextDisk { disk, total, .. }) => (*disk, *total),
            _ => panic!("no next disk"),
        };
        assert_eq!(next, (Some(5), 7), "Insert disk 5 of 7");
        app.session.as_mut().unwrap().next = 4;
        app.end_session();
        let note = "Read 3 of 4 disks. Failed: disk 4. The Log says why.";
        assert_eq!(app.notices["read"], note);
    }

    #[test]
    fn a_set_carried_on_from_disk_4_offers_its_own_numbers_again_and_next() {
        let ctx = egui::Context::default();
        let mut app = offline();
        let runs = Runs {
            args: vec![vec!["read".into()]; 4],
            before: 3,
            ..Runs::default()
        };
        app.session = Some(Session {
            command: "read".into(),
            runs,
            next: 2,
            failed: Vec::new(),
        });
        app.disk = Some(Job::replay(
            "read",
            "Command Failed: GetFluxStatus: No Index",
        ));
        app.ended(&ctx, true);
        let shown = match &app.dialog {
            Some(Dialog::NextDisk {
                disk,
                total,
                failed,
                ..
            }) => (*disk, *total, *failed),
            _ => panic!("no next disk"),
        };
        assert_eq!(
            shown,
            (Some(6), 7, Some(5)),
            "disk 5 failed; disk 6 of 7 is next"
        );
    }
}

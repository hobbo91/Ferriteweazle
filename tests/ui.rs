//! The window driven the way a person would, over the gw 1.23 schema.

use eframe::egui::{self, ThemePreference, accesskit::Role};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, HarnessBuilder, Node, TestRenderer};
use ferriteweazle::form::Output;
use ferriteweazle::job::{Job, LOG_LINES};
use ferriteweazle::schema::{Port, Schema};
use ferriteweazle::{App, Drawer, Page, Settings};

type Window = Harness<'static, Option<App>>;

const DAMAGED: &str = include_str!("data/convert-damaged.log");

const FOUND: &str =
    "Found akai.800. It also matches eagle.dsqd.800, epson.qx10.400 and zx.quorum.ds80.";

/// Height in points of the firmware line a connected device adds to the device card.
const CARD_LINE: f32 = 21.0;

/// The smallest the window goes.
const SMALLEST: egui::Vec2 = egui::vec2(980.0, 744.0);

/// The window as the app first opens.
const DEFAULT: egui::Vec2 = egui::vec2(1040.0, 744.0);

fn schema() -> Schema {
    serde_json::from_str(include_str!("data/schema-1.23.json")).unwrap()
}

/// The app offline, run until it settles, with `disk` as its last disk job.
fn build(builder: HarnessBuilder<Option<App>>, settings: Settings, disk: Option<Job>) -> Window {
    let mut w = start(builder, settings, disk);
    w.run();
    w
}

/// The app offline after its first frame, with `disk` as its last disk job.
fn start(
    builder: HarnessBuilder<Option<App>>,
    settings: Settings,
    mut disk: Option<Job>,
) -> Window {
    let schema = schema();
    builder.build_ui_state(
        move |ui, app| {
            app.get_or_insert_with(|| {
                let mut app = App::offline(ui.ctx(), settings.clone(), Ok(schema.clone()));
                app.disk = disk.take();
                app
            })
            .show(ui);
        },
        None,
    )
}

fn window_at(size: egui::Vec2, settings: Settings) -> Window {
    build(Harness::builder().with_size(size), settings, None)
}

fn window(settings: Settings) -> Window {
    window_at(egui::vec2(1240.0, 780.0), settings)
}

fn app(w: &Window) -> &App {
    w.state().as_ref().expect("the first frame made the app")
}

fn app_mut(w: &mut Window) -> &mut App {
    w.state_mut()
        .as_mut()
        .expect("the first frame made the app")
}

fn set(settings: &mut Settings, command: &str, dest: &str, value: &str) {
    settings
        .values
        .entry(command.into())
        .or_default()
        .set(dest, value);
}

/// The read page with a format and type chosen, as after Detect.
fn chosen() -> Settings {
    let mut settings = Settings::default();
    set(&mut settings, "read", "format", "amiga.amigados");
    settings.outputs.insert(
        "read/file".into(),
        Output {
            ext: ".adf".into(),
            ..Output::default()
        },
    );
    settings
}

/// The `n`th drop-down: 0 is the sidebar's port picker, 1 the page's format, 2 its image type.
fn combo(w: &Window, n: usize) -> Node<'_> {
    w.get_all_by_role(Role::ComboBox)
        .nth(n)
        .expect("the drop-down")
}

/// Types into the focused field. A field takes the focus as it opens.
fn type_text(w: &Window, text: &str) {
    w.event(egui::Event::Text(text.to_owned()));
}

/// Opens the command line and types `line` over what it holds.
fn type_line(w: &mut Window, line: &str) {
    if w.query_by_label("Command line").is_none() {
        w.get_by_label("CLI").click();
        w.run();
    }
    w.get_by_role(Role::MultilineTextInput).click();
    w.run();
    w.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    w.run();
    type_text(w, line);
    w.run();
}

fn line(w: &Window) -> String {
    w.get_by_role(Role::MultilineTextInput)
        .value()
        .unwrap_or_default()
}

/// The disk map's squares.
fn squares(w: &Window) -> impl Iterator<Item = &egui::epaint::RectShape> {
    let left = w.get_by_label("Disk status").rect().left();
    w.output()
        .shapes
        .iter()
        .filter_map(move |c| match &c.shape {
            egui::Shape::Rect(r)
                if r.rect.left() > left
                    && (r.rect.width() - r.rect.height()).abs() < 0.5
                    && r.rect.width() > 8.0 =>
            {
                Some(r)
            }
            _ => None,
        })
}

/// A read whose first track has just come in.
fn first_track_read(builder: HarnessBuilder<Option<App>>) -> Window {
    let job = Job::replay("read", "Reading c=0-79:h=0-1 revs=2");
    let mut w = build(builder, chosen(), Some(job));
    let disk = app_mut(&mut w).disk.as_mut().unwrap();
    disk.progress
        .feed("T0.0: IBM MFM (18/18 sectors) from Raw Flux (500 flux in 400.00ms)");
    w
}

#[test]
fn typing_a_command_line_fills_in_its_page() {
    let mut w = window(Settings::default());
    type_line(
        &mut w,
        "gw write --drive=B --tracks=c=0-39:h=0 --no-verify game.adf",
    );
    let app = app(&w);
    assert_eq!(app.settings.page, Page::Command("write".into()));
    assert_eq!(app.settings.drive, "B", "the drive moves to the sidebar");
    let values = &app.settings.values["write"];
    assert_eq!(values.get("drive"), "");
    assert_eq!(values.get("tracks"), "c=0-39:h=0");
    assert_eq!(values.get("no_verify"), "on");
    assert_eq!(values.get("file"), "game.adf");
}

#[test]
fn a_bad_command_line_says_what_is_wrong_and_changes_nothing() {
    let mut w = window(Settings::default());
    type_line(&mut w, "gw read --revs=4 --bogus x.img");
    w.get_by_label("gw read has no option --bogus.");
    assert_eq!(app(&w).settings.values["read"].get("revs"), "");
}

#[test]
fn only_a_gw_command_is_taken() {
    let mut w = window(Settings::default());
    type_line(&mut w, "rm -rf ~");
    w.get_by_label("A command starts with gw.");
    assert_eq!(app(&w).settings.page, Page::Command("read".into()));
}

#[test]
fn the_command_line_follows_the_page_as_it_changes() {
    let mut settings = Settings::default();
    set(&mut settings, "read", "revs", "5");
    let mut w = window(settings);
    w.get_by_label("CLI").click();
    w.run();
    assert!(line(&w).starts_with("gw read --revs=5"), "{}", line(&w));
    set(&mut app_mut(&mut w).settings, "read", "revs", "2");
    w.run();
    assert!(line(&w).starts_with("gw read --revs=2"), "{}", line(&w));
    w.get_by_label("CLI").click();
    w.run();
    assert!(
        w.query_by_label("Command line").is_none(),
        "CLI hides it again"
    );
}

#[test]
fn a_preset_is_a_file_that_brings_back_the_settings_it_saved() {
    let folder = std::env::temp_dir().join(format!("fw-ui-presets-{}", std::process::id()));
    std::fs::remove_dir_all(&folder).ok();
    let mut settings = Settings {
        presets_folder: Some(folder.clone()),
        ..Settings::default()
    };
    set(&mut settings, "read", "revs", "5");
    let mut w = window(settings);
    w.get_by_label("Presets").click();
    w.run();
    w.get_by_label("No presets saved yet.");
    assert!(!folder.exists(), "the folder waits for a preset");
    // The menu's, not the output log's.
    w.get_all_by_label("Save…").last().unwrap().click();
    w.run();
    type_text(&w, "Five revs");
    w.run();
    w.get_by_label("Save").click();
    w.run();
    assert!(folder.join("Five revs.json").is_file());

    set(&mut app_mut(&mut w).settings, "read", "revs", "2");
    w.run();
    w.get_by_label("Presets").click();
    w.run();
    w.get_by_label("Five revs").click();
    w.run();
    assert_eq!(app(&w).settings.values["read"].get("revs"), "5");
    std::fs::remove_dir_all(folder).ok();
}

#[test]
fn the_status_pane_says_what_has_not_happened_to_a_disk_yet() {
    let w = window(Settings::default());
    w.get_by_label("Disk status");
    w.get_by_label("No disk read yet");
    assert!(
        w.query_by_label("gw's output appears here.").is_none(),
        "the log is in its own drawer"
    );
    let w = window(Settings {
        page: Page::Command("write".into()),
        ..Settings::default()
    });
    w.get_by_label("No disk written yet");
}

#[test]
fn a_read_starts_with_no_format_or_image_type() {
    let w = window(Settings::default());
    let shown: Vec<_> = w
        .get_all_by_role(Role::ComboBox)
        .map(|c| c.value().unwrap_or_default())
        .collect();
    assert_eq!(shown[1..3], ["Choose disk format", "Choose image type"]);
    assert_eq!(app(&w).settings.values["read"].get("file"), "");
}

#[test]
fn choosing_a_format_picks_the_image_type_that_suits_it() {
    let mut w = window(Settings::default());
    combo(&w, 1).click();
    w.run();
    type_text(&w, "amigados");
    w.run();
    w.get_by_label("amiga.amigados").click();
    w.run();
    let app = app(&w);
    assert_eq!(app.settings.values["read"].get("format"), "amiga.amigados");
    assert_eq!(app.settings.outputs["read/file"].ext, ".adf");
    let file = app.settings.values["read"].get("file");
    assert!(file.ends_with("Floppy.adf"), "{file}");
}

#[test]
fn every_page_draws() {
    let pages = schema().commands.into_iter().map(|c| Page::Command(c.name));
    for page in pages.chain([Page::Settings]) {
        let w = window(Settings {
            page: page.clone(),
            ..Settings::default()
        });
        assert_eq!(app(&w).settings.page, page);
    }
}

/// A Greaseweazle as gw lists it, on a made-up port.
fn greaseweazle() -> Port {
    Port {
        device: "/dev/cu.usbmodem14201".into(),
        name: Some("Greaseweazle".into()),
        serial: Some("GW0123456789ABCDEF".into()),
        score: 20,
    }
}

/// The sidebar's entry for a page.
fn entry<'w>(w: &'w Window, title: &'w str) -> Node<'w> {
    w.get_all_by_role_and_label(Role::Button, title)
        .find(|n| n.rect().left() < 60.0)
        .expect("the sidebar entry")
}

fn greyed(w: &Window, title: &str) -> bool {
    entry(w, title).accesskit_node().is_disabled()
}

/// Pages that only act on the Greaseweazle.
const DEVICE_PAGES: [&str; 11] = [
    "Erase disk",
    "Clean heads",
    "Seek",
    "Drive speed",
    "Device info",
    "Update firmware",
    "Delays",
    "Read pin",
    "Set pin",
    "Reset",
    "USB bandwidth",
];

#[test]
fn pages_that_only_act_on_the_device_grey_out_until_one_is_connected() {
    let mut w = window(Settings::default());
    for title in DEVICE_PAGES {
        assert!(greyed(&w, title), "{title} opens");
    }
    for title in ["Read disk", "Write disk", "Convert image"] {
        assert!(!greyed(&w, title), "{title} is greyed");
    }
    entry(&w, "Erase disk").hover();
    w.run();
    w.get_by_label("Connect a Greaseweazle.");
    entry(&w, "Erase disk").click();
    w.run();
    assert_eq!(app(&w).settings.page, Page::Command("read".into()));

    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    w.run();
    for title in DEVICE_PAGES {
        assert!(!greyed(&w, title), "{title} is greyed");
    }
    entry(&w, "Erase disk").click();
    w.run();
    assert_eq!(app(&w).settings.page, Page::Command("erase".into()));

    // The page it is on stays open when it goes.
    app_mut(&mut w).pin_ports(Vec::new());
    w.run();
    assert_eq!(app(&w).settings.page, Page::Command("erase".into()));
    assert!(greyed(&w, "Erase disk"));
}

#[test]
fn detect_needs_the_device_on_the_read_page_only() {
    let detect_greyed = |w: &Window| w.get_by_label("Detect").accesskit_node().is_disabled();
    let mut w = window(chosen());
    assert!(detect_greyed(&w), "it reads the disk in the drive");
    w.get_by_label("Detect").hover();
    w.run();
    w.get_by_label("Connect a Greaseweazle.");
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    w.run();
    assert!(!detect_greyed(&w));

    let mut settings = Settings {
        page: Page::Command("convert".into()),
        ..Settings::default()
    };
    set(&mut settings, "convert", "in_file", "/d/Game.scp");
    let w = window(settings);
    assert!(!detect_greyed(&w), "it reads the image");
}

#[test]
fn until_gw_describes_itself_the_sidebar_lists_no_command() {
    let mut w = Harness::builder().with_size(DEFAULT).build_ui_state(
        |ui, app: &mut Option<App>| {
            let error = Err("Starting.".to_owned());
            app.get_or_insert_with(|| App::offline(ui.ctx(), Settings::default(), error))
                .show(ui);
        },
        None,
    );
    w.run();
    w.get_by_label("Settings");
    let names = ["Read disk", "Erase disk", "Align heads", "USB bandwidth"];
    for name in names.into_iter().chain(["Disk", "Drive", "Device"]) {
        assert!(w.query_by_label(name).is_none(), "{name} is listed");
    }
}

#[test]
fn a_long_notice_wraps_and_keeps_its_dismiss_button_in_view() {
    let mut w = window(Settings::default());
    app_mut(&mut w).notices.insert("read".into(), FOUND.into());
    w.run();
    let dismiss = w.get_by_label("Dismiss").rect();
    let status = w.get_by_label("Disk status").rect();
    assert!(
        dismiss.right() < status.left(),
        "Dismiss at {dismiss:?} runs into the status pane at {status:?}"
    );
    let image_type = combo(&w, 2).rect();
    assert!(
        dismiss.right() <= image_type.right(),
        "the notice ends past the fields: {dismiss:?}, {image_type:?}"
    );
    w.get_by_label("Dismiss").click();
    w.run();
    assert!(app(&w).notices.is_empty());
}

#[test]
fn a_notice_shows_only_on_its_page_and_stays_until_dismissed() {
    let mut w = window(Settings::default());
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    app_mut(&mut w).notices.insert("read".into(), FOUND.into());
    w.run();
    w.get_by_label(FOUND);
    for title in ["Write disk", "Update firmware"] {
        entry(&w, title).click();
        w.run();
        assert!(w.query_by_label(FOUND).is_none(), "it shows on {title}");
    }
    entry(&w, "Read disk").click();
    w.run();
    w.get_by_label(FOUND);
    w.get_by_label("Dismiss").click();
    w.run();
    assert!(w.query_by_label(FOUND).is_none());
    assert!(app(&w).notices.is_empty());
}

#[test]
fn fields_share_one_height_and_end_at_one_right_edge() {
    let w = window(chosen());
    let (format, image_type) = (combo(&w, 1).rect(), combo(&w, 2).rect());
    let inputs: Vec<_> = w
        .get_all_by_role(Role::TextInput)
        .map(|t| t.rect())
        .collect();
    let (folder, name) = (inputs[0], inputs[1]);
    let detect = w.get_by_label("Detect").rect();
    let browse = w.get_by_label("Browse").rect();
    for (what, r) in [
        ("format", format),
        ("image type", image_type),
        ("folder", folder),
        ("name", name),
        ("Detect", detect),
        ("Browse", browse),
    ] {
        assert!((r.height() - 28.0).abs() < 0.5, "{what} is {r:?}");
    }
    let edge = image_type.right();
    for (what, r) in [("Detect", detect), ("Browse", browse), ("name", name)] {
        assert!(
            (r.right() - edge).abs() < 0.5,
            "{what} ends at {r:?}, not {edge}"
        );
    }
    let side_1 = w
        .get_all_by_label("1")
        .last()
        .expect("the track picker's side 1")
        .rect();
    assert!(
        side_1.right() <= edge + 0.5,
        "the track picker runs past the fields: {side_1:?}, {edge}"
    );
}

#[test]
fn every_field_and_its_label_explain_themselves_on_hover() {
    let mut w = window(chosen());
    w.get_by_label("Revolutions").hover();
    w.run();
    w.get_by_label("Revolutions to read per track.");
    combo(&w, 3).hover();
    w.run();
    w.get_by_label("Revolutions to read per track.");
}

#[test]
fn a_button_in_a_field_shows_its_own_tooltip_alone() {
    let mut w = window(chosen());
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    w.run();
    w.get_by_label("Detect").hover();
    w.run();
    w.get_by_label("Attempt to find the disk format and image type.");
    assert!(
        w.query_by_label_contains("The disk's format").is_none(),
        "two tooltips at once"
    );
}

#[test]
fn the_smallest_window_keeps_the_page_clear_of_the_status_pane() {
    let w = window_at(SMALLEST, chosen());
    let image_type = combo(&w, 2).rect();
    let status = w.get_by_label("Disk status").rect();
    assert!(
        image_type.right() < status.left(),
        "the fields run under the status pane: {image_type:?}, {status:?}"
    );
    assert!(
        image_type.width() > 150.0,
        "the fields are squeezed: {image_type:?}"
    );
}

#[test]
fn the_log_and_the_command_line_share_a_drawer_across_the_page_and_the_status_pane() {
    let mut w = window(Settings::default());
    let status = w.get_by_label("Disk status").rect();
    w.get_by_role_and_label(Role::Button, "Log").click();
    w.run();
    let log = w.get_by_label("gw's output appears here.").rect();
    let copy = w.get_by_label("Copy").rect();
    assert!(
        copy.left() > status.left(),
        "it runs under the status pane: {copy:?}"
    );
    let run = w
        .get_all_by_role_and_label(Role::Button, "Read disk")
        .last()
        .unwrap()
        .rect();
    assert!(
        log.top() > run.bottom(),
        "below the run button: {log:?}, {run:?}"
    );

    // It stays open from page to page.
    w.get_by_label("Write disk").click();
    w.run();
    w.get_by_label("gw's output appears here.");

    // The command line takes its place: only one is open, at the same height.
    let log_top = w.get_by_label("Copy").rect().top();
    w.get_by_label("CLI").click();
    w.run();
    w.get_by_label("Command line");
    assert!(w.query_by_label("gw's output appears here.").is_none());
    let cli_top = w.get_by_label("Copy").rect().top();
    assert!(
        (log_top - cli_top).abs() < 1.0,
        "log at {log_top}, CLI at {cli_top}"
    );

    // Settings is a page of its own, with no drawer.
    w.get_by_label("Settings").click();
    w.run();
    assert!(w.query_by_label("Command line").is_none());
    w.get_by_label("Read disk").click();
    w.run();
    w.get_by_label("Command line");
}

/// The window at its first size with a read done, a frame every 60th of a
/// second, after its first frame, with egui's animations on as in the app.
fn smooth(settings: Settings) -> Window {
    let builder = Harness::builder()
        .with_size(DEFAULT)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60);
    let w = start(builder, settings, Some(Job::replay("read", DAMAGED)));
    // egui's own default: kittest turns animations off.
    w.ctx.all_styles_mut(|s| s.animation_time = 0.2);
    w
}

/// The tops of the page's run button and the map's legend. A sliding drawer
/// is only painted where it goes, so the page above it shows where it is.
fn edges(w: &Window) -> [f32; 2] {
    let run = w
        .get_all_by_role_and_label(Role::Button, "Read disk")
        .last()
        .unwrap();
    [
        run.rect().top(),
        w.get_by_label_contains("Good ").rect().top(),
    ]
}

/// Clicks `drawer`'s button and returns the edges of each frame after.
fn toggle(w: &mut Window, drawer: &str) -> Vec<[f32; 2]> {
    w.get_by_role_and_label(Role::Button, drawer).click();
    (0..20)
        .map(|_| {
            w.step();
            edges(w)
        })
        .collect()
}

/// Whether the run button moves one way over several frames and ends at `to`.
fn slides(frames: &[[f32; 2]], from: f32, to: f32) -> bool {
    let tops: Vec<f32> = frames.iter().map(|f| f[0]).collect();
    let steps = tops.windows(2).filter(|p| p[0] != p[1]).count();
    let one_way = tops.windows(2).all(|p| (p[1] - p[0]) * (to - from) >= 0.0);
    one_way && steps >= 5 && tops[0] != to && tops.last() == Some(&to) && from != to
}

#[test]
fn a_drawer_slides_open_and_shut_and_the_map_stays_where_it_fits() {
    let mut w = smooth(chosen());
    w.run();
    let [closed, legend] = edges(&w);
    let opening = toggle(&mut w, "Log");
    let open = opening.last().unwrap()[0];
    assert!(
        slides(&opening, closed, open),
        "{closed} to {open}: {opening:?}"
    );
    assert!(
        opening.iter().all(|f| f[1] == legend),
        "the map moved: {opening:?}"
    );
    assert_eq!(w.run(), 1, "the window keeps drawing");

    let shutting = toggle(&mut w, "Log");
    assert!(
        slides(&shutting, open, closed),
        "{open} to {closed}: {shutting:?}"
    );
    assert!(
        shutting.iter().all(|f| f[1] == legend),
        "the map moved: {shutting:?}"
    );
    assert_eq!(w.run(), 1, "the window keeps drawing");
}

#[test]
fn a_window_that_opens_with_a_drawer_or_switches_drawers_does_not_slide_one() {
    let mut w = smooth(Settings {
        drawer: Some(Drawer::Cli),
        ..chosen()
    });
    w.step();
    let cli = edges(&w);
    w.run();
    assert_eq!(edges(&w), cli, "the drawer slid as the window opened");
    w.get_by_role_and_label(Role::Button, "Log").click();
    w.step();
    w.step();
    assert!(
        w.query_by_label("Command line").is_none(),
        "the command line slid out"
    );
    assert_eq!(edges(&w), cli, "the log slid in");
    w.run();
    assert_eq!(w.run(), 1, "the window keeps drawing");
}

/// Drags the log's top edge up by `by` points and lets the window settle.
fn drag_log(w: &mut Window, by: f32) {
    let heading = w.get_by_role_and_label(Role::Label, "Log").rect();
    // The drawer's top edge, above its heading by the frame's margin.
    let edge = egui::pos2(heading.center().x + 200.0, heading.top() - 12.0);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    w.event(egui::Event::PointerMoved(edge));
    w.step();
    w.event(button(edge, true));
    w.step();
    for i in 1..=10 {
        let at = edge - egui::vec2(0.0, by * i as f32 / 10.0);
        w.event(egui::Event::PointerMoved(at));
        w.step();
    }
    w.event(button(edge - egui::vec2(0.0, by), false));
    w.run();
}

#[test]
fn a_log_dragged_taller_stays_that_tall_and_the_map_shrinks_only_when_it_must() {
    let mut w = smooth(Settings {
        drawer: Some(Drawer::Log),
        ..chosen()
    });
    w.run();
    let [open, legend] = edges(&w);
    // Less than the room the map leaves below it at this size.
    drag_log(&mut w, 20.0);
    let [taller, map] = edges(&w);
    assert!(taller < open - 5.0, "{open} to {taller}");
    assert_eq!(map, legend, "the map shrank with room to spare");
    let settled: Vec<[f32; 2]> = (0..20)
        .map(|_| {
            w.step();
            edges(&w)
        })
        .collect();
    assert!(
        settled.iter().all(|f| f[0] == taller),
        "it slid back: {settled:?}"
    );

    drag_log(&mut w, 400.0);
    let [tallest, map] = edges(&w);
    assert!(tallest < taller && map < legend, "{tallest}, map at {map}");
    let drawer = w.get_by_role_and_label(Role::Label, "Log").rect().top();
    let legend = w.get_by_label_contains("Good ").rect();
    assert!(
        legend.bottom() < drawer,
        "the legend at {legend:?}, the drawer at {drawer}"
    );
}

#[test]
fn with_too_little_room_the_status_pane_scrolls_and_its_rows_keep_their_width() {
    let mut job = Job::replay("read", DAMAGED);
    job.progress.error = Some("The drive did not answer. ".repeat(6));
    let builder = Harness::builder()
        .with_size(DEFAULT)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60);
    let settings = Settings {
        drawer: Some(Drawer::Log),
        ..chosen()
    };
    let mut w = start(builder, settings, Some(job));
    w.run();
    let chip = w.get_by_label_contains("Done ·").rect();
    drag_log(&mut w, 400.0);
    let drawer = w.get_by_role_and_label(Role::Label, "Log").rect().top();
    let legend = w.get_by_label_contains("Good ").rect();
    assert!(legend.bottom() > drawer, "nothing to scroll: {legend:?}");
    let moved = w.get_by_label_contains("Done ·").rect();
    assert_eq!(
        moved.right(),
        chip.right(),
        "the rows narrowed for the scroll bar"
    );

    w.event(egui::Event::PointerMoved(moved.center()));
    w.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -200.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    w.run();
    let scrolled = w.get_by_label_contains("Good ").rect();
    assert!(scrolled.top() < legend.top(), "the pane did not scroll");
}

#[test]
fn clear_empties_the_log() {
    let mut w = window(Settings {
        drawer: Some(Drawer::Log),
        ..Settings::default()
    });
    let mut job = Job::replay("read", "Reading c=0-79:h=0-1 revs=2");
    let log = &mut app_mut(&mut w).log;
    log.begin("gw read x.img".into(), &mut job);
    log.end(&mut job, "Done in 0:01.".into());
    w.run();
    w.get_by_label("Done in 0:01.");
    w.get_by_role_and_label(Role::Button, "Clear").click();
    w.run();
    assert!(w.query_by_label("Done in 0:01.").is_none());
    w.get_by_label("gw's output appears here.");
    let clear = w.get_by_role_and_label(Role::Button, "Clear");
    assert!(clear.accesskit_node().is_disabled(), "nothing to clear");
}

#[test]
fn a_square_fades_in_as_its_track_is_read_then_the_window_rests() {
    let mut w = first_track_read(
        Harness::builder()
            .with_size(SMALLEST)
            .with_step_dt(1.0 / 60.0)
            .with_max_steps(120),
    );
    // Part way through, every square is its full size and nothing is drawn around one.
    w.run_steps(8);
    let rects: Vec<_> = squares(&w)
        .map(|r| {
            assert_eq!(r.stroke.width, 0.0, "an outline around a square: {r:?}");
            r.rect
        })
        .collect();
    assert!(rects.len() >= 160, "{} squares", rects.len());
    let size = rects[0].size();
    assert!(
        rects.iter().all(|r| (r.size() - size).length() < 0.01),
        "a square is not its full size while it fills"
    );
    let steps = w.run();
    assert!(steps > 5, "it filled in at once, in {steps} frames");
    assert_eq!(w.run(), 1, "the window keeps drawing when nothing changes");
}

#[test]
fn with_the_log_open_the_whole_map_still_fits_above_it() {
    let settings = Settings {
        drawer: Some(Drawer::Log),
        ..chosen()
    };
    let w = build(
        Harness::builder().with_size(SMALLEST),
        settings,
        Some(Job::replay("read", DAMAGED)),
    );
    let legend = w.get_by_label_contains("Good ").rect();
    let drawer = w.get_by_label("Copy").rect();
    assert!(
        legend.bottom() < drawer.top(),
        "the map runs into the log: {legend:?}, {drawer:?}"
    );
}

#[test]
fn the_window_as_it_opens_needs_no_scrolling_and_keeps_tracks_on_one_line() {
    let w = window_at(DEFAULT, chosen());
    let bars: Vec<_> = w
        .query_all_by_role(Role::ScrollBar)
        .map(|b| b.rect())
        .collect();
    assert!(bars.is_empty(), "something scrolls: {bars:?}");
    let cylinders = w.get_by_label("Cylinders").rect();
    let side_1 = w.get_all_by_label("1").last().unwrap().rect();
    assert!(
        (side_1.center().y - cylinders.center().y).abs() < 2.0,
        "the sides wrap under the cylinders: {side_1:?}, {cylinders:?}"
    );
    // With no device the card is a line short of its usual height.
    let last = w.get_by_label("USB bandwidth").rect();
    let settings = w.get_by_label("Settings").rect();
    assert!(
        last.bottom() + CARD_LINE <= settings.top(),
        "the sidebar list is cut: {last:?}, {settings:?}"
    );
}

#[test]
fn a_square_lit_after_a_pause_still_fades_from_empty() {
    // Long frames, as when the window has sat idle waiting on gw.
    let mut w = first_track_read(
        Harness::builder()
            .with_size(DEFAULT)
            .with_step_dt(0.3)
            .with_max_steps(20),
    );
    w.step();
    let mut colours: Vec<_> = squares(&w).map(|r| r.fill).collect();
    colours.dedup();
    assert_eq!(colours.len(), 1, "a square jumped ahead on its first frame");
}

#[test]
fn a_long_job_description_wraps_within_the_status_pane() {
    let mut job = Job::replay("read", DAMAGED);
    job.format = Some("commodore.1541".into());
    job.output = Some("/d/Summer Games II side B, the long one.d64".into());
    let w = build(Harness::builder().with_size(DEFAULT), chosen(), Some(job));
    let about = w.get_by_label_contains("Summer Games II").rect();
    let chip = w.get_by_label_contains("Done ·").rect();
    assert!(
        about.right() <= chip.right() + 0.5,
        "it runs out of the pane: {about:?}"
    );
    assert!(
        about.height() > 20.0,
        "it was cut short, not wrapped: {about:?}"
    );
}

#[test]
fn at_its_smallest_the_window_shows_the_whole_sidebar() {
    let w = window_at(SMALLEST, Settings::default());
    let last = w.get_by_label("USB bandwidth").rect();
    let settings = w.get_by_label("Settings").rect();
    assert!(
        last.bottom() + CARD_LINE < settings.top(),
        "the sidebar list is cut: {last:?}, {settings:?}"
    );
}

#[test]
fn device_info_fits_the_window_as_it_opens() {
    let info = "Host Tools: 1.23\nDevice:\n  Port:     /dev/cu.usbmodem1\n  Model:    Greaseweazle V4.1\n  \
                MCU:      AT32F403A, 216MHz, 224kB SRAM\n  Firmware: 1.6\n  Serial:   GW01\n  \
                USB:      Full Speed (12 Mbit/s), 128kB Buffer";
    let settings = Settings {
        page: Page::Command("info".into()),
        ..Settings::default()
    };
    let mut w = window_at(DEFAULT, settings);
    app_mut(&mut w).tool = Some(Job::replay("info", info));
    w.run();
    let usb = w
        .get_by_label("Full Speed (12 Mbit/s), 128kB Buffer")
        .rect();
    let button = w.get_by_label("CLI").rect();
    assert!(
        usb.bottom() < button.top(),
        "the table runs under the buttons: {usb:?}"
    );
    assert!(
        w.query_all_by_role(Role::ScrollBar).next().is_none(),
        "the page scrolls"
    );
}

#[test]
fn next_to_the_input_file_starts_off_and_greys_out_where_it_would_replace_the_input() {
    let mut settings = Settings {
        page: Page::Command("convert".into()),
        ..Settings::default()
    };
    set(&mut settings, "convert", "in_file", "/d/Game.scp");
    set(&mut settings, "convert", "format", "amiga.amigados");
    let mut w = window_at(DEFAULT, settings);
    let beside = w.get_by_role_and_label(Role::CheckBox, "Next to the input file");
    assert_eq!(
        beside.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    app_mut(&mut w)
        .settings
        .outputs
        .get_mut("convert/out_file")
        .unwrap()
        .ext = ".scp".into();
    w.run();
    let beside = w.get_by_role_and_label(Role::CheckBox, "Next to the input file");
    assert!(
        beside.accesskit_node().is_disabled(),
        "an .scp beside Game.scp would replace it"
    );
}

#[test]
fn the_device_card_lines_up_with_the_pages_description() {
    let w = window_at(DEFAULT, Settings::default());
    let about = w
        .get_by_label("Read a disk to the specified image file.")
        .rect();
    let card = w
        .output()
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.rect.left() < 60.0 && r.corner_radius.nw == 10 => {
                Some(r.rect)
            }
            _ => None,
        })
        .expect("the device card");
    // The description's letters start 2 points below its line's top.
    let letters = about.top() + 2.0;
    assert!(
        (card.top() - letters).abs() < 0.5,
        "card at {}, description's letters at {letters}",
        card.top()
    );
}

/// The update page, with the command line open.
fn update_page() -> Window {
    window(Settings {
        page: Page::Command("update".into()),
        drawer: Some(Drawer::Cli),
        ..Settings::default()
    })
}

/// The firmware source that is lit.
fn firmware(w: &Window) -> Vec<&'static str> {
    ["Latest", "Release", "File"]
        .into_iter()
        .filter(|name| {
            let button = w.get_by_role_and_label(Role::Button, name);
            button.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
        })
        .collect()
}

/// Clicks the page's one-line text field and types into it.
fn type_field(w: &mut Window, text: &str) {
    w.get_by_role(Role::TextInput).click();
    w.run();
    type_text(w, text);
    w.run();
}

#[test]
fn update_takes_the_latest_firmware_a_release_or_a_file_and_gives_gw_only_that() {
    let mut w = update_page();
    assert_eq!(firmware(&w), ["Latest"]);
    assert!(
        w.query_by_role(Role::TextInput).is_none(),
        "nothing to fill in"
    );
    assert_eq!(line(&w), "gw update");

    w.get_by_role_and_label(Role::Button, "Release").click();
    w.run();
    assert_eq!(firmware(&w), ["Release"]);
    w.get_by_label("Release tag");
    type_field(&mut w, "v1.6");
    assert_eq!(line(&w), "gw update --tag=v1.6");

    w.get_by_role_and_label(Role::Button, "File").click();
    w.run();
    assert_eq!(
        line(&w),
        "gw update",
        "the tag stays on the page, not in gw's"
    );
    type_field(&mut w, "/x/fw.upd");
    assert_eq!(line(&w), "gw update --file=/x/fw.upd");

    w.get_by_role_and_label(Role::Button, "Release").click();
    w.run();
    assert_eq!(line(&w), "gw update --tag=v1.6");
    w.get_by_role_and_label(Role::Button, "Latest").click();
    w.run();
    assert_eq!(line(&w), "gw update");
}

#[test]
fn a_typed_update_command_chooses_its_firmware_source() {
    let mut w = update_page();
    type_line(&mut w, "gw update --file /x/fw.upd");
    assert_eq!(firmware(&w), ["File"]);
    assert_eq!(app(&w).settings.values["update"].get("file"), "/x/fw.upd");
    type_line(&mut w, "gw update --tag v1.6 --force");
    assert_eq!(firmware(&w), ["Release"]);
    type_line(&mut w, "gw update --force");
    assert_eq!(firmware(&w), ["Latest"]);

    // A typed line wins over the source last chosen on the page.
    w.get_by_role_and_label(Role::Button, "File").click();
    w.run();
    assert_eq!(firmware(&w), ["File"]);
    type_line(&mut w, "gw update --tag v1.7");
    assert_eq!(firmware(&w), ["Release"]);
    assert_eq!(line(&w), "gw update --tag v1.7");
}

/// Settings from `theme`, a frame every 60th of a second.
fn settings_from(theme: ThemePreference, builder: HarnessBuilder<Option<App>>) -> Window {
    let settings = Settings {
        page: Page::Settings,
        theme,
        ..Settings::default()
    };
    let builder = builder
        .with_size(DEFAULT)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60);
    build(builder, settings, None)
}

/// The old theme's picture over the whole window, as it fades: its opacity.
fn fading(w: &Window) -> Option<u8> {
    w.output().shapes.iter().find_map(|c| match &c.shape {
        egui::Shape::Mesh(m)
            if m.texture_id != egui::TextureId::default()
                && m.calc_bounds().width() >= DEFAULT.x - 0.5 =>
        {
            m.vertices.first().map(|v| v.color.a())
        }
        _ => None,
    })
}

#[test]
fn choosing_a_theme_fades_the_old_one_out_then_the_window_rests() {
    for (from, to, shows) in [
        (ThemePreference::Dark, "Light", egui::Theme::Light),
        // The harness's system theme is dark.
        (ThemePreference::Light, "System", egui::Theme::Dark),
    ] {
        let mut w = settings_from(from, Harness::builder());
        w.get_by_label(to).click();
        let mut seen = Vec::new();
        for _ in 0..40 {
            w.step();
            seen.extend(fading(&w));
        }
        assert_eq!(w.ctx.theme(), shows, "to {to}");
        assert!(seen.len() >= 10, "to {to}, over {} frames", seen.len());
        assert_eq!(seen[0], 255, "to {to}, it starts as the old theme");
        assert!(
            seen.windows(2).all(|p| p[1] <= p[0]),
            "to {to}, it does not fade out steadily: {seen:?}"
        );
        assert_eq!(w.run(), 1, "to {to}, the window keeps drawing");
    }
}

#[test]
fn a_long_frame_as_the_fade_begins_does_not_skip_it() {
    let mut w = settings_from(ThemePreference::Dark, Harness::builder());
    let mut time = w.ctx.input(|i| i.time);
    let mut frame = |w: &mut Window, dt: f64| {
        time += dt;
        w.input_mut().time = Some(time);
        w.step();
        fading(w)
    };
    w.get_by_label("Light").click();
    let mut shown = None;
    for _ in 0..10 {
        shown = frame(&mut w, 1.0 / 60.0);
        if shown.is_some() {
            break;
        }
    }
    assert_eq!(shown, Some(255), "the old theme's picture, in full");
    // Loading that picture can hold up the next frame.
    let next = frame(&mut w, 0.4);
    assert!(next.is_some_and(|a| a > 150), "it jumped to {next:?}");
}

/// A renderer that cannot render, so no screenshot comes.
struct Blind;

impl TestRenderer for Blind {
    fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
        delta.clear();
    }

    fn render(
        &mut self,
        _: &egui::Context,
        _: &egui::FullOutput,
    ) -> Result<image::RgbaImage, String> {
        Err("no renderer".into())
    }
}

#[test]
fn with_no_screenshot_to_fade_the_theme_changes_at_once() {
    let mut w = settings_from(ThemePreference::Dark, Harness::builder().renderer(Blind));
    w.get_by_label("Light").click();
    w.run();
    assert_eq!(w.ctx.theme(), egui::Theme::Light);
    assert_eq!(w.run(), 1, "the window keeps drawing");
}

#[test]
fn a_window_too_big_for_one_texture_changes_theme_at_once() {
    // The harness's largest texture is 2048 pixels, the window 2080 wide.
    let builder = Harness::builder().with_pixels_per_point(2.0);
    let mut w = settings_from(ThemePreference::Dark, builder);
    w.get_by_label("Light").click();
    w.run();
    assert_eq!(w.ctx.theme(), egui::Theme::Light);
    assert_eq!(w.run(), 1, "the window keeps drawing");
}

#[test]
fn the_log_says_when_it_has_dropped_its_oldest_lines() {
    let settings = Settings {
        drawer: Some(Drawer::Log),
        ..Settings::default()
    };
    let mut w = window(settings);
    assert!(w.query_by_label("Older lines were dropped.").is_none());
    let lines: Vec<String> = (0..LOG_LINES).map(|i| format!("T{i}")).collect();
    let mut job = Job::replay("read", &lines.join("\n"));
    let log = &mut app_mut(&mut w).log;
    log.begin("gw read x.img".into(), &mut job);
    log.end(&mut job, "Done in 1:00.".into());
    w.run();
    w.get_by_label("Older lines were dropped.");
    w.get_by_label("Done in 1:00.");
}

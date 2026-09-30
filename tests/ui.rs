//! The window driven the way a person would, over the gw 1.23 schema.

mod common;

use common::{
    DAMAGED, DEFAULT, FOUND, REFUSED, Window, app, app_mut, damaged_read, greaseweazle, line,
    run_button, squares,
};
use eframe::egui::{self, ThemePreference, accesskit::Role};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, HarnessBuilder, Node, TestRenderer};
use ferriteweazle::command::Values;
use ferriteweazle::device::Kind;
use ferriteweazle::form::{self, Output};
use ferriteweazle::job::{DETECT, Job, LOG_LINES, Outcome};
use ferriteweazle::presets::{self, Preset};
use ferriteweazle::schema::{Port, Schema};
use ferriteweazle::{App, Drawer, Page, Settings};

/// Height in points of the firmware line a connected device adds to the device card.
const CARD_LINE: f32 = 21.0;

fn schema() -> Schema {
    serde_json::from_str(include_str!("../src/gw-1.23.json")).unwrap()
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
            let app = app.get_or_insert_with(|| {
                let mut app = App::offline(ui.ctx(), settings.clone(), Ok(schema.clone()));
                app.disk = disk.take();
                app
            });
            common::show(ui, app);
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

/// Types into the focused field; a field takes the focus as it opens.
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
    let file = form::images_folder().join("game.adf");
    assert_eq!(values.get("file"), file.to_string_lossy());
}

#[test]
fn a_pasted_line_takes_a_relative_path_in_the_images_folder_and_tilde_as_home() {
    let mut w = window(Settings::default());
    type_line(&mut w, "gw convert ~/in.scp out.img");
    let app = app(&w);
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let input = std::path::PathBuf::from(home.unwrap()).join("in.scp");
    let values = &app.settings.values["convert"];
    assert_eq!(values.get("in_file"), input.to_string_lossy());
    let out = &app.settings.outputs["convert/out_file"];
    assert_eq!(std::path::Path::new(&out.folder), form::images_folder());
    assert_eq!((out.name.as_str(), out.ext.as_str()), ("out", ".img"));
}

#[test]
fn a_bad_command_line_says_what_is_wrong_and_changes_nothing() {
    let mut w = window(Settings::default());
    type_line(&mut w, "gw read --revs=4 --bogus x.img");
    w.get_by_label("gw read has no option --bogus.");
    assert_eq!(app(&w).settings.values["read"].get("revs"), "");
}

/// The command line's Reset, not the sidebar's page of that name.
fn cli_reset(w: &Window) -> egui_kittest::Node<'_> {
    let heading = w.get_by_label("Command line").rect();
    w.get_all_by_label("Reset")
        .find(|b| b.rect().top() > heading.top() - 10.0)
        .expect("the command line's Reset")
}

#[test]
fn a_command_line_gw_cannot_take_stays_until_reset() {
    let mut w = window(Settings::default());
    w.get_by_label("CLI").click();
    w.run();
    let synced = line(&w);
    let reset = |w: &Window| !cli_reset(w).accesskit_node().is_disabled();
    assert!(!reset(&w), "Reset is lit with nothing typed");
    type_line(&mut w, "gw read --revs=4 --bogus x.img");
    w.get_by_label("Command line").click();
    w.run();
    assert_eq!(
        line(&w),
        "gw read --revs=4 --bogus x.img",
        "the typing was lost"
    );
    w.get_by_label("gw read has no option --bogus.");
    assert!(reset(&w));
    cli_reset(&w).click();
    w.run();
    assert_eq!(line(&w), synced);
    assert!(w.query_by_label("gw read has no option --bogus.").is_none());
    assert!(!reset(&w));
}

#[test]
fn a_command_line_longer_than_two_rows_scrolls_in_its_drawer() {
    let mut settings = Settings {
        drawer: Some(Drawer::Cli),
        ..chosen()
    };
    settings.outputs.get_mut("read/file").unwrap().name = "akai_s950_backup_".repeat(20);
    let w = window(settings);
    let heading = w.get_by_label("Command line").rect();
    let bar = w
        .get_all_by_role(Role::ScrollBar)
        .map(|b| b.rect())
        .find(|r| r.top() > heading.bottom() && r.height() > r.width())
        .expect("a scroll bar beside the command");
    let rows = 2.0 * 14.0;
    assert!(bar.height() < rows + 20.0, "{bar:?}");
}

#[test]
fn a_name_takes_no_more_than_its_limit() {
    let mut w = window(chosen());
    let name = w.get_by_label("Name").rect();
    let field = w
        .get_all_by_role(Role::TextInput)
        .find(|f| (f.rect().center().y - name.center().y).abs() < 4.0)
        .expect("the name's field");
    field.click();
    w.run();
    type_text(&w, &"x".repeat(60));
    w.run();
    let name = &app(&w).settings.outputs["read/file"].name;
    assert_eq!(
        name.chars().count(),
        ferriteweazle::form::NAME_LIMIT,
        "{name}"
    );
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
fn a_preset_is_deleted_from_its_menu_after_asking() {
    let folder = std::env::temp_dir().join(format!("fw-ui-delete-{}", std::process::id()));
    std::fs::remove_dir_all(&folder).ok();
    let preset = Preset {
        command: "read".into(),
        ..Preset::default()
    };
    let file = presets::save(&folder, "Five revs", &preset).unwrap();
    let mut w = window(Settings {
        presets_folder: Some(folder.clone()),
        ..Settings::default()
    });
    let choose = |w: &mut Window| {
        w.get_by_label("Presets").click();
        w.run();
        w.get_by_label_contains("Delete").click();
        w.run();
        // The submenu's entry, after the one that loads the preset.
        w.get_all_by_label("Five revs").last().unwrap().click();
        w.run();
    };
    choose(&mut w);
    w.get_by_label("Delete \"Five revs\"?");
    w.get_by_label("This cannot be undone.");
    w.get_by_label("Cancel").click();
    w.run();
    assert!(file.is_file(), "Cancel deleted it");
    choose(&mut w);
    w.get_by_role_and_label(Role::Button, "Delete").click();
    w.run();
    assert!(!file.exists());
    w.get_by_label("Presets").click();
    w.run();
    w.get_by_label("No presets saved yet.");
    assert!(
        w.query_by_label_contains("Delete").is_none(),
        "nothing to delete"
    );
    std::fs::remove_dir_all(folder).ok();
}

#[test]
fn restore_defaults_puts_the_page_back_to_gws_defaults_and_keeps_the_sidebars_choices() {
    let mut settings = Settings {
        images_folder: Some("/disks".into()),
        drive: "B".into(),
        ..chosen()
    };
    set(&mut settings, "read", "revs", "5");
    let out = settings.outputs.get_mut("read/file").unwrap();
    out.name = "Game".into();
    out.disks = 3;
    let mut w = window(settings);
    app_mut(&mut w).notices.insert("read".into(), FOUND.into());
    w.run();
    fn restore(w: &Window) -> Node<'_> {
        w.get_by_role_and_label(Role::Button, "Restore defaults")
    }
    w.get_by_label("Presets").click();
    w.run();
    restore(&w).click();
    w.run();
    let app = app(&w);
    assert_eq!(app.settings.values["read"], Values::default());
    let fresh = Output {
        folder: "/disks".into(),
        ..Output::default()
    };
    assert_eq!(app.settings.outputs["read/file"], fresh);
    assert!(
        app.notices.is_empty(),
        "the detected format's notice stayed"
    );
    assert_eq!(app.settings.drive, "B");
    w.get_by_label("Presets").click();
    w.run();
    assert!(restore(&w).accesskit_node().is_disabled());
    restore(&w).hover();
    w.run();
    w.get_by_label("No changes.");
}

#[test]
fn the_status_pane_says_what_has_not_happened_to_a_disk_yet() {
    let w = window(Settings::default());
    w.get_by_label("Disk status");
    w.get_by_label("No disk read yet");
    assert!(
        w.query_by_label("Greaseweazle Tools' output appears here.")
            .is_none(),
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
    assert_eq!(shown[1..3], ["Select disk format", "Select image type"]);
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

/// The sidebar's entry for a page.
fn entry<'w>(w: &'w Window, title: &'w str) -> Node<'w> {
    w.get_all_by_role_and_label(Role::Button, title)
        .find(|n| n.rect().left() < 60.0)
        .expect("the sidebar entry")
}

/// Each page that acts on the Greaseweazle, and its run button.
const DEVICE_PAGES: [(&str, &str); 13] = [
    ("Read disk", "Read disk"),
    ("Write disk", "Write disk"),
    ("Erase disk", "Erase disk"),
    ("Clean heads", "Clean"),
    ("Seek", "Seek"),
    ("Drive speed", "Measure"),
    ("Device info", "Get info"),
    ("Update firmware", "Update"),
    ("Delays", "Run"),
    ("Read pin", "Read pin"),
    ("Set pin", "Set pin"),
    ("Reset", "Reset"),
    ("USB bandwidth", "Measure"),
];

#[test]
fn every_page_opens_without_a_device_but_cannot_run() {
    let mut w = window(Settings::default());
    for (page, run) in DEVICE_PAGES {
        entry(&w, page).click();
        w.run();
        let disabled = run_button(&w, run).accesskit_node().is_disabled();
        assert!(disabled, "{page} runs with no device");
    }
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
            let app = app.get_or_insert_with(|| App::offline(ui.ctx(), Settings::default(), error));
            common::show(ui, app);
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
    let notice = w.get_by_label(FOUND).rect();
    assert!(
        notice.height() > 20.0,
        "it was cut short, not wrapped: {notice:?}"
    );
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
    let inputs: Vec<_> = w
        .get_all_by_role(Role::TextInput)
        .map(|t| t.rect().center())
        .collect();
    // A schema argument, then the rows with hover text of their own.
    let rows = [
        (
            "Revolutions",
            combo(&w, 3).rect().center(),
            "Revolutions to read per track.",
        ),
        (
            "Image type",
            combo(&w, 2).rect().center(),
            "The type of image to create. Disk format picks one.",
        ),
        ("Folder", inputs[0], "Where the image is saved."),
        (
            "Name",
            inputs[1],
            "The image's file name, extensions are handled by Image type.",
        ),
    ];
    for (label, field, tip) in rows {
        for at in [w.get_by_label(label).rect().center(), field] {
            // One tooltip at a time: the last must close first.
            w.event(egui::Event::PointerGone);
            w.run();
            w.hover_at(at);
            w.run();
            w.get_by_label(tip);
        }
    }
}

#[test]
fn a_button_in_a_field_shows_its_own_tooltip_alone() {
    let mut w = window(chosen());
    app_mut(&mut w).pin_ports(vec![greaseweazle()]);
    w.run();
    w.get_by_label("Detect").hover();
    w.run();
    w.get_by_label("Attempt to find the disk format and the image type that suits it.");
    assert!(
        w.query_by_label_contains("The disk's format").is_none(),
        "two tooltips at once"
    );
}

#[test]
fn the_smallest_window_keeps_the_page_clear_of_the_status_pane() {
    let w = window_at(ferriteweazle::SMALLEST, chosen());
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
    let log = w
        .get_by_label("Greaseweazle Tools' output appears here.")
        .rect();
    let copy = w.get_by_label("Copy").rect();
    assert!(
        copy.left() > status.left(),
        "it runs under the status pane: {copy:?}"
    );
    let run = run_button(&w, "Read disk").rect();
    assert!(
        log.top() > run.bottom(),
        "below the run button: {log:?}, {run:?}"
    );

    // It stays open from page to page.
    w.get_by_label("Write disk").click();
    w.run();
    w.get_by_label("Greaseweazle Tools' output appears here.");

    // The command line takes its place: only one is open, at the same height.
    let log_top = w.get_by_label("Copy").rect().top();
    w.get_by_label("CLI").click();
    w.run();
    w.get_by_label("Command line");
    assert!(
        w.query_by_label("Greaseweazle Tools' output appears here.")
            .is_none()
    );
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
    smooth_at(DEFAULT, settings)
}

/// `smooth` in a window of `size`.
fn smooth_at(size: egui::Vec2, settings: Settings) -> Window {
    let builder = Harness::builder()
        .with_size(size)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60);
    let w = start(
        builder,
        settings,
        Some(Job::replay("read", &damaged_read())),
    );
    // egui's own default: kittest turns animations off.
    w.ctx.all_styles_mut(|s| s.animation_time = 0.2);
    w
}

/// The tops of the page's run button and the map's legend: a sliding drawer is
/// painted only where it goes, so the page above it shows where it is.
fn edges(w: &Window) -> [f32; 2] {
    [
        run_button(w, "Read disk").rect().top(),
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
    // The page's scroll bar fades in, as the Log leaves the form too little room.
    assert!(w.run() < 10, "the window keeps drawing");

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
fn a_log_dragged_down_holds_at_its_least_height_before_it_shuts() {
    let mut w = smooth(Settings {
        drawer: Some(Drawer::Log),
        ..chosen()
    });
    w.run();
    let [open, _] = edges(&w);
    drag_log(&mut w, -25.0);
    assert_eq!(
        app(&w).settings.drawer,
        Some(Drawer::Log),
        "a small pull shut it"
    );
    assert_eq!(edges(&w)[0], open, "it went below its least height");
    drag_log(&mut w, -90.0);
    assert_eq!(app(&w).settings.drawer, None, "a long pull did not shut it");
}

#[test]
fn a_log_dragged_taller_stays_that_tall_and_the_map_shrinks_only_when_it_must() {
    // Taller than DEFAULT, whose map fills the room to the Log exactly.
    let settings = Settings {
        drawer: Some(Drawer::Log),
        ..chosen()
    };
    let mut w = smooth_at(DEFAULT + egui::vec2(0.0, 30.0), settings);
    w.run();
    let [open, legend] = edges(&w);
    // Less than the room the map leaves below it at this size.
    drag_log(&mut w, 15.0);
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
fn an_empty_log_dragged_taller_stays_that_tall() {
    let mut w = smooth(Settings {
        drawer: Some(Drawer::Log),
        ..chosen()
    });
    w.run();
    app_mut(&mut w).log.clear();
    w.run();
    let [open, _] = edges(&w);
    drag_log(&mut w, 100.0);
    for _ in 0..20 {
        w.step();
    }
    let [taller, _] = edges(&w);
    assert!(taller < open - 50.0, "{open} to {taller}");
}

#[test]
fn the_write_page_takes_a_folder_of_images_and_names_them_in_order() {
    let dir =
        std::env::temp_dir().join(format!("ferriteweazle-write-batch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in [
        "Game_Disk10.adf",
        "Game_Disk2.adf",
        "Game_Disk1.adf",
        "readme.txt",
    ] {
        std::fs::write(dir.join(name), "").unwrap();
    }
    let mut settings = Settings {
        page: Page::Command("write".into()),
        ..Settings::default()
    };
    let values = settings.values.entry("write".into()).or_default();
    values.set(ferriteweazle::form::BATCH, "on");
    values.set(ferriteweazle::form::BATCH_FOLDER, dir.to_string_lossy());
    let w = window(settings);
    w.get_by_label("Image");
    w.get_by_label("3 images: Game_Disk1.adf, Game_Disk2.adf, Game_Disk10.adf");
    w.get_by_role_and_label(Role::Button, "Write disks");
    let first = dir.join("Game_Disk1.adf");
    assert_eq!(
        app(&w).settings.values["write"].get("file"),
        first.to_string_lossy(),
        "the first image stands for the folder"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_82_cylinders_gw_erases_add_a_row_not_smaller_squares() {
    let size = |log: &str| {
        let settings = Settings {
            page: Page::Command("erase".into()),
            ..Settings::default()
        };
        let w = build(
            Harness::builder().with_size(DEFAULT),
            settings,
            Some(Job::replay("erase", log)),
        );
        let squares: Vec<_> = squares(&w).map(|s| s.rect).collect();
        let bottom = squares.iter().map(|r| r.bottom()).fold(f32::MIN, f32::max);
        (squares.len(), squares[0].width(), bottom)
    };
    // gw's header for `cyls` cylinders, then the first `done` erased.
    let log = |cyls: u32, done: u32| {
        let header = format!("Erasing c=0-{}:h=0-1, revs=1", cyls - 1);
        let tracks =
            (0..done).flat_map(|c| (0..2).map(move |h| format!("T{c}.{h}: Erasing Track")));
        std::iter::once(header)
            .chain(tracks)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (eighty, cell, _) = size(&log(80, 80));
    let (all, wider, bottom) = size(&log(82, 82));
    assert_eq!((eighty, all), (160, 164));
    assert_eq!(
        size(&log(82, 1)).0,
        164,
        "after the first cylinder, the map lacks the tracks to come"
    );
    assert_eq!(wider, cell, "the squares shrank for two more cylinders");
    assert!(
        bottom < DEFAULT.y,
        "the last row is off the window at {bottom}"
    );
}

#[test]
fn the_maps_squares_fade_in_and_out_with_the_pages_tracks() {
    let builder = Harness::builder()
        .with_size(DEFAULT)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120);
    let settings = Settings {
        page: Page::Settings,
        ..chosen()
    };
    let mut w = start(builder, settings, None);
    let fills = |w: &Window| squares(w).map(|s| s.fill).collect::<Vec<_>>();
    let tracks = |w: &mut Window, list: &str| {
        let values = app_mut(w).settings.values.get_mut("read").unwrap();
        values.set("tracks", list);
        // Half of FILL_TIME's 0.4 s.
        w.run_steps(12);
    };
    app_mut(&mut w).settings.page = Page::Command("read".into());
    w.run_steps(12);
    let appearing = fills(&w);
    w.run();
    assert_eq!(fills(&w).len(), 164, "gw's 82 cylinders on two sides");
    assert_ne!(appearing[0], fills(&w)[0], "the squares showed at once");

    tracks(&mut w, "c=0-39:h=0");
    assert_eq!(squares(&w).count(), 164, "the squares went at once");
    w.run();
    assert_eq!(squares(&w).count(), 40);

    tracks(&mut w, "c=0-83:h=0");
    let growing = fills(&w);
    w.run();
    assert_ne!(growing[83], fills(&w)[83], "cylinder 83 showed at once");

    tracks(&mut w, "c=0-9:h=0");
    assert_eq!(
        squares(&w).count(),
        84,
        "the grid shrank before its squares went"
    );
    w.run();
    assert_eq!(squares(&w).count(), 10);
}

/// The colour of the map's first row number `n`, if drawn.
fn row_number(w: &Window, n: &str) -> Option<egui::Color32> {
    let left = w.get_by_label("Disk status").rect().left();
    w.output().shapes.iter().find_map(|c| match &c.shape {
        egui::Shape::Text(t) if t.pos.x > left && t.galley.text() == n => {
            Some(t.galley.job.sections[0].format.color)
        }
        _ => None,
    })
}

#[test]
fn the_maps_row_numbers_fade_in_and_out_with_its_rows() {
    let builder = Harness::builder()
        .with_size(DEFAULT)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120);
    let mut w = start(builder, chosen(), None);
    w.run();
    let tracks = |w: &mut Window, list: &str| {
        let values = app_mut(w).settings.values.get_mut("read").unwrap();
        values.set("tracks", list);
        // Half of FILL_TIME's 0.4 s.
        w.run_steps(12);
    };
    let dim = row_number(&w, "0");
    tracks(&mut w, "c=0-39");
    let going = row_number(&w, "80");
    w.run();
    assert!(going.is_some() && going != dim, "80 went at once");
    assert_eq!(row_number(&w, "80"), None);

    tracks(&mut w, "c=0-83");
    let coming = row_number(&w, "80");
    w.run();
    assert!(coming.is_some() && coming != dim, "80 showed at once");
    assert_eq!(row_number(&w, "80"), dim);
}

#[test]
fn a_track_list_past_90_cylinders_keeps_the_squares_size_and_scrolls() {
    let map = |list: &str| {
        let mut settings = chosen();
        set(&mut settings, "read", "tracks", list);
        let w = window_at(DEFAULT, settings);
        let rects: Vec<_> = squares(&w).map(|s| s.rect).collect();
        let bottom = rects.iter().map(|r| r.bottom()).fold(f32::MIN, f32::max);
        (rects[0].width(), bottom)
    };
    let (usual, _) = map("");
    let (long, bottom) = map("c=0-254");
    assert_eq!(long, usual, "the squares shrank");
    assert!(bottom > DEFAULT.y, "the map fits, so nothing scrolls");
}

#[test]
fn a_finished_jobs_map_stands_until_its_page_takes_other_tracks() {
    let mut job = Job::replay("read", "Reading c=0-81:h=0-1 revs=2");
    job.planned = Some(((0..82).collect(), vec![0, 1]));
    let mut w = build(Harness::builder().with_size(DEFAULT), chosen(), Some(job));
    let preview = |w: &Window| w.query_by_label("No disk read yet").is_some();
    let tracks = |w: &mut Window, list: &str| {
        let values = app_mut(w).settings.values.get_mut("read").unwrap();
        values.set("tracks", list);
        w.run();
    };
    assert!(!preview(&w));
    tracks(&mut w, "c=0-39");
    assert!(preview(&w));
    tracks(&mut w, "");
    assert!(!preview(&w), "the same tracks again show the job's map");
}

#[test]
fn the_map_of_a_write_says_what_gw_reported_of_each_track() {
    let writing =
        |c, h| format!("T{c}.{h}: Writing Track (Flux: 200.0ms period, 200.2 ms total, Write all)");
    let header = "Writing c=0-1:h=0-1";
    // The image lacks cylinder 1, which gw passes over without a word.
    let verified = [
        header,
        &writing(0, 0),
        &writing(0, 1),
        "All tracks verified",
    ]
    .join("\n");
    // Track 1.0 fails its verify, which ends the write.
    let failed = [
        header,
        &writing(0, 0),
        &writing(0, 1),
        &writing(1, 0),
        "** FATAL ERROR:\nFailed to verify Track 1.0",
    ]
    .join("\n");
    // Flux with no format, which gw cannot verify.
    let reason = "No tracks verified (Reason: Verify unavailable)";
    let unverified = [header, &writing(0, 0), &writing(0, 1), reason].join("\n");
    let good = ("Good 2", "Every sector found, or written and verified.");
    let skipped = ("Skipped 2", "Outside the format, or not in the input.");
    let written = ("Written 2", "Written, no verify reported.");
    let bad = ("Bad 1", "No sectors found, or the write failed.");
    // Once the write has worked, what gw passed over is known.
    let passed = (
        "4 / 4 tracks",
        "Not in the input, so Greaseweazle Tools passed over it.",
    );
    let unreported = (
        "3 / 4 tracks",
        "Greaseweazle Tools has not reported this track.",
    );
    for (log, (count, hover), legend) in [
        (verified, passed, vec![good, skipped]),
        (failed, unreported, vec![written, bad]),
        (unverified, passed, vec![("Written 2", reason), skipped]),
    ] {
        let settings = Settings {
            page: Page::Command("write".into()),
            ..Settings::default()
        };
        let mut w = build(
            Harness::builder().with_size(DEFAULT),
            settings,
            Some(Job::replay("write", &log)),
        );
        w.get_by_label(count);
        // Side 0's squares, then side 1's: the last is cylinder 1, side 1.
        let never = squares(&w).last().expect("the map's squares").rect.center();
        w.hover_at(never);
        w.run();
        w.get_by_label("Cylinder 1, side 1");
        w.get_by_label(hover);
        for (entry, tip) in legend {
            // One tooltip at a time: the last must close first.
            w.event(egui::Event::PointerGone);
            w.run();
            w.get_by_label(entry).hover();
            w.run();
            w.get_by_label(tip);
        }
    }
}

#[test]
fn gws_warning_about_a_damaged_input_shows_in_the_status_pane() {
    // The Log, which also shows it, is shut.
    let w = build(
        Harness::builder().with_size(DEFAULT),
        chosen(),
        Some(Job::replay("convert", DAMAGED)),
    );
    let pane = w.get_by_label("Disk status").rect().left();
    let warning = w.get_by_label("SCP: WARNING: Bad image checksum").rect();
    assert!(warning.left() >= pane, "{warning:?}");
}

#[test]
fn a_square_names_the_rows_of_gws_sector_map_it_is_missing() {
    let mut w = build(
        Harness::builder().with_size(DEFAULT),
        chosen(),
        Some(Job::replay("read", &damaged_read())),
    );
    // Side 0's squares come first, one to a cylinder.
    let square = squares(&w).nth(20).expect("cylinder 20").rect.center();
    w.hover_at(square);
    w.run();
    w.get_by_label("Cylinder 20, side 0");
    w.get_by_label("Missing in Greaseweazle Tools' sector map (S): 5");
}

#[test]
fn a_track_outside_the_format_is_a_hole_in_the_map_not_one_to_come() {
    let log = "Reading c=0-80:h=0 revs=2\n\
               T0.0: IBM MFM (18/18 sectors) from Raw Flux (500 flux in 400.00ms)\n\
               T80.0: WARNING: Out of range for format 'ibm.1440': \
               No format conversion applied: Raw Flux (500 flux in 400.00ms)";
    let w = build(
        Harness::builder().with_size(DEFAULT),
        chosen(),
        Some(Job::replay("read", log)),
    );
    let squares: Vec<_> = squares(&w).collect();
    let (to_come, outside) = (squares[1], squares[80]);
    assert_ne!(outside.fill, to_come.fill);
    assert!(outside.stroke.width > 0.0, "no outline");
    w.get_by_label("Skipped 1");
}

#[test]
fn a_write_names_the_format_gw_takes_from_the_image() {
    let settings = Settings {
        page: Page::Command("write".into()),
        ..Settings::default()
    };
    let log = "Format amiga.amigados\nWriting c=0-79:h=0-1";
    let w = build(
        Harness::builder().with_size(DEFAULT),
        settings,
        Some(Job::replay("write", log)),
    );
    let pane = w.get_by_label("Disk status").rect().left();
    let about = w.get_by_label_contains("amiga.amigados").rect();
    assert!(about.left() >= pane, "not in the status pane: {about:?}");
}

#[test]
fn the_map_keeps_in_line_with_the_text_above_it_however_wide_the_pane() {
    let left = |width: f32| {
        let w = window_at(egui::vec2(width, 780.0), chosen());
        let heading = w.get_by_label("Disk status").rect().left();
        let map = squares(&w).map(|s| s.rect.left()).reduce(f32::min);
        map.expect("the map's squares") - heading
    };
    assert_eq!(left(1340.0), left(1240.0));
}

#[test]
fn with_too_little_room_the_status_pane_scrolls_and_its_rows_keep_their_width() {
    let mut job = Job::replay("read", &damaged_read());
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
fn a_stopped_read_says_so_and_what_it_left() {
    let mut job = Job::replay("read", &damaged_read());
    job.ended = Some((job.started, Outcome::Stopped));
    let w = build(Harness::builder().with_size(DEFAULT), chosen(), Some(job));
    w.get_by_label_contains("Stopped ·");
    w.get_by_label("Stopped: incomplete image.");
}

#[test]
fn settings_keeps_every_path_under_paths() {
    let w = window(Settings {
        page: Page::Settings,
        ..Settings::default()
    });
    let paths = w.get_by_label("Paths").rect().top();
    let jobs = w.get_by_label("Jobs").rect().top();
    for name in [
        "Images folder",
        "Presets folder",
        "Greaseweazle Tools (gw cli)",
    ] {
        let at = w.get_by_label(name).rect().top();
        assert!(paths < at && at < jobs, "{name} is outside Paths");
    }
    assert!(w.query_by_label("Presets").is_none());
}

/// What gw bandwidth printed on Windows 11 on ARM.
const BANDWIDTH: &str = "                   Min.   /   Mean   /   Max.
Write Bandwidth:    7.663 /    7.661 /    7.677 Mbps
Read Bandwidth:     8.004 /    8.153 /    8.349 Mbps

Estimated Consistent Min. Bandwidth: 6.897 Mbps
 -> Max. Flux Rate: 0.776 Msamples/sec
 -> Min. Ave. Flux: 1.289 us";

#[test]
fn a_result_shows_all_its_output_and_the_page_scrolls_under_a_tall_log() {
    let mut w = window(Settings {
        page: Page::Command("bandwidth".into()),
        drawer: Some(Drawer::Log),
        ..Settings::default()
    });
    app_mut(&mut w).tool = Some(Job::replay("bandwidth", BANDWIDTH));
    w.run();
    // The log shows the same lines: only those above it are the result's.
    let drawer = w.get_by_role_and_label(Role::Label, "Log").rect().top();
    for line in ["Write Bandwidth:", "-> Min. Ave. Flux: 1.289 us"] {
        let shown = w
            .query_all_by_label_contains(line)
            .any(|n| n.rect().bottom() < drawer);
        assert!(shown, "{line} is not shown");
    }
    let page_bar = |w: &Window| {
        let run = run_button(w, "Measure").rect();
        w.query_all_by_role(Role::ScrollBar)
            .map(|b| b.rect())
            .find(|r| r.height() > r.width() && r.bottom() <= run.top())
    };
    assert_eq!(page_bar(&w), None, "the page scrolls under a short log");
    drag_log(&mut w, 400.0);
    page_bar(&w).expect("the page does not scroll under a tall log");
}

#[test]
fn text_dragged_across_the_log_is_copied() {
    let mut w = window(Settings {
        drawer: Some(Drawer::Log),
        ..Settings::default()
    });
    // Short enough for the log at its least height to show all of it.
    let mut job = Job::replay("bandwidth", "Write Bandwidth: 7.66\nRead Bandwidth: 8.15");
    let log = &mut app_mut(&mut w).log;
    log.begin("gw bandwidth".into(), &mut job);
    log.follow(&mut job);
    w.run();
    let from = w
        .get_by_label_contains("Write Bandwidth:")
        .rect()
        .left_center();
    let to = w
        .get_by_label_contains("Read Bandwidth:")
        .rect()
        .right_center();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    w.event(egui::Event::PointerMoved(from));
    w.event(button(from, true));
    w.step();
    w.event(egui::Event::PointerMoved(to));
    w.step();
    w.event(button(to, false));
    w.step();
    w.event(egui::Event::Copy);
    w.step();
    let copied = w
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        });
    let copied = copied.expect("something was copied");
    assert!(
        copied.contains("Write Bandwidth:") && copied.contains("Read Bandwidth:"),
        "{copied:?}"
    );
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
    w.get_by_label("Greaseweazle Tools' output appears here.");
    let clear = w.get_by_role_and_label(Role::Button, "Clear");
    assert!(clear.accesskit_node().is_disabled(), "nothing to clear");
}

#[test]
fn a_line_gw_is_still_printing_shows_under_the_result_and_in_the_log() {
    let mut w = window(Settings {
        page: Page::Command("clean".into()),
        drawer: Some(Drawer::Log),
        ..Settings::default()
    });
    let app = app_mut(&mut w);
    let mut job = Job::replay("clean", "");
    job.ended = None;
    app.log.begin("gw clean".into(), &mut job);
    // gw clean prints each cylinder as the heads reach it, on one line per pass.
    job.partial = "Pass 0: 0 10 20".into();
    app.tool = Some(job);
    // Stepped, not run: a running job keeps the window repainting.
    w.run_steps(2);
    assert_eq!(w.query_all_by_label("Pass 0: 0 10 20").count(), 2);
}

#[test]
fn gws_question_and_its_answer_go_in_the_log_as_a_terminal_shows_them() {
    let mut w = window(Settings {
        page: Page::Command("seek".into()),
        ..Settings::default()
    });
    let ask = "@ferriteweazle ask \"Seek to extreme cylinder 90, Yes/No? \"";
    let mut job = Job::replay("seek", ask);
    job.ended = None;
    app_mut(&mut w).tool = Some(job);
    w.run_steps(2);
    w.get_by_role_and_label(Role::Button, "No").click();
    w.run_steps(2);
    let log = &app(&w).tool.as_ref().unwrap().log;
    assert_eq!(log, &["Seek to extreme cylinder 90, Yes/No? No"]);
}

#[test]
fn a_tool_that_printed_nothing_says_so() {
    let mut w = window(Settings {
        page: Page::Command("reset".into()),
        ..Settings::default()
    });
    app_mut(&mut w).tool = Some(Job::replay("reset", ""));
    w.run();
    w.get_by_label("Greaseweazle Tools printed no output.");
    assert!(
        w.query_by_label("Greaseweazle Tools' output appears here.")
            .is_none()
    );
}

#[test]
fn a_square_fades_in_as_its_track_is_read_then_the_window_rests() {
    let mut w = first_track_read(
        Harness::builder()
            .with_size(DEFAULT)
            .with_step_dt(1.0 / 60.0)
            .with_max_steps(120),
    );
    // Part way through, every square is full size, with no outline.
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
fn a_written_track_fades_to_green_as_it_verifies_then_the_window_rests() {
    let mut job = Job::replay("write", "Writing c=0-1:h=0");
    job.progress.verifies = true;
    job.progress.feed("T0.0: Writing Track (Flux: 1)");
    let builder = Harness::builder()
        .with_size(DEFAULT)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120);
    let mut w = build(builder, chosen(), Some(job));
    let first = |w: &Window| squares(w).next().unwrap().fill;
    let written = first(&w);
    // gw goes on to the next track, so the first verified.
    let disk = app_mut(&mut w).disk.as_mut().unwrap();
    disk.progress.feed("T1.0: Writing Track (Flux: 1)");
    // Half of FILL_TIME's 0.4 s.
    w.run_steps(12);
    let between = first(&w);
    w.run();
    let good = first(&w);
    assert_ne!(written, good);
    assert!(
        between != written && between != good,
        "it changed at once: {written:?} to {good:?}"
    );
    assert_eq!(w.run(), 1, "the window keeps drawing when nothing changes");
}

#[test]
fn detects_tracks_fade_in_as_it_reads_them() {
    let mut job = Job::replay(DETECT, "");
    job.ended = None;
    let mut settings = chosen();
    settings.page = Page::Command("read".into());
    let builder = Harness::builder()
        .with_size(DEFAULT)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120);
    let mut w = start(builder, settings, Some(job));
    // Detect's map holds only the tracks it has read, so each square first shows lit.
    let disk = app_mut(&mut w).disk.as_mut().unwrap();
    disk.progress.feed("T0.0: Raw Flux (500 flux in 400.00ms)");
    let first = |w: &Window| squares(w).next().unwrap().fill;
    w.run_steps(1);
    let start = first(&w);
    // Half of FILL_TIME's 0.4 s, then past it.
    w.run_steps(12);
    let between = first(&w);
    w.run_steps(24);
    let lit = first(&w);
    assert!(
        start != lit && between != start && between != lit,
        "it lit at once: {start:?}, {between:?}, {lit:?}"
    );
}

#[test]
fn detects_tracks_give_way_to_the_pages_map_once_another_format_is_chosen() {
    let mut job = Job::replay(DETECT, "T0.0: Raw Flux (500 flux in 400.00ms)");
    job.format = Some("ibm.1440".into());
    let mut settings = chosen();
    settings.page = Page::Command("read".into());
    set(&mut settings, "read", "format", "ibm.1440");
    let mut w = build(Harness::builder().with_size(DEFAULT), settings, Some(job));
    w.get_by_label("Detect disk format");
    assert_eq!(squares(&w).count(), 1, "the one track Detect read");

    let values = app_mut(&mut w).settings.values.get_mut("read").unwrap();
    values.set("format", "amiga.amigados");
    w.run();
    assert!(w.query_by_label("Detect disk format").is_none());
    w.get_by_label("No disk read yet");
    let blank = squares(&w).count();
    assert!(
        blank >= 160,
        "{blank} squares, not a whole disk with nothing read"
    );
}

#[test]
fn with_the_log_open_the_whole_map_still_fits_above_it() {
    let settings = Settings {
        drawer: Some(Drawer::Log),
        ..chosen()
    };
    let w = build(
        Harness::builder().with_size(DEFAULT),
        settings,
        Some(Job::replay("read", &damaged_read())),
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
}

#[test]
fn the_smallest_window_keeps_the_run_bar_settings_and_whole_map_in_view() {
    let small = ferriteweazle::SMALLEST;
    for drawer in [None, Some(Drawer::Log)] {
        let settings = Settings { drawer, ..chosen() };
        let mut w = start(
            Harness::builder().with_size(small),
            settings,
            Some(Job::replay("read", &damaged_read())),
        );
        w.run();
        let inside = |r: egui::Rect| r.bottom() <= small.y && r.right() <= small.x;
        let run = run_button(&w, "Read disk").rect();
        assert!(inside(run), "{drawer:?}: the run button at {run:?}");
        let settings = w.get_by_label("Settings").rect();
        assert!(inside(settings), "{drawer:?}: Settings at {settings:?}");
        let legend = w.get_by_label_contains("Good ").rect();
        let bottom = squares(&w).map(|s| s.rect.bottom()).fold(0.0, f32::max);
        assert!(
            bottom < legend.top(),
            "{drawer:?}: a square under the legend"
        );
        let limit = match drawer {
            Some(_) => w.get_by_role_and_label(Role::Label, "Log").rect().top(),
            None => small.y,
        };
        assert!(
            legend.bottom() < limit,
            "{drawer:?}: the legend at {legend:?}"
        );
        // The sides' buttons wrap with their label, not apart.
        let sides = w.get_by_label("Sides").rect();
        let side_1 = w.get_all_by_label("1").last().unwrap().rect();
        assert!(
            side_1.top() < sides.bottom() && sides.top() < side_1.bottom(),
            "{drawer:?}: {sides:?}, {side_1:?}"
        );
    }
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
    let mut job = Job::replay("read", &damaged_read());
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

/// The status pane's link to a job's image, in the file manager's own words.
const REVEAL: &str = if cfg!(target_os = "macos") {
    "Show in Finder"
} else if cfg!(windows) {
    "Show in Explorer"
} else {
    "Show in folder"
};

#[test]
fn an_image_a_job_left_can_be_shown_in_its_folder() {
    let shown = |command: &str, outcome: Option<Outcome>, no_image: bool| {
        let mut job = Job::replay(command, &damaged_read());
        job.output = Some("/d/Game.img".into());
        job.ended = outcome.map(|o| (job.started, o));
        job.no_image = no_image;
        let mut w = start(Harness::builder().with_size(DEFAULT), chosen(), Some(job));
        // Stepped, not run: a running job keeps the window repainting.
        w.run_steps(2);
        w.query_by_label(REVEAL).is_some()
    };
    assert!(shown("read", Some(Outcome::Succeeded), false));
    assert!(
        shown("read", Some(Outcome::Stopped), false),
        "gw keeps what it read"
    );
    assert!(
        !shown("convert", Some(Outcome::Stopped), false),
        "gw deletes it"
    );
    assert!(!shown("read", Some(Outcome::Failed), true), "gw deleted it");
    assert!(!shown("read", None, false), "it is still being read");
}

#[test]
fn showing_an_image_that_has_gone_says_so() {
    let gone = std::env::temp_dir().join("ferriteweazle-no-such-image.img");
    std::fs::remove_file(&gone).ok();
    let mut job = Job::replay("read", &damaged_read());
    job.output = Some(gone.clone());
    let mut w = build(Harness::builder().with_size(DEFAULT), chosen(), Some(job));
    w.get_by_label(REVEAL).hover();
    w.run();
    w.get_by_label("Show the image in its folder.");
    w.get_by_label(REVEAL).click();
    w.run();
    w.get_by_label(&format!("{} has been moved or deleted.", gone.display()));
}

#[test]
fn at_its_smallest_the_window_shows_the_whole_sidebar() {
    let w = window_at(DEFAULT, Settings::default());
    // With no device the card is a line short of its usual height.
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
fn device_info_gives_gws_steps_to_update_an_f1() {
    // An F1 updates only with its jumper fitted, and gw says where.
    let f1 = "Host Tools: 1.23\nDevice:\n  Port:     COM3\n  Model:    Greaseweazle F1\n  \
              Firmware: 1.0\n  Serial:   GW01\n  USB:      Full Speed (12 Mbit/s)\n\n\
              *** New firmware version 1.6 is available\n\
              To perform an Update:\n \
              - Disconnect from USB\n \
              - Install the Update Jumper at pins DCLK-GND\n \
              - Reconnect to USB\n \
              - Run \"gw update\" to download and install latest firmware";
    let settings = Settings {
        page: Page::Command("info".into()),
        ..Settings::default()
    };
    let mut w = window(settings);
    app_mut(&mut w).tool = Some(Job::replay("info", f1));
    w.run();
    w.get_by_label("Firmware 1.6 is available.");
    w.get_by_label("- Install the Update Jumper at pins DCLK-GND");
    w.get_by_label("- Run \"gw update\" to download and install latest firmware");
}

#[test]
fn device_info_is_done_when_its_device_answers_and_failed_when_gw_finds_none() {
    let settings = Settings {
        page: Page::Command("info".into()),
        ..Settings::default()
    };
    let mut w = window(settings);
    // gw info's report, then its check for newer firmware fails.
    let offline = "Host Tools: 1.23\nDevice:\n  Model:    Greaseweazle V4.1\n  Firmware: 1.6\n  \
                   USB:      Full Speed (12 Mbit/s), 128kB Buffer\n\
                   ** FATAL ERROR:\nGitHub API Rate Limit exceeded";
    app_mut(&mut w).tool = Some(Job::replay("info", offline));
    w.run();
    w.get_by_label("Done");
    w.get_by_label("GitHub API Rate Limit exceeded");
    w.get_by_label("Greaseweazle V4.1");
    let none = "Host Tools: 1.23\nDevice:\n  Not found";
    app_mut(&mut w).tool = Some(Job::replay("info", none));
    w.run();
    w.get_by_label("Failed");
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
fn next_to_each_input_greys_the_output_folder_and_a_batch_is_named_by_one_label() {
    let mut settings = Settings {
        page: Page::Command("convert".into()),
        ..Settings::default()
    };
    set(&mut settings, "convert", form::BATCH, "on");
    set(&mut settings, "convert", "format", "amiga.amigados");
    let out = Output {
        ext: ".adf".into(),
        beside_input: true,
        ..Output::default()
    };
    let folder = out.folder.clone();
    settings.outputs.insert("convert/out_file".into(), out);
    let mut w = window_at(DEFAULT, settings);
    let greyed = |w: &Window| {
        let field = w
            .get_all_by_role(Role::TextInput)
            .find(|n| n.value() == Some(folder.clone()));
        field
            .expect("the output folder shows")
            .accesskit_node()
            .is_disabled()
    };
    w.get_by_label("Output folder");
    assert!(greyed(&w), "the folder is greyed, not hidden");
    assert!(w.query_by_label("Prefix").is_none() && w.query_by_label("Suffix").is_none());
    w.get_by_label("Label");
    let after = w.get_by_label("After the name");
    assert!(after.accesskit_node().is_disabled(), "no label to place");

    let out = app_mut(&mut w)
        .settings
        .outputs
        .get_mut("convert/out_file")
        .unwrap();
    out.beside_input = false;
    out.batch_label = "Backup".into();
    w.run();
    assert!(!greyed(&w));
    assert!(
        !w.get_by_label("After the name")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn the_page_ends_as_far_from_the_status_pane_as_it_starts_from_the_sidebar() {
    let mut settings = chosen();
    settings.page = Page::Command("read".into());
    let w = window_at(DEFAULT, settings);
    let heading = w.get_all_by_label("Read disk").map(|n| n.rect());
    let heading = heading.min_by(|a, b| a.top().total_cmp(&b.top())).unwrap();
    let presets = w.get_by_label("Presets").rect();
    // The status pane's frame starts 18 points left of its heading.
    let divider = w.get_by_label("Disk status").rect().left() - 18.0;
    let left = heading.left() - 240.0;
    let right = divider - presets.right();
    assert!((left - right).abs() <= 1.0, "{left} left, {right} right");
    let folder = w.get_by_label("Folder").rect();
    let row = w
        .get_all_by_role(Role::Button)
        .filter(|b| (b.rect().center().y - folder.center().y).abs() < 4.0);
    let end = row.map(|b| b.rect().right()).fold(0.0, f32::max);
    assert_eq!(end, presets.right(), "the fields end where Presets does");
}

#[test]
fn a_resized_window_keeps_its_size_once_it_settles() {
    let dir = std::env::temp_dir().join(format!("ferriteweazle-window-{}", std::process::id()));
    let file = dir.join("window.txt");
    let mut w = window_at(DEFAULT, chosen());
    app_mut(&mut w).size_file = Some(file.clone());
    w.set_size(DEFAULT + egui::vec2(100.0, 50.0));
    w.run();
    assert!(!file.exists(), "kept while it may still be changing");
    std::thread::sleep(std::time::Duration::from_millis(600));
    w.run();
    let kept = std::fs::read_to_string(&file).unwrap_or_default();
    let want = format!("{} {}", DEFAULT.x + 100.0, DEFAULT.y + 50.0);
    assert_eq!(kept, want);
    w.set_size(DEFAULT);
    w.run();
    std::thread::sleep(std::time::Duration::from_millis(600));
    w.run();
    assert!(!file.exists(), "the default size keeps no file");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_bottom_right_corner_puts_back_the_default_size_once_the_window_is_resized() {
    let reset = |w: &Window| {
        w.get_by_label("Window size").click_secondary();
    };
    let mut w = window_at(DEFAULT, chosen());
    reset(&w);
    w.run();
    let item = w.get_by_role_and_label(Role::Button, "Reset window size");
    assert!(
        item.accesskit_node().is_disabled(),
        "the window is its default size"
    );

    let mut w = window_at(DEFAULT + egui::vec2(200.0, 100.0), chosen());
    reset(&w);
    w.run();
    w.get_by_role_and_label(Role::Button, "Reset window size")
        .click();
    w.step();
    let commands = &w.output().viewport_output[&egui::ViewportId::ROOT].commands;
    let size = egui::ViewportCommand::InnerSize(ferriteweazle::WINDOW);
    assert!(commands.contains(&size), "{commands:?}");
}

#[test]
fn the_device_list_picks_a_greaseweazle_or_an_adafruit_rp2040_then_its_port() {
    let mut w = window(Settings::default());
    let feather = Port {
        device: "COM9".into(),
        name: Some("Feather RP2040".into()),
        score: 0,
        denied: false,
    };
    app_mut(&mut w).pin_ports(vec![greaseweazle(), feather]);
    app_mut(&mut w).settings.drive = "B".into();
    w.run();
    combo(&w, 0).click();
    w.run();
    let ticked = |w: &Window, name: &str| {
        let item = w.get_by_role_and_label(Role::RadioButton, name);
        item.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
    };
    assert!(ticked(&w, "Greaseweazle"), "gw's own, by default");
    assert!(!ticked(&w, "Adafruit RP2040"));
    w.get_by_label("Serial Port");
    // gw names only its Greaseweazle.
    w.get_by_label("COM9");
    w.get_by_role_and_label(Role::RadioButton, "Adafruit RP2040")
        .click();
    w.run();
    assert_eq!(app(&w).settings.kind, Kind::Adafruit);
    assert_eq!(app(&w).settings.drive, "", "B gives way to A");
    // gw cannot pick one out, so the port must be chosen.
    assert_eq!(combo(&w, 0).value().as_deref(), Some("Select device"));
    combo(&w, 0).click();
    w.run();
    assert!(ticked(&w, "Adafruit RP2040"));
    w.get_by_label("COM9 · Feather RP2040").click();
    w.run();
    assert_eq!(app(&w).settings.device, "COM9");
    w.get_by_label("Adafruit RP2040");
    // The sidebar's identifiers, not the page's sides.
    let identifier = |id: &str| {
        let mut buttons = w.get_all_by_role(Role::Button);
        let node = buttons
            .find(|n| n.rect().left() < 240.0 && n.accesskit_node().label().as_deref() == Some(id));
        !node.expect("the identifier").accesskit_node().is_disabled()
    };
    for (id, possible) in [
        ("A", true),
        ("B", false),
        ("0", true),
        ("1", false),
        ("3", false),
    ] {
        assert_eq!(identifier(id), possible, "{id}");
    }
}

#[test]
fn gws_page_descriptions_name_the_adafruit_rp2040_when_it_is_the_device() {
    for (page, about) in [
        (
            "info",
            "Display information about the Adafruit RP2040 setup.",
        ),
        (
            "bandwidth",
            "Report the available USB bandwidth for the Adafruit RP2040 device.",
        ),
        (
            "reset",
            "Reset the Adafruit RP2040 device to power-on default state.",
        ),
    ] {
        let settings = Settings {
            page: Page::Command(page.into()),
            kind: Kind::Adafruit,
            ..Settings::default()
        };
        let mut w = window(settings);
        w.get_by_label(about);
        app_mut(&mut w).settings.kind = Kind::Greaseweazle;
        w.run();
        w.get_by_label(&about.replace("Adafruit RP2040", "Greaseweazle"));
    }
}

#[test]
fn an_adafruit_rp2040_greys_what_it_cannot_do_shown_off_and_kept_for_a_greaseweazle() {
    let mut settings = Settings {
        page: Page::Command("write".into()),
        kind: Kind::Adafruit,
        ..Settings::default()
    };
    set(&mut settings, "write", "pre_erase", "on");
    let mut w = window_at(DEFAULT, settings);
    // pre_erase goes to gw only for a Greaseweazle.
    w.get_by_label("Advanced options (10)").click();
    w.run();
    let schema = schema();
    let text = form::label(schema.command("write").unwrap().arg("pre_erase").unwrap());
    let toggle = |w: &Window| {
        let node = w.get_by_role_and_label(Role::CheckBox, &text);
        let on = node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True);
        (on, node.accesskit_node().is_disabled())
    };
    assert_eq!(toggle(&w), (false, true), "off, as gw gets it, and greyed");
    assert_eq!(app(&w).settings.values["write"].get("pre_erase"), "on");
    app_mut(&mut w).settings.kind = Kind::Greaseweazle;
    w.run();
    assert_eq!(toggle(&w), (true, false), "back on for a Greaseweazle");
    w.get_by_label("Advanced options (10, 1 set)");
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

/// The Settings page in `theme`, a frame every 60th of a second.
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

#[test]
fn a_log_dragged_to_its_tallest_never_pushes_the_page_over_the_run_bar() {
    let mut w = smooth(Settings {
        drawer: Some(Drawer::Log),
        ..chosen()
    });
    w.run();
    app_mut(&mut w).notices.insert("read".into(), FOUND.into());
    w.run();
    drag_log(&mut w, 600.0);
    w.run();
    let run = run_button(&w, "Read disk").rect();
    let status = w.get_by_label("Disk status").rect();
    let page = w
        .get_all_by_role(Role::ScrollBar)
        .map(|b| b.rect())
        .find(|r| r.top() < run.top() && r.right() < status.left())
        .expect("the page scrolls");
    assert!(page.bottom() <= run.top(), "{page:?} over {run:?}");
}

#[test]
fn a_result_wider_than_its_box_shows_a_scroll_bar_without_hovering() {
    let settings = Settings {
        page: Page::Command("update".into()),
        ..Settings::default()
    };
    let builder = Harness::builder()
        .with_size(egui::vec2(1240.0, 780.0))
        .wgpu();
    let mut w = build(builder, settings, None);
    let wide = "Downloading latest firmware: greaseweazle-firmware-1.6.upd, \
                and on well past the right edge of the box it is shown in";
    app_mut(&mut w).tool = Some(Job::replay("update", wide));
    w.run();
    let line = w.get_by_label_contains("Downloading latest").rect();
    let bar = w
        .get_all_by_role(Role::ScrollBar)
        .map(|b| b.rect())
        .find(|r| r.width() > r.height() && r.top() > line.bottom())
        .expect("a horizontal scroll bar");
    let image = w.render().expect("the window renders");
    let px = |p: egui::Pos2| *image.get_pixel(p.x as u32, p.y as u32);
    // The box's fill, between the line and the bar.
    let fill = px(egui::pos2(bar.left() + 4.0, bar.top() - 20.0));
    let marked = (bar.left() as u32..bar.right() as u32)
        .flat_map(|x| (bar.top() as u32..bar.bottom() as u32).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            let p = image.get_pixel(x, y);
            p.0.iter().zip(fill.0).any(|(a, b)| a.abs_diff(b) > 12)
        })
        .count();
    assert!(marked > 50, "the bar is not drawn: {marked} pixels");
}

const RULE: &str = "/opt/Ferriteweazle/greaseweazle/49-greaseweazle.rules";

/// The commands the fix shows, for the rule at RULE.
const COMMANDS: [&str; 2] = [
    "sudo cp /opt/Ferriteweazle/greaseweazle/49-greaseweazle.rules /etc/udev/rules.d/",
    "sudo udevadm control --reload-rules && sudo udevadm trigger",
];

/// The fix for a port Linux refused: named, with a button and the commands.
fn shows_the_fix(w: &Window) {
    w.get_by_label("No access to /dev/ttyACM0");
    let install = w.get_by_role_and_label(Role::Button, "Install udev rule");
    assert!(!install.accesskit_node().is_disabled());
    for command in COMMANDS {
        w.get_by_label(command);
    }
    w.get_by_role_and_label(Role::Link, "Greaseweazle Tools' Linux instructions");
}

#[test]
fn a_disk_job_refused_the_port_names_it_and_gives_gws_udev_rule() {
    let mut w = start(Harness::builder().with_size(DEFAULT), chosen(), None);
    let app = app_mut(&mut w);
    app.udev_rule = Some(RULE.into());
    app.disk = Some(Job::replay("read", REFUSED));
    w.run();
    shows_the_fix(&w);
    assert!(
        w.query_by_label_contains("[Errno 13]").is_none(),
        "not pyserial's words"
    );
}

#[test]
fn a_page_whose_job_was_refused_the_port_says_the_same_under_its_result() {
    let mut w = window(Settings {
        page: Page::Command("seek".into()),
        ..Settings::default()
    });
    let app = app_mut(&mut w);
    app.udev_rule = Some(RULE.into());
    app.tool = Some(Job::replay("seek", REFUSED));
    w.run();
    shows_the_fix(&w);
}

#[test]
fn with_no_rule_shipped_the_commands_name_gws_own_and_the_button_says_why_not() {
    let mut w = window(chosen());
    app_mut(&mut w).udev_rule = None;
    app_mut(&mut w).disk = Some(Job::replay("read", REFUSED));
    w.run();
    let install = w.get_by_role_and_label(Role::Button, "Install udev rule");
    assert!(install.accesskit_node().is_disabled());
    install.hover();
    w.run();
    w.get_by_label("No copy of the rule ships with this build.");
    w.get_by_label("sudo cp scripts/49-greaseweazle.rules /etc/udev/rules.d/");
}

#[test]
fn settings_names_the_default_folders_it_goes_back_to() {
    let mut w = window(Settings {
        page: Page::Settings,
        images_folder: Some("/elsewhere".into()),
        presets_folder: Some("/elsewhere".into()),
        ..Settings::default()
    });
    let defaults = [
        ferriteweazle::form::images_folder(),
        presets::default_folder(),
    ];
    for (n, folder) in defaults.iter().enumerate() {
        // One tooltip at a time: the last must close first.
        w.event(egui::Event::PointerGone);
        w.run();
        w.get_all_by_label("Use the default")
            .nth(n)
            .unwrap()
            .hover();
        w.run();
        w.get_by_label(&format!("Go back to {}.", folder.display()));
    }
}

#[test]
fn the_sound_setting_shows_on_every_system() {
    let w = window(Settings {
        page: Page::Settings,
        ..Settings::default()
    });
    w.get_by_role_and_label(Role::CheckBox, "Play a sound when a job ends");
}

#[test]
fn settings_links_gws_getting_started_guide() {
    let settings = Settings {
        page: Page::Settings,
        ..Settings::default()
    };
    // Tall enough to show About without scrolling.
    let mut w = window_at(egui::vec2(1240.0, 1400.0), settings);
    w.get_by_role_and_label(Role::Link, "Getting started with Greaseweazle")
        .hover();
    w.run();
    w.get_by_label("https://github.com/keirf/greaseweazle/wiki/Getting-Started");
}

#[test]
fn read_pin_says_it_reads_a_pin_and_set_pin_keeps_gws_words() {
    let page = |name: &str| {
        window(Settings {
            page: Page::Command(name.into()),
            ..Settings::default()
        })
    };
    let gws = "Change the setting of a user-modifiable interface pin.";
    let w = page("pin get");
    w.get_by_label("Read the level of a floppy interface pin.");
    assert!(w.query_by_label(gws).is_none(), "it changes nothing");
    page("pin set").get_by_label(gws);
}

#[test]
fn the_device_card_names_a_port_linux_denies_and_shows_how_to_grant_access() {
    let mut w = window(Settings::default());
    let app = app_mut(&mut w);
    app.udev_rule = Some(RULE.into());
    app.pin_ports(vec![Port {
        device: "/dev/ttyACM0".into(),
        denied: true,
        ..greaseweazle()
    }]);
    w.run();
    w.get_by_label("No access to ttyACM0.");
    w.get_by_label("Grant access…").click();
    w.run();
    shows_the_fix(&w);
}

#[test]
fn formats_within_a_family_run_in_numeric_order() {
    let mut w = window(Settings {
        page: Page::Command("write".into()),
        ..Settings::default()
    });
    combo(&w, 1).click();
    w.run();
    let top = |name| w.get_by_label(name).rect().top();
    assert!(top("ibm.360") < top("ibm.1200"));
    assert!(top("ibm.720") < top("ibm.1440"));
}

#[test]
fn the_maps_row_numbers_keep_a_line_apart_in_the_smallest_window() {
    let settings = Settings {
        drawer: Some(Drawer::Log),
        ..chosen()
    };
    let small = ferriteweazle::SMALLEST;
    let job = Job::replay("read", &damaged_read());
    let mut w = start(Harness::builder().with_size(small), settings, Some(job));
    w.run();
    let pane = w.get_by_label("Disk status").rect().left();
    let map = squares(&w)
        .map(|s| s.rect.left())
        .fold(f32::INFINITY, f32::min);
    let mut rows: Vec<f32> = w
        .output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t)
                if t.pos.x > pane && t.pos.x < map && t.galley.text().parse::<u32>().is_ok() =>
            {
                Some(t.visual_bounding_rect().center().y)
            }
            _ => None,
        })
        .collect();
    rows.sort_by(f32::total_cmp);
    assert!(rows.len() >= 4, "{rows:?}");
    // The numbers are 11 points: closer than that, they crowd.
    for pair in rows.windows(2) {
        assert!(pair[1] - pair[0] >= 11.0, "row numbers crowd: {rows:?}");
    }
}

#[test]
fn image_options_end_within_the_field_in_the_smallest_window() {
    let mut settings = chosen();
    settings.outputs.get_mut("read/file").unwrap().ext = ".hfe".into();
    let w = window_at(ferriteweazle::SMALLEST, settings);
    let right = combo(&w, 2).rect().right();
    let lists: Vec<_> = w.get_all_by_role(Role::ComboBox).skip(3).collect();
    assert!(lists.len() >= 4, "bitrate, version, interface and encoding");
    for list in lists {
        assert!(list.rect().right() <= right + 0.5, "{:?}", list.value());
    }
}

#[test]
fn a_typed_bt_turns_on_tracebacks_and_an_option_gw_lacks_is_refused() {
    let mut w = window(Settings {
        page: Page::Command("info".into()),
        ..Settings::default()
    });
    type_line(&mut w, "gw --bt info");
    assert!(app(&w).settings.backtrace);
    type_line(&mut w, "gw --foo info");
    w.get_by_label("gw has no option --foo.");
}

#[test]
fn a_set_carried_on_names_its_first_disk_and_keeps_it_through_the_command_line() {
    let mut settings = Settings {
        drawer: Some(Drawer::Cli),
        ..chosen()
    };
    let out = settings.outputs.get_mut("read/file").unwrap();
    (out.disks, out.first) = (7, 4);
    let mut w = window(settings);
    let shown = line(&w);
    assert!(shown.contains("Floppy_Disk4.adf"), "{shown}");
    w.get_by_label("Multiple disks (4 to 7)").click();
    w.run();
    w.get_by_label_contains("Floppy_Disk4.adf, Floppy_Disk5.adf … Floppy_Disk7.adf");

    type_line(&mut w, &format!("{shown} --revs=3"));
    let app = app(&w);
    assert_eq!(app.settings.values["read"].get("revs"), "3");
    let out = &app.settings.outputs["read/file"];
    assert_eq!((out.disks, out.first), (7, 4), "the set is kept");

    app_mut(&mut w)
        .settings
        .outputs
        .get_mut("read/file")
        .unwrap()
        .disks = 1;
    w.run();
    let label = w.get_by_label("First disk").rect();
    let field = w
        .get_all_by_role(Role::SpinButton)
        .find(|f| (f.rect().center().y - label.center().y).abs() < 4.0)
        .expect("the first disk's field");
    assert!(field.accesskit_node().is_disabled(), "one disk is no set");
}

#[test]
fn a_job_keeps_the_squares_the_window_opened_with() {
    let jobs = [
        None,
        Some(Job::replay(
            "write",
            "Writing c=0-79:h=0-1\nT0.0: Wrote 11 sectors",
        )),
        Some(Job::replay("read", &damaged_read())),
    ];
    // Short enough that the height, not the width, sizes the squares.
    let size = egui::vec2(DEFAULT.x, 768.0);
    let sizes = jobs.map(|job| {
        let settings = Settings {
            page: Page::Command("write".into()),
            ..chosen()
        };
        let mut w = start(Harness::builder().with_size(size), settings, job);
        w.run();
        squares(&w).next().unwrap().rect.width()
    });
    assert_eq!(
        sizes, [sizes[0]; 3],
        "no job, a write, a read with a warning"
    );
}

#[test]
fn a_very_wide_window_widens_the_page_once_the_map_is_as_large_as_it_gets() {
    let size = egui::vec2(3440.0, 1290.0);
    let settings = Settings {
        page: Page::Command("read".into()),
        ..chosen()
    };
    let mut w = start(Harness::builder().with_size(size), settings, None);
    w.run();
    let squares: Vec<_> = squares(&w).map(|s| s.rect).collect();
    assert_eq!(squares[0].width(), 96.0);
    let right = squares.iter().map(|r| r.right()).fold(0.0, f32::max);
    assert!(size.x - right < 200.0, "the map ends at {right}");
    let field = w.get_all_by_role(Role::TextInput).map(|n| n.rect().width());
    assert!(
        field.fold(0.0, f32::max) > 700.0,
        "the fields stayed narrow"
    );
}

#[test]
fn up_to_90_cylinders_keep_one_square_size_with_the_log_shut_or_open() {
    let size = |window: egui::Vec2, cyls: u32, drawer: Option<Drawer>| {
        let settings = Settings {
            page: Page::Command("erase".into()),
            drawer,
            ..Settings::default()
        };
        let header = format!("Erasing c=0-{}:h=0-1, revs=1", cyls - 1);
        let tracks =
            (0..cyls).flat_map(|c| (0..2).map(move |h| format!("T{c}.{h}: Erasing Track")));
        let log = std::iter::once(header).chain(tracks).collect::<Vec<_>>();
        let mut w = build(
            Harness::builder().with_size(window),
            settings,
            Some(Job::replay("erase", &log.join("\n"))),
        );
        w.run();
        squares(&w).next().unwrap().rect.width()
    };
    for (window, cell) in [(DEFAULT, 21.0), (egui::vec2(1920.0, 1080.0), 46.0)] {
        for cyls in [40, 80, 82, 90] {
            for drawer in [None, Some(Drawer::Log)] {
                assert_eq!(
                    size(window, cyls, drawer),
                    cell,
                    "{window:?} {cyls} {drawer:?}"
                );
            }
        }
    }
}

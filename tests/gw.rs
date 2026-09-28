//! End-to-end tests against real gw: the bundled engine in target/engine if
//! built, else an installed one. They skip when there is neither. None opens
//! a device.

use eframe::egui;
use egui_kittest::kittest::{NodeT, Queryable};
use ferriteweazle::engine::{Engine, Origin};
use ferriteweazle::form::Output;
use ferriteweazle::job::{DETECT, Job, Outcome};
use ferriteweazle::progress::Status;
use ferriteweazle::service::{Load, Service};
use ferriteweazle::{App, Page, Settings};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn engine() -> Option<Engine> {
    let python = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/engine/bin/python3");
    let found = if python.is_file() {
        Some(Engine {
            python,
            origin: Origin::Bundled,
        })
    } else {
        Engine::find(None)
    };
    if found.is_none() {
        eprintln!("skipped: no Greaseweazle engine on this machine");
    }
    found
}

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferriteweazle-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn wait<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(t) = ready() {
            return t;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn start(engine: &Engine, command: &str, args: &[&str]) -> Job {
    let args = args.iter().map(|a| a.to_string()).collect();
    Job::start(engine, command, args, Box::new(|| {})).expect("the bridge starts")
}

fn finish(mut job: Job, what: &str) -> Job {
    wait(what, || {
        job.poll();
        (!job.running()).then_some(())
    });
    job
}

/// Runs a conversion to the end; it must succeed.
fn run(engine: &Engine, args: &[&str]) -> Job {
    let job = finish(start(engine, "convert", args), "the job to end");
    assert_eq!(job.outcome(), Some(Outcome::Succeeded), "{:#?}", job.log);
    job
}

fn detect(engine: &Engine, image: &Path) -> Job {
    finish(start(engine, DETECT, &[&path(image)]), "detection")
}

#[test]
fn the_service_describes_gw_and_checks_values() {
    let Some(engine) = engine() else { return };
    let mut service = Service::start(&engine, Box::new(|| {}));
    let schema = wait("the schema", || {
        service.poll();
        if let Some(e) = service.schema.error() {
            panic!("no schema: {e}");
        }
        service.schema.ready().cloned()
    });
    for name in ["read", "write", "convert", "erase", "info", "pin set"] {
        assert!(
            schema.command(name).is_some(),
            "gw {} has no {name}",
            schema.version
        );
    }
    assert!(schema.formats.iter().any(|f| f == "ibm.1440"));
    let info = wait("format details", || {
        service.poll();
        match service.format_info("", "amiga.amigados") {
            Load::Ready(info) => Some(info.clone()),
            Load::Failed(e) => panic!("no format details: {e}"),
            Load::Waiting(_) => None,
        }
    });
    assert_eq!(
        (info.cyls, info.heads, info.sectors, info.bytes),
        (80, 2, Some(11), Some(901_120))
    );
    let complaint = wait("a check", || {
        service.poll();
        service.check("read", "revs", "0").map(str::to_owned)
    });
    assert_eq!(complaint, "must be 1 or greater");
}

#[test]
fn a_conversion_round_trip_is_exact_and_fully_mapped() {
    let Some(engine) = engine() else { return };
    let dir = scratch("round-trip");
    let (img, scp, back) = (dir.join("a.img"), dir.join("a.scp"), dir.join("b.img"));
    std::fs::write(
        &img,
        (0..368_640u32)
            .map(|i| (i * 7 % 251) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    run(
        &engine,
        &["convert", "--format=ibm.360", &path(&img), &path(&scp)],
    );
    let job = run(
        &engine,
        &["convert", "--format=ibm.360", &path(&scp), &path(&back)],
    );

    assert_eq!(std::fs::read(&img).unwrap(), std::fs::read(&back).unwrap());
    let p = &job.progress;
    assert_eq!((p.cyls.len(), p.heads.len()), (40, 2));
    assert!(p.tracks.values().all(|t| t.status == Status::Good));
    assert_eq!(p.total, Some((720, 720)));
    assert_eq!(p.sector_map.len(), 80);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn stopping_a_job_ends_it_and_gw_tidies_up() {
    let Some(engine) = engine() else { return };
    let dir = scratch("stop");
    let (img, scp, back) = (dir.join("a.img"), dir.join("a.scp"), dir.join("b.img"));
    std::fs::write(&img, vec![0u8; 1_474_560]).unwrap();
    run(
        &engine,
        &["convert", "--format=ibm.1440", &path(&img), &path(&scp)],
    );

    let mut job = start(
        &engine,
        "convert",
        &["convert", "--format=ibm.1440", &path(&scp), &path(&back)],
    );
    wait("the first track", || {
        job.poll();
        (!job.progress.tracks.is_empty()).then_some(())
    });
    job.stop();
    let asked = Instant::now();
    let job = finish(job, "the job to stop");
    assert_eq!(job.outcome(), Some(Outcome::Stopped));
    assert!(
        asked.elapsed() < Duration::from_secs(3),
        "stopped in {:?}",
        asked.elapsed()
    );
    assert!(job.progress.tracks.len() < 160);
    assert!(!back.exists(), "gw removes an unfinished conversion");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_question_from_gw_waits_for_an_answer() {
    let Some(engine) = engine() else { return };
    // Past cylinder 83 gw asks first; answering No ends it before any device is opened.
    let mut job = start(&engine, "seek", &["seek", "90"]);
    let question = wait("the question", || {
        job.poll();
        job.question.clone()
    });
    assert_eq!(question, "Seek to extreme cylinder 90, Yes/No? ");
    job.answer("No");
    let job = finish(job, "the job to end");
    assert_eq!(job.outcome(), Some(Outcome::Succeeded));
    assert!(job.log.is_empty(), "{:?}", job.log);
}

type Window = egui_kittest::Harness<'static, Option<App>>;

/// The app's window on `engine`, stepped until gw has described itself.
fn window(engine: &Engine, settings: Settings) -> Window {
    let settings = Settings {
        engine: Some(engine.python.clone()),
        ..settings
    };
    let mut w = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1240.0, 780.0))
        .build_ui_state(
            move |ui, app: &mut Option<App>| {
                app.get_or_insert_with(|| App::with_settings(ui.ctx(), settings.clone()))
                    .show(ui);
            },
            None,
        );
    until(&mut w, "the engine", |app| app.schema().is_some());
    // A click lands on what the last frame drew, as for a person.
    w.run_steps(2);
    w
}

fn until(w: &mut Window, what: &str, done: impl Fn(&App) -> bool) {
    wait(what, || {
        w.step();
        w.state().as_ref().is_some_and(&done).then_some(())
    });
}

/// Writes a blank ibm.360 image, dir/Game.img, and returns the Convert page
/// set to turn it into Game.scp beside it.
fn convert_page(dir: &Path) -> Settings {
    let img = dir.join("Game.img");
    std::fs::write(&img, vec![0u8; 368_640]).unwrap();
    let mut settings = Settings {
        page: Page::Command("convert".into()),
        ..Settings::default()
    };
    let values = settings.values.entry("convert".into()).or_default();
    values.set("in_file", img.to_string_lossy());
    values.set("format", "ibm.360");
    let out = Output {
        beside_input: true,
        ext: ".scp".into(),
        ..Output::default()
    };
    settings.outputs.insert("convert/out_file".into(), out);
    settings
}

#[test]
fn the_convert_page_makes_an_image_and_saves_its_log_beside_it() {
    let Some(engine) = engine() else { return };
    let dir = scratch("page");
    let settings = Settings {
        save_logs: true,
        ..convert_page(&dir)
    };
    let mut w = window(&engine, settings);
    w.get_by_label("Convert").click();
    until(&mut w, "the conversion", |app| {
        app.disk.as_ref().is_some_and(|j| !j.running())
    });

    let job = w.state().as_ref().unwrap().disk.as_ref().unwrap();
    assert_eq!(job.outcome(), Some(Outcome::Succeeded), "{:#?}", job.log);
    assert!(dir.join("Game.scp").is_file());
    let log =
        std::fs::read_to_string(dir.join("Game.scp.log")).expect("the log is beside the image");
    assert!(log.contains("Found 720 sectors of 720 (100%)"), "{log}");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_job_that_would_replace_a_file_asks_first() {
    let Some(engine) = engine() else { return };
    let dir = scratch("overwrite");
    std::fs::write(dir.join("Game.scp"), b"keep me").unwrap();
    let mut w = window(&engine, convert_page(&dir));
    w.get_by_label("Convert").click();
    w.run_steps(2);
    w.get_by_label("Overwrite \"Game.scp\"?");
    w.get_by_label("Cancel").click();
    w.run_steps(2);
    assert!(w.state().as_ref().unwrap().disk.is_none(), "nothing ran");
    assert_eq!(std::fs::read(dir.join("Game.scp")).unwrap(), b"keep me");

    w.get_by_label("Convert").click();
    w.run_steps(2);
    w.get_by_label("Overwrite").click();
    until(&mut w, "the conversion", |app| {
        app.disk.as_ref().is_some_and(|j| !j.running())
    });
    let job = w.state().as_ref().unwrap().disk.as_ref().unwrap();
    assert_eq!(job.outcome(), Some(Outcome::Succeeded), "{:#?}", job.log);
    assert!(std::fs::metadata(dir.join("Game.scp")).unwrap().len() > 1000);
    std::fs::remove_dir_all(dir).ok();
}

/// The run button, not the sidebar's entry of the same name.
fn read_button(w: &Window) -> egui_kittest::Node<'_> {
    w.get_all_by_role_and_label(egui::accesskit::Role::Button, "Read disk")
        .last()
        .expect("a Read disk button")
}

#[test]
fn reading_waits_for_a_format_and_image_type() {
    let Some(engine) = engine() else { return };
    let mut w = window(&engine, Settings::default());
    assert!(read_button(&w).accesskit_node().is_disabled());
    choose_format(&mut w, "amiga.amigados");
    assert!(
        !read_button(&w).accesskit_node().is_disabled(),
        "a format picks its image type, so the read can start"
    );
}

#[test]
fn closing_the_window_during_a_job_asks_then_stops_gw_before_closing() {
    let Some(engine) = engine() else { return };
    let mut w = window(&engine, Settings::default());
    // `seek 90` waits on gw's question, so the job runs until it is stopped.
    w.state_mut().as_mut().unwrap().tool = Some(start(&engine, "seek", &["seek", "90"]));
    until(&mut w, "gw's question", |app| {
        app.tool.as_ref().is_some_and(|j| j.question.is_some())
    });

    w.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    w.step();
    let sent = |w: &Window, command: egui::ViewportCommand| {
        w.output().viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&command)
    };
    assert!(sent(&w, egui::ViewportCommand::CancelClose));
    w.step();
    w.get_by_label("Stop and quit").click();
    until(&mut w, "the job to stop", |app| {
        app.tool.as_ref().is_some_and(|j| !j.running())
    });
    assert_eq!(
        w.state().as_ref().unwrap().tool.as_ref().unwrap().outcome(),
        Some(Outcome::Stopped)
    );
    wait("the window to close", || {
        w.step();
        sent(&w, egui::ViewportCommand::Close).then_some(())
    });
}

#[test]
fn a_job_that_ends_while_quit_asks_lets_the_window_close() {
    let Some(engine) = engine() else { return };
    let mut w = window(&engine, Settings::default());
    w.state_mut().as_mut().unwrap().tool = Some(start(&engine, "seek", &["seek", "90"]));
    until(&mut w, "gw's question", |app| {
        app.tool.as_ref().is_some_and(|j| j.question.is_some())
    });
    w.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    w.step();
    w.step();
    w.get_by_label("Stop and quit");

    let app = w.state_mut().as_mut().unwrap();
    app.tool.as_mut().unwrap().stop();
    wait("the window to close", || {
        w.step();
        w.output().viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Close)
            .then_some(())
    });
}

/// A flux image of `format`, which gw makes from a sector image `bytes` long.
fn flux_of(engine: &Engine, dir: &Path, format: &str, bytes: usize) -> PathBuf {
    let (img, scp) = (
        dir.join(format!("{format}.img")),
        dir.join(format!("{format}.scp")),
    );
    std::fs::write(
        &img,
        (0..bytes).map(|i| (i * 13 % 251) as u8).collect::<Vec<_>>(),
    )
    .unwrap();
    run(
        engine,
        &[
            "convert",
            &format!("--format={format}"),
            &path(&img),
            &path(&scp),
        ],
    );
    scp
}

#[test]
fn detection_names_the_format_of_a_flux_image() {
    let Some(engine) = engine() else { return };
    let dir = scratch("detect");
    let job = detect(&engine, &flux_of(&engine, &dir, "akai.800", 819_200));
    assert_eq!(job.outcome(), Some(Outcome::Succeeded), "{:#?}", job.log);
    assert_eq!(
        job.detected.first().map(String::as_str),
        Some("akai.800"),
        "{:?}",
        job.detected
    );
    assert!(!job.detected.iter().any(|f| f.ends_with(".scan")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_blank_image_is_no_format_and_says_so() {
    let Some(engine) = engine() else { return };
    let dir = scratch("detect-blank");
    // Nothing written, so every track is empty.
    let job = detect(&engine, &flux_of(&engine, &dir, "raw.250", 0));
    assert_eq!(job.outcome(), Some(Outcome::Failed));
    assert!(job.detected.is_empty());
    assert!(
        job.progress
            .error
            .as_deref()
            .is_some_and(|e| e.contains("No format")),
        "{:?}",
        job.progress.error
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_detect_button_chooses_the_format_of_the_input() {
    let Some(engine) = engine() else { return };
    let dir = scratch("detect-button");
    let scp = flux_of(&engine, &dir, "amiga.amigados", 901_120);
    let mut settings = Settings {
        page: Page::Command("convert".into()),
        ..Settings::default()
    };
    settings
        .values
        .entry("convert".into())
        .or_default()
        .set("in_file", scp.to_string_lossy());
    let mut w = window(&engine, settings);
    w.get_by_label("Detect").click();
    until(&mut w, "detection", |app| {
        app.disk.as_ref().is_some_and(|j| !j.running())
    });
    w.run_steps(2);
    let app = w.state().as_ref().unwrap();
    assert_eq!(
        app.settings.values["convert"].get("format"),
        "amiga.amigados"
    );
    assert_eq!(app.settings.outputs["convert/out_file"].ext, ".adf");
    w.get_by_label_contains("Found amiga.amigados.");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn detection_tells_apart_formats_that_differ_only_in_layout() {
    let Some(engine) = engine() else { return };
    let dir = scratch("detect-layout");
    // Each decodes like another format on cylinder 0; only the index mark,
    // skew, gaps or length differ.
    for (format, bytes) in [
        ("atarist.720", 737_280), // no index mark, where ibm.720 has one
        ("ibm.720", 737_280),
        ("akai.800", 819_200), // skews each cylinder, where eagle.dsqd.800 does not
        ("thomson.2s320", 655_360), // interleaves 7:1, where luxor does not
        ("acorn.adfs.320", 327_680), // 80 cylinders, where acorn.adfs.160 has 40
    ] {
        let job = detect(&engine, &flux_of(&engine, &dir, format, bytes));
        assert_eq!(
            job.detected.first().map(String::as_str),
            Some(format),
            "{:#?}",
            job.log
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_forty_track_disk_in_an_eighty_track_drive_needs_double_step() {
    let Some(engine) = engine() else { return };
    let dir = scratch("detect-step");
    let (img, scp) = (dir.join("d.img"), dir.join("d.scp"));
    std::fs::write(
        &img,
        (0..368_640u32).map(|i| (i % 253) as u8).collect::<Vec<_>>(),
    )
    .unwrap();
    // Cylinder c of the disk lands on cylinder 2c, as an 80-track drive sees it.
    run(
        &engine,
        &[
            "convert",
            "--format=ibm.360",
            "--out-tracks=step=2",
            &path(&img),
            &path(&scp),
        ],
    );
    let job = detect(&engine, &scp);
    assert_eq!(
        job.detected.first().map(String::as_str),
        Some("ibm.360"),
        "{:#?}",
        job.log
    );
    assert_eq!(job.step, 2);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn apple_ii_disks_are_told_apart_by_their_filesystem() {
    let Some(engine) = engine() else { return };
    let dir = scratch("detect-apple");
    let mut prodos = vec![0u8; 143_360];
    prodos[1024 + 2] = 3; // block 2: no previous block, the next is 3,
    prodos[1024 + 4] = 0xF0 | 4; // and a volume directory header, named in 4 letters
    let mut dos = vec![0u8; 143_360];
    let vtoc = 17 * 16 * 256; // track 17, sector 0
    for (at, byte) in [
        (1, 17),
        (2, 15),
        (3, 3),
        (0x27, 122),
        (0x34, 35),
        (0x35, 16),
    ] {
        dos[vtoc + at] = byte;
    }
    let neither = (0..143_360u32).map(|i| (i % 249) as u8).collect::<Vec<_>>();
    for (name, format, image) in [
        ("p.po", "apple2.prodos.140", prodos),
        ("d.do", "apple2.appledos.140", dos),
        ("n.do", "apple2.nofs.140", neither),
    ] {
        let (img, scp) = (dir.join(name), dir.join(format!("{name}.scp")));
        std::fs::write(&img, image).unwrap();
        run(
            &engine,
            &[
                "convert",
                &format!("--format={format}"),
                &path(&img),
                &path(&scp),
            ],
        );
        let job = detect(&engine, &scp);
        assert_eq!(
            job.detected.first().map(String::as_str),
            Some(format),
            "{:#?}",
            job.log
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

/// A disk definitions file for mine.800, an Akai-like 800K format with no skew.
fn custom_defs(dir: &Path) -> PathBuf {
    let defs = dir.join("mine.cfg");
    std::fs::write(
        &defs,
        "disk mine.800\n    cyls = 80\n    heads = 2\n    tracks * ibm.mfm\n        secs = 5\n        \
         bps = 1024\n        gap3 = 116\n        rate = 250\n    end\nend\n",
    )
    .unwrap();
    defs
}

#[test]
fn gw_checks_a_disk_definitions_file_line_by_line() {
    let Some(engine) = engine() else { return };
    let dir = scratch("diskdefs-check");
    let good = custom_defs(&dir);
    let bad = dir.join("bad.cfg");
    std::fs::write(
        &bad,
        "disk mine.bad\n    cyls = 80\n    heads = 2\n    tracks * ibm.mfm\n        secs = 5\n        \
         bogus = 3\n    end\nend\ndisk mine.worse\n    cyls = eighty\nend\n",
    )
    .unwrap();
    let mut service = Service::start(&engine, Box::new(|| {}));
    let mut read = |file: &Path| {
        let file = path(file);
        wait("the file's check", || {
            service.poll();
            match service.diskdefs(&file) {
                Load::Ready(d) => Some(Ok(d.clone())),
                Load::Failed(e) => Some(Err(e.clone())),
                Load::Waiting(_) => None,
            }
        })
    };
    let good = read(&good).unwrap();
    assert_eq!(
        (good.formats, good.errors),
        (vec!["mine.800".to_owned()], vec![])
    );
    let bad = read(&bad).unwrap();
    assert_eq!(bad.formats, ["mine.bad", "mine.worse"]);
    assert!(
        bad.errors[0].ends_with("line 6: unrecognised track option bogus"),
        "{bad:?}"
    );
    assert!(bad.errors[1].contains("line 10"), "{bad:?}");
    assert_eq!(
        read(&dir.join("missing.cfg")),
        Err("There is no such file.".into())
    );
    std::fs::remove_dir_all(dir).ok();
}

/// Steps the window until `shown`; on a timeout, lists what it showed instead.
fn until_shown(w: &mut Window, what: &str, shown: impl Fn(&Window) -> bool) {
    let start = Instant::now();
    loop {
        w.step();
        if shown(w) {
            return;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let seen: Vec<String> = w
                .root()
                .children_recursive()
                .filter_map(|n| n.accesskit_node().label())
                .filter(|l| !l.is_empty())
                .collect();
            panic!("timed out waiting for {what}; the window showed {seen:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn cli_line(w: &Window) -> String {
    w.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .value()
        .unwrap_or_default()
}

/// Chooses a format from the page's list by searching for it.
fn choose_format(w: &mut Window, format: &str) {
    w.get_all_by_role(egui::accesskit::Role::ComboBox)
        .nth(1)
        .expect("a format picker")
        .click();
    w.run_steps(2);
    w.event(egui::Event::Text(format.into()));
    w.run_steps(2);
    w.get_by_label(format).click();
    w.run_steps(3);
}

/// The track picker's button for a side, right of the drive identifier of the same name.
fn side<'a>(w: &'a Window, name: &'a str) -> egui_kittest::Node<'a> {
    w.get_all_by_role_and_label(egui::accesskit::Role::Button, name)
        .max_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .expect("a side button")
}

/// Whether each side's button is enabled, and lit.
fn sides(w: &Window) -> [(bool, bool); 2] {
    ["0", "1"].map(|name| {
        let node = side(w, name);
        let node = node.accesskit_node();
        let lit = node.toggled() == Some(egui::accesskit::Toggled::True);
        (!node.is_disabled(), lit)
    })
}

#[test]
fn a_one_sided_format_greys_the_sides_unless_the_list_names_side_1() {
    let Some(engine) = engine() else { return };
    let mut w = window(&engine, Settings::default());
    choose_format(&mut w, "ibm.1440");
    until_shown(&mut w, "two sides", |w| {
        w.query_by_label_contains("2\u{a0}sides").is_some()
    });
    assert_eq!(sides(&w), [(true, true), (true, true)]);
    side(&w, "0").click();
    w.run_steps(2);
    assert_eq!(sides(&w), [(true, false), (true, true)]);

    // The list keeps side 1 alone, so a click still changes the sides.
    choose_format(&mut w, "ibm.160");
    until_shown(&mut w, "one side", |w| {
        w.query_by_label_contains("1\u{a0}side").is_some()
    });
    assert_eq!(sides(&w), [(true, false), (true, true)]);
    side(&w, "0").click();
    w.run_steps(2);
    assert_eq!(sides(&w), [(false, true), (false, false)]);

    side(&w, "1").hover();
    until_shown(&mut w, "why side 1 is greyed", |w| {
        w.query_by_label("The format has one side.").is_some()
    });
    assert!(w.query_by_label("Which tracks to read.").is_none());
}

#[test]
fn a_disk_definitions_file_puts_its_formats_first_and_goes_to_gw_only_with_them() {
    let Some(engine) = engine() else { return };
    let dir = scratch("diskdefs-page");
    let defs = custom_defs(&dir);
    let mut settings = Settings::default();
    let values = settings.values.entry("read".into()).or_default();
    values.set("diskdefs", defs.to_string_lossy());
    let mut w = window(&engine, settings);
    w.get_by_label("CLI").click();
    w.run_steps(2);

    w.get_all_by_role(egui::accesskit::Role::ComboBox)
        .nth(1)
        .expect("a format picker")
        .click();
    until_shown(&mut w, "the file's formats", |w| {
        w.query_by_label("Custom disk definitions").is_some()
    });
    w.get_by_label("mine.800").click();
    w.run_steps(3);
    let line = cli_line(&w);
    assert!(
        line.contains(&format!("--diskdefs={}", defs.display())),
        "{line}"
    );
    assert!(line.contains("--format=mine.800"), "{line}");
    // Non-breaking spaces keep each fact in the format's description whole.
    w.get_by_label_contains("5\u{a0}sectors");

    choose_format(&mut w, "ibm.1440");
    let line = cli_line(&w);
    assert!(line.contains("--format=ibm.1440"), "{line}");
    assert!(
        !line.contains("--diskdefs"),
        "gw's own format needs no file: {line}"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_broken_disk_definitions_file_stops_the_page_and_says_why() {
    let Some(engine) = engine() else { return };
    let dir = scratch("diskdefs-broken");
    let bad = dir.join("bad.cfg");
    std::fs::write(&bad, "disk mine.worse\n    cyls = eighty\nend\n").unwrap();
    let mut settings = Settings::default();
    let values = settings.values.entry("read".into()).or_default();
    values.set("diskdefs", bad.to_string_lossy());
    let mut w = window(&engine, settings);
    choose_format(&mut w, "amiga.amigados");
    w.get_by_label_contains("Advanced options").click();
    until_shown(&mut w, "gw's objection", |w| {
        w.query_by_label_contains("bad.cfg, line 2").is_some()
    });
    assert!(read_button(&w).accesskit_node().is_disabled());
    read_button(&w).hover();
    w.run_steps(3);
    w.get_by_label_contains("has errors");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn detection_finds_a_format_from_a_disk_definitions_file() {
    let Some(engine) = engine() else { return };
    let dir = scratch("diskdefs-detect");
    let defs = format!("--diskdefs={}", custom_defs(&dir).display());
    let (img, scp) = (dir.join("mine.img"), dir.join("mine.scp"));
    std::fs::write(
        &img,
        (0..819_200)
            .map(|i| (i * 7 % 251) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    run(
        &engine,
        &[
            "convert",
            &defs,
            "--format=mine.800",
            &path(&img),
            &path(&scp),
        ],
    );
    let job = finish(start(&engine, DETECT, &[&defs, &path(&scp)]), "detection");
    assert!(
        job.detected.iter().any(|f| f == "mine.800"),
        "{:#?}",
        job.log
    );
    std::fs::remove_dir_all(dir).ok();
}

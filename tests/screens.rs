//! Renders the window in light and dark to target/screens, for looking at.
//!
//!     cargo test --test screens -- --ignored
//!
//! Uses the gw it finds, as the app does, with a made-up Greaseweazle on
//! /dev/cu.usbmodem14201 whatever is plugged in, and replays saved gw output.

mod common;

use common::{
    AKAI_TRACK, DAMAGED, DEFAULT, DETECTED, FOUND, REFUSED, SCRATCHED, TRACK_0, WORKBENCH, WRITTEN,
    Window, app_mut, damaged_read, greaseweazle, held, image_part, on_disk, run_button,
    scratched_adf,
};
use eframe::egui::{self, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ferriteweazle::form::{self, Output};
use ferriteweazle::job::{DETECT, Job, Outcome};
use ferriteweazle::schema::Port;
use ferriteweazle::theme::Choice;
use ferriteweazle::{Analysis, App, Drawer, Media, Page, Settings, Shows};
use std::time::Duration;

/// What gw info prints, as info.py formats it.
const INFO: &str = "Host Tools: 1.23
Device:
  Port:     /dev/cu.usbmodem14201
  Model:    Greaseweazle V4.1
  MCU:      AT32F403A, 216MHz, 224kB SRAM
  Firmware: 1.6
  Serial:   GW0123456789ABCDEF
  USB:      Full Speed (12 Mbit/s), 128kB Buffer

*** New firmware version 1.7 is available
To perform an Update:
 - Run \"gw update\" to download and install latest firmware";

/// `render_sized` at 1240 by 780 points.
fn render(
    name: &str,
    theme: egui::Theme,
    settings: Settings,
    job: Option<Job>,
    act: impl FnOnce(&mut Window),
) {
    render_sized(name, egui::vec2(1240.0, 780.0), theme, settings, job, act);
}

/// Renders to target/screens after `act`, such as opening a menu.
fn render_sized(
    name: &str,
    size: egui::Vec2,
    theme: egui::Theme,
    settings: Settings,
    mut job: Option<Job>,
    act: impl FnOnce(&mut Window),
) {
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(2.0)
        .with_theme(theme)
        .wgpu()
        .build_ui_state(
            move |ui, app: &mut Option<App>| {
                let app = app.get_or_insert_with(|| {
                    let mut app = App::with_settings(ui.ctx(), settings.clone());
                    app.pin_ports(vec![greaseweazle()]);
                    app
                });
                match job.take() {
                    Some(job) if job.command == "info" => app.tool = Some(job),
                    Some(job) => app.disk = Some(job),
                    None => {}
                }
                common::show(ui, app);
            },
            None,
        );
    for _ in 0..60 {
        harness.step();
        std::thread::sleep(Duration::from_millis(25));
    }
    act(&mut harness);
    harness.run_steps(20);
    let suffix = match theme {
        egui::Theme::Dark => "dark",
        egui::Theme::Light => "light",
    };
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/screens");
    std::fs::create_dir_all(dir).unwrap();
    let image = harness.render().expect("the window renders");
    image.save(format!("{dir}/{name}-{suffix}.png")).unwrap();
}

fn settings(page: &str, theme: egui::Theme) -> Settings {
    let mut s = Settings {
        page: Page::Command(page.into()),
        theme: theme.into(),
        ..Settings::default()
    };
    let read = s.values.entry("read".into()).or_default();
    read.set("format", "ibm.1440");
    read.set("revs", "2");
    let convert = s.values.entry("convert".into()).or_default();
    convert.set("in_file", INPUT);
    convert.set("format", "ibm.1440");
    s.outputs.insert(
        "convert/out_file".into(),
        Output {
            beside_input: true,
            ext: ".img".into(),
            ..Output::default()
        },
    );
    s.outputs.insert(
        "read/file".into(),
        Output {
            folder: "/Users/you/Documents/Ferriteweazle/Images".into(),
            ext: ".img".into(),
            ..Output::default()
        },
    );
    s
}

/// The damaged conversion told as a read for the read page, with one track
/// retried as gw does by default: three times, over three revolutions.
fn read_job() -> Job {
    let mut job = Job::replay("read", &read_log());
    job.format = Some("ibm.1440".into());
    job.output = Some("/Users/you/Documents/Ferriteweazle/Images/Floppy.img".into());
    job
}

/// read_job still running, its short track found whole in a second pass.
fn passes_job() -> Job {
    let log = read_log()
        + "\nPass 2 of 3: 1 track\n\
           T21.1: IBM MFM (16/18 sectors) from Raw Flux (188466 flux in 400.79ms)\n\
           T21.1: IBM MFM (18/18 sectors) from 2 passes";
    let mut job = Job::replay("read", &log);
    job.ended = None;
    job.format = Some("ibm.1440".into());
    job
}

fn read_log() -> String {
    damaged_read()
        .replace(
            "Format ibm.1440\nConverting c=0-79:h=0-1 -> c=0-79:h=0-1",
            "Reading c=0-79:h=0-1 revs=2\nFormat ibm.1440",
        )
        .replace(
            "T21.1: IBM MFM (17/18 sectors) from Raw Flux (188470 flux in 400.80ms)",
            "T21.1: IBM MFM (16/18 sectors) from Raw Flux (188470 flux in 400.80ms)\n\
             T21.1: IBM MFM (17/18 sectors) from Raw Flux (282706 flux in 601.20ms) (Retry #1.1)\n\
             T21.1: IBM MFM (17/18 sectors) from Raw Flux (282691 flux in 601.19ms) (Retry #1.2)\n\
             T21.1: IBM MFM (17/18 sectors) from Raw Flux (282712 flux in 601.21ms) (Retry #1.3)\n\
             T21.1: Giving up: 1 sectors missing",
        )
}

/// A flux image of 80 cylinders written over gw's default 82: gw passes
/// over the last two without a word, and cannot verify flux.
fn write_job() -> Job {
    let tracks = (0..80).flat_map(|c| {
        (0..2).map(move |h| {
            format!("T{c}.{h}: Writing Track (Flux: 200.0ms period, 200.2 ms total, Write all)")
        })
    });
    let log = std::iter::once("Writing c=0-81:h=0-1".to_owned())
        .chain(tracks)
        .chain(std::iter::once(
            "No tracks verified (Reason: Verify unavailable)".to_owned(),
        ))
        .collect::<Vec<_>>();
    Job::replay("write", &log.join("\n"))
}

/// A Greaseweazle on /dev/ttyACM0 that Linux denies this account, and gw's
/// udev rule where the tarball keeps it.
fn denied(w: &mut Window) {
    let app = w.state_mut().as_mut().unwrap();
    app.udev_rule = Some("/home/you/Ferriteweazle/greaseweazle/49-greaseweazle.rules".into());
    app.pin_ports(vec![Port {
        device: "/dev/ttyACM0".into(),
        denied: true,
        ..greaseweazle()
    }]);
}

/// gw info, a read and a drive speed in the session's log.
fn session(w: &mut Window) {
    let log = &mut w.state_mut().as_mut().unwrap().log;
    let rpm = "Rate: 300.121 rpm ; Period: 199.919 ms";
    for (heading, mut job, ending) in [
        ("gw info", Job::replay("info", INFO), "Done in 0:01."),
        (
            "gw read --format=ibm.1440 --revs=2 /Users/you/Documents/Ferriteweazle/Images/Floppy.img",
            read_job(),
            "Done in 0:52.",
        ),
        ("gw rpm", Job::replay("rpm", rpm), "Done in 0:01."),
    ] {
        log.begin(heading.into(), &mut job);
        log.end(&mut job, ending.into());
    }
}

#[test]
#[ignore = "writes pictures for people to look at"]
fn screens() {
    // Floppy.img exists, so Read disk stops at the overwrite prompt.
    let dir = std::env::temp_dir().join("ferriteweazle-screens");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Floppy.img"), b"").unwrap();
    // gw finds a North Star image's format from its size: this is one-sided FM.
    let nsi = dir.join("Disk.nsi");
    std::fs::write(&nsi, vec![0u8; 89_600]).unwrap();
    let bad = dir.join("Bad.nsi");
    std::fs::write(&bad, vec![0u8; 1000]).unwrap();
    // Tall enough for every section of Settings, or of a page opened out.
    let tall = egui::vec2(1240.0, 1180.0);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        for (name, file) in [("write-nsi", &nsi), ("write-nsi-bad", &bad)] {
            let mut write = settings("write", theme);
            let values = write.values.entry("write".into()).or_default();
            values.set("file", file.to_string_lossy());
            render(name, theme, write, None, |_| {});
        }
        render("read-more", theme, settings("read", theme), None, |w| {
            w.get_by_label_contains("Advanced options").click();
            w.run_steps(10);
            w.hover_at(egui::pos2(600.0, 400.0));
            w.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -560.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
        });
        let read = settings("read", theme);
        render_sized("default", DEFAULT, theme, read, Some(read_job()), |_| {});
        let small = ferriteweazle::SMALLEST;
        let read = settings("read", theme);
        render_sized("smallest", small, theme, read, Some(read_job()), |_| {});
        let read = settings("read", theme);
        render_sized("smallest-log", small, theme, read, Some(read_job()), |w| {
            session(w);
            w.get_by_role_and_label(Role::Button, "Log").click();
        });
        let read = settings("read", theme);
        render_sized("log", DEFAULT, theme, read, Some(read_job()), |w| {
            session(w);
            w.get_by_role_and_label(Role::Button, "Log").click();
        });
        let read = settings("read", theme);
        render_sized("log-tall", DEFAULT, theme, read, Some(read_job()), |w| {
            session(w);
            w.get_by_role_and_label(Role::Button, "Log").click();
            w.run_steps(4);
            // The drawer's top edge, at its first height, dragged up.
            let edge = egui::pos2(700.0, DEFAULT.y - 124.0);
            let up = edge - egui::vec2(0.0, 300.0);
            w.hover_at(edge);
            w.run_steps(2);
            w.drag_at(edge);
            w.run_steps(2);
            w.hover_at(up);
            w.run_steps(2);
            w.drop_at(up);
        });
        render("found", theme, settings("read", theme), None, |w| {
            let app = w.state_mut().as_mut().unwrap();
            app.notices.insert("read".into(), FOUND.into());
        });
        render("no-device", theme, settings("read", theme), None, |w| {
            w.state_mut().as_mut().unwrap().pin_ports(Vec::new());
            run_button(w, "Read disk").hover();
        });
        let mut disks = settings("read", theme);
        disks.outputs.get_mut("read/file").unwrap().disks = 3;
        render("disks", theme, disks, None, |w| {
            w.get_by_label_contains("Multiple disks").click();
        });
        let mut total = settings("read", theme);
        let out = total.outputs.get_mut("read/file").unwrap();
        (out.disks, out.label, out.total) = (12, "Disk".into(), true);
        render_sized("disks-total", tall, theme, total, None, |w| {
            w.get_by_label_contains("Multiple disks").click();
        });
        let mut flippy = settings("read", theme);
        let read = flippy.values.entry("read".into()).or_default();
        read.set("tracks", "c=0-39:step=2:h1.off=-8");
        render("flippy", theme, flippy, None, |_| {});
        render("save-preset", theme, settings("read", theme), None, |w| {
            w.get_by_label("Presets").click();
            w.run();
            w.get_all_by_label("Save…").last().unwrap().click();
            w.run();
            w.event(egui::Event::Text("Amiga DD".into()));
        });
        for (name, open) in [("disks-named", true), ("disk-name", false)] {
            let mut named = settings("read", theme);
            let out = named.outputs.get_mut("read/file").unwrap();
            (out.disks, out.ask_names) = (100, true);
            render(name, theme, named, None, |w| match open {
                true => w.get_by_label_contains("Multiple disks").click(),
                false => run_button(w, "Read disks").click(),
            });
        }
        let mut passes = settings("read", theme);
        let out = passes.outputs.get_mut("read/file").unwrap();
        (out.passes, out.keep_passes) = (3, true);
        render("passes", theme, passes, Some(passes_job()), |w| {
            w.get_by_label_contains("Read passes").click();
        });
        let mut replace = settings("read", theme);
        replace.outputs.get_mut("read/file").unwrap().folder = dir.to_string_lossy().into();
        render("overwrite", theme, replace, None, |w| {
            run_button(w, "Read disk").click();
        });
        render(
            "cli",
            theme,
            settings("read", theme),
            Some(read_job()),
            |w| {
                w.get_by_label("CLI").click();
            },
        );
        let mut cancelled = read_job();
        cancelled.ended = Some((cancelled.started, Outcome::Stopped));
        render(
            "cancelled",
            theme,
            settings("read", theme),
            Some(cancelled),
            |_| {},
        );
        render("formats", theme, settings("write", theme), None, |w| {
            // The sidebar's port picker comes first, then the page's format picker.
            w.get_all_by_role(Role::ComboBox)
                .nth(1)
                .expect("a format picker")
                .click();
        });
        let games = concat!(env!("CARGO_MANIFEST_DIR"), "/target/screens-games");
        std::fs::create_dir_all(games).unwrap();
        for d in 1..=12 {
            std::fs::write(format!("{games}/Game_Disk{d}.scp"), "").unwrap();
        }
        for page in ["write", "convert"] {
            let mut batch = settings(page, theme);
            let values = batch.values.entry(page.into()).or_default();
            values.set(form::BATCH, "on");
            values.set(form::BATCH_FOLDER, games);
            values.set("format", "amiga.amigados");
            if page == "convert" {
                let out = batch.outputs.entry("convert/out_file".into()).or_default();
                out.beside_input = false;
                out.ext = ".adf".into();
                out.folder = "/Users/you/Documents/Ferriteweazle/Images".into();
                out.batch_label = "Backup".into();
                out.label_first = true;
            }
            render(&format!("{page}-batch"), theme, batch, None, |_| {});
        }
        for (name, job) in [
            ("read", Some(read_job())),
            ("convert", Some(Job::replay("convert", DAMAGED))),
            ("write", None),
            ("info", Some(Job::replay("info", INFO))),
        ] {
            render(name, theme, settings(name, theme), job, |_| {});
        }
        let written = Some(write_job());
        render(
            "write-done",
            theme,
            settings("write", theme),
            written,
            |_| {},
        );
        render("delays", theme, settings("delays", theme), None, |_| {});
        let mut typed = settings("read", theme);
        let read = typed.values.entry("read".into()).or_default();
        read.set("tracks", "c=0-7,9-12:h=0-1");
        render("tracks-help", theme, typed, None, |w| {
            w.get_by_label("Track settings").hover();
        });
        for ext in [".hfe", ".scp"] {
            let mut options = settings("read", theme);
            options.outputs.get_mut("read/file").unwrap().ext = ext.into();
            let name = format!("options{}", ext.replace('.', "-"));
            render(&name, theme, options, None, |_| {});
        }
        let mut update = settings("update", theme);
        let firmware = "/Users/you/Downloads/greaseweazle-firmware-v1.7.upd";
        update
            .values
            .entry("update".into())
            .or_default()
            .set("file", firmware);
        render("update", theme, update, None, |_| {});
        let read = settings("read", theme);
        let refused = Some(Job::replay("read", REFUSED));
        render("refused", theme, read, refused, denied);
        render("refused-card", theme, settings("read", theme), None, |w| {
            denied(w);
            w.run_steps(4);
            w.get_by_label("Grant access…").click();
        });
        let page = Settings {
            page: Page::Settings,
            ..settings("read", theme)
        };
        render_sized("settings", tall, theme, page, None, |_| {});
        // A newer release on offer: the banner on a page, and Settings' Update.
        let read = settings("read", theme);
        render("update-banner", theme, read, None, |w| {
            w.state_mut().as_mut().unwrap().offer_update("v1.3.4");
        });
        let page = Settings {
            page: Page::Settings,
            ..settings("read", theme)
        };
        render_sized("settings-update", tall, theme, page, None, |w| {
            w.state_mut().as_mut().unwrap().offer_update("v1.3.4");
        });
        about(theme);
    }
}

/// A real Akai S950 disk's HFE image as flux, a scratch cut into side 1,
/// cylinders 10 to 70, converted to sectors.
const AKAI: &str = include_str!("data/convert-akai.log");

/// The Convert page's input in these pictures.
const INPUT: &str = "/Users/you/Floppies/Disk07.scp";

/// A conversion's recording replayed as though run from the Convert page
/// these pictures show: its input, as gw named it, the page's.
fn converted(log: &str) -> Job {
    let mut job = Job::replay("convert", log);
    if let Some(source) = job.progress.source.as_mut() {
        source.file = Some(INPUT.into());
    }
    job
}

/// The Akai conversion, its track 0.0 with its sectors' data.
fn akai_job() -> Job {
    let mut job = converted(AKAI);
    let line = AKAI_TRACK.trim().strip_prefix("@ferriteweazle track ");
    job.progress.report(line.expect("a track report"));
    job
}

fn workbench() -> Job {
    let mut job = Job::replay("read", WORKBENCH);
    job.output = Some("/Users/you/Documents/Ferriteweazle/Images/Workbench.adf".into());
    job
}

/// The Analyse drawer over real disks, at the window's first size, its
/// smallest, and the size the other pictures have.
#[test]
#[ignore = "writes pictures for people to look at"]
fn analyse() {
    let open = |page: &str, theme, media| Settings {
        drawer: Some(Drawer::Analyse),
        media,
        ..settings(page, theme)
    };
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let read = open("read", theme, Media::Fit);
        render_sized(
            "analyse-read",
            DEFAULT,
            theme,
            read,
            Some(workbench()),
            |_| {},
        );
        let read = open("read", theme, Media::Fit);
        render("analyse-read-wide", theme, read, Some(workbench()), |_| {});
        let read = open("read", theme, Media::Fit);
        let small = ferriteweazle::SMALLEST;
        render_sized(
            "analyse-smallest",
            small,
            theme,
            read,
            Some(workbench()),
            |_| {},
        );
        let read = open("read", theme, Media::ThreeHalf);
        render_sized(
            "analyse-read-3.5",
            DEFAULT,
            theme,
            read,
            Some(workbench()),
            |_| {},
        );
        let convert = open("convert", theme, Media::Fit);
        render_sized(
            "analyse-akai",
            DEFAULT,
            theme,
            convert,
            Some(akai_job()),
            |_| {},
        );
        for (name, media, shows) in [
            ("analyse-scratched", Media::Fit, Shows::Sectors),
            ("analyse-scratched-flux", Media::Fit, Shows::Flux),
            ("analyse-scratched-3.5", Media::ThreeHalf, Shows::Sectors),
        ] {
            let scratched = converted(SCRATCHED);
            let convert = Settings {
                shows,
                ..open("convert", theme, media)
            };
            render(name, theme, convert, Some(scratched), |_| {});
        }
        let akai = Settings {
            shows: Shows::Flux,
            ..open("convert", theme, Media::Fit)
        };
        render_sized(
            "analyse-akai-flux",
            DEFAULT,
            theme,
            akai,
            Some(akai_job()),
            |_| {},
        );
        let write = open("write", theme, Media::ThreeHalf);
        let written = Job::replay("write", WRITTEN);
        render_sized(
            "analyse-write",
            DEFAULT,
            theme,
            write,
            Some(written),
            |_| {},
        );
        // The read as it reaches track 41.0, ringed.
        let reached = WORKBENCH
            .split_inclusive('\n')
            .take_while(|l| !l.starts_with("T41.1"))
            .collect::<String>();
        let mut running = Job::replay("read", &reached);
        running.ended = None;
        running.progress.current = Some((41, 0));
        let read = open("read", theme, Media::ThreeHalf);
        render_sized(
            "analyse-running",
            DEFAULT,
            theme,
            read,
            Some(running),
            |_| {},
        );
        let convert = open("convert", theme, Media::ThreeHalf);
        render_sized(
            "analyse-hover",
            DEFAULT,
            theme,
            convert,
            Some(akai_job()),
            |w| {
                w.run_steps(4);
                w.hover_at(on_disk(w, TRACK_0, 60.0));
            },
        );
        let convert = open("convert", theme, Media::ThreeHalf);
        render_sized(
            "analyse-sector",
            DEFAULT,
            theme,
            convert,
            Some(akai_job()),
            |w| {
                w.run_steps(4);
                let at = on_disk(w, TRACK_0, -20.0);
                w.drag_at(at);
                w.run_steps(2);
                w.drop_at(at);
                w.run_steps(4);
                // Some of its bytes selected, from the first row's into the third.
                let first = w.get_by_label_contains("0000  ").rect();
                let from = first.left_top() + egui::vec2(44.0, first.height() / 2.0);
                w.hover_at(from);
                w.run_steps(2);
                w.drag_at(from);
                w.run_steps(2);
                w.hover_at(from + egui::vec2(180.0, 2.0 * first.height()));
                w.run_steps(2);
                w.drop_at(from + egui::vec2(180.0, 2.0 * first.height()));
            },
        );
        let idle = settings("read", theme);
        render_sized("analyse-greyed", DEFAULT, theme, idle, None, |w| {
            let button = w.get_by_role_and_label(Role::Button, "Analyse").rect();
            w.hover_at(button.center());
        });
        // Detect: the tracks it read, as the format it found decodes them.
        let mut detected = Job::replay(DETECT, DETECTED);
        detected.page = "read".into();
        let mut found = settings("read", theme);
        let read = found.values.entry("read".into()).or_default();
        read.set("format", "amiga.amigados");
        render_sized(
            "analyse-detect",
            DEFAULT,
            theme,
            found,
            Some(detected),
            |w| {
                w.get_by_role_and_label(Role::Button, "Analyse").click();
                w.run_steps(30);
            },
        );
    }
}

/// The Analyse drawer's image view: the file gw makes of a disk, or writes
/// one from, as gw lays it out.
#[test]
#[ignore = "writes pictures for people to look at"]
fn images() {
    let open = |page: &str, theme| Settings {
        drawer: Some(Drawer::Analyse),
        analysis: Analysis::Image,
        ..settings(page, theme)
    };
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        render_sized(
            "image-read",
            DEFAULT,
            theme,
            open("read", theme),
            Some(workbench()),
            |_| {},
        );
        let scratched = converted(SCRATCHED);
        render(
            "image-scratched",
            theme,
            open("convert", theme),
            Some(scratched),
            |_| {},
        );
        render_sized(
            "image-akai",
            DEFAULT,
            theme,
            open("convert", theme),
            Some(akai_job()),
            |_| {},
        );
        let written = Job::replay("write", WRITTEN);
        render_sized(
            "image-write",
            DEFAULT,
            theme,
            open("write", theme),
            Some(written),
            |_| {},
        );
        // The read as it reaches track 41.0.
        let reached = WORKBENCH
            .split_inclusive('\n')
            .take_while(|l| !l.starts_with("T41.1"))
            .collect::<String>();
        let mut running = Job::replay("read", &reached);
        running.ended = None;
        running.progress.current = Some((41, 0));
        render_sized(
            "image-running",
            DEFAULT,
            theme,
            open("read", theme),
            Some(running),
            |_| {},
        );
        // Over the scratch's first lost sector, then a click on it.
        for (name, click) in [("image-hover", false), ("image-part", true)] {
            let mut scratched = converted(SCRATCHED);
            held(&mut scratched, (18, 0));
            render(
                name,
                theme,
                open("convert", theme),
                Some(scratched),
                move |w| {
                    w.run_steps(4);
                    // Track 36 of two columns of 80 rows, its sector 3 of 11.
                    let at = image_part(w, 80, 11)(36, 3);
                    w.hover_at(at);
                    if click {
                        w.drag_at(at);
                        w.run_steps(2);
                        w.drop_at(at);
                    }
                },
            );
        }
        // A shorter window: the file in more columns, where the disks lie.
        render_sized(
            "image-small",
            egui::vec2(1240.0, 560.0),
            theme,
            open("convert", theme),
            Some(converted(SCRATCHED)),
            |_| {},
        );
        // A write as it reaches track 41.0: the file as it is, gw's place in
        // it marked.
        let reached = WRITTEN
            .split_inclusive('\n')
            .take_while(|l| !l.starts_with("T41.1"))
            .collect::<String>();
        let mut writing = Job::replay("write", &reached);
        writing.ended = None;
        writing.progress.current = Some((41, 0));
        render_sized(
            "image-writing",
            DEFAULT,
            theme,
            open("write", theme),
            Some(writing),
            |_| {},
        );
        // Before a write: the file it is to take its tracks from, as gw
        // reads it, gw's filler where the file holds it.
        let mut before = settings("write", theme);
        let write = before.values.entry("write".into()).or_default();
        write.set("file", "/Users/you/Floppies/Workbench.adf");
        render_sized("image-before", DEFAULT, theme, before, None, |w| {
            app_mut(w).pin_image(scratched_adf());
            w.run_steps(2);
            w.get_by_role_and_label(Role::Button, "Analyse").click();
            w.run_steps(30);
        });
        // A read to flux: nothing laid out to show.
        let mut flux = workbench();
        let line = r#"{"event":"open","role":"made","file":"Disk.scp","type":"SCP","layout":null}"#;
        flux.progress.image(line);
        render_sized(
            "image-flux",
            DEFAULT,
            theme,
            open("read", theme),
            Some(flux),
            |w| {
                w.run_steps(4);
                let chip = w
                    .get_by_role_and_label(Role::Button, "Image analysis")
                    .rect();
                w.hover_at(chip.center());
            },
        );
    }
}

/// The analyses' tabs in the Classic palettes, each chosen.
#[test]
#[ignore = "writes pictures for people to look at"]
fn analysis_tabs() {
    for (name, choice) in [("classic", Choice::Classic), ("blue", Choice::Blue)] {
        for (view, analysis) in [("disk", Analysis::Disk), ("image", Analysis::Image)] {
            let settings = Settings {
                theme: choice,
                drawer: Some(Drawer::Analyse),
                analysis,
                ..settings("convert", egui::Theme::Light)
            };
            render_sized(
                &format!("tabs-{name}-{view}"),
                DEFAULT,
                egui::Theme::Light,
                settings,
                Some(converted(SCRATCHED)),
                |_| {},
            );
        }
    }
}

/// The About window's contents, as the window shows them.
fn about(theme: egui::Theme) {
    let mut texture = None;
    let mut harness = Harness::builder()
        .with_size(ferriteweazle::ABOUT_SIZE)
        .with_pixels_per_point(2.0)
        .with_theme(theme)
        .wgpu()
        .build_ui(move |ui| {
            let image = texture.get_or_insert_with(|| {
                ferriteweazle::theme::install(ui.ctx());
                ferriteweazle::theme::apply(ui.ctx(), theme.into());
                ferriteweazle::about_image(ui.ctx())
            });
            // The window's panel, which the harness's root does not fill.
            let whole = ui.ctx().content_rect();
            ui.painter()
                .rect_filled(whole, 0.0, ui.visuals().panel_fill);
            ferriteweazle::about(ui, image, Some("Greaseweazle Tools 1.23"));
        });
    harness.run_steps(5);
    let suffix = match theme {
        egui::Theme::Dark => "dark",
        egui::Theme::Light => "light",
    };
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/screens");
    std::fs::create_dir_all(dir).unwrap();
    let image = harness.render().expect("the window renders");
    image.save(format!("{dir}/about-{suffix}.png")).unwrap();
}

/// A Detect on `page` that found no format, with the bridge's `message`.
fn undetected(page: &str, message: &str) -> Job {
    let read: String = DETECTED
        .lines()
        .take(7)
        .map(|l| l.to_owned() + "\n")
        .collect();
    let log =
        read + "@ferriteweazle result {\"formats\": [], \"step\": 1}\n** FATAL ERROR:\n" + message;
    let mut job = Job::replay(DETECT, &log);
    job.page = page.into();
    job
}

#[test]
#[ignore = "writes pictures for people to look at"]
fn detect_failed() {
    const DISK: &str = "No format Greaseweazle Tools knows reads this disk in full. \
                        Set Disk format to None to read as raw flux (.scp).";
    const IMAGE: &str = "No format Greaseweazle Tools knows reads this image in full. \
                         Set Disk format to None to use its tracks as they are.";
    const IPF: &str = "/Users/you/Documents/Ferriteweazle/Images/Lemmings_Disk1.ipf";
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        for (page, message) in [("read", DISK), ("write", IMAGE), ("convert", IMAGE)] {
            for (size, at) in [("smallest", ferriteweazle::SMALLEST), ("default", DEFAULT)] {
                let mut s = settings(page, theme);
                s.values.entry("write".into()).or_default().set("file", IPF);
                let convert = s.values.entry("convert".into()).or_default();
                convert.set("in_file", IPF);
                convert.set("format", "");
                let job = undetected(page, message);
                render_sized(
                    &format!("detect-failed-{page}-{size}"),
                    at,
                    theme,
                    s,
                    Some(job),
                    |_| {},
                );
            }
        }
    }
}

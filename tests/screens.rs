//! Renders the window in light and dark to target/screens, for looking at.
//!
//!     cargo test --test screens -- --ignored
//!
//! Uses the gw it finds, as the app does, with a made-up Greaseweazle on
//! /dev/cu.usbmodem14201 whatever is plugged in, and replays saved gw output.

use eframe::egui::{self, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ferriteweazle::form::Output;
use ferriteweazle::job::{Job, Outcome};
use ferriteweazle::schema::Port;
use ferriteweazle::{App, Page, Settings};
use std::time::Duration;

type Window<'a> = Harness<'a, Option<App>>;

const DAMAGED: &str = include_str!("data/convert-damaged.log");

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

const FOUND: &str =
    "Found akai.800. Disk also matches eagle.dsqd.800, epson.qx10.400 and zx.quorum.ds80.";

/// What gw prints when Linux refuses it the port: pyserial's EACCES error.
const REFUSED: &str = "** FATAL ERROR:
[Errno 13] could not open port /dev/ttyACM0: [Errno 13] Permission denied: '/dev/ttyACM0'";

/// The smallest the window goes.
const SMALLEST: egui::Vec2 = egui::vec2(980.0, 744.0);

/// The window as the app first opens.
const DEFAULT: egui::Vec2 = egui::vec2(1040.0, 744.0);

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
            |ui, app: &mut Option<App>| {
                let app = app.get_or_insert_with(|| {
                    let mut app = App::with_settings(ui.ctx(), settings.clone());
                    app.pin_ports(vec![Port {
                        device: "/dev/cu.usbmodem14201".into(),
                        name: Some("Greaseweazle".into()),
                        serial: Some("GW0123456789ABCDEF".into()),
                        score: 20,
                        denied: false,
                    }]);
                    app
                });
                match job.take() {
                    Some(job) if job.command == "info" => app.tool = Some(job),
                    Some(job) => app.disk = Some(job),
                    None => {}
                }
                app.show(ui);
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
    convert.set("in_file", "/Users/you/Floppies/Disk07.scp");
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

/// The damaged conversion, told as a read with a retry, for the read page.
fn read_job() -> Job {
    let log = DAMAGED
        .replace("SCP: WARNING: Bad image checksum\n", "")
        .replace(
            "Converting c=0-79:h=0-1 -> c=0-79:h=0-1",
            "Reading c=0-79:h=0-1 revs=2",
        )
        .replace(
            "T21.1: IBM MFM (17/18 sectors) from Raw Flux (188470 flux in 400.80ms)",
            "T21.1: IBM MFM (16/18 sectors) from Raw Flux (188470 flux in 400.80ms)\n\
             T21.1: IBM MFM (17/18 sectors) from Raw Flux (188471 flux in 400.80ms) (Retry #1.1)\n\
             T21.1: Giving up: 1 sectors missing",
        );
    let mut job = Job::replay("read", &log);
    job.format = Some("ibm.1440".into());
    job.output = Some("/Users/you/Documents/Ferriteweazle/Images/Floppy.img".into());
    job
}

/// A Greaseweazle on /dev/ttyACM0 that Linux denies this account, and gw's
/// udev rule where the tarball keeps it.
fn denied(w: &mut Window) {
    let app = w.state_mut().as_mut().unwrap();
    app.udev_rule = Some("/home/you/Ferriteweazle/ferriteweazle-data/49-greaseweazle.rules".into());
    app.pin_ports(vec![Port {
        device: "/dev/ttyACM0".into(),
        name: Some("Greaseweazle".into()),
        serial: Some("GW0123456789ABCDEF".into()),
        score: 20,
        denied: true,
    }]);
}

/// gw info, a read and a drive speed in the session's log.
fn session(w: &mut Window) {
    let log = &mut w.state_mut().as_mut().unwrap().log;
    let rpm = "Rate: 300.121 rpm ; Period: 199.919 ms";
    for (heading, mut job, ending) in [
        ("gw info", Job::replay("info", INFO), "Done in 0:01."),
        (
            "gw read --revs=2 --format=ibm.1440 /Users/you/Documents/Ferriteweazle/Images/Floppy.img",
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
        for (name, size) in [("default", DEFAULT), ("smallest", SMALLEST)] {
            let read = settings("read", theme);
            render_sized(name, size, theme, read, Some(read_job()), |_| {});
        }
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
            w.get_all_by_role_and_label(Role::Button, "Read disk")
                .last()
                .expect("the run button")
                .hover();
        });
        let mut disks = settings("read", theme);
        disks.outputs.get_mut("read/file").unwrap().disks = 3;
        render("disks", theme, disks, None, |w| {
            w.get_by_label_contains("Multiple disks").click();
        });
        let mut replace = settings("read", theme);
        replace.outputs.get_mut("read/file").unwrap().folder = dir.to_string_lossy().into();
        render("overwrite", theme, replace, None, |w| {
            w.get_all_by_role_and_label(Role::Button, "Read disk")
                .last()
                .expect("the run button")
                .click();
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
        for (name, job) in [
            ("read", Some(read_job())),
            ("convert", Some(Job::replay("convert", DAMAGED))),
            ("write", None),
            ("info", Some(Job::replay("info", INFO))),
        ] {
            render(name, theme, settings(name, theme), job, |_| {});
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
        render("settings", theme, page, None, |_| {});
    }
}

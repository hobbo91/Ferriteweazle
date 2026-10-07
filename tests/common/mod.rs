//! What the window tests share: the harness, gw's output and helpers.
#![allow(dead_code)] // each test crate uses some of it

use eframe::egui::{self, accesskit::Role};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, Node};
use ferriteweazle::App;
use ferriteweazle::schema::Port;

pub type Window = Harness<'static, Option<App>>;

pub const DAMAGED: &str = include_str!("../data/convert-damaged.log");

/// DAMAGED as a read would print it: a read loads no .scp to warn about.
pub fn damaged_read() -> String {
    DAMAGED.replace("SCP: WARNING: Bad image checksum\n", "")
}

pub const FOUND: &str =
    "Found akai.800. Disk also matches eagle.dsqd.800, epson.qx10.400 and zx.quorum.ds80.";

/// What gw prints when Linux refuses it the port: pyserial's EACCES error.
pub const REFUSED: &str = "** FATAL ERROR:
[Errno 13] could not open port /dev/ttyACM0: [Errno 13] Permission denied: '/dev/ttyACM0'";

/// The window's size as it opens.
pub const DEFAULT: egui::Vec2 = ferriteweazle::WINDOW;

/// Shows `app` over the whole harness: kittest insets its ui by 8 points, so
/// a harness of the window's size lays the app out as the window does.
pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let rect = ui.ctx().content_rect();
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect);
        app.show(ui);
    });
}

/// A Greaseweazle as gw lists it, on a made-up port.
pub fn greaseweazle() -> Port {
    Port {
        device: "/dev/cu.usbmodem14201".into(),
        name: Some("Greaseweazle".into()),
        score: 20,
        denied: false,
    }
}

pub fn app(w: &Window) -> &App {
    w.state().as_ref().expect("the first frame made the app")
}

pub fn app_mut(w: &mut Window) -> &mut App {
    w.state_mut()
        .as_mut()
        .expect("the first frame made the app")
}

/// The page's run button, not the sidebar entry of the same name.
pub fn run_button<'w>(w: &'w Window, name: &'w str) -> Node<'w> {
    w.get_all_by_role_and_label(Role::Button, name)
        .find(|n| n.rect().left() > 240.0)
        .expect("the run button")
}

/// What the command line holds.
pub fn line(w: &Window) -> String {
    w.get_by_role(Role::MultilineTextInput)
        .value()
        .unwrap_or_default()
}

/// The disk map's squares, larger than the legend's 10-point swatches.
pub fn squares(w: &Window) -> impl Iterator<Item = &egui::epaint::RectShape> {
    let left = w.get_by_label("Disk status").rect().left();
    w.output()
        .shapes
        .iter()
        .filter_map(move |c| match &c.shape {
            egui::Shape::Rect(r)
                if r.rect.left() > left
                    && (r.rect.width() - r.rect.height()).abs() < 0.5
                    && r.rect.width() > 10.5 =>
            {
                Some(r)
            }
            _ => None,
        })
}

/// A point on head 0's disk, the first of the two side by side in the disk
/// map, each under its side's name, 20 points tall, 32 points apart:
/// `share` of the way from its centre to its edge, at `degrees` from the
/// right, anticlockwise.
pub fn on_disk(w: &Window, share: f32, degrees: f32) -> egui::Pos2 {
    let map = w.get_by_label("Disk map").rect();
    let diameter = map.height() - 20.0;
    let left = map.center().x - (2.0 * diameter + 32.0) / 2.0;
    let radius = diameter / 2.0;
    let centre = egui::pos2(left + radius, map.top() + 20.0 + radius);
    let a = degrees.to_radians();
    centre + share * radius * egui::vec2(a.cos(), -a.sin())
}

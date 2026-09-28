//! The desktop's light or dark preference on Linux, from the XDG desktop
//! portal, which winit never asks.

use crate::service::Repaint;
use eframe::egui::Theme;
use std::sync::mpsc::{self, Receiver, Sender};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// The preference now and at each change, on a thread. Nothing comes when
/// there is no portal.
pub fn watch(repaint: Repaint) -> Receiver<Theme> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = follow(&tx, &repaint);
    });
    rx
}

fn follow(tx: &Sender<Theme>, repaint: &Repaint) -> zbus::Result<()> {
    let bus = Connection::session()?;
    let portal = Proxy::new(
        &bus,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )?;
    // Listening first, so a change while reading is not missed.
    let changes = portal.receive_signal_with_args("SettingChanged", &[(0, NAMESPACE), (1, KEY)])?;
    // Portals older than version 2 have only Read.
    let now: OwnedValue = portal
        .call("ReadOne", &(NAMESPACE, KEY))
        .or_else(|_| portal.call("Read", &(NAMESPACE, KEY)))?;
    let changed = changes.map(|change| {
        let body = change.body();
        body.deserialize::<(String, String, OwnedValue)>()
            .map(|(_, _, value)| value)
    });
    for value in std::iter::once(Ok(now)).chain(changed).flatten() {
        let Some(theme) = theme(&value) else { continue };
        if tx.send(theme).is_err() {
            break; // the window has closed
        }
        repaint();
    }
    Ok(())
}

/// color-scheme's value: 1 prefers dark, 2 light, and 0 states no
/// preference, which GNOME shows as light. Read wraps it in one more variant.
fn theme(value: &Value) -> Option<Theme> {
    match value {
        Value::Value(inner) => theme(inner),
        Value::U32(1) => Some(Theme::Dark),
        Value::U32(0 | 2) => Some(Theme::Light),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portals_colour_scheme_is_dark_light_or_no_preference() {
        assert_eq!(theme(&Value::U32(1)), Some(Theme::Dark));
        assert_eq!(theme(&Value::U32(2)), Some(Theme::Light));
        assert_eq!(theme(&Value::U32(0)), Some(Theme::Light));
        assert_eq!(theme(&Value::U32(3)), None);
        assert_eq!(theme(&Value::from("dark")), None);
    }

    #[test]
    fn an_older_portals_read_wraps_the_value_in_a_variant() {
        let read = Value::Value(Box::new(Value::U32(1)));
        assert_eq!(theme(&read), Some(Theme::Dark));
    }
}

//! What `gw info` says about the connected Greaseweazle.

/// The device part of `gw info`, such as `Model: Greaseweazle V4.1`.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub fields: Vec<(String, String)>,
    /// A newer firmware version, if gw found one.
    pub update: Option<String>,
    /// gw's steps to update, as it prints them: `- Reconnect to USB`.
    pub steps: Vec<String>,
}

impl DeviceInfo {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Reads `gw info`'s output; None if gw found no device. The fields precede
/// the firmware check, so they survive its failure.
pub fn parse(log: &[String]) -> Option<DeviceInfo> {
    let fields: Vec<_> = log
        .iter()
        .skip_while(|l| l.trim_end() != "Device:")
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let update = log.iter().find_map(|l| {
        l.trim()
            .strip_prefix("*** New firmware version ")?
            .strip_suffix(" is available")
            .map(str::to_owned)
    });
    let steps = log
        .iter()
        .skip_while(|l| l.trim_end() != "To perform an Update:")
        .skip(1)
        .take_while(|l| l.starts_with(" - "))
        .map(|l| l.trim().to_owned())
        .collect();
    (!fields.is_empty()).then_some(DeviceInfo {
        fields,
        update,
        steps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    #[test]
    fn a_device_report_is_read_field_by_field() {
        let log = lines(
            "Host Tools: 1.23\n\
             Device:\n\
             \x20 Port:     /dev/cu.usbmodem14201\n\
             \x20 Model:    Greaseweazle V4.1\n\
             \x20 MCU:      AT32F403A, 216MHz, 224kB SRAM\n\
             \x20 Firmware: 1.6\n\
             \x20 Serial:   GW0123456789ABCDEF\n\
             \x20 USB:      Full Speed (12 Mbit/s), 128kB Buffer\n\
             \n\
             *** New firmware version 1.7 is available\n\
             To perform an Update:\n\
             \x20- Run \"gw update\" to download and install latest firmware",
        );
        let info = parse(&log).unwrap();
        assert_eq!(info.get("Model"), Some("Greaseweazle V4.1"));
        assert_eq!(info.get("Port"), Some("/dev/cu.usbmodem14201"));
        assert_eq!(
            info.get("USB"),
            Some("Full Speed (12 Mbit/s), 128kB Buffer")
        );
        assert_eq!(info.fields.len(), 6);
        assert_eq!(info.update.as_deref(), Some("1.7"));
    }

    #[test]
    fn no_device_is_no_report() {
        assert_eq!(
            parse(&lines("Host Tools: 1.23\nDevice:\n  Not found")),
            None
        );
        assert_eq!(
            parse(&lines(
                "** FATAL ERROR:\nCannot find the Greaseweazle device"
            )),
            None
        );
    }

    #[test]
    fn a_failed_firmware_check_keeps_the_fields() {
        let log = lines(
            "Device:\n  Model:    Greaseweazle F7 Plus\n** FATAL ERROR:\nGitHub API Rate Limit exceeded",
        );
        let info = parse(&log).unwrap();
        assert_eq!(info.get("Model"), Some("Greaseweazle F7 Plus"));
        assert_eq!(info.update, None);
    }
}

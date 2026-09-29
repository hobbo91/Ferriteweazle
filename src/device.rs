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
    let mut info = DeviceInfo {
        fields,
        update,
        steps,
    };
    // gw offers Greaseweazle's newest firmware to any older one, Adafruit's
    // too, which gw cannot update.
    if info.get("Model").is_some_and(adafruit::model) {
        info.update = None;
        info.steps.clear();
    }
    (!info.fields.is_empty()).then_some(info)
}

/// The device the card drives. gw finds a Greaseweazle by itself; an
/// Adafruit RP2040 only on the port chosen for it, as its firmware names
/// itself nothing gw looks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Greaseweazle,
    Adafruit,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Greaseweazle => "Greaseweazle",
            Kind::Adafruit => adafruit::NAME,
        }
    }
}

/// What gw 1.23 cannot do on Adafruit's Greaseweazle-compatible firmware
/// (Adafruit_Floppy's examples/greaseweazle and library, 0.6.1 and main).
pub mod adafruit {
    pub const NAME: &str = "Adafruit RP2040";
    /// Its library clamps a seek to FLOPPY_IBMPC_HD_TRACKS - 1 and reports success.
    pub const LAST_CYLINDER: u32 = 79;
    /// Its SELECT takes unit 0 alone, on either bus: gw's A and 0.
    pub const DRIVES: &[&str] = &["A", "0"];
    /// Its GETPIN answers pin 26 alone, and SETPIN pin 2.
    pub const GET_PIN: u32 = 26;
    pub const SET_PIN: u32 = 2;
    /// Why an option greys.
    pub const OPTION: &str = "Not possible on the Adafruit RP2040.";

    /// Whether gw info's Model names hardware model 8, Adafruit's: gw names
    /// its submodel 0 alone and gives others by number.
    pub fn model(model: &str) -> bool {
        model == "Adafruit Floppy Generic" || model.starts_with("Unknown (0x08")
    }

    /// Why gw cannot run `command` on it at all.
    pub fn command(command: &str) -> Option<&'static str> {
        match command {
            // No EraseFlux; --hfreq's Astable opcode is skipped by its flux
            // decoder, which then writes nothing.
            "erase" => Some("The Adafruit RP2040 cannot erase disks."),
            // No SwitchFwMode, and no firmware for its hardware model, 8.
            "update" => Some("gw cannot update the Adafruit RP2040's firmware."),
            // gw asks for 16 bytes of delays and steps down only on an error;
            // the firmware sends its 10 with none, so gw waits for ever.
            "delays" => Some("gw delays hangs on the Adafruit RP2040."),
            // gw reset reads the delays first.
            "reset" => Some("gw reset hangs on the Adafruit RP2040."),
            _ => None,
        }
    }

    /// Whether `command`'s option `dest` cannot work on it.
    pub fn option(command: &str, dest: &str) -> bool {
        matches!(
            (command, dest),
            // Density select and TG43 read pin 2 first, which GETPIN refuses.
            // Hard sectors need each hole's index time; a timed capture keeps
            // the last alone.
            ("read" | "write", "densel" | "gen_tg43" | "hard_sectors")
                // Each erases a track.
                | ("write", "pre_erase" | "erase_empty")
                // No SwitchFwMode.
                | ("info", "bootloader")
        )
    }
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
        let silent =
            "Host Tools: 1.23\nDevice:\n** FATAL ERROR:\nGreaseweazle interface did not answer.";
        assert_eq!(parse(&lines(silent)), None);
    }

    #[test]
    fn gws_offer_of_greaseweazle_firmware_to_the_adafruit_rp2040_is_dropped() {
        // gw info as its code prints it for hardware model 8, firmware 1.0:
        // no MCU line, no USB buffer, and the one update step.
        let log = lines(
            "Host Tools: 1.23\n\
             Device:\n\
             \x20 Port:     /dev/cu.usbmodem1101\n\
             \x20 Model:    Adafruit Floppy Generic\n\
             \x20 Firmware: 1.0\n\
             \x20 Serial:   E6614C311B2F6A2A\n\
             \x20 USB:      Full Speed (12 Mbit/s)\n\
             \n\
             *** New firmware version 1.7 is available\n\
             To perform an Update:\n\
             \x20- Run \"gw update\" to download and install latest firmware",
        );
        let info = parse(&log).unwrap();
        assert_eq!(info.get("Model"), Some("Adafruit Floppy Generic"));
        assert_eq!(info.get("Firmware"), Some("1.0"));
        assert_eq!((info.update, info.steps.len()), (None, 0));
        assert!(adafruit::model("Unknown (0x0801)"));
        assert!(!adafruit::model("Greaseweazle V4.1"));
        assert!(!adafruit::model("Unknown (0x0400)"));
    }

    #[test]
    fn the_adafruit_rp2040_refuses_what_its_firmware_cannot_do() {
        for command in ["erase", "update", "delays", "reset"] {
            assert!(adafruit::command(command).is_some(), "{command}");
        }
        let fine = ["read", "write", "convert", "clean", "seek", "rpm", "info"];
        let fine = fine.into_iter().chain(["bandwidth", "pin get", "pin set"]);
        for command in fine {
            assert_eq!(adafruit::command(command), None, "{command}");
        }
        for (command, dest) in [
            ("read", "densel"),
            ("read", "gen_tg43"),
            ("read", "hard_sectors"),
            ("write", "densel"),
            ("write", "gen_tg43"),
            ("write", "hard_sectors"),
            ("write", "pre_erase"),
            ("write", "erase_empty"),
            ("info", "bootloader"),
        ] {
            assert!(adafruit::option(command, dest), "{command} {dest}");
        }
        for (command, dest) in [
            ("read", "fake_index"),
            ("read", "revs"),
            ("write", "fake_index"),
            ("write", "no_verify"),
            ("write", "precomp"),
            ("seek", "motor_on"),
        ] {
            assert!(!adafruit::option(command, dest), "{command} {dest}");
        }
        assert_eq!(Kind::default(), Kind::Greaseweazle);
    }

    #[test]
    fn a_failed_firmware_check_keeps_the_fields() {
        let log = lines(
            "Device:\n  Model:    Greaseweazle F7 Plus (Ant Goffart, v1)\n** FATAL ERROR:\nGitHub API Rate Limit exceeded",
        );
        let info = parse(&log).unwrap();
        assert_eq!(
            info.get("Model"),
            Some("Greaseweazle F7 Plus (Ant Goffart, v1)")
        );
        assert_eq!(info.update, None);
    }
}

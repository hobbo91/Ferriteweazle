//! gw as a standalone program, such as the gw.exe of gw's Windows download,
//! whose Python is sealed inside it, so the bridge cannot run there. Its pages
//! come from its help, and what help does not print (value types, options
//! that exclude each other, image types) from gw 1.23's schema.

use crate::engine::quiet;
use crate::schema::{Arg, Command, Image, Port, Schema};
use std::collections::BTreeMap;
use std::path::Path;

/// gw 1.23's schema, as the bridge reads it from gw's own parsers.
const KNOWN: &str = include_str!("gw-1.23.json");

/// The schema of the standalone gw at `gw`, from its help.
pub fn schema(gw: &Path) -> Result<Schema, String> {
    let mut schema = from_help(|args| help(gw, args))
        .map_err(|e| format!("{} is not Greaseweazle Tools: {e}", gw.display()))?;
    schema.version = version(gw).unwrap_or_default();
    Ok(schema)
}

/// What `gw ARGS --help` prints: gw prints everything on stderr.
fn help(gw: &Path, args: &[&str]) -> Result<String, String> {
    let out = quiet(std::process::Command::new(gw))
        .args(args)
        .arg("--help")
        .output()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout))
}

/// The release gw info names on its first line, "Host Tools: 1.23", asked
/// of a port that cannot exist so that it opens no device.
fn version(gw: &Path) -> Option<String> {
    let out = quiet(std::process::Command::new(gw))
        .args(["info", "--device=ferriteweazle-no-such-port"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let version = text.lines().find_map(|l| l.strip_prefix("Host Tools:"))?;
    Some(version.trim().to_owned())
}

/// The schema from gw's help, where `help(ARGS)` is what `gw ARGS --help` prints.
pub fn from_help<H>(help: H) -> Result<Schema, String>
where
    H: Fn(&[&str]) -> Result<String, String> + Sync,
{
    let actions = actions(&help(&[])?);
    if actions.is_empty() {
        return Err("its help lists no gw commands.".into());
    }
    // Each gw start takes a moment, so the commands' help is read at once.
    let pages = std::thread::scope(|s| {
        let reading: Vec<_> = actions
            .iter()
            .map(|a| s.spawn(|| pages(&help, a)))
            .collect();
        let read = reading
            .into_iter()
            .map(|r| r.join().unwrap_or_else(|_| Err("it failed.".into())));
        read.collect::<Result<Vec<_>, _>>()
    })?;
    let known: Schema = serde_json::from_str(KNOWN).expect("gw-1.23.json is a schema");
    let mut schema = Schema {
        version: String::new(),
        commands: Vec::new(),
        formats: Vec::new(),
        images: BTreeMap::new(),
        notes: BTreeMap::new(),
    };
    let mut suffixes = Vec::new();
    for (name, text) in pages.into_iter().flatten() {
        let blocks = blocks(&text);
        schema
            .commands
            .push(command(&name, &blocks, known.command(&name)));
        for block in &blocks {
            epilog(block, &mut schema, &mut suffixes);
        }
    }
    schema.images = match suffixes.is_empty() {
        true => known.images,
        false => images(&known, &suffixes),
    };
    Ok(schema)
}

/// A command's help, or each of its subcommands' as `pin get` and `pin set`.
fn pages<H>(help: &H, name: &str) -> Result<Vec<(String, String)>, String>
where
    H: Fn(&[&str]) -> Result<String, String>,
{
    let text = help(&[name])?;
    // "usage: gw pin get|set [-h] ..."
    let subs = text.split_whitespace().nth(3).filter(|w| w.contains('|'));
    match subs {
        Some(subs) => subs
            .split('|')
            .map(|sub| Ok((format!("{name} {sub}"), help(&[name, sub])?)))
            .collect(),
        None => Ok(vec![(name.to_owned(), text)]),
    }
}

/// The actions under "Actions:" in gw's own help.
fn actions(top: &str) -> Vec<String> {
    top.lines()
        .skip_while(|l| l.trim_end() != "Actions:")
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

/// The help's paragraphs, split at blank lines.
fn blocks(text: &str) -> Vec<Vec<&str>> {
    let mut blocks = vec![Vec::new()];
    for line in text.lines().map(str::trim_end) {
        match (line.is_empty(), blocks.last_mut()) {
            (true, Some(last)) if last.is_empty() => {}
            (true, _) => blocks.push(Vec::new()),
            (false, Some(last)) => last.push(line),
            (false, None) => {}
        }
    }
    blocks.retain(|b| !b.is_empty());
    blocks
}

/// A command from its help: usage, description, then argparse's sections.
/// Its arguments keep gw 1.23's order, and any it lacks follow.
fn command(name: &str, blocks: &[Vec<&str>], known: Option<&Command>) -> Command {
    let usage = blocks.first().map(|b| b.join(" ")).unwrap_or_default();
    let about = blocks
        .get(1)
        .filter(|b| !b[0].ends_with(':'))
        .map(|b| b.join(" "))
        .unwrap_or_default();
    let mut args = Vec::new();
    for block in blocks {
        let positional = match block[0] {
            "positional arguments:" => true,
            "options:" | "optional arguments:" => false,
            _ => continue,
        };
        for (invocation, help) in entries(&block[1..]) {
            let mut arg = arg(&invocation, &help, positional, &usage);
            if arg.dest == "help" {
                continue;
            }
            if let Some(k) = known.and_then(|k| k.arg(&arg.dest)) {
                arg.ty.clone_from(&k.ty);
                arg.group = k.group;
            }
            args.push(arg);
        }
    }
    let place = |a: &Arg| {
        let known = known.map(|k| k.args.as_slice()).unwrap_or_default();
        known
            .iter()
            .position(|k| k.dest == a.dest)
            .unwrap_or(usize::MAX)
    };
    args.sort_by_key(place);
    Command {
        name: name.to_owned(),
        about,
        args,
    }
}

/// argparse's entries: `  --revs N   number of revolutions`, where a long
/// entry's help starts on the next line and help runs on indented lines.
fn entries(lines: &[&str]) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = Vec::new();
    for line in lines {
        let Some(body) = line.strip_prefix("  ") else {
            continue;
        };
        if !body.starts_with(' ') {
            let (invocation, help) = body.split_once("  ").unwrap_or((body, ""));
            entries.push((invocation.trim().to_owned(), help.trim().to_owned()));
        } else if let Some((_, help)) = entries.last_mut() {
            if !help.is_empty() {
                help.push(' ');
            }
            help.push_str(body.trim());
        }
    }
    entries
}

/// An argument from its entry: `--densel, --dd LEVEL` and its help. A value's
/// help ends with "(default: X)" where it has one; a switch's is gw's own words.
fn arg(invocation: &str, help: &str, positional: bool, usage: &str) -> Arg {
    let mut arg = Arg {
        flags: Vec::new(),
        dest: invocation.to_owned(),
        switch: false,
        ty: None,
        default: None,
        choices: Vec::new(),
        // An optional positional shows as [name] in the usage.
        required: positional && !usage.contains(&format!("[{invocation}]")),
        group: None,
        metavar: None,
        help: help.to_owned(),
    };
    if positional {
        return arg;
    }
    let parts: Vec<&str> = invocation.split(", ").collect();
    arg.flags = parts
        .iter()
        .filter_map(|p| p.split(' ').next())
        .map(str::to_owned)
        .collect();
    let long = arg.flags.iter().find(|f| f.starts_with("--"));
    let first = long.or(arg.flags.first()).map_or("", String::as_str);
    arg.dest = first.trim_start_matches('-').replace('-', "_");
    let metavar = parts.last().and_then(|p| p.split_once(' ')).map(|(_, m)| m);
    arg.switch = metavar.is_none();
    if let Some(choices) = metavar.and_then(|m| m.strip_prefix('{')?.strip_suffix('}')) {
        arg.choices = choices.split(',').map(str::to_owned).collect();
    }
    // argparse shows DEST in capitals where gw names no value of its own.
    arg.metavar = metavar
        .filter(|m| *m != arg.dest.to_uppercase() && arg.choices.is_empty())
        .map(str::to_owned);
    let default = help
        .strip_suffix(')')
        .and_then(|h| h.rsplit_once(" (default: "));
    // gw's own words may say "(default: --tracks)": a default never names an option.
    if let Some((help, default)) = default.filter(|(_, d)| !arg.switch && !d.starts_with('-')) {
        arg.help = help.to_owned();
        arg.default = Some(default.to_owned());
    }
    arg
}

/// The notes, formats and image suffixes after a command's options.
fn epilog(block: &[&str], schema: &mut Schema, suffixes: &mut Vec<String>) {
    let words = || {
        block[1..]
            .iter()
            .flat_map(|l| l.split_whitespace())
            .map(str::to_owned)
    };
    match block[0] {
        "FORMAT options:" if schema.formats.is_empty() => schema.formats = words().collect(),
        "Supported file suffixes:" if suffixes.is_empty() => *suffixes = words().collect(),
        head => {
            // A note is headed `TSPEC: ...`.
            let name = head.split_once(": ").map(|(n, _)| n);
            if let Some(name) = name.filter(|n| n.chars().all(|c| c.is_ascii_uppercase())) {
                schema
                    .notes
                    .insert(name.to_owned(), block.join("\n") + "\n");
            }
        }
    }
}

/// gw 1.23's image type for each suffix gw lists, or one it can only read.
fn images(known: &Schema, suffixes: &[String]) -> BTreeMap<String, Image> {
    let image = |ext: &String| {
        known.images.get(ext).cloned().unwrap_or_else(|| Image {
            name: ext.trim_start_matches('.').to_uppercase(),
            writable: false,
            default_format: None,
            finds_format: false,
            tracks: false,
            needs_format: false,
            read_opts: Vec::new(),
            write_opts: Vec::new(),
        })
    };
    suffixes
        .iter()
        .map(|ext| (ext.clone(), image(ext)))
        .collect()
}

/// Serial ports as Windows lists them, with gw's score for a Greaseweazle by
/// its USB ID. Elsewhere a standalone gw finds its device by itself.
pub fn ports() -> Vec<Port> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let present = reg(r"HKLM\HARDWARE\DEVICEMAP\SERIALCOMM", &[]);
    let mut ports: Vec<Port> = values(&present)
        .map(|device| Port {
            device,
            name: None,
            score: 0,
            denied: false,
        })
        .collect();
    // gw's own IDs, and their scores (util.py's score_port).
    for (id, score) in [("VID_1209&PID_4D69", 20), ("VID_1209&PID_0001", 10)] {
        let key = format!(r"HKLM\SYSTEM\CurrentControlSet\Enum\USB\{id}");
        for device in values(&reg(&key, &["/s", "/v", "PortName"])) {
            if let Some(port) = ports.iter_mut().find(|p| p.device == device) {
                port.score = port.score.max(score);
                port.name = Some("Greaseweazle".into());
            }
        }
    }
    ports.sort_by_key(|p| -p.score);
    ports
}

/// What `reg query KEY ARGS` prints, or nothing.
fn reg(key: &str, args: &[&str]) -> String {
    let out = quiet(std::process::Command::new("reg"))
        .args(["query", key])
        .args(args)
        .output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// The data of each REG_SZ value in `reg query`'s output: `    PortName    REG_SZ    COM3`.
fn values(text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines().filter_map(|line| {
        let mut words = line.split_whitespace();
        words.next()?;
        (words.next()? == "REG_SZ").then(|| words.next().map(str::to_owned))?
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// gw 1.23's help, as recorded in tests/data/help-1.23.
    fn recorded(args: &[&str]) -> Result<String, String> {
        let name = match args {
            [] => "gw".to_owned(),
            args => args.join(" "),
        };
        let path = format!(
            "{}/tests/data/help-1.23/{name}.txt",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))
    }

    #[test]
    fn gws_help_gives_the_schema_its_parsers_give() {
        let from_help = from_help(recorded).unwrap();
        let known: Schema = serde_json::from_str(KNOWN).unwrap();
        let json = |s: &Schema| serde_json::to_value(s).unwrap();
        let (mut help, mut parsers) = (json(&from_help), json(&known));
        for schema in [&mut help, &mut parsers] {
            schema["version"] = Value::Null;
            let formats = schema["formats"].as_array_mut().unwrap();
            formats.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
        assert_eq!(help["commands"].as_array().unwrap().len(), 14);
        for (h, p) in help["commands"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(parsers["commands"].as_array().unwrap())
        {
            // gw's help describes pin get in pin set's words; the page has its own.
            if h["name"] == "pin get" {
                h["about"] = p["about"].clone();
            }
            assert_eq!(h, p);
        }
        assert_eq!(help["notes"], parsers["notes"]);
        assert_eq!(help["formats"], parsers["formats"]);
    }

    #[test]
    fn a_program_whose_help_lists_no_gw_commands_is_not_gw() {
        let other = |_: &[&str]| Ok("usage: something [-h]\n".to_owned());
        assert!(from_help(other).is_err());
    }

    #[test]
    fn windows_lists_serial_ports_as_reg_values() {
        let serialcomm = "\r\nHKEY_LOCAL_MACHINE\\HARDWARE\\DEVICEMAP\\SERIALCOMM\r\n    \
            \\Device\\USBSER000    REG_SZ    COM3\r\n    \\Device\\USBSER001    REG_SZ    COM9\r\n";
        assert_eq!(values(serialcomm).collect::<Vec<_>>(), ["COM3", "COM9"]);
        let enumerated = "HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Enum\\USB\\VID_1209&PID_4D69\\GW01\\Device Parameters\r\n    \
            PortName    REG_SZ    COM3\r\n\r\nEnd of search: 1 match(es) found.\r\n";
        assert_eq!(values(enumerated).collect::<Vec<_>>(), ["COM3"]);
    }
}

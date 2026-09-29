//! A command's settings, and the gw arguments they stand for.

use crate::schema::{Arg, Command, Schema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Value of a switch that is on.
pub const ON: &str = "on";

/// Settings by argument `dest`. A missing value leaves gw's default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Values(BTreeMap<String, String>);

impl Values {
    pub fn get(&self, dest: &str) -> &str {
        self.0.get(dest).map_or("", String::as_str)
    }

    pub fn set(&mut self, dest: &str, value: impl Into<String>) {
        let value = value.into();
        if value.is_empty() {
            self.0.remove(dest);
        } else {
            self.0.insert(dest.to_owned(), value);
        }
    }

    pub fn on(&self, dest: &str) -> bool {
        !self.get(dest).is_empty()
    }
}

/// The arguments after `gw` for `cmd` with `values`.
pub fn argv(cmd: &Command, values: &Values) -> Vec<String> {
    let mut out: Vec<String> = cmd.name.split(' ').map(String::from).collect();
    for a in &cmd.args {
        let Some(flag) = a.flag() else { continue };
        match values.get(&a.dest) {
            "" => {}
            _ if a.switch => out.push(flag.to_owned()),
            value => out.push(format!("{flag}={value}")),
        }
    }
    // gw fills positionals in order: one after an empty one would take its place.
    let positional = cmd
        .args
        .iter()
        .filter(|a| a.positional())
        .map(|a| values.get(&a.dest))
        .take_while(|v| !v.is_empty());
    if positional.clone().any(|v| v.starts_with('-')) {
        out.push("--".into());
    }
    out.extend(positional.map(String::from));
    out
}

/// Required arguments with no value yet.
pub fn missing<'a>(cmd: &'a Command, values: &Values) -> impl Iterator<Item = &'a Arg> {
    cmd.args
        .iter()
        .filter(|a| a.required && values.get(&a.dest).is_empty())
}

/// A pasted gw command line, as its command name, its settings, and whether
/// it asks for Python tracebacks (gw's --bt).
pub fn parse(schema: &Schema, line: &str) -> Result<(String, Values, bool), String> {
    let words = split(line)?;
    let mut words = words.iter().map(String::as_str).peekable();
    if words.next_if(|w| is_gw(w)).is_none() {
        return Err("A command starts with gw.".into());
    }
    // gw's own options, which it takes only before the command. Every job is
    // timed, so --time adds nothing.
    let mut backtrace = false;
    while let Some(word) = words.next_if(|w| w.starts_with("--")) {
        match word {
            "--bt" => backtrace = true,
            "--time" => {}
            _ => {
                let flag = word.split_once('=').map_or(word, |(f, _)| f);
                return Err(format!("gw has no option {flag}."));
            }
        }
    }
    let first = words
        .next()
        .ok_or("Paste a gw command, such as: gw read --format=ibm.1440 disk.img")?;
    let sub = words
        .peek()
        .and_then(|second| schema.command(&format!("{first} {second}")));
    if sub.is_some() {
        words.next();
    }
    let cmd = sub
        .or_else(|| schema.command(first))
        .ok_or_else(|| format!("gw has no command called \"{first}\"."))?;

    let mut values = Values::default();
    let mut positional = cmd.args.iter().filter(|a| a.positional());
    let mut options_done = false;
    while let Some(word) = words.next() {
        if word == "--" {
            options_done = true;
        } else if word.starts_with('-') && word.len() > 1 && !options_done {
            let (flag, inline) = match word.split_once('=') {
                Some((f, v)) => (f, Some(v)),
                None => (word, None),
            };
            let arg = cmd
                .args
                .iter()
                .find(|a| a.flags.iter().any(|f| f == flag))
                .ok_or_else(|| format!("gw {} has no option {flag}.", cmd.name))?;
            let value = match (arg.switch, inline) {
                (true, None) => ON,
                (true, Some(_)) => return Err(format!("{flag} takes no value.")),
                (false, Some(v)) => v,
                (false, None) => words
                    .next()
                    .ok_or_else(|| format!("{flag} needs a value."))?,
            };
            values.set(&arg.dest, value);
        } else {
            let arg = positional
                .next()
                .ok_or_else(|| format!("Unexpected \"{word}\"."))?;
            values.set(&arg.dest, word);
        }
    }
    // gw refuses two options of one exclusive group.
    for a in cmd
        .args
        .iter()
        .filter(|a| a.group.is_some() && values.on(&a.dest))
    {
        let clash = |b: &&Arg| b.group == a.group && b.dest != a.dest && values.on(&b.dest);
        if let Some(b) = cmd.args.iter().find(clash) {
            let (a, b) = (a.flag().unwrap_or(&a.dest), b.flag().unwrap_or(&b.dest));
            return Err(format!("{a} cannot be used with {b}."));
        }
    }
    Ok((cmd.name.clone(), values, backtrace))
}

fn is_gw(word: &str) -> bool {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    ["gw", "gw.exe", "gw.py"]
        .iter()
        .any(|n| name.eq_ignore_ascii_case(n))
}

/// Words of a command line, honouring quotes the way a shell does.
fn split(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                let w = word.get_or_insert_default();
                loop {
                    match chars.next() {
                        Some(q) if q == c => break,
                        Some('\\') if c == '"' && chars.peek() == Some(&'"') => {
                            w.extend(chars.next());
                        }
                        Some(ch) => w.push(ch),
                        None => return Err("A quote is not closed.".into()),
                    }
                }
            }
            // A backslash escapes a space or a quote; any other stays, as in a Windows path.
            '\\' if matches!(chars.peek(), Some(' ' | '\'' | '"')) => {
                word.get_or_insert_default().extend(chars.next());
            }
            // A backslash ending a line joins the next to it, as in a shell.
            '\\' if matches!(chars.peek(), Some('\n' | '\r')) => {
                chars.next_if_eq(&'\r');
                chars.next_if_eq(&'\n');
            }
            c if c.is_whitespace() => words.extend(word.take()),
            c => word.get_or_insert_default().push(c),
        }
    }
    words.extend(word);
    Ok(words)
}

/// A word, quoted if a shell needs it.
pub fn quote(word: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./:=,+@%".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        word.to_owned()
    } else if cfg!(windows) {
        format!("\"{}\"", word.replace('"', "\\\""))
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The whole command line, quoted for a shell.
pub fn line(args: &[String]) -> String {
    args.iter()
        .fold("gw".to_owned(), |out, a| out + " " + &quote(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        serde_json::from_str(include_str!("../tests/data/schema-1.23.json")).unwrap()
    }

    fn values(pairs: &[(&str, &str)]) -> Values {
        let mut v = Values::default();
        pairs.iter().for_each(|(k, x)| v.set(k, *x));
        v
    }

    #[test]
    fn options_use_their_canonical_flag_and_positionals_come_last() {
        let s = schema();
        let read = s.command("read").unwrap();
        let v = values(&[
            ("file", "a b.img"),
            ("densel", "H"),
            ("raw", ON),
            ("revs", "2"),
        ]);
        assert_eq!(
            argv(read, &v),
            ["read", "--revs=2", "--raw", "--densel=H", "a b.img"]
        );
    }

    #[test]
    fn subcommands_are_two_words() {
        let s = schema();
        let v = values(&[("pin", "2"), ("level", "H")]);
        assert_eq!(
            argv(s.command("pin set").unwrap(), &v),
            ["pin", "set", "2", "H"]
        );
    }

    #[test]
    fn a_positional_that_looks_like_an_option_is_protected() {
        let s = schema();
        let v = values(&[("file", "-odd.img")]);
        assert_eq!(
            argv(s.command("write").unwrap(), &v),
            ["write", "--", "-odd.img"]
        );
    }

    #[test]
    fn required_arguments_are_reported_until_set() {
        let s = schema();
        let convert = s.command("convert").unwrap();
        let dests: Vec<&str> = missing(convert, &Values::default())
            .map(|a| a.dest.as_str())
            .collect();
        assert_eq!(dests, ["in_file", "out_file"]);
        let v = values(&[("in_file", "a.scp"), ("out_file", "b.img")]);
        assert_eq!(missing(convert, &v).count(), 0);
    }

    #[test]
    fn a_pasted_command_fills_the_settings_it_names() {
        let s = schema();
        let line = r#"gw read --format ibm.1440 --tracks=c=0-39:h=0 --dd H -n "My Disk.img""#;
        let (name, v, _) = parse(&s, line).unwrap();
        assert_eq!(name, "read");
        assert_eq!(
            v,
            values(&[
                ("format", "ibm.1440"),
                ("tracks", "c=0-39:h=0"),
                ("densel", "H"),
                ("no_clobber", ON),
                ("file", "My Disk.img")
            ])
        );
    }

    #[test]
    fn a_pasted_command_round_trips() {
        let s = schema();
        let v = values(&[
            ("in_file", "C:\\disks\\x.scp"),
            ("out_file", "it's.img"),
            ("format", "amiga.amigados"),
        ]);
        let line = line(&argv(s.command("convert").unwrap(), &v));
        assert_eq!(parse(&s, &line).unwrap(), ("convert".into(), v, false));
    }

    #[test]
    fn pasting_accepts_a_path_to_gw_and_its_global_options() {
        let s = schema();
        let (name, v, backtrace) = parse(&s, "/usr/local/bin/gw --time pin get 34").unwrap();
        assert_eq!((name.as_str(), v.get("pin")), ("pin get", "34"));
        assert!(!backtrace);
        let (_, _, backtrace) = parse(&s, "gw --bt --time info").unwrap();
        assert!(backtrace, "--bt asks for tracebacks");
    }

    #[test]
    fn pasting_explains_what_it_cannot_read() {
        let s = schema();
        assert_eq!(
            parse(&s, "gw frobnicate").unwrap_err(),
            "gw has no command called \"frobnicate\"."
        );
        assert!(
            parse(&s, "gw read --bogus x.img")
                .unwrap_err()
                .contains("--bogus")
        );
        assert!(
            parse(&s, "gw read --format")
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(parse(&s, "gw read 'x.img").unwrap_err().contains("quote"));
        assert_eq!(
            parse(&s, "gw read x.img y.img").unwrap_err(),
            "Unexpected \"y.img\"."
        );
        assert_eq!(
            parse(&s, "gw").unwrap_err(),
            "Paste a gw command, such as: gw read --format=ibm.1440 disk.img"
        );
        assert_eq!(
            parse(&s, "read --format=ibm.1440 x.img").unwrap_err(),
            "A command starts with gw."
        );
        assert_eq!(
            parse(&s, "gw --foo=1 rpm").unwrap_err(),
            "gw has no option --foo."
        );
    }

    #[test]
    fn pasting_two_options_gw_holds_exclusive_is_refused() {
        let s = schema();
        assert_eq!(
            parse(&s, "gw read --hard-sectors --fake-index=300rpm x.scp").unwrap_err(),
            "--fake-index cannot be used with --hard-sectors."
        );
        assert_eq!(
            parse(&s, "gw write --dd H --gen-tg43 x.adf").unwrap_err(),
            "--densel cannot be used with --gen-tg43."
        );
        assert!(parse(&s, "gw write --dd H --fake-index=300rpm x.adf").is_ok());
    }

    #[test]
    fn positionals_after_an_empty_one_are_left_out_not_moved_up() {
        let s = schema();
        let v = values(&[("out_file", "b.img")]);
        assert_eq!(argv(s.command("convert").unwrap(), &v), ["convert"]);
        let v = values(&[("level", "H")]);
        assert_eq!(argv(s.command("pin set").unwrap(), &v), ["pin", "set"]);
    }

    #[test]
    fn a_windows_share_keeps_both_its_leading_backslashes() {
        let s = schema();
        for line in [
            r#"gw convert "\\nas\f\x.scp" y.img"#,
            r"gw convert \\nas\f\x.scp y.img",
        ] {
            let (_, v, _) = parse(&s, line).unwrap();
            assert_eq!(v.get("in_file"), r"\\nas\f\x.scp", "{line}");
        }
    }

    #[test]
    fn a_line_ending_in_a_backslash_goes_on_to_the_next() {
        let s = schema();
        for line in [
            "gw read --format=ibm.1440 \\\n  disk.img",
            "gw read --format=ibm.1440 \\\r\n  disk.img",
        ] {
            let (_, v, _) = parse(&s, line).unwrap();
            assert_eq!(v.get("file"), "disk.img", "{line:?}");
        }
    }

    #[test]
    fn plain_words_are_not_quoted() {
        assert_eq!(quote("--tracks=c=0-79:h=0"), "--tracks=c=0-79:h=0");
        assert_eq!(quote(""), if cfg!(windows) { "\"\"" } else { "''" });
    }
}

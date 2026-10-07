// Python source made smaller for the command line: build.rs packs the bridge
// with this, and tools.rs's tests check it. Included by both, so it uses no
// crates.

/// `source` with its comments and docstrings taken out, each line where it
/// was, so tracebacks name the source's lines: a string standing alone on
/// its line, as a docstring does, becomes `pass`, so that a body which held
/// only it still has a statement. Every other string is kept as it is, `#`
/// and all; an f-string alone is kept too, as it may do something.
pub fn strip(source: &str) -> String {
    let s = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    // Copied up to `from`; brackets open; nothing yet on this logical line.
    let (mut from, mut depth, mut fresh) = (0, 0usize, true);
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'#' => {
                out.push_str(source[from..i].trim_end_matches([' ', '\t']));
                while i < s.len() && s[i] != b'\n' {
                    i += 1;
                }
                from = i;
            }
            b'\n' => {
                fresh |= depth == 0;
                i += 1;
            }
            // A line continued: the next goes on with it.
            b'\\' => i += if s.get(i + 1) == Some(&b'\r') { 3 } else { 2 },
            b' ' | b'\t' | b'\r' | b'\x0c' => i += 1,
            b'(' | b'[' | b'{' => (depth, fresh, i) = (depth + 1, false, i + 1),
            b')' | b']' | b'}' => (depth, fresh, i) = (depth.saturating_sub(1), false, i + 1),
            c if c == b'\'' || c == b'"' || c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 => {
                let word = (i..s.len())
                    .find(|&j| !(s[j].is_ascii_alphanumeric() || s[j] == b'_' || s[j] >= 0x80))
                    .unwrap_or(s.len());
                let prefix = source[i..word].to_ascii_lowercase();
                let quoted = matches!(s.get(word), Some(b'\'' | b'"'))
                    && matches!(prefix.as_str(), "" | "r" | "b" | "u" | "f" | "rb" | "br" | "fr" | "rf");
                if !quoted {
                    (fresh, i) = (false, word);
                    continue;
                }
                let end = string_end(s, word);
                if fresh && depth == 0 && !prefix.contains('f') && blank_after(s, end) {
                    out.push_str(&source[from..i]);
                    out.push_str("pass");
                    out.extend(source[i..end].chars().filter(|&c| c == '\n'));
                    from = end;
                }
                (fresh, i) = (false, end);
            }
            _ => (fresh, i) = (false, i + 1),
        }
    }
    out.push_str(&source[from.min(s.len())..]);
    out
}

/// Where the string whose quotes start at `at` ends: past its closing quotes,
/// or at the end of the source.
fn string_end(s: &[u8], at: usize) -> usize {
    let quote = s[at];
    let triple = s.get(at..at + 3) == Some(&[quote; 3][..]);
    let close = if triple { 3 } else { 1 };
    let mut i = at + close;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 2,
            c if c == quote && s.get(i..i + close).is_some_and(|q| q.iter().all(|&b| b == quote)) => {
                return i + close;
            }
            _ => i += 1,
        }
    }
    s.len()
}

/// Whether nothing but spaces and a comment follow `at` on its line.
fn blank_after(s: &[u8], at: usize) -> bool {
    let rest = s[at.min(s.len())..].iter().skip_while(|&&c| c == b' ' || c == b'\t');
    matches!(rest.copied().next(), None | Some(b'\n' | b'\r' | b'#'))
}

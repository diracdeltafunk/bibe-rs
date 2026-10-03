//! Name splitting (`Text::BibTeX::Name->new`) and part formatting
//! (`Text::BibTeX::NameFormat` with `BTJ_MAYTIE`), spec §6–§7.

use crate::diag::{Diag, DiagKind};
use crate::lower_table::LOWER_RANGES;

/// The four parts of a BibTeX name. Each part is a list of tokens borrowed
/// from the input string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameParts<'a> {
    pub first: Vec<&'a str>,
    pub von: Vec<&'a str>,
    pub last: Vec<&'a str>,
    pub jr: Vec<&'a str>,
}

/// Which name part, for [`NameParts::format`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamePart {
    First,
    Von,
    Last,
    Jr,
}

/// Inputs on which btparse misbehaves (spec §6.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameAnomaly {
    /// btparse crashes (SIGSEGV/abort) on this input. The parts returned are a
    /// best-effort guess; biber would die.
    Crash,
    /// A leading comma or two commas with no token between them. btparse does
    /// not crash but its result is not fully characterised; ours is best effort.
    IrregularCommas,
}

/// Result of [`split_name`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitName<'a> {
    pub parts: NameParts<'a>,
    /// Warnings in the order btparse prints them (location-less `Bare` lines).
    pub diags: Vec<Diag>,
    pub anomaly: Option<NameAnomaly>,
}

impl<'a> NameParts<'a> {
    pub fn part(&self, p: NamePart) -> &[&'a str] {
        match p {
            NamePart::First => &self.first,
            NamePart::Von => &self.von,
            NamePart::Last => &self.last,
            NamePart::Jr => &self.jr,
        }
    }

    /// Format one part the way biber does (`NameFormat->new(p, 0)` +
    /// `set_options(part, 0, BTJ_MAYTIE, BTJ_NOTHING)`): see [`format_tokens`].
    pub fn format(&self, p: NamePart) -> String {
        format_tokens(self.part(p))
    }
}

/// Is `c` a lower-case code point in btparse's sense (spec §6.3)?
pub fn is_lower_char(c: char) -> bool {
    let c = c as u32;
    LOWER_RANGES
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                std::cmp::Ordering::Less
            } else if lo > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// A token is "lower-case" (a von particle) iff its first character is a
/// lower-case letter. Tokens starting with `{`, `\`, digits or punctuation
/// never are (spec §6.3).
pub fn is_lowercase_token(tok: &str) -> bool {
    tok.chars().next().is_some_and(is_lower_char)
}

#[inline]
fn is_name_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Split one name into first/von/last/jr (spec §6).
pub fn split_name(s: &str) -> SplitName<'_> {
    let bytes = s.as_bytes();
    let mut tokens: Vec<&str> = Vec::new();
    let mut commas: Vec<usize> = Vec::new();
    let mut brace_diags: Vec<Diag> = Vec::new();
    let mut depth: usize = 0;
    let mut start: Option<usize> = None;
    let mut irregular = false;
    let mut prev_was_comma_or_start = true; // no token since start / last comma

    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => {
                depth += 1;
                if start.is_none() {
                    start = Some(i);
                }
            }
            b'}' => {
                if depth == 0 {
                    brace_diags.push(Diag::new(
                        DiagKind::Bare,
                        0,
                        "warning: unmatched '}' (ignoring)",
                    ));
                } else {
                    depth -= 1;
                }
                if start.is_none() {
                    start = Some(i);
                }
            }
            _ if depth == 0 && (b == b',' || is_name_space(b)) => {
                if let Some(st) = start.take() {
                    tokens.push(&s[st..i]);
                    prev_was_comma_or_start = false;
                }
                if b == b',' {
                    if prev_was_comma_or_start {
                        irregular = true;
                    }
                    commas.push(tokens.len());
                    prev_was_comma_or_start = true;
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(st) = start {
        tokens.push(&s[st..]);
    }
    if depth > 0 {
        brace_diags.push(Diag::new(
            DiagKind::Bare,
            0,
            "warning: unmatched '{' (ignoring)",
        ));
    }

    // btparse scans the braces twice, so each brace warning appears twice.
    let mut diags = brace_diags.clone();
    diags.extend(brace_diags);

    let n = tokens.len();
    let mut anomaly = if irregular {
        Some(NameAnomaly::IrregularCommas)
    } else {
        None
    };

    if commas.len() > 2 {
        diags.push(Diag::new(
            DiagKind::Bare,
            0,
            "warning: too many commas in name (removing extras)",
        ));
        commas.truncate(2);
    } else if commas.last() == Some(&n) {
        diags.push(Diag::new(
            DiagKind::Bare,
            0,
            "warning: comma(s) at end of name (removing)",
        ));
        while commas.last() == Some(&n) {
            commas.pop();
        }
        if irregular && n > 0 {
            // e.g. "Doe,," / "Doe, ," : btparse segfaults
            anomaly = Some(NameAnomaly::Crash);
        }
    }

    if irregular {
        // Best-effort model of btparse's handling of leading/consecutive commas
        // (spec §6.5).
        if commas.len() == 2 {
            if commas[0] == 0 {
                commas[0] = commas[1];
            }
            if commas[0] == commas[1] {
                commas[1] = 2;
            }
            if commas[1] > n || commas[0] > n {
                anomaly = Some(NameAnomaly::Crash);
            }
        } else if commas.len() == 1 && commas[0] == 0 && n > 0 {
            anomaly = Some(NameAnomaly::Crash);
        }
    }

    let range = |a: usize, b: usize| -> Vec<&str> {
        let b = b.min(n);
        if a < b {
            tokens[a..b].to_vec()
        } else {
            Vec::new()
        }
    };

    let mut parts = NameParts::default();
    let lc = |i: usize| is_lowercase_token(tokens[i]);
    match commas.len() {
        0 => {
            if n > 0 {
                match (0..n).find(|&i| lc(i)) {
                    None => {
                        parts.first = range(0, n - 1);
                        parts.last = range(n - 1, n);
                    }
                    Some(vs) => {
                        let mut ve = vs;
                        while ve < n && lc(ve) {
                            ve += 1;
                        }
                        if ve == n {
                            ve = n - 1;
                        }
                        parts.first = range(0, vs);
                        parts.von = range(vs, ve);
                        parts.last = range(ve, n);
                    }
                }
            }
        }
        k => {
            let c1 = commas[0];
            let (jr_end, first_start) = if k == 2 {
                (commas[1], commas[1])
            } else {
                (c1, c1)
            };
            let mut ve = 0;
            if c1 > 0 && n > 0 && lc(0) {
                // The run of lower-case tokens is not bounded by the comma (spec §6.4).
                while ve < n && lc(ve) {
                    ve += 1;
                }
                if ve == c1 {
                    ve = c1 - 1;
                }
            }
            parts.von = range(0, ve);
            parts.last = range(ve, c1);
            if k == 2 {
                parts.jr = range(c1, jr_end);
            }
            parts.first = range(first_start, n);
        }
    }

    SplitName {
        parts,
        diags,
        anomaly,
    }
}

/// Length of a token for the "short token" test of `BTJ_MAYTIE` (spec §7.2):
/// code points, not counting braces, with a `{\...}` group at brace depth 0
/// counting as one character.
pub fn token_length(tok: &str) -> usize {
    let b = tok.as_bytes();
    let mut len = 0;
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' => {
                if depth == 0 && b.get(i + 1) == Some(&b'\\') {
                    // special character: skip to the matching close brace
                    len += 1;
                    let mut d = 0usize;
                    while i < b.len() {
                        match b[i] {
                            b'{' => d += 1,
                            b'}' => {
                                d -= 1;
                                if d == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                } else {
                    depth += 1;
                }
            }
            b'}' => depth = depth.saturating_sub(1),
            c => {
                // count code points: skip UTF-8 continuation bytes
                if c & 0xC0 != 0x80 {
                    len += 1;
                }
            }
        }
        i += 1;
    }
    len
}

/// Join the tokens of one part with `BTJ_MAYTIE` (spec §7.1): one token as
/// is; two tokens joined by `~`; three or more: a `~` after the first token
/// if it is shorter than 3 characters (else a space), spaces in the middle
/// and a `~` before the last token.
pub fn format_tokens<S: AsRef<str>>(tokens: &[S]) -> String {
    let n = tokens.len();
    let mut out = String::with_capacity(tokens.iter().map(|t| t.as_ref().len() + 1).sum());
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 {
            let tie = i == n - 1 || (i == 1 && token_length(tokens[0].as_ref()) < 3);
            out.push(if tie { '~' } else { ' ' });
        }
        out.push_str(t.as_ref());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(s: &str) -> String {
        let r = split_name(s);
        let p = &r.parts;
        format!(
            "F={} V={} L={} J={}",
            p.first.join("|"),
            p.von.join("|"),
            p.last.join("|"),
            p.jr.join("|")
        )
    }

    #[test]
    fn basic() {
        assert_eq!(show("John Smith"), "F=John V= L=Smith J=");
        assert_eq!(show("a b c D e F"), "F= V=a|b|c L=D|e|F J=");
        assert_eq!(show("a, b"), "F=b V=a|b L= J=");
        assert_eq!(show("a, b, c"), "F=c V=a|b|c L= J=b");
        assert_eq!(show("Doe, Jr., John"), "F=John V= L=Doe J=Jr.");
        assert_eq!(
            show("Jean {\\relax d}e Fontaine"),
            "F=Jean|{\\relax d}e V= L=Fontaine J="
        );
        assert_eq!(
            show("Jean ébauche Fontaine"),
            "F=Jean V=ébauche L=Fontaine J="
        );
    }

    #[test]
    fn ties() {
        assert_eq!(format_tokens(&["A", "B", "C", "D", "E"]), "A~B C D~E");
        assert_eq!(format_tokens(&["Abc", "B", "C"]), "Abc B~C");
        assert_eq!(format_tokens(&["{\\'E}b", "B", "C"]), "{\\'E}b~B~C");
        assert_eq!(format_tokens(&["{{\\'E}}", "B", "C"]), "{{\\'E}} B~C");
        assert_eq!(format_tokens(&["李李", "D", "E"]), "李李~D~E");
        assert_eq!(format_tokens(&["李李李", "D", "E"]), "李李李 D~E");
    }
}

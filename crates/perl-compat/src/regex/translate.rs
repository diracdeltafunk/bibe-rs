//! Translation of perl regex syntax into PCRE2 syntax.
//!
//! Most perl syntax means the same in PCRE2 (UTF + UCP mode). The
//! differences handled here:
//! * escapes perl does not know are literal letters with a warning
//!   ("Unrecognized escape \Q passed through"): in a *runtime* pattern
//!   (`qr/$var/`), `\Q`, `\E`, `\U`, `\L`, `\u`, `\l`, `\F` etc. are not
//!   quoting/case operators but match the letters `Q`, `E`, … PCRE2 treats
//!   them as operators or errors, so they become literal letters;
//! * perl-only inline flags `a`, `u`, `d`, `l`, `p` in `(?…)` groups are
//!   dropped (`u` is perl's default for these patterns anyway);
//! * duplicate group names are allowed in perl: `(?J)` is prepended when the
//!   pattern has named groups.
//!
//! Unsupported (compile error [`super::RegexError::Unsupported`]): code
//! blocks `(?{…})`/`(??{…})` (perl also refuses them at run time without
//! `use re 'eval'`), `\N{CHARACTER NAME}`, `\b{…}`/`\B{…}` boundary types.

use std::fmt::Write as _;
use std::sync::OnceLock;

use super::RegexError;
use crate::unicode::Prop;

/// Escape letters perl's regex compiler recognises (outside character
/// classes). Everything else after a backslash is a literal letter.
const PERL_ESCAPES: &str = "abBcdDefgGhHkKnNoprRsStvVwWxXzZAP";

/// Classes where PCRE2 (UCP) and perl 5.42 disagree, replaced by explicit
/// ranges generated from perl: `\w` (PCRE2 includes `²` and other `No`, perl
/// includes `Me`/`Mc` such as U+0488, U+0903), `\s` and `\h` (PCRE2 includes U+180E).
fn class_ranges(p: Prop) -> &'static str {
    static CACHE: OnceLock<[String; 3]> = OnceLock::new();
    let c = CACHE.get_or_init(|| {
        let mk = |p: Prop| {
            let mut s = String::new();
            for &(lo, hi) in p.ranges() {
                if lo == hi {
                    let _ = write!(s, "\\x{{{lo:x}}}");
                } else {
                    let _ = write!(s, "\\x{{{lo:x}}}-\\x{{{hi:x}}}");
                }
            }
            s
        };
        [mk(Prop::PerlWord), mk(Prop::PerlSpace), mk(Prop::PerlHSpace)]
    });
    match p {
        Prop::PerlWord => &c[0],
        Prop::PerlSpace => &c[1],
        _ => &c[2],
    }
}

fn substituted(n: char) -> Option<(Prop, bool)> {
    match n {
        'w' => Some((Prop::PerlWord, false)),
        'W' => Some((Prop::PerlWord, true)),
        's' => Some((Prop::PerlSpace, false)),
        'S' => Some((Prop::PerlSpace, true)),
        'h' => Some((Prop::PerlHSpace, false)),
        'H' => Some((Prop::PerlHSpace, true)),
        _ => None,
    }
}

/// A bracketed class being translated.
struct ClassBuf {
    negated: bool,
    first: bool,
    body: String,
    /// `\W`, `\S`, `\H` inside the class.
    neg_sets: Vec<Prop>,
}

impl ClassBuf {
    fn finish(self, out: &mut String) {
        if self.neg_sets.is_empty() {
            out.push('[');
            if self.negated {
                out.push('^');
            }
            out.push_str(&self.body);
            out.push(']');
        } else if !self.negated {
            // [A\W] = A | not-W
            let mut alts = Vec::new();
            if !self.body.is_empty() {
                alts.push(format!("[{}]", self.body));
            }
            for p in &self.neg_sets {
                alts.push(format!("[^{}]", class_ranges(*p)));
            }
            let _ = write!(out, "(?:{})", alts.join("|"));
        } else {
            // [^A\W] = not-A and W
            out.push_str("(?:");
            if !self.body.is_empty() {
                let _ = write!(out, "(?![{}])", self.body);
            }
            for p in &self.neg_sets {
                let _ = write!(out, "(?=[{}])", class_ranges(*p));
            }
            out.push_str("(?s:.))");
        }
    }
}

fn push_piece(out: &mut String, cls: &mut Option<ClassBuf>, piece: &str) {
    match cls.as_mut() {
        Some(cb) => cb.body.push_str(piece),
        None => out.push_str(piece),
    }
}

/// Translate a perl pattern for PCRE2.
pub fn translate(pat: &str) -> Result<String, RegexError> {
    let chars: Vec<char> = pat.chars().collect();
    let mut out = String::with_capacity(pat.len() + 8);
    let mut i = 0;
    let mut cls: Option<ClassBuf> = None;
    let mut named = false;
    while i < chars.len() {
        let c = chars[i];
        let in_class = cls.is_some();
        if c == '\\' {
            let Some(&n) = chars.get(i + 1) else {
                return Err(RegexError::Compile("Trailing \\ in regex".into()));
            };
            let mut piece = String::new();
            if let Some((p, neg)) = substituted(n) {
                match cls.as_mut() {
                    Some(cb) if neg => cb.neg_sets.push(p),
                    Some(_) => piece.push_str(class_ranges(p)),
                    None => {
                        let _ = write!(piece, "[{}{}]", if neg { "^" } else { "" }, class_ranges(p));
                    }
                }
            } else if n.is_ascii_alphabetic() {
                match n {
                    'C' => return Err(RegexError::Compile("\\C no longer supported in regex".into())),
                    'N' if chars.get(i + 2) == Some(&'{') => {
                        let rest: String = chars[i + 3..].iter().collect();
                        let close = rest.find('}').ok_or_else(|| RegexError::Compile("Missing right brace on \\N{}".into()))?;
                        let body = rest[..close].trim();
                        let hex = body.strip_prefix("U+").ok_or_else(|| {
                            RegexError::Unsupported(format!("\\N{{{body}}} (named characters)"))
                        })?;
                        let hex: String = hex.chars().filter(|&c| c != '_' && c != '.').collect();
                        if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                            return Err(RegexError::Unsupported(format!("\\N{{{body}}}")));
                        }
                        let _ = write!(piece, "\\x{{{hex}}}");
                        if let Some(cb) = cls.as_mut() {
                            cb.first = false;
                        }
                        push_piece(&mut out, &mut cls, &piece);
                        i += 3 + rest[..close].chars().count() + 1;
                        continue;
                    }
                    'b' | 'B' if !in_class && chars.get(i + 2) == Some(&'{') => {
                        return Err(RegexError::Unsupported(format!("\\{n}{{…}} boundaries")));
                    }
                    'b' | 'B' if !in_class => {
                        // word boundary with perl's \w
                        let w = class_ranges(Prop::PerlWord);
                        if n == 'b' {
                            let _ = write!(piece, "(?:(?<=[{w}])(?![{w}])|(?<![{w}])(?=[{w}]))");
                        } else {
                            let _ = write!(piece, "(?:(?<=[{w}])(?=[{w}])|(?<![{w}])(?![{w}]))");
                        }
                    }
                    _ if PERL_ESCAPES.contains(n) => {
                        piece.push('\\');
                        piece.push(n);
                    }
                    _ => piece.push(n), // unrecognized escape: literal letter
                }
            } else {
                piece.push('\\');
                piece.push(n);
            }
            if let Some(cb) = cls.as_mut() {
                cb.first = false;
            }
            push_piece(&mut out, &mut cls, &piece);
            i += 2;
            continue;
        }
        if let Some(cb) = cls.as_mut() {
            if c == ']' && !cb.first {
                if let Some(cb) = cls.take() {
                    cb.finish(&mut out);
                }
                i += 1;
                continue;
            }
            cb.first = false;
            if c == '[' && chars.get(i + 1) == Some(&':') {
                // POSIX class [:name:] — copy through
                let rest: String = chars[i..].iter().collect();
                if let Some(end) = rest.find(":]") {
                    cb.body.push_str(&rest[..end + 2]);
                    i += rest[..end + 2].chars().count();
                    continue;
                }
            }
            cb.body.push(c);
            i += 1;
            continue;
        }
        match c {
            '[' => {
                let negated = chars.get(i + 1) == Some(&'^');
                cls = Some(ClassBuf { negated, first: true, body: String::new(), neg_sets: Vec::new() });
                i += if negated { 2 } else { 1 };
                continue;
            }
            '(' if chars.get(i + 1) == Some(&'?') => {
                let next = chars.get(i + 2).copied();
                match next {
                    Some('{') => return Err(RegexError::Unsupported("(?{ code }) blocks".into())),
                    Some('?') if chars.get(i + 3) == Some(&'{') => {
                        return Err(RegexError::Unsupported("(??{ code }) blocks".into()));
                    }
                    Some('<') if !matches!(chars.get(i + 3), Some('=' | '!')) => named = true,
                    Some('P') if chars.get(i + 3) == Some(&'<') => named = true,
                    Some('\'') => named = true,
                    Some('#') => {
                        // comment: copy to ')'
                        while i < chars.len() && chars[i] != ')' {
                            out.push(chars[i]);
                            i += 1;
                        }
                        continue;
                    }
                    _ => {}
                }
                // flag group: (?^flags-flags) or (?flags:
                let mut j = i + 2;
                let mut flags = String::new();
                while let Some(&f) = chars.get(j) {
                    if f.is_ascii_alphabetic() || f == '^' || f == '-' {
                        flags.push(f);
                        j += 1;
                    } else {
                        break;
                    }
                }
                if !flags.is_empty() && matches!(chars.get(j), Some(')' | ':')) {
                    out.push_str("(?");
                    out.extend(flags.chars().filter(|f| !matches!(f, 'a' | 'u' | 'd' | 'l' | 'p')));
                    i = j;
                    continue;
                }
                out.push('(');
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    if let Some(cb) = cls {
        // unterminated class: let PCRE2 report it
        out.push('[');
        out.push_str(&cb.body);
    }
    if named {
        out.insert_str(0, "(?J)");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_basics() {
        assert_eq!(translate(r"\Q.\E").unwrap(), r"Q.E");
        assert_eq!(translate(r"\Uc").unwrap(), "Uc");
        assert_eq!(translate(r"\N{U+263A}").unwrap(), r"\x{263A}");
        assert_eq!(translate(r"(?^ui:x)").unwrap(), r"(?^i:x)");
        assert_eq!(translate(r"[\Q]").unwrap(), r"[Q]");
        assert_eq!(translate(r"[]a]").unwrap(), r"[]a]");
        assert!(translate(r"(?{ 1 })").is_err());
        assert!(translate(r"\N{LATIN SMALL LETTER A}").is_err());
        assert_eq!(translate(r"(?<n>a)").unwrap(), r"(?J)(?<n>a)");
    }
}

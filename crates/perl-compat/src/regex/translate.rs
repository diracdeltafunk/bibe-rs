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
        [
            mk(Prop::PerlWord),
            mk(Prop::PerlSpace),
            mk(Prop::PerlHSpace),
        ]
    });
    match p {
        Prop::PerlWord => &c[0],
        Prop::PerlSpace => &c[1],
        Prop::PerlDigit => "\\p{Nd}",
        _ => &c[2],
    }
}

/// ASCII bodies under perl's `/a` (`(?a)`).
fn ascii_body(p: Prop) -> &'static str {
    match p {
        Prop::PerlWord => "A-Za-z0-9_",
        Prop::PerlSpace => "\\t\\n\\x0B\\f\\r ",
        Prop::PerlDigit => "0-9",
        _ => class_ranges(p),
    }
}

fn body_for(p: Prop, ascii: bool) -> &'static str {
    if ascii {
        ascii_body(p)
    } else {
        class_ranges(p)
    }
}

/// perl-specific property names PCRE2 does not know, as class bodies.
fn perl_property(name: &str) -> Option<&'static str> {
    let key: String = name
        .chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .collect::<String>()
        .to_ascii_lowercase();
    let key = key
        .strip_prefix("is")
        .filter(|k| !k.is_empty())
        .unwrap_or(&key)
        .to_string();
    Some(match key.as_str() {
        "xposixalpha" => "\\p{Alphabetic}",
        "alnum" | "xposixalnum" => "\\p{Alphabetic}\\p{Nd}",
        "xposixdigit" => "\\p{Nd}",
        "xposixlower" => "\\p{Lowercase}",
        "xposixupper" => "\\p{Uppercase}",
        "xposixpunct" => "\\p{P}\\$+<=>^`|~",
        "space" | "spaceperl" | "xperlspace" | "xposixspace" | "perlspace" => {
            class_ranges(Prop::PerlSpace)
        }
        "word" | "xposixword" | "perlword" => class_ranges(Prop::PerlWord),
        "blank" | "xposixblank" | "horizspace" => class_ranges(Prop::PerlHSpace),
        "cntrl" | "xposixcntrl" => "\\p{Cc}",
        "xdigit" | "xposixxdigit" => {
            "0-9A-Fa-f\\x{FF10}-\\x{FF19}\\x{FF21}-\\x{FF26}\\x{FF41}-\\x{FF46}"
        }
        "posixalpha" => "A-Za-z",
        "posixalnum" => "A-Za-z0-9",
        "posixdigit" => "0-9",
        "posixlower" => "a-z",
        "posixupper" => "A-Z",
        "posixpunct" => "!-\\/:-@\\[-`{-~",
        "posixspace" => "\\t\\n\\x0B\\f\\r ",
        "posixword" => "A-Za-z0-9_",
        "posixblank" => "\\t ",
        "posixcntrl" => "\\x00-\\x1F\\x7F",
        "posixxdigit" => "0-9A-Fa-f",
        "posixgraph" => "!-~",
        "posixprint" => " -~",
        _ => return None,
    })
}

fn substituted(n: char) -> Option<(Prop, bool)> {
    match n {
        'w' => Some((Prop::PerlWord, false)),
        'W' => Some((Prop::PerlWord, true)),
        's' => Some((Prop::PerlSpace, false)),
        'S' => Some((Prop::PerlSpace, true)),
        'd' => Some((Prop::PerlDigit, false)),
        'D' => Some((Prop::PerlDigit, true)),
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
    /// Bodies of `\W`, `\S`, `\H`, … inside the class.
    neg_sets: Vec<String>,
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
            for b in &self.neg_sets {
                alts.push(format!("[^{b}]"));
            }
            let _ = write!(out, "(?:{})", alts.join("|"));
        } else {
            // [^A\W] = not-A and W
            out.push_str("(?:");
            if !self.body.is_empty() {
                let _ = write!(out, "(?![{}])", self.body);
            }
            for b in &self.neg_sets {
                let _ = write!(out, "(?=[{b}])");
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

/// perl's POSIX bracket classes (`[:name:]`), which under `/u` are the
/// XPosix properties; under `/i`, `[:upper:]` and `[:lower:]` match every
/// cased letter.
fn posix_class(name: &str, ci: bool) -> Option<&'static str> {
    Some(match name {
        "alpha" => "\\p{Alphabetic}",
        "alnum" => "\\p{Alphabetic}\\p{Nd}",
        "digit" => "\\p{Nd}",
        "upper" if ci => "\\p{Cased}",
        "lower" if ci => "\\p{Cased}",
        "upper" => "\\p{Uppercase}",
        "lower" => "\\p{Lowercase}",
        "punct" => "\\p{P}\\$+<=>^`|~",
        "space" => class_ranges(Prop::PerlSpace),
        "word" => class_ranges(Prop::PerlWord),
        "blank" => class_ranges(Prop::PerlHSpace),
        "cntrl" => "\\p{Cc}",
        "xdigit" => "0-9A-Fa-f\\x{FF10}-\\x{FF19}\\x{FF21}-\\x{FF26}\\x{FF41}-\\x{FF46}",
        _ => return None,
    })
}

/// `\p{InName}`, `\p{Block=Name}`, `\p{Blk:Name}`: a Unicode block (perl
/// 5.42 / Unicode 16 block list), as a class body.
fn block_body(name: &str) -> Option<String> {
    let key: String = name
        .chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .collect::<String>()
        .to_ascii_lowercase();
    let rest = key
        .strip_prefix("in")
        .or_else(|| {
            key.strip_prefix("block=")
                .or_else(|| key.strip_prefix("block:"))
        })
        .or_else(|| {
            key.strip_prefix("blk=")
                .or_else(|| key.strip_prefix("blk:"))
        })?;
    let t = crate::tables::props::BLOCKS;
    let i = t.binary_search_by(|e| e.0.cmp(rest)).ok()?;
    Some(format!("\\x{{{:x}}}-\\x{{{:x}}}", t[i].1, t[i].2))
}

/// Translate a perl pattern for PCRE2 (`ci`: compiled with `/i`).
pub fn translate(pat: &str, ci: bool) -> Result<String, RegexError> {
    let chars: Vec<char> = pat.chars().collect();
    let mut out = String::with_capacity(pat.len() + 8);
    let mut i = 0;
    let mut cls: Option<ClassBuf> = None;
    let mut named = false;
    let mut ascii = false;
    while i < chars.len() {
        let c = chars[i];
        let in_class = cls.is_some();
        if c == '\\' {
            let Some(&n) = chars.get(i + 1) else {
                return Err(RegexError::Compile("Trailing \\ in regex".into()));
            };
            let mut piece = String::new();
            if let Some((p, neg)) = substituted(n) {
                let body = body_for(p, ascii);
                match cls.as_mut() {
                    Some(cb) if neg => cb.neg_sets.push(body.to_string()),
                    Some(_) => piece.push_str(body),
                    None => {
                        let _ = write!(piece, "[{}{}]", if neg { "^" } else { "" }, body);
                    }
                }
            } else if n == 'p' || n == 'P' {
                // \pL, \p{Name}, \p{^Name}
                let (name, len) = if chars.get(i + 2) == Some(&'{') {
                    let rest: String = chars[i + 3..].iter().collect();
                    let close = rest.find('}').ok_or_else(|| {
                        RegexError::Compile("Missing right brace on \\p{}".into())
                    })?;
                    (
                        rest[..close].to_string(),
                        3 + rest[..close].chars().count() + 1,
                    )
                } else {
                    (
                        chars.get(i + 2).map(|c| c.to_string()).unwrap_or_default(),
                        3,
                    )
                };
                let mut neg = n == 'P';
                let mut nm = name.trim();
                if let Some(x) = nm.strip_prefix('^') {
                    neg = !neg;
                    nm = x.trim();
                }
                let block = block_body(nm);
                if let Some(body) = perl_property(nm).map(str::to_string).or(block) {
                    let body = body.as_str();
                    match cls.as_mut() {
                        Some(cb) if neg => cb.neg_sets.push(body.to_string()),
                        Some(_) => piece.push_str(body),
                        None => {
                            let _ = write!(piece, "[{}{}]", if neg { "^" } else { "" }, body);
                        }
                    }
                } else {
                    let lower = nm.to_ascii_lowercase();
                    if lower.starts_with("in") && nm.len() > 2 {
                        return Err(RegexError::Unsupported(format!(
                            "block property \\p{{{nm}}}"
                        )));
                    }
                    let nm = if lower.starts_with("is") && nm.len() > 2 {
                        &nm[2..]
                    } else {
                        nm
                    };
                    let _ = write!(piece, "\\{}{{{}}}", if neg { 'P' } else { 'p' }, nm);
                }
                if let Some(cb) = cls.as_mut() {
                    cb.first = false;
                }
                push_piece(&mut out, &mut cls, &piece);
                i += len;
                continue;
            } else if n.is_ascii_alphabetic() {
                match n {
                    'C' => {
                        return Err(RegexError::Compile(
                            "\\C no longer supported in regex".into(),
                        ));
                    }
                    'N' if chars.get(i + 2) == Some(&'{') => {
                        let rest: String = chars[i + 3..].iter().collect();
                        let close = rest.find('}').ok_or_else(|| {
                            RegexError::Compile("Missing right brace on \\N{}".into())
                        })?;
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
                        let w = body_for(Prop::PerlWord, ascii);
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
                    let name = &rest[2..end];
                    match name.strip_prefix('^') {
                        Some(n) => match posix_class(n, ci) {
                            Some(body) => cb.neg_sets.push(body.to_string()),
                            None => cb.body.push_str(&rest[..end + 2]),
                        },
                        None => match posix_class(name, ci) {
                            Some(body) => cb.body.push_str(body),
                            None => cb.body.push_str(&rest[..end + 2]),
                        },
                    }
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
                cls = Some(ClassBuf {
                    negated,
                    first: true,
                    body: String::new(),
                    neg_sets: Vec::new(),
                });
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
                    // perl /a: ASCII \d \s \w (approximation: to the end of the pattern)
                    if flags.split('-').next().is_some_and(|on| on.contains('a')) {
                        ascii = true;
                    }
                    out.push_str("(?");
                    out.extend(
                        flags
                            .chars()
                            .filter(|f| !matches!(f, 'a' | 'u' | 'd' | 'l' | 'p')),
                    );
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

    fn translate_(p: &str) -> Result<String, RegexError> {
        translate(p, false)
    }

    #[test]
    fn translate_basics() {
        assert_eq!(translate_(r"\Q.\E").unwrap(), r"Q.E");
        assert_eq!(translate_(r"\Uc").unwrap(), "Uc");
        assert_eq!(translate_(r"\N{U+263A}").unwrap(), r"\x{263A}");
        assert_eq!(translate_(r"(?^ui:x)").unwrap(), r"(?^i:x)");
        assert_eq!(translate_(r"[\Q]").unwrap(), r"[Q]");
        assert_eq!(translate_(r"[]a]").unwrap(), r"[]a]");
        assert!(translate_(r"(?{ 1 })").is_err());
        assert!(translate_(r"\N{LATIN SMALL LETTER A}").is_err());
        assert_eq!(translate_(r"(?<n>a)").unwrap(), r"(?J)(?<n>a)");
    }
}

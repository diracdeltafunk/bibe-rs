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

use super::RegexError;

/// Escape letters perl's regex compiler recognises (outside character
/// classes). Everything else after a backslash is a literal letter.
const PERL_ESCAPES: &str = "abBcdDefgGhHkKnNoprRsStvVwWxXzZAP";

/// Translate a perl pattern for PCRE2.
pub fn translate(pat: &str) -> Result<String, RegexError> {
    let chars: Vec<char> = pat.chars().collect();
    let mut out = String::with_capacity(pat.len() + 8);
    let mut i = 0;
    let mut in_class = false;
    let mut class_first = false; // just after `[` or `[^`
    let mut named = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            let Some(&n) = chars.get(i + 1) else {
                return Err(RegexError::Compile("Trailing \\ in regex".into()));
            };
            class_first = false;
            if n.is_ascii_alphabetic() {
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
                        out.push_str("\\x{");
                        out.push_str(&hex);
                        out.push('}');
                        i += 3 + rest[..close].chars().count() + 1;
                        continue;
                    }
                    'b' | 'B' if !in_class && chars.get(i + 2) == Some(&'{') => {
                        return Err(RegexError::Unsupported(format!("\\{n}{{…}} boundaries")));
                    }
                    _ if PERL_ESCAPES.contains(n) => {
                        out.push('\\');
                        out.push(n);
                    }
                    _ => out.push(n), // unrecognized escape: literal letter
                }
            } else {
                out.push('\\');
                out.push(n);
            }
            i += 2;
            continue;
        }
        if in_class {
            if c == ']' && !class_first {
                in_class = false;
            } else if c == '[' && chars.get(i + 1) == Some(&':') {
                // POSIX class [:name:] — copy through
                let rest: String = chars[i..].iter().collect();
                if let Some(end) = rest.find(":]") {
                    out.push_str(&rest[..end + 2]);
                    i += rest[..end + 2].chars().count();
                    class_first = false;
                    continue;
                }
            }
            class_first = false;
            out.push(c);
            i += 1;
            continue;
        }
        match c {
            '[' => {
                in_class = true;
                class_first = true;
                out.push('[');
                i += 1;
                if chars.get(i) == Some(&'^') {
                    out.push('^');
                    i += 1;
                }
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
        assert!(translate(r"(?{ 1 })").is_err());
        assert!(translate(r"\N{LATIN SMALL LETTER A}").is_err());
        assert_eq!(translate(r"(?<n>a)").unwrap(), r"(?J)(?<n>a)");
    }
}

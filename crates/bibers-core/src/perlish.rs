//! Minimal Perl-semantics helpers used by the configuration front end.
//!
//! TODO(perl-compat): switch to the `perl-compat` crate's helpers once its
//! API is available (truthiness, `looks_like_number`, `lc`/`fc`, numeric
//! conversion). These private versions cover what the config layer needs.

use std::cmp::Ordering;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::{IsNormalized, is_nfc_quick, is_nfd_quick};

/// Perl truthiness of a scalar: `undef`, `""` and `"0"` are false.
pub fn truthy(s: Option<&str>) -> bool {
    matches!(s, Some(v) if !v.is_empty() && v != "0")
}

/// Perl's `looks_like_number` (as implemented by `grok_number`): optional
/// leading whitespace, optional sign, decimal digits with an optional
/// fraction, or a fraction alone, an optional exponent, optional trailing
/// whitespace; or `Inf`/`Infinity`/`NaN` in any case.
pub fn looks_like_number(s: &str) -> bool {
    let t = s.trim_start_matches(is_perl_space);
    let t = t.trim_end_matches(is_perl_space);
    let t = t.strip_prefix(['+', '-']).unwrap_or(t);
    if t.is_empty() {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if lower == "inf" || lower == "infinity" || lower == "nan" {
        return true;
    }
    let b = t.as_bytes();
    let mut i = 0;
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - fs;
    }
    if int_digits == 0 && frac_digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let es = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == es {
            return false;
        }
        i = j;
    }
    i == b.len()
}

fn is_perl_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

/// Perl's numification of a string (the value `0 + $s` would give): the
/// longest numeric prefix after leading whitespace, `0` if there is none.
pub fn numify(s: &str) -> f64 {
    let t = s.trim_start_matches(is_perl_space);
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let rest = &t[i..];
    let lower: String = rest
        .chars()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    if lower.starts_with("inf") || lower.starts_with("nan") {
        let neg = t.starts_with('-');
        if lower.starts_with("nan") {
            return f64::NAN;
        }
        return if neg {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let ds = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - ds;
    if i < b.len() && b[i] == b'.' {
        let save = i;
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if digits == 0 && i == fs {
            i = save;
        }
        digits += i - fs;
    }
    if digits == 0 {
        return 0.0;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let es = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > es {
            i = j;
        }
    }
    t[..i].parse::<f64>().unwrap_or(0.0)
}

/// Perl's `<=>` on two (possibly undef) strings, as used by
/// `sort {$a->{order} <=> $b->{order}}`. `NaN` compares as equal.
pub fn num_cmp(a: Option<&str>, b: Option<&str>) -> Ordering {
    let x = a.map(numify).unwrap_or(0.0);
    let y = b.map(numify).unwrap_or(0.0);
    x.partial_cmp(&y).unwrap_or(Ordering::Equal)
}

/// Perl's `lc`.
pub fn lc(s: &str) -> String {
    if s.is_ascii() {
        s.to_ascii_lowercase()
    } else {
        // Perl's lc has no final-sigma rule; Rust's to_lowercase does.
        s.chars().flat_map(char::to_lowercase).collect()
    }
}

/// Perl's `fc` (full case folding).
///
/// TODO(perl-compat): use the full Unicode CaseFolding table; this treats
/// non-ASCII text with simple lowercasing, which is exact for the ASCII names
/// data models use.
pub fn fc(s: &str) -> String {
    if s.is_ascii() {
        s.to_ascii_lowercase()
    } else {
        s.chars()
            .flat_map(|c| match c {
                'ß' => "ss".chars().collect::<Vec<_>>(),
                'ς' => vec!['σ'],
                c => c.to_lowercase().collect(),
            })
            .collect()
    }
}

/// Unicode NFD (Unicode 16.0.0 tables), with an ASCII fast path.
pub fn nfd(s: &str) -> String {
    if s.is_ascii() || is_nfd_quick(s.chars()) == IsNormalized::Yes {
        s.to_string()
    } else {
        s.nfd().collect()
    }
}

/// Unicode NFC (Unicode 16.0.0 tables), with an ASCII fast path.
pub fn nfc(s: &str) -> String {
    if s.is_ascii() || is_nfc_quick(s.chars()) == IsNormalized::Yes {
        s.to_string()
    } else {
        s.nfc().collect()
    }
}

/// Split on Perl's `/\s*,\s*/`, dropping trailing empty fields (Perl `split`
/// semantics with no limit).
pub fn split_comma(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ',' {
            // remove trailing whitespace already pushed
            while cur.ends_with(|c: char| c.is_whitespace()) {
                cur.pop();
            }
            out.push(std::mem::take(&mut cur));
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            continue;
        }
        cur.push(chars[i]);
        i += 1;
    }
    out.push(cur);
    // A leading empty field is kept by Perl's split unless the match is
    // zero-width; trailing empty fields are removed.
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// Perl's stable `sort { $a->{order} <=> $b->{order} }` over items with an
/// `order` key.
pub fn sort_by_order<T>(items: &mut [T], order: impl Fn(&T) -> Option<&str>) {
    items.sort_by(|a, b| num_cmp(order(a), order(b)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        for s in [
            "1", " 1", "1 ", "-1.5", ".5", "5.", "1e3", "1E-3", "Inf", "nan", "+3",
        ] {
            assert!(looks_like_number(s), "{s}");
        }
        for s in ["", " ", "a", "1a", "0x10", "1e", ".", "--1", "true"] {
            assert!(!looks_like_number(s), "{s}");
        }
        assert_eq!(numify("10abc"), 10.0);
        assert_eq!(numify("  -2.5e1x"), -25.0);
        assert_eq!(numify("abc"), 0.0);
        assert_eq!(numify(".5"), 0.5);
        assert_eq!(num_cmp(Some("10"), Some("9")), Ordering::Greater);
    }

    #[test]
    fn split() {
        assert_eq!(split_comma("a, b ,c"), vec!["a", "b", "c"]);
        assert_eq!(split_comma("a,,b,,"), vec!["a", "", "b"]);
        assert_eq!(split_comma(""), Vec::<String>::new());
    }
}

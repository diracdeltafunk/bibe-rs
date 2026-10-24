//! Perl-semantics helpers used by the configuration front end.
//!
//! Thin wrappers over the `perl-compat` crate (verified exhaustively against
//! perl 5.42), kept so call sites read like the Perl they port. Helpers that
//! encode biber-specific logic (`split_comma`, `sort_by_order`) live here.

use std::cmp::Ordering;

/// Perl truthiness of a scalar: `undef`, `""` and `"0"` are false.
pub fn truthy(s: Option<&str>) -> bool {
    perl_compat::truth::perl_truthy_opt(s)
}

/// Perl's `looks_like_number` (Scalar::Util, perl 5.42 `grok_number`).
pub fn looks_like_number(s: &str) -> bool {
    perl_compat::truth::looks_like_number(s)
}

/// Perl's numification of a string (the value `0 + $s` would give).
pub fn numify(s: &str) -> f64 {
    perl_compat::truth::numify(s)
}

/// Perl's `<=>` on two (possibly undef) strings, as used by
/// `sort {$a->{order} <=> $b->{order}}`. `NaN` compares as equal.
pub fn num_cmp(a: Option<&str>, b: Option<&str>) -> Ordering {
    perl_compat::truth::num_cmp_opt(a, b)
}

/// Perl's `lc` (full case mapping, no final-sigma rule).
pub fn lc(s: &str) -> String {
    perl_compat::case::lc(s)
}

/// Perl's `fc` (full case folding).
pub fn fc(s: &str) -> String {
    perl_compat::case::fc(s)
}

/// Unicode NFD (Unicode 16.0.0, as perl 5.42).
pub fn nfd(s: &str) -> String {
    perl_compat::unicode::nfd(s).into_owned()
}

/// Unicode NFC (Unicode 16.0.0, as perl 5.42).
pub fn nfc(s: &str) -> String {
    perl_compat::unicode::nfc(s).into_owned()
}

/// [`nfd`] of an owned string (no copy when it is already NFD).
pub fn nfd_string(s: String) -> String {
    perl_compat::unicode::nfd_string(s)
}

/// [`nfc`] of an owned string (no copy when it is already NFC).
pub fn nfc_string(s: String) -> String {
    perl_compat::unicode::nfc_string(s)
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

/// Perl's `$!` text for an I/O error (Rust's `Display` appends
/// ` (os error N)`; both use the C library's `strerror` text).
pub fn errno_text(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
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

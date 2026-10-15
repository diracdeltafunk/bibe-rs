//! String normalisation used for sorting (`Biber::Utils`):
//! `strip_nosort`, `normalise_string_sort`, `normalise_string`, and
//! `_process_sort_attributes` (Internals.pm:1547).

use crate::bcf::templates::SortItemAttrs;
use crate::config::Config;
use crate::perlish::{fc, lc, numify, truthy};
use perl_compat::grapheme::gcstring;
use perl_compat::regex::PerlRegex;
use perl_compat::unicode::{Prop, is_perl_space};

/// `s/([^\\])~/$1 /g`: ties preceded by anything but a backslash become
/// spaces (matches don't overlap: `a~~` → `a ~`).
pub fn replace_ties(s: &str) -> String {
    if !s.contains('~') {
        return s.to_string();
    }
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        if cs[i] != '\\' && cs.get(i + 1) == Some(&'~') {
            out.push(cs[i]);
            out.push(' ');
            i += 2;
        } else {
            out.push(cs[i]);
            i += 1;
        }
    }
    out
}

/// `s/\\[A-Za-z]+//g`: remove LaTeX macros (ASCII letters only).
pub fn strip_macros(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
            out.push_str(&s[last..i]);
            i += 1;
            while b.get(i).is_some_and(u8::is_ascii_alphabetic) {
                i += 1;
            }
            last = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[last..]);
    out
}

/// `s/^\s+|\s+$//g; s/\s+/ /g` (perl `\s`, Unicode).
pub fn trim_collapse(s: &str) -> String {
    let t = s.trim_matches(is_perl_space);
    let mut out = String::with_capacity(t.len());
    let mut in_ws = false;
    for c in t.chars() {
        if is_perl_space(c) {
            if !in_ws {
                out.push(' ');
            }
            in_ws = true;
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

/// `Biber::Utils::strip_nosort` (Utils.pm:559): apply the `nosort` regexes
/// for `field` (by field name or datafield set).
pub fn strip_nosort(cfg: &Config, s: &str, field: &str) -> String {
    strip_nosort_with(cfg, s, &nosort_patterns(cfg, field))
}

/// The positions of the `nosort` patterns that apply to `field` (by field
/// name or datafield set), for [`strip_nosort_with`].
pub fn nosort_patterns(cfg: &Config, field: &str) -> Vec<usize> {
    let pats = cfg.biber.patterns("nosort");
    if pats.is_empty() {
        return Vec::new();
    }
    let ffield = fc(field);
    let mut out = Vec::new();
    for (i, p) in pats.iter().enumerate() {
        let name = p.name.as_deref().unwrap_or("");
        let applies = fc(name) == ffield
            || cfg
                .datafield_set(&lc(name))
                .is_some_and(|set| set.iter().any(|f| fc(f) == ffield));
        if applies {
            out.push(i);
        }
    }
    out
}

/// [`strip_nosort`] with the patterns that apply (from [`nosort_patterns`]).
pub fn strip_nosort_with(cfg: &Config, s: &str, applicable: &[usize]) -> String {
    if !truthy(Some(s)) {
        return String::new();
    }
    let pats = cfg.biber.patterns("nosort");
    let mut out = s.to_string();
    for &i in applicable {
        let p = &pats[i];
        // `qr/$re/` compiled once (Biber.pm:4433); an invalid pattern would
        // have died there.
        if let Ok(re) = PerlRegex::cached(p.value.as_deref().unwrap_or(""), false)
            && let Ok(r) = re.remove_all(&out)
        {
            out = r;
        }
    }
    out
}

/// `Biber::Utils::normalise_string_sort` (Utils.pm:659).
pub fn normalise_string_sort(cfg: &Config, s: &str, field: &str) -> String {
    normalise_string_sort_with(cfg, s, &nosort_patterns(cfg, field))
}

/// [`normalise_string_sort`] with the `nosort` patterns that apply (from
/// [`nosort_patterns`]).
pub fn normalise_string_sort_with(cfg: &Config, s: &str, nosort: &[usize]) -> String {
    if !truthy(Some(s)) {
        return String::new();
    }
    let s = strip_nosort_with(cfg, s, nosort);
    let s = replace_ties(&s);
    let s = strip_macros(&s);
    let s: String = s.chars().filter(|c| *c != '{' && *c != '}').collect();
    trim_collapse(&s)
}

/// `Biber::Utils::normalise_string` (Utils.pm:698) with
/// `normalise_string_common`: also removes `\p{P}`, `\p{S}`, `\p{C}`.
pub fn normalise_string(s: &str) -> String {
    if !truthy(Some(s)) {
        return String::new();
    }
    let s = replace_ties(s);
    let s = strip_macros(&s);
    let s: String = s
        .chars()
        .filter(|&c| !(Prop::P.contains(c) || Prop::S.contains(c) || Prop::C.contains(c)))
        .collect();
    trim_collapse(&s)
}

/// Perl numification truncated to an integer (as XS `IV` arguments are).
fn iv(s: &str) -> i64 {
    let f = numify(s);
    if f.is_nan() { 0 } else { f.trunc() as i64 }
}

/// `_process_sort_attributes` (Internals.pm:1547): `"0"` becomes the
/// `BIBERZERO` sentinel (no padding, BB-057); then substring, then padding,
/// both counted in sombok grapheme clusters.
pub fn process_sort_attributes(s: &str, a: &SortItemAttrs) -> String {
    if s == "0" {
        return "BIBERZERO".into();
    }
    if s.is_empty() {
        return String::new();
    }
    let t = |v: &Option<String>| truthy(v.as_deref());
    let mut out = s.to_string();
    if t(&a.substring_width) || t(&a.substring_side) {
        let width = if t(&a.substring_width) {
            iv(a.substring_width.as_deref().unwrap_or(""))
        } else {
            4
        };
        let side = if t(&a.substring_side) {
            a.substring_side.as_deref().unwrap_or("")
        } else {
            "left"
        };
        let offset = if side == "right" { -width } else { 0 };
        out = gcstring::substr(&out, offset, Some(width)).to_string();
    }
    if t(&a.pad_side) || t(&a.pad_width) || t(&a.pad_char) {
        let width = if t(&a.pad_width) {
            numify(a.pad_width.as_deref().unwrap_or(""))
        } else {
            4.0
        };
        let side = if t(&a.pad_side) {
            a.pad_side.as_deref().unwrap_or("")
        } else {
            "left"
        };
        let ch = if t(&a.pad_char) {
            a.pad_char.as_deref().unwrap_or("")
        } else {
            "0"
        };
        let len = width - gcstring::length(&out) as f64;
        if len > 0.0 {
            let pad = ch.repeat(len as usize);
            if side == "left" {
                out = pad + &out;
            } else if side == "right" {
                out.push_str(&pad);
            }
        }
    }
    out
}

/// `NFD(sprintf("%-*s", $width, NFC($s)))`: pad with spaces (counting code
/// points of the NFC form), as `_namestring` pads name parts.
pub fn pad_namepart(s: &str, width: f64) -> String {
    let n = perl_compat::unicode::nfc(s);
    let len = n.chars().count() as f64;
    let mut out = n.into_owned();
    if width > len {
        for _ in 0..((width - len) as usize) {
            out.push(' ');
        }
    }
    perl_compat::unicode::nfd(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ties_and_macros() {
        assert_eq!(replace_ties("a~~b"), "a ~b");
        assert_eq!(replace_ties("\\~a~b"), "\\~a b");
        assert_eq!(strip_macros("x\\emph{y}\\ z"), "x{y}\\ z");
        assert_eq!(trim_collapse("  a \t b  "), "a b");
        assert_eq!(normalise_string("$100, {T}he"), "100 The");
    }

    #[test]
    fn attributes() {
        let mut a = SortItemAttrs::default();
        assert_eq!(process_sort_attributes("0", &a), "BIBERZERO");
        a.pad_width = Some("6".into());
        assert_eq!(process_sort_attributes("12", &a), "000012");
        a.pad_side = Some("right".into());
        a.pad_char = Some("x".into());
        assert_eq!(process_sort_attributes("12", &a), "12xxxx");
        let b = SortItemAttrs {
            substring_side: Some("right".into()),
            ..SortItemAttrs::default()
        };
        assert_eq!(process_sort_attributes("abcdef", &b), "cdef");
        assert_eq!(pad_namepart("A\u{30A}b", 4.0), "A\u{30A}b  ");
    }
}

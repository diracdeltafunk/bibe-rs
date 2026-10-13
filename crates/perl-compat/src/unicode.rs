//! Unicode 16.0.0 behaviour as seen by biber's perl 5.42:
//! normalisation (`Unicode::Normalize` 1.32), `Unicode::UCD::num` (0.81) and
//! the regex character classes biber's own code uses.
//!
//! # Normalisation
//!
//! [`nfd`], [`nfc`], [`nfkd`], [`nfkc`] wrap `unicode-normalization`, pinned
//! to 0.1.24, the last release implementing Unicode 16.0.0 (0.1.25 moved to
//! Unicode 17). A test checks every code point against a digest produced by
//! `Unicode::Normalize` 1.32 in the oracle. biber keeps strings in NFD
//! internally (`NFD()` on input, `bibtex.pm`/`biblatexml.pm`) and emits NFC
//! (`Output/*.pm`), and uses `NFKD` to turn Unicode roman numerals into ASCII
//! (`Utils.pm:1647` `rangelen`, `Internals.pm:1346` integer sorting).
//!
//! # `Unicode::UCD::num`
//!
//! [`ucd_num`] is an exact port: a single character returns its Unicode
//! `Numeric_Value` (which may be fractional or negative, e.g. `½` → 0.5,
//! `༳` → -0.5); a longer string must consist entirely of decimal digits
//! (`\d`) from one block of ten consecutive code points (so mixed scripts,
//! or ASCII mixed with full-width digits, return `None`).
//! biber call sites: `Constants.pm:125/165` (datamodel `integer`/`datepart`
//! validation), `bibtex.pm:1057/1063` (legacy year/month warnings),
//! `Internals.pm:1352` (integer sort values), `Date/Format.pm:136`
//! (non-ASCII digit dates).
//!
//! # Character classes
//!
//! [`Prop`] enumerates the perl regex classes biber's code uses (`\s`, `\d`,
//! `\w`, `\h`, `\v`, `\p{L}`, `\p{Lu}`, `\p{Ll}`, `\p{M}`, `\p{Nd}`, `\p{Nl}`,
//! `\p{P}` and its subcategories, `\p{S}`, `\p{C}`, `\p{Dash}`, `\p{Dia}`, …)
//! with tables generated from perl 5.42 itself, so they are exactly perl's
//! Unicode 16 sets (Rust's `char::is_whitespace` etc. differ: e.g. perl's
//! `\s` excludes U+200B and U+FEFF but includes U+0085).

use std::borrow::Cow;

use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick, is_nfd_quick};

use crate::tables::props;
use crate::truth::PerlNum;

/// The Unicode version of every table in this crate (except the sombok
/// grapheme rules in [`crate::grapheme::gcstring`], which are Unicode 8.0.0).
pub const UNICODE_VERSION: (u8, u8, u8) = (16, 0, 0);

const _: () = {
    assert!(unicode_normalization::UNICODE_VERSION.0 == 16);
};

/// Is `s` already in NFD? Exact: ASCII is NFD, and NFD's quick check has
/// no `Maybe` answers (every `NFD_QC` value is `Yes` or `No`), so a `Yes`
/// is authoritative and anything else is normalised.
#[inline]
fn is_nfd_fast(s: &str) -> bool {
    s.is_ascii() || is_nfd_quick(s.chars()) == IsNormalized::Yes
}

/// Is `s` certainly already in NFC? Exact for `true`: ASCII is NFC, and a
/// quick-check `Yes` is authoritative. `Maybe` (combining marks that might
/// compose with what precedes them) counts as `false`: the caller then
/// normalises, which gives the same string when it was NFC after all.
#[inline]
fn is_nfc_fast(s: &str) -> bool {
    s.is_ascii() || is_nfc_quick(s.chars()) == IsNormalized::Yes
}

/// `Unicode::Normalize::NFD`.
pub fn nfd(s: &str) -> Cow<'_, str> {
    if is_nfd_fast(s) {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(s.nfd().collect())
    }
}

/// `Unicode::Normalize::NFC`.
pub fn nfc(s: &str) -> Cow<'_, str> {
    if is_nfc_fast(s) {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(s.nfc().collect())
    }
}

/// [`nfd`] of an owned string, reusing it when it is already NFD.
pub fn nfd_string(s: String) -> String {
    if is_nfd_fast(&s) {
        s
    } else {
        s.nfd().collect()
    }
}

/// [`nfc`] of an owned string, reusing it when it is already NFC.
pub fn nfc_string(s: String) -> String {
    if is_nfc_fast(&s) {
        s
    } else {
        s.nfc().collect()
    }
}

/// `Unicode::Normalize::NFKD`.
pub fn nfkd(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(s.nfkd().collect())
    }
}

/// `Unicode::Normalize::NFKC`.
pub fn nfkc(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(s.nfkc().collect())
    }
}

// ---------------------------------------------------------------------------
// Unicode::UCD::num

/// A `Numeric_Value` from the UCD: integer, or a rational stored as
/// numerator/denominator (perl computes it as a double division).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumVal {
    /// Integral value.
    Int(i64),
    /// Rational value `n/d`.
    Frac(i64, i64),
}

impl NumVal {
    /// The value as perl holds it.
    pub fn to_perl(self) -> PerlNum {
        match self {
            NumVal::Int(i) => PerlNum::Int(i),
            NumVal::Frac(n, d) => PerlNum::Float(n as f64 / d as f64),
        }
    }
}

/// `Numeric_Value` of a single code point, if it has one.
pub fn numeric_value(c: char) -> Option<NumVal> {
    let t = crate::tables::num::NUMERIC;
    t.binary_search_by_key(&(c as u32), |e| e.0)
        .ok()
        .map(|i| t[i].1)
}

/// `Unicode::UCD::num($s)`; `None` is perl's `undef`.
///
/// ```
/// use perl_compat::unicode::ucd_num;
/// assert_eq!(ucd_num("0012").map(|n| n.to_string()), Some("12".into()));
/// assert_eq!(ucd_num("١٢").map(|n| n.to_string()), Some("12".into()));
/// assert_eq!(ucd_num("½").map(|n| n.to_string()), Some("0.5".into()));
/// assert!(ucd_num("1١").is_none() && ucd_num("").is_none() && ucd_num("-1").is_none());
/// ```
pub fn ucd_num(s: &str) -> Option<PerlNum> {
    let mut chars = s.chars();
    let first = chars.next()?;
    let v = numeric_value(first)?;
    if chars.as_str().is_empty() {
        return Some(v.to_perl());
    }
    // More than one char: must start with a decimal digit (`/^\D/` fails)
    if !Prop::PerlDigit.contains(first) {
        return None;
    }
    let NumVal::Int(first_val) = v else {
        return None;
    };
    let zero = first as i64 - first_val;
    let mut value = PerlNum::Int(first_val);
    for c in chars {
        let digit = c as i64 - zero;
        if !(0..=9).contains(&digit) {
            return None;
        }
        value = match value {
            PerlNum::Int(x) => match x.checked_mul(10).and_then(|y| y.checked_add(digit)) {
                Some(y) => PerlNum::Int(y),
                None => {
                    // perl: IV overflow → UV, then NV
                    let u = (x as u64)
                        .checked_mul(10)
                        .and_then(|y| y.checked_add(digit as u64));
                    match u {
                        Some(u) => PerlNum::UInt(u),
                        None => PerlNum::Float(x as f64 * 10.0 + digit as f64),
                    }
                }
            },
            PerlNum::UInt(x) => match x.checked_mul(10).and_then(|y| y.checked_add(digit as u64)) {
                Some(u) => PerlNum::UInt(u),
                None => PerlNum::Float(x as f64 * 10.0 + digit as f64),
            },
            PerlNum::Float(f) => PerlNum::Float(f * 10.0 + digit as f64),
        };
    }
    Some(value)
}

// ---------------------------------------------------------------------------
// properties

/// A perl regex character class / Unicode property, Unicode 16.0.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Prop {
    /// `\s` (perl ≥ 5.18: `\p{XPerlSpace}`, = White_Space)
    PerlSpace,
    /// `\d` (`\p{Nd}`)
    PerlDigit,
    /// `\w` (`\p{XPosixWord}`: Alphabetic, M, Nd, Pc, Join_Control)
    PerlWord,
    /// `\h` (horizontal whitespace)
    PerlHSpace,
    /// `\v` (vertical whitespace)
    PerlVSpace,
    /// `\p{L}`
    L,
    /// `\p{Lu}`
    Lu,
    /// `\p{Ll}`
    Ll,
    /// `\p{Lt}`
    Lt,
    /// `\p{Lm}`
    Lm,
    /// `\p{Lo}`
    Lo,
    /// `\p{M}`
    M,
    /// `\p{Mn}`
    Mn,
    /// `\p{Mc}`
    Mc,
    /// `\p{Me}`
    Me,
    /// `\p{N}`
    N,
    /// `\p{Nd}`
    Nd,
    /// `\p{Nl}`
    Nl,
    /// `\p{No}`
    No,
    /// `\p{P}`
    P,
    /// `\p{Pc}`
    Pc,
    /// `\p{Pd}`
    Pd,
    /// `\p{Ps}`
    Ps,
    /// `\p{Pe}`
    Pe,
    /// `\p{Pi}`
    Pi,
    /// `\p{Pf}`
    Pf,
    /// `\p{Po}`
    Po,
    /// `\p{S}`
    S,
    /// `\p{Sm}`
    Sm,
    /// `\p{Sc}`
    Sc,
    /// `\p{Sk}`
    Sk,
    /// `\p{So}`
    So,
    /// `\p{Z}`
    Z,
    /// `\p{Zs}`
    Zs,
    /// `\p{C}` (includes unassigned code points)
    C,
    /// `\p{Cc}`
    Cc,
    /// `\p{Cf}`
    Cf,
    /// `\p{Cn}` (unassigned)
    Cn,
    /// `\p{Dash}`
    Dash,
    /// `\p{Dia}` (Diacritic)
    Dia,
    /// `\p{Alpha}` (Alphabetic)
    Alpha,
    /// `\p{Upper}` (Uppercase)
    Upper,
    /// `\p{Lower}` (Lowercase)
    Lower,
    /// `\p{ASCII_Hex_Digit}`
    AsciiHexDigit,
}

impl Prop {
    /// Every property, for iteration in tests.
    pub const ALL: &'static [Prop] = &[
        Prop::PerlSpace,
        Prop::PerlDigit,
        Prop::PerlWord,
        Prop::PerlHSpace,
        Prop::PerlVSpace,
        Prop::L,
        Prop::Lu,
        Prop::Ll,
        Prop::Lt,
        Prop::Lm,
        Prop::Lo,
        Prop::M,
        Prop::Mn,
        Prop::Mc,
        Prop::Me,
        Prop::N,
        Prop::Nd,
        Prop::Nl,
        Prop::No,
        Prop::P,
        Prop::Pc,
        Prop::Pd,
        Prop::Ps,
        Prop::Pe,
        Prop::Pi,
        Prop::Pf,
        Prop::Po,
        Prop::S,
        Prop::Sm,
        Prop::Sc,
        Prop::Sk,
        Prop::So,
        Prop::Z,
        Prop::Zs,
        Prop::C,
        Prop::Cc,
        Prop::Cf,
        Prop::Cn,
        Prop::Dash,
        Prop::Dia,
        Prop::Alpha,
        Prop::Upper,
        Prop::Lower,
        Prop::AsciiHexDigit,
    ];

    /// The perl regex syntax for this class (e.g. `\p{Dash}`).
    pub fn perl_syntax(self) -> &'static str {
        match self {
            Prop::PerlSpace => r"\s",
            Prop::PerlDigit => r"\d",
            Prop::PerlWord => r"\w",
            Prop::PerlHSpace => r"\h",
            Prop::PerlVSpace => r"\v",
            Prop::L => r"\p{L}",
            Prop::Lu => r"\p{Lu}",
            Prop::Ll => r"\p{Ll}",
            Prop::Lt => r"\p{Lt}",
            Prop::Lm => r"\p{Lm}",
            Prop::Lo => r"\p{Lo}",
            Prop::M => r"\p{M}",
            Prop::Mn => r"\p{Mn}",
            Prop::Mc => r"\p{Mc}",
            Prop::Me => r"\p{Me}",
            Prop::N => r"\p{N}",
            Prop::Nd => r"\p{Nd}",
            Prop::Nl => r"\p{Nl}",
            Prop::No => r"\p{No}",
            Prop::P => r"\p{P}",
            Prop::Pc => r"\p{Pc}",
            Prop::Pd => r"\p{Pd}",
            Prop::Ps => r"\p{Ps}",
            Prop::Pe => r"\p{Pe}",
            Prop::Pi => r"\p{Pi}",
            Prop::Pf => r"\p{Pf}",
            Prop::Po => r"\p{Po}",
            Prop::S => r"\p{S}",
            Prop::Sm => r"\p{Sm}",
            Prop::Sc => r"\p{Sc}",
            Prop::Sk => r"\p{Sk}",
            Prop::So => r"\p{So}",
            Prop::Z => r"\p{Z}",
            Prop::Zs => r"\p{Zs}",
            Prop::C => r"\p{C}",
            Prop::Cc => r"\p{Cc}",
            Prop::Cf => r"\p{Cf}",
            Prop::Cn => r"\p{Cn}",
            Prop::Dash => r"\p{Dash}",
            Prop::Dia => r"\p{Dia}",
            Prop::Alpha => r"\p{Alpha}",
            Prop::Upper => r"\p{Upper}",
            Prop::Lower => r"\p{Lower}",
            Prop::AsciiHexDigit => r"\p{ASCII_Hex_Digit}",
        }
    }

    /// The code point ranges (inclusive) of this class.
    pub fn ranges(self) -> &'static [(u32, u32)] {
        match self {
            Prop::PerlSpace => props::PERL_SPACE,
            Prop::PerlDigit => props::PERL_DIGIT,
            Prop::PerlWord => props::PERL_WORD,
            Prop::PerlHSpace => props::PERL_HSPACE,
            Prop::PerlVSpace => props::PERL_VSPACE,
            Prop::L => props::L,
            Prop::Lu => props::LU,
            Prop::Ll => props::LL,
            Prop::Lt => props::LT,
            Prop::Lm => props::LM,
            Prop::Lo => props::LO,
            Prop::M => props::M,
            Prop::Mn => props::MN,
            Prop::Mc => props::MC,
            Prop::Me => props::ME,
            Prop::N => props::N,
            Prop::Nd => props::ND,
            Prop::Nl => props::NL,
            Prop::No => props::NO,
            Prop::P => props::P,
            Prop::Pc => props::PC,
            Prop::Pd => props::PD,
            Prop::Ps => props::PS,
            Prop::Pe => props::PE,
            Prop::Pi => props::PI,
            Prop::Pf => props::PF,
            Prop::Po => props::PO,
            Prop::S => props::S,
            Prop::Sm => props::SM,
            Prop::Sc => props::SC,
            Prop::Sk => props::SK,
            Prop::So => props::SO,
            Prop::Z => props::Z,
            Prop::Zs => props::ZS,
            Prop::C => props::C,
            Prop::Cc => props::CC,
            Prop::Cf => props::CF,
            Prop::Cn => props::CN,
            Prop::Dash => props::DASH,
            Prop::Dia => props::DIA,
            Prop::Alpha => props::ALPHA,
            Prop::Upper => props::UPPER,
            Prop::Lower => props::LOWER,
            Prop::AsciiHexDigit => props::ASCII_HEX_DIGIT,
        }
    }

    /// Does `c` match this class?
    #[inline]
    pub fn contains(self, c: char) -> bool {
        in_ranges(self.ranges(), c as u32)
    }
}

/// Binary search in a sorted table of inclusive ranges.
#[inline]
pub(crate) fn in_ranges(t: &[(u32, u32)], cp: u32) -> bool {
    t.binary_search_by(|&(lo, hi)| {
        if hi < cp {
            std::cmp::Ordering::Less
        } else if lo > cp {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

/// perl `\s`.
#[inline]
pub fn is_perl_space(c: char) -> bool {
    if c.is_ascii() {
        return matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0B' | '\x0C');
    }
    Prop::PerlSpace.contains(c)
}

/// perl `\d`.
#[inline]
pub fn is_perl_digit(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_digit();
    }
    Prop::PerlDigit.contains(c)
}

/// perl `\w`.
#[inline]
pub fn is_perl_word(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '_';
    }
    Prop::PerlWord.contains(c)
}

/// Strip leading and trailing perl `\s` (`s/^\s+|\s+$//g`).
pub fn trim_perl_space(s: &str) -> &str {
    s.trim_matches(is_perl_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props() {
        assert!(is_perl_space('\u{85}'));
        assert!(!is_perl_space('\u{200B}'));
        assert!(Prop::Dash.contains('-'));
        assert!(Prop::Pd.contains('\u{2013}'));
        assert!(Prop::L.contains('é'));
        assert!(!Prop::L.contains('1'));
        assert!(is_perl_word('\u{200C}'));
    }

    #[test]
    fn num() {
        assert_eq!(ucd_num("༳").map(|n| n.to_string()), Some("-0.5".into()));
        assert_eq!(ucd_num("Ⅻ").map(|n| n.to_string()), Some("12".into()));
        assert!(ucd_num("ⅫⅫ").is_none());
        assert!(ucd_num("１2").is_none());
        assert_eq!(
            ucd_num("２０２４").map(|n| n.to_string()),
            Some("2024".into())
        );
    }

    #[test]
    fn norm() {
        assert_eq!(nfd("é"), "e\u{301}");
        assert_eq!(nfc("e\u{301}"), "é");
        assert_eq!(nfkd("Ⅻ"), "XII");
    }
}

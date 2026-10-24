//! Perl truthiness, `Scalar::Util::looks_like_number`, and Perl's
//! string → number conversion and number → string formatting.
//!
//! # Truthiness
//!
//! In Perl a scalar is false if it is `undef`, `""`, `"0"` or numerically `0`
//! (as a number). Strings like `"0.0"`, `"00"`, `" 0"` and `"0E0"` are *true*.
//! biber relies on this all over the place (`if (my $x = …)`, `$a || $b`,
//! `return '' unless $str`); that is the BB-055…059 family of bugs where a
//! field value of `0` is treated as missing. Use [`perl_truthy`] at every such
//! call site so the quirk stays visible.
//!
//! # Numbers
//!
//! [`looks_like_number`] is an exact port of perl 5.42's `grok_number` as used
//! by `Scalar::Util::looks_like_number` 1.70 (via `Perl_looks_like_number`):
//! leading ASCII whitespace, optional sign, digits with optional `.` fraction
//! (or `.` followed by digits), optional exponent, trailing ASCII whitespace,
//! the special string `"0 but true"`, and the `Inf`/`Infinity`/`NaN`
//! spellings (`nanq`, `nans`, `qnan`, `snan`, `nan(123)`, `1.#INF`, `1.#IND`).
//! Hex (`0x1A`), binary, underscores and non-ASCII digits are *not* numbers.
//!
//! [`PerlNum`] models the numeric value perl derives from a string (an IV/UV
//! when the string is a plain integer that fits, otherwise an NV parsed from
//! the longest numeric prefix, like `Atof`), the integer-preferring arithmetic
//! of `+`/`-`, `abs`, `<=>`, and the `%.15g` stringification of NVs.
//!
//! biber call sites:
//! * `looks_like_number`: `Utils.pm:1274/1278` (`map_boolean`), `Utils.pm:1656`
//!   (`rangelen`), `Biber.pm:4393` (integer sort columns), `Output/bbl.pm:124`
//!   and `Output/bblxml.pm:415` (`abs($year)`), `Constants.pm:125/165` and
//!   `bibtex.pm:1057/1063` (with `Unicode::UCD::num`, see
//!   [`crate::unicode::ucd_num`]).
//! * numeric conversion: `rangelen` (`$n < $m`, `$n - $m + 1`), `abs($y)` in
//!   the `.bbl` year output, `<=>` in integer sorting.

use std::cmp::Ordering;
use std::fmt;

/// Perl truthiness of a defined string: false iff `""` or `"0"`.
#[inline]
pub fn perl_truthy(s: &str) -> bool {
    !(s.is_empty() || s == "0")
}

/// Perl truthiness of a possibly-undefined string (`None` = `undef`).
#[inline]
pub fn perl_truthy_opt(s: Option<&str>) -> bool {
    s.is_some_and(perl_truthy)
}

/// Perl's `$a || $b` on strings: `a` if it is true, else `b`.
#[inline]
pub fn perl_or<'a>(a: &'a str, b: &'a str) -> &'a str {
    if perl_truthy(a) { a } else { b }
}

// ---------------------------------------------------------------------------
// grok_number

const IS_NUMBER_IN_UV: u32 = 0x01;
const IS_NUMBER_GREATER_THAN_UV_MAX: u32 = 0x02;
const IS_NUMBER_NOT_INT: u32 = 0x04;
const IS_NUMBER_NEG: u32 = 0x08;
const IS_NUMBER_INFINITY: u32 = 0x10;
const IS_NUMBER_NAN: u32 = 0x20;
const IS_NUMBER_TRAILING: u32 = 0x40;

/// Perl's `isSPACE` on a byte (ASCII whitespace including VT).
#[inline]
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C)
}

#[inline]
fn fold_eq(b: u8, c: u8) -> bool {
    b.eq_ignore_ascii_case(&c)
}

/// Port of `Perl_grok_infnan`. `sp` is the index into `s`; returns flags.
fn grok_infnan(s: &[u8], sp: &mut usize) -> u32 {
    let send = s.len();
    let mut i = *sp;
    let mut flags = 0;
    let mut odh = false;
    let at = |i: usize| -> u8 { if i < send { s[i] } else { 0 } };

    if at(i) == b'+' {
        i += 1;
        if i == send {
            return 0;
        }
    } else if at(i) == b'-' {
        flags |= IS_NUMBER_NEG;
        i += 1;
        if i == send {
            return 0;
        }
    }
    if at(i) == b'1' {
        i += 1;
        if i == send {
            return 0;
        }
        if at(i) == b'.' {
            i += 1;
            if i == send {
                return 0;
            }
        }
        if at(i) == b'#' {
            i += 1;
            if i == send {
                return 0;
            }
        } else {
            return 0;
        }
        odh = true;
    }
    if fold_eq(at(i), b'I') {
        i += 1;
        if i == send || !fold_eq(at(i), b'N') {
            return 0;
        }
        i += 1;
        if i == send {
            return 0;
        }
        if fold_eq(at(i), b'F') {
            flags |= IS_NUMBER_INFINITY | IS_NUMBER_NOT_INT;
            i += 1;
            *sp = i;
            if i < send && fold_eq(at(i), b'I') {
                let trail = flags | IS_NUMBER_TRAILING;
                for c in *b"NITY" {
                    i += 1;
                    if i == send || !fold_eq(at(i), c) {
                        return trail;
                    }
                }
                i += 1;
                *sp = i;
            } else if odh {
                while i < send && s[i] == b'0' {
                    i += 1;
                }
            }
        } else if fold_eq(at(i), b'D') && odh {
            i += 1;
            flags |= IS_NUMBER_NAN | IS_NUMBER_NOT_INT;
            while i < send && s[i] == b'0' {
                i += 1;
            }
        } else {
            return 0;
        }
    } else {
        if fold_eq(at(i), b'S') || fold_eq(at(i), b'Q') {
            i += 1;
            if i == send {
                return 0;
            }
        }
        if !fold_eq(at(i), b'N') {
            return 0;
        }
        i += 1;
        if i == send || !fold_eq(at(i), b'A') {
            return 0;
        }
        i += 1;
        if i == send || !fold_eq(at(i), b'N') {
            return 0;
        }
        flags |= IS_NUMBER_NAN | IS_NUMBER_NOT_INT;
        i += 1;
        *sp = i;
        if i == send {
            return flags;
        }
        if fold_eq(at(i), b'q') || fold_eq(at(i), b's') {
            i += 1;
            *sp = i;
            if i == send {
                return flags;
            }
        }
        if at(i) == b'(' {
            let trail = flags | IS_NUMBER_TRAILING;
            i += 1;
            if i == send {
                return trail;
            }
            let mut t = i + 1;
            while t < send && s[t] != 0 && s[t] != b')' {
                t += 1;
            }
            if t == send || s[t] != b')' {
                return trail;
            }
            let payload = &s[i..t];
            let nantype;
            if payload.len() > 2
                && payload[0] == b'0'
                && fold_eq(payload[1], b'x')
                && payload[2].is_ascii_hexdigit()
            {
                let (len, overflow) = grok_based(payload, 16);
                nantype = if overflow { 0 } else { IS_NUMBER_IN_UV };
                i += len;
            } else if payload.len() > 2
                && payload[0] == b'0'
                && fold_eq(payload[1], b'b')
                && (payload[2] == b'0' || payload[2] == b'1')
            {
                let (len, overflow) = grok_based(payload, 2);
                nantype = if overflow { 0 } else { IS_NUMBER_IN_UV };
                i += len;
            } else {
                nantype = grok_number_flags(payload, true);
                let mut u = i;
                while u < t && s[u].is_ascii_digit() {
                    u += 1;
                }
                i = u;
            }
            if (nantype & IS_NUMBER_NOT_INT) != 0 || (nantype & IS_NUMBER_IN_UV) == 0 {
                return trail;
            }
            while i < t && is_space(s[i]) {
                i += 1;
            }
            if i < t {
                return trail;
            }
            i = t + 1;
        }
    }
    // ok_check_space
    while i < send && is_space(s[i]) {
        i += 1;
    }
    *sp = i;
    flags | if i < send { IS_NUMBER_TRAILING } else { 0 }
}

/// `grok_hex`/`grok_bin` on a `0x…`/`0b…` NaN payload: returns the number of
/// bytes consumed and whether the value overflowed a UV.
fn grok_based(s: &[u8], base: u32) -> (usize, bool) {
    let mut i = 2;
    let mut value: u64 = 0;
    let mut overflow = false;
    while i < s.len() {
        let c = s[i];
        if c == b'_' && i + 1 < s.len() && (s[i + 1] as char).is_digit(base) {
            i += 1;
            continue;
        }
        let Some(d) = (c as char).to_digit(base) else {
            break;
        };
        match value
            .checked_mul(u64::from(base))
            .and_then(|v| v.checked_add(u64::from(d)))
        {
            Some(v) => value = v,
            None => overflow = true,
        }
        i += 1;
    }
    (i, overflow)
}

/// Port of `Perl_grok_number_flags` (perl 5.42, `numeric.c`). Returns the
/// `IS_NUMBER_*` flags (0 = not a number). `trailing` = `PERL_SCAN_TRAILING`.
fn grok_number_flags(s: &[u8], trailing: bool) -> u32 {
    let send = s.len();
    let at = |i: usize| -> u8 { if i < send { s[i] } else { 0 } };
    let mut i = 0;
    let mut numtype = 0;

    if is_space(at(i)) {
        i += 1;
        loop {
            if i >= send {
                return 0;
            }
            if !is_space(s[i]) {
                break;
            }
            i += 1;
        }
    }
    if at(i) == b'-' {
        i += 1;
        numtype = IS_NUMBER_NEG;
        if i == send {
            return 0;
        }
    } else if at(i) == b'+' {
        i += 1;
        if i == send {
            return 0;
        }
    }
    let d = i;
    if at(i).is_ascii_digit() {
        let mut value: u64 = u64::from(at(i) - b'0');
        i += 1;
        let mut overflow = false;
        while i < send && s[i].is_ascii_digit() {
            let digit = u64::from(s[i] - b'0');
            match value.checked_mul(10).and_then(|v| v.checked_add(digit)) {
                Some(v) => {
                    value = v;
                    i += 1;
                }
                None => {
                    while i < send && s[i].is_ascii_digit() {
                        i += 1;
                    }
                    numtype |= IS_NUMBER_GREATER_THAN_UV_MAX;
                    overflow = true;
                    break;
                }
            }
        }
        let _ = value;
        if i >= send && !overflow {
            return numtype | IS_NUMBER_IN_UV;
        }
        if !overflow {
            numtype |= IS_NUMBER_IN_UV;
        }
        if at(i) == b'.' && i < send {
            i += 1;
            numtype |= IS_NUMBER_NOT_INT;
            while i < send && s[i].is_ascii_digit() {
                i += 1;
            }
        }
    } else if at(i) == b'.' && i < send {
        i += 1;
        numtype |= IS_NUMBER_NOT_INT | IS_NUMBER_IN_UV;
        if i < send && s[i].is_ascii_digit() {
            while i < send && s[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            return 0;
        }
    }
    if i > d && i < send && fold_eq(s[i], b'e') {
        i += 1;
        if i < send && (s[i] == b'-' || s[i] == b'+') {
            i += 1;
        }
        if i < send && s[i].is_ascii_digit() {
            while i < send && s[i].is_ascii_digit() {
                i += 1;
            }
        } else if trailing {
            return numtype | IS_NUMBER_TRAILING;
        } else {
            return 0;
        }
        numtype &= IS_NUMBER_NEG;
        numtype |= IS_NUMBER_NOT_INT;
    }
    while i < send {
        if !is_space(s[i]) {
            // end_space
            if s == b"0 but true" {
                return IS_NUMBER_IN_UV;
            }
            if i + 2 < send && matches!(s[i].to_ascii_lowercase(), b'i' | b'n' | b'q' | b's' | b'#')
            {
                let mut dd = d;
                let infnan = grok_infnan(s, &mut dd);
                if (infnan & IS_NUMBER_TRAILING) != 0 && !trailing {
                    return 0;
                }
                if (infnan & IS_NUMBER_INFINITY) != 0 {
                    return numtype | infnan;
                } else if (infnan & IS_NUMBER_NAN) != 0 {
                    return (numtype | infnan) & !IS_NUMBER_NEG;
                }
            } else if trailing {
                return numtype | IS_NUMBER_TRAILING;
            }
            return 0;
        }
        i += 1;
    }
    numtype
}

/// `Scalar::Util::looks_like_number` on a defined string.
///
/// ```
/// use perl_compat::truth::looks_like_number;
/// assert!(looks_like_number(" 12\n"));
/// assert!(looks_like_number("1e5") && looks_like_number(".5") && looks_like_number("Inf"));
/// assert!(!looks_like_number("0x10") && !looks_like_number("") && !looks_like_number("1_000"));
/// ```
pub fn looks_like_number(s: &str) -> bool {
    let t = grok_number_flags(s.as_bytes(), false);
    t != 0 && (t & IS_NUMBER_TRAILING) == 0
}

// ---------------------------------------------------------------------------
// numeric values

/// A Perl numeric value: IV, UV or NV.
#[derive(Debug, Clone, Copy)]
pub enum PerlNum {
    /// Signed integer (IV).
    Int(i64),
    /// Unsigned integer larger than `i64::MAX` (UV).
    UInt(u64),
    /// Floating point (NV).
    Float(f64),
}

impl PerlNum {
    /// Numify a string as perl does (`$s + 0`): an integer if the whole string
    /// (modulo surrounding whitespace) is an integer that fits in an IV/UV,
    /// otherwise a double parsed from the longest numeric prefix (0 if none).
    pub fn parse(s: &str) -> PerlNum {
        let b = s.as_bytes();
        let t = grok_number_flags(b, false);
        if (t & (IS_NUMBER_IN_UV | IS_NUMBER_NOT_INT)) == IS_NUMBER_IN_UV {
            // pure integer: parse digits
            let mut v: u64 = 0;
            if s != "0 but true" {
                for &c in b
                    .iter()
                    .skip_while(|&&c| is_space(c) || c == b'+' || c == b'-')
                {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    v = v * 10 + u64::from(c - b'0');
                }
            }
            if t & IS_NUMBER_NEG != 0 {
                return match v {
                    v if v <= i64::MAX as u64 => PerlNum::Int(-(v as i64)),
                    v if v == i64::MAX as u64 + 1 => PerlNum::Int(i64::MIN),
                    v => PerlNum::Float(-(v as f64)),
                };
            }
            return if v <= i64::MAX as u64 {
                PerlNum::Int(v as i64)
            } else {
                PerlNum::UInt(v)
            };
        }
        let nv = atof(b);
        // sv_2iuv_common: does the NV also get a public integer value (IOK)?
        if t == 0
            || (t & (IS_NUMBER_IN_UV | IS_NUMBER_NOT_INT)) == (IS_NUMBER_IN_UV | IS_NUMBER_NOT_INT)
        {
            return PerlNum::Float(nv);
        }
        if nv.is_nan() || nv.fract() != 0.0 {
            return PerlNum::Float(nv);
        }
        if nv.abs() < 9_007_199_254_740_992.0 {
            return PerlNum::Int(nv as i64);
        }
        // sv_2iuv_non_preserve
        if nv < i64::MIN as f64 || nv > u64::MAX as f64 {
            return PerlNum::Float(nv);
        }
        if nv < -(i64::MIN as f64) {
            return PerlNum::Int(nv as i64);
        }
        let u = nv as u64;
        if u == u64::MAX {
            PerlNum::Float(nv)
        } else {
            PerlNum::UInt(u)
        }
    }

    /// The value as a double.
    pub fn as_f64(self) -> f64 {
        match self {
            PerlNum::Int(i) => i as f64,
            PerlNum::UInt(u) => u as f64,
            PerlNum::Float(f) => f,
        }
    }

    /// Integer view used by perl's arithmetic ops (`SvIV_please_nomg`):
    /// IVs and UVs. NVs are treated as having no public integer value (true
    /// for NVs numified from strings that are not plain integers; computed
    /// integral NVs below 2**53 would get one in perl, which only matters
    /// for chained arithmetic beyond 2**53).
    fn as_int(self) -> Option<i128> {
        match self {
            PerlNum::Int(i) => Some(i128::from(i)),
            PerlNum::UInt(u) => Some(i128::from(u)),
            PerlNum::Float(_) => None,
        }
    }

    fn from_i128(r: i128) -> Option<PerlNum> {
        if let Ok(i) = i64::try_from(r) {
            Some(PerlNum::Int(i))
        } else if let Ok(u) = u64::try_from(r) {
            Some(PerlNum::UInt(u))
        } else {
            None
        }
    }

    /// Perl `$a + $b`.
    pub fn plus(self, o: PerlNum) -> PerlNum {
        if let (Some(a), Some(b)) = (self.as_int(), o.as_int())
            && let Some(r) = PerlNum::from_i128(a + b)
        {
            return r;
        }
        PerlNum::Float(self.as_f64() + o.as_f64())
    }

    /// Perl `$a - $b`.
    pub fn minus(self, o: PerlNum) -> PerlNum {
        if let (Some(a), Some(b)) = (self.as_int(), o.as_int())
            && let Some(r) = PerlNum::from_i128(a - b)
        {
            return r;
        }
        PerlNum::Float(self.as_f64() - o.as_f64())
    }

    /// Perl `abs($a)`.
    pub fn abs(self) -> PerlNum {
        match self {
            PerlNum::Int(i) => match i.checked_abs() {
                Some(a) => PerlNum::Int(a),
                None => PerlNum::UInt(i.unsigned_abs()),
            },
            PerlNum::UInt(u) => PerlNum::UInt(u),
            PerlNum::Float(f) => match self.as_int().and_then(|i| PerlNum::from_i128(i.abs())) {
                Some(n) => n,
                None => PerlNum::Float(f.abs()),
            },
        }
    }

    /// Perl `<=>`: `None` if either side is NaN (perl returns `undef`).
    pub fn num_cmp(self, o: PerlNum) -> Option<Ordering> {
        match (self, o) {
            (PerlNum::Int(a), PerlNum::Int(b)) => Some(a.cmp(&b)),
            (PerlNum::UInt(a), PerlNum::UInt(b)) => Some(a.cmp(&b)),
            (PerlNum::Int(_), PerlNum::UInt(_)) => Some(Ordering::Less),
            (PerlNum::UInt(_), PerlNum::Int(_)) => Some(Ordering::Greater),
            _ => self.as_f64().partial_cmp(&o.as_f64()),
        }
    }
}

impl fmt::Display for PerlNum {
    /// Perl's stringification: integers in decimal, NVs as `%.15g`, `Inf`,
    /// `-Inf`, `NaN`; negative zero prints as `0`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            PerlNum::Int(i) => write!(f, "{i}"),
            PerlNum::UInt(u) => write!(f, "{u}"),
            PerlNum::Float(x) => f.write_str(&format_nv(x)),
        }
    }
}

/// Perl's NV stringification (`%.15g`, plus `Inf`/`-Inf`/`NaN`).
pub fn format_nv(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 {
            "Inf".to_string()
        } else {
            "-Inf".to_string()
        };
    }
    if x == 0.0 {
        return "0".to_string();
    }
    format_g(x, 15)
}

/// C `printf("%.{prec}g", x)` for finite non-zero `x`.
pub fn format_g(x: f64, prec: usize) -> String {
    let prec = prec.max(1);
    // Scientific representation with `prec` significant digits, exactly rounded.
    let sci = format!("{:.*e}", prec - 1, x);
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if exp < -4 || exp >= prec as i32 {
        let mant = strip_zeros(mant);
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{mant}e{sign}{:02}", exp.unsigned_abs())
    } else {
        let decimals = (prec as i32 - 1 - exp).max(0) as usize;
        strip_zeros(&format!("{x:.decimals$}")).to_string()
    }
}

fn strip_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// Perl's `Atof` (`my_atof3`): parse the longest numeric prefix after
/// leading whitespace; `Inf`/`NaN` spellings; 0 when there is none.
fn atof(s: &[u8]) -> f64 {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    // S_my_atof_infnan: Inf/NaN spellings win, trailing garbage allowed.
    let mut p = i;
    let fl = grok_infnan(s, &mut p);
    if fl != 0 && p != i {
        if fl & IS_NUMBER_INFINITY != 0 {
            return if fl & IS_NUMBER_NEG != 0 {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        }
        if fl & IS_NUMBER_NAN != 0 {
            return f64::NAN;
        }
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    // "0x…"/"0b…" are not parsed (value 0).
    if i + 1 < s.len() && s[i] == b'0' && matches!(s[i + 1].to_ascii_lowercase(), b'x' | b'b') {
        return 0.0;
    }
    // strtod on the decimal prefix.
    let num_start = i;
    let mut seen_digit = false;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
        seen_digit = true;
    }
    if i < s.len() && s[i] == b'.' {
        let mut k = i + 1;
        while k < s.len() && s[k].is_ascii_digit() {
            k += 1;
        }
        if k > i + 1 || seen_digit {
            seen_digit |= k > i + 1;
            i = k;
        }
    }
    if !seen_digit {
        return 0.0;
    }
    if i < s.len() && fold_eq(s[i], b'e') {
        let mut k = i + 1;
        if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
            k += 1;
        }
        let ds = k;
        while k < s.len() && s[k].is_ascii_digit() {
            k += 1;
        }
        if k > ds {
            i = k;
        }
    }
    // The prefix is pure ASCII.
    let text = std::str::from_utf8(&s[num_start..i]).unwrap_or("0");
    let v: f64 = text.parse().unwrap_or(0.0);
    if neg { -v } else { v }
}

/// `$a <=> $b` on two strings (numifying both).
pub fn num_cmp_str(a: &str, b: &str) -> Option<Ordering> {
    PerlNum::parse(a).num_cmp(PerlNum::parse(b))
}

/// `$a <=> $b` on possibly-undefined strings (`undef` numifies to 0), with
/// NaN comparisons (perl: `undef`, which `sort` treats as 0) mapped to
/// `Equal` — what `sort { $a->{order} <=> $b->{order} }` sees.
pub fn num_cmp_opt(a: Option<&str>, b: Option<&str>) -> Ordering {
    let x = a.map_or(PerlNum::Int(0), PerlNum::parse);
    let y = b.map_or(PerlNum::Int(0), PerlNum::parse);
    x.num_cmp(y).unwrap_or(Ordering::Equal)
}

/// Perl numification of a string as a double (`$s + 0.0`).
pub fn numify(s: &str) -> f64 {
    PerlNum::parse(s).as_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthiness() {
        assert!(!perl_truthy(""));
        assert!(!perl_truthy("0"));
        for t in ["00", "0.0", " 0", "0E0", "a", "0\n"] {
            assert!(perl_truthy(t), "{t:?}");
        }
        assert!(!perl_truthy_opt(None));
        assert_eq!(perl_or("0", "x"), "x");
    }

    #[test]
    fn lln_basic() {
        for t in [
            "1",
            "-1",
            "+1",
            " 1 ",
            "1\n",
            "1.",
            ".1",
            "1.5e3",
            "1E-3",
            "Inf",
            "-inf",
            "nan",
            "Infinity",
            "0 but true",
            "1.#INF",
            "nan(123)",
            "nanq",
            "\x0B5",
        ] {
            assert!(looks_like_number(t), "{t:?}");
        }
        for t in [
            "", " ", ".", "-", "+", "1e", "0x10", "1_0", "e5", "1 2", "abc", "infin", "١",
        ] {
            assert!(!looks_like_number(t), "{t:?}");
        }
    }

    #[test]
    fn numify() {
        assert_eq!(PerlNum::parse("12abc").to_string(), "12");
        assert_eq!(PerlNum::parse("-0500").abs().to_string(), "500");
        assert_eq!(PerlNum::parse("1e3").to_string(), "1000");
        assert_eq!(PerlNum::parse("1e15").to_string(), "1000000000000000");
        assert_eq!(format_nv(1e15), "1e+15");
        assert_eq!(
            PerlNum::parse("0.1")
                .plus(PerlNum::parse("0.2"))
                .to_string(),
            "0.3"
        );
        assert_eq!(PerlNum::parse("abc").to_string(), "0");
        assert_eq!(PerlNum::parse("1e400").to_string(), "Inf");
        assert_eq!(
            PerlNum::parse("12").minus(PerlNum::parse("5")).to_string(),
            "7"
        );
        assert_eq!(format_nv(1.0 / 3.0), "0.333333333333333");
        assert_eq!(format_nv(1.215_766_545_905_69e19), "1.21576654590569e+19");
    }
}

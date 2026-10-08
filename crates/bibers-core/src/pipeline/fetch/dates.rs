//! ISO 8601-2 date parsing: `parse_date_range`, `parse_date_unspecified`,
//! `parse_date` (Utils.pm:1407-1551) and `Biber::Date::Format` (the
//! `DateTime::Format::Builder` parser with its `_pre` preprocessing, plus the
//! `DateTime` validation and Julian conversion biber relies on).

use perl_compat::truth::PerlNum;
use perl_compat::unicode::{Prop, is_perl_digit, is_perl_space, ucd_num};
use std::collections::HashMap;

/// `%YEARDIVISIONS` (Constants.pm:77).
fn yeardivision(n: &str) -> Option<&'static str> {
    Some(match n {
        "21" => "spring",
        "22" => "summer",
        "23" => "autumn",
        "24" => "winter",
        "25" => "springN",
        "26" => "summerN",
        "27" => "autumnN",
        "28" => "winterN",
        "29" => "springS",
        "30" => "summerS",
        "31" => "autumnS",
        "32" => "winterS",
        "33" => "Q1",
        "34" => "Q2",
        "35" => "Q3",
        "36" => "Q4",
        "37" => "QD1",
        "38" => "QD2",
        "39" => "QD3",
        "40" => "S1",
        "41" => "S2",
        _ => return None,
    })
}

/// A successfully parsed (and validated) date/time: a `DateTime` (or a
/// `DateTime::Calendar::Julian` after conversion).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dt {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// The time zone name; `None` for a floating time zone.
    pub tz: Option<String>,
}

impl Dt {
    /// `lc($dt->secular_era)`.
    pub fn era(&self) -> &'static str {
        if self.year > 0 { "ce" } else { "bce" }
    }
}

/// The result of `parse_date`: `undef` (bad parse), `0` (no date / open
/// `..`), or a date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    Undef,
    Zero,
    Date(Dt),
}

impl Parsed {
    pub fn is_defined(&self) -> bool {
        !matches!(self, Parsed::Undef)
    }
}

/// One `Biber::Date::Format` object (biber has two: start and end) with the
/// meta-information of its last parse.
#[derive(Clone, Debug, Default)]
pub struct DateParser {
    missing_month: bool,
    missing_day: bool,
    missing_time: bool,
    pub approximate: bool,
    pub uncertain: bool,
    pub yeardivision: Option<&'static str>,
    pub julian: bool,
    /// `{scriptmap}{atos}`: arabic → original script digits.
    atos: Option<HashMap<String, String>>,
}

/// Julian-conversion settings (`julian`, `gregorianstart` biblatex options).
#[derive(Clone, Copy, Debug)]
pub struct JulianOpts {
    pub enabled: bool,
    /// `(year, month, day)` of `gregorianstart`, if it parses.
    pub gregorianstart: Option<(i64, u32, u32)>,
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Days (same epoch as [`days_from_civil`]) of a Julian-calendar date, before
/// the epoch shift.
fn days_from_julian_raw(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(4);
    let yoe = y - era * 4;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    era * 1461 + yoe * 365 + doy
}

/// The Julian calendar date of a day number (as from [`days_from_civil`]).
fn julian_from_days(z: i64) -> (i64, u32, u32) {
    // 1970-01-01 Gregorian is 1969-12-19 Julian.
    let z = z + days_from_julian_raw(1969, 12, 19);
    let era = z.div_euclid(1461);
    let doe = z - era * 1461;
    let yoe = (doe / 365).min(3);
    let doy = doe - 365 * yoe;
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 4 + i64::from(m <= 2);
    (y, m, d)
}

/// Text of a `\d{N}` match at the start of `s` (ASCII digits only after
/// script mapping); returns the digits and the rest.
fn take_digits(s: &str, n: usize) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    if b.len() < n || !b[..n].iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some((&s[..n], &s[n..]))
}

/// `(-?\d{4})` at the start of `s`.
fn take_year4(s: &str) -> Option<(i64, &str)> {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (d, rest) = take_digits(rest, 4)?;
    let v: i64 = d.parse().ok()?;
    Some((if neg { -v } else { v }, rest))
}

fn two(s: &str) -> Option<(u32, &str)> {
    let (d, rest) = take_digits(s, 2)?;
    Some((d.parse().ok()?, rest))
}

/// `s/^\s*(.+?)\s*X\s*$/$1/`: strip a trailing marker character.
fn strip_marker(s: &str, marker: char) -> Option<String> {
    let t = s.trim_start_matches(is_perl_space);
    let u = t.trim_end_matches(is_perl_space);
    let inner = u.strip_suffix(marker)?;
    if inner.is_empty() {
        return None;
    }
    Some(inner.trim_end_matches(is_perl_space).to_string())
}

/// `sprintf("%0${len}d", $num)`.
fn pad_num(n: Option<PerlNum>, len: usize) -> String {
    let v: i128 = match n {
        Some(PerlNum::Int(i)) => i128::from(i),
        Some(PerlNum::UInt(u)) => i128::from(u),
        Some(PerlNum::Float(f)) => f.trunc() as i128,
        None => 0,
    };
    if v < 0 {
        format!("-{:0w$}", -v, w = len.saturating_sub(1))
    } else {
        format!("{v:0len$}")
    }
}

fn num_string(n: Option<PerlNum>) -> String {
    match n {
        Some(PerlNum::Int(i)) => i.to_string(),
        Some(PerlNum::UInt(u)) => u.to_string(),
        Some(PerlNum::Float(f)) => perl_compat::truth::format_nv(f),
        None => String::new(),
    }
}

/// Runs of `\d` (Unicode decimal digits) in `s`, as (start, end) byte spans.
fn digit_runs(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        if is_perl_digit(c) {
            start.get_or_insert(i);
        } else if let Some(st) = start.take() {
            out.push((st, i));
        }
    }
    if let Some(st) = start {
        out.push((st, s.len()));
    }
    out
}

/// Valid `±hh:mm` offset → time zone name as `DateTime::TimeZone` gives it.
fn tz_name(off: &str) -> Option<String> {
    let sign = &off[..1];
    let h: u32 = off[1..3].parse().ok()?;
    let m: u32 = off[4..6].parse().ok()?;
    if m > 59 || h > 99 {
        return None;
    }
    if h == 0 && m == 0 {
        return Some("UTC".into());
    }
    Some(format!("{sign}{h:02}{m:02}"))
}

impl DateParser {
    /// `init`: reset the meta-information.
    pub fn init(&mut self) {
        *self = DateParser::default();
    }

    /// `missing($part)`.
    pub fn missing(&self, part: &str) -> bool {
        match part {
            "month" => self.missing_month,
            "day" => self.missing_day,
            "time" => self.missing_time,
            _ => false,
        }
    }

    /// `resolvescript($dp)`: the original-script form of a date part.
    pub fn resolvescript(&self, dp: &str) -> String {
        self.atos
            .as_ref()
            .and_then(|m| m.get(dp))
            .cloned()
            .unwrap_or_else(|| dp.to_string())
    }

    /// `_pre` (Format.pm:117): returns the preprocessed input and time zone.
    fn pre(&mut self, input: &str) -> (String, Option<String>) {
        self.missing_month = false;
        self.missing_day = false;
        self.missing_time = false;
        self.approximate = false;
        self.uncertain = false;
        self.yeardivision = None;
        let mut s = input.to_string();
        let mut stoa: HashMap<String, String> = HashMap::new();
        for (a, b) in digit_runs(&s) {
            let num = &s[a..b];
            let lnum = num.chars().count();
            let rnum = ucd_num(num);
            let anum = pad_num(rnum, lnum);
            if num != anum {
                let atos = self.atos.get_or_insert_with(HashMap::new);
                atos.insert(anum.clone(), num.to_string());
                atos.insert(num_string(rnum), num.to_string());
                stoa.insert(num.to_string(), anum);
            }
        }
        if self.atos.is_some() {
            let mut out = String::new();
            let mut last = 0;
            for (a, b) in digit_runs(&s) {
                out.push_str(&s[last..a]);
                if let Some(r) = stoa.get(&s[a..b]) {
                    out.push_str(r);
                }
                last = b;
            }
            out.push_str(&s[last..]);
            s = out;
        }
        if let Some(r) = strip_marker(&s, '?') {
            s = r;
            self.uncertain = true;
        }
        if let Some(r) = strip_marker(&s, '~') {
            s = r;
            self.approximate = true;
        }
        if let Some(r) = strip_marker(&s, '%') {
            s = r;
            self.uncertain = true;
            self.approximate = true;
        }
        let mut tz = None;
        if let Some(r) = s.strip_suffix('Z') {
            s = r.to_string();
            tz = Some("UTC".to_string());
        } else if let Some(t) = s.get(s.len().saturating_sub(6)..)
            && t.len() == 6
        {
            // A timezone suffix is ASCII; a non-boundary offset (multi-byte
            // text) simply cannot be one.
            let b = t.as_bytes();
            if (b[0] == b'+' || b[0] == b'-')
                && b[1].is_ascii_digit()
                && b[2].is_ascii_digit()
                && b[3] == b':'
                && b[4].is_ascii_digit()
                && b[5].is_ascii_digit()
            {
                tz = Some(t.to_string());
                let cut = s.len() - 6;
                s.truncate(cut);
            }
        }
        // ISO8601-2:2016 4.8 (yeardivisions)
        if let Some((_, rest)) = take_year4(&s)
            && let Some(div) = rest.strip_prefix('-')
            && div.len() == 2
            && let Some((n, "")) = two(div)
            && ((20..=39).contains(&n) || n == 40 || n == 41)
        {
            self.yeardivision = yeardivision(div);
            let yl = s.len() - 3;
            s.truncate(yl);
        }
        (s, tz)
    }

    /// `parse_datetime`: `None` if no parser matched or `DateTime` rejected
    /// the values.
    pub fn parse_datetime(&mut self, input: &str) -> Option<Dt> {
        let (s, tz) = self.pre(input);
        let len = s.chars().count();
        let mut dt = Dt {
            year: 0,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            tz: None,
        };
        let mut matched = false;
        if [19, 20, 23, 24].contains(&len)
            && let Some((y, r)) = take_year4(&s)
            && let Some(r) = r.strip_prefix('-')
            && let Some((mo, r)) = two(r)
            && let Some(r) = r.strip_prefix('-')
            && let Some((d, r)) = two(r)
            && let Some(r) = r.strip_prefix('T')
            && let Some((h, r)) = two(r)
            && let Some(r) = r.strip_prefix(':')
            && let Some((mi, r)) = two(r)
            && let Some(r) = r.strip_prefix(':')
            && let Some((se, r)) = two(r)
            && (r.is_empty()
                || r.strip_prefix('.')
                    .and_then(|x| take_digits(x, 3))
                    .is_some_and(|(_, x)| x.is_empty()))
        {
            dt = Dt {
                year: y,
                month: mo,
                day: d,
                hour: h,
                minute: mi,
                second: se,
                tz: None,
            };
            matched = true;
        } else if [10, 11].contains(&len)
            && let Some((y, r)) = take_year4(&s)
            && let Some(r) = r.strip_prefix('-')
            && let Some((mo, r)) = two(r)
            && let Some(r) = r.strip_prefix('-')
            && let Some((d, "")) = two(r)
        {
            dt.year = y;
            dt.month = mo;
            dt.day = d;
            self.missing_time = true;
            matched = true;
        } else if [7, 8].contains(&len)
            && let Some((y, r)) = take_year4(&s)
            && let Some(r) = r.strip_prefix('-')
            && let Some((mo, "")) = two(r)
        {
            dt.year = y;
            dt.month = mo;
            self.missing_day = true;
            self.missing_time = true;
            matched = true;
        } else if [4, 5].contains(&len)
            && let Some((y, "")) = take_year4(&s)
        {
            dt.year = y;
            self.missing_month = true;
            self.missing_day = true;
            self.missing_time = true;
            matched = true;
        } else if let Some(r) = s.strip_prefix('Y') {
            let (neg, digits) = match r.strip_prefix('-') {
                Some(x) => (true, x),
                None => (false, r),
            };
            if digits.len() >= 5 && digits.bytes().all(|b| b.is_ascii_digit()) {
                let v: i64 = digits.parse().ok()?;
                dt.year = if neg { -v } else { v };
                self.missing_month = true;
                self.missing_day = true;
                self.missing_time = true;
                matched = true;
            }
        }
        if !matched {
            return None;
        }
        // DateTime->new validation
        if !(1..=12).contains(&dt.month)
            || dt.day < 1
            || dt.day > days_in_month(dt.year, dt.month)
            || dt.hour > 23
            || dt.minute > 59
            || dt.second > 59
        {
            return None;
        }
        dt.tz = match tz {
            None => None,
            Some(t) if t == "UTC" => Some(t),
            Some(t) => Some(tz_name(&t)?),
        };
        Some(dt)
    }
}

/// `parse_date($obj, $string)` (Utils.pm:1508).
pub fn parse_date(p: &mut DateParser, s: Option<&str>, jul: JulianOpts) -> Parsed {
    p.init();
    let Some(s) = s.filter(|s| perl_compat::perl_truthy(s)) else {
        return Parsed::Zero;
    };
    if s == ".." {
        return Parsed::Zero;
    }
    let Some(mut dt) = p.parse_datetime(s) else {
        return Parsed::Undef;
    };
    if jul.enabled
        && !p.missing("month")
        && !p.missing("day")
        && let Some((gy, gm, gd)) = jul.gregorianstart
        && (dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second) < (gy, gm, gd, 0, 0, 0)
    {
        let (y, m, d) = julian_from_days(days_from_civil(dt.year, dt.month, dt.day));
        dt.year = y;
        dt.month = m;
        dt.day = d;
        p.julian = true;
    }
    Parsed::Date(dt)
}

/// `parse_date_unspecified` (Utils.pm:1438): `(start, sep, end, unspec)`,
/// or `None` (Perl's empty list) when no pattern matches.
fn parse_date_unspecified(d: &str) -> Option<(String, String, String, &'static str)> {
    let c: Vec<char> = d.chars().collect();
    let dig = |r: std::ops::Range<usize>| r.clone().all(|i| i < c.len() && is_perl_digit(c[i]));
    let dash = |i: usize| i < c.len() && Prop::Dash.contains(c[i]);
    let x = |i: usize| i < c.len() && c[i] == 'X';
    let s = |r: std::ops::Range<usize>| c[r].iter().collect::<String>();
    let n = c.len();
    if n == 4 && dig(0..3) && x(3) {
        let p = s(0..3);
        return Some((format!("{p}0"), "/".into(), format!("{p}9"), "yearindecade"));
    }
    if n == 4 && dig(0..2) && x(2) && x(3) {
        let p = s(0..2);
        return Some((
            format!("{p}00"),
            "/".into(),
            format!("{p}99"),
            "yearincentury",
        ));
    }
    if n == 7 && dig(0..4) && dash(4) && x(5) && x(6) {
        let y = s(0..4);
        return Some((
            format!("{y}-01"),
            "/".into(),
            format!("{y}-12"),
            "monthinyear",
        ));
    }
    if n == 10 && dig(0..4) && dash(4) && dig(5..7) && dash(7) && x(8) && x(9) {
        let y = s(0..4);
        let m = s(5..7);
        let yn: i64 = perl_compat::truth::numify(&y) as i64;
        let days = match m.as_str() {
            "01" | "03" | "05" | "07" | "08" | "10" | "12" => "31",
            "09" | "04" | "06" | "11" => "30",
            "02" => {
                if is_leap(yn) {
                    "29"
                } else {
                    "28"
                }
            }
            // $monthdays{$2} is undef for other months
            _ => "",
        };
        return Some((
            format!("{y}-{m}-01"),
            "/".into(),
            format!("{y}-{m}-{days}"),
            "dayinmonth",
        ));
    }
    if n == 10 && dig(0..4) && dash(4) && x(5) && x(6) && dash(7) && x(8) && x(9) {
        let y = s(0..4);
        return Some((
            format!("{y}-01-01"),
            "/".into(),
            format!("{y}-12-31"),
            "dayinyear",
        ));
    }
    None
}

/// What `parse_date_range` returns plus the flags it sets on the entry.
#[derive(Clone, Debug)]
pub struct DateRange {
    pub start: Parsed,
    pub end: Parsed,
    pub sep: bool,
    pub unspec: Option<&'static str>,
    /// `<type>dateunknown` was set.
    pub start_unknown: bool,
    /// `<type>enddateunknown` was set.
    pub end_unknown: bool,
}

/// `parse_date_range` (Utils.pm:1407). Returns `None` for the early
/// "very bad date format" return (all `undef`).
pub fn parse_date_range(
    start_p: &mut DateParser,
    end_p: &mut DateParser,
    datestring: &str,
    jul: JulianOpts,
) -> Option<DateRange> {
    // m|^([^/]+)?(/)?([^/]+)?$|
    let ds = datestring.strip_suffix('\n').unwrap_or(datestring);
    let parts: Vec<&str> = ds.split('/').collect();
    let (sd, sep, ed) = match parts.len() {
        1 => (Some(parts[0]), false, None),
        2 => (Some(parts[0]), true, Some(parts[1])),
        _ => (None, false, None),
    };
    if parts.len() > 2 {
        return None;
    }
    let sd = sd.filter(|x| !x.is_empty()).map(str::to_string);
    let ed = ed.filter(|x| !x.is_empty()).map(str::to_string);
    let truthy = |x: &Option<String>| x.as_deref().is_some_and(perl_compat::perl_truthy);
    if !truthy(&sd) && !truthy(&ed) {
        return None;
    }
    let (mut sd, mut sep, mut ed, mut unspec) = (sd, sep, ed, None);
    if sd.as_deref().is_some_and(|s| s.contains('X')) {
        match parse_date_unspecified(sd.as_deref().unwrap_or("")) {
            Some((a, _, c, u)) => {
                sd = Some(a);
                sep = true;
                ed = Some(c);
                unspec = Some(u);
            }
            None => {
                sd = None;
                sep = false;
                ed = None;
            }
        }
    }
    let start_unknown = sep && !truthy(&sd);
    let end_unknown = sep && !truthy(&ed);
    let start = parse_date(start_p, sd.as_deref(), jul);
    let end = parse_date(end_p, ed.as_deref(), jul);
    Some(DateRange {
        start,
        end,
        sep,
        unspec,
        start_unknown,
        end_unknown,
    })
}

/// `tzformat` (Utils.pm:1903).
pub fn tzformat(tz: &str) -> String {
    let b = tz.as_bytes();
    if b.len() >= 3
        && (b[0] == b'+' || b[0] == b'-')
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
    {
        let mut rest = &tz[3..];
        rest = rest.strip_prefix(':').unwrap_or(rest);
        let mins = take_digits(rest, 2).map(|(m, _)| m);
        let mut out = tz[..3].to_string();
        if let Some(m) = mins.filter(|m| perl_compat::perl_truthy(m)) {
            out.push_str("\\bibtzminsep ");
            out.push_str(m);
        }
        out
    } else if tz == "UTC" {
        "Z".into()
    } else {
        tz.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOJ: JulianOpts = JulianOpts {
        enabled: false,
        gregorianstart: None,
    };

    #[test]
    fn basic() {
        let mut p = DateParser::default();
        let Parsed::Date(d) = parse_date(&mut p, Some("2004-04-05T14:34:00+05:00"), NOJ) else {
            panic!()
        };
        assert_eq!((d.year, d.month, d.day, d.hour), (2004, 4, 5, 14));
        assert_eq!(tzformat(d.tz.as_deref().unwrap()), "+05\\bibtzminsep 00");
        assert_eq!(parse_date(&mut p, Some("2004-02-30"), NOJ), Parsed::Undef);
        let Parsed::Date(d) = parse_date(&mut p, Some("-0044-03-15"), NOJ) else {
            panic!()
        };
        assert_eq!((d.year, d.era()), (-44, "bce"));
        let Parsed::Date(_) = parse_date(&mut p, Some("1988?"), NOJ) else {
            panic!()
        };
        assert!(p.uncertain && p.missing("month"));
        let Parsed::Date(_) = parse_date(&mut p, Some("2001-22"), NOJ) else {
            panic!()
        };
        assert_eq!(p.yeardivision, Some("summer"));
        let Parsed::Date(d) = parse_date(&mut p, Some("٢٠٠٠-٠١"), NOJ) else {
            panic!()
        };
        assert_eq!(p.resolvescript(&d.year.to_string()), "٢٠٠٠");
        assert_eq!(p.resolvescript(&d.month.to_string()), "٠١");
    }

    #[test]
    fn julian() {
        // 1582-10-15 Gregorian = 1582-10-05 Julian
        let z = days_from_civil(1582, 10, 15);
        assert_eq!(julian_from_days(z), (1582, 10, 5));
        assert_eq!(
            julian_from_days(days_from_civil(1970, 1, 1)),
            (1969, 12, 19)
        );
        assert_eq!(julian_from_days(days_from_civil(-44, 3, 13)), (-44, 3, 15));
    }
}

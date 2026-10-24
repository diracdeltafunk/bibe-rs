//! Datatype libraries: the RELAX NG built-in library (`string`, `token`;
//! specification §6.2.9) and the W3C XML Schema datatypes, with the
//! parameters allowed by *Guidelines for using W3C XML Schema Datatypes
//! with RELAX NG* (all facets except `whiteSpace` and `enumeration`).
//!
//! Each datatype has a whitespace mode (preserve, replace or collapse), a
//! lexical space checked by a hand-written parser, and a value space used
//! for `value` patterns and the ordering facets.

use std::cmp::Ordering;

use regex::Regex;

use crate::XSD_DATATYPES;
use crate::schema::NsContext;
use crate::xsdregex;

/// Resolves a namespace prefix ("" for the default namespace) in the
/// context of a text or attribute.
pub(crate) type Ctx<'a> = &'a dyn Fn(&str) -> Option<String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ws {
    Preserve,
    Replace,
    Collapse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    String,
    NormalizedString,
    Token,
    Language,
    Name,
    NcName,
    NmToken,
    NmTokens,
    NcNames, // IDREFS, ENTITIES
    Boolean,
    Decimal,
    Integer {
        min: Option<&'static str>,
        max: Option<&'static str>,
    },
    Float,
    Double,
    AnyUri,
    QName,
    DateTime,
    Date,
    Time,
    GYear,
    GYearMonth,
    GMonth,
    GMonthDay,
    GDay,
    Duration,
    HexBinary,
    Base64Binary,
}

/// A value in a datatype's value space.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Val {
    Str(String),
    Bool(bool),
    Dec(Decimal),
    Float(f64),
    QName(String, String),
    Bytes(Vec<u8>),
    List(Vec<String>),
    Moment(Moment),
    Duration(i64, Decimal),
}

/// A compiled `data` datatype (with its parameters).
#[derive(Clone, Debug)]
pub(crate) struct DataType {
    kind: Kind,
    ws: Ws,
    length: Option<usize>,
    min_length: Option<usize>,
    max_length: Option<usize>,
    patterns: Vec<Regex>,
    min_inclusive: Option<Val>,
    max_inclusive: Option<Val>,
    min_exclusive: Option<Val>,
    max_exclusive: Option<Val>,
    total_digits: Option<usize>,
    fraction_digits: Option<usize>,
}

/// A compiled `value` pattern.
#[derive(Clone, Debug)]
pub(crate) struct ValueSpec {
    pub(crate) dt: DataType,
    pub(crate) value: Val,
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

pub(crate) fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for w in s.split(is_ws).filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

fn normalize(ws: Ws, s: &str) -> std::borrow::Cow<'_, str> {
    match ws {
        Ws::Preserve => s.into(),
        Ws::Replace => s.replace(is_ws, " ").into(),
        Ws::Collapse => collapse(s).into(),
    }
}

fn lookup(lib: &str, ty: &str) -> Result<Kind, String> {
    use Kind::*;
    if lib.is_empty() {
        return match ty {
            "string" => Ok(String),
            "token" => Ok(Token),
            _ => Err(format!("unknown datatype '{ty}' in the built-in library")),
        };
    }
    if lib != XSD_DATATYPES {
        return Err(format!("unsupported datatype library '{lib}'"));
    }
    Ok(match ty {
        "string" => String,
        "normalizedString" => NormalizedString,
        "token" => Token,
        "language" => Language,
        "Name" => Name,
        "NCName" | "ID" | "IDREF" | "ENTITY" => NcName,
        "NMTOKEN" => NmToken,
        "NMTOKENS" => NmTokens,
        "IDREFS" | "ENTITIES" => NcNames,
        "boolean" => Boolean,
        "decimal" => Decimal,
        "integer" => Integer {
            min: None,
            max: None,
        },
        "nonPositiveInteger" => Integer {
            min: None,
            max: Some("0"),
        },
        "negativeInteger" => Integer {
            min: None,
            max: Some("-1"),
        },
        "nonNegativeInteger" => Integer {
            min: Some("0"),
            max: None,
        },
        "positiveInteger" => Integer {
            min: Some("1"),
            max: None,
        },
        "long" => Integer {
            min: Some("-9223372036854775808"),
            max: Some("9223372036854775807"),
        },
        "int" => Integer {
            min: Some("-2147483648"),
            max: Some("2147483647"),
        },
        "short" => Integer {
            min: Some("-32768"),
            max: Some("32767"),
        },
        "byte" => Integer {
            min: Some("-128"),
            max: Some("127"),
        },
        "unsignedLong" => Integer {
            min: Some("0"),
            max: Some("18446744073709551615"),
        },
        "unsignedInt" => Integer {
            min: Some("0"),
            max: Some("4294967295"),
        },
        "unsignedShort" => Integer {
            min: Some("0"),
            max: Some("65535"),
        },
        "unsignedByte" => Integer {
            min: Some("0"),
            max: Some("255"),
        },
        "float" => Float,
        "double" => Double,
        "anyURI" => AnyUri,
        "QName" | "NOTATION" => QName,
        "dateTime" => DateTime,
        "date" => Date,
        "time" => Time,
        "gYear" => GYear,
        "gYearMonth" => GYearMonth,
        "gMonth" => GMonth,
        "gMonthDay" => GMonthDay,
        "gDay" => GDay,
        "duration" => Duration,
        "hexBinary" => HexBinary,
        "base64Binary" => Base64Binary,
        _ => return Err(format!("unknown XML Schema datatype '{ty}'")),
    })
}

fn ws_of(lib: &str, kind: Kind) -> Ws {
    match kind {
        Kind::String => Ws::Preserve,
        Kind::NormalizedString => Ws::Replace,
        // The built-in token is collapsed, like everything else.
        _ => {
            let _ = lib;
            Ws::Collapse
        }
    }
}

fn parse_len(name: &str, v: &str) -> Result<usize, String> {
    collapse(v)
        .parse::<usize>()
        .map_err(|_| format!("invalid value '{v}' for parameter '{name}'"))
}

impl DataType {
    /// Compile datatype `ty` of library `lib` with `params`.
    pub(crate) fn new(
        lib: &str,
        ty: &str,
        params: &[(String, String)],
    ) -> Result<DataType, String> {
        let kind = lookup(lib, ty)?;
        let mut dt = DataType {
            kind,
            ws: ws_of(lib, kind),
            length: None,
            min_length: None,
            max_length: None,
            patterns: Vec::new(),
            min_inclusive: None,
            max_inclusive: None,
            min_exclusive: None,
            max_exclusive: None,
            total_digits: None,
            fraction_digits: None,
        };
        if lib.is_empty() && !params.is_empty() {
            return Err(format!("the built-in datatype '{ty}' takes no parameters"));
        }
        let lengthy = !matches!(
            kind,
            Kind::Boolean
                | Kind::Decimal
                | Kind::Integer { .. }
                | Kind::Float
                | Kind::Double
                | Kind::DateTime
                | Kind::Date
                | Kind::Time
                | Kind::GYear
                | Kind::GYearMonth
                | Kind::GMonth
                | Kind::GMonthDay
                | Kind::GDay
                | Kind::Duration
        );
        let ordered = matches!(
            kind,
            Kind::Decimal
                | Kind::Integer { .. }
                | Kind::Float
                | Kind::Double
                | Kind::DateTime
                | Kind::Date
                | Kind::Time
                | Kind::GYear
                | Kind::GYearMonth
                | Kind::GMonth
                | Kind::GMonthDay
                | Kind::GDay
        );
        let digits = matches!(kind, Kind::Decimal | Kind::Integer { .. });
        let no_ctx = |_: &str| None;
        for (name, v) in params {
            let bad = || format!("parameter '{name}' is not allowed for datatype '{ty}'");
            match name.as_str() {
                "length" | "minLength" | "maxLength" if lengthy => {
                    let n = parse_len(name, v)?;
                    match name.as_str() {
                        "length" => dt.length = Some(n),
                        "minLength" => dt.min_length = Some(n),
                        _ => dt.max_length = Some(n),
                    }
                }
                "pattern" => dt.patterns.push(xsdregex::compile(v)?),
                "minInclusive" | "maxInclusive" | "minExclusive" | "maxExclusive" if ordered => {
                    let val = dt
                        .lexical_value(&normalize(dt.ws, v), &no_ctx)
                        .ok_or_else(|| format!("invalid value '{v}' for parameter '{name}'"))?;
                    match name.as_str() {
                        "minInclusive" => dt.min_inclusive = Some(val),
                        "maxInclusive" => dt.max_inclusive = Some(val),
                        "minExclusive" => dt.min_exclusive = Some(val),
                        _ => dt.max_exclusive = Some(val),
                    }
                }
                "totalDigits" | "fractionDigits" if digits => {
                    let n = parse_len(name, v)?;
                    if name == "totalDigits" {
                        if n == 0 {
                            return Err("totalDigits must be positive".into());
                        }
                        dt.total_digits = Some(n);
                    } else {
                        dt.fraction_digits = Some(n);
                    }
                }
                "whiteSpace" | "enumeration" => {
                    return Err(format!("parameter '{name}' is not allowed in RELAX NG"));
                }
                _ => return Err(bad()),
            }
        }
        Ok(dt)
    }

    /// Parse the (whitespace-normalised) lexical form `s`.
    fn lexical_value(&self, s: &str, ctx: Ctx<'_>) -> Option<Val> {
        use Kind::*;
        Some(match self.kind {
            String | NormalizedString | Token | AnyUri => Val::Str(s.to_string()),
            Language => {
                let mut parts = s.split('-');
                let first = parts.next()?;
                let ok = |p: &str, alnum: bool| {
                    (1..=8).contains(&p.len())
                        && p.chars()
                            .all(|c| c.is_ascii_alphabetic() || (alnum && c.is_ascii_digit()))
                };
                if !ok(first, false) || !parts.all(|p| ok(p, true)) {
                    return None;
                }
                Val::Str(s.to_string())
            }
            Name => is_name(s).then(|| Val::Str(s.to_string()))?,
            NcName => is_ncname(s).then(|| Val::Str(s.to_string()))?,
            NmToken => is_nmtoken(s).then(|| Val::Str(s.to_string()))?,
            NmTokens | NcNames => {
                let items: Vec<std::string::String> = s.split(' ').map(str::to_string).collect();
                let check = if self.kind == NmTokens {
                    is_nmtoken
                } else {
                    is_ncname
                };
                if s.is_empty() || !items.iter().all(|i| check(i)) {
                    return None;
                }
                Val::List(items)
            }
            Boolean => match s {
                "true" | "1" => Val::Bool(true),
                "false" | "0" => Val::Bool(false),
                _ => return None,
            },
            Decimal => Val::Dec(self::Decimal::parse(s, true)?),
            Integer { min, max } => {
                let d = self::Decimal::parse(s, false)?;
                if min.is_some_and(|m| {
                    d.cmp(&self::Decimal::parse(m, false).unwrap()) == Ordering::Less
                }) || max.is_some_and(|m| {
                    d.cmp(&self::Decimal::parse(m, false).unwrap()) == Ordering::Greater
                }) {
                    return None;
                }
                Val::Dec(d)
            }
            Float | Double => Val::Float(parse_float(s)?),
            QName => {
                let (prefix, local) = match s.split_once(':') {
                    Some((p, l)) => (p, l),
                    None => ("", s),
                };
                if (!prefix.is_empty() && !is_ncname(prefix)) || !is_ncname(local) {
                    return None;
                }
                let uri = match ctx(prefix) {
                    Some(u) => u,
                    None if prefix.is_empty() => std::string::String::new(),
                    None => return None,
                };
                Val::QName(uri, local.to_string())
            }
            DateTime | Date | Time | GYear | GYearMonth | GMonth | GMonthDay | GDay => {
                Val::Moment(Moment::parse(self.kind, s)?)
            }
            Duration => {
                let (m, sec) = parse_duration(s)?;
                Val::Duration(m, sec)
            }
            HexBinary => {
                if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                let bytes = (0..s.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                    .collect();
                Val::Bytes(bytes)
            }
            Base64Binary => Val::Bytes(parse_base64(s)?),
        })
    }

    fn length_of(&self, s: &str, v: &Val) -> usize {
        match v {
            Val::Bytes(b) => b.len(),
            Val::List(l) => l.len(),
            _ => s.chars().count(),
        }
    }

    /// Whether `text` is in this datatype (with its parameters).
    pub(crate) fn allows(&self, text: &str, ctx: Ctx<'_>) -> bool {
        self.check(text, ctx).is_some()
    }

    /// The value of `text`, if allowed.
    fn check(&self, text: &str, ctx: Ctx<'_>) -> Option<Val> {
        let s = normalize(self.ws, text);
        let v = self.lexical_value(&s, ctx)?;
        if !self.patterns.iter().all(|r| r.is_match(&s)) {
            return None;
        }
        if self.length.is_some() || self.min_length.is_some() || self.max_length.is_some() {
            let n = self.length_of(&s, &v);
            if self.length.is_some_and(|l| n != l)
                || self.min_length.is_some_and(|l| n < l)
                || self.max_length.is_some_and(|l| n > l)
            {
                return None;
            }
        }
        let cmp = |a: &Val, b: &Val| compare(a, b);
        if let Some(m) = &self.min_inclusive
            && !matches!(cmp(&v, m), Some(Ordering::Greater | Ordering::Equal))
        {
            return None;
        }
        if let Some(m) = &self.max_inclusive
            && !matches!(cmp(&v, m), Some(Ordering::Less | Ordering::Equal))
        {
            return None;
        }
        if let Some(m) = &self.min_exclusive
            && cmp(&v, m) != Some(Ordering::Greater)
        {
            return None;
        }
        if let Some(m) = &self.max_exclusive
            && cmp(&v, m) != Some(Ordering::Less)
        {
            return None;
        }
        if let Val::Dec(d) = &v
            && (self.total_digits.is_some_and(|t| d.total_digits() > t)
                || self.fraction_digits.is_some_and(|f| d.frac.len() > f))
        {
            return None;
        }
        Some(v)
    }

    /// Whether `text` (in context `ctx`) equals the value of `spec`.
    pub(crate) fn equal(spec: &ValueSpec, text: &str, ctx: Ctx<'_>) -> bool {
        match spec.dt.check(text, ctx) {
            Some(v) => values_equal(&spec.value, &v),
            None => false,
        }
    }
}

impl ValueSpec {
    pub(crate) fn new(
        lib: &str,
        ty: &str,
        value: &str,
        nsctx: &NsContext,
    ) -> Result<ValueSpec, String> {
        let dt = DataType::new(lib, ty, &[])?;
        let ctx = |p: &str| nsctx.resolve(p).map(str::to_string);
        let v = dt
            .check(value, &ctx)
            .ok_or_else(|| format!("'{value}' is not a valid value of datatype '{ty}'"))?;
        Ok(ValueSpec { dt, value: v })
    }
}

fn values_equal(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Float(x), Val::Float(y)) => x == y || (x.is_nan() && y.is_nan()),
        (Val::Moment(x), Val::Moment(y)) => x.cmp_partial(y) == Some(Ordering::Equal),
        (Val::Duration(m1, s1), Val::Duration(m2, s2)) => m1 == m2 && s1 == s2,
        _ => a == b,
    }
}

fn compare(a: &Val, b: &Val) -> Option<Ordering> {
    match (a, b) {
        (Val::Dec(x), Val::Dec(y)) => Some(x.cmp(y)),
        (Val::Float(x), Val::Float(y)) => x.partial_cmp(y),
        (Val::Moment(x), Val::Moment(y)) => x.cmp_partial(y),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Names (XML 1.0 fifth edition)

pub(crate) fn is_name_start_char(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

pub(crate) fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

fn is_name(s: &str) -> bool {
    let mut it = s.chars();
    it.next().is_some_and(is_name_start_char) && it.all(is_name_char)
}

fn is_ncname(s: &str) -> bool {
    is_name(s) && !s.contains(':')
}

fn is_nmtoken(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_name_char)
}

// ---------------------------------------------------------------------
// Numbers

/// An arbitrary-precision decimal: sign, integer digits without leading
/// zeros, fraction digits without trailing zeros. Zero is never negative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Decimal {
    neg: bool,
    int: String,
    frac: String,
}

impl Decimal {
    fn parse(s: &str, allow_point: bool) -> Option<Decimal> {
        let (neg, rest) = match s.as_bytes().first()? {
            b'-' => (true, &s[1..]),
            b'+' => (false, &s[1..]),
            _ => (false, s),
        };
        let (ip, fp) = match rest.split_once('.') {
            Some((i, f)) if allow_point => (i, f),
            Some(_) => return None,
            None => (rest, ""),
        };
        if ip.is_empty() && fp.is_empty() {
            return None;
        }
        if !ip.bytes().all(|b| b.is_ascii_digit()) || !fp.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let int = ip.trim_start_matches('0').to_string();
        let frac = fp.trim_end_matches('0').to_string();
        let zero = int.is_empty() && frac.is_empty();
        Some(Decimal {
            neg: neg && !zero,
            int,
            frac,
        })
    }

    fn cmp_abs(&self, o: &Decimal) -> Ordering {
        self.int
            .len()
            .cmp(&o.int.len())
            .then_with(|| self.int.cmp(&o.int))
            .then_with(|| self.frac.cmp(&o.frac))
    }

    fn total_digits(&self) -> usize {
        (self.int.len() + self.frac.len()).max(1)
    }
}

impl Ord for Decimal {
    fn cmp(&self, o: &Decimal) -> Ordering {
        match (self.neg, o.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => self.cmp_abs(o),
            (true, true) => o.cmp_abs(self),
        }
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, o: &Decimal) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn parse_float(s: &str) -> Option<f64> {
    match s {
        "INF" => return Some(f64::INFINITY),
        "-INF" => return Some(f64::NEG_INFINITY),
        "NaN" => return Some(f64::NAN),
        _ => {}
    }
    let (mant, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    Decimal::parse(mant, true)?;
    if let Some(e) = exp {
        let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    s.parse::<f64>().ok()
}

// ---------------------------------------------------------------------
// Dates and times

/// A point (or recurring point) in time: date/time fields, an optional
/// timezone offset in minutes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Moment {
    kind_tag: u8,
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    frac: String,
    tz: Option<i32>,
}

fn digits_n(s: &str, n: usize) -> Option<(u32, &str)> {
    if s.len() < n || !s.as_bytes()[..n].iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some((s[..n].parse().ok()?, &s[n..]))
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: Option<i64>, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => match y {
            Some(y) if !is_leap(y) => 28,
            _ => 29,
        },
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

impl Moment {
    fn parse(kind: Kind, s: &str) -> Option<Moment> {
        let mut m = Moment {
            kind_tag: match kind {
                Kind::DateTime => 0,
                Kind::Date => 1,
                Kind::Time => 2,
                Kind::GYear => 3,
                Kind::GYearMonth => 4,
                Kind::GMonth => 5,
                Kind::GMonthDay => 6,
                _ => 7,
            },
            year: 2000,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            frac: String::new(),
            tz: None,
        };
        let mut rest = s;
        let mut has_year = false;
        fn year(r: &str) -> Option<(i64, &str)> {
            let (neg, r) = match r.strip_prefix('-') {
                Some(r) => (true, r),
                None => (false, r),
            };
            let n = r.bytes().take_while(u8::is_ascii_digit).count();
            if n < 4 || (n > 4 && r.starts_with('0')) {
                return None;
            }
            let y: i64 = r[..n].parse().ok()?;
            if y == 0 {
                return None;
            }
            Some((if neg { -y } else { y }, &r[n..]))
        }
        match kind {
            Kind::DateTime | Kind::Date | Kind::GYear | Kind::GYearMonth => {
                let (y, r) = year(rest)?;
                m.year = y;
                has_year = true;
                rest = r;
                if kind != Kind::GYear {
                    let (mo, r) = digits_n(rest.strip_prefix('-')?, 2)?;
                    m.month = mo;
                    rest = r;
                    if kind != Kind::GYearMonth {
                        let (d, r) = digits_n(rest.strip_prefix('-')?, 2)?;
                        m.day = d;
                        rest = r;
                    }
                }
            }
            Kind::GMonth | Kind::GMonthDay => {
                let (mo, r) = digits_n(rest.strip_prefix("--")?, 2)?;
                m.month = mo;
                rest = r;
                if kind == Kind::GMonthDay {
                    let (d, r) = digits_n(rest.strip_prefix('-')?, 2)?;
                    m.day = d;
                    rest = r;
                } else if let Some(r) = rest.strip_prefix("--") {
                    rest = r;
                }
            }
            Kind::GDay => {
                let (d, r) = digits_n(rest.strip_prefix("---")?, 2)?;
                m.day = d;
                rest = r;
            }
            _ => {}
        }
        if kind == Kind::DateTime {
            rest = rest.strip_prefix('T')?;
        }
        if matches!(kind, Kind::DateTime | Kind::Time) {
            let (h, r) = digits_n(rest, 2)?;
            let (mi, r) = digits_n(r.strip_prefix(':')?, 2)?;
            let (se, r) = digits_n(r.strip_prefix(':')?, 2)?;
            rest = r;
            if let Some(r) = rest.strip_prefix('.') {
                let n = r.bytes().take_while(u8::is_ascii_digit).count();
                if n == 0 {
                    return None;
                }
                m.frac = r[..n].trim_end_matches('0').to_string();
                rest = &r[n..];
            }
            if mi > 59
                || se > 59
                || h > 24
                || (h == 24 && (mi != 0 || se != 0 || !m.frac.is_empty()))
            {
                return None;
            }
            m.hour = h;
            m.minute = mi;
            m.second = se;
        }
        if !rest.is_empty() {
            if rest == "Z" {
                m.tz = Some(0);
            } else {
                let sign = match rest.as_bytes()[0] {
                    b'+' => 1,
                    b'-' => -1,
                    _ => return None,
                };
                let (h, r) = digits_n(&rest[1..], 2)?;
                let (mi, r) = digits_n(r.strip_prefix(':')?, 2)?;
                if !r.is_empty() || mi > 59 || h > 14 || (h == 14 && mi != 0) {
                    return None;
                }
                m.tz = Some(sign * (h as i32 * 60 + mi as i32));
            }
        }
        if !(1..=12).contains(&m.month) {
            return None;
        }
        if m.day < 1 || m.day > days_in_month(has_year.then_some(m.year), m.month) {
            return None;
        }
        Some(m)
    }

    /// (days, seconds of day) after normalising to UTC (when `utc`).
    fn instant(&self, utc: bool) -> (i64, i64) {
        let days = days_from_civil(self.year, self.month, self.day);
        let mut secs =
            days * 86400 + self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64;
        if utc && let Some(tz) = self.tz {
            secs -= tz as i64 * 60;
        }
        (secs.div_euclid(86400), secs.rem_euclid(86400))
    }

    fn cmp_partial(&self, o: &Moment) -> Option<Ordering> {
        if self.kind_tag != o.kind_tag || self.tz.is_some() != o.tz.is_some() {
            return None;
        }
        let a = self.instant(true);
        let b = o.instant(true);
        Some(a.cmp(&b).then_with(|| self.frac.cmp(&o.frac)))
    }
}

/// `PnYnMnDTnHnMnS`: (months, seconds as a decimal), negated if `-`.
fn parse_duration(s: &str) -> Option<(i64, Decimal)> {
    let (neg, r) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let mut r = r.strip_prefix('P')?;
    let mut months: i64 = 0;
    let mut secs = Decimal::parse("0", true)?;
    let mut any = false;
    let mut in_time = false;
    let mut order = 0;
    let add = |a: &Decimal, b: &Decimal| Some(add_abs(a, b));
    while !r.is_empty() {
        if let Some(rest) = r.strip_prefix('T') {
            if in_time {
                return None;
            }
            in_time = true;
            r = rest;
            if r.is_empty() {
                return None;
            }
            continue;
        }
        let n = r
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b'.')
            .count();
        if n == 0 || n >= r.len() {
            return None;
        }
        let num = &r[..n];
        let unit = r.as_bytes()[n];
        let rank = match (in_time, unit) {
            (false, b'Y') => 1,
            (false, b'M') => 2,
            (false, b'D') => 3,
            (true, b'H') => 4,
            (true, b'M') => 5,
            (true, b'S') => 6,
            _ => return None,
        };
        if rank <= order || (num.contains('.') && rank != 6) {
            return None;
        }
        order = rank;
        let d = Decimal::parse(num, rank == 6)?;
        let whole: i64 = if d.int.is_empty() {
            0
        } else {
            d.int.parse().ok()?
        };
        match rank {
            1 => months += whole * 12,
            2 => months += whole,
            3 => secs = add(&secs, &Decimal::parse(&(whole * 86400).to_string(), false)?)?,
            4 => secs = add(&secs, &Decimal::parse(&(whole * 3600).to_string(), false)?)?,
            5 => secs = add(&secs, &Decimal::parse(&(whole * 60).to_string(), false)?)?,
            _ => secs = add(&secs, &d)?,
        }
        any = true;
        r = &r[n + 1..];
    }
    if !any {
        return None;
    }
    if neg {
        months = -months;
        if !(secs.int.is_empty() && secs.frac.is_empty()) {
            secs.neg = true;
        }
    }
    Some((months, secs))
}

/// `|a| + |b|` (both non-negative here).
fn add_abs(a: &Decimal, b: &Decimal) -> Decimal {
    let fl = a.frac.len().max(b.frac.len());
    let il = a.int.len().max(b.int.len());
    let digits = |d: &Decimal| -> Vec<u8> {
        let mut v = vec![0u8; il - d.int.len()];
        v.extend(d.int.bytes().map(|c| c - b'0'));
        v.extend(d.frac.bytes().map(|c| c - b'0'));
        v.resize(il + fl, 0);
        v
    };
    let (x, y) = (digits(a), digits(b));
    let mut out = vec![0u8; il + fl];
    let mut carry = 0;
    for i in (0..il + fl).rev() {
        let s = x[i] + y[i] + carry;
        out[i] = s % 10;
        carry = s / 10;
    }
    let mut text = String::new();
    if carry > 0 {
        text.push('1');
    }
    for (i, d) in out.iter().enumerate() {
        if i == il {
            text.push('.');
        }
        text.push((b'0' + d) as char);
    }
    if text.is_empty() || text.starts_with('.') {
        text.insert(0, '0');
    }
    Decimal::parse(&text, true).expect("well-formed decimal")
}

fn parse_base64(s: &str) -> Option<Vec<u8>> {
    let chars: Vec<u8> = s.bytes().filter(|b| *b != b' ').collect();
    if !chars.len().is_multiple_of(4) {
        return None;
    }
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a') as u32 + 26,
            b'0'..=b'9' => (c - b'0') as u32 + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let pad = chars.iter().rev().take_while(|c| **c == b'=').count();
    if pad > 2 {
        return None;
    }
    let data = &chars[..chars.len() - pad];
    if data.contains(&b'=') {
        return None;
    }
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &c in data {
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    // The bits left over before padding must be zero.
    if acc != 0 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(ty: &str, params: &[(&str, &str)]) -> DataType {
        let p: Vec<(String, String)> = params
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        DataType::new(XSD_DATATYPES, ty, &p).unwrap()
    }

    fn ok(d: &DataType, s: &str) -> bool {
        d.allows(s, &|_| None)
    }

    #[test]
    fn integers() {
        let d = dt("integer", &[]);
        assert!(ok(&d, " -0012 "));
        assert!(!ok(&d, "1.0"));
        assert!(!ok(&d, ""));
        assert!(!ok(&d, "+"));
        let b = dt("unsignedByte", &[]);
        assert!(ok(&b, "255") && !ok(&b, "256") && !ok(&b, "-1"));
        let i = dt("int", &[("minInclusive", "-5"), ("maxExclusive", "10")]);
        assert!(ok(&i, "-5") && ok(&i, "9") && !ok(&i, "10") && !ok(&i, "-6"));
    }

    #[test]
    fn decimals() {
        let d = dt("decimal", &[("totalDigits", "3"), ("fractionDigits", "1")]);
        assert!(ok(&d, "12.5") && ok(&d, "012.50") && !ok(&d, "1.25") && !ok(&d, "1234"));
        assert!(ok(&d, ".5") && ok(&d, "5.") && !ok(&d, "."));
    }

    #[test]
    fn strings() {
        let d = dt("string", &[("minLength", "1")]);
        assert!(ok(&d, " ") && !ok(&d, ""));
        let n = dt("normalizedString", &[("pattern", r"\S+")]);
        assert!(ok(&n, "ab") && !ok(&n, "a b") && !ok(&n, " a"));
        let t = dt("token", &[("maxLength", "3")]);
        assert!(ok(&t, "  abc  ") && !ok(&t, "abcd"));
        let p = dt("string", &[("pattern", r"\d+t?")]);
        assert!(ok(&p, "12t") && ok(&p, "3") && !ok(&p, "t") && !ok(&p, " 3"));
        assert!(ok(&dt("string", &[("pattern", "use.*")]), "useprefix"));
    }

    #[test]
    fn booleans_and_values() {
        let ctx = NsContext::default();
        let v = ValueSpec::new(XSD_DATATYPES, "boolean", "true", &ctx).unwrap();
        assert!(DataType::equal(&v, "1", &|_| None));
        assert!(DataType::equal(&v, " true ", &|_| None));
        assert!(!DataType::equal(&v, "0", &|_| None));
        let t = ValueSpec::new("", "token", " a  b ", &ctx).unwrap();
        assert!(DataType::equal(&t, "a b", &|_| None));
        let s = ValueSpec::new("", "string", "a", &ctx).unwrap();
        assert!(!DataType::equal(&s, " a", &|_| None));
        let i = ValueSpec::new(XSD_DATATYPES, "integer", "10", &ctx).unwrap();
        assert!(DataType::equal(&i, "+010", &|_| None));
    }

    #[test]
    fn names() {
        assert!(ok(&dt("NCName", &[]), "a-b.c"));
        assert!(!ok(&dt("NCName", &[]), "a:b"));
        assert!(ok(&dt("Name", &[]), "a:b"));
        assert!(!ok(&dt("Name", &[]), "1a"));
        assert!(ok(&dt("NMTOKENS", &[("length", "2")]), " 1a  b "));
        assert!(ok(&dt("language", &[]), "en-GB"));
        assert!(!ok(&dt("language", &[]), "englishlanguage"));
    }

    #[test]
    fn dates() {
        let d = dt("date", &[]);
        assert!(ok(&d, "2020-02-29") && !ok(&d, "2019-02-29") && !ok(&d, "2020-13-01"));
        assert!(ok(&d, "-0044-03-15Z") && !ok(&d, "0000-01-01"));
        let t = dt("dateTime", &[("minInclusive", "2000-01-01T00:00:00Z")]);
        assert!(ok(&t, "2000-01-01T01:00:00+01:00"));
        assert!(!ok(&t, "1999-12-31T23:59:59Z"));
        assert!(ok(&dt("gYear", &[]), "1999") && ok(&dt("gMonthDay", &[]), "--02-29"));
        assert!(ok(&dt("time", &[]), "24:00:00") && !ok(&dt("time", &[]), "24:00:01"));
    }

    #[test]
    fn misc() {
        assert!(ok(&dt("duration", &[]), "P1Y2M3DT4H5M6.5S"));
        assert!(!ok(&dt("duration", &[]), "P1YT"));
        assert!(!ok(&dt("duration", &[]), "P"));
        assert!(ok(&dt("hexBinary", &[("length", "2")]), "0aFF"));
        assert!(ok(&dt("base64Binary", &[("length", "3")]), "YWJj"));
        assert!(!ok(&dt("base64Binary", &[]), "YWJ"));
        assert!(ok(&dt("double", &[]), "-1.5E3") && ok(&dt("float", &[]), "INF"));
        assert!(!ok(&dt("double", &[]), "1e"));
        assert!(
            DataType::new(
                XSD_DATATYPES,
                "string",
                &[("whiteSpace".into(), "collapse".into())]
            )
            .is_err()
        );
        assert!(
            DataType::new(
                XSD_DATATYPES,
                "boolean",
                &[("minLength".into(), "1".into())]
            )
            .is_err()
        );
        assert!(DataType::new(XSD_DATATYPES, "nosuch", &[]).is_err());
    }
}

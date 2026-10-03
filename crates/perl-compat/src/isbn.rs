//! `Business::ISBN` 3.012 (with `Business::ISBN::Data` 20260218.001),
//! `Business::ISSN` 1.008 and `Business::ISMN` 1.205, as biber uses them.
//!
//! biber call sites:
//! * `bibtex.pm:1068-1100` (`_isbn` field handler): `Business::ISBN->new`,
//!   `is_valid` (invalid → warning, value kept), `--isbn13` → `as_isbn13->isbn`,
//!   `--isbn10` → `as_isbn10->isbn`, `--isbn-normalise` → `as_string`
//!   (hyphenated per the agency ranges);
//! * `Constants.pm:170-206` (datamodel `isbn`/`issn`/`ismn` datatypes, used by
//!   `--validate-datamodel`): the `isbn` check only requires `new` to succeed
//!   (BB-080: bad checksums pass); `issn` requires `is_valid`; `ismn`
//!   requires `is_valid` to be *perl-true*, and since `Business::ISMN`
//!   returns -1 (bad checksum) or -3 (bad publisher) for invalid ISMNs, every
//!   well-formed ISMN passes (see [`Ismn::is_valid`]).
//!
//! Range data: the vendored `data/isbn/RangeMessage.xml` (byte-identical to
//! biber 2.22's) is compiled in by `build.rs`. Like `Business::ISBN::Data`,
//! the `ISBN_RANGE_MESSAGE` environment variable names a replacement file; it
//! is read once, on first use. If it names a missing file, the built-in data
//! is used and [`range_data_warning`] returns `Business::ISBN::Data`'s carp
//! text.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::isbn_ranges::parse_range_message;

#[allow(clippy::type_complexity)]
mod built {
    include!(concat!(env!("OUT_DIR"), "/isbn_ranges.rs"));
}

/// `Business::ISBN` validity codes.
pub mod code {
    /// `ARTICLE_CODE_OUT_OF_RANGE`
    pub const ARTICLE_CODE_OUT_OF_RANGE: i32 = -5;
    /// `INVALID_PREFIX`
    pub const INVALID_PREFIX: i32 = -4;
    /// `INVALID_PUBLISHER_CODE`
    pub const INVALID_PUBLISHER_CODE: i32 = -3;
    /// `INVALID_GROUP_CODE`
    pub const INVALID_GROUP_CODE: i32 = -2;
    /// `BAD_CHECKSUM`
    pub const BAD_CHECKSUM: i32 = -1;
    /// `BAD_ISBN`
    pub const BAD_ISBN: i32 = 0;
    /// `GOOD_ISBN`
    pub const GOOD_ISBN: i32 = 1;
}
use code::*;

/// ISBN range data: `{978,979} → group → publisher ranges`.
#[derive(Debug)]
pub struct RangeData {
    groups: HashMap<(String, String), Vec<(String, String)>>,
    agencies: HashMap<(String, String), String>,
    max_group_len: usize,
    /// Where the data came from (`"built-in"` or the file path).
    pub source: String,
}

impl RangeData {
    fn from_groups<I: IntoIterator<Item = (String, String, String, Vec<(String, String)>)>>(
        it: I,
        source: String,
    ) -> RangeData {
        let mut groups = HashMap::new();
        let mut agencies = HashMap::new();
        for (ean, g, agency, ranges) in it {
            groups.insert((ean.clone(), g.clone()), ranges);
            agencies.insert((ean, g), agency);
        }
        // MAX_COUNTRY_CODE_LENGTH: length of the numerically largest key.
        let max_group_len = groups
            .keys()
            .max_by(|a, b| {
                let x: f64 = a.1.parse().unwrap_or(0.0);
                let y: f64 = b.1.parse().unwrap_or(0.0);
                x.total_cmp(&y)
            })
            .map_or(0, |k| k.1.len());
        RangeData {
            groups,
            agencies,
            max_group_len,
            source,
        }
    }

    /// The compiled-in data.
    pub fn builtin() -> RangeData {
        RangeData::from_groups(
            built::GROUPS.iter().map(|(e, g, a, r)| {
                (
                    e.to_string(),
                    g.to_string(),
                    a.to_string(),
                    r.iter()
                        .map(|(l, h)| (l.to_string(), h.to_string()))
                        .collect(),
                )
            }),
            "built-in".to_string(),
        )
    }

    /// Parse a `RangeMessage.xml` document.
    pub fn parse(xml: &str, source: &str) -> RangeData {
        RangeData::from_groups(
            parse_range_message(xml)
                .into_iter()
                .map(|g| (g.ean_prefix, g.group, g.agency, g.ranges)),
            source.to_string(),
        )
    }

    fn group(&self, ean: &str, g: &str) -> Option<&Vec<(String, String)>> {
        self.groups.get(&(ean.to_string(), g.to_string()))
    }

    /// The `MessageDate` of the built-in file.
    pub fn builtin_date() -> &'static str {
        built::MESSAGE_DATE
    }
}

static DATA: OnceLock<(RangeData, Option<String>)> = OnceLock::new();

fn global() -> &'static (RangeData, Option<String>) {
    DATA.get_or_init(|| {
        if let Some(p) = std::env::var_os("ISBN_RANGE_MESSAGE") {
            let path = std::path::PathBuf::from(&p);
            if path.exists() {
                // `open '<:utf8'`; unreadable → carp and use the defaults.
                if let Ok(bytes) = std::fs::read(&path) {
                    let text = String::from_utf8_lossy(&bytes);
                    return (RangeData::parse(&text, &path.display().to_string()), None);
                }
            } else {
                return (
                    RangeData::builtin(),
                    Some(format!(
                        "ISBN_RANGE_MESSAGE is set to [{}] but that file does not exist!\nTrying to use the default locations\n",
                        path.display()
                    )),
                );
            }
        }
        (RangeData::builtin(), None)
    })
}

/// The range data in use (built-in unless `ISBN_RANGE_MESSAGE` overrides).
pub fn range_data() -> &'static RangeData {
    &global().0
}

/// `Business::ISBN::Data`'s warning when `ISBN_RANGE_MESSAGE` names a
/// missing file.
pub fn range_data_warning() -> Option<&'static str> {
    global().1.as_deref()
}

/// ISBN-10 or ISBN-13.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsbnType {
    /// `Business::ISBN10`
    Isbn10,
    /// `Business::ISBN13`
    Isbn13,
}

/// A parsed ISBN (`Business::ISBN10`/`Business::ISBN13` object).
#[derive(Debug, Clone)]
pub struct Isbn<'d> {
    data: &'d RangeData,
    input: String,
    isbn: String,
    kind: IsbnType,
    prefix: Option<String>,
    group_code: Option<String>,
    publisher_code: Option<String>,
    article_code: Option<String>,
    checksum: Option<String>,
    valid: i32,
}

/// `_common_format`: `uc`, strip all but `[0-9X]`, require
/// `\A(?:\d{3})?\d{9}[0-9X]\z`.
fn common_format(input: &str) -> Option<String> {
    let up = crate::case::uc(input);
    let d: String = up
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X')
        .collect();
    let n = d.len();
    if (n == 10 || n == 13) && d[..n - 1].bytes().all(|b| b.is_ascii_digit()) {
        Some(d)
    } else {
        None
    }
}

fn checksum10(data: &str) -> String {
    let sum: u32 = data
        .bytes()
        .take(9)
        .zip((2..=10).rev())
        .map(|(b, w)| digit(b) * w)
        .sum();
    let c = (11 - (sum % 11)) % 11;
    if c == 10 {
        "X".to_string()
    } else {
        c.to_string()
    }
}

fn checksum13(data: &str) -> String {
    let b = data.as_bytes();
    let mut sum = 0u32;
    for i in [0, 2, 4, 6, 8, 10] {
        sum += digit(b[i]) + 3 * digit(b[i + 1]);
    }
    ((10 * (sum / 10 + 1) - sum) % 10).to_string()
}

/// Perl numifies `"X"` (and anything else non-numeric) to 0.
fn digit(b: u8) -> u32 {
    if b.is_ascii_digit() {
        u32::from(b - b'0')
    } else {
        0
    }
}

impl<'d> Isbn<'d> {
    /// `Business::ISBN->new($input)` with the global range data; `None` is
    /// perl's `undef` (input not of ISBN shape).
    pub fn new(input: &str) -> Option<Isbn<'static>> {
        Isbn::new_with(range_data(), input)
    }

    /// `Business::ISBN->new` with explicit range data.
    pub fn new_with(data: &'d RangeData, input: &str) -> Option<Isbn<'d>> {
        let common = common_format(input)?;
        let kind = if common.len() == 10 {
            IsbnType::Isbn10
        } else {
            IsbnType::Isbn13
        };
        let mut me = Isbn {
            data,
            input: input.to_string(),
            isbn: common,
            kind,
            prefix: None,
            group_code: None,
            publisher_code: None,
            article_code: None,
            checksum: None,
            valid: INVALID_GROUP_CODE,
        };
        me.parse_isbn();
        Some(me)
    }

    fn max_length(&self) -> usize {
        match self.kind {
            IsbnType::Isbn10 => 10,
            IsbnType::Isbn13 => 13,
        }
    }

    fn prefix_len(&self) -> usize {
        self.prefix.as_deref().map_or(0, str::len)
    }

    fn group_len(&self) -> usize {
        self.group_code.as_deref().map_or(0, str::len)
    }

    fn publisher_len(&self) -> usize {
        self.publisher_code.as_deref().map_or(0, str::len)
    }

    fn data_prefix(&self) -> &str {
        match self.kind {
            IsbnType::Isbn10 => "978",
            IsbnType::Isbn13 => self.prefix.as_deref().unwrap_or(""),
        }
    }

    fn substr(&self, off: usize, len: usize) -> &str {
        let s = &self.isbn;
        if off > s.len() {
            return "";
        }
        &s[off..(off + len).min(s.len())]
    }

    fn parse_isbn(&mut self) {
        // prefix
        let prefix = match self.kind {
            IsbnType::Isbn10 => Some(String::new()),
            IsbnType::Isbn13 => {
                let p = &self.isbn[..3];
                (p == "978" || p == "979").then(|| p.to_string())
            }
        };
        let Some(prefix) = prefix else {
            self.valid = INVALID_PREFIX;
            return;
        };
        self.prefix = Some(prefix);
        // group code
        let pl = self.prefix_len();
        let mut group = None;
        for count in 1..=self.data.max_group_len.max(1) {
            let trial = self.substr(pl, count);
            if self.data.group(self.data_prefix(), trial).is_some() {
                group = Some(trial.to_string());
                break;
            }
        }
        let Some(group) = group else {
            self.valid = INVALID_GROUP_CODE;
            return;
        };
        self.group_code = Some(group);
        // publisher code
        let gl = self.group_len();
        let max_pub = self.max_length().saturating_sub(pl + gl + 2);
        let longest = self.substr(pl + gl, max_pub).to_string();
        let ranges = self
            .data
            .group(self.data_prefix(), self.group_code.as_deref().unwrap_or(""))
            .cloned()
            .unwrap_or_default();
        let mut publisher = None;
        for (lo, hi) in &ranges {
            let trial: String = longest.chars().take(lo.chars().count()).collect();
            if trial.as_str() >= lo.as_str() && trial.as_str() <= hi.as_str() {
                publisher = Some(trial);
                break;
            }
        }
        let Some(publisher) = publisher else {
            self.valid = INVALID_PUBLISHER_CODE;
            return;
        };
        self.publisher_code = Some(publisher);
        // article code and checksum
        let head = pl + gl + self.publisher_len();
        let alen = self.isbn.len().saturating_sub(head + 1);
        self.article_code = Some(self.substr(head, alen).to_string());
        self.checksum = Some(self.isbn[self.isbn.len() - 1..].to_string());
        self.valid = self.is_valid_checksum();
    }

    fn compute_checksum(&self) -> String {
        match self.kind {
            IsbnType::Isbn10 => checksum10(&self.isbn),
            IsbnType::Isbn13 => checksum13(&self.isbn),
        }
    }

    /// `is_valid_checksum`: [`code::GOOD_ISBN`] or [`code::BAD_CHECKSUM`].
    pub fn is_valid_checksum(&self) -> i32 {
        if self.checksum.as_deref().unwrap_or("") == self.compute_checksum() {
            GOOD_ISBN
        } else {
            BAD_CHECKSUM
        }
    }

    /// `is_valid`.
    pub fn is_valid(&self) -> bool {
        self.valid == GOOD_ISBN
    }

    /// The validity code (`$isbn->{valid}`, see [`code`]).
    pub fn validity(&self) -> i32 {
        self.valid
    }

    /// `error_text`.
    pub fn error_text(&self) -> &'static str {
        match self.valid {
            0 => "Bad ISBN",
            1 => "Good ISBN",
            -1 => "Bad ISBN checksum",
            -2 => "Invalid group code",
            -3 => "Invalid publisher code",
            -4 => "Invalid prefix (must be 978 or 979)",
            -5 => "Incremented article code would be out of range",
            _ => "",
        }
    }

    /// `isbn`: the digits (and final `X`), no hyphens.
    pub fn isbn(&self) -> &str {
        &self.isbn
    }

    /// `input_isbn`.
    pub fn input_isbn(&self) -> &str {
        &self.input
    }

    /// `type`.
    pub fn kind(&self) -> IsbnType {
        self.kind
    }

    /// `prefix` (`""` for ISBN-10).
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    /// `group_code`.
    pub fn group_code(&self) -> Option<&str> {
        self.group_code.as_deref()
    }

    /// `group`: the agency name.
    pub fn group(&self) -> Option<&str> {
        let g = self.group_code.as_deref()?;
        self.data
            .agencies
            .get(&(self.data_prefix().to_string(), g.to_string()))
            .map(String::as_str)
    }

    /// `publisher_code`.
    pub fn publisher_code(&self) -> Option<&str> {
        self.publisher_code.as_deref()
    }

    /// `article_code`.
    pub fn article_code(&self) -> Option<&str> {
        self.article_code.as_deref()
    }

    /// `checksum`.
    pub fn checksum(&self) -> Option<&str> {
        self.checksum.as_deref()
    }

    /// `fix_checksum`: returns whether the checksum changed.
    pub fn fix_checksum(&mut self) -> bool {
        let last = self.isbn[self.isbn.len() - 1..].to_string();
        let c = self.compute_checksum();
        self.isbn.truncate(self.isbn.len() - 1);
        self.isbn.push_str(&c);
        self.checksum = Some(c.clone());
        // _check_validity
        if self.is_valid_checksum() == GOOD_ISBN
            && self.group_code.is_some()
            && self.publisher_code.is_some()
            && self.prefix.is_some()
        {
            self.valid = GOOD_ISBN;
        } else {
            if self.publisher_code.is_none() {
                self.valid = INVALID_PUBLISHER_CODE;
            }
            if self.group_code.is_none() {
                self.valid = INVALID_GROUP_CODE;
            }
            if self.prefix.is_none() {
                self.valid = INVALID_PREFIX;
            }
        }
        last != c
    }

    fn hyphen_positions(&self) -> Vec<usize> {
        let (pl, gl, publ) = (self.prefix_len(), self.group_len(), self.publisher_len());
        match self.kind {
            IsbnType::Isbn10 => vec![gl, gl + publ, 9],
            IsbnType::Isbn13 => vec![pl, pl + gl, pl + gl + publ, self.isbn.len() - 1],
        }
    }

    /// `as_string`: hyphenated form; `None` (undef) unless valid.
    pub fn as_string(&self) -> Option<String> {
        self.as_string_with(&self.hyphen_positions())
    }

    /// `as_string(\@positions)`.
    pub fn as_string_with(&self, positions: &[usize]) -> Option<String> {
        if !self.is_valid() {
            return None;
        }
        let mut p: Vec<usize> = positions.to_vec();
        p.sort_unstable_by(|a, b| b.cmp(a));
        let mut s = self.isbn.clone();
        for pos in p {
            if !(1..=12).contains(&pos) || pos > s.len() {
                continue;
            }
            s.insert(pos, '-');
        }
        Some(s)
    }

    /// `as_isbn13`. For an ISBN-13 this re-parses `as_string`, which is
    /// `undef` for an invalid ISBN (perl then dies calling `fix_checksum` on
    /// undef); that case returns `None`.
    pub fn as_isbn13(&self) -> Option<Isbn<'d>> {
        let src = match self.kind {
            IsbnType::Isbn10 => format!("978{}", self.isbn),
            IsbnType::Isbn13 => self.as_string()?,
        };
        let mut n = Isbn::new_with(self.data, &src)?;
        n.fix_checksum();
        Some(n)
    }

    /// `as_isbn10`: `None` (undef) for a 979 ISBN-13. (biber then dies with
    /// "Can't call method "isbn" on an undefined value" under `--isbn10`.)
    pub fn as_isbn10(&self) -> Option<Isbn<'d>> {
        let src = match self.kind {
            IsbnType::Isbn10 => self.isbn.clone(),
            IsbnType::Isbn13 => {
                if self.prefix.as_deref() != Some("978") {
                    return None;
                }
                self.isbn[3..].to_string()
            }
        };
        let mut n = Isbn::new_with(self.data, &src)?;
        n.fix_checksum();
        Some(n)
    }
}

// ---------------------------------------------------------------------------
// ISSN

/// A `Business::ISSN` object.
#[derive(Debug, Clone)]
pub struct Issn {
    issn: String,
    valid: bool,
}

fn issn_common(input: &str) -> Option<String> {
    let up = crate::case::uc(input);
    let d: String = up
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X')
        .collect();
    (d.len() == 8 && d[..7].bytes().all(|b| b.is_ascii_digit())).then_some(d)
}

fn issn_checksum(d: &str) -> String {
    let sum: u32 = d
        .bytes()
        .take(7)
        .zip((2..=8).rev())
        .map(|(b, w)| digit(b) * w)
        .sum();
    let c = (11 - (sum % 11)) % 11;
    if c == 10 { "X".into() } else { c.to_string() }
}

impl Issn {
    /// `Business::ISSN->new`; `None` if not of ISSN shape.
    pub fn new(input: &str) -> Option<Issn> {
        let issn = issn_common(input)?;
        let valid = issn[7..] == issn_checksum(&issn);
        Some(Issn { issn, valid })
    }

    /// `is_valid`.
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// The 8 characters.
    pub fn issn(&self) -> &str {
        &self.issn
    }

    /// `as_string` (`NNNN-NNNC`), `None` unless valid.
    pub fn as_string(&self) -> Option<String> {
        self.valid
            .then(|| format!("{}-{}", &self.issn[..4], &self.issn[4..]))
    }

    /// `fix_checksum`: whether it changed.
    pub fn fix_checksum(&mut self) -> bool {
        let c = issn_checksum(&self.issn);
        let changed = self.issn[7..] != c;
        self.issn.truncate(7);
        self.issn.push_str(&c);
        self.valid = true;
        changed
    }
}

// ---------------------------------------------------------------------------
// ISMN

/// `Business::ISMN` publisher ranges (`Business/ISMN/Data.pm` 1.205), as
/// `(country, low, high)`; `None` high is perl's `undef`.
static ISMN_TUPLES: &[(&str, &str, Option<&str>)] = &[
    ("AU", "075", None),
    ("AU", "2700", Some("2701")),
    ("AU", "67300", Some("67310")),
    ("AU", "720000", Some("720200")),
    ("AU", "9009500", Some("9009799")),
    ("CA", "53001", Some("53010")),
    ("CA", "706001", Some("706050")),
    ("CA", "9001301", Some("9001600")),
    ("CY", "720300", Some("720320")),
    ("CY", "9009800", Some("9009850")),
    ("CZ", "2600", Some("2605")),
    ("CZ", "66050", Some("66100")),
    ("CZ", "706500", Some("706600")),
    ("CZ", "9004000", Some("9005000")),
    ("DE", "000", Some("025")),
    ("DE", "2000", Some("2150")),
    ("DE", "50000", Some("52000")),
    ("DE", "700000", Some("705000")),
    ("DE", "9000000", Some("9001000")),
    ("DK", "66130", Some("66140")),
    ("DK", "706750", Some("706800")),
    ("DK", "9001801", Some("9001900")),
    ("ES", "3500", Some("3507")),
    ("ES", "69200", Some("69224")),
    ("ES", "801200", Some("801219")),
    ("ES", "9013100", Some("9013119")),
    ("FI", "042", None),
    ("FI", "55001", Some("55020")),
    ("FI", "706300", Some("706349")),
    ("FI", "9001601", Some("9001700")),
    ("FR", "043", Some("048")),
    ("FR", "2301", Some("2400")),
    ("FR", "56000", Some("56500")),
    ("GB", "050", Some("060")),
    ("GB", "2201", Some("2300")),
    ("GB", "57000", Some("57999")),
    ("GB", "708001", Some("709000")),
    ("GB", "9002000", Some("9002999")),
    ("GH", "9008000", Some("9008100")),
    ("GR", "69150", Some("69159")),
    ("GR", "801150", Some("801179")),
    ("GR", "9013030", Some("9013079")),
    ("HR", "706700", Some("706710")),
    ("HR", "9005201", Some("9005250")),
    ("HU", "55030", None),
    ("HU", "706251", Some("706260")),
    ("HU", "9005251", Some("9005260")),
    ("HU", "080", None),
    ("IT", "039", Some("041")),
    ("IT", "2151", Some("2200")),
    ("IT", "52001", Some("53000")),
    ("IT", "705001", Some("706000")),
    ("IT", "9001001", Some("9001300")),
    ("JP", "65001", None),
    ("LK", "710000", Some("710100")),
    ("LK", "9006400", Some("9006500")),
    ("LT", "59995", Some("59999")),
    ("LT", "706201", Some("706250")),
    ("LV", "706650", Some("706660")),
    ("LV", "9005001", Some("9005100")),
    ("NO", "065", None),
    ("NO", "2610", Some("2617")),
    ("NO", "66101", Some("66115")),
    ("NO", "706680", Some("706690")),
    ("NO", "9005101", Some("9005200")),
    ("PT", "3000", None),
    ("PT", "707700", Some("707750")),
    ("PT", "9007500", Some("9007600")),
    ("RU", "66000", Some("66055")),
    ("SE", "070", None),
    ("SE", "66150", Some("66170")),
    ("SE", "706850", Some("706900")),
    ("SE", "9006000", Some("9006200")),
    ("SI", "709001", Some("709100")),
    ("SI", "9009000", Some("9009100")),
    ("SK", "68500", Some("68520")),
    ("SK", "9010001", Some("9010050")),
    ("TR", "036", Some("037")),
    ("TR", "3440", Some("3445")),
    ("TR", "69080", Some("69095")),
    ("TR", "801100", Some("801120")),
    ("TR", "9013000", Some("9013010")),
    ("UA", "707500", Some("707600")),
    ("UA", "9007000", Some("9007200")),
    ("US", "081", Some("099")),
    ("US", "3005", Some("3205")),
    ("US", "58000", Some("58199")),
    ("US", "60001", Some("60002")),
    ("US", "800000", Some("800199")),
    ("US", "9012000", Some("9012099")),
];

/// `Business::ISMN` validity codes: 1 good, -1 bad checksum, -3 invalid
/// publisher (country) code.
pub mod ismn_code {
    /// `GOOD_ISMN`
    pub const GOOD_ISMN: i32 = 1;
    /// `BAD_CHECKSUM`
    pub const BAD_CHECKSUM: i32 = -1;
    /// `INVALID_PUBLISHER_CODE`
    pub const INVALID_PUBLISHER_CODE: i32 = -3;
}

/// A `Business::ISMN` object.
#[derive(Debug, Clone)]
pub struct Ismn {
    ismn: String,
    publisher_code: String,
    country: Option<&'static str>,
    valid: i32,
    code_end: usize,
}

fn ismn_common(input: &str) -> Option<String> {
    let up = crate::case::uc(input);
    let d: String = up
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'M')
        .collect();
    (d.len() == 10 && d.starts_with('M') && d[1..].bytes().all(|b| b.is_ascii_digit())).then_some(d)
}

fn ismn_checksum(d: &str) -> u32 {
    let mut sum = 9u32;
    for (i, b) in d.bytes().skip(1).take(8).enumerate() {
        sum += digit(b) * if i % 2 == 0 { 1 } else { 3 };
    }
    (10 - (sum % 10)) % 10
}

fn ismn_valid_checksum(d: &str) -> i32 {
    if d[9..] == ismn_checksum(d).to_string() {
        ismn_code::GOOD_ISMN
    } else {
        ismn_code::BAD_CHECKSUM
    }
}

impl Ismn {
    /// `Business::ISMN->new`; `None` if not of ISMN shape (`M` + 9 digits
    /// after removing everything but digits and `M`).
    pub fn new(input: &str) -> Option<Ismn> {
        let ismn = ismn_common(input)?;
        let len = match ismn.as_bytes()[1] {
            b'0' => 3,
            b'1'..=b'3' => 4,
            b'4'..=b'6' => 5,
            b'7' | b'8' => 6,
            _ => 7,
        };
        let publisher_code = ismn[1..1 + len].to_string();
        let mut me = Ismn {
            ismn,
            publisher_code,
            country: None,
            valid: ismn_code::INVALID_PUBLISHER_CODE,
            code_end: len + 1,
        };
        if !me.is_valid_country_code() {
            return Some(me);
        }
        me.valid = ismn_valid_checksum(&me.ismn);
        Some(me)
    }

    fn is_valid_country_code(&mut self) -> bool {
        // perl compares numerically; `undef` upper bounds are 0.
        let code: u64 = self.publisher_code.parse().unwrap_or(0);
        let mut tuples: Vec<(&str, u64, Option<u64>)> = ISMN_TUPLES
            .iter()
            .map(|(c, lo, hi)| {
                (
                    *c,
                    lo.parse().unwrap_or(0),
                    hi.map(|h| h.parse().unwrap_or(0)),
                )
            })
            .collect();
        tuples.sort_by_key(|t| t.1);
        for (country, lo, hi) in tuples {
            if let Some(h) = hi {
                if code > h {
                    continue;
                }
            }
            if code < lo {
                break;
            }
            if code >= lo && code <= hi.unwrap_or(0) {
                self.country = Some(country);
                return true;
            }
        }
        false
    }

    /// `is_valid`: the validity code. biber tests it for perl truth, so
    /// `-1`/`-3` count as valid there; use [`Ismn::is_valid_perl_true`].
    pub fn is_valid(&self) -> i32 {
        self.valid
    }

    /// `$ismn->is_valid` in boolean context (always true for objects).
    pub fn is_valid_perl_true(&self) -> bool {
        self.valid != 0
    }

    /// `ismn`.
    pub fn ismn(&self) -> &str {
        &self.ismn
    }

    /// `country`.
    pub fn country(&self) -> Option<&'static str> {
        self.country
    }

    /// `as_string`: hyphenated (`M-ppp-aaaaa-c`), `None` unless valid.
    pub fn as_string(&self) -> Option<String> {
        if self.valid != ismn_code::GOOD_ISMN {
            return None;
        }
        let mut s = self.ismn.clone();
        for pos in [9, self.code_end, 1] {
            s.insert(pos, '-');
        }
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isbn() {
        let i = Isbn::new("0-306-40615-2").unwrap();
        assert!(i.is_valid());
        assert_eq!(i.as_string().as_deref(), Some("0-306-40615-2"));
        let i13 = i.as_isbn13().unwrap();
        assert_eq!(i13.isbn(), "9780306406157");
        assert_eq!(i13.as_string().as_deref(), Some("978-0-306-40615-7"));
        assert!(Isbn::new("abc").is_none());
        let bad = Isbn::new("0306406153").unwrap();
        assert!(!bad.is_valid());
        assert_eq!(bad.validity(), code::BAD_CHECKSUM);
        assert!(Isbn::new("9791234567896").unwrap().as_isbn10().is_none());
    }

    #[test]
    fn issn_ismn() {
        assert!(Issn::new("0317-8471").unwrap().is_valid());
        assert!(!Issn::new("0317-8472").unwrap().is_valid());
        let m = Ismn::new("M-2306-7118-7").unwrap();
        assert_eq!(m.is_valid(), 1);
        assert_eq!(m.as_string().as_deref(), Some("M-2306-7118-7"));
        assert_eq!(Ismn::new("M-2306-7118-8").unwrap().is_valid(), -1);
    }
}

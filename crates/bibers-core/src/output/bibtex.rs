//! The bibtex writer (`Biber::Output::bibtex`, bibtex.pm, with the parts of
//! `Biber::Output::base` it inherits): tool-mode output (`--tool`) and
//! `--output-format=bibtex` from a `.bcf`.
//!
//! Every entry of the pseudo-section `99999` is written as a `.bib` entry
//! (`set_output_entry`, bibtex.pm:121), formatted by the `--output-*`
//! options: field name case and replacement, indentation, alignment, field
//! order, name and list separators, extended name format, legacy dates,
//! XDATA reference markers, annotation markers, line wrapping, macros and
//! comments, LaTeX encoding of the output. Entries are output in the order
//! of the section's global data list (bibtex.pm:459-470), preceded by the
//! used `@string` macros and followed by the `@comment`s of the data.
//!
//! Reproduced biber behaviour (docs/biber-bugs.md):
//! - values equal to `0` are dropped, as are year-0 dates and midnight
//!   times (BB-056, [`Quirk::ZeroIsEmpty`]);
//! - the `dates` class of `--output-field-order` matches the stripped,
//!   mutated date field list (BB-028, [`Quirk::DatefieldHelperMutation`]);
//! - the `winterS` year division is lost (BB-047,
//!   [`Quirk::BibtexWinterSTypo`]);
//! - with a non-UTF-8 output encoding, any literal `?` forces LaTeX
//!   recoding of the entry (BB-048, [`Quirk::BibtexEncodingQuestionMark`]);
//! - `--output-safechars` recodes the whole entry, keys and verbatim fields
//!   included (BB-046).
//!
//! The order of the `OPTIONS` field's options (Perl hash order in biber) is
//! sorted by option name (DC-054).

use std::collections::{BTreeMap, HashMap};

use perl_compat::grapheme::gcstring;
use perl_compat::regex::PerlRegex;
use perl_compat::textwrap;
use perl_compat::truth::PerlNum;
use perl_compat::unicode::{is_perl_space, nfc, nfd};
use perl_compat::{fc, lc, perl_truthy, uc, ucfirst};
use serde_json::Value as J;

use crate::biber_error;
use crate::config::blx::{OptValue, map_boolean_tostring_raw};
use crate::log::Fatal;
use crate::model::{Entry, Name, NameListId, SectionState, Value};
use crate::pipeline::{Ctx, Processed, resolve};
use crate::quirks::Quirk;

/// The pseudo-section bibtex output is written from.
pub const SECTION: &str = "99999";

/// `--output-fieldcase` (DC-007: accepted case-insensitively).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldCase {
    Upper,
    Lower,
    Title,
}

impl FieldCase {
    fn apply(self, s: &str) -> String {
        match self {
            FieldCase::Upper => uc(s),
            FieldCase::Lower => lc(s),
            FieldCase::Title => ucfirst(s),
        }
    }
}

/// Option values used while writing, looked up once.
struct Opts {
    case: FieldCase,
    field_replace: BTreeMap<String, String>,
    xname: bool,
    namesep: String,
    listsep: String,
    resolve_xdata: bool,
    resolve_crossrefs: bool,
    legacy_dates: bool,
    nostdmacros: bool,
    ann_marker: String,
    named_ann_marker: String,
    /// The *input* annotation markers (`strip_annotation`).
    in_ann_marker: String,
    align: bool,
    field_order: String,
    indent: String,
    wraplines: i64,
    xnamesep_out: String,
    xdata: XdataRefs,
    zero_is_empty: bool,
    all_macrodefs: bool,
    no_macrodefs: bool,
}

/// The regexes of `xdatarefout`/`xdatarefcheck` (Utils.pm:1955-1984).
struct XdataRefs {
    /// `m/^\s*$xdmi(?=$xnsi)/`
    check: Option<PerlRegex>,
    /// `s/^$xdmi$xnsi//x` (implicit marker; unused by bibtex output)
    /// `s/^$xdmi(?=$xnsi)/$xdmo/x`
    marker: Option<PerlRegex>,
    /// `s/$xnsi/$xnso/xg`
    namesep: Option<PerlRegex>,
    /// `s/$xdsi/$xdso/xg`
    datasep: Option<PerlRegex>,
    xdmo: String,
    xnso: String,
    xdso: String,
}

impl XdataRefs {
    fn new(ctx: &Ctx) -> XdataRefs {
        let b = &ctx.cfg.biber;
        let s = |o: &str| b.get_str(o).unwrap_or("").to_string();
        let (xdmi, xnsi, xdsi) = (s("xdatamarker"), s("xnamesep"), s("xdatasep"));
        let re = |p: String| PerlRegex::new(&p, false).ok();
        XdataRefs {
            check: re(format!(r"^\s*{xdmi}(?={xnsi})")),
            marker: re(format!("(?x)^{xdmi}(?={xnsi})")),
            namesep: re(format!("(?x){xnsi}")),
            datasep: re(format!("(?x){xdsi}")),
            xdmo: s("output_xdatamarker"),
            xnso: s("output_xnamesep"),
            xdso: s("output_xdatasep"),
        }
    }

    /// `xdatarefout($xdataref)` (without the implicit marker).
    fn out(&self, xdataref: &str) -> String {
        let mut s = xdataref.to_string();
        if let Some(re) = &self.marker
            && let Ok(ms) = re.matches(&s)
            && let Some((a, b)) = ms.first().and_then(|g| g[0])
        {
            s = format!("{}{}{}", &s[..a], self.xdmo, &s[b..]);
        }
        if let Some(re) = &self.namesep {
            s = re.replace_all_with(&s, |_| self.xnso.clone()).unwrap_or(s);
        }
        if let Some(re) = &self.datasep {
            s = re.replace_all_with(&s, |_| self.xdso.clone()).unwrap_or(s);
        }
        s
    }

    /// `xdatarefcheck($val)`: the output form of an XDATA reference, or
    /// `None` if `val` is not one.
    fn check(&self, val: &str) -> Option<String> {
        if !perl_truthy(val) {
            return None;
        }
        let re = self.check.as_ref()?;
        re.is_match(val).ok()?.then(|| self.out(val))
    }

    /// `xdatarefcheck($val) // $val`.
    fn or_value(&self, val: String) -> String {
        self.check(&val).unwrap_or(val)
    }
}

impl Opts {
    fn new(ctx: &mut Ctx) -> Result<Opts, Fatal> {
        let b = &ctx.cfg.biber;
        let s = |o: &str| b.get_str(o).unwrap_or("").to_string();
        let case = match lc(&s("output_fieldcase")).as_str() {
            "upper" => FieldCase::Upper,
            "lower" => FieldCase::Lower,
            "title" => FieldCase::Title,
            other => {
                // biber dies here (an undefined casing sub); bibers reports it.
                let other = other.to_string();
                biber_error!(
                    ctx.log,
                    "Invalid value '{other}' for option 'output_fieldcase' - must be one of 'upper', 'lower' or 'title'"
                )?;
                FieldCase::Upper
            }
        };
        let b = &ctx.cfg.biber;
        let s = |o: &str| b.get_str(o).unwrap_or("").to_string();
        let wraplines = s("wraplines");
        Ok(Opts {
            case,
            field_replace: ctx.cfg.output_field_replace.clone(),
            xname: b.is_true("output_xname"),
            namesep: s("output_namesep"),
            listsep: s("output_listsep"),
            resolve_xdata: b.is_true("output_resolve_xdata"),
            resolve_crossrefs: b.is_true("output_resolve_crossrefs"),
            legacy_dates: b.is_true("output_legacy_dates"),
            nostdmacros: b.is_true("nostdmacros"),
            ann_marker: s("output_annotation_marker"),
            named_ann_marker: s("output_named_annotation_marker"),
            in_ann_marker: s("annotation_marker"),
            align: b.is_true("output_align"),
            field_order: s("output_field_order"),
            indent: s("output_indent"),
            wraplines: if perl_truthy(&wraplines) {
                perl_num_int(&wraplines)
            } else {
                0
            },
            xnamesep_out: s("output_xnamesep"),
            xdata: XdataRefs::new(ctx),
            zero_is_empty: ctx.cfg.quirks.on(Quirk::ZeroIsEmpty),
            all_macrodefs: b.is_true("output_all_macrodefs"),
            no_macrodefs: b.is_true("output_no_macrodefs"),
        })
    }

    /// `$outmap`: field replacement, then case.
    fn outmap(&self, f: &str) -> String {
        self.case
            .apply(self.field_replace.get(f).map_or(f, String::as_str))
    }

    /// The casing function alone (macros, `@comment`, `@string`).
    fn casing(&self, s: &str) -> String {
        self.case.apply(s)
    }

    /// Perl truthiness of an output value (BB-056: `0` is false).
    fn present(&self, s: &str) -> bool {
        if self.zero_is_empty {
            perl_truthy(s)
        } else {
            !s.is_empty()
        }
    }

    /// `--output-indent`: indent character and count.
    fn indent(&self) -> (char, usize) {
        match self.indent.strip_suffix('t') {
            Some(n) => ('\t', perl_num_int(n).max(0) as usize),
            None => (' ', perl_num_int(&self.indent).max(0) as usize),
        }
    }
}

/// A string's integer value as Perl's `x`/`sprintf "%d"` see it.
fn perl_num_int(s: &str) -> i64 {
    match PerlNum::parse(s) {
        PerlNum::Int(i) => i,
        PerlNum::UInt(u) => i64::try_from(u).unwrap_or(i64::MAX),
        PerlNum::Float(f) if f.is_nan() => 0,
        PerlNum::Float(f) => f.trunc().clamp(i64::MIN as f64, i64::MAX as f64) as i64,
    }
}

/// `sprintf('%.2d', $s)`.
fn pad2(s: &str) -> String {
    let n = perl_num_int(s);
    if n < 0 {
        format!("-{:02}", n.unsigned_abs())
    } else {
        format!("{n:02}")
    }
}

/// The bibtex writer state (`$self->{output_data}` and the globals it
/// shares with the reader).
pub struct Writer {
    opts: Opts,
    /// `{ENTRIES}{99999}{index}`.
    entries: HashMap<String, String>,
    /// `{MACROS}` (`None` until one is set: `exists` is tested).
    macros: Option<Vec<String>>,
    /// `{COMMENTS}`.
    comments: Vec<String>,
    /// `%USEDSTRINGS`: macro name → value.
    usedstrings: HashMap<String, String>,
}

/// A field value as `get_field` returns it, interpolated or tested.
fn val_truthy(v: Option<&Value>) -> bool {
    match v {
        None => false,
        Some(Value::Str(s)) => perl_truthy(s),
        Some(Value::Other(J::Null)) => false,
        Some(Value::Other(J::String(s))) => perl_truthy(s),
        Some(_) => true,
    }
}

fn val_str(v: Option<&Value>) -> String {
    match v {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Other(J::String(s))) => s.clone(),
        Some(Value::List(l)) => l.join(","),
        _ => String::new(),
    }
}

/// A Perl-truthy untyped slot.
fn slot_truthy(v: Option<&J>) -> bool {
    match v {
        None | Some(J::Null) => false,
        Some(J::String(s)) => perl_truthy(s),
        Some(J::Bool(b)) => *b,
        Some(J::Number(n)) => perl_truthy(&n.to_string()),
        Some(_) => true,
    }
}

fn slot_str(v: Option<&J>) -> String {
    match v {
        None | Some(J::Null) => String::new(),
        Some(J::String(s)) => s.clone(),
        Some(o) => o.to_string(),
    }
}

/// `map_boolean($opt, $val, 'tostring')`.
fn map_boolean_tostring(ctx: &Ctx, opt: &str, val: &str) -> String {
    if ctx.cfg.blx.registry.datatype(opt) != Some("boolean") {
        return val.to_string();
    }
    map_boolean_tostring_raw(Some(val)).unwrap_or_default()
}

/// The year-division month codes of `construct_datetime` (bibtex.pm:654).
/// BB-047: biber's map has `WinterS` where the data has `winterS`.
fn yeardivision(s: &str, typo: bool) -> Option<&'static str> {
    Some(match s {
        "spring" => "21",
        "summer" => "22",
        "autumn" => "23",
        "winter" => "24",
        "springN" => "25",
        "summerN" => "26",
        "autumnN" => "27",
        "winterN" => "28",
        "springS" => "29",
        "summerS" => "30",
        "autumnS" => "31",
        "WinterS" if typo => "32",
        "winterS" if !typo => "32",
        "Q1" => "33",
        "Q2" => "34",
        "Q3" => "35",
        "Q4" => "36",
        "QD1" => "37",
        "QD2" => "38",
        "QD3" => "39",
        "S1" => "40",
        "S2" => "41",
        _ => return None,
    })
}

/// The reverse of biber's `%MONTHS` (`jan` → `1`, …).
fn rmonth(m: &str) -> Option<&'static str> {
    btparse::BIBER_MONTHS
        .iter()
        .find(|(_, n)| *n == m)
        .map(|(k, _)| *k)
}

impl Writer {
    /// A writer with the current options.
    pub fn new(ctx: &mut Ctx) -> Result<Writer, Fatal> {
        Ok(Writer {
            opts: Opts::new(ctx)?,
            entries: HashMap::new(),
            macros: None,
            comments: Vec::new(),
            usedstrings: HashMap::new(),
        })
    }

    /// `construct_datetime` (bibtex.pm:646): rebuild an ISO 8601-2 date from
    /// its parts, deleting them from the (copied) entry.
    fn construct_datetime(&self, ctx: &Ctx, be: &mut Entry, d: &str) -> String {
        let o = &self.opts;
        let get = |be: &Entry, f: &str| -> Option<String> {
            let v = be.get_field(&format!("{d}{f}"))?;
            Some(val_str(Some(v)))
        };
        // `my $x = …` truthiness of a date part
        let present = |v: &Option<String>| v.as_deref().is_some_and(|s| o.present(s));
        let mut datestring = String::new();
        // Quirk `bibtex-unspecified-dates`: `sprintf('%.2d', 'XX')` is `00`.
        let xx_quirk = ctx.cfg.quirks.on(Quirk::BibtexUnspecifiedDates);
        let pad2_unspec = |v: &str| {
            if !xx_quirk && v == "XX" {
                v.to_string()
            } else {
                pad2(v)
            }
        };
        let mut overridey: Option<String> = None;
        let mut overridem: Option<String> = None;
        let mut overrideem: Option<String> = None;
        let mut overrided: Option<String> = None;
        let del = |be: &mut Entry, f: &str| be.del_field(&format!("{d}{f}"));

        if let Some(unspec) = get(be, "dateunspecified").filter(|s| perl_truthy(s)) {
            match unspec.as_str() {
                "yearindecade" => {
                    // m/^(\d+)\d$/
                    let y = get(be, "year").unwrap_or_default();
                    overridey = Some(format!("{}X", strip_digits(&y, 1)));
                    del(be, "endyear");
                }
                "yearincentury" => {
                    let y = get(be, "year").unwrap_or_default();
                    overridey = Some(format!("{}XX", strip_digits(&y, 2)));
                    del(be, "endyear");
                }
                "monthinyear" => {
                    overridem = Some("XX".into());
                    del(be, "endyear");
                    del(be, "endmonth");
                }
                "dayinmonth" => {
                    overrided = Some("XX".into());
                    del(be, "endyear");
                    del(be, "endmonth");
                    del(be, "endday");
                }
                "dayinyear" => {
                    overridem = Some("XX".into());
                    overrided = Some("XX".into());
                    del(be, "endyear");
                    del(be, "endmonth");
                    del(be, "endday");
                }
                _ => {}
            }
        }

        let typo = ctx.cfg.quirks.on(Quirk::BibtexWinterSTypo);
        if let Some(s) = get(be, "yeardivision").filter(|s| perl_truthy(s)) {
            overridem = yeardivision(&s, typo).map(str::to_string);
        }
        if let Some(s) = get(be, "endyeardivision").filter(|s| perl_truthy(s)) {
            overrideem = yeardivision(&s, typo).map(str::to_string);
        }

        // `$a || $b`
        let or = |a: Option<String>, b: Option<String>| -> Option<String> {
            if a.as_deref().is_some_and(perl_truthy) {
                a
            } else {
                b
            }
        };

        let sy = or(overridey, get(be, "year"));
        if present(&sy) {
            datestring.push_str(sy.as_deref().unwrap_or(""));
            del(be, "year");

            let sm = or(overridem, get(be, "month"));
            if present(&sm) {
                datestring.push('-');
                datestring.push_str(&pad2_unspec(sm.as_deref().unwrap_or("")));
                del(be, "month");
            }
            let sd = or(overrided, get(be, "day"));
            if present(&sd) {
                datestring.push('-');
                datestring.push_str(&pad2_unspec(sd.as_deref().unwrap_or("")));
                del(be, "day");
            }
            let t = |be: &Entry, f: &str| get(be, f).is_some_and(|s| perl_truthy(&s));
            if t(be, "dateuncertain") && t(be, "dateapproximate") {
                datestring.push('%');
            } else {
                if t(be, "dateuncertain") {
                    datestring.push('?');
                }
                if t(be, "dateapproximate") {
                    datestring.push('~');
                }
            }
            let sh = get(be, "hour");
            if present(&sh) {
                datestring.push('T');
                datestring.push_str(&pad2(sh.as_deref().unwrap_or("")));
                datestring.push(':');
                datestring.push_str(&pad2(&get(be, "minute").unwrap_or_default()));
                datestring.push(':');
                datestring.push_str(&pad2(&get(be, "second").unwrap_or_default()));
                del(be, "hour");
                del(be, "minute");
                del(be, "second");
            }
            if let Some(tz) = get(be, "timezone").filter(|s| perl_truthy(s)) {
                datestring.push_str(&bibtzminsep(&tz));
                del(be, "timezone");
            }
            if be.field_exists(&format!("{d}endyear")) {
                datestring.push('/');
            }
            let ey = get(be, "endyear");
            if present(&ey) {
                datestring.push_str(ey.as_deref().unwrap_or(""));
                del(be, "endyear");
                let em = or(overrideem, get(be, "endmonth"));
                if present(&em) {
                    datestring.push('-');
                    datestring.push_str(&pad2(em.as_deref().unwrap_or("")));
                    del(be, "endmonth");
                }
                let ed = get(be, "endday");
                if present(&ed) {
                    datestring.push('-');
                    datestring.push_str(&pad2(ed.as_deref().unwrap_or("")));
                    del(be, "endday");
                }
                if t(be, "enddateuncertain") && t(be, "enddateapproximate") {
                    datestring.push('%');
                } else {
                    if t(be, "enddateuncertain") {
                        datestring.push('?');
                    }
                    if t(be, "enddateapproximate") {
                        datestring.push('~');
                    }
                }
                let eh = get(be, "endhour");
                if present(&eh) {
                    datestring.push('T');
                    datestring.push_str(&pad2(eh.as_deref().unwrap_or("")));
                    datestring.push(':');
                    datestring.push_str(&pad2(&get(be, "endminute").unwrap_or_default()));
                    datestring.push(':');
                    datestring.push_str(&pad2(&get(be, "endsecond").unwrap_or_default()));
                    del(be, "endhour");
                    del(be, "endminute");
                    del(be, "endsecond");
                }
                if let Some(tz) = get(be, "endtimezone").filter(|s| perl_truthy(s)) {
                    datestring.push_str(&bibtzminsep(&tz));
                    del(be, "endtimezone");
                }
            }
        }
        datestring
    }

    /// `name_to_bibtex` (Name.pm:566).
    fn name_to_bibtex(n: &Name) -> String {
        if let Some(x) = n.slots.get("xdata").filter(|v| slot_truthy(Some(v))) {
            return slot_str(Some(x));
        }
        let mut parts: HashMap<&str, String> = HashMap::new();
        for np in ["prefix", "family", "suffix", "given"] {
            let s = n
                .nameparts
                .get(np)
                .and_then(|p| p.string.clone())
                .unwrap_or_default();
            if perl_truthy(&s) {
                let mut s = s.replace('~', " ");
                if n.strip
                    .get(np)
                    .and_then(|v| v.as_deref())
                    .is_some_and(perl_truthy)
                {
                    s = format!("{{{s}}}");
                }
                parts.insert(np, s);
            }
        }
        let p = |k: &str| parts.get(k).filter(|s| perl_truthy(s));
        let mut out = String::new();
        if let Some(x) = p("prefix") {
            out.push_str(x);
            out.push(' ');
        }
        if let Some(x) = p("family") {
            out.push_str(x);
        }
        if let Some(x) = p("suffix") {
            out.push_str(", ");
            out.push_str(x);
        }
        if let Some(x) = p("given") {
            out.push_str(", ");
            out.push_str(x);
        }
        out
    }

    /// `name_to_xname` (Name.pm:598).
    fn name_to_xname(&self, ctx: &Ctx, n: &Name) -> String {
        let xns = &self.opts.xnamesep_out;
        let mut nps = ctx.dm.get_constant_value("nameparts");
        nps.sort();
        let mut out = Vec::new();
        for np in nps {
            let s = n
                .nameparts
                .get(&np)
                .and_then(|p| p.string.clone())
                .unwrap_or_default();
            if perl_truthy(&s) {
                out.push(format!("{np}{xns}{}", s.replace('~', " ")));
            }
        }
        if let Some(v) = n.slots.get("useprefix").filter(|v| !v.is_null()) {
            out.push(format!(
                "useprefix{xns}{}",
                map_boolean_tostring(ctx, "useprefix", &slot_str(Some(v)))
            ));
        }
        if let Some(v) = n
            .slots
            .get("sortingnamekeytemplatename")
            .filter(|v| slot_truthy(Some(v)))
        {
            out.push(format!(
                "sortingnamekeytemplatename{xns}{}",
                slot_str(Some(v))
            ));
        }
        out.join(", ")
    }

    /// The output of one name list field.
    fn names_field(&self, ctx: &Ctx, st: &SectionState, id: NameListId) -> String {
        let o = &self.opts;
        let arena = &st.section.names;
        let list = arena.list(id);
        let mut namelist = Vec::new();
        if let Some(v) = list.slots.get("useprefix").filter(|v| !v.is_null()) {
            namelist.push(format!(
                "useprefix={}",
                map_boolean_tostring(ctx, "useprefix", &slot_str(Some(v)))
            ));
        }
        if let Some(v) = list
            .slots
            .get("sortingnamekeytemplatename")
            .filter(|v| slot_truthy(Some(v)))
        {
            namelist.push(format!("sortingnamekeytemplatename={}", slot_str(Some(v))));
        }
        for n in arena.names_of(id) {
            if !o.resolve_xdata
                && let Some(x) = n.slots.get("xdata").filter(|v| slot_truthy(Some(v)))
            {
                namelist.push(o.xdata.out(&slot_str(Some(x))));
                continue;
            }
            namelist.push(if o.xname {
                self.name_to_xname(ctx, n)
            } else {
                Self::name_to_bibtex(n)
            });
        }
        let mut s = namelist.join(&format!(" {} ", o.namesep));
        if slot_truthy(list.slots.get("morenames")) {
            s.push_str(&format!(" {} others", o.namesep));
        }
        s
    }

    /// `construct_annotation` (bibtex.pm:594).
    fn construct_annotation(ctx: &Ctx, key: &str, field: &str, name: &str) -> String {
        let ann = &ctx.run.annotations;
        let mut out = Vec::new();
        // Literal annotations lose their quotes (quirk
        // `bibtex-literal-annotations`).
        let keep_literal = !ctx.cfg.quirks.on(Quirk::BibtexLiteralAnnotations);
        let v = |a: &crate::model::Annotation| {
            let s = a.value.clone().unwrap_or_default();
            if keep_literal && a.literal {
                format!("\"{s}\"")
            } else {
                s
            }
        };
        if let Some(a) = ann
            .field
            .get(key)
            .and_then(|m| m.get(field))
            .and_then(|m| m.get(name))
            .filter(|a| a.value.as_deref().is_some_and(perl_truthy))
        {
            out.push(format!("={}", v(a)));
        }
        if let Some(items) = ann
            .item
            .get(key)
            .and_then(|m| m.get(field))
            .and_then(|m| m.get(name))
        {
            for (count, a) in items {
                out.push(format!("{count}={}", v(a)));
            }
        }
        if let Some(items) = ann
            .part
            .get(key)
            .and_then(|m| m.get(field))
            .and_then(|m| m.get(name))
        {
            for (count, parts) in items {
                for (part, a) in parts {
                    out.push(format!("{count}:{part}={}", v(a)));
                }
            }
        }
        out.join(";")
    }

    /// `set_output_entry` (bibtex.pm:121): the text of one entry.
    fn set_output_entry(
        &mut self,
        ctx: &mut Ctx,
        st: &SectionState,
        key: &str,
    ) -> Result<(), Fatal> {
        // `Section::bibentry` resolves citekey aliases (Section.pm:518).
        let real = st
            .section
            .citekey_alias
            .get(key)
            .filter(|k| perl_truthy(k))
            .map_or(key, String::as_str);
        let Some(orig) = st.section.bibentry(real) else {
            biber_error!(ctx.log, "Cannot find entry with key '{key}' to output")?;
            return Ok(());
        };
        // The entry is written (and indexed) under its own key.
        let key = orig.citekey().to_string();
        let key = key.as_str();
        // The writer deletes fields as it goes (crossref/xdata, date parts).
        let mut be = orig.clone();
        let secnum = st.section.number.clone();
        let dm = ctx.dm;
        let dmh = &dm.helpers;
        let bee = be.entrytype().to_string();
        let mut out = format!("@{}{{{key},\n", self.opts.outmap(&bee));
        let mut acc: BTreeMap<String, String> = BTreeMap::new();

        // IDs
        if val_truthy(be.get_field("ids")) {
            acc.insert(self.opts.outmap("ids"), val_str(be.get_field("ids")));
        }

        // Name fields
        for nf in &dmh.namelists {
            let Some(Value::Names(id)) = be.get_field(nf) else {
                continue;
            };
            let s = self.names_field(ctx, st, *id);
            acc.insert(self.opts.outmap(nf), s);
        }

        // List fields and verbatim list fields
        for lf in dmh.lists.iter().chain(dmh.vlists.iter()) {
            let v = be.get_field(lf);
            if !val_truthy(v) {
                continue;
            }
            let items: Vec<String> = match v {
                Some(Value::List(l)) => l
                    .iter()
                    .map(|i| {
                        if self.opts.resolve_xdata {
                            i.clone()
                        } else {
                            self.opts.xdata.or_value(i.clone())
                        }
                    })
                    .collect(),
                other => vec![val_str(other)],
            };
            acc.insert(
                self.opts.outmap(lf),
                items.join(&format!(" {} ", self.opts.listsep)),
            );
        }

        // Per-entry options (DC-054: in option name order).
        let entryoptions: Vec<String> = ctx
            .cfg
            .blx
            .entry_options(&secnum, key)
            .into_iter()
            .map(|opt| {
                let v = match ctx.cfg.blx.get(Some(&secnum), opt, None, Some(key)) {
                    None | Some(OptValue::Undef) => String::new(),
                    Some(OptValue::Str(s)) => s.clone(),
                    Some(other) => other.to_json().to_string(),
                };
                format!("{opt}={v}")
            })
            .collect();
        if !entryoptions.is_empty() {
            acc.insert(self.opts.outmap("options"), entryoptions.join(","));
        }

        // Date fields
        for d in resolve::datefield_stems(ctx) {
            let year = be.get_field(&format!("{d}year"));
            if !year.is_some_and(|v| self.opts.present(&val_str(Some(v)))) {
                continue;
            }
            if d.is_empty() && self.opts.legacy_dates {
                let val = val_str(be.get_field("year"));
                if self.opts.present(&val) {
                    if !val_truthy(be.get_field("day")) && !val_truthy(be.get_field("endyear")) {
                        acc.insert(self.opts.outmap("year"), val);
                        let mval = val_str(be.get_field("month"));
                        if self.opts.present(&mval) {
                            let m = if self.opts.nostdmacros {
                                mval
                            } else {
                                rmonth(&mval).unwrap_or("").to_string()
                            };
                            acc.insert(self.opts.outmap("month"), m);
                        }
                        continue;
                    }
                    ctx.log.biber_warn(format!(
                        "Date in entry '{key}' has DAY or ENDYEAR, cannot be output in legacy format."
                    ));
                }
            }
            let ds = self.construct_datetime(ctx, &mut be, &d);
            acc.insert(self.opts.outmap(&format!("{d}date")), ds);
        }

        // If CROSSREF and XDATA have been resolved, don't output them.
        if self.opts.resolve_crossrefs && val_truthy(be.get_field("crossref")) {
            be.del_field("crossref");
        }
        if self.opts.resolve_xdata && val_truthy(be.get_field("xdata")) {
            be.del_field("xdata");
        }

        // Standard fields
        for f in &dmh.fields {
            let v = val_str(be.get_field(f));
            if be.get_field(f).is_none() || !self.opts.present(&v) {
                continue;
            }
            let v = if self.opts.resolve_xdata {
                v
            } else {
                self.opts.xdata.or_value(v)
            };
            let k = self.opts.outmap(f);
            // Could have been set in dates above (MONTH, YEAR).
            if !acc.get(&k).is_some_and(|x| self.opts.present(x)) {
                acc.insert(k, v);
            }
        }

        // XSV fields (keywords has its own output below)
        for f in &dmh.xsv {
            if f == "keywords" || !val_truthy(be.get_field(f)) {
                continue;
            }
            let fl = val_str(be.get_field(f));
            let fl = if self.opts.resolve_xdata {
                fl
            } else {
                self.opts.xdata.or_value(fl)
            };
            acc.entry(self.opts.outmap(f)).or_default().push_str(&fl);
        }

        // Ranges
        for f in &dmh.ranges {
            let Some(Value::Ranges(r)) = be.get_field(f) else {
                continue;
            };
            let rfl = construct_range(r);
            let rfl = if self.opts.resolve_xdata {
                rfl
            } else {
                self.opts.xdata.or_value(rfl)
            };
            acc.entry(self.opts.outmap(f)).or_default().push_str(&rfl);
        }

        // Verbatim fields
        for f in &dmh.vfields {
            if !val_truthy(be.get_field(f)) {
                continue;
            }
            let v = val_str(be.get_field(f));
            let v = if self.opts.resolve_xdata {
                v
            } else {
                self.opts.xdata.or_value(v)
            };
            acc.insert(self.opts.outmap(f), v);
        }

        // Keywords
        if val_truthy(be.get_field("keywords")) {
            let kl = val_str(be.get_field("keywords"));
            let kl = if self.opts.resolve_xdata {
                kl
            } else {
                self.opts.xdata.or_value(kl)
            };
            acc.insert(self.opts.outmap("keywords"), kl);
        }

        // Annotations
        let fields: Vec<String> = acc.keys().cloned().collect();
        for f in fields {
            let lf = lc(&f);
            if !ctx.run.annotations.is_annotated_field(key, &lf) {
                continue;
            }
            let names = ctx
                .run
                .annotations
                .names
                .get(key)
                .and_then(|m| m.get(&lf))
                .cloned()
                .unwrap_or_default();
            for n in names {
                let k = format!(
                    "{}{}{}{n}",
                    self.opts.outmap(&f),
                    self.opts.ann_marker,
                    self.opts.named_ann_marker
                );
                acc.insert(k, Self::construct_annotation(ctx, key, &lf, &n));
            }
        }

        // Maximum length of field names
        let max_field_len = if self.opts.align {
            acc.keys().map(|k| gcstring::length(k)).max()
        } else {
            None
        };

        // Order of fields
        let datefields_now = resolve::datefields_now(ctx);
        let class = |c: &str| -> Vec<String> {
            match c {
                "names" => dmh.namelists.clone(),
                "lists" => dmh.lists.clone(),
                _ => datefields_now.clone(),
            }
        };
        for field in split_field_order(&self.opts.field_order) {
            if matches!(field.as_str(), "names" | "lists" | "dates") {
                let members = class(&field);
                let mut done = Vec::new();
                for k in acc.keys() {
                    let sk = fc(strip_annotation(k, &self.opts.in_ann_marker));
                    // `first {…}` returns the element: a false one (`''`
                    // from `date`, BB-028) does not count.
                    if members
                        .iter()
                        .find(|m| fc(m) == sk)
                        .is_some_and(|m| perl_truthy(m))
                    {
                        out.push_str(&self.bibfield(ctx, k, &acc[k], max_field_len));
                        done.push(k.clone());
                    }
                }
                for k in done {
                    acc.remove(&k);
                }
            } else {
                let k = self.opts.outmap(&field);
                if let Some(value) = acc.remove(&k)
                    && self.opts.present(&value)
                {
                    out.push_str(&self.bibfield(ctx, &k, &value, max_field_len));
                }
            }
        }
        // Then the remaining fields
        for (k, v) in &acc {
            out.push_str(&self.bibfield(ctx, k, v, max_field_len));
        }
        out.push_str("}\n\n");

        let out = self.recode(ctx, key, out)?;
        self.entries.insert(key.to_string(), out);
        Ok(())
    }

    /// The LaTeX recoding of an entry (bibtex.pm:383-397).
    fn recode(&self, ctx: &mut Ctx, key: &str, text: String) -> Result<String, Fatal> {
        let b = &ctx.cfg.biber;
        let dset = b.get_str("decodecharsset").unwrap_or("base").to_string();
        let eset = b
            .get_str("output_safecharsset")
            .unwrap_or("base")
            .to_string();
        let recoder =
            || latex_recode::Recoder::builtin(&dset, &eset, latex_recode::RecodeOptions::BIBER);
        if b.is_true("output_safechars") {
            return Ok(recoder().encode(&text));
        }
        let outenc = b.get_str("output_encoding").unwrap_or("UTF-8").to_string();
        if outenc == "UTF-8" {
            return Ok(text);
        }
        let enc = crate::output::bbl::find_output_encoding(ctx)?;
        let n = nfc(&text);
        // BB-048: `encode($outenc, NFC($acc)) =~ /\?/` also matches a
        // literal `?`.
        let bad = enc.has_unencodable(&n)
            || (ctx.cfg.quirks.on(Quirk::BibtexEncodingQuestionMark) && n.contains('?'));
        if bad {
            let r = recoder().encode(&text);
            ctx.log.biber_warn(format!(
                "The entry '{key}' has characters which cannot be encoded in '{outenc}'. Recoding problematic characters into macros."
            ));
            return Ok(r);
        }
        Ok(text)
    }

    /// `bibfield` (bibtex.pm:530): format one field.
    fn bibfield(
        &mut self,
        ctx: &Ctx,
        field: &str,
        value: &str,
        max_field_len: Option<usize>,
    ) -> String {
        let o = &self.opts;
        let (ichar, inum) = o.indent();
        let mut acc: String = std::iter::repeat_n(ichar, inum).collect();
        acc.push_str(field);
        let flen = gcstring::length(field);
        if let Some(m) = max_field_len.filter(|m| *m > 0) {
            acc.extend(std::iter::repeat_n(' ', m.saturating_sub(flen)));
        }
        acc.push_str(" = ");
        // Is the field value a macro? If so, replace with macro.
        let mut value = value.to_string();
        if let Some(m) = ctx
            .run
            .bibcache
            .rstrings
            .get(&value)
            .filter(|m| perl_truthy(m))
        {
            self.usedstrings.insert(m.clone(), value.clone());
            value = o.casing(m);
        }
        if macro_text(ctx, &value).is_some_and(|t| !t.is_empty()) {
            acc.push_str(&value);
            acc.push_str(",\n");
        } else {
            acc.push('{');
            acc.push_str(&value);
            acc.push_str("},\n");
        }
        if o.wraplines != 0 {
            // +4 is for ' = {'
            let indent = inum + max_field_len.unwrap_or(flen) + 4;
            let xp: String = std::iter::repeat_n(ichar, indent).collect();
            return textwrap::wrap_no_unexpand(o.wraplines, "", &xp, &acc);
        }
        acc
    }

    /// `set_output_comment` (bibtex.pm:51).
    fn set_output_comment(&mut self, comment: &str) {
        let c = format!("@{}{{{comment}}}\n", self.opts.casing("comment"));
        self.comments.push(c);
    }

    /// `set_output_macro` (bibtex.pm:83).
    fn set_output_macro(&mut self, ctx: &Ctx, m: &str) {
        if !self.opts.all_macrodefs && !self.usedstrings.get(m).is_some_and(|v| perl_truthy(v)) {
            return;
        }
        let text = macro_text(ctx, m).unwrap_or("");
        let s = format!(
            "@{}{{{} = \"{}\"}}\n",
            self.opts.casing("string"),
            self.opts.casing(m),
            nfd(text)
        );
        self.macros.get_or_insert_with(Vec::new).push(s);
    }

    /// `create_output_section` (bibtex.pm:493).
    pub fn create_output_section(&mut self, ctx: &mut Ctx, st: &SectionState) -> Result<(), Fatal> {
        for k in &st.section.citekeys {
            self.set_output_entry(ctx, st, k)?;
        }
        if let Some(cs) = ctx.run.bibcache.master_comments.clone() {
            for c in cs {
                self.set_output_comment(&c);
            }
        }
        if !self.opts.no_macrodefs {
            let mut ms: Vec<String> = ctx.run.bibcache.rstrings.values().cloned().collect();
            ms.sort();
            for m in ms {
                self.set_output_macro(ctx, &m);
            }
        }
        Ok(())
    }

    /// `output` (bibtex.pm:412), up to encoding: the text of the file.
    pub fn output(&self, ctx: &Ctx, processed: &Processed) -> String {
        let mut out = String::new();
        let mut put = |s: &str| out.push_str(&nfc(s));
        let tool = ctx.cfg.biber.tool();
        if tool && let Some(ms) = &self.macros {
            let mut ms = ms.clone();
            ms.sort();
            for m in &ms {
                put(m);
            }
            put("\n");
        }
        // Quirk `tool-drops-preambles` fixed: the data's `@preamble`s.
        if tool && !ctx.cfg.quirks.on(Quirk::ToolDropsPreambles) {
            let pre: Vec<&String> = processed
                .sections
                .iter()
                .filter(|s| s.section.number == SECTION)
                .flat_map(|s| s.preamble.iter().flatten())
                .collect();
            for p in &pre {
                put(&format!("@{}{{\"{p}\"}}\n", self.opts.casing("preamble")));
            }
            if !pre.is_empty() {
                put("\n");
            }
        }
        // One special section, sorted by the global sorting template.
        let stn = match ctx.cfg.blx.global("sortingtemplatename") {
            Some(OptValue::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let want = format!("{stn}/global//global/global/global");
        let list = processed
            .sections
            .iter()
            .filter(|s| s.section.number == SECTION)
            .flat_map(|s| s.datalists.iter())
            .find(|l| {
                l.section == SECTION
                    && l.name == want
                    && l.type_ == "entry"
                    && l.sortingtemplatename == stn
                    && l.sortingnamekeytemplatename == "global"
                    && l.labelprefix.is_empty()
                    && l.uniquenametemplatename == "global"
                    && l.labelalphanametemplatename == "global"
                    && l.namehashtemplatename == "global"
            });
        // biber dies (`->[0]->get_keys` on undef) when the global sorting
        // template is not the list's (e.g. biber.conf's `sortingtemplatename`
        // with `blx-defaults-override-conf` fixed); bibers uses the section's
        // first entry list instead.
        let list = list.or_else(|| {
            processed
                .sections
                .iter()
                .filter(|s| s.section.number == SECTION)
                .flat_map(|s| s.datalists.iter())
                .find(|l| l.type_ == "entry")
        });
        if let Some(l) = list {
            for k in l.keys.as_deref().unwrap_or(&[]) {
                if let Some(e) = self.entries.get(k) {
                    put(e);
                }
            }
        }
        if tool {
            for c in &self.comments {
                put(c);
            }
        }
        out
    }
}

/// `Text::BibTeX::macro_text`/`macro_length` for a (NFD) Perl string: the
/// XS passes the string's UTF-8 bytes, but libbtparse keys its table by the
/// NFC bytes of the source, so a name with decomposable characters is never
/// found.
fn macro_text<'a>(ctx: &'a Ctx, name: &str) -> Option<&'a str> {
    if nfc(name) != name {
        return None;
    }
    ctx.run.bt.macros.get(name)
}

/// `$stz =~ s/\\bibtzminsep\s+/:/`.
fn bibtzminsep(tz: &str) -> String {
    const M: &str = "\\bibtzminsep";
    if let Some(p) = tz.find(M) {
        let rest = &tz[p + M.len()..];
        let trimmed = rest.trim_start_matches(is_perl_space);
        if trimmed.len() < rest.len() {
            return format!("{}:{trimmed}", &tz[..p]);
        }
    }
    tz.to_string()
}

/// `m/^(\d+)\d{n}$/`: the leading digits of a year without its last `n`.
fn strip_digits(y: &str, n: usize) -> String {
    let c: Vec<char> = y.chars().collect();
    if c.len() > n && c.iter().all(|c| perl_compat::unicode::is_perl_digit(*c)) {
        c[..c.len() - n].iter().collect()
    } else {
        String::new()
    }
}

/// `construct_range` (bibtex.pm:627).
fn construct_range(r: &[Vec<Option<String>>]) -> String {
    r.iter()
        .map(|e| {
            let mut rs = e.first().cloned().flatten().unwrap_or_default();
            if let Some(Some(end)) = e.get(1) {
                rs.push_str("--");
                rs.push_str(end);
            }
            rs
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `split(/\s*,\s*/, $order)`.
fn split_field_order(s: &str) -> Vec<String> {
    perl_compat::regex::split_default_xsvsep(s)
}

/// `strip_annotation` (Utils.pm:1742): `s/$ann$nam?.*$//`, i.e. cut at the
/// first annotation marker.
fn strip_annotation<'a>(s: &'a str, ann: &str) -> &'a str {
    match s.find(ann) {
        Some(p) => &s[..p],
        None => s,
    }
}

/// Render the bibtex output of a processed run (`create_output_section` for
/// the `99999` section, then `output`). Returns the text to encode.
pub fn render(ctx: &mut Ctx, processed: &Processed) -> Result<String, Fatal> {
    let mut w = Writer::new(ctx)?;
    for st in processed
        .sections
        .iter()
        .filter(|s| s.section.number == SECTION)
    {
        w.create_output_section(ctx, st)?;
    }
    Ok(w.output(ctx, processed))
}

//! Alphabetic labels: `process_labelalpha` / `process_extraalpha`
//! (Biber.pm:3020, 3043) and the label template machinery of
//! `Biber::Internals` (Internals.pm:300-1022: `_genlabel`, `_labelpart`,
//! `_dispatch_label`, `_label_*`, `_process_label_attributes`,
//! `_label_listdisambiguation`, …). Called from
//! `entries::process_entries_post` for every section citekey.
//!
//! State: the label and sort label go to the data list's
//! `state.fields{citekey}{labelalpha|sortlabelalpha}`, extraalpha counts to
//! `state.ladisambiguation{label}`; the `v`/`l` width disambiguation caches
//! live in the section (`labelcache_v`/`labelcache_l`, keyed by field name
//! only, so shared by all data lists and label parts of the section: BB-094).
//! All of these keep biber's dump shapes (they are [`PerlData`]).

use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value as J};

use super::Ctx;
use super::sort::strings::{replace_ties, strip_macros, trim_collapse};
use crate::config::Config;
use crate::config::blx::OptValue;
use crate::datamodel::DataModel;
use crate::log::Fatal;
use crate::model::{Entry, NameArena, NameListId, PerlData, SectionState, Value};
use crate::perlish::{numify, sort_by_order, truthy};
use crate::quirks::Quirk;
use crate::xmlin::{XHash, XVal};
use perl_compat::grapheme::gcstring;
use perl_compat::regex::PerlRegex;
use perl_compat::{lc, uc};

/// Run-level helper state for label generation (owned by
/// [`super::RunState`]).
#[derive(Debug, Default)]
pub struct LabelRunCache {
    /// citekey → its first index in the section's citekeys: replaces the
    /// `first_index` scan biber does on every call of
    /// `_process_label_attributes` (Internals.pm:703-704, O(N²) overall).
    /// Tagged with the section number and citekey count it was built for.
    index: Option<(String, usize, HashMap<String, usize>)>,
}

impl LabelRunCache {
    /// `first_index {$_ eq $citekey} @citekeys` (`-1` if absent).
    fn first_index(&mut self, secnum: &str, citekeys: &[String], citekey: &str) -> i64 {
        let valid = |c: &Option<(String, usize, HashMap<String, usize>)>| {
            c.as_ref().is_some_and(|(s, n, m)| {
                s == secnum
                    && *n == citekeys.len()
                    && m.get(citekey)
                        .is_some_and(|&i| citekeys.get(i).is_some_and(|k| k == citekey))
            })
        };
        if !valid(&self.index) {
            let mut m = HashMap::with_capacity(citekeys.len());
            for (i, k) in citekeys.iter().enumerate() {
                m.entry(k.clone()).or_insert(i);
            }
            self.index = Some((secnum.to_string(), citekeys.len(), m));
        }
        self.index
            .as_ref()
            .and_then(|(_, _, m)| m.get(citekey))
            .map_or(-1, |&i| i as i64)
    }
}

/// `Biber::process_labelalpha` (Biber.pm:3020) for entry `citekey` in the
/// data list `st.datalists[list]`.
pub fn process_labelalpha(
    ctx: &mut Ctx,
    st: &mut SectionState,
    list: usize,
    citekey: &str,
) -> Result<(), Fatal> {
    let cfg: &Config = ctx.cfg;
    let secnum = st.section.number.clone();
    let Some(be) = st.section.bibentry(citekey) else {
        return Ok(());
    };
    let bee = be.entrytype().to_string();
    // Don't add a label if skiplab is set for entry
    if opt_truthy(
        cfg.blx
            .get(Some(&secnum), "skiplab", Some(&bee), Some(citekey)),
    ) {
        return Ok(());
    }
    // getblxoption(undef, 'labelalpha', …): per-entry values invisible (BB-021)
    let la = cfg
        .blx
        .get_undef_secnum(cfg.quirks, &secnum, "labelalpha", Some(&bee), Some(citekey));
    if !opt_truthy(la) {
        return Ok(());
    }
    let SectionState {
        section, datalists, ..
    } = st;
    let dl = &mut datalists[list];
    let mut g = Gen {
        cfg,
        dm: ctx.dm,
        secnum: &secnum,
        entries: &section.entries,
        names: &section.names,
        citekeys: &section.citekeys,
        slots: &mut section.slots,
        dlstate: &mut dl.state,
        lantname: &dl.labelalphanametemplatename,
        listname: &dl.name,
        run: &mut ctx.run.labelalpha,
        label_final: false,
    };
    let (label, slabel) = g.genlabel(citekey);
    set_entryfield(
        &mut dl.state,
        citekey,
        "labelalpha",
        label.map_or(J::Null, J::from),
    );
    set_entryfield(
        &mut dl.state,
        citekey,
        "sortlabelalpha",
        slabel.map_or(J::Null, J::from),
    );
    Ok(())
}

/// `Biber::process_extraalpha` (Biber.pm:3043) for entry `citekey` in the
/// data list `st.datalists[list]`: counts the labels for extraalpha.
pub fn process_extraalpha(
    ctx: &mut Ctx,
    st: &mut SectionState,
    list: usize,
    citekey: &str,
) -> Result<(), Fatal> {
    let cfg: &Config = ctx.cfg;
    let secnum = &st.section.number;
    let Some(be) = st.section.bibentry(citekey) else {
        return Ok(());
    };
    let bee = be.entrytype();
    let la = cfg
        .blx
        .get_undef_secnum(cfg.quirks, secnum, "labelalpha", Some(bee), Some(citekey));
    if !opt_truthy(la) {
        return Ok(());
    }
    let state = &mut st.datalists[list].state;
    // get_entryfield autovivifies state.fields{citekey}
    let label = vivify(vivify(state_obj(state), "fields"), citekey)
        .get("labelalpha")
        .and_then(J::as_str)
        .filter(|s| truthy(Some(s)))
        .map(str::to_string);
    if let Some(la) = label {
        // incr_la_disambiguation
        let m = vivify(state_obj(state), "ladisambiguation");
        let n = m.get(&la).and_then(J::as_str).map_or(0.0, numify);
        m.insert(la, J::from(perl_compat::truth::format_nv(n + 1.0)));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// PerlData helpers

/// Perl truthiness of an option value (references are true).
fn opt_truthy(v: Option<&OptValue>) -> bool {
    v.is_some_and(OptValue::truthy)
}

/// A scalar option value.
fn opt_str(v: Option<&OptValue>) -> Option<&str> {
    v.and_then(OptValue::as_str)
}

/// The data list state as a hash (autovivified).
fn state_obj(state: &mut Option<PerlData>) -> &mut Map<String, J> {
    let s = state.get_or_insert_with(|| J::Object(Map::new()));
    if !s.is_object() {
        *s = J::Object(Map::new());
    }
    match s {
        J::Object(o) => o,
        _ => unreachable!(),
    }
}

/// `$m->{k}` as a hash, autovivified.
fn vivify<'j>(m: &'j mut Map<String, J>, k: &str) -> &'j mut Map<String, J> {
    let v = m
        .entry(k.to_string())
        .or_insert_with(|| J::Object(Map::new()));
    if !v.is_object() {
        *v = J::Object(Map::new());
    }
    match v {
        J::Object(o) => o,
        _ => unreachable!(),
    }
}

/// `$dlist->set_entryfield($citekey, $f, $v)`.
fn set_entryfield(state: &mut Option<PerlData>, citekey: &str, f: &str, v: J) {
    vivify(vivify(state_obj(state), "fields"), citekey).insert(f.to_string(), v);
}

/// `$dlist->get_visible_alpha($nlid)` (autovivifies
/// `state.namelistdata{nlid}`).
fn visible_alpha(state: &mut Option<PerlData>, nlid: NameListId) -> Option<String> {
    vivify(vivify(state_obj(state), "namelistdata"), &nlid.key())
        .get("visalpha")
        .and_then(J::as_str)
        .map(str::to_string)
}

/// Perl numeric value truncated to an integer (array indices, `IV` XS
/// arguments).
fn iv(s: &str) -> i64 {
    let f = numify(s);
    if f.is_nan() { 0 } else { f.trunc() as i64 }
}

/// Perl `$a[$i]` (negative indices count from the end).
fn perl_idx(len: usize, i: i64) -> Option<usize> {
    let i = if i < 0 { i + len as i64 } else { i };
    (i >= 0 && (i as usize) < len).then_some(i as usize)
}

/// A field value as a Perl string (references stringify to an address in
/// biber; we use a fixed placeholder, cf. DC-039).
fn value_string(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::List(_) | Value::Ranges(_) => "ARRAY".into(),
        Value::Names(_) => "Biber::Entry::Names".into(),
        Value::Other(_) => "HASH".into(),
    }
}

/// Perl truthiness of a field value.
fn value_truthy(v: Option<&Value>) -> bool {
    match v {
        None => false,
        Some(Value::Str(s)) => truthy(Some(s)),
        Some(_) => true,
    }
}

// ---------------------------------------------------------------------------
// Biber::Utils string helpers

/// `Biber::Utils::escape_label` (Utils.pm:766).
pub fn escape_label(s: &str) -> String {
    if !truthy(Some(s)) {
        return String::new();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '_' | '^' | '$' | '#' | '%' | '&' => {
                out.push('\\');
                out.push(c);
            }
            '~' => out.push_str("{\\textasciitilde}"),
            '>' => out.push_str("{\\textgreater}"),
            '<' => out.push_str("{\\textless}"),
            _ => out.push(c),
        }
    }
    out
}

/// `Biber::Utils::unescape_label` (Utils.pm:783).
pub fn unescape_label(s: &str) -> String {
    if !truthy(Some(s)) {
        return String::new();
    }
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '\\'
            && cs
                .get(i + 1)
                .is_some_and(|c| matches!(c, '_' | '^' | '$' | '~' | '#' | '%' | '&'))
        {
            out.push(cs[i + 1]);
            i += 2;
        } else {
            out.push(cs[i]);
            i += 1;
        }
    }
    out.replace("{\\textasciitilde}", "~")
        .replace("{\\textgreater}", ">")
        .replace("{\\textless}", "<")
}

/// `Biber::Utils::normalise_string_label` (Utils.pm:635).
pub fn normalise_string_label(cfg: &Config, s: &str) -> String {
    if !truthy(Some(s)) {
        return String::new();
    }
    let s = strip_macros(s);
    let mut s = replace_ties(&s);
    for p in cfg.biber.patterns("nolabel") {
        // `qr/$re/` compiled once (Biber.pm:4447); an invalid pattern would
        // have died there.
        if let Ok(re) = PerlRegex::cached(p.value.as_deref().unwrap_or(""), false)
            && let Ok(r) = re.remove_all(&s)
        {
            s = r;
        }
    }
    trim_collapse(&s)
}

/// The match of `m/\A\s*(\P{Pd}+)?\s*(\p{Pd})*\s*(\P{Pd}+)?\s*\z/xms`:
/// `($1, $2, $3)`, or `None` if it doesn't match.
#[allow(clippy::type_complexity)]
fn range_parts(rs: &str) -> Option<(Option<String>, Option<String>, Option<String>)> {
    let re = PerlRegex::cached(r"\A\s*(\P{Pd}+)?\s*(\p{Pd})*\s*(\P{Pd}+)?\s*\z", false).ok()?;
    let g = re.matches(rs).ok()?.into_iter().next()?;
    let cap = |i: usize| {
        g.get(i)
            .copied()
            .flatten()
            .map(|(a, b)| rs[a..b].to_string())
    };
    Some((cap(1), cap(2), cap(3)))
}

/// `Biber::Utils::parse_range` (Utils.pm:1725): `[start, end]`.
fn parse_range(rs: &str) -> (Option<String>, Option<String>) {
    let (a, d, b) = range_parts(rs).unwrap_or((None, None, None));
    if truthy(d.as_deref()) {
        (Some(a.unwrap_or_else(|| "1".into())), b)
    } else {
        (Some("1".into()), a)
    }
}

/// `Biber::Utils::parse_range_alt` (Utils.pm:1756); `undef` is returned as
/// `[undef, undef]` (the caller only indexes into it).
fn parse_range_alt(rs: &str) -> (Option<String>, Option<String>) {
    let (a, d, b) = range_parts(rs).unwrap_or((None, None, None));
    if truthy(d.as_deref()) {
        (a, b)
    } else {
        (None, None)
    }
}

/// Byte offset → code point offset.
fn char_off(s: &str, b: usize) -> i64 {
    s[..b].chars().count() as i64
}

/// `Biber::Utils::match_indices` (Utils.pm:1698): for each regex, every
/// match as `[grapheme substring, offset]`. As in biber the offset is a code
/// point offset (`$-[0]`) used as a grapheme offset, minus the grapheme
/// length of the previous regex's last match. Last match first.
fn match_indices(regexes: &[&str], s: &str) -> Option<Vec<(String, i64)>> {
    let mut ret = Vec::new();
    let mut relen = 0i64;
    for r in regexes {
        let mut len = 0i64;
        if let Ok(re) = PerlRegex::cached(r, false)
            && let Ok(ms) = re.matches(s)
        {
            for g in ms {
                let Some((a, b)) = g[0] else { continue };
                let (ca, cb) = (char_off(s, a), char_off(s, b));
                let gcs = gcstring::substr(s, ca, Some(cb - ca)).to_string();
                len = gcstring::length(&gcs) as i64;
                ret.push((gcs, ca - relen));
            }
        }
        relen += len;
    }
    if ret.is_empty() {
        None
    } else {
        ret.reverse();
        Some(ret)
    }
}

/// `split(/$re/, $s)` with a built-in pattern.
fn split(re: &str, s: &str) -> Vec<String> {
    PerlRegex::cached(re, false)
        .and_then(|r| r.split(s))
        .map(|v| v.into_iter().map(Option::unwrap_or_default).collect())
        .unwrap_or_else(|_| vec![s.to_string()])
}

// ---------------------------------------------------------------------------
// Label generation

/// The per-namepart substring options of a labelalphaname template part
/// (`$fieldinfo->[1]`).
#[derive(Clone, Debug, Default)]
struct NpOpts {
    substring_width: Option<String>,
    substring_side: Option<String>,
    substring_compound: Option<String>,
}

/// One `[string, namepart options]` pair (`$fieldinfo`).
type FieldInfo = (String, NpOpts);

/// Label dispatch targets (`%internal_dispatch_label` and the data-model
/// fallback, Internals.pm:306-336).
enum Dispatch {
    Basic { field: String, nostrip: bool },
    Citekey,
    Name(String),
    Literal(String),
}

struct Gen<'a> {
    cfg: &'a Config,
    dm: &'a DataModel,
    secnum: &'a str,
    entries: &'a BTreeMap<String, Entry>,
    names: &'a NameArena,
    citekeys: &'a [String],
    /// Section slots (`labelcache_v`, `labelcache_l`).
    slots: &'a mut BTreeMap<String, PerlData>,
    /// The data list's `state`.
    dlstate: &'a mut Option<PerlData>,
    /// The data list's labelalphanametemplate name.
    lantname: &'a str,
    /// The data list's name.
    listname: &'a str,
    run: &'a mut LabelRunCache,
    /// `$LABEL_FINAL`.
    label_final: bool,
}

/// `$part->{key}` as a string.
fn attr<'x>(part: &'x XHash, key: &str) -> Option<&'x str> {
    part.str(key)
}

fn attr_true(part: &XHash, key: &str) -> bool {
    truthy(attr(part, key))
}

impl<'a> Gen<'a> {
    fn quirk(&self, q: Quirk) -> bool {
        self.cfg.quirks.on(q)
    }

    fn opt(&self, opt: &str, et: Option<&str>, key: Option<&str>) -> Option<&OptValue> {
        self.cfg
            .blx
            .get(Some(self.secnum), opt, et, key)
            .filter(|v| v.is_defined())
    }

    /// The `labelcache_v`/`labelcache_l` key: the field name (BB-094,
    /// `labelcache-field-key`), or with that quirk fixed, the field, data
    /// list and nameparts.
    fn cache_key(&self, field: &str, nameparts: Option<&[String]>) -> String {
        if self.quirk(Quirk::LabelcacheFieldKey) {
            field.to_string()
        } else {
            format!(
                "{field}\u{1f}{}\u{1f}{}",
                self.listname,
                nameparts.map(|n| n.join(",")).unwrap_or_default()
            )
        }
    }

    /// `_genlabel` (Internals.pm:340): `[label, sortlabel]` (`undef` if the
    /// template has no label elements).
    fn genlabel(&mut self, citekey: &str) -> (Option<String>, Option<String>) {
        let cfg = self.cfg;
        let entries = self.entries;
        let Some(be) = entries.get(citekey) else {
            return (None, None);
        };
        let bee = be.entrytype().to_string();
        let tmpl = match cfg
            .blx
            .get(Some(self.secnum), "labelalphatemplate", Some(&bee), None)
        {
            Some(OptValue::Xml(XVal::Hash(h))) => Some(h),
            _ => None,
        };
        let mut label: Option<String> = None;
        let mut slabel: Option<String> = None;
        self.label_final = false;
        let Some(tmpl) = tmpl else {
            return (label, slabel);
        };
        let mut elements: Vec<&XHash> = tmpl.list("labelelement").hashes().collect();
        sort_by_order(&mut elements, |e| e.str("order"));
        for el in elements {
            let parts: Vec<&XHash> = el.list("labelpart").hashes().collect();
            let (lp, slp) = self.labelpart(&parts, citekey);
            let l = label.get_or_insert_with(String::new);
            if truthy(lp.as_deref()) {
                l.push_str(lp.as_deref().unwrap_or(""));
            }
            let s = slabel.get_or_insert_with(String::new);
            if truthy(slp.as_deref()) {
                s.push_str(slp.as_deref().unwrap_or(""));
            }
            if self.label_final {
                break;
            }
        }
        (label, slabel)
    }

    /// `_labelpart` (Internals.pm:361): the first part giving a (true) label
    /// wins.
    fn labelpart(&mut self, parts: &[&XHash], citekey: &str) -> (Option<String>, Option<String>) {
        let entries = self.entries;
        let be = &entries[citekey];
        let bee = be.entrytype();
        let maxan =
            opt_str(self.opt("maxalphanames", Some(bee), Some(citekey))).map_or(0.0, numify);
        let minan =
            opt_str(self.opt("minalphanames", Some(bee), Some(citekey))).map_or(0.0, numify);
        let mut lp: Option<String> = None;
        let mut slp: Option<String> = None;
        for part in parts {
            // ifnames: only use this part if the name list has a certain length
            if let Some(inc) = attr(part, "ifnames").filter(|s| truthy(Some(s))) {
                let mut f = attr(part, "content").unwrap_or("").to_string();
                if f == "labelname" {
                    f = be
                        .slots
                        .get("labelnameinfo")
                        .and_then(J::as_str)
                        .filter(|s| truthy(Some(s)))
                        .unwrap_or("")
                        .to_string();
                }
                if self
                    .dm
                    .get_fields_of_type("list", &["name"], None)
                    .contains(&f)
                {
                    let name = be.get_field(&f);
                    if !value_truthy(name) {
                        continue;
                    }
                    let total = match name {
                        Some(Value::Names(id)) => self.names.list(*id).names.len() as f64,
                        _ => 0.0,
                    };
                    let visible = if total > maxan { minan } else { total };
                    // m/^\d+$/ (`$` also matches before a final newline)
                    let body = inc.strip_suffix('\n').unwrap_or(inc);
                    let is_digits =
                        !body.is_empty() && body.chars().all(perl_compat::unicode::is_perl_digit);
                    if is_digits {
                        if visible != numify(inc) {
                            continue;
                        }
                    } else {
                        let (a, b) = parse_range_alt(inc);
                        let num = |x: &Option<String>| x.as_deref().map_or(0.0, numify);
                        let ok = if a.is_none() {
                            visible <= num(&b)
                        } else if b.is_none() {
                            visible >= num(&a)
                        } else {
                            visible >= num(&a) && visible <= num(&b)
                        };
                        if !ok {
                            continue;
                        }
                    }
                }
            }
            let (l, s) = self.dispatch_label(part, citekey);
            lp.get_or_insert_with(String::new).push_str(&l);
            slp.get_or_insert_with(String::new).push_str(&s);
            if truthy(Some(&l)) {
                if attr_true(part, "final") {
                    self.label_final = true;
                }
                break;
            }
        }
        (lp, slp)
    }

    /// `_dispatch_table_label` (Internals.pm:318).
    fn dispatch_table(&self, field: &str) -> Option<Dispatch> {
        let basic = |f: &str, nostrip: bool| {
            Some(Dispatch::Basic {
                field: f.to_string(),
                nostrip,
            })
        };
        match field {
            "label" | "shorthand" | "sortkey" => return basic(field, true),
            "citekey" | "entrykey" => return Some(Dispatch::Citekey),
            "labelname" => return Some(Dispatch::Name("labelname".into())),
            "labeltitle" | "labelmonth" | "labelday" | "labelyear" => return basic(field, false),
            _ => {}
        }
        let am = self.cfg.biber.annotation_marker();
        let nm = self.cfg.biber.named_annotation_marker();
        if !self.dm.is_field(field, am, nm) {
            return None;
        }
        if self.dm.get_fieldtype(field) == Some("list")
            && self.dm.get_datatype(field) == Some("name")
        {
            Some(Dispatch::Name(field.to_string()))
        } else {
            basic(field, false)
        }
    }

    /// `_dispatch_label` (Internals.pm:443).
    fn dispatch_label(&mut self, part: &XHash, citekey: &str) -> (String, String) {
        let content = attr(part, "content").unwrap_or("").to_string();
        match self
            .dispatch_table(&content)
            .unwrap_or(Dispatch::Literal(content))
        {
            Dispatch::Basic { field, nostrip } => self.label_basic(citekey, &field, nostrip, part),
            Dispatch::Citekey => {
                // _label_citekey
                let k = self.process_label_attributes(
                    citekey,
                    &[(citekey.to_string(), NpOpts::default())],
                    part,
                    None,
                    None,
                    None,
                );
                let u = unescape_label(&k);
                (k, u)
            }
            Dispatch::Name(n) => self.label_name(citekey, &n, part),
            Dispatch::Literal(s) => {
                // _label_literal
                let u = unescape_label(&s);
                (escape_label(&u), u)
            }
        }
    }

    /// `_label_basic` (Internals.pm:474).
    fn label_basic(
        &mut self,
        citekey: &str,
        e: &str,
        nostrip: bool,
        part: &XHash,
    ) -> (String, String) {
        let entries = self.entries;
        let be = &entries[citekey];
        let v = be.get_field(e);
        let f = if nostrip {
            match v {
                Some(v) if value_truthy(Some(v)) => value_string(v),
                _ => String::new(),
            }
        } else {
            match v {
                Some(v) if value_truthy(Some(v)) => {
                    normalise_string_label(self.cfg, &value_string(v))
                }
                _ => String::new(),
            }
        };
        if truthy(Some(&f)) {
            let b = self.process_label_attributes(
                citekey,
                &[(f, NpOpts::default())],
                part,
                Some(e),
                None,
                None,
            );
            let u = unescape_label(&b);
            (b, u)
        } else {
            (String::new(), String::new())
        }
    }

    /// `_label_name` (Internals.pm:500).
    fn label_name(&mut self, citekey: &str, namename: &str, attrs: &XHash) -> (String, String) {
        let cfg = self.cfg;
        let entries = self.entries;
        let names_arena = self.names;
        let be = &entries[citekey];
        let bee = be.entrytype().to_string();
        let mut useprefix: Option<String> =
            opt_str(self.opt("useprefix", Some(&bee), Some(citekey))).map(str::to_string);
        let alphaothers = opt_str(cfg.blx.get(None, "alphaothers", Some(&bee), None))
            .unwrap_or("")
            .to_string();
        let sortalphaothers = opt_str(cfg.blx.get(None, "sortalphaothers", Some(&bee), None))
            .unwrap_or("")
            .to_string();
        let mut lantname = self.lantname.to_string();
        if let Some(v) = opt_str(cfg.blx.get(
            Some(self.secnum),
            "labelalphanametemplatename",
            None,
            Some(citekey),
        )) {
            lantname = v.to_string();
        }
        // No labelname: nothing to do
        let Some(lni) = be.slots.get("labelnameinfo").and_then(J::as_str) else {
            return (String::new(), String::new());
        };
        let realname = if namename == "labelname" {
            lni.to_string()
        } else {
            namename.to_string()
        };
        let names = be.get_field(&realname);
        // short* labelnames test the use* option of the base name
        let lnameopt = match realname.strip_prefix("short") {
            Some(rest) if !rest.is_empty() => rest.to_string(),
            _ => realname.clone(),
        };
        let use_ok = opt_truthy(self.opt(&format!("use{lnameopt}"), Some(&bee), Some(citekey)));
        let Some(Value::Names(nlid)) = names.filter(|_| use_ok) else {
            return (String::new(), String::new());
        };
        let nlid = *nlid;
        let nl = names_arena.list(nlid);
        if let Some(v) = nl
            .slots
            .get("labelalphanametemplatename")
            .and_then(J::as_str)
        {
            lantname = v.to_string();
        }
        if let Some(v) = nl.slots.get("useprefix").and_then(J::as_str) {
            useprefix = Some(v.to_string());
        }
        let numnames = nl.names.len() as f64;
        let visibility = visible_alpha(self.dlstate, nlid);
        let (nr_start, nr_end): (f64, Option<String>) = if attrs.contains("names") {
            let (s, e) = parse_range(attr(attrs, "names").unwrap_or(""));
            let start = s.as_deref().map_or(0.0, numify);
            let end = match e {
                Some(e) if e == "+" => visibility.clone(),
                Some(e) if numify(&e) <= numnames => Some(e),
                _ => Some(perl_compat::truth::format_nv(numnames)),
            };
            (start, end)
        } else {
            (1.0, visibility.clone())
        };
        let nr_end_n = nr_end.as_deref().map_or(0.0, numify);

        let leak = self.quirk(Quirk::PerNameOptionLeak);
        let templates = match cfg.blx.get(None, "labelalphanametemplate", None, None) {
            Some(OptValue::LabelAlphaNameTemplates(m)) => Some(m),
            _ => None,
        };
        let mut pre_strings: Vec<Option<Vec<FieldInfo>>> = Vec::new();
        let mut main_strings: Vec<Option<Vec<FieldInfo>>> = Vec::new();
        let mut main_partnames: Vec<Option<Vec<String>>> = Vec::new();
        for nid in &nl.names {
            let name = names_arena.name(*nid);
            let mut n_lantname = lantname.clone();
            if let Some(v) = name
                .slots
                .get("labelalphanametemplatename")
                .and_then(J::as_str)
            {
                n_lantname = v.to_string();
            }
            let mut n_useprefix = useprefix.clone();
            if let Some(v) = name.slots.get("useprefix").and_then(J::as_str) {
                n_useprefix = Some(v.to_string());
            }
            if leak {
                // BB-019: the overrides persist for the following names
                lantname = n_lantname.clone();
                useprefix = n_useprefix.clone();
            }
            let lnat = templates.and_then(|m| m.get(&n_lantname));
            let mut preacc: Option<Vec<FieldInfo>> = None;
            let mut mainacc: Option<Vec<FieldInfo>> = None;
            let mut mpns: Option<Vec<String>> = None;
            for lnp in lnat.map(Vec::as_slice).unwrap_or(&[]) {
                let npn = lnp.namepart.as_deref().unwrap_or("");
                let np = name
                    .nameparts
                    .get(npn)
                    .and_then(|p| p.string.as_deref())
                    .unwrap_or("");
                if !truthy(Some(np)) {
                    continue;
                }
                if truthy(lnp.use_.as_deref()) {
                    // only useprefix is ever set in $opts
                    let ok = npn == "prefix" && truthy(n_useprefix.as_deref());
                    if !ok {
                        continue;
                    }
                }
                let fi = (
                    normalise_string_label(cfg, np),
                    NpOpts {
                        substring_width: lnp.substring_width.clone(),
                        substring_side: lnp.substring_side.clone(),
                        substring_compound: lnp.substring_compound.clone(),
                    },
                );
                if truthy(lnp.pre.as_deref()) {
                    preacc.get_or_insert_with(Vec::new).push(fi);
                } else {
                    mpns.get_or_insert_with(Vec::new).push(npn.to_string());
                    mainacc.get_or_insert_with(Vec::new).push(fi);
                }
            }
            pre_strings.push(preacc);
            main_strings.push(mainacc);
            main_partnames.push(mpns);
        }

        let mut acc = String::new();
        let nsep = attr(attrs, "namessep").filter(|s| truthy(Some(s)));
        let mut i = nr_start - 1.0;
        while i < nr_end_n {
            let ii = i.trunc() as i64;
            // pre parts
            if let Some(Some(pre)) = perl_idx(pre_strings.len(), ii).map(|k| &pre_strings[k]) {
                for (np, npo) in pre {
                    let w = npo.substring_width.as_deref().map_or(1, iv);
                    if truthy(npo.substring_compound.as_deref()) {
                        for p in split(r"[\s\p{Dash}~]+", np) {
                            acc.push_str(gcstring::substr(&p, 0, Some(w)));
                        }
                    } else {
                        acc.push_str(gcstring::substr(np, 0, Some(w)));
                    }
                }
            }
            let k = perl_idx(main_strings.len(), ii);
            let fs: Vec<FieldInfo> = k.and_then(|k| main_strings[k].clone()).unwrap_or_default();
            let pns: Option<Vec<String>> = k.and_then(|k| main_partnames[k].clone());
            let s = self.process_label_attributes(
                citekey,
                &fs,
                attrs,
                Some(&realname),
                pns.as_deref(),
                Some(ii),
            );
            acc.push_str(&s);
            if let Some(sep) = nsep
                && i != nr_end_n - 1.0
            {
                acc.push_str(sep);
            }
            i += 1.0;
        }
        let mut sortacc = acc.clone();
        if !attr_true(attrs, "noalphaothers") {
            let morenames = nl.slots.get("morenames").and_then(J::as_str);
            if numnames > nr_end_n || truthy(morenames) {
                acc.push_str(&alphaothers);
                sortacc.push_str(&sortalphaothers);
            }
        }
        let u = unescape_label(&sortacc);
        (acc, u)
    }

    /// `_process_label_attributes` (Internals.pm:696).
    fn process_label_attributes(
        &mut self,
        citekey: &str,
        fieldstrings: &[FieldInfo],
        attrs: &XHash,
        field: Option<&str>,
        nameparts: Option<&[String]>,
        index: Option<i64>,
    ) -> String {
        let mut rfield = String::new();
        let field = field.filter(|f| truthy(Some(f)));
        for (fs, npo) in fieldstrings {
            let mut field_string = fs.clone();
            if let Some(sw) = attr(attrs, "substring_width") {
                if let (true, Some(f)) = (sw.contains('v'), field) {
                    field_string = self.label_v(f, &field_string, attrs, nameparts, sw);
                } else if let (true, Some(f)) = (sw.contains('l'), field) {
                    field_string = self.label_l(citekey, f, nameparts, index);
                } else {
                    field_string = self.label_static(field_string, attrs, npo, nameparts.is_some());
                }
            }
            rfield.push_str(&field_string);
        }
        let up = attr_true(attrs, "uppercase");
        let low = attr_true(attrs, "lowercase");
        if up && low {
            // do nothing if both are set
            rfield
        } else if up {
            uc(&rfield)
        } else if low {
            lc(&rfield)
        } else {
            rfield
        }
    }

    /// The static substring width branch of `_process_label_attributes`.
    fn label_static(
        &self,
        mut field_string: String,
        attrs: &XHash,
        npo: &NpOpts,
        is_name: bool,
    ) -> String {
        let padchar = attr(attrs, "pad_char").filter(|s| truthy(Some(s)));
        let mut subs_side = attr(attrs, "substring_side")
            .filter(|s| truthy(Some(s)))
            .unwrap_or("left")
            .to_string();
        let mut subs_width = attr(attrs, "substring_width")
            .filter(|s| truthy(Some(s)))
            .map_or(1, iv);
        if is_name {
            if let Some(w) = npo.substring_width.as_deref().filter(|s| truthy(Some(s))) {
                subs_width = iv(w);
            }
            if let Some(s) = npo.substring_side.as_deref().filter(|s| truthy(Some(s))) {
                subs_side = s.to_string();
            }
        }
        let subs_offset = if subs_side == "right" { -subs_width } else { 0 };

        // nolabelwidthcount: matches don't count against the width; remember
        // their places, remove them, and put them back afterwards
        let nlwc = self.cfg.biber.patterns("nolabelwidthcount");
        let mut nolabelwcis = None;
        if !nlwc.is_empty() {
            let res: Vec<&str> = nlwc
                .iter()
                .map(|p| p.value.as_deref().unwrap_or(""))
                .collect();
            nolabelwcis = match_indices(&res, &field_string);
            for r in &res {
                if let Ok(re) = PerlRegex::cached(r, false)
                    && let Ok(s) = re.remove_all(&field_string)
                {
                    field_string = s;
                }
            }
        }

        if is_name && truthy(npo.substring_compound.as_deref()) {
            let mut tmp = String::new();
            for p in split(r"[\s\p{Dash}]+", &field_string) {
                tmp.push_str(gcstring::substr(&p, subs_offset, Some(subs_width)));
            }
            field_string = tmp;
        } else {
            field_string =
                gcstring::substr(&field_string, subs_offset, Some(subs_width)).to_string();
        }
        // Padding
        if let Some(pc) = padchar {
            let pc = unescape_label(pc);
            let pad_side = attr(attrs, "pad_side")
                .filter(|s| truthy(Some(s)))
                .unwrap_or("right");
            let paddiff = subs_width - gcstring::length(&field_string) as i64;
            if paddiff > 0 {
                let pad = pc.repeat(paddiff as usize);
                if pad_side == "right" {
                    field_string.push_str(&pad);
                } else if pad_side == "left" {
                    field_string = pad + &field_string;
                }
            }
            field_string = escape_label(&field_string);
        }
        // Reinstate nolabelwidthcount matches
        if let Some(wcis) = nolabelwcis {
            let mut gcs: Vec<String> = gcstring::clusters(&field_string)
                .map(str::to_string)
                .collect();
            for (s, off) in wcis {
                let len = gcs.len() as i64;
                // `$off + 1 <= $gc_string->length`
                if off < len {
                    let mut o = off;
                    if o < 0 {
                        o = (o + len).max(0);
                    }
                    let ins: Vec<String> = gcstring::clusters(&s).map(str::to_string).collect();
                    let o = o as usize;
                    gcs.splice(o..o, ins);
                }
            }
            field_string = gcs.concat();
        }
        field_string
    }

    /// The `v` (per-string variable width) branch of
    /// `_process_label_attributes`, with the section's `labelcache_v`.
    fn label_v(
        &mut self,
        field: &str,
        field_string: &str,
        attrs: &XHash,
        nameparts: Option<&[String]>,
        sw: &str,
    ) -> String {
        let ck = self.cache_key(field, nameparts);
        let zero_quirk = self.quirk(Quirk::LabelGlobalIndexZero);
        let root = self
            .slots
            .entry("labelcache_v".to_string())
            .or_insert_with(|| J::Object(Map::new()));
        let exists = root.get(&ck).is_some_and(|v| !v.is_null());
        if !exists {
            let lcache = self.build_cache_v(field, attrs, nameparts, sw);
            let root = self
                .slots
                .entry("labelcache_v".to_string())
                .or_insert_with(|| J::Object(Map::new()));
            if let J::Object(o) = root {
                o.insert(ck.clone(), lcache);
            } else {
                let mut o = Map::new();
                o.insert(ck.clone(), lcache);
                *root = J::Object(o);
            }
        }
        let root = self.slots.get_mut("labelcache_v").expect("labelcache_v");
        let Some(J::Object(lcache)) = root.get_mut(&ck) else {
            return String::new();
        };
        lookup_v(lcache, field_string, zero_quirk).unwrap_or_default()
    }

    /// Build the `v` cache for `field` over all section citekeys.
    fn build_cache_v(
        &mut self,
        field: &str,
        attrs: &XHash,
        nameparts: Option<&[String]>,
        sw: &str,
    ) -> J {
        // The (normalised) strings and their name indices
        let mut indices: BTreeMap<String, J> = BTreeMap::new();
        for key in self.citekeys {
            let Some(be) = self.entries.get(key) else {
                continue;
            };
            let f = be.get_field(field);
            if !value_truthy(f) {
                continue;
            }
            if let Some(nps) = nameparts {
                let Some(Value::Names(nlid)) = f else {
                    continue;
                };
                let vis = visible_alpha(self.dlstate, *nlid);
                for n in first_n_names(self.names, *nlid, vis.as_deref()) {
                    let s: String = nps
                        .iter()
                        .map(|p| {
                            let s = n
                                .nameparts
                                .get(p)
                                .and_then(|x| x.string.as_deref())
                                .unwrap_or("");
                            if truthy(Some(s)) { s } else { "" }
                        })
                        .collect();
                    let k = normalise_string_label(self.cfg, &s);
                    indices.insert(k, n.index.clone().map_or(J::Null, J::from));
                }
            } else if let Some(v) = f {
                indices.insert(value_string(v), J::from("0"));
            }
        }
        let strings: Vec<&String> = indices.keys().collect();
        let maxlen: f64 = match attr(attrs, "substring_width_max").filter(|s| truthy(Some(s))) {
            Some(m) => numify(m),
            None => strings
                .iter()
                .map(|s| gcstring::length(s) as f64)
                .fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.max(x))))
                .unwrap_or(f64::NEG_INFINITY),
        };
        struct VEnt {
            data: Vec<String>,
            index: Option<i64>,
        }
        let mut ents: BTreeMap<&str, VEnt> = BTreeMap::new();
        let mut substr_cache: HashMap<String, u64> = HashMap::new();
        let mut i = 1i64;
        while (i as f64) <= maxlen {
            let maps: Vec<(&str, String)> = strings
                .iter()
                .map(|s| {
                    let sub = gcstring::substr(s, 0, Some(i)).to_string();
                    *substr_cache.entry(sub.clone()).or_insert(0) += 1;
                    (s.as_str(), sub)
                })
                .collect();
            for (s, sub) in maps {
                let e = ents.entry(s).or_insert(VEnt {
                    data: Vec::new(),
                    index: None,
                });
                e.data.push(sub.clone());
                if e.index.is_none() && (substr_cache[&sub] == 1 || i as f64 == maxlen) {
                    e.index = Some(gcstring::length(&sub) as i64 - 1);
                }
            }
            i += 1;
        }
        let mut lcache = Map::new();
        for (s, e) in &ents {
            let mut o = Map::new();
            o.insert(
                "data".into(),
                J::Array(e.data.iter().map(|x| J::from(x.as_str())).collect()),
            );
            o.insert("nameindex".into(), indices[*s].clone());
            if let Some(ix) = e.index {
                o.insert("index".into(), J::from(ix.to_string()));
            }
            lcache.insert((*s).to_string(), J::Object(o));
        }
        // `f`: one width per name position: the largest unique width that
        // occurs at least substring_fixed_threshold times
        if sw.contains('f') {
            let threshold = attr(attrs, "substring_fixed_threshold").map_or(1.0, numify);
            let jstr = |v: Option<&J>| v.and_then(J::as_str).unwrap_or("").to_string();
            let mut is: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
            for v in lcache.values() {
                *is.entry(jstr(v.get("nameindex")))
                    .or_default()
                    .entry(jstr(v.get("index")))
                    .or_insert(0) += 1;
            }
            let mut gi = Map::new();
            for s in lcache.keys() {
                let sidx = numify(indices[s].as_str().unwrap_or(""));
                for (ind, counts) in &is {
                    if sidx != numify(ind) {
                        continue;
                    }
                    let best = counts
                        .iter()
                        .filter(|(_, c)| **c as f64 >= threshold)
                        .map(|(k, _)| k)
                        .fold(None, |a: Option<&String>, k| match a {
                            Some(a) if numify(a) >= numify(k) => Some(a),
                            _ => Some(k),
                        });
                    gi.insert(s.clone(), best.map_or(J::Null, |k| J::from(k.as_str())));
                }
            }
            lcache.insert("globalindices".into(), J::Object(gi));
        }
        J::Object(lcache)
    }

    /// The `l` (list disambiguation) branch of `_process_label_attributes`,
    /// with the section's `labelcache_l`.
    fn label_l(
        &mut self,
        citekey: &str,
        field: &str,
        nameparts: Option<&[String]>,
        index: Option<i64>,
    ) -> String {
        let ck = self.cache_key(field, nameparts);
        let nindex = self.run.first_index(self.secnum, self.citekeys, citekey);
        let exists = self
            .slots
            .get("labelcache_l")
            .and_then(|r| r.get(&ck))
            .is_some_and(|v| !v.is_null());
        if !exists {
            let mut strings: Vec<Vec<String>> = Vec::with_capacity(self.citekeys.len());
            for key in self.citekeys {
                let f = self.entries.get(key).and_then(|be| be.get_field(field));
                if !value_truthy(f) {
                    strings.push(vec![String::new()]);
                    continue;
                }
                match (nameparts, f) {
                    (Some(nps), Some(Value::Names(nlid))) => {
                        let vis = visible_alpha(self.dlstate, *nlid);
                        strings.push(
                            first_n_names(self.names, *nlid, vis.as_deref())
                                .map(|n| {
                                    nps.iter()
                                        .map(|p| {
                                            let s = n
                                                .nameparts
                                                .get(p)
                                                .and_then(|x| x.string.as_deref())
                                                .unwrap_or("");
                                            if truthy(Some(s)) { s } else { "" }
                                        })
                                        .collect::<String>()
                                })
                                .collect(),
                        );
                    }
                    (_, Some(v)) => strings.push(vec![value_string(v)]),
                    (_, None) => strings.push(vec![String::new()]),
                }
            }
            let data = label_listdisambiguation(&strings);
            let mut o = Map::new();
            o.insert(
                "data".into(),
                J::Array(
                    data.into_iter()
                        .map(|r| J::Array(r.into_iter().map(J::from).collect()))
                        .collect(),
                ),
            );
            let root = self
                .slots
                .entry("labelcache_l".to_string())
                .or_insert_with(|| J::Object(Map::new()));
            if !root.is_object() {
                *root = J::Object(Map::new());
            }
            if let J::Object(r) = root {
                r.insert(ck.clone(), J::Object(o));
            }
        }
        let Some(J::Object(lcache)) = self
            .slots
            .get_mut("labelcache_l")
            .and_then(|r| r.get_mut(&ck))
        else {
            return String::new();
        };
        // $lcache->{data}[$nindex][$index] (autovivifies data[$nindex])
        let data = lcache
            .entry("data".to_string())
            .or_insert_with(|| J::Array(Vec::new()));
        if !data.is_array() {
            *data = J::Array(Vec::new());
        }
        let J::Array(rows) = data else { unreachable!() };
        let ni = if nindex < 0 {
            perl_idx(rows.len(), nindex)
        } else {
            let ni = nindex as usize;
            if rows.len() <= ni {
                rows.resize(ni + 1, J::Null);
            }
            Some(ni)
        };
        let Some(ni) = ni else { return String::new() };
        if rows[ni].is_null() {
            rows[ni] = J::Array(Vec::new());
        }
        let J::Array(row) = &rows[ni] else {
            return String::new();
        };
        perl_idx(row.len(), index.unwrap_or(0))
            .and_then(|j| row[j].as_str())
            .unwrap_or("")
            .to_string()
    }
}

/// `$names->first_n_names($n)` (Names.pm:178).
fn first_n_names<'a>(
    arena: &'a NameArena,
    nlid: NameListId,
    n: Option<&str>,
) -> impl Iterator<Item = &'a crate::model::Name> + 'a {
    let list = arena.list(nlid);
    let size = list.names.len() as i64;
    let n = n.map_or(0, iv);
    let last = if n > size { size - 1 } else { n - 1 };
    let take = if last < 0 { 0 } else { (last + 1) as usize };
    list.names.iter().take(take).map(move |id| arena.name(*id))
}

/// The `v` cache lookup:
/// `${$lcache->{$s}{data}}[$lcache->{globalindices}{$s} || $lcache->{$s}{index}]`,
/// with Perl's autovivification of `globalindices` and `$lcache->{$s}{data}`.
///
/// With `zero_quirk` (BB-096, `label-global-index-zero`) a global index of
/// `0` is ignored (`||`); fixed, any defined global index is used.
fn lookup_v(lcache: &mut Map<String, J>, s: &str, zero_quirk: bool) -> Option<String> {
    let gi = lcache
        .entry("globalindices".to_string())
        .or_insert_with(|| J::Object(Map::new()))
        .get(s)
        .and_then(J::as_str)
        .filter(|x| !zero_quirk || truthy(Some(x)))
        .map(str::to_string);
    let ent = lcache
        .entry(s.to_string())
        .or_insert_with(|| J::Object(Map::new()));
    if !ent.is_object() {
        return None;
    }
    let J::Object(ent) = ent else { unreachable!() };
    let idx = gi
        .or_else(|| ent.get("index").and_then(J::as_str).map(str::to_string))
        .map_or(0, |x| iv(&x));
    let data = ent
        .entry("data".to_string())
        .or_insert_with(|| J::Array(Vec::new()));
    let J::Array(d) = data else { return None };
    perl_idx(d.len(), idx).and_then(|i| d[i].as_str().map(str::to_string))
}

/// Upper bound on disambiguation rounds: biber's loop has no termination
/// guarantee (and hangs on some inputs); we stop and keep the current
/// substrings.
const MAX_LD_ROUNDS: usize = 100_000;

/// `_label_listdisambiguation` (Internals.pm:927) with `_do_substr`,
/// `_check_counts` and `_gen_first_disambiguating_name_map`: the shortest
/// per-name prefixes that make each name list's label unique.
pub fn label_listdisambiguation(strings: &[Vec<String>]) -> Vec<Vec<String>> {
    let n = strings.len();
    // substr_map: per list, per name, the substring width
    let mut substr_map: Vec<Vec<i64>> = strings.iter().map(|r| vec![1; r.len()]).collect();
    let mut data: Vec<Option<Vec<String>>> = vec![None; n];
    let mut name_map: Vec<Option<i64>> = vec![None; n];

    struct KeyInfo {
        index: usize,
        indices: Vec<usize>,
        strings: Vec<String>,
    }
    // _do_substr
    let do_substr = |data: &[Option<Vec<String>>], substr_map: &[Vec<i64>]| {
        let mut keys: BTreeMap<String, KeyInfo> = BTreeMap::new();
        for (i, row) in strings.iter().enumerate() {
            if data[i].is_some() {
                continue;
            }
            let s: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(j, x)| {
                    let w = substr_map[i].get(j).copied().unwrap_or(0);
                    gcstring::substr(x, 0, Some(w)).to_string()
                })
                .collect();
            let js = s.concat();
            let e = keys.entry(js).or_insert(KeyInfo {
                index: i,
                indices: Vec::new(),
                strings: Vec::new(),
            });
            e.index = i;
            e.indices.push(i);
            e.strings = s;
        }
        keys
    };
    let mut keys = do_substr(&data, &substr_map);
    let mut rounds = 0;
    while data.iter().any(Option::is_none) {
        rounds += 1;
        if rounds > MAX_LD_ROUNDS {
            for (_, k) in keys {
                for i in k.indices {
                    data[i].get_or_insert_with(|| k.strings.clone());
                }
            }
            for (i, d) in data.iter_mut().enumerate() {
                d.get_or_insert_with(|| strings[i].clone());
            }
            break;
        }
        // _check_counts
        let mut ambiguity: Vec<Vec<usize>> = Vec::new();
        for (_, k) in std::mem::take(&mut keys) {
            if k.indices.len() > 1 {
                ambiguity.push(k.indices);
            } else {
                data[k.index] = Some(k.strings);
            }
        }
        for amb in &ambiguity {
            let rows: Vec<&Vec<String>> = amb.iter().map(|&i| &strings[i]).collect();
            if rows.iter().all(|r| *r == rows[0]) {
                // all lists identical: 1-char substrings
                data[amb[0]] = Some(
                    rows[0]
                        .iter()
                        .map(|x| gcstring::substr(x, 0, Some(1)).to_string())
                        .collect(),
                );
            } else {
                // _gen_first_disambiguating_name_map
                let len = rows[0].len();
                let get =
                    |r: &Vec<String>, j: usize| r.get(j).map_or("", String::as_str).to_string();
                for (i, ri) in rows.iter().enumerate() {
                    let check: Vec<&&Vec<String>> = rows
                        .iter()
                        .enumerate()
                        .filter(|(k, r)| *k != i && **r != *ri)
                        .map(|(_, r)| r)
                        .collect();
                    for j in 0..len {
                        let mine = get(ri, j);
                        if !check.iter().any(|r| get(r, j) == mine) {
                            name_map[amb[i]] = Some(j as i64);
                            break;
                        }
                    }
                }
                // increment the substr map at the disambiguating position
                let first = amb[0];
                let pos = name_map[first].unwrap_or(0);
                let row = &mut substr_map[first];
                let p = if pos < 0 { pos + row.len() as i64 } else { pos };
                if p >= 0 {
                    let p = p as usize;
                    if row.len() <= p {
                        // autovivified beyond the row: no effect on substrings
                        row.resize(p + 1, 0);
                    }
                    row[p] += 1;
                }
            }
            keys = do_substr(&data, &substr_map);
        }
        if ambiguity.is_empty() {
            keys = do_substr(&data, &substr_map);
        }
    }
    data.into_iter().map(Option::unwrap_or_default).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn listdisambiguation_biber_example() {
        // The example from Internals.pm:880-925
        let input = v(&[
            &["Agassi", "Chang", "Laver", "bob"],
            &["Agassi", "Chang", "Laver"],
            &["Agassi", "Chang", "Laver"],
            &["Agassi", "Connors", "Lendl"],
            &["Agassi", "Courier", "Laver"],
            &["Borg", "Connors", "Edberg"],
            &["Borg", "Connors", "Emerson"],
            &["Becker", "Connors", "Emerson"],
            &["Becker"],
            &["Zoo", "Xaa"],
            &["Zoo", "Xaa"],
            &["Zaa"],
            &["Abc", "Abc", "Abc"],
            &["Abc", "Abc", "Abc"],
            &["Abc", "Abc", "Abc"],
        ]);
        let want = v(&[
            &["A", "C", "L", "b"],
            &["A", "Ch", "L"],
            &["A", "Ch", "L"],
            &["A", "Co", "L"],
            &["A", "C", "L"],
            &["B", "C", "Ed"],
            &["Bo", "C", "E"],
            &["B", "C", "E"],
            &["B"],
            &["Z", "X"],
            &["Z", "X"],
            &["Z"],
            &["A", "A", "A"],
            &["A", "A", "A"],
            &["A", "A", "A"],
        ]);
        assert_eq!(label_listdisambiguation(&input), want);
    }

    #[test]
    fn escapes() {
        assert_eq!(
            escape_label("a_b~c<d"),
            "a\\_b{\\textasciitilde}c{\\textless}d"
        );
        assert_eq!(
            unescape_label("a\\_b{\\textasciitilde}c{\\textless}d"),
            "a_b~c<d"
        );
        assert_eq!(escape_label("0"), "");
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("1-3"), (Some("1".into()), Some("3".into())));
        assert_eq!(parse_range("2"), (Some("1".into()), Some("2".into())));
        assert_eq!(parse_range("-3"), (Some("1".into()), Some("3".into())));
        assert_eq!(parse_range("2-"), (Some("2".into()), None));
        assert_eq!(parse_range_alt("2"), (None, None));
        assert_eq!(parse_range_alt("-3"), (None, Some("3".into())));
    }
}

//! Per-entry processing: `process_entries_static` (labelname, labeldate,
//! labeltitle, nocite, presort), the per-list `process_entries_pre`
//! (name disambiguation strings), `process_visible_names`,
//! `process_entries_post` (hashes, extra* and work-uniqueness bookkeeping;
//! labelalpha is delegated to [`super::labelalpha`]) and
//! `process_entries_final` (singletitle, uniquetitle, …).
//!
//! A port of Biber.pm:1771-2900 and 3020-3076 and of the hash functions of
//! `Biber::Internals` (Internals.pm:39-311). Per-list state lives in
//! [`DataList::state`] in the exact shape of biber's `$dlist->{state}`
//! (see [`ls`], which reproduces the autovivification of `DataList.pm`'s
//! accessors). Name-list state is keyed by name-list/name id, so name lists
//! shared between entries share it, last writer winning (BB-026).

use super::{Ctx, labelalpha};
use crate::bcf::templates::{NamehashPart, UniquenamePart};
use crate::biber_error;
use crate::config::{Config, OptValue};
use crate::datamodel::DataModel;
use crate::log::{Fatal, Logger};
use crate::model::{DataList, Entry, Name, NameId, NameListId, Section, SectionState, Value};
use crate::perlish::{fc, lc, nfc, numify, truthy};
use crate::quirks::Quirk;
use md5::{Digest, Md5};
use perl_compat::regex::PerlRegex;
use perl_compat::unicode::{Prop, is_perl_space};
use serde_json::{Map, Value as J};

// ---------------------------------------------------------------------------
// Per-list state access (`Biber::DataList` accessors)
// ---------------------------------------------------------------------------

/// Access to `$dlist->{state}` with Perl's autovivification: an rvalue
/// fetch `$self->{state}{a}{b}{c}` creates the intermediate hashes `a` and
/// `a.b` (DataList.pm has no `no autovivification`), and those empty hashes
/// show in biber's dumps.
pub(crate) mod ls {
    use crate::model::DataList;
    use serde_json::{Map, Value as J};

    /// `$self->{state}` (created if needed).
    pub fn root(dl: &mut DataList) -> &mut J {
        let s = dl.state.get_or_insert_with(|| J::Object(Map::new()));
        if !s.is_object() {
            *s = J::Object(Map::new());
        }
        s
    }

    /// Walk `path`, creating hashes on the way; returns the (hash) value at
    /// the end of the path.
    pub fn viv<'a>(mut v: &'a mut J, path: &[&str]) -> &'a mut J {
        for k in path {
            if !v.is_object() {
                *v = J::Object(Map::new());
            }
            v = v
                .as_object_mut()
                .expect("object")
                .entry(k.to_string())
                .or_insert_with(|| J::Object(Map::new()));
        }
        if !v.is_object() {
            *v = J::Object(Map::new());
        }
        v
    }

    /// rvalue `$self->{state}{p0}…{pn}`: vivifies `p0…p(n-1)`; `undef` is
    /// `None`.
    pub fn get(dl: &mut DataList, path: &[&str]) -> Option<J> {
        let (last, parents) = path.split_last().expect("non-empty path");
        viv(root(dl), parents)
            .get(*last)
            .filter(|x| !x.is_null())
            .cloned()
    }

    /// `exists $self->{state}{p0}…{pn}` (vivifies `p0…p(n-1)`).
    pub fn exists(dl: &mut DataList, path: &[&str]) -> bool {
        let (last, parents) = path.split_last().expect("non-empty path");
        viv(root(dl), parents)
            .as_object()
            .is_some_and(|o| o.contains_key(*last))
    }

    /// `$self->{state}{p0}…{pn} = $v`.
    pub fn set(dl: &mut DataList, path: &[&str], v: J) {
        let (last, parents) = path.split_last().expect("non-empty path");
        viv(root(dl), parents)
            .as_object_mut()
            .expect("object")
            .insert(last.to_string(), v);
    }

    /// `++$self->{state}{p0}…{pn}`; returns the new value.
    pub fn incr(dl: &mut DataList, path: &[&str]) -> i64 {
        let n = get(dl, path).map_or(0.0, |v| num(&v)) as i64 + 1;
        set(dl, path, J::String(n.to_string()));
        n
    }

    /// Perl numeric value of a state scalar.
    pub fn num(v: &J) -> f64 {
        match v {
            J::String(s) => crate::perlish::numify(s),
            J::Number(n) => n.as_f64().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    /// `get_entryfield($citekey, $f)`.
    pub fn entryfield(dl: &mut DataList, key: &str, f: &str) -> Option<J> {
        get(dl, &["fields", key, f])
    }

    /// `set_entryfield($citekey, $f, $v)`.
    pub fn set_entryfield(dl: &mut DataList, key: &str, f: &str, v: J) {
        set(dl, &["fields", key, f], v);
    }

    /// A state scalar as a string (`undef` → `None`).
    pub fn str_of(v: &Option<J>) -> Option<String> {
        match v {
            Some(J::String(s)) => Some(s.clone()),
            Some(J::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    }

    /// Perl truthiness of a state value.
    pub fn truthy(v: &Option<J>) -> bool {
        match v {
            None | Some(J::Null) => false,
            Some(J::String(s)) => crate::perlish::truthy(Some(s)),
            Some(J::Bool(b)) => *b,
            Some(J::Number(n)) => n.as_f64() != Some(0.0),
            Some(_) => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// `%UNIQUENAME_CONTEXTS` (Constants.pm:223).
fn uniquename_context(un: &str) -> Option<&'static str> {
    Some(match un {
        "false" => "none",
        "init" | "allinit" | "mininit" | "minyearinit" => "init",
        "full" | "allfull" | "minfull" | "minyearfull" => "initorfull",
        _ => return None,
    })
}

/// `%UNIQUENAME_VALUES` (Constants.pm:234).
pub(crate) fn uniquename_value(v: &str) -> Option<i64> {
    Some(match v {
        "none" => 0,
        "init" => 1,
        "full" => 2,
        _ => return None,
    })
}

/// Perl truthiness of a field value (references are true).
pub(crate) fn value_true(v: Option<&Value>) -> bool {
    match v {
        None => false,
        Some(Value::Str(s)) => truthy(Some(s)),
        Some(_) => true,
    }
}

/// A field value in Perl string context. A reference stringifies to its
/// class and memory address in biber (`Biber::Entry::Names=HASH(0x…)`), so
/// e.g. a name list used as `labeltitle` makes extratitleyear and work
/// uniqueness compare name-list *identity*. bibers uses a fixed fake address
/// per name list (same identity semantics, deterministic); other references
/// stringify as `ARRAY` (cf. DC-039).
pub(crate) fn perl_str(v: Option<&Value>) -> String {
    match v {
        None => String::new(),
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Names(id)) => format!(
            "Biber::Entry::Names=HASH(0x{:x})",
            0x1000_0000 + id.0 as u64 * 0x18
        ),
        Some(_) => "ARRAY".to_string(),
    }
}

/// A defined scalar slot (`undef` → `None`).
fn slot_str<'a>(slots: &'a crate::model::Slots, k: &str) -> Option<&'a str> {
    slots.get(k).and_then(J::as_str)
}

/// The string value of an option (`undef`/reference → `None`).
fn opt_string(v: Option<&OptValue>) -> Option<String> {
    v.and_then(OptValue::as_str).map(str::to_string)
}

fn opt_true(v: Option<&OptValue>) -> bool {
    v.is_some_and(OptValue::truthy)
}

/// `md5_hex(encode_utf8($s))`.
pub(crate) fn md5_hex(s: &str) -> String {
    format!("{:x}", Md5::digest(s.as_bytes()))
}

/// `md5_hex(encode_utf8(NFC(normalise_string_hash($s))))`, the form of every
/// name/context hash.
pub(crate) fn hash_string(s: &str) -> String {
    md5_hex(&nfc(&normalise_string_hash(s)))
}

/// `Biber::Utils::normalise_string_hash` (Utils.pm:736).
pub(crate) fn normalise_string_hash(s: &str) -> String {
    if !truthy(Some(s)) {
        return String::new();
    }
    let is_l = |c: char| Prop::L.contains(c);
    // s/\\(\p{L}+)\s*/$1:/g
    let cs: Vec<char> = s.chars().collect();
    let mut a = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '\\' && cs.get(i + 1).is_some_and(|c| is_l(*c)) {
            i += 1;
            while i < cs.len() && is_l(cs[i]) {
                a.push(cs[i]);
                i += 1;
            }
            a.push(':');
            while i < cs.len() && is_perl_space(cs[i]) {
                i += 1;
            }
        } else {
            a.push(cs[i]);
            i += 1;
        }
    }
    // s/\\([^\p{L}])\s*/ord($1).':'/ge
    let cs: Vec<char> = a.chars().collect();
    let mut b = String::with_capacity(a.len());
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '\\' && cs.get(i + 1).is_some_and(|c| !is_l(*c)) {
            b.push_str(&(cs[i + 1] as u32).to_string());
            b.push(':');
            i += 2;
            while i < cs.len() && is_perl_space(cs[i]) {
                i += 1;
            }
        } else {
            b.push(cs[i]);
            i += 1;
        }
    }
    // s/[\{\}~\.\s]+//g
    b.chars()
        .filter(|&c| !matches!(c, '{' | '}' | '~' | '.') && !is_perl_space(c))
        .collect()
}

/// `Biber::Utils::strip_nonamestring` (Utils.pm:599): apply the
/// `nonamestring` regexes for `field` (by field name or datafield set).
pub(crate) fn strip_nonamestring(cfg: &Config, s: &str, field: Option<&str>) -> String {
    if !truthy(Some(s)) {
        return String::new();
    }
    let pats = cfg.biber.patterns("nonamestring");
    if pats.is_empty() {
        return s.to_string();
    }
    let ffield = fc(field.unwrap_or(""));
    let mut out = s.to_string();
    for p in pats {
        let name = p.name.as_deref().unwrap_or("");
        let applies = fc(name) == ffield
            || cfg
                .datafield_set(&lc(name))
                .is_some_and(|set| set.iter().any(|f| fc(f) == ffield));
        if !applies {
            continue;
        }
        if let Ok(re) = PerlRegex::cached(p.value.as_deref().unwrap_or(""), false)
            && let Ok(r) = re.remove_all(&out)
        {
            out = r;
        }
    }
    out
}

/// `Name::get_namepart`: `string || ''`.
fn namepart(n: &Name, np: &str) -> String {
    n.nameparts
        .get(np)
        .and_then(|p| p.string.as_deref())
        .filter(|s| truthy(Some(s)))
        .unwrap_or("")
        .to_string()
}

/// `join('', $n->get_namepart_initial($np)->@*)`.
fn namepart_initials(n: &Name, np: &str) -> String {
    n.nameparts
        .get(np)
        .and_then(|p| p.initial.as_ref())
        .map(|v| v.concat())
        .unwrap_or_default()
}

/// `Name::get_hash_namepart` (Name.pm:137).
fn hash_namepart(n: &Name, np: &str, nht: &[NamehashPart]) -> String {
    let fnp = fc(np);
    for p in nht {
        if fc(p.namepart.as_deref().unwrap_or("")) != fnp {
            continue;
        }
        let scope = fc(p.hashscope.as_deref().unwrap_or(""));
        if scope == "init" {
            return n
                .nameparts
                .get(np)
                .and_then(|x| x.initial.as_ref())
                .map(|v| v.concat())
                .unwrap_or_default();
        } else if scope == "full" {
            return namepart(n, np);
        }
    }
    String::new()
}

/// Read-only processing context shared by the helpers.
pub(crate) struct H<'a> {
    pub cfg: &'a Config,
    pub dm: &'a DataModel,
    pub secnum: &'a str,
}

impl H<'_> {
    /// `getblxoption($secnum, …)`.
    fn opt(&self, opt: &str, et: Option<&str>, key: Option<&str>) -> Option<&OptValue> {
        self.cfg.blx.get(Some(self.secnum), opt, et, key)
    }

    /// `getblxoption(undef, $opt, $bee, $citekey)`: per-entry values are
    /// invisible with the `entry-scoped-options-ignored` quirk (BB-021).
    fn opt_undef(&self, opt: &str, et: Option<&str>, key: Option<&str>) -> Option<&OptValue> {
        self.cfg
            .blx
            .get_undef_secnum(self.cfg.quirks, self.secnum, opt, et, key)
    }

    fn leak(&self) -> bool {
        self.cfg.quirks.on(Quirk::PerNameOptionLeak)
    }

    /// `$dm->get_constant_value('nameparts')`.
    fn nameparts(&self) -> Vec<String> {
        self.dm.get_constant_value("nameparts")
    }

    fn namehash_template(&self, name: &str) -> Option<&[NamehashPart]> {
        match self.cfg.blx.global("namehashtemplate") {
            Some(OptValue::NamehashTemplates(m)) => m.get(name).map(Vec::as_slice),
            _ => None,
        }
    }

    fn uniquename_template(&self, name: &str) -> Option<&[UniquenamePart]> {
        match self.cfg.blx.global("uniquenametemplate") {
            Some(OptValue::UniquenameTemplates(m)) => m.get(name).map(Vec::as_slice),
            _ => None,
        }
    }
}

/// `first_n_names($n)`: how many names are taken.
fn first_n(visible: f64, size: usize) -> usize {
    if visible > size as f64 {
        size
    } else if visible >= 1.0 {
        visible as usize
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Hashes (Internals.pm:39-311)
// ---------------------------------------------------------------------------

/// The per-namelist `nohashothers` override chain.
fn nohashothers(h: &H, sec: &Section, et: &str, key: &str, nlid: NameListId) -> bool {
    let mut nho = opt_true(h.opt("nohashothers", Some(et), Some(key)));
    if let Some(v) = slot_str(&sec.names.list(nlid).slots, "nohashothers") {
        nho = truthy(Some(v));
    }
    nho
}

/// The `namehashtemplatename` in force for a list: per-entry, else the
/// data list's, then the per-namelist override.
fn namehash_tname(h: &H, sec: &Section, dl: &DataList, key: &str, nlid: NameListId) -> String {
    let mut t = opt_string(h.opt("namehashtemplatename", None, Some(key)))
        .unwrap_or_else(|| dl.namehashtemplatename.clone());
    if let Some(v) = slot_str(&sec.names.list(nlid).slots, "namehashtemplatename") {
        t = v.to_string();
    }
    t
}

/// The namehash template `name`, or `biber_error`.
fn need_nht<'a>(h: &'a H, log: &mut Logger, name: &str) -> Result<&'a [NamehashPart], Fatal> {
    match h.namehash_template(name) {
        Some(t) => Ok(t),
        None => {
            biber_error!(log, "No namehash template called '{name}'")?;
            Ok(&[])
        }
    }
}

/// `_getnamehash` (Internals.pm:39): namehash (`bib` false) or bibnamehash.
/// A per-name `namehashtemplatename` leaks to the following names (BB-019).
pub(crate) fn getnamehash(
    h: &H,
    log: &mut Logger,
    sec: &Section,
    dl: &mut DataList,
    key: &str,
    nlid: NameListId,
    bib: bool,
) -> Result<String, Fatal> {
    let et = sec.bibentry(key).map_or("", Entry::entrytype);
    let names = sec.names.list(nlid);
    let count = names.names.len();
    let visible = ls::get(
        dl,
        &[
            "namelistdata",
            &nlid.key(),
            if bib { "visbib" } else { "viscite" },
        ],
    )
    .map_or(0.0, |v| ls::num(&v));
    let nps = h.nameparts();
    let mut nhtname = namehash_tname(h, sec, dl, key, nlid);
    let mut hashkey = String::new();
    for nid in names.names.iter().take(first_n(visible, count)) {
        let n = sec.names.name(*nid);
        if let Some(hid) = slot_str(&n.slots, "hashid").filter(|s| truthy(Some(s))) {
            hashkey.push_str(hid);
            continue;
        }
        let mut n_nht = nhtname.clone();
        if let Some(v) = slot_str(&n.slots, "namehashtemplatename") {
            n_nht = v.to_string();
        }
        if h.leak() {
            nhtname = n_nht.clone();
        }
        let nht = need_nht(h, log, &n_nht)?;
        for nt in &nps {
            hashkey.push_str(&hash_namepart(n, nt, nht));
        }
    }
    if !nohashothers(h, sec, et, key, nlid)
        && ((visible < count as f64) || truthy(slot_str(&names.slots, "morenames")))
    {
        hashkey.push('+');
    }
    Ok(hash_string(&hashkey))
}

/// `_getfullhash` (Internals.pm:104).
pub(crate) fn getfullhash(
    h: &H,
    log: &mut Logger,
    sec: &Section,
    dl: &DataList,
    key: &str,
    nlid: NameListId,
) -> Result<String, Fatal> {
    let names = sec.names.list(nlid);
    let nps = h.nameparts();
    let mut nhtname = namehash_tname(h, sec, dl, key, nlid);
    let mut hashkey = String::new();
    for nid in &names.names {
        let n = sec.names.name(*nid);
        if let Some(hid) = slot_str(&n.slots, "hashid").filter(|s| truthy(Some(s))) {
            hashkey.push_str(hid);
            continue;
        }
        let mut n_nht = nhtname.clone();
        if let Some(v) = slot_str(&n.slots, "namehashtemplatename") {
            n_nht = v.to_string();
        }
        if h.leak() {
            nhtname = n_nht.clone();
        }
        let nht = need_nht(h, log, &n_nht)?;
        for nt in &nps {
            hashkey.push_str(&strip_nonamestring(
                h.cfg,
                &hash_namepart(n, nt, nht),
                names.type_.as_deref(),
            ));
        }
    }
    if truthy(slot_str(&names.slots, "morenames")) {
        hashkey.push('+');
    }
    Ok(hash_string(&hashkey))
}

/// `_getfullhashraw` (Internals.pm:154).
pub(crate) fn getfullhashraw(h: &H, sec: &Section, nlid: NameListId) -> String {
    let names = sec.names.list(nlid);
    let nps = h.nameparts();
    let mut hashkey = String::new();
    for n in sec.names.names_of(nlid) {
        for nt in &nps {
            let np = namepart(n, nt);
            if truthy(Some(&np)) {
                hashkey.push_str(&strip_nonamestring(h.cfg, &np, names.type_.as_deref()));
            }
        }
    }
    if truthy(slot_str(&names.slots, "morenames")) {
        hashkey.push('+');
    }
    hash_string(&hashkey)
}

/// `_getnamehash_u` (Internals.pm:180): the namehash under the uniquename
/// template, for extra* tracking.
pub(crate) fn getnamehash_u(
    h: &H,
    sec: &Section,
    dl: &mut DataList,
    key: &str,
    nlid: NameListId,
) -> String {
    let et = sec.bibentry(key).map_or("", Entry::entrytype);
    let names = sec.names.list(nlid);
    let count = names.names.len();
    let nlkey = nlid.key();
    let visible = ls::get(dl, &["namelistdata", &nlkey, "viscite"]).map_or(0.0, |v| ls::num(&v));
    let mut untname = opt_string(h.opt("uniquenametemplatename", None, Some(key)))
        .unwrap_or_else(|| dl.uniquenametemplatename.clone());
    if let Some(v) = slot_str(&names.slots, "uniquenametemplatename") {
        untname = v.to_string();
    }
    let mut hashkey = String::new();
    for nid in names.names.iter().take(first_n(visible, count)) {
        let n = sec.names.name(*nid);
        let mut n_unt = untname.clone();
        if let Some(v) = slot_str(&n.slots, "uniquenametemplatename") {
            n_unt = v.to_string();
        }
        if h.leak() {
            untname = n_unt.clone();
        }
        for nps in h.uniquename_template(&n_unt).unwrap_or(&[]) {
            if nps.disambiguation.as_deref() == Some("none") {
                continue;
            }
            let npn = nps.namepart.as_deref().unwrap_or("");
            let np = namepart(n, npn);
            if !truthy(Some(&np)) {
                continue;
            }
            if truthy(nps.base.as_deref()) {
                hashkey.push_str(&np);
            } else {
                let un = ls::get(dl, &["namelistdata", &nlkey, &nid.key(), "un"]);
                if let Some(J::Array(un)) = un
                    && un.first().and_then(J::as_str) != Some("base")
                {
                    match un.get(1).and_then(J::as_str) {
                        Some("full" | "fullonly") => hashkey.push_str(&np),
                        Some("init") => hashkey.push_str(&namepart_initials(n, npn)),
                        _ => {}
                    }
                }
            }
        }
    }
    if !nohashothers(h, sec, et, key, nlid)
        && ((visible < count as f64) || truthy(slot_str(&names.slots, "morenames")))
    {
        hashkey.push('+');
    }
    hash_string(&hashkey)
}

/// `_genpnhash` (Internals.pm:256): the per-name hash.
pub(crate) fn genpnhash(
    h: &H,
    log: &mut Logger,
    sec: &Section,
    dl: &DataList,
    key: &str,
    nlid: NameListId,
    nid: NameId,
) -> Result<String, Fatal> {
    let n = sec.names.name(nid);
    if let Some(hid) = slot_str(&n.slots, "hashid").filter(|s| truthy(Some(s))) {
        return Ok(hash_string(hid));
    }
    let mut nhtname = namehash_tname(h, sec, dl, key, nlid);
    if let Some(v) = slot_str(&n.slots, "namehashtemplatename") {
        nhtname = v.to_string();
    }
    let nht = need_nht(h, log, &nhtname)?;
    let mut hashkey = String::new();
    for nt in h.nameparts() {
        hashkey.push_str(&hash_namepart(n, &nt, nht));
    }
    Ok(hash_string(&hashkey))
}

// ---------------------------------------------------------------------------
// process_entries_static
// ---------------------------------------------------------------------------

/// `Biber::process_entries_static` (Biber.pm:1966): nocite, labelname,
/// labeldate, labeltitle and presort for every citekey.
pub fn process_entries_static(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    let secnum = st.section.number.clone();
    for key in st.section.citekeys.clone() {
        if st.section.bibentry(&key).is_none() {
            continue;
        }
        process_nocite(&mut st.section, &key);
        process_labelname(ctx, &mut st.section, &secnum, &key);
        process_labeldate(ctx, &mut st.section, &secnum, &key);
        process_labeltitle(ctx, &mut st.section, &key);
        process_presort(ctx, &st.section, &secnum, &key);
    }
    Ok(())
}

/// `process_nocite` (Biber.pm:2530).
fn process_nocite(sec: &mut Section, key: &str) {
    let specific = sec.cite_citekeys.contains(key) || sec.nocite_citekeys.contains(key);
    if sec.nocite_citekeys.contains(key) || (sec.allkeys_nocite && !specific) {
        if let Some(be) = sec.bibentry_mut(key) {
            be.set_field("nocite", Some(Value::Str("1".into())));
        }
    }
}

/// `process_labelname` (Biber.pm:2549).
fn process_labelname(ctx: &mut Ctx, sec: &mut Section, secnum: &str, key: &str) {
    let cfg = &*ctx.cfg;
    let be = sec.bibentry(key).expect("entry");
    let et = be.entrytype().to_string();
    let spec: Vec<String> = match cfg.blx.get(None, "labelnamespec", Some(&et), None) {
        Some(OptValue::Multi(v)) => v
            .iter()
            .map(|h| h.content().unwrap_or("").to_string())
            .collect(),
        _ => Vec::new(),
    };
    let namelistsall = &ctx.dm.helpers.namelistsall;
    let use_ok = |opt: &str| {
        !cfg.blx.registry.is_declared(&format!("use{opt}"))
            || opt_true(
                cfg.blx
                    .get(Some(secnum), &format!("use{opt}"), Some(&et), Some(key)),
            )
    };
    let mut lni = None;
    for ln in &spec {
        let lnameopt = ln
            .strip_prefix("short")
            .filter(|r| !r.is_empty())
            .unwrap_or(ln);
        if !namelistsall.iter().any(|n| n == ln) {
            ctx.log.biber_warn(format!(
                "Labelname candidate '{ln}' is not a name field - skipping"
            ));
            continue;
        }
        if !use_ok(lnameopt) {
            continue;
        }
        if value_true(be.get_field(ln)) {
            lni = Some(ln.clone());
            break;
        }
    }
    let mut lnfhi = None;
    for ln in &spec {
        if ln.strip_prefix("short").is_some_and(|r| !r.is_empty()) {
            continue;
        }
        if !namelistsall.iter().any(|n| n == ln) || !use_ok(ln) {
            continue;
        }
        if value_true(be.get_field(ln)) {
            lnfhi = Some(ln.clone());
            break;
        }
    }
    let be = sec.bibentry_mut(key).expect("entry");
    if let Some(l) = lni {
        be.slots.insert("labelnameinfo", J::String(l));
    }
    if let Some(l) = lnfhi {
        be.slots.insert("labelnamefhinfo", J::String(l));
    }
}

/// `process_labeldate` (Biber.pm:2623).
fn process_labeldate(ctx: &mut Ctx, sec: &mut Section, secnum: &str, key: &str) {
    let h = H {
        cfg: ctx.cfg,
        dm: ctx.dm,
        secnum,
    };
    let be = sec.bibentry_mut(key).expect("entry");
    let et = be.entrytype().to_string();
    if !opt_true(h.opt_undef("labeldateparts", Some(&et), Some(key))) {
        return;
    }
    let spec: Vec<(String, String)> = match h.cfg.blx.get(None, "labeldatespec", Some(&et), None) {
        Some(OptValue::Multi(v)) => v
            .iter()
            .map(|x| {
                (
                    x.str("type").unwrap_or("").to_string(),
                    x.content().unwrap_or("").to_string(),
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    let defined = |be: &Entry, f: Option<&str>| f.is_some_and(|f| be.get_field(f).is_some());
    for (ty, ld) in &spec {
        if ty == "field" {
            let (ldy, rest, pseudodate, datetype) = if h.dm.field_is_datatype("date", ld) {
                let dt = ld.strip_suffix("date").unwrap_or(ld).to_string();
                let p = |s: &str| Some(format!("{dt}{s}"));
                (
                    format!("{dt}year"),
                    [
                        p("endyear"),
                        p("month"),
                        p("day"),
                        p("hour"),
                        p("minute"),
                        p("second"),
                        p("timezone"),
                    ],
                    false,
                    dt,
                )
            } else {
                (ld.clone(), Default::default(), true, String::new())
            };
            if defined(be, Some(&ldy)) || defined(be, rest[0].as_deref()) {
                let s = |x: &Option<String>| x.clone().map_or(J::Null, J::String);
                let mut f = Map::new();
                f.insert("year".into(), J::String(ldy.clone()));
                f.insert("month".into(), s(&rest[1]));
                f.insert("day".into(), s(&rest[2]));
                f.insert("hour".into(), s(&rest[3]));
                f.insert("minute".into(), s(&rest[4]));
                f.insert("second".into(), s(&rest[5]));
                f.insert("timezone".into(), s(&rest[6]));
                f.insert(
                    "pseudodate".into(),
                    if pseudodate {
                        J::String("1".into())
                    } else {
                        J::Null
                    },
                );
                f.insert(
                    "source".into(),
                    J::String(if pseudodate { ldy.clone() } else { datetype }),
                );
                let mut o = Map::new();
                o.insert("field".into(), J::Object(f));
                be.slots.insert("labeldateinfo", J::Object(o));
                break;
            }
        } else if ty == "string" {
            let mut o = Map::new();
            o.insert("string".into(), J::String(ld.clone()));
            be.slots.insert("labeldateinfo", J::Object(o));
            break;
        }
    }

    let Some(J::Object(ldi)) = be.slots.get("labeldateinfo").cloned() else {
        return;
    };
    let field_str =
        |be: &Entry, f: &str| be.get_field(f).and_then(Value::as_str).map(str::to_string);
    if let Some(J::Object(df)) = ldi.get("field") {
        let g = |k: &str| df.get(k).and_then(J::as_str).map(str::to_string);
        let pseudodate = df
            .get("pseudodate")
            .and_then(J::as_str)
            .is_some_and(|s| truthy(Some(s)));
        let year = g("year").unwrap_or_default();
        let yv = be.get_field(&year).cloned();
        be.set_field("labelyear", yv);
        for (lf, k) in [
            ("labelmonth", "month"),
            ("labelday", "day"),
            ("labelhour", "hour"),
            ("labelminute", "minute"),
            ("labelsecond", "second"),
            ("labeltimezone", "timezone"),
        ] {
            if let Some(f) = g(k).filter(|s| truthy(Some(s))) {
                let v = be.get_field(&f).cloned();
                be.set_field(lf, v);
            }
        }
        be.set_field("labeldatesource", g("source").map(Value::Str));
        // `my ($ytype) = $df->{year} =~ /\A(\X*)year\z/xms`
        let ytype = year.strip_suffix("year").unwrap_or("").to_string();
        let endyear = format!("{ytype}endyear");
        if be.field_exists(&endyear) {
            let ey = field_str(be, &endyear).unwrap_or_default();
            if field_str(be, &year).unwrap_or_default() != ey {
                let ly = field_str(be, "labelyear").unwrap_or_default();
                be.set_field(
                    "labelyear",
                    Some(Value::Str(format!("{ly}\\bibdatedash {ey}"))),
                );
            }
        }
        if !pseudodate {
            for (part, lf) in [
                ("month", "labelmonth"),
                ("day", "labelday"),
                ("hour", "labelhour"),
                ("minute", "labelminute"),
                ("second", "labelsecond"),
            ] {
                let endf = format!("{ytype}end{part}");
                let Some(ev) = field_str(be, &endf).filter(|s| truthy(Some(s))) else {
                    continue;
                };
                let start = g(part).and_then(|f| field_str(be, &f)).unwrap_or_default();
                if start != ev {
                    let cur = field_str(be, lf).unwrap_or_default();
                    be.set_field(lf, Some(Value::Str(format!("{cur}\\bibdatedash {ev}"))));
                }
            }
        }
    } else if let Some(ys) = ldi
        .get("string")
        .and_then(J::as_str)
        .filter(|s| truthy(Some(s)))
    {
        be.set_field("labeldatesource", Some(Value::Str(ys.to_string())));
    }
}

/// `process_labeltitle` (Biber.pm:2771).
fn process_labeltitle(ctx: &mut Ctx, sec: &mut Section, key: &str) {
    let be = sec.bibentry_mut(key).expect("entry");
    let et = be.entrytype().to_string();
    let spec: Vec<String> = match ctx.cfg.blx.get(None, "labeltitlespec", Some(&et), None) {
        Some(OptValue::Multi(v)) => v
            .iter()
            .map(|h| h.content().unwrap_or("").to_string())
            .collect(),
        _ => Vec::new(),
    };
    for ltn in spec {
        if let Some(lt) = be.get_field(&ltn).filter(|v| value_true(Some(v))).cloned() {
            be.slots.insert("labeltitleinfo", J::String(ltn));
            be.set_field("labeltitle", Some(lt));
            break;
        }
    }
}

/// `process_presort` (Biber.pm:3066).
fn process_presort(ctx: &mut Ctx, sec: &Section, secnum: &str, key: &str) {
    let be = sec.bibentry(key).expect("entry");
    if let Some(Value::Str(ps)) = be.get_field("presort")
        && truthy(Some(ps))
    {
        ctx.cfg
            .blx
            .set_entry(secnum, key, "presort", OptValue::Str(ps.clone()));
    }
}

// ---------------------------------------------------------------------------
// process_entries_pre: name disambiguation
// ---------------------------------------------------------------------------

/// One element of a name disambiguation schema.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DisPart {
    /// `['base', [parts]]`.
    Base(Vec<String>),
    /// `[namepart, 'init'|'full'|'fullonly']`.
    Part(String, &'static str),
}

impl DisPart {
    fn to_json(&self) -> J {
        match self {
            DisPart::Base(p) => J::Array(vec![
                J::String("base".into()),
                J::Array(p.iter().map(|x| J::String(x.clone())).collect()),
            ]),
            DisPart::Part(np, l) => J::Array(vec![J::String(np.clone()), J::String((*l).into())]),
        }
    }
}

/// The result of `process_namedis` for one name.
#[derive(Clone, Debug)]
pub(crate) struct NameDis {
    pub nlid: NameListId,
    pub nid: NameId,
    pub nameun: Option<String>,
    pub namelistul: Option<String>,
    pub namestring: String,
    pub namestrings: Vec<String>,
    pub schema: Vec<DisPart>,
}

/// `process_namedis` (Biber.pm:1771): name strings and disambiguation
/// schemata for every name of the entry. Per-namelist and per-name
/// `uniquenametemplatename`/`uniquename` (and per-namelist `uniquelist`)
/// overrides leak to the following names and name lists (BB-019).
pub(crate) fn process_namedis(
    h: &H,
    log: &mut Logger,
    sec: &Section,
    list_untname: &str,
    key: &str,
) -> Result<Vec<NameDis>, Fatal> {
    let Some(be) = sec.bibentry(key) else {
        return Ok(Vec::new());
    };
    let et = be.entrytype();
    let leak = h.leak();
    let e_un = opt_string(h.opt("uniquename", Some(et), Some(key)));
    let e_ul = opt_string(h.opt("uniquelist", Some(et), Some(key)));
    let e_unt = opt_string(h.opt("uniquenametemplatename", None, Some(key)))
        .unwrap_or_else(|| list_untname.to_string());
    let (mut un, mut ul, mut untname) = (e_un.clone(), e_ul.clone(), e_unt.clone());
    let mut out = Vec::new();
    for pn in &h.dm.helpers.namelistsall {
        let Some(Value::Names(nlid)) = be.get_field(pn) else {
            continue;
        };
        let nl = sec.names.list(*nlid);
        if !leak {
            (un, ul, untname) = (e_un.clone(), e_ul.clone(), e_unt.clone());
        }
        if let Some(v) = slot_str(&nl.slots, "uniquenametemplatename") {
            untname = v.to_string();
        }
        if let Some(v) = slot_str(&nl.slots, "uniquelist") {
            ul = Some(v.to_string());
        }
        if let Some(v) = slot_str(&nl.slots, "uniquename") {
            un = Some(v.to_string());
        }
        for nid in &nl.names {
            let n = sec.names.name(*nid);
            let mut n_unt = untname.clone();
            if let Some(v) = slot_str(&n.slots, "uniquenametemplatename") {
                n_unt = v.to_string();
            }
            let tmpl = match h.uniquename_template(&n_unt) {
                Some(t) => t,
                None => {
                    biber_error!(
                        log,
                        "No uniquenametemplate called '{n_unt}' found, cannot continue."
                    )?;
                    &[]
                }
            };
            let mut n_un = un.clone();
            if let Some(v) = slot_str(&n.slots, "uniquename") {
                n_un = Some(v.to_string());
            }
            if leak {
                untname = n_unt.clone();
                un = n_un.clone();
            }
            let use_ok = |npn: &str| {
                let m = format!("use{npn}");
                let mut ok = opt_true(h.opt(&m, Some(et), Some(key)));
                if let Some(v) = slot_str(&nl.slots, &m) {
                    ok = truthy(Some(v));
                }
                if let Some(v) = slot_str(&n.slots, &m) {
                    ok = truthy(Some(v));
                }
                ok
            };

            let mut namestring = String::new();
            let mut namestrings: Vec<String> = Vec::new();
            let mut schema = Vec::new();
            let mut base = String::new();
            let mut baseparts: Option<Vec<String>> = None;
            for np in tmpl.iter().filter(|np| truthy(np.base.as_deref())) {
                let npn = np.namepart.as_deref().unwrap_or("");
                let p = namepart(n, npn);
                if !truthy(Some(&p)) {
                    continue;
                }
                if truthy(np.use_.as_deref()) && !use_ok(npn) {
                    continue;
                }
                base.push_str(&p);
                baseparts.get_or_insert_with(Vec::new).push(npn.to_string());
            }
            namestring.push_str(&base);
            namestrings.push(base);
            if let Some(bp) = baseparts {
                schema.push(DisPart::Base(bp));
            }
            for np in tmpl.iter().filter(|np| !truthy(np.base.as_deref())) {
                if np.disambiguation.as_deref() == Some("none") {
                    continue;
                }
                let npn = np.namepart.as_deref().unwrap_or("");
                let level = np
                    .disambiguation
                    .clone()
                    .or_else(|| {
                        uniquename_context(n_un.as_deref().unwrap_or("false")).map(str::to_string)
                    })
                    .map(|l| fc(&l))
                    .unwrap_or_default();
                let lastns = namestrings.last().cloned().unwrap_or_default();
                let p = namepart(n, npn);
                if !truthy(Some(&p)) {
                    continue;
                }
                let pi = namepart_initials(n, npn);
                if truthy(np.use_.as_deref()) && !use_ok(npn) {
                    continue;
                }
                namestring.push_str(&p);
                if level == "full" {
                    namestrings.push(format!("{lastns}{p}"));
                    schema.push(DisPart::Part(npn.to_string(), "fullonly"));
                }
                if level == "initorfull" {
                    namestrings.push(format!("{lastns}{pi}"));
                    schema.push(DisPart::Part(npn.to_string(), "init"));
                    namestrings.push(format!("{lastns}{p}"));
                    schema.push(DisPart::Part(npn.to_string(), "full"));
                } else if level == "init" {
                    namestrings.push(format!("{lastns}{pi}"));
                    schema.push(DisPart::Part(npn.to_string(), "init"));
                }
            }
            let ty = nl.type_.as_deref();
            out.push(NameDis {
                nlid: *nlid,
                nid: *nid,
                nameun: n_un,
                namelistul: ul.clone(),
                namestring: strip_nonamestring(h.cfg, &namestring, ty),
                namestrings: namestrings
                    .iter()
                    .map(|s| strip_nonamestring(h.cfg, s, ty))
                    .collect(),
                schema,
            });
        }
    }
    Ok(out)
}

/// `DataList::set_namedis` (DataList.pm:305).
fn set_namedis(dl: &mut DataList, d: &NameDis) {
    let (nlk, nk) = (d.nlid.key(), d.nid.key());
    let o = ls::viv(ls::root(dl), &["namelistdata", &nlk, &nk])
        .as_object_mut()
        .expect("object");
    o.insert("namestring".into(), J::String(d.namestring.clone()));
    o.insert(
        "namestrings".into(),
        J::Array(d.namestrings.iter().map(|s| J::String(s.clone())).collect()),
    );
    for (i, se) in d.schema.iter().enumerate() {
        if let DisPart::Base(parts) = se {
            o.insert(
                "basenamestring".into(),
                d.namestrings
                    .get(i)
                    .map_or(J::Null, |s| J::String(s.clone())),
            );
            o.insert(
                "basenamestringparts".into(),
                J::Array(parts.iter().map(|s| J::String(s.clone())).collect()),
            );
            break;
        }
    }
    o.insert(
        "namedisschema".into(),
        J::Array(d.schema.iter().map(DisPart::to_json).collect()),
    );
}

/// `Biber::process_entries_pre` (Biber.pm:2000), for the data list
/// `st.datalists[list]`.
pub fn process_entries_pre(ctx: &mut Ctx, st: &mut SectionState, list: usize) -> Result<(), Fatal> {
    let secnum = st.section.number.clone();
    let h = H {
        cfg: ctx.cfg,
        dm: ctx.dm,
        secnum: &secnum,
    };
    let untname = st.datalists[list].uniquenametemplatename.clone();
    for key in &st.section.citekeys {
        let nd = process_namedis(&h, ctx.log, &st.section, &untname, key)?;
        for d in &nd {
            if d.nameun.as_deref() == Some("false") && d.namelistul.as_deref() == Some("false") {
                continue;
            }
            set_namedis(&mut st.datalists[list], d);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// process_visible_names
// ---------------------------------------------------------------------------

/// `Biber::process_visible_names` (Biber.pm:2892), for the data list
/// `st.datalists[list]`. Visibility is stored per name list, so name lists
/// shared between entries get the last entry's values (BB-026).
pub fn process_visible_names(
    ctx: &mut Ctx,
    st: &mut SectionState,
    list: usize,
) -> Result<(), Fatal> {
    let secnum = st.section.number.clone();
    let h = H {
        cfg: ctx.cfg,
        dm: ctx.dm,
        secnum: &secnum,
    };
    let pluralothers = opt_true(h.cfg.blx.global("pluralothers"));
    let dl = &mut st.datalists[list];
    let sec = &st.section;
    for key in &sec.citekeys {
        let Some(be) = sec.bibentry(key) else {
            continue;
        };
        let et = be.entrytype();
        // Option values keep their string form (biber stores the scalar).
        let o = |name: &str| -> (f64, J) {
            match opt_string(h.opt(name, Some(et), Some(key))) {
                Some(s) => (numify(&s), J::String(s)),
                None => (0.0, J::Null),
            }
        };
        let (maxcn, _) = o("maxcitenames");
        let mincn = o("mincitenames");
        let (maxbn, _) = o("maxbibnames");
        let minbn = o("minbibnames");
        let (maxsn, _) = o("maxsortnames");
        let minsn = o("minsortnames");
        let (maxan, _) = o("maxalphanames");
        let minan = o("minalphanames");
        for n in &h.dm.helpers.namelistsall {
            let Some(Value::Names(nlid)) = be.get_field(n) else {
                continue;
            };
            let nlk = nlid.key();
            let count = sec.names.list(*nlid).names.len();
            let cnt = (count as f64, J::String(count.to_string()));
            let cap = |m: &(f64, J)| if cnt.0 < m.0 { cnt.clone() } else { m.clone() };
            let (l_mincn, l_minbn, l_minsn, l_minan) =
                (cap(&mincn), cap(&minbn), cap(&minsn), cap(&minan));
            let vis_alpha = if cnt.0 > maxan { l_minan } else { cnt.clone() };
            let ul_or =
                |dl: &mut DataList, d: (f64, J)| match ls::get(dl, &["namelistdata", &nlk, "ul"]) {
                    Some(v) => (ls::num(&v), v),
                    None => d,
                };
            let mut vis_cite = if cnt.0 > maxcn {
                ul_or(dl, l_mincn)
            } else {
                cnt.clone()
            };
            if pluralothers && cnt.0 - vis_cite.0 == 1.0 {
                vis_cite = cnt.clone();
            }
            let vis_bib = if cnt.0 > maxbn {
                ul_or(dl, l_minbn)
            } else {
                cnt.clone()
            };
            let vis_sort = if cnt.0 > maxsn {
                ul_or(dl, l_minsn)
            } else {
                cnt.clone()
            };
            ls::set(dl, &["namelistdata", &nlk, "viscite"], vis_cite.1);
            ls::set(dl, &["namelistdata", &nlk, "visbib"], vis_bib.1);
            ls::set(dl, &["namelistdata", &nlk, "vissort"], vis_sort.1);
            ls::set(dl, &["namelistdata", &nlk, "visalpha"], vis_alpha.1);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// process_entries_post
// ---------------------------------------------------------------------------

/// `Biber::process_entries_post` (Biber.pm:2049), for the data list
/// `st.datalists[list]`.
pub fn process_entries_post(
    ctx: &mut Ctx,
    st: &mut SectionState,
    list: usize,
) -> Result<(), Fatal> {
    let secnum = st.section.number.clone();
    for key in st.section.citekeys.clone() {
        if st.section.bibentry(&key).is_none() {
            continue;
        }
        process_fullhash(ctx, st, &secnum, list, &key)?;
        labelalpha::process_labelalpha(ctx, st, list, &key)?;
        labelalpha::process_extraalpha(ctx, st, list, &key)?;
        let h = H {
            cfg: ctx.cfg,
            dm: ctx.dm,
            secnum: &secnum,
        };
        process_extradate(&h, st, list, &key);
        process_extraname(&h, st, list, &key);
        process_extratitle(&h, st, list, &key);
        process_extratitleyear(&h, st, list, &key);
        process_workuniqueness(&h, ctx.log, st, list, &key)?;
        process_namehash(&h, ctx.log, st, list, &key)?;
        process_pername_hashes(&h, ctx.log, st, list, &key)?;
        process_uniqueprimaryauthor(&h, ctx.log, st, list, &key)?;
    }
    Ok(())
}

/// The entry's `labelnameinfo`/`labelnamefhinfo`/`labeltitleinfo` slot.
pub(crate) fn info_slot<'a>(be: &'a Entry, slot: &str) -> Option<&'a str> {
    be.slots.get(slot).and_then(J::as_str)
}

/// The name list of field `f` of `be`, if it is a (true) name list.
fn names_field(be: &Entry, f: Option<&str>) -> Option<NameListId> {
    f.and_then(|f| be.get_field(f)).and_then(Value::as_names)
}

/// `process_fullhash` (Biber.pm:2797).
fn process_fullhash(
    ctx: &mut Ctx,
    st: &mut SectionState,
    secnum: &str,
    list: usize,
    key: &str,
) -> Result<(), Fatal> {
    let h = H {
        cfg: ctx.cfg,
        dm: ctx.dm,
        secnum,
    };
    let sec = &st.section;
    let dl = &mut st.datalists[list];
    let be = sec.bibentry(key).expect("entry");
    if let Some(nlid) = names_field(be, info_slot(be, "labelnamefhinfo")) {
        let fh = getfullhash(&h, ctx.log, sec, dl, key, nlid)?;
        ls::set_entryfield(dl, key, "fullhash", J::String(fh));
        ls::set_entryfield(
            dl,
            key,
            "fullhashraw",
            J::String(getfullhashraw(&h, sec, nlid)),
        );
    }
    for n in &h.dm.helpers.namelistsall {
        let Some(nlid) = names_field(be, Some(n)) else {
            continue;
        };
        let fh = getfullhash(&h, ctx.log, sec, dl, key, nlid)?;
        ls::set_entryfield(dl, key, &format!("{n}fullhash"), J::String(fh));
        ls::set_entryfield(
            dl,
            key,
            &format!("{n}fullhashraw"),
            J::String(getfullhashraw(&h, sec, nlid)),
        );
    }
    Ok(())
}

/// `get_${field}_info` for `extradatecontext` items `label…`.
fn label_info(be: &Entry, f: &str) -> Option<String> {
    let slot = match f {
        "labelname" => "labelnameinfo",
        "labelnamefh" => "labelnamefhinfo",
        "labeltitle" => "labeltitleinfo",
        _ => return None,
    };
    info_slot(be, slot).map(str::to_string)
}

/// `process_extradate` (Biber.pm:2272).
fn process_extradate(h: &H, st: &mut SectionState, list: usize, key: &str) {
    let sec = &st.section;
    let dl = &mut st.datalists[list];
    let be = sec.bibentry(key).expect("entry");
    let et = be.entrytype();
    if !opt_true(h.opt_undef("labeldateparts", Some(et), Some(key))) {
        return;
    }
    if opt_true(h.opt("skiplab", Some(et), Some(key))) {
        return;
    }
    let mut contexthash = String::new();
    if let Some(OptValue::Multi(edc)) = h.cfg.blx.global("extradatecontext") {
        for item in edc {
            let fieldc = item.content().unwrap_or("");
            let fieldc = if fieldc.len() > 5 && fieldc.starts_with("label") {
                label_info(be, fieldc)
            } else {
                Some(fieldc.to_string())
            };
            let Some(fieldc) = fieldc.filter(|f| truthy(Some(f))) else {
                continue;
            };
            let fv = be.get_field(&fieldc);
            if !value_true(fv) {
                continue;
            }
            let fv = fv.expect("value");
            if h.dm.field_is_datatype("name", &fieldc) {
                if let Value::Names(nlid) = fv {
                    contexthash = getnamehash_u(h, sec, dl, key, *nlid);
                }
            } else if h.dm.field_is_fieldtype("list", &fieldc) {
                let joined = fv.as_list().map(|l| l.concat()).unwrap_or_default();
                contexthash = hash_string(&joined);
            } else {
                contexthash = hash_string(fv.as_str().unwrap_or(""));
            }
            break;
        }
    }
    let mut datestring = String::new();
    let mut edscope: Option<String> = None;
    if let Some(OptValue::ExtradateSpec(spec)) = h.cfg.blx.global("extradatespec") {
        for scope in spec {
            for field in scope {
                if let Some(v) = be.get_field(field) {
                    datestring.push_str(v.as_str().unwrap_or(""));
                    edscope = Some(field.clone());
                    break;
                }
            }
        }
    }
    let tracking = format!("{contexthash},{datestring}");
    let be = st.section.bibentry_mut(key).expect("entry");
    be.set_field("extradatescope", edscope.map(Value::Str));
    ls::set_entryfield(dl, key, "nametitledateparts", J::String(tracking.clone()));
    // incr_seen_nametitledateparts
    let path = ["seen_nametitledateparts", tracking.as_str()];
    if !ls::exists(dl, &path) || truthy(Some(&contexthash)) {
        ls::incr(dl, &path);
    }
}

/// `process_extraname` (Biber.pm:2358).
fn process_extraname(h: &H, st: &mut SectionState, list: usize, key: &str) {
    let sec = &st.section;
    let dl = &mut st.datalists[list];
    let be = sec.bibentry(key).expect("entry");
    if opt_true(h.opt("skiplab", Some(be.entrytype()), Some(key))) {
        return;
    }
    let lni = info_slot(be, "labelnameinfo").filter(|s| truthy(Some(s)));
    // `$be->get_field($lni)` must be a name list here.
    if let Some(nlid) = names_field(be, lni) {
        let nh = getnamehash_u(h, sec, dl, key, nlid);
        ls::set_entryfield(dl, key, "labelnamehash", J::String(nh.clone()));
        ls::incr(dl, &["seen_labelname", &nh]);
    }
}

/// `process_extratitle` (Biber.pm:2393). biber looks the `labeltitle`
/// option up without the citekey; per-entry values are ignored (BB-021).
fn process_extratitle(h: &H, st: &mut SectionState, list: usize, key: &str) {
    let sec = &st.section;
    let dl = &mut st.datalists[list];
    let be = sec.bibentry(key).expect("entry");
    let et = be.entrytype();
    if !opt_true(h.opt_undef("labeltitle", Some(et), Some(key))) {
        return;
    }
    if opt_true(h.opt("skiplab", Some(et), Some(key))) {
        return;
    }
    let lni = info_slot(be, "labelnameinfo").filter(|s| truthy(Some(s)));
    let namehash = names_field(be, lni)
        .map(|nlid| getnamehash_u(h, sec, dl, key, nlid))
        .unwrap_or_default();
    let title = perl_str(info_slot(be, "labeltitleinfo").and_then(|f| be.get_field(f)));
    let nt = format!("{namehash},{title}");
    ls::set_entryfield(dl, key, "nametitle", J::String(nt.clone()));
    let path = ["seen_nametitle", nt.as_str()];
    if !ls::truthy(&ls::get(dl, &path)) || truthy(Some(&title)) {
        ls::incr(dl, &path);
    }
}

/// `process_extratitleyear` (Biber.pm:2443).
fn process_extratitleyear(h: &H, st: &mut SectionState, list: usize, key: &str) {
    let sec = &st.section;
    let dl = &mut st.datalists[list];
    let be = sec.bibentry(key).expect("entry");
    let et = be.entrytype();
    if !opt_true(h.opt_undef("labeltitleyear", Some(et), Some(key))) {
        return;
    }
    if opt_true(h.opt("skiplab", Some(et), Some(key))) {
        return;
    }
    let title = perl_str(info_slot(be, "labeltitleinfo").and_then(|f| be.get_field(f)));
    let s = |f: &str| be.get_str(f).filter(|s| truthy(Some(s)));
    let year = s("labelyear")
        .or_else(|| s("year"))
        .unwrap_or("")
        .to_string();
    let ty = format!("{title},{year}");
    ls::set_entryfield(dl, key, "titleyear", J::String(ty.clone()));
    let path = ["seen_titleyear", ty.as_str()];
    if !ls::truthy(&ls::get(dl, &path)) || truthy(Some(&title)) {
        ls::incr(dl, &path);
    }
}

/// `process_workuniqueness` (Biber.pm:2180).
fn process_workuniqueness(
    h: &H,
    log: &mut Logger,
    st: &mut SectionState,
    list: usize,
    key: &str,
) -> Result<(), Fatal> {
    let sec = &st.section;
    let dl = &mut st.datalists[list];
    let be = sec.bibentry(key).expect("entry");
    let et = be.entrytype();
    let lni = info_slot(be, "labelnameinfo").filter(|s| truthy(Some(s)));
    let lti = info_slot(be, "labeltitleinfo").filter(|s| truthy(Some(s)));
    // `$ignore->{$kind}->@*` (in `first`'s argument list) autovivifies
    // `{$kind} = []` in the global uniqignore record when the entry has one.
    let ignore = st
        .config_state
        .get_mut("uniqignore")
        .and_then(|u| u.get_mut(key))
        .filter(|i| i.is_object());
    let mut ignore: Option<&mut J> = ignore;
    let mut ignored = |kind: &str, f: Option<&str>| -> bool {
        let Some(i) = ignore.as_deref_mut() else {
            return false;
        };
        let a = i
            .as_object_mut()
            .expect("object")
            .entry(kind.to_string())
            .or_insert_with(|| J::Array(Vec::new()));
        let Some(f) = f else { return false };
        a.as_array().is_some_and(|a| {
            a.iter()
                .filter_map(J::as_str)
                .any(|x| fc(x) == fc(f) && truthy(Some(x)))
        })
    };

    let skip = if h.cfg.quirks.on(Quirk::WorkuniqSkipbibToken) {
        // BB-027: only the literal token `skipbib` in the `options` field.
        match be.get_field("options") {
            Some(Value::List(o)) => o.iter().any(|x| x == "skipbib"),
            Some(Value::Str(s)) if truthy(Some(s)) => s == "skipbib",
            _ => false,
        }
    } else {
        opt_true(h.opt("skipbib", Some(et), Some(key)))
    };
    if skip {
        return Ok(());
    }

    if let Some(ln) = lni
        && opt_true(h.opt_undef("singletitle", Some(et), Some(key)))
    {
        let id = match names_field(be, Some(ln)) {
            Some(nlid) => getfullhash(h, log, sec, dl, key, nlid)?,
            None => String::new(),
        };
        if !(ignored("singletitle", lni) && lti.is_some() && ignored("singletitle", lti)) {
            ls::incr(dl, &["seenname", &id]);
        }
        ls::set_entryfield(dl, key, "seenname", J::String(id));
    }
    let title = lti.and_then(|f| be.get_field(f)).map(|v| perl_str(Some(v)));
    if lti.is_some() && opt_true(h.opt_undef("uniquetitle", Some(et), Some(key))) {
        let id = title.clone().unwrap_or_default();
        if !ignored("uniquetitle", lti) {
            ls::incr(dl, &["seentitle", &id]);
        }
        ls::set_entryfield(
            dl,
            key,
            "seentitle",
            title.clone().map_or(J::Null, J::String),
        );
    }
    if lti.is_some()
        && lni.is_none()
        && opt_true(h.opt_undef("uniquebaretitle", Some(et), Some(key)))
    {
        let id = title.clone().unwrap_or_default();
        if !ignored("uniquebaretitle", lti) {
            ls::incr(dl, &["seenbaretitle", &id]);
        }
        ls::set_entryfield(
            dl,
            key,
            "seenbaretitle",
            title.clone().map_or(J::Null, J::String),
        );
    }
    if let Some(ln) = lni
        && lti.is_some()
        && opt_true(h.opt_undef("uniquework", Some(et), Some(key)))
    {
        let fh = match names_field(be, Some(ln)) {
            Some(nlid) => getfullhash(h, log, sec, dl, key, nlid)?,
            None => String::new(),
        };
        let id = format!("{fh}{}", title.clone().unwrap_or_default());
        if !(ignored("uniquework", lni) && ignored("uniquework", lti)) {
            ls::incr(dl, &["seenwork", &id]);
        }
        ls::set_entryfield(dl, key, "seenwork", J::String(id));
    }
    Ok(())
}

/// `process_namehash` (Biber.pm:2829).
fn process_namehash(
    h: &H,
    log: &mut Logger,
    st: &mut SectionState,
    list: usize,
    key: &str,
) -> Result<(), Fatal> {
    let sec = &st.section;
    let dl = &mut st.datalists[list];
    let be = sec.bibentry(key).expect("entry");
    if let Some(nlid) = names_field(be, info_slot(be, "labelnameinfo")) {
        let nh = getnamehash(h, log, sec, dl, key, nlid, false)?;
        ls::set_entryfield(dl, key, "namehash", J::String(nh));
        let bnh = getnamehash(h, log, sec, dl, key, nlid, true)?;
        ls::set_entryfield(dl, key, "bibnamehash", J::String(bnh));
    }
    for n in &h.dm.helpers.namelistsall {
        let Some(nlid) = names_field(be, Some(n)) else {
            continue;
        };
        let nh = getnamehash(h, log, sec, dl, key, nlid, false)?;
        ls::set_entryfield(dl, key, &format!("{n}namehash"), J::String(nh));
        let bnh = getnamehash(h, log, sec, dl, key, nlid, true)?;
        ls::set_entryfield(dl, key, &format!("{n}bibnamehash"), J::String(bnh));
    }
    Ok(())
}

/// `process_pername_hashes` (Biber.pm:2861): sets each name's `hash` (on
/// the shared `Name` object) and the list's per-name hash.
fn process_pername_hashes(
    h: &H,
    log: &mut Logger,
    st: &mut SectionState,
    list: usize,
    key: &str,
) -> Result<(), Fatal> {
    let be = st.section.bibentry(key).expect("entry");
    let lists: Vec<NameListId> =
        h.dm.helpers
            .namelistsall
            .iter()
            .filter_map(|n| names_field(be, Some(n)))
            .collect();
    for nlid in lists {
        for nid in st.section.names.list(nlid).names.clone() {
            let hash = genpnhash(h, log, &st.section, &st.datalists[list], key, nlid, nid)?;
            st.section
                .names
                .name_mut(nid)
                .slots
                .insert("hash", J::String(hash.clone()));
            ls::set(
                &mut st.datalists[list],
                &["namelistdata", &nlid.key(), &nid.key(), "hash"],
                J::String(hash),
            );
        }
    }
    Ok(())
}

/// `process_uniqueprimaryauthor` (Biber.pm:2142).
fn process_uniqueprimaryauthor(
    h: &H,
    log: &mut Logger,
    st: &mut SectionState,
    list: usize,
    key: &str,
) -> Result<(), Fatal> {
    let sec = &st.section;
    let be = sec.bibentry(key).expect("entry");
    let Some(lni) = info_slot(be, "labelnameinfo").filter(|s| truthy(Some(s))) else {
        return Ok(());
    };
    if !opt_true(h.opt_undef("uniqueprimaryauthor", Some(be.entrytype()), Some(key))) {
        return Ok(());
    }
    let Some(nlid) = names_field(be, Some(lni)) else {
        return Ok(());
    };
    let Some(first) = sec.names.list(nlid).names.first().copied() else {
        return Ok(());
    };
    let untname = st.datalists[list].uniquenametemplatename.clone();
    let nd = process_namedis(h, log, sec, &untname, key)?;
    let mut pabase: Option<String> = None;
    if let Some(d) = nd.iter().rev().find(|d| d.nlid == nlid && d.nid == first) {
        for (i, se) in d.schema.iter().enumerate() {
            if matches!(se, DisPart::Base(_)) {
                pabase = d.namestrings.get(i).cloned();
            }
        }
    }
    let hash = slot_str(&sec.names.name(first).slots, "hash")
        .unwrap_or("")
        .to_string();
    let dl = &mut st.datalists[list];
    ls::set_entryfield(
        dl,
        key,
        "seenprimaryauthor",
        pabase.clone().map_or(J::Null, J::String),
    );
    ls::set(
        dl,
        &["seenpa", pabase.as_deref().unwrap_or(""), &hash],
        J::String("1".into()),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// process_entries_final
// ---------------------------------------------------------------------------

/// `Biber::process_entries_final` (Biber.pm:2107), for the data list
/// `st.datalists[list]`: singletitle, uniquetitle, uniquebaretitle,
/// uniquework and uniqueprimaryauthor (`generate_*`, Biber.pm:4082-4240).
pub fn process_entries_final(
    ctx: &mut Ctx,
    st: &mut SectionState,
    list: usize,
) -> Result<(), Fatal> {
    let secnum = st.section.number.clone();
    let h = H {
        cfg: ctx.cfg,
        dm: ctx.dm,
        secnum: &secnum,
    };
    let sec = &st.section;
    let dl = &mut st.datalists[list];
    for key in &sec.citekeys {
        let Some(be) = sec.bibentry(key) else {
            continue;
        };
        let et = be.entrytype();
        // generate_singletitle / uniquetitle / uniquebaretitle: `defined`
        for (opt, seen, out) in [
            ("singletitle", "seenname", "singletitle"),
            ("uniquetitle", "seentitle", "uniquetitle"),
            ("uniquebaretitle", "seenbaretitle", "uniquebaretitle"),
        ] {
            if !opt_true(h.opt_undef(opt, Some(et), Some(key))) {
                continue;
            }
            let Some(id) = ls::entryfield(dl, key, seen) else {
                continue;
            };
            let id = ls::str_of(&Some(id)).unwrap_or_default();
            if ls::get(dl, &[seen, &id]).map_or(0.0, |v| ls::num(&v)) < 2.0 {
                ls::set_entryfield(dl, key, out, J::String("1".into()));
            }
        }
        // generate_uniquework / uniquepa: truthiness
        for (opt, seen, out) in [
            ("uniquework", "seenwork", "uniquework"),
            (
                "uniqueprimaryauthor",
                "seenprimaryauthor",
                "uniqueprimaryauthor",
            ),
        ] {
            if !opt_true(h.opt_undef(opt, Some(et), Some(key))) {
                continue;
            }
            let id = ls::entryfield(dl, key, seen);
            if !ls::truthy(&id) {
                continue;
            }
            let id = ls::str_of(&id).unwrap_or_default();
            let n = if seen == "seenprimaryauthor" {
                // get_seenpa: `scalar keys %{…{seenpa}{$base}}`
                ls::get(dl, &["seenpa", &id])
                    .and_then(|v| v.as_object().map(Map::len))
                    .unwrap_or(0) as f64
            } else {
                ls::get(dl, &[seen, &id]).map_or(0.0, |v| ls::num(&v))
            };
            if n < 2.0 {
                ls::set_entryfield(dl, key, out, J::String("1".into()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values from perl 5.42 (oracle O3) running biber's
    /// `normalise_string_hash` and `md5_hex(encode_utf8(NFC(…)))`.
    #[test]
    fn hashes_match_perl() {
        for (s, n, h) in [
            ("Smith", "Smith", "e95f770ac4fb91ac2e4873e4b2dfc0e6"),
            ("0", "", "d41d8cd98f00b204e9800998ecf8427e"),
            ("\\\"{a} b.~c", "34:abc", "92ba9a8239f0751943cee98d7845a0b3"),
            ("\\\\é  x", "\\é:x", "c2033b4b5f262edb0609baf4e4d96492"),
            ("a\\\nb", "a10:b", "0253981ad5a64d7109311753912d5869"),
            (
                "\\ss {}x\u{2003}y",
                "ss:xy",
                "3f0282ebf01e62d30a005f7a2ea49778",
            ),
            (
                "Mu\u{308}ller+",
                "Mu\u{308}ller+",
                "a2830c9f96a7c99944cd583ad11a8b8f",
            ),
        ] {
            assert_eq!(normalise_string_hash(s), n, "{s:?}");
            assert_eq!(hash_string(s), h, "{s:?}");
        }
    }
}

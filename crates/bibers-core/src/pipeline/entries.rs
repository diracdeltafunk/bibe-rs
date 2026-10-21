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
use crate::model::{
    DataList, Entry, LabelDateFields, LabelDateInfo, Name, NameId, NameListId, Section,
    SectionState, Sym, Value,
};
use crate::par;
use crate::perlish::{fc, lc, numify, truthy};
use crate::quirks::Quirk;
use md5::{Digest, Md5};
use perl_compat::regex::PerlRegex;
use perl_compat::unicode::{Prop, is_perl_space};
use serde_json::{Map, Value as J};
use std::collections::HashMap;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Per-list state access (`Biber::DataList` accessors)
// ---------------------------------------------------------------------------

/// Access to `$dlist->{state}` with Perl's autovivification: an rvalue
/// fetch `$self->{state}{a}{b}{c}` creates the intermediate hashes `a` and
/// `a.b` (DataList.pm has no `no autovivification`), and those empty hashes
/// show in biber's dumps.
pub(crate) mod ls {
    use crate::model::{DataList, ListState};
    use serde_json::{Map, Value as J};

    /// `$self->{state}` (created if needed).
    pub fn root(dl: &mut DataList) -> &mut ListState {
        dl.state.get_or_insert_with(ListState::default)
    }

    /// Walk `path` from `v`, creating hashes on the way; returns the (hash)
    /// value at the end of the path.
    pub fn viv_in<'a>(mut v: &'a mut J, path: &[&str]) -> &'a mut J {
        for k in path {
            if !v.is_object() {
                *v = J::Object(Map::new());
            }
            let m = v.as_object_mut().expect("object");
            if !m.contains_key(*k) {
                m.insert(k.to_string(), J::Object(Map::new()));
            }
            v = m.get_mut(*k).expect("just inserted");
        }
        if !v.is_object() {
            *v = J::Object(Map::new());
        }
        v
    }

    /// `$self->{state}{p0}{p1}…{pn}` as a hash, creating hashes on the way
    /// (at least two keys).
    pub fn viv<'a>(dl: &'a mut DataList, path: &[&str]) -> &'a mut J {
        let v = root(dl).level1(path[0]).hash(path[1]);
        viv_in(v, &path[2..])
    }

    /// `viv(…)` as a map.
    pub fn viv_map<'a>(dl: &'a mut DataList, path: &[&str]) -> &'a mut Map<String, J> {
        viv(dl, path).as_object_mut().expect("object")
    }

    /// rvalue `$self->{state}{p0}…{pn}`: vivifies `p0…p(n-1)`; `undef` is
    /// `None`.
    pub fn get(dl: &mut DataList, path: &[&str]) -> Option<J> {
        get_ref(dl, path).cloned()
    }

    /// [`get`] without the copy.
    pub fn get_ref<'a>(dl: &'a mut DataList, path: &[&str]) -> Option<&'a J> {
        let (last, parents) = path.split_last().expect("non-empty path");
        let v = match parents {
            [k0] => root(dl).level1(k0).into_get(last),
            _ => {
                let v: &J = viv(dl, parents);
                v.get(*last)
            }
        };
        v.filter(|x| !x.is_null())
    }

    /// The numeric value of [`get`] (`undef` is 0).
    pub fn num_at(dl: &mut DataList, path: &[&str]) -> f64 {
        get_ref(dl, path).map_or(0.0, num)
    }

    /// `exists $self->{state}{p0}…{pn}` (vivifies `p0…p(n-1)`).
    pub fn exists(dl: &mut DataList, path: &[&str]) -> bool {
        let (last, parents) = path.split_last().expect("non-empty path");
        match parents {
            [k0] => root(dl).level1(k0).contains_key(last),
            _ => viv_map(dl, parents).contains_key(*last),
        }
    }

    /// `$self->{state}{p0}…{pn} = $v`.
    pub fn set(dl: &mut DataList, path: &[&str], v: J) {
        let (last, parents) = path.split_last().expect("non-empty path");
        match parents {
            [k0] => root(dl).level1(k0).insert(last, v),
            _ => {
                let m = viv_map(dl, parents);
                match m.get_mut(*last) {
                    Some(slot) => *slot = v,
                    None => {
                        m.insert(last.to_string(), v);
                    }
                }
            }
        }
    }

    /// `++$self->{state}{p0}…{pn}`; returns the new value.
    pub fn incr(dl: &mut DataList, path: &[&str]) -> i64 {
        let n = get_ref(dl, path).map_or(0.0, num) as i64 + 1;
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

    /// `set_entryfield($citekey, $f, $v)` for each `(f, v)`, in order.
    pub fn set_entryfields(dl: &mut DataList, key: &str, fv: Vec<(String, J)>) {
        if fv.is_empty() {
            return;
        }
        let m = viv_map(dl, &["fields", key]);
        for (f, v) in fv {
            m.insert(f, v);
        }
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
        truthy_ref(v.as_ref())
    }

    /// [`truthy`] of a borrowed value.
    pub fn truthy_ref(v: Option<&J>) -> bool {
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
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let d = Md5::digest(s.as_bytes());
    let mut out = String::with_capacity(32);
    for b in d {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0xF)]));
    }
    out
}

/// `md5_hex(encode_utf8(NFC(normalise_string_hash($s))))`, the form of every
/// name/context hash.
pub(crate) fn hash_string(s: &str) -> String {
    let n = normalise_string_hash(s);
    if n.is_ascii() {
        md5_hex(&n) // NFC of ASCII is itself
    } else {
        md5_hex(&perl_compat::unicode::nfc(&n))
    }
}

/// `Biber::Utils::normalise_string_hash` (Utils.pm:736).
pub(crate) fn normalise_string_hash(s: &str) -> String {
    if !truthy(Some(s)) {
        return String::new();
    }
    // Without backslashes only the last substitution can apply.
    if !s.contains('\\') {
        return s
            .chars()
            .filter(|&c| !matches!(c, '{' | '}' | '~' | '.') && !is_perl_space(c))
            .collect();
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
fn namepart_str<'n>(n: &'n Name, np: &str) -> &'n str {
    n.nameparts
        .get(np)
        .and_then(|p| p.string.as_deref())
        .filter(|s| truthy(Some(s)))
        .unwrap_or("")
}

/// `Name::get_namepart`, owned.
fn namepart(n: &Name, np: &str) -> String {
    namepart_str(n, np).to_string()
}

/// `join('', $n->get_namepart_initial($np)->@*)`, appended to `out`.
fn push_namepart_initials(out: &mut String, n: &Name, np: &str) {
    if let Some(inits) = n.nameparts.get(np).and_then(|p| p.initial.as_ref()) {
        for i in inits {
            out.push_str(i);
        }
    }
}

/// `join('', $n->get_namepart_initial($np)->@*)`.
fn namepart_initials(n: &Name, np: &str) -> String {
    let mut s = String::new();
    push_namepart_initials(&mut s, n, np);
    s
}

/// What a namehash template hashes of one namepart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HashScope {
    Nothing,
    Initials,
    Full,
}

/// For each of `nameparts`, the scope `Name::get_hash_namepart`
/// (Name.pm:137) finds in the template: the first template part naming it
/// (case-folded) with hashscope `init` or `full`.
fn hash_scopes(nameparts: &[String], nht: &[NamehashPart]) -> Vec<HashScope> {
    let parts: Vec<(String, String)> = nht
        .iter()
        .map(|p| {
            (
                fc(p.namepart.as_deref().unwrap_or("")),
                fc(p.hashscope.as_deref().unwrap_or("")),
            )
        })
        .collect();
    nameparts
        .iter()
        .map(|np| {
            let fnp = fc(np);
            parts
                .iter()
                .filter(|(n, _)| *n == fnp)
                .find_map(|(_, scope)| match scope.as_str() {
                    "init" => Some(HashScope::Initials),
                    "full" => Some(HashScope::Full),
                    _ => None,
                })
                .unwrap_or(HashScope::Nothing)
        })
        .collect()
}

/// `Name::get_hash_namepart` (Name.pm:137) with the scope from
/// [`hash_scopes`], appended to `out`.
fn push_hash_namepart(out: &mut String, n: &Name, np: &str, scope: HashScope) {
    match scope {
        HashScope::Nothing => {}
        HashScope::Initials => push_namepart_initials(out, n, np),
        HashScope::Full => out.push_str(namepart_str(n, np)),
    }
}

/// Read-only processing context shared by the helpers.
pub(crate) struct H<'a> {
    pub cfg: &'a Config,
    pub dm: &'a DataModel,
    pub secnum: &'a str,
    /// What the name hashes need, computed on first use.
    hashing: OnceLock<Hashing>,
}

/// Lookups the name hashes need, computed once per phase.
struct Hashing {
    /// `$dm->get_constant_value('nameparts')`.
    nameparts: Vec<String>,
    /// Namehash template name → [`hash_scopes`].
    hash_scopes: HashMap<String, Vec<HashScope>>,
    /// Whether any `nonamestring` patterns are set.
    nonamestring: bool,
}

impl<'a> H<'a> {
    pub fn new(cfg: &'a Config, dm: &'a DataModel, secnum: &'a str) -> Self {
        H {
            cfg,
            dm,
            secnum,
            hashing: OnceLock::new(),
        }
    }

    fn hashing(&self) -> &Hashing {
        self.hashing.get_or_init(|| {
            let nameparts = self.dm.get_constant_value("nameparts");
            let hash_scopes = match self.cfg.blx.global("namehashtemplate") {
                Some(OptValue::NamehashTemplates(m)) => m
                    .iter()
                    .map(|(name, t)| (name.clone(), hash_scopes(&nameparts, t)))
                    .collect(),
                _ => HashMap::new(),
            };
            Hashing {
                nameparts,
                hash_scopes,
                nonamestring: !self.cfg.biber.patterns("nonamestring").is_empty(),
            }
        })
    }

    /// `$dm->get_constant_value('nameparts')`.
    fn nameparts(&self) -> &[String] {
        &self.hashing().nameparts
    }

    /// The hash scopes of the namehash template `name`, or `biber_error`.
    fn need_nht(&self, log: &mut Logger, name: &str) -> Result<&[HashScope], Fatal> {
        match self.hashing().hash_scopes.get(name) {
            Some(t) => Ok(t),
            None => {
                biber_error!(log, "No namehash template called '{name}'")?;
                Ok(&[])
            }
        }
    }

    /// `strip_nonamestring($s, $field)` appended to `out`.
    fn push_stripped(&self, out: &mut String, s: &str, field: Option<&str>) {
        if !self.hashing().nonamestring {
            if truthy(Some(s)) {
                out.push_str(s);
            }
        } else {
            out.push_str(&strip_nonamestring(self.cfg, s, field));
        }
    }
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
fn namehash_tname<'x>(
    h: &'x H,
    sec: &'x Section,
    dl: &'x DataList,
    key: &str,
    nlid: NameListId,
) -> &'x str {
    slot_str(&sec.names.list(nlid).slots, "namehashtemplatename").unwrap_or_else(|| {
        h.opt("namehashtemplatename", None, Some(key))
            .and_then(OptValue::as_str)
            .unwrap_or(&dl.namehashtemplatename)
    })
}

/// Append the namehash parts of `n` to `hashkey`, under the namehash
/// template in force, `nhtname` (a per-name template replaces it for the
/// following names with the `per-name-option-leak` quirk, BB-019). With
/// `strip`, each part goes through `strip_nonamestring` for that field.
fn push_name_hashparts<'x>(
    h: &H,
    log: &mut Logger,
    hashkey: &mut String,
    n: &'x Name,
    nhtname: &mut &'x str,
    strip: Option<Option<&str>>,
) -> Result<(), Fatal> {
    if let Some(hid) = slot_str(&n.slots, "hashid").filter(|s| truthy(Some(s))) {
        hashkey.push_str(hid);
        return Ok(());
    }
    let n_nht = slot_str(&n.slots, "namehashtemplatename").unwrap_or(nhtname);
    if h.leak() {
        *nhtname = n_nht;
    }
    let scopes = h.need_nht(log, n_nht)?;
    for (i, nt) in h.nameparts().iter().enumerate() {
        let scope = scopes.get(i).copied().unwrap_or(HashScope::Nothing);
        match strip {
            None => push_hash_namepart(hashkey, n, nt, scope),
            Some(field) => {
                let mut part = String::new();
                push_hash_namepart(&mut part, n, nt, scope);
                h.push_stripped(hashkey, &part, field);
            }
        }
    }
    Ok(())
}

/// `_getnamehash` (Internals.pm:39): namehash (`bib` false) or bibnamehash.
/// A per-name `namehashtemplatename` leaks to the following names (BB-019).
///
/// Reading the visibility autovivifies `{namelistdata}{$nlid}` in biber: the
/// caller does that ([`viv_namelistdata`]), so that this can run in parallel.
pub(crate) fn getnamehash(
    h: &H,
    log: &mut Logger,
    sec: &Section,
    dl: &DataList,
    key: &str,
    nlid: NameListId,
    bib: bool,
) -> Result<String, Fatal> {
    let et = sec.bibentry(key).map_or("", Entry::entrytype);
    let names = sec.names.list(nlid);
    let count = names.names.len();
    let nlk = nlid.key();
    let visible = nld_num(dl, &nlk, if bib { "visbib" } else { "viscite" });
    let mut nhtname = namehash_tname(h, sec, dl, key, nlid);
    let mut hashkey = String::new();
    for nid in names.names.iter().take(first_n(visible, count)) {
        let n = sec.names.name(*nid);
        push_name_hashparts(h, log, &mut hashkey, n, &mut nhtname, None)?;
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
    let mut nhtname = namehash_tname(h, sec, dl, key, nlid);
    let mut hashkey = String::new();
    for nid in &names.names {
        let n = sec.names.name(*nid);
        let strip = Some(names.type_.as_deref());
        push_name_hashparts(h, log, &mut hashkey, n, &mut nhtname, strip)?;
    }
    if truthy(slot_str(&names.slots, "morenames")) {
        hashkey.push('+');
    }
    Ok(hash_string(&hashkey))
}

/// `_getfullhashraw` (Internals.pm:154).
pub(crate) fn getfullhashraw(h: &H, sec: &Section, nlid: NameListId) -> String {
    let names = sec.names.list(nlid);
    let mut hashkey = String::new();
    for n in sec.names.names_of(nlid) {
        for nt in h.nameparts() {
            let np = namepart_str(n, nt);
            if truthy(Some(np)) {
                h.push_stripped(&mut hashkey, np, names.type_.as_deref());
            }
        }
    }
    if truthy(slot_str(&names.slots, "morenames")) {
        hashkey.push('+');
    }
    hash_string(&hashkey)
}

/// `{namelistdata}{$nlk}` without autovivification.
fn nld_peek<'d>(dl: &'d DataList, nlk: &str) -> Option<&'d J> {
    dl.state.as_ref()?.peek2("namelistdata", nlk)
}

/// The numeric value of `{namelistdata}{$nlk}{$f}` (`undef` is 0), without
/// autovivification.
fn nld_num(dl: &DataList, nlk: &str, f: &str) -> f64 {
    nld_peek(dl, nlk)
        .and_then(|m| m.get(f))
        .filter(|v| !v.is_null())
        .map_or(0.0, ls::num)
}

/// The autovivification of `{namelistdata}{$nlk}` (and of
/// `{namelistdata}{$nlk}{$nid}` for each of `names`) that biber's reads in
/// [`getnamehash`] and [`getnamehash_u`] cause.
fn viv_namelistdata(dl: &mut DataList, nlid: NameListId, names: &[NameId]) {
    let nlk = nlid.key();
    ls::viv(dl, &["namelistdata", &nlk]);
    for nid in names {
        ls::viv(dl, &["namelistdata", &nlk, &nid.key()]);
    }
}

/// `_getnamehash_u` (Internals.pm:180): the namehash under the uniquename
/// template, for extra* tracking. Also returns the names whose `un` it read:
/// biber's reads autovivify their data ([`viv_namelistdata`]).
pub(crate) fn getnamehash_u(
    h: &H,
    sec: &Section,
    dl: &DataList,
    key: &str,
    nlid: NameListId,
) -> (String, Vec<NameId>) {
    let et = sec.bibentry(key).map_or("", Entry::entrytype);
    let names = sec.names.list(nlid);
    let count = names.names.len();
    let nlkey = nlid.key();
    let nld = nld_peek(dl, &nlkey);
    let visible = nld_num(dl, &nlkey, "viscite");
    let mut read = Vec::new();
    let mut untname = slot_str(&names.slots, "uniquenametemplatename")
        .or_else(|| {
            h.opt("uniquenametemplatename", None, Some(key))
                .and_then(OptValue::as_str)
        })
        .unwrap_or(&dl.uniquenametemplatename)
        .to_string();
    let mut hashkey = String::new();
    for nid in names.names.iter().take(first_n(visible, count)) {
        let n = sec.names.name(*nid);
        let n_unt = slot_str(&n.slots, "uniquenametemplatename").unwrap_or(&untname);
        let template = h.uniquename_template(n_unt).unwrap_or(&[]);
        if h.leak() && n_unt != untname {
            untname = n_unt.to_string();
        }
        for nps in template {
            if nps.disambiguation.as_deref() == Some("none") {
                continue;
            }
            let npn = nps.namepart.as_deref().unwrap_or("");
            let np = namepart_str(n, npn);
            if !truthy(Some(np)) {
                continue;
            }
            if truthy(nps.base.as_deref()) {
                hashkey.push_str(np);
            } else {
                read.push(*nid);
                let un = nld.and_then(|m| m.get(nid.key())).and_then(|m| m.get("un"));
                if let Some(J::Array(un)) = un
                    && un.first().and_then(J::as_str) != Some("base")
                {
                    match un.get(1).and_then(J::as_str) {
                        Some("full" | "fullonly") => hashkey.push_str(np),
                        Some("init") => push_namepart_initials(&mut hashkey, n, npn),
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
    (hash_string(&hashkey), read)
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
    let nhtname = slot_str(&n.slots, "namehashtemplatename")
        .unwrap_or_else(|| namehash_tname(h, sec, dl, key, nlid));
    let scopes = h.need_nht(log, nhtname)?;
    let mut hashkey = String::new();
    for (i, nt) in h.nameparts().iter().enumerate() {
        let scope = scopes.get(i).copied().unwrap_or(HashScope::Nothing);
        push_hash_namepart(&mut hashkey, n, nt, scope);
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
    let h = H::new(ctx.cfg, ctx.dm, secnum);
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
                let [_, month, day, hour, minute, second, timezone] = rest;
                let sym = |f: Option<String>| f.map(Sym::from);
                be.labeldateinfo = Some(LabelDateInfo::Field(LabelDateFields {
                    year: Some(Sym::new(&ldy)),
                    month: sym(month),
                    day: sym(day),
                    hour: sym(hour),
                    minute: sym(minute),
                    second: sym(second),
                    timezone: sym(timezone),
                    pseudodate: pseudodate.then(|| Sym::new("1")),
                    source: Some(Sym::new(if pseudodate { &ldy } else { &datetype })),
                }));
                break;
            }
        } else if ty == "string" {
            be.labeldateinfo = Some(LabelDateInfo::String(Some(ld.clone())));
            break;
        }
    }

    let Some(ldi) = be.labeldateinfo.clone() else {
        return;
    };
    let field_str =
        |be: &Entry, f: &str| be.get_field(f).and_then(Value::as_str).map(str::to_string);
    if let LabelDateInfo::Field(df) = &ldi {
        let g = |k: &str| df.get(k).map(str::to_string);
        let pseudodate = df.get("pseudodate").is_some_and(|s| truthy(Some(s)));
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
    } else if let LabelDateInfo::String(Some(ys)) = &ldi
        && truthy(Some(ys))
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
    let o = ls::viv(dl, &["namelistdata", &nlk, &nk])
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
    let h = H::new(ctx.cfg, ctx.dm, &secnum);
    let untname = st.datalists[list].uniquenametemplatename.clone();
    // The name strings in parallel, stored in citekey order.
    let sec = &st.section;
    let (nds, res) = par::map_logged(ctx.log, &sec.citekeys, |log, key| {
        process_namedis(&h, log, sec, &untname, key)
    });
    for nd in nds {
        for d in &nd {
            if d.nameun.as_deref() == Some("false") && d.namelistul.as_deref() == Some("false") {
                continue;
            }
            set_namedis(&mut st.datalists[list], d);
        }
    }
    res
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
    let h = H::new(ctx.cfg, ctx.dm, &secnum);
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
            let m = ls::viv(dl, &["namelistdata", &nlk])
                .as_object_mut()
                .expect("object");
            m.insert("viscite".into(), vis_cite.1);
            m.insert("visbib".into(), vis_bib.1);
            m.insert("vissort".into(), vis_sort.1);
            m.insert("visalpha".into(), vis_alpha.1);
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
    let h = H::new(ctx.cfg, ctx.dm, &secnum);
    // The name hashes first, for all entries in parallel: they only read
    // state from earlier phases.
    let hashes: Vec<Option<PostHashes>> = {
        let (sec, dl, parent) = (&st.section, &st.datalists[list], &*ctx.log);
        par::map(&sec.citekeys, |key| {
            sec.bibentry(key)
                .map(|_| PostHashes::new(&h, parent, sec, dl, key))
        })
    };
    let (log, run) = (&mut *ctx.log, &mut ctx.run.labelalpha);
    let keys = st.section.citekeys.clone();
    for (key, ph) in keys.iter().zip(hashes) {
        let Some(mut ph) = ph else { continue };
        let key = key.as_str();
        process_fullhash(log, st, list, key, ph.fullhash)?;
        labelalpha::process_labelalpha_in(h.cfg, h.dm, run, st, list, key)?;
        labelalpha::process_extraalpha_in(h.cfg, st, list, key)?;
        process_extradate(&h, st, list, key, &ph.namehash_u);
        process_extraname(&h, st, list, key, &ph.namehash_u);
        process_extratitle(&h, st, list, key, &ph.namehash_u);
        process_extratitleyear(&h, st, list, key);
        let (stf, uwf) = (ph.singletitle_fh.take(), ph.uniquework_fh.take());
        process_workuniqueness(&h, log, st, list, key, stf, uwf)?;
        process_namehash(log, st, list, key, ph.namehash)?;
        process_pername_hashes(log, st, list, ph.pername)?;
        process_uniqueprimaryauthor(&h, log, st, list, key, ph.upa_namedis)?;
    }
    Ok(())
}

/// A value computed ahead of time with the messages its computation logged,
/// replayed where the serial code would have computed it.
struct Logged<T> {
    log: Logger,
    value: Result<T, Fatal>,
}

impl<T> Logged<T> {
    fn run(parent: &Logger, f: impl FnOnce(&mut Logger) -> Result<T, Fatal>) -> Self {
        let mut log = parent.fork();
        let value = f(&mut log);
        Logged { log, value }
    }

    /// Emit the messages and return the value.
    fn take(self, log: &mut Logger) -> Result<T, Fatal> {
        log.join(self.log);
        self.value
    }
}

/// [`getnamehash_u`] of each name list of an entry.
type NamehashU = Vec<(NameListId, String, Vec<NameId>)>;

/// The hashes `process_entries_post` computes for one entry. They depend
/// only on the state of earlier phases (and the entry's own name data), so
/// they are computed for all entries in parallel before the serial loop,
/// which applies them in citekey order. Each part that can log carries its
/// messages ([`Logged`]).
struct PostHashes {
    /// `process_fullhash`'s entry fields.
    fullhash: Logged<Vec<(String, J)>>,
    /// The labelname's full hash for singletitle / uniquework
    /// (`process_workuniqueness`), when the option is on.
    singletitle_fh: Option<Logged<String>>,
    uniquework_fh: Option<Logged<String>>,
    /// `process_namehash`'s entry fields, and the name lists whose data it
    /// read.
    namehash: Logged<(Vec<(String, J)>, Vec<NameListId>)>,
    /// `process_pername_hashes`: each name's hash.
    pername: Logged<Vec<(NameListId, NameId, String)>>,
    /// [`getnamehash_u`] of each name list (extradate/extraname/extratitle).
    namehash_u: NamehashU,
    /// `process_uniqueprimaryauthor`'s `process_namedis`, when it runs.
    upa_namedis: Option<Logged<Vec<NameDis>>>,
}

impl PostHashes {
    fn new(h: &H, parent: &Logger, sec: &Section, dl: &DataList, key: &str) -> Self {
        let be = sec.bibentry(key).expect("entry");
        let et = be.entrytype();
        let namelists = || {
            h.dm.helpers
                .namelistsall
                .iter()
                .filter_map(|n| names_field(be, Some(n)).map(|nlid| (n, nlid)))
        };
        // process_fullhash (Biber.pm:2797)
        let fullhash = Logged::run(parent, |log| {
            let mut out = Vec::new();
            if let Some(nlid) = names_field(be, info_slot(be, "labelnamefhinfo")) {
                let fh = getfullhash(h, log, sec, dl, key, nlid)?;
                out.push(("fullhash".to_string(), J::String(fh)));
                let fhr = getfullhashraw(h, sec, nlid);
                out.push(("fullhashraw".to_string(), J::String(fhr)));
            }
            for (n, nlid) in namelists() {
                let fh = getfullhash(h, log, sec, dl, key, nlid)?;
                out.push((format!("{n}fullhash"), J::String(fh)));
                let fhr = getfullhashraw(h, sec, nlid);
                out.push((format!("{n}fullhashraw"), J::String(fhr)));
            }
            Ok(out)
        });
        // process_workuniqueness (Biber.pm:2180)
        let lni = info_slot(be, "labelnameinfo").filter(|s| truthy(Some(s)));
        let lni_fh = |opt: &str| {
            (lni.is_some() && opt_true(h.opt_undef(opt, Some(et), Some(key)))).then(|| {
                Logged::run(parent, |log| match names_field(be, lni) {
                    Some(nlid) => getfullhash(h, log, sec, dl, key, nlid),
                    None => Ok(String::new()),
                })
            })
        };
        let singletitle_fh = lni_fh("singletitle");
        let uniquework_fh = lni_fh("uniquework");
        // process_namehash (Biber.pm:2829)
        let namehash = Logged::run(parent, |log| {
            let mut out = Vec::new();
            let mut read = Vec::new();
            if let Some(nlid) = names_field(be, info_slot(be, "labelnameinfo")) {
                read.push(nlid);
                let nh = getnamehash(h, log, sec, dl, key, nlid, false)?;
                out.push(("namehash".to_string(), J::String(nh)));
                let bnh = getnamehash(h, log, sec, dl, key, nlid, true)?;
                out.push(("bibnamehash".to_string(), J::String(bnh)));
            }
            for (n, nlid) in namelists() {
                read.push(nlid);
                let nh = getnamehash(h, log, sec, dl, key, nlid, false)?;
                out.push((format!("{n}namehash"), J::String(nh)));
                let bnh = getnamehash(h, log, sec, dl, key, nlid, true)?;
                out.push((format!("{n}bibnamehash"), J::String(bnh)));
            }
            Ok((out, read))
        });
        // process_pername_hashes (Biber.pm:2861)
        let pername = Logged::run(parent, |log| {
            let mut out = Vec::new();
            for (_, nlid) in namelists() {
                for nid in &sec.names.list(nlid).names {
                    let hash = genpnhash(h, log, sec, dl, key, nlid, *nid)?;
                    out.push((nlid, *nid, hash));
                }
            }
            Ok(out)
        });
        let namehash_u = namelists()
            .map(|(_, nlid)| {
                let (s, read) = getnamehash_u(h, sec, dl, key, nlid);
                (nlid, s, read)
            })
            .collect();
        // process_uniqueprimaryauthor (Biber.pm:2142)
        let upa = lni.is_some()
            && opt_true(h.opt_undef("uniqueprimaryauthor", Some(et), Some(key)))
            && names_field(be, lni).is_some_and(|nlid| !sec.names.list(nlid).names.is_empty());
        let upa_namedis = upa.then(|| {
            Logged::run(parent, |log| {
                process_namedis(h, log, sec, &dl.uniquenametemplatename, key)
            })
        });
        PostHashes {
            fullhash,
            singletitle_fh,
            uniquework_fh,
            namehash,
            pername,
            namehash_u,
            upa_namedis,
        }
    }
}

/// [`getnamehash_u`] of `nlid` from `pre` (computed with the entry's other
/// hashes) or now, with its autovivification.
fn namehash_u(
    h: &H,
    sec: &Section,
    dl: &mut DataList,
    key: &str,
    nlid: NameListId,
    pre: &NamehashU,
) -> String {
    let (s, read) = match pre.iter().find(|(id, _, _)| *id == nlid) {
        Some((_, s, read)) => (s.clone(), read.clone()),
        None => getnamehash_u(h, sec, dl, key, nlid),
    };
    viv_namelistdata(dl, nlid, &read);
    s
}

/// The entry's `labelnameinfo`/`labelnamefhinfo`/`labeltitleinfo` slot.
pub(crate) fn info_slot<'a>(be: &'a Entry, slot: &str) -> Option<&'a str> {
    be.slots.get(slot).and_then(J::as_str)
}

/// The name list of field `f` of `be`, if it is a (true) name list.
fn names_field(be: &Entry, f: Option<&str>) -> Option<NameListId> {
    f.and_then(|f| be.get_field(f)).and_then(Value::as_names)
}

/// `process_fullhash` (Biber.pm:2797), with the hashes computed by
/// [`PostHashes::new`].
fn process_fullhash(
    log: &mut Logger,
    st: &mut SectionState,
    list: usize,
    key: &str,
    fullhash: Logged<Vec<(String, J)>>,
) -> Result<(), Fatal> {
    let out = fullhash.take(log)?;
    ls::set_entryfields(&mut st.datalists[list], key, out);
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
fn process_extradate(h: &H, st: &mut SectionState, list: usize, key: &str, pre: &NamehashU) {
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
                    contexthash = namehash_u(h, sec, dl, key, *nlid, pre);
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
fn process_extraname(h: &H, st: &mut SectionState, list: usize, key: &str, pre: &NamehashU) {
    let sec = &st.section;
    let dl = &mut st.datalists[list];
    let be = sec.bibentry(key).expect("entry");
    if opt_true(h.opt("skiplab", Some(be.entrytype()), Some(key))) {
        return;
    }
    let lni = info_slot(be, "labelnameinfo").filter(|s| truthy(Some(s)));
    // `$be->get_field($lni)` must be a name list here.
    if let Some(nlid) = names_field(be, lni) {
        let nh = namehash_u(h, sec, dl, key, nlid, pre);
        ls::set_entryfield(dl, key, "labelnamehash", J::String(nh.clone()));
        ls::incr(dl, &["seen_labelname", &nh]);
    }
}

/// `process_extratitle` (Biber.pm:2393). biber looks the `labeltitle`
/// option up without the citekey; per-entry values are ignored (BB-021).
fn process_extratitle(h: &H, st: &mut SectionState, list: usize, key: &str, pre: &NamehashU) {
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
        .map(|nlid| namehash_u(h, sec, dl, key, nlid, pre))
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
    singletitle_fh: Option<Logged<String>>,
    uniquework_fh: Option<Logged<String>>,
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

    if lni.is_some() && opt_true(h.opt_undef("singletitle", Some(et), Some(key))) {
        // (`getfullhash` of the labelname, or "")
        let id = singletitle_fh.expect("computed").take(log)?;
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
    if lni.is_some() && lti.is_some() && opt_true(h.opt_undef("uniquework", Some(et), Some(key))) {
        let fh = uniquework_fh.expect("computed").take(log)?;
        let id = format!("{fh}{}", title.clone().unwrap_or_default());
        if !(ignored("uniquework", lni) && ignored("uniquework", lti)) {
            ls::incr(dl, &["seenwork", &id]);
        }
        ls::set_entryfield(dl, key, "seenwork", J::String(id));
    }
    Ok(())
}

/// `process_namehash` (Biber.pm:2829), with the hashes computed by
/// [`PostHashes::new`].
fn process_namehash(
    log: &mut Logger,
    st: &mut SectionState,
    list: usize,
    key: &str,
    namehash: Logged<(Vec<(String, J)>, Vec<NameListId>)>,
) -> Result<(), Fatal> {
    let dl = &mut st.datalists[list];
    let (out, read) = namehash.take(log)?;
    for nlid in read {
        viv_namelistdata(dl, nlid, &[]);
    }
    ls::set_entryfields(dl, key, out);
    Ok(())
}

/// `process_pername_hashes` (Biber.pm:2861): sets each name's `hash` (on
/// the shared `Name` object) and the list's per-name hash.
fn process_pername_hashes(
    log: &mut Logger,
    st: &mut SectionState,
    list: usize,
    pername: Logged<Vec<(NameListId, NameId, String)>>,
) -> Result<(), Fatal> {
    for (nlid, nid, hash) in pername.take(log)? {
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
    Ok(())
}

/// `process_uniqueprimaryauthor` (Biber.pm:2142).
fn process_uniqueprimaryauthor(
    h: &H,
    log: &mut Logger,
    st: &mut SectionState,
    list: usize,
    key: &str,
    namedis: Option<Logged<Vec<NameDis>>>,
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
    let nd = match namedis {
        Some(nd) => nd.take(log)?,
        None => {
            let untname = st.datalists[list].uniquenametemplatename.clone();
            process_namedis(h, log, sec, &untname, key)?
        }
    };
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
    let h = H::new(ctx.cfg, ctx.dm, &secnum);
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
            if ls::num_at(dl, &[seen, &id]) < 2.0 {
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
                ls::num_at(dl, &[seen, &id])
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

//! Loading and dumping [`SectionState`] in the canonical JSON format of
//! biber's phase dumps (`tools/oracle/probes/pipeline/BiberPhaseDump.pm`).
//!
//! This lets each pipeline phase be tested in isolation: load biber's state
//! from just before a phase, run the Rust phase, dump, and compare with
//! biber's state just after it ([`json_diff`]).
//!
//! Format (produced by the Perl dumper): all scalars are strings, `undef` is
//! `null`, blessed `Biber::*` objects carry `"__class"`, and the random ids of
//! `Names`/`Name` objects are replaced by `__nl<N>__`/`__n<N>__`, numbered in
//! order of first appearance in a deterministic traversal (sorted hash keys).
//!
//! Inside [`PerlData`] held by the model, name ids are written in the
//! *internal* form `#nl<index>`/`#n<index>` (arena indices); the loader and
//! dumper translate between the two forms.

use super::*;
use serde_json::{Map, Value as J};
use std::collections::HashMap;

const NAMES_CLASS: &str = "Biber::Entry::Names";
const NAME_CLASS: &str = "Biber::Entry::Name";

impl NameListId {
    /// Internal string form used inside [`PerlData`].
    pub fn key(self) -> String {
        format!("#nl{}", self.0)
    }
}

impl NameId {
    /// Internal string form used inside [`PerlData`].
    pub fn key(self) -> String {
        format!("#n{}", self.0)
    }
}

fn is_canonical_id(s: &str) -> bool {
    s.starts_with("__n") && s.ends_with("__")
}

fn is_internal_id(s: &str) -> bool {
    s.starts_with("#n")
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

struct Loader {
    arena: NameArena,
    /// Canonical id (`__nl3__`) → internal key (`#nl0`).
    ids: HashMap<String, String>,
    lists: HashMap<String, NameListId>,
    names: HashMap<String, NameId>,
}

fn s(v: &J) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn obj(v: &J) -> Result<&Map<String, J>, String> {
    v.as_object()
        .ok_or_else(|| format!("expected an object, got {v}"))
}

impl Loader {
    fn name(&mut self, v: &J) -> Result<NameId, String> {
        let o = obj(v)?;
        let cid = o.get("id").and_then(J::as_str).unwrap_or("").to_string();
        if let Some(id) = self.names.get(&cid) {
            return Ok(*id);
        }
        let mut n = Name {
            index: o.get("index").and_then(s).map(Into::into),
            rawstring: o.get("rawstring").and_then(s),
            ..Name::default()
        };
        if let Some(np) = o.get("nameparts").and_then(J::as_object) {
            for (part, pv) in np {
                let po = obj(pv)?;
                let initial = match po.get("initial") {
                    Some(J::Array(a)) => Some(a.iter().filter_map(s).map(Into::into).collect()),
                    _ => None,
                };
                n.nameparts.insert(
                    part.clone(),
                    NamePart {
                        string: po.get("string").and_then(s).map(Into::into),
                        initial,
                    },
                );
            }
        }
        if let Some(st) = o.get("strip").and_then(J::as_object) {
            for (k, v) in st {
                let flag = match s(v).as_deref() {
                    None => None,
                    Some("1") => Some(true),
                    Some("0") => Some(false),
                    Some(other) => return Err(format!("strip flag {other:?}")),
                };
                n.strip.insert(k.clone(), flag);
            }
        }
        for (k, v) in o {
            if !matches!(
                k.as_str(),
                "__class" | "id" | "index" | "nameparts" | "strip" | "rawstring"
            ) {
                n.slots.insert(k.clone(), v.clone());
            }
        }
        let id = self.arena.new_name(n);
        if !cid.is_empty() {
            self.names.insert(cid.clone(), id);
            self.ids.insert(cid, id.key());
        }
        Ok(id)
    }

    fn names(&mut self, v: &J) -> Result<NameListId, String> {
        let o = obj(v)?;
        let cid = o.get("id").and_then(J::as_str).unwrap_or("").to_string();
        if let Some(id) = self.lists.get(&cid) {
            return Ok(*id);
        }
        let id = self.arena.new_list();
        if !cid.is_empty() {
            self.lists.insert(cid.clone(), id);
            self.ids.insert(cid, id.key());
        }
        let mut names = Vec::new();
        if let Some(J::Array(a)) = o.get("namelist") {
            for nv in a {
                names.push(self.name(nv)?);
            }
        }
        let list = self.arena.list_mut(id);
        list.names = names;
        list.type_ = o.get("type").and_then(s);
        for (k, v) in o {
            if !matches!(k.as_str(), "__class" | "id" | "namelist" | "type") {
                list.slots.insert(k.clone(), v.clone());
            }
        }
        Ok(id)
    }

    fn value(&mut self, v: &J) -> Result<Option<Value>, String> {
        Ok(Some(match v {
            J::Null => return Ok(None),
            J::String(x) => Value::Str(x.clone()),
            J::Array(a) if a.iter().all(J::is_string) => {
                Value::List(a.iter().filter_map(s).collect())
            }
            J::Array(a) if a.iter().all(J::is_array) => Value::Ranges(
                a.iter()
                    .map(|r| r.as_array().unwrap().iter().map(s).collect())
                    .collect(),
            ),
            J::Object(o) if o.get("__class").and_then(J::as_str) == Some(NAMES_CLASS) => {
                Value::Names(self.names(v)?)
            }
            other => Value::Other(other.clone()),
        }))
    }

    fn fields(&mut self, v: Option<&J>) -> Result<FieldMap, String> {
        let mut m = FieldMap::new();
        if let Some(J::Object(o)) = v {
            for (k, fv) in o {
                m.insert(k.clone(), self.value(fv)?);
            }
        }
        Ok(m)
    }

    fn entry(&mut self, v: &J) -> Result<Entry, String> {
        let o = obj(v)?;
        let mut e = Entry {
            datafields: self.fields(o.get("datafields"))?,
            derivedfields: self.fields(o.get("derivedfields"))?,
            ..Entry::default()
        };
        for (k, v) in o {
            if !matches!(k.as_str(), "__class" | "datafields" | "derivedfields") {
                if k == "labeldateinfo"
                    && let Some(ldi) = LabelDateInfo::from_json(v)
                {
                    e.labeldateinfo = Some(ldi);
                    continue;
                }
                e.slots.insert(k.clone(), v.clone());
            }
        }
        Ok(e)
    }

    /// Rewrite canonical name ids to internal keys throughout untyped data.
    fn internalise(&self, v: &J) -> J {
        map_ids(v, &|x| self.ids.get(x).cloned())
    }
}

/// Apply `f` to every string (value or object key) that looks like a name id.
fn map_ids(v: &J, f: &dyn Fn(&str) -> Option<String>) -> J {
    let conv = |x: &str| {
        if is_canonical_id(x) || is_internal_id(x) {
            f(x).unwrap_or_else(|| x.to_string())
        } else {
            x.to_string()
        }
    };
    match v {
        J::String(x) => J::String(conv(x)),
        J::Array(a) => J::Array(a.iter().map(|x| map_ids(x, f)).collect()),
        J::Object(o) => J::Object(o.iter().map(|(k, x)| (conv(k), map_ids(x, f))).collect()),
        other => other.clone(),
    }
}

fn strings(v: Option<&J>) -> Vec<String> {
    match v {
        Some(J::Array(a)) => a.iter().filter_map(s).collect(),
        _ => Vec::new(),
    }
}

fn key_set(v: Option<&J>) -> BTreeSet<String> {
    match v {
        Some(J::Object(o)) => o.keys().cloned().collect(),
        _ => BTreeSet::new(),
    }
}

fn truthy(v: Option<&J>) -> bool {
    v.and_then(J::as_str)
        .is_some_and(perl_compat::truth::perl_truthy)
}

/// Load a [`SectionState`] from the `state` object of a phase dump.
pub fn load_state(state: &J) -> Result<SectionState, String> {
    let st = obj(state)?;
    let sec = obj(st.get("section").ok_or("dump has no section")?)?;
    let mut l = Loader {
        arena: NameArena::default(),
        ids: HashMap::new(),
        lists: HashMap::new(),
        names: HashMap::new(),
    };
    // Load entries in sorted key order (the dumper's traversal order).
    let mut entries = EntryMap::new();
    if let Some(J::Object(be)) = sec.get("bibentries") {
        for (k, ev) in be {
            if k != "__class" {
                entries.insert(k.clone(), l.entry(ev)?);
            }
        }
    }
    let mut section = Section {
        number: sec.get("number").and_then(s).unwrap_or_default(),
        entries,
        citekeys: strings(sec.get("citekeys")),
        citekeys_h: key_set(sec.get("citekeys_h")),
        orig_order_citekeys: strings(sec.get("orig_order_citekeys")),
        undef_citekeys: strings(sec.get("undef_citekeys")),
        citekey_alias: match sec.get("citekey_alias") {
            Some(J::Object(o)) => o
                .iter()
                .map(|(k, v)| (k.clone(), s(v).unwrap_or_default()))
                .collect(),
            _ => BTreeMap::new(),
        },
        allkeys: truthy(sec.get("allkeys")),
        allkeys_nocite: truthy(sec.get("allkeysnocite")),
        cite_citekeys: key_set(sec.get("cite_citekeys")),
        nocite_citekeys: key_set(sec.get("nocite_citekeys")),
        ..Section::default()
    };
    for (k, v) in sec {
        if !SECTION_TYPED.contains(&k.as_str()) {
            section.slots.insert(k.clone(), l.internalise(v));
        }
    }
    // Entry and name slots may mention name ids too.
    for e in section.entries.values_mut_unordered() {
        for v in e.slots.values_mut() {
            *v = l.internalise(v);
        }
    }
    let mut datalists = Vec::new();
    if let Some(J::Array(a)) = st.get("datalists") {
        for dv in a {
            let d = obj(dv)?;
            let g = |k: &str| d.get(k).and_then(s).unwrap_or_default();
            let mut dl = DataList {
                name: g("name"),
                type_: g("type"),
                section: g("section"),
                sortingtemplatename: g("sortingtemplatename"),
                sortingnamekeytemplatename: g("sortingnamekeytemplatename"),
                uniquenametemplatename: g("uniquenametemplatename"),
                labelalphanametemplatename: g("labelalphanametemplatename"),
                namehashtemplatename: g("namehashtemplatename"),
                labelprefix: g("labelprefix"),
                keys: d.get("keys").map(|k| strings(Some(k))),
                state: d
                    .get("state")
                    .map(|v| ListState::from_json(l.internalise(v))),
                sortdata: None,
                sortinitdata: None,
                slots: Slots::new(),
            };
            for (k, v) in d {
                match k.as_str() {
                    "sortdata" => dl.set_sortdata_json(&l.internalise(v))?,
                    "sortinitdata" => dl.set_sortinitdata_json(&l.internalise(v))?,
                    k if DATALIST_TYPED.contains(&k) => {}
                    _ => {
                        dl.slots.insert(k.to_string(), l.internalise(v));
                    }
                }
            }
            datalists.push(dl);
        }
    }
    let mut entry_options = BTreeMap::new();
    if let Some(J::Object(eo)) = st.get("entry_options") {
        for (k, v) in eo {
            if let J::Object(opts) = v {
                entry_options.insert(
                    k.clone(),
                    opts.iter().map(|(o, x)| (o.clone(), x.clone())).collect(),
                );
            }
        }
    }
    let preamble = match st.get("preamble") {
        Some(J::Array(a)) => Some(a.iter().filter_map(s).collect()),
        _ => None,
    };
    let config_state = l.internalise(st.get("config_state").unwrap_or(&J::Null));
    let annotations = st.get("annotations").cloned();
    let shared_lists = st.get("shared_lists").and_then(J::as_array).map(|groups| {
        groups
            .iter()
            .map(|g| {
                g.as_array()
                    .map(|m| {
                        m.iter()
                            .filter_map(|p| {
                                let p = p.as_array()?;
                                Some((s(p.first()?)?, s(p.get(1)?)?))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect()
    });
    section.names = l.arena;
    Ok(SectionState {
        section,
        datalists,
        config_state,
        entry_options,
        preamble,
        annotations,
        shared_lists,
    })
}

const SECTION_TYPED: &[&str] = &[
    "__class",
    "number",
    "bibentries",
    "citekeys",
    "citekeys_h",
    "orig_order_citekeys",
    "undef_citekeys",
    "citekey_alias",
    "allkeys",
    "allkeysnocite",
    "cite_citekeys",
    "nocite_citekeys",
];

const DATALIST_TYPED: &[&str] = &[
    "__class",
    "name",
    "type",
    "section",
    "sortingtemplatename",
    "sortingnamekeytemplatename",
    "uniquenametemplatename",
    "labelalphanametemplatename",
    "namehashtemplatename",
    "labelprefix",
    "keys",
    "state",
];

// ---------------------------------------------------------------------------
// Dumping
// ---------------------------------------------------------------------------

struct Dumper<'a> {
    arena: &'a NameArena,
    /// Internal key → canonical id.
    ids: HashMap<String, String>,
    nl: u32,
    n: u32,
}

impl Dumper<'_> {
    /// Assign canonical ids in the Perl dumper's traversal order: entries by
    /// sorted key, entry hash keys sorted (`datafields`, `derivedfields`,
    /// then slots), fields sorted; a list gets its id before its names.
    fn collect_entries(&mut self, entries: &EntryMap) {
        for e in entries.values() {
            for map in [&e.datafields, &e.derivedfields] {
                for v in map.values().flatten() {
                    if let Value::Names(id) = v {
                        self.collect_list(*id);
                    }
                }
            }
        }
    }

    fn collect_list(&mut self, id: NameListId) {
        let k = id.key();
        if self.ids.contains_key(&k) {
            return;
        }
        self.nl += 1;
        self.ids.insert(k, format!("__nl{}__", self.nl));
        for nid in &self.arena.list(id).names {
            let nk = nid.key();
            if !self.ids.contains_key(&nk) {
                self.n += 1;
                self.ids.insert(nk, format!("__n{}__", self.n));
            }
        }
    }

    fn canon(&self, v: &J) -> J {
        map_ids(v, &|x| {
            if is_internal_id(x) {
                Some(
                    self.ids
                        .get(x)
                        .cloned()
                        .unwrap_or_else(|| "__unreachable__".into()),
                )
            } else {
                None
            }
        })
    }

    fn name(&self, id: NameId) -> J {
        let n = self.arena.name(id);
        let mut o = Map::new();
        o.insert("__class".into(), J::from(NAME_CLASS));
        o.insert("id".into(), J::from(self.ids[&id.key()].clone()));
        if let Some(i) = &n.index {
            o.insert("index".into(), J::from(i.as_str()));
        }
        let mut np = Map::new();
        for (part, p) in &n.nameparts {
            let mut po = Map::new();
            po.insert("string".into(), opt_cstr(&p.string));
            po.insert(
                "initial".into(),
                match &p.initial {
                    Some(v) => J::Array(v.iter().map(|x| J::from(x.as_str())).collect()),
                    None => J::Null,
                },
            );
            np.insert(part.to_string(), J::Object(po));
        }
        if !n.nameparts.is_empty() {
            o.insert("nameparts".into(), J::Object(np));
        }
        if !n.strip.is_empty() {
            o.insert(
                "strip".into(),
                J::Object(
                    n.strip
                        .iter()
                        .map(|(k, v)| {
                            let flag = v.map_or(J::Null, |f| J::from(if f { "1" } else { "0" }));
                            (k.to_string(), flag)
                        })
                        .collect(),
                ),
            );
        }
        if let Some(r) = &n.rawstring {
            o.insert("rawstring".into(), J::from(r.clone()));
        }
        for (k, v) in &n.slots {
            o.insert(k.to_string(), self.canon(v));
        }
        J::Object(o)
    }

    fn names(&self, id: NameListId) -> J {
        let l = self.arena.list(id);
        let mut o = Map::new();
        o.insert("__class".into(), J::from(NAMES_CLASS));
        o.insert("id".into(), J::from(self.ids[&id.key()].clone()));
        o.insert(
            "namelist".into(),
            J::Array(l.names.iter().map(|n| self.name(*n)).collect()),
        );
        if let Some(t) = &l.type_ {
            o.insert("type".into(), J::from(t.clone()));
        }
        for (k, v) in &l.slots {
            o.insert(k.to_string(), self.canon(v));
        }
        J::Object(o)
    }

    fn value(&self, v: &Option<Value>) -> J {
        match v {
            None => J::Null,
            Some(Value::Str(x)) => J::from(x.clone()),
            Some(Value::List(l)) => J::Array(l.iter().map(|x| J::from(x.clone())).collect()),
            Some(Value::Ranges(r)) => J::Array(
                r.iter()
                    .map(|p| J::Array(p.iter().map(opt_str).collect()))
                    .collect(),
            ),
            Some(Value::Names(id)) => self.names(*id),
            Some(Value::Other(o)) => self.canon(o),
        }
    }

    fn fields(&self, m: &FieldMap) -> J {
        J::Object(
            m.iter()
                .map(|(k, v)| (k.to_string(), self.value(v)))
                .collect(),
        )
    }

    fn entry(&self, e: &Entry) -> J {
        let mut o = Map::new();
        o.insert("__class".into(), J::from("Biber::Entry"));
        o.insert("datafields".into(), self.fields(&e.datafields));
        o.insert("derivedfields".into(), self.fields(&e.derivedfields));
        for (k, v) in &e.slots {
            o.insert(k.to_string(), self.canon(v));
        }
        if let Some(ldi) = &e.labeldateinfo {
            o.insert("labeldateinfo".into(), ldi.to_json());
        }
        J::Object(o)
    }
}

fn opt_cstr(v: &Option<compact_str::CompactString>) -> J {
    v.as_deref().map_or(J::Null, J::from)
}

fn opt_str(v: &Option<String>) -> J {
    match v {
        Some(x) => J::from(x.clone()),
        None => J::Null,
    }
}

fn str_array(v: &[String]) -> J {
    J::Array(v.iter().map(|x| J::from(x.clone())).collect())
}

fn key_hash(v: &BTreeSet<String>) -> J {
    J::Object(v.iter().map(|k| (k.clone(), J::from("1"))).collect())
}

/// Dump a [`SectionState`] in the Perl dumper's canonical format (the
/// `state` object of a phase dump).
pub fn dump_state(st: &SectionState) -> J {
    let sec = &st.section;
    let mut d = Dumper {
        arena: &sec.names,
        ids: HashMap::new(),
        nl: 0,
        n: 0,
    };
    d.collect_entries(&sec.entries);

    let mut so = Map::new();
    so.insert("__class".into(), J::from("Biber::Section"));
    so.insert("number".into(), J::from(sec.number.clone()));
    let mut be = Map::new();
    be.insert("__class".into(), J::from("Biber::Entries"));
    for (k, e) in &sec.entries {
        be.insert(k.clone(), d.entry(e));
    }
    so.insert("bibentries".into(), J::Object(be));
    so.insert("citekeys".into(), str_array(&sec.citekeys));
    so.insert("citekeys_h".into(), key_hash(&sec.citekeys_h));
    so.insert(
        "orig_order_citekeys".into(),
        str_array(&sec.orig_order_citekeys),
    );
    so.insert("undef_citekeys".into(), str_array(&sec.undef_citekeys));
    so.insert(
        "citekey_alias".into(),
        J::Object(
            sec.citekey_alias
                .iter()
                .map(|(k, v)| (k.clone(), J::from(v.clone())))
                .collect(),
        ),
    );
    so.insert(
        "allkeys".into(),
        J::from(if sec.allkeys { "1" } else { "0" }),
    );
    so.insert(
        "allkeysnocite".into(),
        J::from(if sec.allkeys_nocite { "1" } else { "0" }),
    );
    so.insert("cite_citekeys".into(), key_hash(&sec.cite_citekeys));
    so.insert("nocite_citekeys".into(), key_hash(&sec.nocite_citekeys));
    for (k, v) in &sec.slots {
        so.insert(k.to_string(), d.canon(v));
    }

    let lists = st
        .datalists
        .iter()
        .map(|dl| {
            let mut o = Map::new();
            o.insert("__class".into(), J::from("Biber::DataList"));
            for (k, v) in [
                ("name", &dl.name),
                ("type", &dl.type_),
                ("section", &dl.section),
                ("sortingtemplatename", &dl.sortingtemplatename),
                ("sortingnamekeytemplatename", &dl.sortingnamekeytemplatename),
                ("uniquenametemplatename", &dl.uniquenametemplatename),
                ("labelalphanametemplatename", &dl.labelalphanametemplatename),
                ("namehashtemplatename", &dl.namehashtemplatename),
                ("labelprefix", &dl.labelprefix),
            ] {
                o.insert(k.into(), J::from(v.clone()));
            }
            if let Some(k) = &dl.keys {
                o.insert("keys".into(), str_array(k));
            }
            if let Some(st) = &dl.state {
                o.insert("state".into(), d.canon(&st.to_json()));
            }
            for (k, v) in &dl.slots {
                o.insert(k.to_string(), d.canon(v));
            }
            if let Some(v) = dl.sortdata_json() {
                o.insert("sortdata".into(), d.canon(&v));
            }
            if let Some(v) = dl.sortinitdata_json() {
                o.insert("sortinitdata".into(), d.canon(&v));
            }
            J::Object(o)
        })
        .collect();

    let mut out = Map::new();
    out.insert("section".into(), J::Object(so));
    out.insert("datalists".into(), J::Array(lists));
    out.insert("config_state".into(), d.canon(&st.config_state));
    out.insert(
        "entry_options".into(),
        J::Object(
            st.entry_options
                .iter()
                .map(|(k, opts)| {
                    (
                        k.clone(),
                        J::Object(opts.iter().map(|(o, v)| (o.clone(), v.clone())).collect()),
                    )
                })
                .collect(),
        ),
    );
    out.insert(
        "preamble".into(),
        match &st.preamble {
            Some(p) => str_array(p),
            None => J::Null,
        },
    );
    if let Some(a) = &st.annotations {
        out.insert("annotations".into(), a.clone());
    }
    if let Some(g) = &st.shared_lists {
        out.insert(
            "shared_lists".into(),
            J::Array(
                g.iter()
                    .map(|m| {
                        J::Array(
                            m.iter()
                                .map(|(k, f)| J::Array(vec![k.clone().into(), f.clone().into()]))
                                .collect(),
                        )
                    })
                    .collect(),
            ),
        );
    }
    J::Object(out)
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// Structural difference between two dumps: a list of
/// `path: expected … got …` lines (at most `limit`). `ignore` holds path
/// patterns where `*` matches one path component, e.g.
/// `section.bibentries.*.datafields.*.namelist.*.rawstring`.
pub fn json_diff(expected: &J, actual: &J, ignore: &[&str], limit: usize) -> Vec<String> {
    let pats: Vec<Vec<&str>> = ignore.iter().map(|p| p.split('.').collect()).collect();
    let mut out = Vec::new();
    let mut path = Vec::new();
    walk_diff(expected, actual, &mut path, &pats, &mut out, limit);
    out
}

fn ignored(path: &[String], pats: &[Vec<&str>]) -> bool {
    pats.iter()
        .any(|p| p.len() == path.len() && p.iter().zip(path).all(|(a, b)| *a == "*" || a == b))
}

fn short(v: &J) -> String {
    let s = v.to_string();
    if s.chars().count() > 120 {
        format!("{}…", s.chars().take(120).collect::<String>())
    } else {
        s
    }
}

fn walk_diff(
    e: &J,
    a: &J,
    path: &mut Vec<String>,
    pats: &[Vec<&str>],
    out: &mut Vec<String>,
    limit: usize,
) {
    if out.len() >= limit || ignored(path, pats) {
        return;
    }
    match (e, a) {
        (J::Object(eo), J::Object(ao)) => {
            let keys: BTreeSet<&String> = eo.keys().chain(ao.keys()).collect();
            for k in keys {
                path.push(k.clone());
                match (eo.get(k), ao.get(k)) {
                    (Some(x), Some(y)) => walk_diff(x, y, path, pats, out, limit),
                    (Some(x), None) if !ignored(path, pats) => out.push(format!(
                        "{}: missing (expected {})",
                        path.join("."),
                        short(x)
                    )),
                    (None, Some(y)) if !ignored(path, pats) => {
                        out.push(format!("{}: unexpected {}", path.join("."), short(y)))
                    }
                    _ => {}
                }
                path.pop();
                if out.len() >= limit {
                    return;
                }
            }
        }
        (J::Array(ea), J::Array(aa)) if ea.len() == aa.len() => {
            for (i, (x, y)) in ea.iter().zip(aa).enumerate() {
                path.push(i.to_string());
                walk_diff(x, y, path, pats, out, limit);
                path.pop();
            }
        }
        _ if e != a => out.push(format!(
            "{}: expected {} got {}",
            path.join("."),
            short(e),
            short(a)
        )),
        _ => {}
    }
}

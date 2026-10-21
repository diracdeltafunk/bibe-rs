//! uniquename / uniquelist: the iterative `Biber::uniqueness` loop
//! (Biber.pm:3362-3965) and the `Biber::DataList` state it reads and writes
//! (DataList.pm:138-790, 2251-2324).
//!
//! The per-list state lives in the data list's `state` (biber's dumped
//! shape):
//! - `namelistdata.{nlid}.{nid}`: `un`, `unall`, `unmininfo` (written here),
//!   `namestring(s)`, `basenamestring(parts)`, `namedisschema` (written by
//!   `process_entries_pre`, read here); `namelistdata.{nlid}.ul`;
//! - `uniquenamecount`/`uniquenamecount_all`: `{namestring: {scope: {key:
//!   count}}}`;
//! - `uniquelistcount`: `{global: {prefix: count, final: {list: count,
//!   year: {list: count}}}, minyear: {visible list: {year: {list: count}}}}`.
//!
//! The counters are typed here (rebuilt on every pass, as in biber) and
//! written to the state in biber's shape when the loop ends; the per-name
//! data is read and written in place, including the entries biber's
//! accessors autovivify by reading through them.
//!
//! Differences from biber's data structures (same results):
//! - `namelist_differs_index`/`namelist_differs_nth` (BB-109, quadratic in
//!   biber: every call rescans every distinct complete list) are answered
//!   from a trie over the complete lists, built once per uniquelist pass.
//! - Prefix counts and complete-list counts are kept apart, so a name whose
//!   namestring is `final` no longer collides with the `final` sub-hash
//!   (BB-001, a crash in biber; DC-001). In the state dump the `final`
//!   sub-hash wins over a prefix count `final`.
//! - The iteration is capped (biber has no bound; see [`MAX_PASSES`]).

use super::Ctx;
use crate::log::Fatal;
use crate::model::{DataList, ListState, NameArena, NameListId, Section, SectionState, Value};
use crate::perlish::{numify, truthy};
use crate::quirks::{Quirk, Quirks};
use rustc_hash::FxHashMap as HashMap;
use crate::model::{PMap as Map, PV as J};
use serde_json::Value as SJ;
use std::rc::Rc;

/// The separator biber joins namestrings and name lists with.
const SEP: char = '\u{10FFFD}';

/// Upper bound on uniquename/uniquelist passes. Biber's loop has no bound
/// and could in principle cycle forever (a name whose disambiguation schema
/// is empty is re-flagged as changed on every pass); real data converges in
/// two or three passes.
const MAX_PASSES: usize = 1000;

/// `Biber::uniqueness` (Biber.pm:3362), for the data list `st.datalists[list]`.
pub fn uniqueness(ctx: &mut Ctx, st: &mut SectionState, list: usize) -> Result<(), Fatal> {
    let SectionState {
        section, datalists, ..
    } = st;
    let entries = entry_infos(ctx, section);
    let converged = run_counting(
        ctx.cfg.quirks,
        &section.names,
        &entries,
        &mut datalists[list],
        // Only biber's dumps show the counters after this phase.
        !ctx.run.free_dead_state,
    );
    if !converged {
        // biber never terminates here (BB-149); bibers stops (DC-052).
        ctx.log.biber_warn(format!(
            "uniquename/uniquelist processing did not converge after {MAX_PASSES} passes for list '{}' in section {} (biber would loop forever on this data); uniqueness information may be incomplete",
            datalists[list].name, section.number
        ));
    }
    Ok(())
}

/// What the uniqueness passes need of one citekey with a labelname, with
/// its options resolved (`getblxoption($secnum, …, $entrytype, $citekey)`;
/// they cannot change during the loop).
#[derive(Clone, Debug)]
pub struct EntryInfo {
    /// The labelname list.
    pub nl: NameListId,
    /// The `uniquename` option (`undef` → `None`).
    pub uniquename: Option<String>,
    /// The `uniquelist` option.
    pub uniquelist: Option<String>,
    /// `maxcitenames`, numified.
    pub maxcn: f64,
    /// `mincitenames`, numified.
    pub mincn: f64,
    /// The `labelyear` field.
    pub labelyear: Option<String>,
}

/// The citekeys (in order) that have a labelname list, with their options.
fn entry_infos(ctx: &Ctx, section: &Section) -> Vec<EntryInfo> {
    let secnum = section.number.as_str();
    let opt = |o: &str, et: &str, key: &str| {
        ctx.cfg
            .blx
            .get(Some(secnum), o, Some(et), Some(key))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let mut out = Vec::new();
    for key in &section.citekeys {
        let Some(be) = section.bibentry(key) else {
            continue;
        };
        let bee = be.entrytype();
        // `next unless defined($lni)`
        let Some(lni) = be.slots.get("labelnameinfo").and_then(SJ::as_str) else {
            continue;
        };
        let Some(nl) = be.get_field(lni).and_then(Value::as_names) else {
            continue;
        };
        let num = |o: &str| opt(o, bee, key).as_deref().map_or(0.0, numify);
        out.push(EntryInfo {
            nl,
            uniquename: opt("uniquename", bee, key),
            uniquelist: opt("uniquelist", bee, key),
            maxcn: num("maxcitenames"),
            mincn: num("mincitenames"),
            labelyear: be.get_str("labelyear").map(str::to_string),
        });
    }
    out
}

/// Run the uniqueness loop on a data list (the part of
/// [`uniqueness`] that doesn't need the option store).
/// Returns `false` if the iteration was stopped at [`MAX_PASSES`].
pub fn run(quirks: Quirks, names: &NameArena, entries: &[EntryInfo], dl: &mut DataList) -> bool {
    run_counting(quirks, names, entries, dl, true)
}

/// [`run`], leaving the counters (`uniquenamecount`, `uniquenamecount_all`,
/// `uniquelistcount`) in the state only if `keep_counts`.
fn run_counting(
    quirks: Quirks,
    names: &NameArena,
    entries: &[EntryInfo],
    dl: &mut DataList,
    keep_counts: bool,
) -> bool {
    let changed = dl
        .slots
        .get("unulchanged")
        .is_some_and(|v| truthy(v.as_str()));
    let mut state = dl.state.take().unwrap_or_default();
    let mut u = Uniq {
        quirks,
        names,
        entries,
        nld: NameListData::take(&mut state),
        changed,
        unc: None,
        unc_all: None,
        ulc: None,
        statics: HashMap::default(),
        lists: HashMap::default(),
    };
    let converged = u.iterate();
    let Uniq {
        changed,
        unc,
        unc_all,
        ulc,
        nld,
        statics,
        lists,
        ..
    } = u;
    drop((statics, lists));
    nld.finish(&mut state);
    let (unc, unc_all, ulc) = if keep_counts {
        (unc, unc_all, ulc)
    } else {
        (None, None, None)
    };
    if let Some(c) = unc {
        state.insert("uniquenamecount", c.to_json());
    }
    if let Some(c) = unc_all {
        state.insert("uniquenamecount_all", c.to_json());
    }
    if let Some(c) = ulc {
        state.insert("uniquelistcount", c.to_json());
    }
    dl.state = Some(state);
    dl.slots
        .insert("unulchanged", SJ::from(if changed { "1" } else { "0" }));
    converged
}

// ---------------------------------------------------------------------------
// Per-name state (`namelistdata`)
// ---------------------------------------------------------------------------

/// `$dlist->{state}{namelistdata}`, accessed like biber's accessors: reading
/// `{nlid}{nid}{x}` autovivifies `{nlid}{nid}`, reading `{nlid}{ul}`
/// autovivifies `{nlid}`.
///
/// The name lists the passes touch are taken out of the (large, string
/// keyed) `namelistdata` hash into a map by id while the passes run, and put
/// back by [`NameListData::finish`].
struct NameListData {
    /// `namelistdata` as found (`None` if absent), minus the lists taken.
    orig: Option<HashMap<String, J>>,
    lists: HashMap<NameListId, Map>,
}

fn obj_entry<'m>(m: &'m mut Map, k: &str) -> &'m mut Map {
    if !matches!(m.get(k), Some(J::Object(_))) {
        m.insert(k.to_string(), J::Object(Map::new()));
    }
    match m.get_mut(k) {
        Some(J::Object(o)) => o,
        _ => unreachable!(),
    }
}

/// A defined value (`undef`/absent → `None`).
fn defined(v: Option<&J>) -> Option<&J> {
    v.filter(|x| !x.is_null())
}

/// A scalar as Perl would stringify it (`undef` → `""`).
fn s_or_empty(v: Option<&J>) -> String {
    match v {
        Some(J::String(s)) => s.to_string(),
        Some(J::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

impl NameListData {
    /// Take `namelistdata` out of the list state `state`.
    fn take(state: &mut ListState) -> Self {
        NameListData {
            orig: state.take_namelistdata(),
            lists: HashMap::default(),
        }
    }

    /// Put `namelistdata` back into `state`, with everything accessed
    /// vivified as biber's accessors do.
    fn finish(self, state: &mut ListState) {
        if self.lists.is_empty() {
            if self.orig.is_some() {
                state.set_namelistdata(self.orig);
            }
            return;
        }
        let mut m = self.orig.unwrap_or_default();
        for (id, l) in self.lists {
            m.insert(id.key(), J::Object(l));
        }
        state.set_namelistdata(Some(m));
    }

    fn nl(&mut self, nlid: NameListId) -> &mut Map {
        let orig = &mut self.orig;
        self.lists
            .entry(nlid)
            .or_insert_with(|| match orig.as_mut() {
                Some(m) => match m.remove(&nlid.key()) {
                    Some(J::Object(l)) => l,
                    _ => Map::new(),
                },
                None => Map::new(),
            })
    }

    fn nd(&mut self, nlid: NameListId, nid: &str) -> &mut Map {
        obj_entry(self.nl(nlid), nid)
    }

    /// `get_uniquelist($nlid)`.
    fn ul(&mut self, nlid: NameListId) -> Option<String> {
        defined(self.nl(nlid).get("ul")).map(|v| s_or_empty(Some(v)))
    }
}

/// The per-name data written by `process_entries_pre`, which doesn't change
/// during the loop (`get_basenamestring`, `get_namestring(s)`). The
/// `namedisschema` is read in place when needed.
struct NameStatic {
    basename: Option<String>,
    namestring: Option<String>,
    namestrings: Vec<Option<String>>,
    /// `join("\x{10FFFD}", $namestrings->@*)`.
    joined: String,
}

impl NameStatic {
    fn read(d: &Map) -> NameStatic {
        let opt_s = |k: &str| defined(d.get(k)).map(|v| s_or_empty(Some(v)));
        let namestrings: Vec<Option<String>> = match d.get("namestrings") {
            Some(J::Array(a)) => a
                .iter()
                .map(|v| defined(Some(v)).map(|v| s_or_empty(Some(v))))
                .collect(),
            _ => Vec::new(),
        };
        NameStatic {
            basename: opt_s("basenamestring"),
            namestring: opt_s("namestring"),
            joined: join(namestrings.iter().map(|s| s.as_deref().unwrap_or(""))),
            namestrings,
        }
    }
}

/// A name's `namedisschema` (`[]` if unset).
fn schema(d: &Map) -> &[J] {
    match d.get("namedisschema") {
        Some(J::Array(a)) => a,
        _ => &[],
    }
}

fn join<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for (i, p) in parts.into_iter().enumerate() {
        if i > 0 {
            out.push(SEP);
        }
        out.push_str(p);
    }
    out
}

fn count_json(n: u64) -> J {
    J::from(n.to_string())
}

// ---------------------------------------------------------------------------
// uniquename counters
// ---------------------------------------------------------------------------

/// Two levels of string-keyed hashes over counts.
type Counts3 = HashMap<String, HashMap<String, HashMap<String, u64>>>;

/// `uniquenamecount` or `uniquenamecount_all`: namestring → scope → key →
/// count.
#[derive(Default)]
struct NameCount(Counts3);

impl NameCount {
    /// The scope hash `{ns}{scope}`, created if missing.
    fn scope(&mut self, ns: &str, scope: &str) -> &mut HashMap<String, u64> {
        if !self.0.contains_key(ns) {
            self.0.insert(ns.to_string(), HashMap::default());
        }
        let scopes = self.0.get_mut(ns).unwrap();
        if !scopes.contains_key(scope) {
            scopes.insert(scope.to_string(), HashMap::default());
        }
        scopes.get_mut(scope).unwrap()
    }

    /// `add_uniquenamecount`.
    fn add(&mut self, ns: &str, scope: &str, key: &str) {
        let keys = self.scope(ns, scope);
        match keys.get_mut(key) {
            Some(c) => *c += 1,
            None => {
                keys.insert(key.to_string(), 1);
            }
        }
    }

    /// `get_numofuniquenames`: the number of distinct keys (autovivifies the
    /// scope hash).
    fn num(&mut self, ns: &str, scope: &str) -> usize {
        self.scope(ns, scope).len()
    }

    fn to_json(&self) -> J {
        J::Object(
            self.0
                .iter()
                .map(|(ns, scopes)| {
                    (
                        ns.clone(),
                        J::Object(
                            scopes
                                .iter()
                                .map(|(sc, keys)| {
                                    (
                                        sc.clone(),
                                        J::Object(
                                            keys.iter()
                                                .map(|(k, c)| (k.clone(), count_json(*c)))
                                                .collect(),
                                        ),
                                    )
                                })
                                .collect(),
                        ),
                    )
                })
                .collect(),
        )
    }
}

// ---------------------------------------------------------------------------
// uniquelist counters and the complete-list trie
// ---------------------------------------------------------------------------

/// `uniquelistcount` (`None` members are hashes not yet vivified).
#[derive(Default)]
struct ListCount {
    global: Option<GlobalCount>,
    /// visible-list key → year → complete-list key → count.
    minyear: Option<Counts3>,
}

#[derive(Default)]
struct GlobalCount {
    /// `{global}{<prefix>}`.
    prefix: HashMap<String, u64>,
    /// `{global}{final}`.
    fin: Option<FinalCount>,
}

#[derive(Default)]
struct FinalCount {
    /// `{global}{final}{<list>}`.
    lists: HashMap<String, u64>,
    /// `{global}{final}{<labelyear>}{<list>}` (uniquelist=minyear).
    years: HashMap<String, HashMap<String, u64>>,
    /// Trie over every key of `{global}{final}` (lists *and* years, which
    /// biber also scans as pseudo-lists), built on first use.
    trie: Option<Trie>,
    /// Tries over each year's lists.
    year_tries: HashMap<String, Trie>,
}

impl FinalCount {
    /// The trie over all keys of `{final}`.
    fn trie(&mut self) -> &Trie {
        if self.trie.is_none() {
            let mut t = Trie::new();
            for k in self.lists.keys() {
                t.insert(k);
            }
            for y in self.years.keys() {
                if !self.lists.contains_key(y) {
                    t.insert(y);
                }
            }
            self.trie = Some(t);
        }
        self.trie.as_ref().unwrap()
    }
}

impl ListCount {
    fn global(&mut self) -> &mut GlobalCount {
        self.global.get_or_insert_with(Default::default)
    }

    fn fin(&mut self) -> &mut FinalCount {
        self.global().fin.get_or_insert_with(Default::default)
    }

    /// `add_uniquelistcount`.
    fn add_prefix(&mut self, key: &str) {
        *self.global().prefix.entry(key.to_string()).or_default() += 1;
    }

    /// `get_uniquelistcount` (vivifies `{global}`).
    fn prefix_count(&mut self, key: &str) -> Option<u64> {
        self.global().prefix.get(key).copied()
    }

    /// `add_uniquelistcount_final`.
    fn add_final(&mut self, key: &str, labelyear: Option<&str>) {
        let f = self.fin();
        f.trie = None;
        f.year_tries.clear();
        *f.lists.entry(key.to_string()).or_default() += 1;
        if let Some(y) = labelyear.filter(|y| truthy(Some(y))) {
            *f.years
                .entry(y.to_string())
                .or_default()
                .entry(key.to_string())
                .or_default() += 1;
        }
    }

    /// `get_uniquelistcount_final`.
    fn final_count(&mut self, key: &str) -> u64 {
        self.fin().lists.get(key).copied().unwrap_or(0)
    }

    /// `add_uniquelistcount_minyear`.
    fn add_minyear(&mut self, visible: &str, year: Option<&str>, key: &str) {
        *self
            .minyear
            .get_or_insert_with(Default::default)
            .entry(visible.to_string())
            .or_default()
            .entry(year.unwrap_or("0").to_string())
            .or_default()
            .entry(key.to_string())
            .or_default() += 1;
    }

    /// `get_uniquelistcount_minyear` (vivifies the year hash).
    fn minyear_count(&mut self, visible: &str, year: Option<&str>) -> usize {
        self.minyear
            .get_or_insert_with(Default::default)
            .entry(visible.to_string())
            .or_default()
            .entry(year.unwrap_or("0").to_string())
            .or_default()
            .len()
    }

    /// `namelist_differs_index` (DataList.pm:2251): the 0-based index at
    /// which `list` starts to differ from the most similar *other* complete
    /// list, or the last index if another list extends it; `None` if no
    /// other list shares its first name.
    fn differs_index(&mut self, list: &[String]) -> Option<usize> {
        self.fin().trie().differs_index(list)
    }

    /// `namelist_differs_nth` (DataList.pm:2298): is there a complete list
    /// at least as long as `list` that equals it up to `n - 1` and differs
    /// at `n`? With uniquelist=minyear only lists of the same labelyear
    /// count.
    fn differs_nth(
        &mut self,
        list: &[String],
        n: usize,
        ul: &str,
        labelyear: Option<&str>,
    ) -> bool {
        let f = self.fin();
        if ul == "minyear" {
            let y = labelyear.unwrap_or("");
            let Some(lists) = f.years.get(y) else {
                return false;
            };
            let t = f.year_tries.entry(y.to_string()).or_insert_with(|| {
                let mut t = Trie::new();
                for k in lists.keys() {
                    t.insert(k);
                }
                t
            });
            return t.differs_nth(list, n);
        }
        f.trie().differs_nth(list, n)
    }

    fn to_json(&self) -> J {
        let mut o = Map::new();
        if let Some(g) = &self.global {
            let mut go: Map = g
                .prefix
                .iter()
                .map(|(k, c)| (k.clone(), count_json(*c)))
                .collect();
            if let Some(f) = &g.fin {
                let mut fo: Map = f
                    .lists
                    .iter()
                    .map(|(k, c)| (k.clone(), count_json(*c)))
                    .collect();
                for (y, m) in &f.years {
                    fo.insert(
                        y.clone(),
                        J::Object(m.iter().map(|(k, c)| (k.clone(), count_json(*c))).collect()),
                    );
                }
                // DC-001: the `final` sub-hash wins over a prefix `final`.
                go.insert("final".into(), J::Object(fo));
            }
            o.insert("global".into(), J::Object(go));
        }
        if let Some(m) = &self.minyear {
            o.insert(
                "minyear".into(),
                J::Object(
                    m.iter()
                        .map(|(v, years)| {
                            (
                                v.clone(),
                                J::Object(
                                    years
                                        .iter()
                                        .map(|(y, lists)| {
                                            (
                                                y.clone(),
                                                J::Object(
                                                    lists
                                                        .iter()
                                                        .map(|(k, c)| (k.clone(), count_json(*c)))
                                                        .collect(),
                                                ),
                                            )
                                        })
                                        .collect(),
                                ),
                            )
                        })
                        .collect(),
                ),
            );
        }
        J::Object(o)
    }
}

/// Perl's `split("\x{10FFFD}", $s)`: trailing empty fields are dropped (so
/// `""` gives no fields).
fn perl_split(s: &str) -> Vec<&str> {
    let mut v: Vec<&str> = s.split(SEP).collect();
    while v.last().is_some_and(|x| x.is_empty()) {
        v.pop();
    }
    v
}

/// A trie over a set of hash keys, each split into a name list as biber
/// splits them. Each node records how many keys pass through it, how many
/// end exactly there, and the longest key below it.
pub(crate) struct Trie {
    nodes: Vec<TrieNode>,
}

#[derive(Default)]
struct TrieNode {
    children: HashMap<String, usize>,
    /// Keys with this prefix.
    count: usize,
    /// Keys equal to this prefix.
    term: usize,
    /// Length of the longest key with this prefix.
    max_len: usize,
}

impl Trie {
    pub(crate) fn new() -> Trie {
        Trie {
            nodes: vec![TrieNode::default()],
        }
    }

    /// Add a hash key (a joined list).
    pub(crate) fn insert(&mut self, key: &str) {
        let parts = perl_split(key);
        let len = parts.len();
        let mut node = 0;
        self.bump(node, len);
        for p in parts {
            node = match self.nodes[node].children.get(p) {
                Some(&c) => c,
                None => {
                    let c = self.nodes.len();
                    self.nodes.push(TrieNode::default());
                    self.nodes[node].children.insert(p.to_string(), c);
                    c
                }
            };
            self.bump(node, len);
        }
        self.nodes[node].term += 1;
    }

    fn bump(&mut self, node: usize, len: usize) {
        let n = &mut self.nodes[node];
        n.count += 1;
        n.max_len = n.max_len.max(len);
    }

    /// The nodes along `list`'s path, as far as it exists in the trie.
    fn path(&self, list: &[String]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut node = 0;
        for x in list {
            match self.nodes[node].children.get(x) {
                Some(&c) => {
                    out.push(c);
                    node = c;
                }
                None => break,
            }
        }
        out
    }

    /// `namelist_differs_index` over the keys of this trie.
    pub(crate) fn differs_index(&self, list: &[String]) -> Option<usize> {
        if list.is_empty() {
            return None;
        }
        let path = self.path(list);
        // Keys identical to `list` are skipped (`Compare`).
        let same = if path.len() == list.len() {
            self.nodes[*path.last().unwrap()].term
        } else {
            0
        };
        // The deepest prefix of `list` shared with some other key.
        let mut best = None;
        for (i, &n) in path.iter().enumerate() {
            if self.nodes[n].count > same {
                best = Some(i);
            } else {
                break;
            }
        }
        best.map(|i| if i == list.len() - 1 { i } else { i + 1 })
    }

    /// `namelist_differs_nth` over the keys of this trie.
    pub(crate) fn differs_nth(&self, list: &[String], n: usize) -> bool {
        if n == 0 || n > list.len() {
            return false;
        }
        let path = self.path(&list[..n - 1]);
        if path.len() != n - 1 {
            return false;
        }
        let node = path.last().copied().unwrap_or(0);
        self.nodes[node]
            .children
            .iter()
            .any(|(k, &c)| *k != list[n - 1] && self.nodes[c].max_len >= list.len())
    }
}

// ---------------------------------------------------------------------------
// The passes
// ---------------------------------------------------------------------------

struct Uniq<'a> {
    quirks: Quirks,
    names: &'a NameArena,
    entries: &'a [EntryInfo],
    nld: NameListData,
    /// The data list's `unulchanged` flag.
    changed: bool,
    unc: Option<NameCount>,
    unc_all: Option<NameCount>,
    ulc: Option<ListCount>,
    /// Cache of [`NameStatic`] by name.
    statics: HashMap<crate::model::NameId, Rc<NameStatic>>,
    /// Cache of the names of each list.
    lists: HashMap<NameListId, Rc<Vec<NameRef>>>,
}

fn is_min(un: &str) -> bool {
    matches!(un, "mininit" | "minfull" | "minyearinit" | "minyearfull")
}

/// A name slot as a defined string (`$n->get_uniquename` etc.).
fn slot(slots: &crate::model::Slots, k: &str) -> Option<String> {
    match slots.get(k) {
        None | Some(SJ::Null) => None,
        Some(SJ::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    }
}

/// One name of a labelname list, as the passes see it.
struct NameRef {
    id: crate::model::NameId,
    nid: String,
    /// `get_index`, numified (`undef` → 0).
    index: f64,
    /// Per-name `uniquename`.
    uniquename: Option<String>,
}

impl<'a> Uniq<'a> {
    fn leak(&self) -> bool {
        self.quirks.on(Quirk::PerNameOptionLeak)
    }

    fn names_of(&mut self, nl: NameListId) -> Rc<Vec<NameRef>> {
        let names = self.names;
        self.lists
            .entry(nl)
            .or_insert_with(|| {
                Rc::new(
                    names
                        .names_of(nl)
                        .map(|n| NameRef {
                            id: n.id,
                            nid: n.id.key(),
                            index: n.index.as_deref().map_or(0.0, numify),
                            uniquename: slot(&n.slots, "uniquename"),
                        })
                        .collect(),
                )
            })
            .clone()
    }

    /// A name's data (autovivifies `namelistdata.{nlid}.{nid}`, as reading
    /// any of it does in biber).
    fn data(&mut self, nlid: NameListId, n: &NameRef) -> Rc<NameStatic> {
        let d = self.nld.nd(nlid, &n.nid);
        match self.statics.get(&n.id) {
            Some(st) => st.clone(),
            None => {
                let st = Rc::new(NameStatic::read(d));
                self.statics.insert(n.id, st.clone());
                st
            }
        }
    }

    /// The namestring(s) that represent a name in uniquelist keys: the
    /// basename when `unall` is unset or `base`, else the namestrings whose
    /// schema element equals `unall` (Biber.pm:3836-3849).
    fn ul_strings(&mut self, nlid: NameListId, n: &NameRef) -> Vec<Option<String>> {
        let st = self.data(nlid, n);
        let d = self.nld.nd(nlid, &n.nid);
        let unall = defined(d.get("unall"));
        let base = match unall {
            None => true,
            Some(J::Array(a)) => a.first().and_then(J::as_str) == Some("base"),
            Some(_) => false,
        };
        let Some(unall) = unall.filter(|_| !base) else {
            return vec![st.basename.clone()];
        };
        schema(d)
            .iter()
            .enumerate()
            .filter(|(_, nss)| *nss == unall)
            .map(|(i, _)| st.namestrings.get(i).cloned().flatten())
            .collect()
    }

    /// Element `i` of a name's `namedisschema` (`undef` if absent).
    fn schema_elem(&mut self, nlid: NameListId, nid: &str, i: usize) -> J {
        schema(self.nld.nd(nlid, nid))
            .get(i)
            .cloned()
            .unwrap_or(J::Null)
    }

    fn done(&self) -> bool {
        !self.changed
    }

    /// The loop of `Biber::uniqueness` (Biber.pm:3387-3414). Returns
    /// `false` if it did not converge within [`MAX_PASSES`] (biber would
    /// loop forever: BB-149).
    fn iterate(&mut self) -> bool {
        let mut first_ul_pass = true;
        for _ in 0..MAX_PASSES {
            if !self.done() {
                self.changed = false;
                self.create_uniquename_info();
                self.generate_uniquename();
            } else {
                return true;
            }
            if first_ul_pass || !self.done() {
                self.changed = false;
                first_ul_pass = false;
                self.create_uniquelist_info();
                self.generate_uniquelist();
            } else {
                return true;
            }
        }
        false
    }

    /// The per-name uniquename values of a list's names, as the first name
    /// loop of `create_uniquename_info`/`generate_uniquename` sees them:
    /// per-name overrides leak to following names (BB-019,
    /// `per-name-option-leak`). Returns the value per name and the value
    /// left in `$un` after the loop.
    fn name_uns(
        &self,
        base: &Option<String>,
        ns: &[NameRef],
    ) -> (Vec<Option<String>>, Option<String>) {
        let leak = self.leak();
        let mut un = base.clone();
        let mut out = Vec::with_capacity(ns.len());
        for n in ns {
            if let Some(o) = &n.uniquename {
                un = Some(o.clone());
            } else if !leak {
                un = base.clone();
            }
            out.push(un.clone());
        }
        (out, un)
    }

    /// `create_uniquename_info` (Biber.pm:3475).
    fn create_uniquename_info(&mut self) {
        // `reset_uniquenamecount`
        let mut unc = NameCount::default();
        let mut unc_all = NameCount::default();
        let leak = self.leak();
        let skip_entry = self.quirks.on(Quirk::UniquenameFalseSkipsEntry);
        let minyear_visible = self.quirks.on(Quirk::UniquenameMinyearVisibility);
        'main: for e in self.entries {
            let list = self.names.list(e.nl);
            let nlid = e.nl;
            let mut base_un = e.uniquename.clone();
            if let Some(v) = slot(&list.slots, "uniquename") {
                base_un = Some(v);
            }
            let ul = self.nld.ul(nlid);
            let ul_n = ul.as_deref().filter(|u| truthy(Some(u))).map(numify);
            let names = self.names_of(e.nl);
            let num_names = names.len() as f64;
            let morenames = list
                .slots
                .get("morenames")
                .is_some_and(|v| truthy(v.as_str()));
            let (uns, final_un) = self.name_uns(&base_un, &names);

            let mut trunc = vec![false; names.len()];
            let mut skip = vec![false; names.len()];
            let mut basenames: Vec<String> = Vec::new();
            let mut allnames: Vec<String> = Vec::new();
            for (i, n) in names.iter().enumerate() {
                let un = uns[i].as_deref().unwrap_or("");
                if un == "false" {
                    if skip_entry {
                        continue 'main;
                    }
                    skip[i] = true;
                    continue;
                }
                let visible = matches!(un, "allinit" | "allfull")
                    || (minyear_visible && matches!(un, "minyearinit" | "minyearfull"))
                    || ul_n.is_some_and(|u| n.index <= u)
                    || morenames
                    || num_names <= e.maxcn
                    || n.index <= e.mincn;
                if visible {
                    trunc[i] = true;
                    if is_min(un) {
                        let d = self.data(nlid, n);
                        basenames.push(d.basename.clone().unwrap_or_default());
                        allnames.push(d.namestring.clone().unwrap_or_default());
                    }
                }
            }

            let mut min_basename = join(basenames.iter().map(String::as_str));
            let mut min_namestring = join(allnames.iter().map(String::as_str));
            if (basenames.len() as f64) < num_names || morenames {
                min_basename.push_str("\u{10FFFD}et al");
                min_namestring.push_str("\u{10FFFD}et al");
            }
            let lyear = e.labelyear.clone().unwrap_or_default();

            // Per-namelist uniquelist: note the truth test (not `defined`).
            let mut eul = e.uniquelist.clone();
            if let Some(v) = slot(&list.slots, "uniquelist").filter(|v| truthy(Some(v))) {
                eul = Some(v);
            }
            let eul_on = eul.as_deref() != Some("false");

            for (i, n) in names.iter().enumerate() {
                if skip[i] {
                    continue;
                }
                let un = if leak { &final_un } else { &uns[i] };
                let d = self.data(nlid, n);
                let (scope, key) = match un.as_deref().unwrap_or("") {
                    "init" | "full" | "allinit" | "allfull" => {
                        ("global".to_string(), d.joined.clone())
                    }
                    "mininit" | "minfull" => {
                        self.nld
                            .nd(nlid, &n.nid)
                            .insert("unmininfo".into(), J::from(min_basename.clone()));
                        (min_basename.clone(), min_namestring.clone())
                    }
                    "minyearinit" | "minyearfull" => {
                        let s = format!("{min_basename}{lyear}");
                        self.nld
                            .nd(nlid, &n.nid)
                            .insert("unmininfo".into(), J::from(s.clone()));
                        (s, format!("{min_namestring}{lyear}"))
                    }
                    _ => (String::new(), String::new()),
                };
                if trunc[i] {
                    for ns in &d.namestrings {
                        unc.add(ns.as_deref().unwrap_or(""), &scope, &key);
                    }
                }
                if eul_on {
                    for ns in &d.namestrings {
                        unc_all.add(ns.as_deref().unwrap_or(""), &scope, &key);
                    }
                }
            }
        }
        self.unc = Some(unc);
        self.unc_all = Some(unc_all);
    }

    /// `set_uniquename`: flags a change.
    fn set_uniquename(&mut self, nlid: NameListId, nid: &str, v: J) {
        let d = self.nld.nd(nlid, nid);
        match defined(d.get("un")) {
            Some(cur) if *cur == v => {}
            _ => self.changed = true,
        }
        d.insert("un".into(), v);
    }

    /// `generate_uniquename` (Biber.pm:3653).
    fn generate_uniquename(&mut self) {
        let leak = self.leak();
        let skip_entry = self.quirks.on(Quirk::UniquenameFalseSkipsEntry);
        let mut unc = self.unc.take().unwrap_or_default();
        let mut unc_all = self.unc_all.take().unwrap_or_default();
        'main: for e in self.entries {
            let list = self.names.list(e.nl);
            let nlid = e.nl;
            let mut base_un = e.uniquename.clone();
            if let Some(v) = slot(&list.slots, "uniquename") {
                base_un = Some(v);
            }
            let ul = self.nld.ul(nlid);
            let ul_n = ul.as_deref().filter(|u| truthy(Some(u))).map(numify);
            let names = self.names_of(e.nl);
            let num_names = names.len() as f64;
            let morenames = list
                .slots
                .get("morenames")
                .is_some_and(|v| truthy(v.as_str()));
            let (uns, final_un) = self.name_uns(&base_un, &names);

            let mut trunc = vec![false; names.len()];
            let mut skip = vec![false; names.len()];
            for (i, n) in names.iter().enumerate() {
                let un = uns[i].as_deref().unwrap_or("");
                if un == "false" {
                    if skip_entry {
                        continue 'main;
                    }
                    skip[i] = true;
                    continue;
                }
                // Unlike create_uniquename_info, minyear* doesn't make
                // every name visible here (BB-095).
                let visible = matches!(un, "allinit" | "allfull")
                    || ul_n.is_some_and(|u| n.index <= u)
                    || morenames
                    || num_names <= e.maxcn
                    || n.index <= e.mincn;
                if visible {
                    trunc[i] = true;
                } else {
                    // `reset_uniquename`: no change flag.
                    let d = self.nld.nd(nlid, &n.nid);
                    let parts = d.get("basenamestringparts").cloned().unwrap_or(J::Null);
                    d.insert("un".into(), J::Array(vec![J::from("base"), parts]));
                }
            }

            // Per-namelist uniquelist (`defined` here).
            let mut eul = e.uniquelist.clone();
            if let Some(v) = slot(&list.slots, "uniquelist") {
                eul = Some(v);
            }
            let eul_on = eul.as_deref() != Some("false");

            for (i, n) in names.iter().enumerate() {
                if skip[i] {
                    continue;
                }
                let un = if leak { &final_un } else { &uns[i] };
                let d = self.data(nlid, n);
                let scope = if is_min(un.as_deref().unwrap_or("")) {
                    s_or_empty(self.nld.nd(nlid, &n.nid).get("unmininfo"))
                } else {
                    "global".to_string()
                };
                if trunc[i] {
                    for (j, ns) in d.namestrings.iter().enumerate() {
                        if unc.num(ns.as_deref().unwrap_or(""), &scope) == 1 {
                            let v = self.schema_elem(nlid, &n.nid, j);
                            self.set_uniquename(nlid, &n.nid, v);
                            break;
                        }
                    }
                    // Nothing disambiguates: base, unless already defined
                    // (possibly by an earlier pass).
                    if defined(self.nld.nd(nlid, &n.nid).get("un")).is_none() {
                        let v = self.schema_elem(nlid, &n.nid, 0);
                        self.set_uniquename(nlid, &n.nid, v);
                    }
                }
                if eul_on {
                    // `set_uniquename_all`: no change flag.
                    for (j, ns) in d.namestrings.iter().enumerate() {
                        if unc_all.num(ns.as_deref().unwrap_or(""), &scope) == 1 {
                            let v = self.schema_elem(nlid, &n.nid, j);
                            self.nld.nd(nlid, &n.nid).insert("unall".into(), v);
                            break;
                        }
                    }
                    let nd = self.nld.nd(nlid, &n.nid);
                    if defined(nd.get("unall")).is_none() {
                        let v = schema(nd).first().cloned().unwrap_or(J::Null);
                        nd.insert("unall".into(), v);
                    }
                }
            }
        }
        self.unc = Some(unc);
        self.unc_all = Some(unc_all);
    }

    /// The effective uniquelist option of an entry (`defined` override).
    fn entry_ul(&self, e: &EntryInfo) -> Option<String> {
        slot(&self.names.list(e.nl).slots, "uniquelist").or_else(|| e.uniquelist.clone())
    }

    /// `create_uniquelist_info` (Biber.pm:3778).
    fn create_uniquelist_info(&mut self) {
        // `reset_uniquelistcount`
        let mut ulc = ListCount::default();
        let double = self.quirks.on(Quirk::UniquelistMinyearDoubleCount);
        for e in self.entries {
            let ul = self.entry_ul(e);
            let ul = ul.as_deref().unwrap_or("");
            if ul == "false" {
                continue;
            }
            let nlid = e.nl;
            let names = self.names_of(e.nl);
            let num_names = names.len() as f64;
            let mut namelist: Vec<String> = Vec::new();
            let mut joined = String::new();
            let mut minyear_list: Vec<Option<String>> = Vec::new();
            for n in names.iter() {
                let flag = ul == "minyear" && num_names > e.maxcn && n.index <= e.mincn;
                for s in self.ul_strings(nlid, n) {
                    if flag {
                        minyear_list.push(s.clone());
                    }
                    if let Some(s) = s {
                        if !namelist.is_empty() {
                            joined.push(SEP);
                        }
                        joined.push_str(&s);
                        namelist.push(s);
                    }
                }
                ulc.add_prefix(&joined);
            }
            let year = e.labelyear.as_deref();
            if ul == "minyear" {
                // BB-097: the complete list is counted twice.
                if double {
                    ulc.add_final(&joined, None);
                }
                ulc.add_final(&joined, year);
            } else {
                ulc.add_final(&joined, None);
            }
            if !minyear_list.is_empty() {
                let v = join(minyear_list.iter().map(|s| s.as_deref().unwrap_or("")));
                ulc.add_minyear(&v, year, &joined);
            }
        }
        self.ulc = Some(ulc);
    }

    /// `generate_uniquelist` (Biber.pm:3882).
    fn generate_uniquelist(&mut self) {
        let mut ulc = self.ulc.take().unwrap_or_default();
        'main: for e in self.entries {
            let ul = self.entry_ul(e);
            let ul = ul.as_deref().unwrap_or("");
            if ul == "false" {
                continue;
            }
            let nlid = e.nl;
            let names = self.names_of(e.nl);
            let num_names = names.len() as f64;
            let year = e.labelyear.as_deref();
            let mut namelist: Vec<String> = Vec::new();
            let mut joined = String::new();
            for n in names.iter() {
                for s in self.ul_strings(nlid, n).into_iter().flatten() {
                    if !namelist.is_empty() {
                        joined.push(SEP);
                    }
                    joined.push_str(&s);
                    namelist.push(s);
                }
                // uniquelist=minyear: no uniquelist at all unless another
                // entry of the same year has the same visible list.
                if ul == "minyear"
                    && num_names > e.maxcn
                    && n.index <= e.mincn
                    && ulc.minyear_count(&joined, year) == 1
                {
                    continue 'main;
                }
                if ulc.prefix_count(&joined) == Some(1) {
                    break;
                }
            }
            self.set_uniquelist(&mut ulc, e, nlid, &namelist, &joined, ul);
        }
        self.ulc = Some(ulc);
    }

    /// `DataList::set_uniquelist` (DataList.pm:571).
    fn set_uniquelist(
        &mut self,
        ulc: &mut ListCount,
        e: &EntryInfo,
        nlid: NameListId,
        namelist: &[String],
        joined: &str,
        ul: &str,
    ) {
        let mut uniquelist = namelist.len();
        let num_names = self.names.list(e.nl).names.len();
        let cur = self.nld.ul(nlid);
        if cur
            .as_deref()
            .is_none_or(|c| numify(c) != uniquelist as f64)
        {
            self.changed = true;
        }
        if uniquelist <= 1 {
            return;
        }
        if (num_names as f64) <= e.maxcn {
            return;
        }
        if (uniquelist as f64) <= e.mincn && e.mincn != num_names as f64 {
            return;
        }
        if ulc.final_count(joined) > 1 {
            // Identical complete lists: only disambiguate from lists that
            // differ somewhere.
            match ulc.differs_index(namelist) {
                Some(i) if i > 0 => uniquelist = i + 1,
                _ => return,
            }
        } else if num_names > uniquelist
            && !ulc.differs_nth(namelist, uniquelist, ul, e.labelyear.as_deref())
        {
            uniquelist -= 1;
        }
        self.nld
            .nl(nlid)
            .insert("ul".into(), J::from(uniquelist.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// biber's `namelist_differs_index`, verbatim (quadratic).
    fn naive_index(keys: &[String], list: &[String]) -> Option<usize> {
        let mut index: Option<usize> = None;
        for k in keys {
            let l: Vec<&str> = perl_split(k);
            if l.len() == list.len() && l.iter().zip(list).all(|(a, b)| *a == b) {
                continue;
            }
            for i in 0..list.len() {
                if i < l.len() && l[i] == list[i] {
                    if index.is_none_or(|x| i > x) {
                        index = Some(i);
                    }
                } else {
                    break;
                }
            }
        }
        index.map(|i| if i == list.len() - 1 { i } else { i + 1 })
    }

    /// biber's `namelist_differs_nth`, verbatim.
    fn naive_nth(keys: &[String], list: &[String], n: usize) -> bool {
        for k in keys {
            let l: Vec<&str> = perl_split(k);
            if l.len() < list.len() {
                continue;
            }
            if list[n - 1] == l[n - 1] {
                continue;
            }
            if l[..n - 1].iter().zip(&list[..n - 1]).all(|(a, b)| *a == b) {
                return true;
            }
        }
        false
    }

    #[test]
    fn trie_matches_biber_scans() {
        // Deterministic pseudo-random lists over a tiny alphabet, so that
        // prefixes are shared a lot (including empty names).
        let mut seed: u64 = 0x2545F4914F6CDD1D;
        let mut rnd = |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % m
        };
        let alphabet = ["a", "b", "c", ""];
        for _round in 0..200 {
            let nkeys = 1 + rnd(12) as usize;
            let mut keys: Vec<String> = Vec::new();
            for _ in 0..nkeys {
                let len = rnd(5) as usize;
                let l: Vec<&str> = (0..len).map(|_| alphabet[rnd(4) as usize]).collect();
                let k = join(l.iter().copied());
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
            let mut t = Trie::new();
            for k in &keys {
                t.insert(k);
            }
            for _ in 0..20 {
                let len = 1 + rnd(5) as usize;
                let list: Vec<String> = (0..len)
                    .map(|_| alphabet[rnd(3) as usize].to_string())
                    .collect();
                assert_eq!(
                    t.differs_index(&list),
                    naive_index(&keys, &list),
                    "{keys:?} {list:?}"
                );
                for n in 1..=list.len() {
                    if n == list.len() {
                        assert_eq!(
                            t.differs_nth(&list, n),
                            naive_nth(&keys, &list, n),
                            "{keys:?} {list:?} {n}"
                        );
                    }
                }
            }
        }
    }

    /// The quadratic shape of research 03 §5 (`ul3`, BB-109): pairs of
    /// identical 4-name lists sharing a 3-name prefix, uniquelist=true.
    /// biber 2.22 takes 13.5 s / 33.5 s / 103 s for 1000/2000/4000 entries.
    #[test]
    fn identical_lists_are_not_quadratic() {
        use crate::model::{Name, NamePart};
        let n_entries = 4000;
        let mut arena = NameArena::default();
        let mut entries = Vec::new();
        let mut nld = Map::new();
        for e in 0..n_entries {
            let nl = arena.new_list();
            let fams = [
                "Smith".to_string(),
                "Jones".to_string(),
                "Brown".to_string(),
                format!("X{}", e / 2),
            ];
            let mut lj = Map::new();
            for (i, f) in fams.iter().enumerate() {
                let mut name = Name {
                    index: Some((i + 1).to_string()),
                    ..Name::default()
                };
                name.nameparts.insert(
                    "family",
                    NamePart {
                        string: Some(f.clone()),
                        initial: None,
                    },
                );
                let id = arena.new_name(name);
                arena.list_mut(nl).names.push(id);
                lj.insert(
                    id.key(),
                    J::from_json(serde_json::json!({
                        "basenamestring": f,
                        "basenamestringparts": ["family"],
                        "namedisschema": [["base", ["family"]], ["given", "init"]],
                        "namestring": format!("{f}J"),
                        "namestrings": [f, format!("{f}J")],
                    })),
                );
            }
            nld.insert(nl.key(), J::Object(lj));
            entries.push(EntryInfo {
                nl,
                uniquename: Some("init".into()),
                uniquelist: Some("true".into()),
                maxcn: 3.0,
                mincn: 1.0,
                labelyear: Some("2000".into()),
            });
        }
        let mut state = Map::new();
        state.insert("namelistdata".into(), J::Object(nld));
        let mut dl = DataList {
            state: Some(ListState::from_json(J::Object(state).to_json())),
            ..DataList::default()
        };
        dl.slots.insert("unulchanged", SJ::from("1"));
        let t = std::time::Instant::now();
        run(Quirks::default(), &arena, &entries, &mut dl);
        let elapsed = t.elapsed();
        let st = dl.state.unwrap().to_json();
        // Every list differs from the others at the 4th name.
        for l in st["namelistdata"].as_object().unwrap().values() {
            assert_eq!(l["ul"], "4");
        }
        // biber needs ~100 s here; linear work takes well under a second in
        // release (the bound is generous for debug builds on a busy machine).
        assert!(elapsed.as_secs_f64() < 30.0, "{elapsed:?}");
        eprintln!("uniqueness, {n_entries} entries: {elapsed:?}");
    }
}

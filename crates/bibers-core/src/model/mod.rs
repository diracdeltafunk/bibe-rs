//! The in-memory model the processing pipeline works on.
//!
//! It mirrors biber's own data structures (`Biber::Section`, `Biber::Entry`,
//! `Biber::Entry::Names`/`Name`, `Biber::DataList`) closely, so that pipeline
//! phases can be ported function by function from `Biber.pm` and tested phase
//! by phase against biber's state dumps (`tools/oracle/probes/pipeline/`,
//! [`dump`]).
//!
//! Where biber's data has a fixed shape it is typed here. State whose shape is
//! still being ported is kept as [`PerlData`] (generic JSON-like data,
//! exactly as biber's dump shows it) so that nothing is lost when a state is
//! loaded from a dump and dumped again; phases replace these with typed
//! structures as they are implemented.
//!
//! Name lists live in a per-section arena ([`NameArena`]) and are referenced
//! by [`NameListId`] from entry fields. Biber shares `Names` objects between
//! entries on inheritance and XDATA resolution, and keeps per-list state keyed
//! by the name-list id, so sharing is observable (BB-026); copying a
//! `NameListId` reproduces that sharing explicitly.

pub mod annotation;
pub mod dump;
pub mod entrymap;
pub mod liststate;
pub mod pv;
pub mod sym;
pub mod symmap;

pub use annotation::{Annotation, AnnotationScope, Annotations};
pub use entrymap::EntryMap;
pub use liststate::{Level1, ListState};
pub use pv::{PMap, PV};
pub use sym::{IntoSym, Sym, SymKey};
pub use symmap::SymMap;

use compact_str::CompactString;
use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet};

/// Untyped data in the shape of biber's state dump (scalars are strings,
/// `undef` is `Null`).
pub type PerlData = serde_json::Value;

/// A field value in an entry (`datafields` or `derivedfields`). `undef` is
/// represented by `None` in the field maps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// A Perl scalar: literals, keys, integers, date parts, verbatim and
    /// URI fields, flags (`"1"`), ….
    Str(String),
    /// An array of strings: list fields (location, publisher, keywords,
    /// related, entryset, ids, options, …).
    List(Vec<String>),
    /// A range field: `[[start, end], …]`; `end` is `None` for open ranges
    /// (`100-`) and may be absent in the source structure.
    Ranges(Vec<Vec<Option<String>>>),
    /// A name list.
    Names(NameListId),
    /// Anything else (rare structures not yet typed).
    Other(PerlData),
}

impl Value {
    /// The string value, if this is a scalar.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The list, if this is a list of strings.
    pub fn as_list(&self) -> Option<&[String]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    /// The name list id, if this is a name list.
    pub fn as_names(&self) -> Option<NameListId> {
        match self {
            Value::Names(id) => Some(*id),
            _ => None,
        }
    }
}

/// Field name → value (`None` = Perl `undef`, which differs from absence:
/// biber's `field_exists` is true for an undef field). Sorted, like biber's
/// `sort keys` over field names.
pub type FieldMap = SymMap<Option<Value>>;

/// Slot name → untyped value (the per-object slots biber keeps on its
/// hashes), iterated in name order.
pub type Slots = SymMap<PerlData>;

/// `Biber::Entry`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    /// Fields from the data source (after source mapping and handlers).
    pub datafields: FieldMap,
    /// Fields computed during processing (`citekey`, `entrytype`,
    /// `labelyear`, `sortinit`, …).
    pub derivedfields: FieldMap,
    /// Other per-entry slots biber keeps directly on the entry hash
    /// (`labelnameinfo`, `labelnamefhinfo`, `labeltitleinfo`,
    /// `xdatarefs`, `origfields`, `warnings`, `inherited_fields`, …), as
    /// dumped. Typed accessors can be added as phases are ported.
    pub slots: Slots,
    /// The `labeldateinfo` slot (typed: it is set on every entry and is
    /// large as [`PerlData`]).
    pub labeldateinfo: Option<LabelDateInfo>,
}

/// An entry's `labeldateinfo` (`set_labeldate_info`, Biber.pm:2623): where
/// its label date comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelDateInfo {
    /// `{field => {year => …, month => …, …}}`: the label date fields.
    Field(LabelDateFields),
    /// `{string => …}`: a literal label date.
    String(Option<String>),
}

/// The `field` hash of [`LabelDateInfo::Field`]: field *names* (`undef`
/// is `None`), interned (bounded by the data model, no allocation per
/// entry).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelDateFields {
    pub year: Option<Sym>,
    pub month: Option<Sym>,
    pub day: Option<Sym>,
    pub hour: Option<Sym>,
    pub minute: Option<Sym>,
    pub second: Option<Sym>,
    pub timezone: Option<Sym>,
    /// `"1"` or `undef`.
    pub pseudodate: Option<Sym>,
    pub source: Option<Sym>,
}

impl LabelDateFields {
    /// The hash keys, in dump (sorted) order.
    pub const KEYS: [&'static str; 9] = [
        "day",
        "hour",
        "minute",
        "month",
        "pseudodate",
        "second",
        "source",
        "timezone",
        "year",
    ];

    /// `$info->{field}{$k}` (`None` for `undef` and unknown keys).
    pub fn get(&self, k: &str) -> Option<&str> {
        self.slot(k).and_then(|v| v.map(Sym::as_str))
    }

    fn slot(&self, k: &str) -> Option<&Option<Sym>> {
        Some(match k {
            "year" => &self.year,
            "month" => &self.month,
            "day" => &self.day,
            "hour" => &self.hour,
            "minute" => &self.minute,
            "second" => &self.second,
            "timezone" => &self.timezone,
            "pseudodate" => &self.pseudodate,
            "source" => &self.source,
            _ => return None,
        })
    }

    fn slot_mut(&mut self, k: &str) -> Option<&mut Option<Sym>> {
        Some(match k {
            "year" => &mut self.year,
            "month" => &mut self.month,
            "day" => &mut self.day,
            "hour" => &mut self.hour,
            "minute" => &mut self.minute,
            "second" => &mut self.second,
            "timezone" => &mut self.timezone,
            "pseudodate" => &mut self.pseudodate,
            "source" => &mut self.source,
            _ => return None,
        })
    }
}

impl LabelDateInfo {
    /// The dump's shape.
    pub fn to_json(&self) -> PerlData {
        let s = |v: &Option<String>| v.clone().map_or(PerlData::Null, PerlData::String);
        let mut o = serde_json::Map::new();
        match self {
            LabelDateInfo::Field(f) => {
                let fo = LabelDateFields::KEYS
                    .iter()
                    .map(|k| {
                        let v = f.slot(k).expect("key").map(|v| v.to_string());
                        (k.to_string(), s(&v))
                    })
                    .collect();
                o.insert("field".into(), PerlData::Object(fo));
            }
            LabelDateInfo::String(v) => {
                o.insert("string".into(), s(v));
            }
        }
        PerlData::Object(o)
    }

    /// From the dump's shape; `None` if it has any other shape.
    pub fn from_json(v: &PerlData) -> Option<LabelDateInfo> {
        let o = v.as_object()?;
        if o.len() != 1 {
            return None;
        }
        let opt = |v: &PerlData| match v {
            PerlData::Null => Some(None),
            PerlData::String(s) => Some(Some(s.clone())),
            _ => None,
        };
        if let Some(f) = o.get("field") {
            let fo = f.as_object()?;
            let mut out = LabelDateFields::default();
            if fo.len() != LabelDateFields::KEYS.len() {
                return None;
            }
            for (k, v) in fo {
                *out.slot_mut(k)? = opt(v)?.map(Sym::from);
            }
            Some(LabelDateInfo::Field(out))
        } else {
            Some(LabelDateInfo::String(opt(o.get("string")?)?))
        }
    }
}

impl Entry {
    /// `get_field`: data field first, then derived field (`//`: an undef
    /// data field falls through to the derived one). `f` is a `&str`, a
    /// `&String` or a `&Sym`; [`Entry::get_field_sym`] skips the name lookup.
    #[inline]
    pub fn get_field<K: SymKey + ?Sized>(&self, f: &K) -> Option<&Value> {
        self.get_field_sym(f.to_sym()?)
    }

    /// [`Entry::get_field`] by symbol.
    #[inline]
    pub fn get_field_sym(&self, f: Sym) -> Option<&Value> {
        self.datafields
            .get_sym(f)
            .and_then(Option::as_ref)
            .or_else(|| self.derivedfields.get_sym(f).and_then(Option::as_ref))
    }

    /// `get_field` as a string, if it is a scalar.
    pub fn get_str<K: SymKey + ?Sized>(&self, f: &K) -> Option<&str> {
        self.get_field(f).and_then(Value::as_str)
    }

    /// `get_datafield`.
    pub fn get_datafield<K: SymKey + ?Sized>(&self, f: &K) -> Option<&Value> {
        self.datafields.get(f).and_then(Option::as_ref)
    }

    /// `set_field` (derived).
    pub fn set_field(&mut self, f: impl IntoSym, v: Option<Value>) {
        self.derivedfields.insert(f, v);
    }

    /// `set_datafield`.
    pub fn set_datafield(&mut self, f: impl IntoSym, v: Option<Value>) {
        self.datafields.insert(f, v);
    }

    /// `del_field`: removes from both maps.
    pub fn del_field<K: SymKey + ?Sized>(&mut self, f: &K) {
        if let Some(f) = f.to_sym() {
            self.datafields.remove(&f);
            self.derivedfields.remove(&f);
        }
    }

    /// `field_exists` (true even for undef values).
    pub fn field_exists<K: SymKey + ?Sized>(&self, f: &K) -> bool {
        f.to_sym().is_some_and(|f| {
            self.datafields.contains_key(&f) || self.derivedfields.contains_key(&f)
        })
    }

    /// The entry's citekey (`derivedfields.citekey`).
    pub fn citekey(&self) -> &str {
        self.derivedfields
            .get_sym(sym::CITEKEY)
            .and_then(|v| v.as_ref())
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    /// The entry type (`derivedfields.entrytype`).
    pub fn entrytype(&self) -> &str {
        self.derivedfields
            .get_sym(sym::ENTRYTYPE)
            .and_then(|v| v.as_ref())
            .and_then(Value::as_str)
            .unwrap_or("")
    }
}

/// Index of a name list in a section's [`NameArena`] (biber's `Names` id).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NameListId(pub u32);

/// Index of a name in a section's [`NameArena`] (biber's `Name` id).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NameId(pub u32);

/// One name part (`{string, initial}` in biber). Short strings (most
/// name parts, every initial) are stored inline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamePart {
    pub string: Option<CompactString>,
    pub initial: Option<Vec<CompactString>>,
}

/// `Biber::Entry::Name`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Name {
    pub id: NameId,
    /// Position in its list (biber's `index`, normally 1-based; see BB-024).
    pub index: Option<CompactString>,
    /// Name parts by part name (`family`, `given`, `prefix`, `suffix`, and
    /// any data-model-defined parts).
    pub nameparts: SymMap<NamePart>,
    /// Per-part "outer braces were stripped" flags.
    pub strip: SymMap<Option<CompactString>>,
    /// `rawstring` (joined in Perl hash order upstream: nondeterministic,
    /// only used for the "others" test).
    pub rawstring: Option<String>,
    /// Other slots: per-name biblatex options (`useprefix`, `uniquename`,
    /// …), `gender`, `hashid`, `xdata`, ….
    pub slots: Slots,
}

/// `Biber::Entry::Names`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameList {
    pub id: NameListId,
    /// Name ids, in order.
    pub names: Vec<NameId>,
    /// The field this list was parsed for (`type`).
    pub type_: Option<String>,
    /// Other slots: `morenames`, per-namelist biblatex options, `xdata`, ….
    pub slots: Slots,
}

/// Arena of name lists and names for one section.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameArena {
    pub lists: Vec<NameList>,
    pub names: Vec<Name>,
}

impl NameArena {
    pub fn list(&self, id: NameListId) -> &NameList {
        &self.lists[id.0 as usize]
    }
    pub fn list_mut(&mut self, id: NameListId) -> &mut NameList {
        &mut self.lists[id.0 as usize]
    }
    pub fn name(&self, id: NameId) -> &Name {
        &self.names[id.0 as usize]
    }
    pub fn name_mut(&mut self, id: NameId) -> &mut Name {
        &mut self.names[id.0 as usize]
    }

    /// Allocate a new empty name list.
    pub fn new_list(&mut self) -> NameListId {
        let id = NameListId(self.lists.len() as u32);
        self.lists.push(NameList {
            id,
            ..NameList::default()
        });
        id
    }

    /// Allocate a new name (not yet in any list).
    pub fn new_name(&mut self, mut name: Name) -> NameId {
        let id = NameId(self.names.len() as u32);
        name.id = id;
        self.names.push(name);
        id
    }

    /// The names of a list.
    pub fn names_of(&self, id: NameListId) -> impl Iterator<Item = &Name> {
        self.list(id).names.iter().map(|n| self.name(*n))
    }
}

/// `Biber::Section`: one refsection's entries and bookkeeping.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    /// Section number (`number`).
    pub number: String,
    /// `bibentries`: citekey → entry.
    pub entries: EntryMap,
    /// Name lists of this section's entries.
    pub names: NameArena,
    /// `citekeys`: the ordered list of keys being processed.
    pub citekeys: Vec<String>,
    /// `citekeys_h`: the set of keys in `citekeys` (kept in step by biber's
    /// add/delete methods).
    pub citekeys_h: BTreeSet<String>,
    /// `orig_order_citekeys`.
    pub orig_order_citekeys: Vec<String>,
    /// `undef_citekeys`.
    pub undef_citekeys: Vec<String>,
    /// `citekey_alias`: alias → real key.
    pub citekey_alias: BTreeMap<String, String>,
    /// `allkeys` (`\nocite{*}`).
    pub allkeys: bool,
    /// `allkeysnocite`.
    pub allkeys_nocite: bool,
    /// `cite_citekeys`.
    pub cite_citekeys: BTreeSet<String>,
    /// `nocite_citekeys`.
    pub nocite_citekeys: BTreeSet<String>,
    /// Everything else in `Biber::Section` (`dkeys`, `keytods`,
    /// `keytorelclone`, `relclonetokey`, `relkeys`, `everykey`,
    /// `everykey_lc`, `static_keys`, `namepartlengths`, `labelcache_l`,
    /// `labelcache_v`, `bcfkeycache`, `seenkeys`, `citecount`, `datasources`,
    /// `state`, …), as dumped.
    pub slots: Slots,
}

impl Section {
    /// `bibentry($key)`.
    pub fn bibentry(&self, key: &str) -> Option<&Entry> {
        self.entries.get(key)
    }

    /// `bibentry($key)` (mutable).
    pub fn bibentry_mut(&mut self, key: &str) -> Option<&mut Entry> {
        self.entries.get_mut(key)
    }

    /// `has_citekey`.
    pub fn has_citekey(&self, key: &str) -> bool {
        self.citekeys.iter().any(|k| k == key)
    }
}

/// `Biber::DataList`: one sorted list (refcontext) of a section.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DataList {
    /// Full list name, e.g. `nyt/global//global/global/global`.
    pub name: String,
    /// `entry` or `list`.
    pub type_: String,
    pub section: String,
    pub sortingtemplatename: String,
    pub sortingnamekeytemplatename: String,
    pub uniquenametemplatename: String,
    pub labelalphanametemplatename: String,
    pub namehashtemplatename: String,
    pub labelprefix: String,
    /// The list's keys, in (sorted) order (`None` until `process_lists`
    /// sets them).
    pub keys: Option<Vec<String>>,
    /// `state` (per-list processing state: name hashes, uniqueness data,
    /// extra* counters, …), as dumped; `None` until first set. To be typed as
    /// phases are ported.
    pub state: Option<ListState>,
    /// `sortdata` (`None` while absent): citekey → its sort data.
    pub sortdata: Option<FxHashMap<String, SortData>>,
    /// `sortinitdata` (`None` while absent): citekey → `{init => …}`.
    pub sortinitdata: Option<FxHashMap<String, String>>,
    /// Other slots (`sortdataschema`, `sortingtemplate`, `unulchanged`,
    /// `filters`, …), as dumped.
    pub slots: Slots,
}

/// One key's `sortdata`: `[sort string, [one value per sort set]]`, where
/// the sort string is the values joined with `,` (Biber.pm:4291).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SortData {
    /// One value per sort set.
    pub obj: Box<[CompactString]>,
}

impl SortData {
    /// The sort string (`join(',', @sortobj)`).
    pub fn string(&self) -> String {
        self.obj.join(",")
    }

    /// The dump's shape.
    pub fn to_json(&self) -> PerlData {
        PerlData::Array(vec![
            PerlData::from(self.string()),
            PerlData::Array(self.obj.iter().map(|s| PerlData::from(s.as_str())).collect()),
        ])
    }

    /// From the dump's shape; `None` if it has any other shape (or the
    /// string is not the values joined).
    pub fn from_json(v: &PerlData) -> Option<SortData> {
        let [string, obj] = v.as_array()?.as_slice() else {
            return None;
        };
        let d = SortData {
            obj: (obj.as_array()?.iter())
                .map(|s| s.as_str().map(CompactString::from))
                .collect::<Option<_>>()?,
        };
        (string.as_str()? == d.string()).then_some(d)
    }
}

impl DataList {
    /// The `sortdata` hash in the dump's shape.
    pub fn sortdata_json(&self) -> Option<PerlData> {
        let m = self.sortdata.as_ref()?;
        Some(PerlData::Object(
            m.iter().map(|(k, d)| (k.clone(), d.to_json())).collect(),
        ))
    }

    /// The `sortinitdata` hash in the dump's shape.
    pub fn sortinitdata_json(&self) -> Option<PerlData> {
        let m = self.sortinitdata.as_ref()?;
        Some(PerlData::Object(
            m.iter()
                .map(|(k, i)| {
                    let mut o = serde_json::Map::new();
                    o.insert("init".into(), PerlData::from(i.as_str()));
                    (k.clone(), PerlData::Object(o))
                })
                .collect(),
        ))
    }

    /// Set `sortdata` from the dump's shape (`Err` for another shape).
    pub fn set_sortdata_json(&mut self, v: &PerlData) -> Result<(), String> {
        let o = v.as_object().ok_or("sortdata: not a hash")?;
        let mut m = FxHashMap::default();
        for (k, d) in o {
            let d = SortData::from_json(d).ok_or_else(|| format!("sortdata.{k}: unexpected shape"))?;
            m.insert(k.clone(), d);
        }
        self.sortdata = Some(m);
        Ok(())
    }

    /// Set `sortinitdata` from the dump's shape (`Err` for another shape).
    pub fn set_sortinitdata_json(&mut self, v: &PerlData) -> Result<(), String> {
        let o = v.as_object().ok_or("sortinitdata: not a hash")?;
        let mut m = FxHashMap::default();
        for (k, d) in o {
            let init = (d.as_object())
                .filter(|o| o.len() == 1)
                .and_then(|o| o.get("init"))
                .and_then(PerlData::as_str)
                .ok_or_else(|| format!("sortinitdata.{k}: unexpected shape"))?;
            m.insert(k.clone(), init.to_string());
        }
        self.sortinitdata = Some(m);
        Ok(())
    }
}

/// The per-section processing state the pipeline phases read and write,
/// together with the parts of biber's global config state that are reset per
/// section.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SectionState {
    pub section: Section,
    /// The section's data lists, in `.bcf` order.
    pub datalists: Vec<DataList>,
    /// `$CONFIG->{state}` (crossrefkeys, xrefkeys, uniqignore, xdata,
    /// crossref, keyorder, internalkeyorder, …), as dumped.
    pub config_state: PerlData,
    /// Per-entry biblatex options for this section:
    /// `$CONFIG->{options}{biblatex}{ENTRY}{key}{secnum}`.
    pub entry_options: BTreeMap<String, BTreeMap<String, PerlData>>,
    /// biber's global `$biber->{preamble}` as of this point: every
    /// `@preamble` string read so far, cumulative across sections (`None`
    /// until the first one). The pipeline carries it from section to section.
    pub preamble: Option<Vec<String>>,
    /// `Biber::Annotation` data of this section's entries (a global store
    /// keyed by citekey in biber), in the dump's shape:
    /// `{citekey: {field: {f: {name: {value, literal}}}, item: {f: {name:
    /// {count: {value, literal}}}}, part: {f: {name: {count: {part: {value,
    /// literal}}}}}}}`. `None` when absent from a (older) dump.
    pub annotations: Option<PerlData>,
    /// Groups of `(citekey, datafield)` whose list values are one shared
    /// Perl array (biber copies list fields by reference on inheritance), so
    /// an in-place change to one shows in all: the `.bbl` writer pops a
    /// trailing `others` off the live array (05-B21). Groups of two or more,
    /// sorted. `None` when absent from a (older) dump.
    pub shared_lists: Option<Vec<Vec<(String, String)>>>,
}

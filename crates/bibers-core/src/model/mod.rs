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

pub use annotation::{Annotation, AnnotationScope, Annotations};

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
pub type FieldMap = BTreeMap<String, Option<Value>>;

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
    /// `labeldateinfo`, `xdatarefs`, `origfields`, `warnings`,
    /// `inherited_fields`, …), as dumped. Typed accessors can be added as
    /// phases are ported.
    pub slots: BTreeMap<String, PerlData>,
}

impl Entry {
    /// `get_field`: data field first, then derived field (`//`: an undef
    /// data field falls through to the derived one).
    pub fn get_field(&self, f: &str) -> Option<&Value> {
        self.datafields
            .get(f)
            .and_then(Option::as_ref)
            .or_else(|| self.derivedfields.get(f).and_then(Option::as_ref))
    }

    /// `get_field` as a string, if it is a scalar.
    pub fn get_str(&self, f: &str) -> Option<&str> {
        self.get_field(f).and_then(Value::as_str)
    }

    /// `get_datafield`.
    pub fn get_datafield(&self, f: &str) -> Option<&Value> {
        self.datafields.get(f).and_then(Option::as_ref)
    }

    /// `set_field` (derived).
    pub fn set_field(&mut self, f: impl Into<String>, v: Option<Value>) {
        self.derivedfields.insert(f.into(), v);
    }

    /// `set_datafield`.
    pub fn set_datafield(&mut self, f: impl Into<String>, v: Option<Value>) {
        self.datafields.insert(f.into(), v);
    }

    /// `del_field`: removes from both maps.
    pub fn del_field(&mut self, f: &str) {
        self.datafields.remove(f);
        self.derivedfields.remove(f);
    }

    /// `field_exists` (true even for undef values).
    pub fn field_exists(&self, f: &str) -> bool {
        self.datafields.contains_key(f) || self.derivedfields.contains_key(f)
    }

    /// The entry's citekey (`derivedfields.citekey`).
    pub fn citekey(&self) -> &str {
        self.derivedfields
            .get("citekey")
            .and_then(|v| v.as_ref())
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    /// The entry type (`derivedfields.entrytype`).
    pub fn entrytype(&self) -> &str {
        self.derivedfields
            .get("entrytype")
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

/// One name part (`{string, initial}` in biber).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamePart {
    pub string: Option<String>,
    pub initial: Option<Vec<String>>,
}

/// `Biber::Entry::Name`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Name {
    pub id: NameId,
    /// Position in its list (biber's `index`, normally 1-based; see BB-024).
    pub index: Option<String>,
    /// Name parts by part name (`family`, `given`, `prefix`, `suffix`, and
    /// any data-model-defined parts).
    pub nameparts: BTreeMap<String, NamePart>,
    /// Per-part "outer braces were stripped" flags.
    pub strip: BTreeMap<String, Option<String>>,
    /// `rawstring` (joined in Perl hash order upstream: nondeterministic,
    /// only used for the "others" test).
    pub rawstring: Option<String>,
    /// Other slots: per-name biblatex options (`useprefix`, `uniquename`,
    /// …), `gender`, `hashid`, `xdata`, ….
    pub slots: BTreeMap<String, PerlData>,
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
    pub slots: BTreeMap<String, PerlData>,
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
    pub entries: BTreeMap<String, Entry>,
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
    pub slots: BTreeMap<String, PerlData>,
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
    pub state: Option<PerlData>,
    /// Other slots (`sortdata`, `sortinitdata`, `sortdataschema`,
    /// `sortingtemplate`, `unulchanged`, `filters`, …), as dumped.
    pub slots: BTreeMap<String, PerlData>,
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

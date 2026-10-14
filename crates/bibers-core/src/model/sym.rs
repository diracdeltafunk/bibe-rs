//! Interned names ([`Sym`]): field names, slot names, name-part names.
//!
//! The model's small maps ([`super::SymMap`]: entry fields and slots, name
//! parts, …) are keyed by `Sym` instead of `String`. A `Sym` is a pointer to
//! a unique, never-freed record of its name, so
//!
//! - equality is a pointer comparison (no `memcmp`),
//! - [`Sym::as_str`] is a field read (no lock, no table),
//! - ordering is the byte order of the names, exactly like `String`'s, so maps
//!   keyed by `Sym` iterate in the same order as the `BTreeMap<String, _>`
//!   they replace (biber's `sort keys`), and
//! - hashing uses a hash of the name computed once at interning time, so it
//!   is deterministic across runs.
//!
//! Looking up a `&str` costs one hash-table probe. The names biber and
//! biblatex use (the default data model, derived fields, slots) are interned
//! up front into a table that is read without locking; other names (custom
//! data model fields, …) go to a second table behind an `RwLock`. The set of
//! interned names is bounded by the data model and the code, never by the
//! data: only map *keys* are interned, and looking up a name that was never
//! interned does not intern it.

use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, RwLock};

/// The interned record behind a [`Sym`].
#[derive(Debug)]
pub struct SymData {
    name: &'static str,
    hash: u64,
}

impl SymData {
    const fn new(name: &'static str) -> SymData {
        SymData {
            name,
            hash: fnv1a(name),
        }
    }
}

/// FNV-1a over the bytes of `s` (a `const fn`, so the well-known symbols'
/// hashes are computed at compile time).
const fn fnv1a(s: &str) -> u64 {
    let b = s.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < b.len() {
        h ^= b[i] as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    h
}

/// An interned name. `Copy`, pointer-sized; see the [module docs](self).
#[derive(Clone, Copy)]
pub struct Sym(&'static SymData);

impl Sym {
    /// Intern `s` (find its symbol, creating it if needed).
    pub fn new(s: &str) -> Sym {
        if let Some(sym) = Sym::lookup(s) {
            return sym;
        }
        let mut dynamic = DYNAMIC.write().unwrap_or_else(|e| e.into_inner());
        if let Some(sym) = dynamic.get(s) {
            return *sym;
        }
        let name: &'static str = Box::leak(s.into());
        let sym = Sym(Box::leak(Box::new(SymData::new(name))));
        dynamic.insert(name, sym);
        sym
    }

    /// The symbol of `s` if it has been interned. A name that was never
    /// interned cannot be a key of any [`super::SymMap`].
    pub fn lookup(s: &str) -> Option<Sym> {
        if let Some(sym) = WELL_KNOWN.get(s) {
            return Some(*sym);
        }
        DYNAMIC
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(s)
            .copied()
    }

    /// The name.
    pub fn as_str(self) -> &'static str {
        self.0.name
    }
}

impl PartialEq for Sym {
    fn eq(&self, other: &Sym) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for Sym {}

impl Ord for Sym {
    /// The byte order of the names (`String`'s order).
    fn cmp(&self, other: &Sym) -> Ordering {
        if self == other {
            Ordering::Equal
        } else {
            self.0.name.cmp(other.0.name)
        }
    }
}

impl PartialOrd for Sym {
    fn partial_cmp(&self, other: &Sym) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Sym {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.0.hash);
    }
}

impl std::ops::Deref for Sym {
    type Target = str;
    fn deref(&self) -> &str {
        self.0.name
    }
}

impl fmt::Display for Sym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.name)
    }
}

impl fmt::Debug for Sym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0.name, f)
    }
}

impl PartialEq<str> for Sym {
    fn eq(&self, other: &str) -> bool {
        self.0.name == other
    }
}

impl PartialEq<&str> for Sym {
    fn eq(&self, other: &&str) -> bool {
        self.0.name == *other
    }
}

impl PartialEq<String> for Sym {
    fn eq(&self, other: &String) -> bool {
        self.0.name == other
    }
}

impl PartialEq<Sym> for str {
    fn eq(&self, other: &Sym) -> bool {
        self == other.0.name
    }
}

impl PartialEq<Sym> for &str {
    fn eq(&self, other: &Sym) -> bool {
        *self == other.0.name
    }
}

impl PartialEq<Sym> for String {
    fn eq(&self, other: &Sym) -> bool {
        self == other.0.name
    }
}

impl From<&str> for Sym {
    fn from(s: &str) -> Sym {
        Sym::new(s)
    }
}

impl From<String> for Sym {
    fn from(s: String) -> Sym {
        Sym::new(&s)
    }
}

impl From<&String> for Sym {
    fn from(s: &String) -> Sym {
        Sym::new(s)
    }
}

impl From<Sym> for String {
    fn from(s: Sym) -> String {
        s.as_str().to_string()
    }
}

/// A name a [`super::SymMap`] can be queried with (`str`, `String`, `Sym`,
/// and references to them), mirroring `BTreeMap::get`'s `&Q` argument.
/// Querying never interns.
pub trait SymKey {
    /// The symbol, if the name has been interned.
    fn to_sym(&self) -> Option<Sym>;
}

impl SymKey for Sym {
    fn to_sym(&self) -> Option<Sym> {
        Some(*self)
    }
}

impl SymKey for str {
    fn to_sym(&self) -> Option<Sym> {
        Sym::lookup(self)
    }
}

impl SymKey for String {
    fn to_sym(&self) -> Option<Sym> {
        Sym::lookup(self)
    }
}

impl SymKey for Box<str> {
    fn to_sym(&self) -> Option<Sym> {
        Sym::lookup(self)
    }
}

impl SymKey for Cow<'_, str> {
    fn to_sym(&self) -> Option<Sym> {
        Sym::lookup(self)
    }
}

impl<T: SymKey + ?Sized> SymKey for &T {
    fn to_sym(&self) -> Option<Sym> {
        (**self).to_sym()
    }
}

/// A name a [`super::SymMap`] key can be made from (interning it), mirroring
/// the `impl Into<String>` arguments of the setters.
pub trait IntoSym {
    fn into_sym(self) -> Sym;
}

impl IntoSym for Sym {
    fn into_sym(self) -> Sym {
        self
    }
}

impl IntoSym for &Sym {
    fn into_sym(self) -> Sym {
        *self
    }
}

impl IntoSym for &str {
    fn into_sym(self) -> Sym {
        Sym::new(self)
    }
}

impl IntoSym for &&str {
    fn into_sym(self) -> Sym {
        Sym::new(self)
    }
}

impl IntoSym for String {
    fn into_sym(self) -> Sym {
        Sym::new(&self)
    }
}

impl IntoSym for &String {
    fn into_sym(self) -> Sym {
        Sym::new(self)
    }
}

impl IntoSym for Box<str> {
    fn into_sym(self) -> Sym {
        Sym::new(&self)
    }
}

impl IntoSym for Cow<'_, str> {
    fn into_sym(self) -> Sym {
        Sym::new(&self)
    }
}

/// Names interned at start-up, without a constant of their own: biblatex's
/// default data model (fields and entry types) and its constants.
const PRESEEDED: &[&str] = &[
    // Fields of biblatex's default data model.
    "abstract",
    "addendum",
    "afterword",
    "annotation",
    "annotator",
    "author",
    "authortype",
    "bookauthor",
    "bookpagination",
    "booksubtitle",
    "booktitle",
    "booktitleaddon",
    "chapter",
    "commentator",
    "crossref",
    "date",
    "day",
    "doi",
    "edition",
    "editor",
    "editora",
    "editoratype",
    "editorb",
    "editorbtype",
    "editorc",
    "editorctype",
    "editortype",
    "eid",
    "endday",
    "endhour",
    "endminute",
    "endmonth",
    "endsecond",
    "endtimezone",
    "endyear",
    "endyeardivision",
    "entryset",
    "entrysubtype",
    "eprint",
    "eprintclass",
    "eprinttype",
    "eventdate",
    "eventday",
    "eventendday",
    "eventendhour",
    "eventendminute",
    "eventendmonth",
    "eventendsecond",
    "eventendtimezone",
    "eventendyear",
    "eventendyeardivision",
    "eventhour",
    "eventminute",
    "eventmonth",
    "eventsecond",
    "eventtimezone",
    "eventtitle",
    "eventtitleaddon",
    "eventyear",
    "eventyeardivision",
    "execute",
    "file",
    "foreword",
    "gender",
    "holder",
    "hour",
    "howpublished",
    "ids",
    "indexsorttitle",
    "indextitle",
    "institution",
    "introduction",
    "isan",
    "isbn",
    "ismn",
    "isrn",
    "issn",
    "issue",
    "issuesubtitle",
    "issuetitle",
    "issuetitleaddon",
    "iswc",
    "journalsubtitle",
    "journaltitle",
    "journaltitleaddon",
    "keywords",
    "label",
    "langid",
    "langidopts",
    "language",
    "library",
    "lista",
    "listb",
    "listc",
    "listd",
    "liste",
    "listf",
    "location",
    "mainsubtitle",
    "maintitle",
    "maintitleaddon",
    "minute",
    "month",
    "namea",
    "nameaddon",
    "nameatype",
    "nameb",
    "namebtype",
    "namec",
    "namectype",
    "note",
    "number",
    "options",
    "organization",
    "origdate",
    "origday",
    "origendday",
    "origendhour",
    "origendminute",
    "origendmonth",
    "origendsecond",
    "origendtimezone",
    "origendyear",
    "origendyeardivision",
    "orighour",
    "origlanguage",
    "origlocation",
    "origminute",
    "origmonth",
    "origpublisher",
    "origsecond",
    "origtimezone",
    "origtitle",
    "origyear",
    "origyeardivision",
    "pages",
    "pagetotal",
    "pagination",
    "part",
    "presort",
    "publisher",
    "pubstate",
    "related",
    "relatedoptions",
    "relatedstring",
    "relatedtype",
    "reprinttitle",
    "second",
    "series",
    "shortauthor",
    "shorteditor",
    "shorthand",
    "shorthandintro",
    "shortjournal",
    "shortseries",
    "shorttitle",
    "sortkey",
    "sortname",
    "sortshorthand",
    "sorttitle",
    "sortyear",
    "subtitle",
    "timezone",
    "title",
    "titleaddon",
    "translator",
    "type",
    "url",
    "urldate",
    "urlday",
    "urlendday",
    "urlendhour",
    "urlendminute",
    "urlendmonth",
    "urlendsecond",
    "urlendtimezone",
    "urlendyear",
    "urlendyeardivision",
    "urlhour",
    "urlminute",
    "urlmonth",
    "urlsecond",
    "urltimezone",
    "urlyear",
    "urlyeardivision",
    "usera",
    "userb",
    "userc",
    "userd",
    "usere",
    "userf",
    "venue",
    "verba",
    "verbb",
    "verbc",
    "version",
    "volume",
    "volumes",
    "xdata",
    "xref",
    "year",
    "yeardivision",
    // Date metadata fields set by the date handlers.
    "dateera",
    "dateunspecified",
    "datejulian",
    "dateapproximate",
    "dateuncertain",
    "enddateera",
    "enddatejulian",
    "enddateapproximate",
    "enddateuncertain",
    "origdateera",
    "eventdateera",
    "urldateera",
    // Derived fields.
    "labelalpha",
    "extraalpha",
    "sortinit",
    "sortinithash",
    "labelname",
    "labeltitle",
    "labelmonth",
    "labelday",
    "labelhour",
    "labelminute",
    "labelsecond",
    "labeltimezone",
    "labelendyear",
    "labelendmonth",
    "labelendday",
    "era",
    "clonesourcekey",
    "crossrefsource",
    "xrefsource",
    "labelprefix",
    "extradate",
    "extraname",
    "extratitle",
    "extratitleyear",
    "singletitle",
    "uniquetitle",
    "uniquebaretitle",
    "uniquework",
    "uniqueprimaryauthor",
    "namehash",
    "fullhash",
    "fullhashraw",
    "bibnamehash",
    // Name parts and name slots.
    "family",
    "given",
    "prefix",
    "suffix",
    "hash",
    "uniquename",
    "uniquelist",
    "useprefix",
    "morenames",
];

macro_rules! well_known {
    ($($id:ident = $s:literal),* $(,)?) => {
        $(
            #[doc = concat!("The interned name `", $s, "`.")]
            pub static $id: Sym = Sym(&SymData::new($s));
        )*
        const NAMED: &[&Sym] = &[$(&$id),*];
    };
}

// Names the code uses as constants (`sym::CITEKEY`). Each is interned at
// start-up: `Sym::new("citekey") == sym::CITEKEY`.
well_known! {
    CITEKEY = "citekey",
    ENTRYTYPE = "entrytype",
    DATATYPE = "datatype",
    DATESPLIT = "datesplit",
    RAWDATA = "rawdata",
    LABELYEAR = "labelyear",
    LABELDATESOURCE = "labeldatesource",
    EXTRADATESCOPE = "extradatescope",
    LABELNAMEINFO = "labelnameinfo",
    LABELNAMEFHINFO = "labelnamefhinfo",
    LABELTITLEINFO = "labeltitleinfo",
    LABELDATEINFO = "labeldateinfo",
    XDATAREFS = "xdatarefs",
    ORIGFIELDS = "origfields",
    WARNINGS = "warnings",
    INHERITED_FIELDS = "inherited_fields",
}

/// The start-up table, read without locking.
static WELL_KNOWN: LazyLock<FxHashMap<&'static str, Sym>> = LazyLock::new(|| {
    let mut m = FxHashMap::default();
    for s in NAMED {
        m.insert(s.as_str(), **s);
    }
    for name in PRESEEDED {
        m.entry(*name)
            .or_insert_with(|| Sym(Box::leak(Box::new(SymData::new(name)))));
    }
    m
});

/// Names interned later.
static DYNAMIC: LazyLock<RwLock<FxHashMap<&'static str, Sym>>> =
    LazyLock::new(|| RwLock::new(FxHashMap::default()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_is_unique() {
        assert_eq!(Sym::new("citekey"), CITEKEY);
        assert_eq!(Sym::new("author"), Sym::new("author"));
        let a = Sym::new("a-name-only-this-test-uses");
        assert_eq!(Sym::lookup("a-name-only-this-test-uses"), Some(a));
        assert_eq!(Sym::lookup("a-name-nobody-interned"), None);
        assert_ne!(Sym::new("title"), Sym::new("titleaddon"));
    }

    #[test]
    fn order_is_byte_order() {
        let mut v: Vec<Sym> = ["year", "Zeta", "author", "", "authortype", "é", "aa"]
            .into_iter()
            .map(Sym::new)
            .collect();
        v.sort();
        let names: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            ["", "Zeta", "aa", "author", "authortype", "year", "é"]
        );
    }

    #[test]
    fn hash_is_deterministic() {
        use std::hash::BuildHasher;
        let h = rustc_hash::FxBuildHasher;
        assert_eq!(h.hash_one(Sym::new("title")), h.hash_one(Sym::new("title")));
        assert_eq!(Sym::new("title").0.hash, fnv1a("title"));
    }
}

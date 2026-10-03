//! Biber options (`$CONFIG->{options}{biber}`): defaults
//! (`$CONFIG_DEFAULT_BIBER`, Constants.pm:236-311), provenance and typed
//! accessors.
//!
//! biber's option store is a dynamic hash: the command line, biber.conf and
//! the `.bcf` can each set any key, and values keep Perl's string semantics
//! until they are used. We keep that model (a map from name to
//! [`BiberValue`] with provenance) so later stages see exactly biber's
//! values, and add typed accessors that apply Perl's conversions at the use
//! site (truthiness, numification).

use std::collections::BTreeMap;

use crate::bcf::sourcemap::SourceMaps;
use crate::perlish::{numify, truthy};

/// Where an option value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// `$CONFIG_DEFAULT_BIBER`.
    Default,
    /// biber-tool.conf or the user's biber.conf (`setconfigfileoption`).
    ConfigFile,
    /// The command line (`setcmdlineoption`).
    CommandLine,
    /// The `.bcf` or an internal adjustment (`setoption`).
    Set,
}

/// A regex option entry: `{value => re}` (noinit, nolabel,
/// nolabelwidthcount) or `{name => field, value => re}` (nosort,
/// nonamestring).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternOpt {
    /// `name`: a field or datafield set (`None` for value-only options or
    /// an absent attribute).
    pub name: Option<String>,
    /// `value`: the (Perl) regular expression.
    pub value: Option<String>,
    /// Whether the entry has a `name` key at all.
    pub has_name: bool,
}

/// A biber option value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BiberValue {
    /// `undef`.
    Undef,
    /// A scalar (Perl keeps numbers and booleans as strings here).
    Str(String),
    /// A multivalued `.bcf` option: contents sorted by `order`.
    List(Vec<Option<String>>),
    /// A hash of strings (`collate_options`, `dot_include`).
    Map(BTreeMap<String, String>),
    /// A list of regex entries (noinit, nolabel, nolabelwidthcount, nosort,
    /// nonamestring).
    Patterns(Vec<PatternOpt>),
    /// Source maps (`sourcemap`).
    Sourcemap(Vec<SourceMaps>),
}

impl BiberValue {
    /// The scalar string, if this is a defined scalar.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            BiberValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Perl truthiness (references are true).
    pub fn truthy(&self) -> bool {
        match self {
            BiberValue::Undef => false,
            BiberValue::Str(s) => truthy(Some(s)),
            _ => true,
        }
    }

    /// Is this `undef`?
    pub fn is_undef(&self) -> bool {
        matches!(self, BiberValue::Undef)
    }
}

impl From<&str> for BiberValue {
    fn from(s: &str) -> Self {
        BiberValue::Str(s.to_string())
    }
}

impl From<String> for BiberValue {
    fn from(s: String) -> Self {
        BiberValue::Str(s)
    }
}

impl From<Option<String>> for BiberValue {
    fn from(s: Option<String>) -> Self {
        s.map_or(BiberValue::Undef, BiberValue::Str)
    }
}

/// One option slot.
#[derive(Clone, Debug)]
pub struct Slot {
    /// The current value.
    pub value: BiberValue,
    /// Who set the current value last.
    pub provenance: Provenance,
    /// `defined $CONFIG->{cmdlineoptions}{$opt}`.
    pub cmdline: bool,
    /// `defined $CONFIG->{configfileoptions}{$opt}`.
    pub configfile: bool,
}

/// The biber option store.
#[derive(Clone, Debug, Default)]
pub struct BiberOptions {
    map: BTreeMap<String, Slot>,
}

/// The default value of every biber option (Constants.pm:236-311).
pub fn default_options() -> Vec<(&'static str, BiberValue)> {
    let s = |v: &str| BiberValue::Str(v.to_string());
    let map = |kv: &[(&str, &str)]| {
        BiberValue::Map(
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    };
    let pats = |v: &[&str]| {
        BiberValue::Patterns(
            v.iter()
                .map(|r| PatternOpt {
                    name: None,
                    value: Some(r.to_string()),
                    has_name: false,
                })
                .collect(),
        )
    };
    vec![
        ("annotation_marker", s("+an")),
        ("clrmacros", s("0")),
        (
            "collate_options",
            map(&[
                ("level", "4"),
                ("variable", "non-ignorable"),
                ("normalization", "prenormalized"),
            ]),
        ),
        ("graph", s("0")),
        ("debug", s("0")),
        ("dieondatamodel", s("0")),
        ("decodecharsset", s("base")),
        (
            "dot_include",
            map(&[
                ("section", "1"),
                ("xdata", "1"),
                ("crossref", "1"),
                ("xref", "1"),
            ]),
        ),
        ("fixinits", s("0")),
        ("glob_datasources", s("0")),
        ("input_encoding", s("UTF-8")),
        ("input_format", s("bibtex")),
        ("isbn10", s("0")),
        ("isbn13", s("0")),
        ("isbn_normalise", s("0")),
        ("listsep", s("and")),
        ("mincrossrefs", s("2")),
        ("minxrefs", s("2")),
        ("named_annotation_marker", s(":")),
        ("namesep", s("and")),
        ("no_bblxml_schema", s("0")),
        ("no_bltxml_schema", s("0")),
        ("no_default_datamodel", s("0")),
        ("nodieonerror", s("0")),
        (
            "noinit",
            pats(&[r"\b\p{Ll}{2}\p{Pd}(?=\S)", r"[\x{2bf}\x{2018}]"]),
        ),
        (
            "nolabel",
            pats(&[r"[\p{Pc}\p{Ps}\p{Pe}\p{Pi}\p{Pf}\p{Po}\p{S}\p{C}]+"]),
        ),
        ("nolog", s("0")),
        ("noskipduplicates", s("0")),
        ("nostdmacros", s("0")),
        (
            "nosort",
            BiberValue::Patterns(
                [r"\A\p{L}{2}\p{Pd}(?=\S)", r"[\x{2bf}\x{2018}]"]
                    .iter()
                    .map(|r| PatternOpt {
                        name: Some("setnames".to_string()),
                        value: Some(r.to_string()),
                        has_name: true,
                    })
                    .collect(),
            ),
        ),
        ("onlylog", s("0")),
        ("others_string", s("others")),
        ("output_align", s("0")),
        ("output_all_macrodefs", s("0")),
        ("output_annotation_marker", s("+an")),
        ("output_named_annotation_marker", s(":")),
        ("output_encoding", s("UTF-8")),
        (
            "output_field_order",
            s("options,abstract,names,lists,dates"),
        ),
        ("output_format", s("bbl")),
        ("output_indent", s("2")),
        ("output_fieldcase", s("upper")),
        ("output_legacy_dates", s("0")),
        ("output_listsep", s("and")),
        ("output_namesep", s("and")),
        ("output_no_macrodefs", s("0")),
        ("output_resolve_xdata", s("0")),
        ("output_resolve_crossrefs", s("0")),
        ("output_resolve_sets", s("0")),
        ("output_safechars", s("0")),
        ("output_safecharsset", s("base")),
        ("output_xdatamarker", s("xdata")),
        ("output_xdatasep", s("-")),
        ("output_xnamesep", s("=")),
        ("quiet", s("0")),
        ("remove_tmp_dir", s("1")),
        ("sortdebug", s("0")),
        ("sortcase", s("1")),
        ("sortupper", s("1")),
        ("strip_comments", s("0")),
        ("tool", s("0")),
        ("tool_noremove_missing_dependants", s("0")),
        ("trace", s("0")),
        ("nouri_encode", s("0")),
        ("validate_bblxml", s("0")),
        ("validate_bltxml", s("0")),
        ("validate_config", s("0")),
        ("validate_control", s("0")),
        ("validate_datamodel", s("0")),
        ("winunicode", s("0")),
        ("wraplines", s("0")),
        ("xdatamarker", s("xdata")),
        ("xdatasep", s("-")),
        ("xnamesep", s("=")),
        ("xsvsep", s(r"\s*,\s*")),
    ]
}

impl BiberOptions {
    /// An empty store (no defaults).
    pub fn empty() -> Self {
        BiberOptions::default()
    }

    /// A store holding the hard-coded defaults.
    pub fn with_defaults() -> Self {
        let mut o = BiberOptions::empty();
        for (k, v) in default_options() {
            o.map.insert(
                k.to_string(),
                Slot {
                    value: v,
                    provenance: Provenance::Default,
                    cmdline: false,
                    configfile: false,
                },
            );
        }
        o
    }

    fn slot_mut(&mut self, opt: &str) -> &mut Slot {
        self.map.entry(opt.to_string()).or_insert(Slot {
            value: BiberValue::Undef,
            provenance: Provenance::Set,
            cmdline: false,
            configfile: false,
        })
    }

    /// `setoption`.
    pub fn set(&mut self, opt: &str, val: impl Into<BiberValue>) {
        let s = self.slot_mut(opt);
        s.value = val.into();
        s.provenance = Provenance::Set;
    }

    /// `setcmdlineoption`.
    pub fn set_cmdline(&mut self, opt: &str, val: impl Into<BiberValue>) {
        let s = self.slot_mut(opt);
        s.value = val.into();
        s.cmdline = !s.value.is_undef();
        s.provenance = Provenance::CommandLine;
    }

    /// `setconfigfileoption` (the biblatex half is handled by the caller).
    pub fn set_configfile(&mut self, opt: &str, val: impl Into<BiberValue>) {
        let s = self.slot_mut(opt);
        s.value = val.into();
        s.configfile = !s.value.is_undef();
        s.provenance = Provenance::ConfigFile;
    }

    /// `getoption`: the value, or `None` if never set.
    pub fn get(&self, opt: &str) -> Option<&BiberValue> {
        self.map.get(opt).map(|s| &s.value)
    }

    /// The slot (value and provenance).
    pub fn slot(&self, opt: &str) -> Option<&Slot> {
        self.map.get(opt)
    }

    /// `getoption` as a string (`None` for undef or a reference).
    pub fn get_str(&self, opt: &str) -> Option<&str> {
        self.get(opt).and_then(BiberValue::as_str)
    }

    /// Perl truthiness of `getoption`.
    pub fn is_true(&self, opt: &str) -> bool {
        self.get(opt).is_some_and(BiberValue::truthy)
    }

    /// Numeric value of `getoption` (Perl numification; undef is 0).
    pub fn num(&self, opt: &str) -> f64 {
        self.get_str(opt).map_or(0.0, numify)
    }

    /// `isexplicitoption`: set on the command line or in a config file.
    pub fn is_explicit(&self, opt: &str) -> bool {
        self.map.get(opt).is_some_and(|s| s.cmdline || s.configfile)
    }

    /// `iscmdlineoption`.
    pub fn is_cmdline(&self, opt: &str) -> bool {
        self.map.get(opt).is_some_and(|s| s.cmdline)
    }

    /// All options, sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Slot)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// `delete` an option (used for internal bookkeeping only).
    pub fn remove(&mut self, opt: &str) {
        self.map.remove(opt);
    }

    // ---- typed accessors for the options later stages use ----

    /// `output_format`.
    pub fn output_format(&self) -> &str {
        self.get_str("output_format").unwrap_or("bbl")
    }
    /// `input_encoding`.
    pub fn input_encoding(&self) -> Option<&str> {
        self.get_str("input_encoding")
    }
    /// `output_encoding`.
    pub fn output_encoding(&self) -> Option<&str> {
        self.get_str("output_encoding")
    }
    /// `mincrossrefs` (numified).
    pub fn mincrossrefs(&self) -> f64 {
        self.num("mincrossrefs")
    }
    /// `minxrefs` (numified).
    pub fn minxrefs(&self) -> f64 {
        self.num("minxrefs")
    }
    /// `sortlocale` (biber option; overrides template locales).
    pub fn sortlocale(&self) -> Option<&str> {
        self.get_str("sortlocale")
    }
    /// `tool`.
    pub fn tool(&self) -> bool {
        self.is_true("tool")
    }
    /// `debug`.
    pub fn debug(&self) -> bool {
        self.is_true("debug")
    }
    /// `trace`.
    pub fn trace(&self) -> bool {
        self.is_true("trace")
    }
    /// `nodieonerror`.
    pub fn nodieonerror(&self) -> bool {
        self.is_true("nodieonerror")
    }
    /// `validate_datamodel`.
    pub fn validate_datamodel(&self) -> bool {
        self.is_true("validate_datamodel")
    }
    /// `dieondatamodel`.
    pub fn dieondatamodel(&self) -> bool {
        self.is_true("dieondatamodel")
    }
    /// `annotation_marker`.
    pub fn annotation_marker(&self) -> &str {
        self.get_str("annotation_marker").unwrap_or("")
    }
    /// `named_annotation_marker`.
    pub fn named_annotation_marker(&self) -> &str {
        self.get_str("named_annotation_marker").unwrap_or("")
    }
    /// `collate_options` as a map.
    pub fn collate_options(&self) -> Option<&BTreeMap<String, String>> {
        match self.get("collate_options") {
            Some(BiberValue::Map(m)) => Some(m),
            _ => None,
        }
    }
    /// A regex-list option (noinit, nolabel, nolabelwidthcount, nosort,
    /// nonamestring).
    pub fn patterns(&self, opt: &str) -> &[PatternOpt] {
        match self.get(opt) {
            Some(BiberValue::Patterns(p)) => p,
            _ => &[],
        }
    }
    /// `sourcemap`.
    pub fn sourcemap(&self) -> &[SourceMaps] {
        match self.get("sourcemap") {
            Some(BiberValue::Sourcemap(s)) => s,
            _ => &[],
        }
    }
}

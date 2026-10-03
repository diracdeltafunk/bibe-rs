//! Typed templates and declarations from the `.bcf` (and biber.conf), in the
//! normalised form biber builds in `parse_ctrlfile` (Biber.pm:643-850,
//! 4995-5045) and `_config_file_set` (Config.pm:450-583).
//!
//! Values are kept as the raw attribute strings (biber never converts them
//! at parse time); `None` stands for Perl's `undef`.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::config::locale::locale2bcp47;
use crate::perlish::sort_by_order;
use crate::xmlin::{XHash, XVal};

fn opt(v: &Option<String>) -> Value {
    v.clone().map_or(Value::Null, Value::String)
}

fn s(h: &XHash, k: &str) -> Option<String> {
    h.str(k).map(str::to_string)
}

/// Sorted copies of the hashes under `key`, by their `order` attribute.
fn sorted_by_order<'a>(h: &'a XHash, key: &str) -> Vec<&'a XHash> {
    let mut v: Vec<&XHash> = h.list(key).hashes().collect();
    sort_by_order(&mut v, |x| x.str("order"));
    v
}

/// A part of a `\DeclareLabelalphaNameTemplate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelAlphaNamePart {
    /// The namepart (element content).
    pub namepart: Option<String>,
    /// `use`.
    pub use_: Option<String>,
    /// `pre`.
    pub pre: Option<String>,
    /// `substring_compound`.
    pub substring_compound: Option<String>,
    /// `substring_side`.
    pub substring_side: Option<String>,
    /// `substring_width`.
    pub substring_width: Option<String>,
}

impl LabelAlphaNamePart {
    fn from_xval(np: &XHash) -> Self {
        LabelAlphaNamePart {
            namepart: np.content().map(str::to_string),
            use_: s(np, "use"),
            pre: s(np, "pre"),
            substring_compound: s(np, "substring_compound"),
            substring_side: s(np, "substring_side"),
            substring_width: s(np, "substring_width"),
        }
    }

    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("namepart".into(), opt(&self.namepart));
        m.insert("use".into(), opt(&self.use_));
        m.insert("pre".into(), opt(&self.pre));
        m.insert("substring_compound".into(), opt(&self.substring_compound));
        m.insert("substring_side".into(), opt(&self.substring_side));
        m.insert("substring_width".into(), opt(&self.substring_width));
        Value::Object(m)
    }
}

/// Parse one `<labelalphanametemplate>` (name, parts sorted by order).
pub fn labelalphaname_template(t: &XHash) -> (String, Vec<LabelAlphaNamePart>) {
    let parts = sorted_by_order(t, "namepart")
        .into_iter()
        .map(LabelAlphaNamePart::from_xval)
        .collect();
    (t.str("name").unwrap_or("").to_string(), parts)
}

/// A part of a `\DeclareUniquenameTemplate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UniquenamePart {
    /// The namepart.
    pub namepart: Option<String>,
    /// `use`.
    pub use_: Option<String>,
    /// `disambiguation`.
    pub disambiguation: Option<String>,
    /// `base`.
    pub base: Option<String>,
}

/// Parse one `<uniquenametemplate>`; also reports whether any part has a
/// (Perl-true) `base` attribute.
pub fn uniquename_template(t: &XHash) -> (String, Vec<UniquenamePart>, bool) {
    let mut has_base = false;
    let parts = sorted_by_order(t, "namepart")
        .into_iter()
        .map(|np| {
            if crate::perlish::truthy(np.str("base")) {
                has_base = true;
            }
            UniquenamePart {
                namepart: np.content().map(str::to_string),
                use_: s(np, "use"),
                disambiguation: s(np, "disambiguation"),
                base: s(np, "base"),
            }
        })
        .collect();
    (t.str("name").unwrap_or("").to_string(), parts, has_base)
}

impl UniquenamePart {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("namepart".into(), opt(&self.namepart));
        m.insert("use".into(), opt(&self.use_));
        m.insert("disambiguation".into(), opt(&self.disambiguation));
        m.insert("base".into(), opt(&self.base));
        Value::Object(m)
    }
}

/// A part of a `\DeclareNamehashTemplate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamehashPart {
    /// The namepart.
    pub namepart: Option<String>,
    /// `hashscope` (init, full).
    pub hashscope: Option<String>,
}

/// Parse one `<namehashtemplate>`.
pub fn namehash_template(t: &XHash) -> (String, Vec<NamehashPart>) {
    let parts = sorted_by_order(t, "namepart")
        .into_iter()
        .map(|np| NamehashPart {
            namepart: np.content().map(str::to_string),
            hashscope: s(np, "hashscope"),
        })
        .collect();
    (t.str("name").unwrap_or("").to_string(), parts)
}

impl NamehashPart {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("namepart".into(), opt(&self.namepart));
        m.insert("hashscope".into(), opt(&self.hashscope));
        Value::Object(m)
    }
}

/// A part of a sorting name key template key part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnkPart {
    /// A namepart: `{type => 'namepart', value, use?, inits?}`.
    Namepart {
        /// The namepart.
        value: Option<String>,
        /// `use`, if the attribute is present.
        use_: Option<Option<String>>,
        /// `inits`, if the attribute is present.
        inits: Option<Option<String>>,
    },
    /// A literal string.
    Literal {
        /// The literal.
        value: Option<String>,
    },
    /// A part of another type (biber pushes `undef`).
    Undef,
}

/// A `\DeclareSortingNamekeyTemplate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortingNameKeyTemplate {
    /// `visibility`.
    pub visibility: Option<String>,
    /// Key parts in order, each a list of parts in order.
    pub template: Vec<Vec<SnkPart>>,
}

/// Parse one `<sortingnamekeytemplate>`.
pub fn sortingnamekey_template(t: &XHash) -> (String, SortingNameKeyTemplate) {
    let template = sorted_by_order(t, "keypart")
        .into_iter()
        .map(|kp| {
            sorted_by_order(kp, "part")
                .into_iter()
                .map(|p| match p.str("type") {
                    Some("namepart") => SnkPart::Namepart {
                        value: p.content().map(str::to_string),
                        use_: p.contains("use").then(|| s(p, "use")),
                        inits: p.contains("inits").then(|| s(p, "inits")),
                    },
                    Some("literal") => SnkPart::Literal {
                        value: p.content().map(str::to_string),
                    },
                    _ => SnkPart::Undef,
                })
                .collect()
        })
        .collect();
    (
        t.str("name").unwrap_or("").to_string(),
        SortingNameKeyTemplate {
            visibility: s(t, "visibility"),
            template,
        },
    )
}

impl SortingNameKeyTemplate {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("visibility".into(), opt(&self.visibility));
        let tpl = if self.template.is_empty() {
            Value::Null
        } else {
            Value::Array(
                self.template
                    .iter()
                    .map(|kp| {
                        if kp.is_empty() {
                            return Value::Null;
                        }
                        Value::Array(
                            kp.iter()
                                .map(|p| match p {
                                    SnkPart::Namepart { value, use_, inits } => {
                                        let mut o = Map::new();
                                        o.insert("type".into(), "namepart".into());
                                        o.insert("value".into(), opt(value));
                                        if let Some(u) = use_ {
                                            o.insert("use".into(), opt(u));
                                        }
                                        if let Some(i) = inits {
                                            o.insert("inits".into(), opt(i));
                                        }
                                        Value::Object(o)
                                    }
                                    SnkPart::Literal { value } => {
                                        let mut o = Map::new();
                                        o.insert("type".into(), "literal".into());
                                        o.insert("value".into(), opt(value));
                                        Value::Object(o)
                                    }
                                    SnkPart::Undef => Value::Null,
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            )
        };
        m.insert("template".into(), tpl);
        Value::Object(m)
    }
}

/// Options of one sorting pass (`<sort>` attributes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SortPassOpts {
    /// `final`.
    pub final_: Option<String>,
    /// `sort_direction`.
    pub sort_direction: Option<String>,
    /// `sortcase`.
    pub sortcase: Option<String>,
    /// `sortupper`.
    pub sortupper: Option<String>,
    /// `locale`.
    pub locale: Option<String>,
}

/// Attributes of a sort item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SortItemAttrs {
    /// `substring_side`.
    pub substring_side: Option<String>,
    /// `substring_width`.
    pub substring_width: Option<String>,
    /// `pad_width`.
    pub pad_width: Option<String>,
    /// `pad_char`.
    pub pad_char: Option<String>,
    /// `pad_side`.
    pub pad_side: Option<String>,
    /// `literal`.
    pub literal: Option<String>,
}

/// A sort item: the field/pseudo-field (or literal) and its attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortItem {
    /// The item (element content; `""` for an empty element, as Perl's hash
    /// key would be).
    pub item: String,
    /// Its attributes.
    pub attrs: SortItemAttrs,
}

/// One sorting pass: options plus items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortPass {
    /// The pass options.
    pub opts: SortPassOpts,
    /// The items, in order.
    pub items: Vec<SortItem>,
}

/// A `\DeclareSortingTemplate` (`_parse_sort`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortingTemplate {
    /// `locale2bcp47(template locale || global biblatex sortlocale)`.
    pub locale: Option<String>,
    /// The passes (passes without items are dropped).
    pub spec: Vec<SortPass>,
}

/// `_parse_sort` (Biber.pm:4995): `global_sortlocale` is the biblatex
/// `sortlocale` option at parse time.
pub fn parse_sort(root: &XHash, global_sortlocale: Option<&str>) -> SortingTemplate {
    let mut spec = Vec::new();
    for sort in sorted_by_order(root, "sort") {
        let items: Vec<SortItem> = sorted_by_order(sort, "sortitem")
            .into_iter()
            .map(|si| SortItem {
                item: si.content().unwrap_or("").to_string(),
                attrs: SortItemAttrs {
                    substring_side: s(si, "substring_side"),
                    substring_width: s(si, "substring_width"),
                    pad_width: s(si, "pad_width"),
                    pad_char: s(si, "pad_char"),
                    pad_side: s(si, "pad_side"),
                    literal: s(si, "literal"),
                },
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        spec.push(SortPass {
            opts: SortPassOpts {
                final_: s(sort, "final"),
                sort_direction: s(sort, "sort_direction"),
                sortcase: s(sort, "sortcase"),
                sortupper: s(sort, "sortupper"),
                locale: s(sort, "locale"),
            },
            items,
        });
    }
    // `$root_obj->{locale} || getblxoption(undef, 'sortlocale')`
    let loc = match root.str("locale") {
        Some(l) if crate::perlish::truthy(Some(l)) => Some(l),
        _ => global_sortlocale,
    };
    SortingTemplate {
        locale: locale2bcp47(loc).map(str::to_string),
        spec,
    }
}

impl SortingTemplate {
    /// The shape `_parse_sort` returns.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("locale".into(), opt(&self.locale));
        let spec = if self.spec.is_empty() {
            Value::Null
        } else {
            Value::Array(
                self.spec
                    .iter()
                    .map(|p| {
                        let mut v = Vec::new();
                        let mut o = Map::new();
                        for (k, x) in [
                            ("final", &p.opts.final_),
                            ("sort_direction", &p.opts.sort_direction),
                            ("sortcase", &p.opts.sortcase),
                            ("sortupper", &p.opts.sortupper),
                            ("locale", &p.opts.locale),
                        ] {
                            if let Some(x) = x {
                                o.insert(k.into(), x.clone().into());
                            }
                        }
                        v.push(Value::Object(o));
                        for it in &p.items {
                            let mut a = Map::new();
                            for (k, x) in [
                                ("substring_side", &it.attrs.substring_side),
                                ("substring_width", &it.attrs.substring_width),
                                ("pad_width", &it.attrs.pad_width),
                                ("pad_char", &it.attrs.pad_char),
                                ("pad_side", &it.attrs.pad_side),
                                ("literal", &it.attrs.literal),
                            ] {
                                if let Some(x) = x {
                                    a.insert(k.into(), x.clone().into());
                                }
                            }
                            let mut item = Map::new();
                            item.insert(it.item.clone(), Value::Object(a));
                            v.push(Value::Object(item));
                        }
                        Value::Array(v)
                    })
                    .collect(),
            )
        };
        m.insert("spec".into(), spec);
        Value::Object(m)
    }
}

/// A set of named templates (Perl hash: later names overwrite earlier ones).
pub type Named<T> = BTreeMap<String, T>;

/// JSON for a named set of part lists.
pub fn named_parts_json<T>(m: &Named<Vec<T>>, f: impl Fn(&T) -> Value) -> Value {
    let mut o = Map::new();
    for (k, v) in m {
        o.insert(k.clone(), Value::Array(v.iter().map(&f).collect()));
    }
    Value::Object(o)
}

/// JSON for labelalphaname templates.
pub fn labelalphaname_json(m: &Named<Vec<LabelAlphaNamePart>>) -> Value {
    named_parts_json(m, LabelAlphaNamePart::to_json)
}

/// JSON for uniquename templates.
pub fn uniquename_json(m: &Named<Vec<UniquenamePart>>) -> Value {
    named_parts_json(m, UniquenamePart::to_json)
}

/// JSON for namehash templates.
pub fn namehash_json(m: &Named<Vec<NamehashPart>>) -> Value {
    named_parts_json(m, NamehashPart::to_json)
}

/// JSON for sorting name key templates.
pub fn sortingnamekey_json(m: &Named<SortingNameKeyTemplate>) -> Value {
    let mut o = Map::new();
    for (k, v) in m {
        o.insert(k.clone(), v.to_json());
    }
    Value::Object(o)
}

/// JSON for sorting templates.
pub fn sortingtemplates_json(m: &Named<SortingTemplate>) -> Value {
    let mut o = Map::new();
    for (k, v) in m {
        o.insert(k.clone(), v.to_json());
    }
    Value::Object(o)
}

// ---------------------------------------------------------------------------
// Typed views of the declarations biber stores raw.

/// A typed view of a `\DeclareLabelalphaTemplate` (stored raw by biber).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelAlphaTemplate {
    /// `type` (`global` or an entry type).
    pub type_: Option<String>,
    /// Label elements, sorted by `order`; each is a disjunction of parts.
    pub elements: Vec<Vec<LabelPart>>,
}

/// A `<labelpart>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelPart {
    /// The field or literal (content).
    pub content: Option<String>,
    /// All attributes (`final`, `pad_char`, `substring_width`, `ifnames`, …).
    pub attrs: BTreeMap<String, String>,
}

impl LabelAlphaTemplate {
    /// Build from the raw `XMLin` hash biber stores.
    pub fn from_xval(t: &XHash) -> Self {
        let elements = sorted_by_order(t, "labelelement")
            .into_iter()
            .map(|le| {
                le.list("labelpart")
                    .hashes()
                    .map(|p| LabelPart {
                        content: p.content().map(str::to_string),
                        attrs: p
                            .iter()
                            .filter(|(k, _)| *k != "content")
                            .filter_map(|(k, v)| v.as_str().map(|v| (k.to_string(), v.to_string())))
                            .collect(),
                    })
                    .collect()
            })
            .collect();
        LabelAlphaTemplate {
            type_: s(t, "type"),
            elements,
        }
    }
}

/// A typed view of a `<translit>` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Translit {
    /// `langids` (comma list).
    pub langids: Option<String>,
    /// `target`: a field, a datafield set, or `*`.
    pub target: Option<String>,
    /// `from` scheme.
    pub from: Option<String>,
    /// `to` scheme.
    pub to: Option<String>,
}

impl Translit {
    /// Build the list from the raw `translit` array biber stores.
    pub fn list_from_xval(v: &XVal) -> Vec<Translit> {
        let items: Vec<&XVal> = match v {
            XVal::Array(a) => a.iter().collect(),
            other => vec![other],
        };
        items
            .into_iter()
            .filter_map(XVal::as_hash)
            .map(|h| Translit {
                langids: s(h, "langids"),
                target: s(h, "target"),
                from: s(h, "from"),
                to: s(h, "to"),
            })
            .collect()
    }
}

/// A typed view of `\DeclareDataInheritance` (stored raw by biber).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inheritance {
    /// `<defaults>` attributes.
    pub inherit_all: Option<String>,
    /// `override_target` default.
    pub override_target: Option<String>,
    /// `ignore` default.
    pub ignore: Option<String>,
    /// Default type pairs.
    pub default_pairs: Vec<TypePair>,
    /// `<inherit>` rules in document order.
    pub inherits: Vec<InheritRule>,
}

/// A `<type_pair>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypePair {
    /// `source` type or `*`.
    pub source: Option<String>,
    /// `target` type or `*`.
    pub target: Option<String>,
    /// `inherit_all`.
    pub inherit_all: Option<String>,
    /// `override_target`.
    pub override_target: Option<String>,
    /// `suppress`.
    pub suppress: Option<String>,
    /// `ignore`.
    pub ignore: Option<String>,
}

/// An `<inherit>` rule.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InheritRule {
    /// `ignore`.
    pub ignore: Option<String>,
    /// Type pairs the rule applies to.
    pub type_pairs: Vec<TypePair>,
    /// Field specifications.
    pub fields: Vec<InheritField>,
}

/// An inheritance `<field>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InheritField {
    /// `source`.
    pub source: Option<String>,
    /// `target`.
    pub target: Option<String>,
    /// `skip`.
    pub skip: Option<String>,
    /// `override_target`.
    pub override_target: Option<String>,
}

fn type_pairs(h: &XHash) -> Vec<TypePair> {
    h.list("type_pair")
        .hashes()
        .map(|t| TypePair {
            source: s(t, "source"),
            target: s(t, "target"),
            inherit_all: s(t, "inherit_all"),
            override_target: s(t, "override_target"),
            suppress: s(t, "suppress"),
            ignore: s(t, "ignore"),
        })
        .collect()
}

impl Inheritance {
    /// Build from the raw `XMLin` hash.
    pub fn from_xval(h: &XHash) -> Self {
        let d = h.hash("defaults");
        Inheritance {
            inherit_all: d.and_then(|d| s(d, "inherit_all")),
            override_target: d.and_then(|d| s(d, "override_target")),
            ignore: d.and_then(|d| s(d, "ignore")),
            default_pairs: d.map(type_pairs).unwrap_or_default(),
            inherits: h
                .list("inherit")
                .hashes()
                .map(|i| InheritRule {
                    ignore: s(i, "ignore"),
                    type_pairs: type_pairs(i),
                    fields: i
                        .list("field")
                        .hashes()
                        .map(|f| InheritField {
                            source: s(f, "source"),
                            target: s(f, "target"),
                            skip: s(f, "skip"),
                            override_target: s(f, "override_target"),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

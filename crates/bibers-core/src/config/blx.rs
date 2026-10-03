//! biblatex options: the option-scope registry declared by `<optionscope>`
//! (`%CONFIG_OPTSCOPE_BIBLATEX`, `%CONFIG_SCOPEOPT_BIBLATEX`,
//! `%CONFIG_OPTTYPE_BIBLATEX`, `%CONFIG_BIBLATEX_OPTIONS`) and the scoped
//! option store (`$CONFIG->{options}{biblatex}`) with `setblxoption` /
//! `getblxoption` semantics (Config.pm:930-1021).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{Map, Value};

use crate::bcf::templates::{
    LabelAlphaNamePart, Named, NamehashPart, SortingNameKeyTemplate, SortingTemplate,
    UniquenamePart,
};
use crate::perlish::{lc, looks_like_number, split_comma, truthy};
use crate::quirks::{Quirk, Quirks};
use crate::xmlin::{XHash, XVal};

/// An option scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    /// GLOBAL
    Global,
    /// ENTRYTYPE
    EntryType,
    /// ENTRY
    Entry,
    /// NAMELIST
    NameList,
    /// NAME
    Name,
}

impl Scope {
    /// All scopes.
    pub const ALL: [Scope; 5] = [
        Scope::Global,
        Scope::EntryType,
        Scope::Entry,
        Scope::NameList,
        Scope::Name,
    ];

    /// The `.bcf` name.
    pub fn name(self) -> &'static str {
        match self {
            Scope::Global => "GLOBAL",
            Scope::EntryType => "ENTRYTYPE",
            Scope::Entry => "ENTRY",
            Scope::NameList => "NAMELIST",
            Scope::Name => "NAME",
        }
    }

    /// Parse a `.bcf` scope name.
    pub fn from_name(s: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|x| x.name() == s)
    }

    fn idx(self) -> usize {
        self as usize
    }
}

/// `backendin`: how a meta-option is split into sub-options
/// (`process_backendin`, Utils.pm:1935).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendIn {
    /// A comma list: set every sub-option to the meta-option's value.
    List(Vec<String>),
    /// `k=v` pairs: per-sub-option values.
    Map(BTreeMap<String, String>),
}

/// `process_backendin`.
pub fn process_backendin(bin: Option<&str>) -> Option<BackendIn> {
    let bin = bin?;
    if !truthy(Some(bin)) {
        return None;
    }
    let opts = split_comma(bin);
    if opts.iter().any(|o| o.contains('=')) {
        let mut m = BTreeMap::new();
        for o in &opts {
            // m/\s*([^=]+)=(.+)\s*/ — unanchored, greedy
            if let Some((k, v)) = backendin_kv(o) {
                m.insert(k, v);
            }
        }
        Some(BackendIn::Map(m))
    } else {
        Some(BackendIn::List(opts))
    }
}

fn backendin_kv(o: &str) -> Option<(String, String)> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"\s*([^=]+)=(.+)\s*").expect("valid regex"));
    let c = re.captures(o)?;
    Some((c[1].to_string(), c[2].to_string()))
}

/// What `<optionscope>` declares for one option at one scope
/// (`%CONFIG_BIBLATEX_OPTIONS{scope}{opt}`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeDecl {
    /// `OUTPUT`: `backendout || 0`.
    pub output: String,
    /// `INPUT`: `process_backendin(backendin)`, if any.
    pub input: Option<BackendIn>,
}

/// The option registry.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    /// opt → scope → declaration.
    decls: HashMap<String, [Option<ScopeDecl>; 5]>,
    /// Declaration order per scope (`%CONFIG_SCOPEOPT_BIBLATEX`; we keep
    /// first-declaration order, DC-013).
    scope_order: [Vec<String>; 5],
    /// `%CONFIG_OPTTYPE_BIBLATEX` (lowercased datatype).
    types: HashMap<String, String>,
    /// Options that `setblxoption`/`addtoblxoption` looked up without being
    /// declared: Perl autovivifies `$CONFIG_OPTSCOPE_BIBLATEX{$opt}` to `{}`,
    /// which makes them "known" to `setconfigfileoption`.
    vivified: std::collections::BTreeSet<String>,
}

impl Registry {
    /// Declare `opt` at `scope` (one `<option>` of an `<optionscope>`).
    /// `datatype` is recorded only when `record_type` (the `.bcf` does,
    /// biber.conf does not); a conflicting datatype yields the warning text.
    pub fn declare(
        &mut self,
        scope: Scope,
        opt: &str,
        backendout: Option<&str>,
        backendin: Option<&str>,
        datatype: Option<&str>,
        record_type: bool,
    ) -> Option<String> {
        let output = match backendout {
            Some(b) if truthy(Some(b)) => b.to_string(),
            _ => "0".to_string(),
        };
        let input = process_backendin(backendin);
        let entry = self
            .decls
            .entry(opt.to_string())
            .or_insert_with(|| [None, None, None, None, None]);
        if entry[scope.idx()].is_none() {
            self.scope_order[scope.idx()].push(opt.to_string());
        }
        // `$CONFIG_BIBLATEX_OPTIONS{$scope}{$opt}{OUTPUT} = …; {INPUT} = … if $bin`
        let prev_input = entry[scope.idx()].as_ref().and_then(|d| d.input.clone());
        entry[scope.idx()] = Some(ScopeDecl {
            output,
            input: input.or(prev_input),
        });
        if record_type {
            let dt = lc(datatype.unwrap_or(""));
            if let Some(existing) = self.types.get(opt) {
                if lc(existing) != dt {
                    return Some(format!(
                        "Warning: Datatype for biblatex option '{opt}' has conflicting values, probably at different scopes. This is not supported."
                    ));
                }
            } else {
                self.types.insert(opt.to_string(), dt);
            }
        }
        None
    }

    /// `$CONFIG_OPTSCOPE_BIBLATEX{$opt}{$scope}`.
    pub fn has_scope(&self, opt: &str, scope: Scope) -> bool {
        self.decls
            .get(opt)
            .is_some_and(|d| d[scope.idx()].is_some())
    }

    /// `$CONFIG_OPTSCOPE_BIBLATEX{$opt}` is true: declared at any scope, or
    /// autovivified by an earlier lookup.
    pub fn is_declared(&self, opt: &str) -> bool {
        self.decls.contains_key(opt) || self.vivified.contains(opt)
    }

    /// Record the autovivification of `$CONFIG_OPTSCOPE_BIBLATEX{$opt}`.
    pub fn touch(&mut self, opt: &str) {
        if !self.decls.contains_key(opt) && !self.vivified.contains(opt) {
            self.vivified.insert(opt.to_string());
        }
    }

    /// `%CONFIG_BIBLATEX_OPTIONS{scope}{opt}`.
    pub fn decl(&self, opt: &str, scope: Scope) -> Option<&ScopeDecl> {
        self.decls.get(opt)?[scope.idx()].as_ref()
    }

    /// `%CONFIG_OPTTYPE_BIBLATEX{opt}`.
    pub fn datatype(&self, opt: &str) -> Option<&str> {
        self.types.get(opt).map(String::as_str)
    }

    /// Set a datatype directly (tests use this for `map_boolean`).
    pub fn set_datatype(&mut self, opt: &str, dt: &str) {
        self.types.insert(opt.to_string(), dt.to_string());
    }

    /// Options declared at `scope`, in first-declaration order.
    pub fn options_in_scope(&self, scope: Scope) -> &[String] {
        &self.scope_order[scope.idx()]
    }

    /// `map_boolean($bn, $bv, 'tonum')` (Utils.pm:1262) for a scalar:
    /// non-boolean options pass through; `true`/`false` (any case) map to
    /// `1`/`0`; numbers are returned lowercased; anything else is undef.
    pub fn map_boolean_tonum(&self, opt: &str, val: Option<&str>) -> Option<String> {
        if self.datatype(opt) != Some("boolean") {
            return val.map(str::to_string);
        }
        map_boolean_tonum_raw(val)
    }

    /// JSON dump of the registry in biber's shapes.
    pub fn to_json(&self) -> Value {
        let mut optscope = Map::new();
        let mut blxopts = Map::new();
        for sc in Scope::ALL {
            blxopts.insert(sc.name().into(), Value::Object(Map::new()));
        }
        let mut names: Vec<&String> = self.decls.keys().collect();
        names.sort();
        for opt in names {
            let d = &self.decls[opt];
            let mut sc = Map::new();
            for s in Scope::ALL {
                if let Some(decl) = &d[s.idx()] {
                    sc.insert(s.name().into(), "1".into());
                    let mut o = Map::new();
                    o.insert("OUTPUT".into(), decl.output.clone().into());
                    if let Some(i) = &decl.input {
                        o.insert(
                            "INPUT".into(),
                            match i {
                                BackendIn::List(l) => {
                                    Value::Array(l.iter().map(|x| x.clone().into()).collect())
                                }
                                BackendIn::Map(m) => Value::Object(
                                    m.iter()
                                        .map(|(k, v)| (k.clone(), v.clone().into()))
                                        .collect(),
                                ),
                            },
                        );
                    }
                    if let Some(Value::Object(m)) = blxopts.get_mut(s.name()) {
                        m.insert(opt.clone(), Value::Object(o));
                    }
                }
            }
            optscope.insert(opt.clone(), Value::Object(sc));
        }
        for v in &self.vivified {
            if !self.decls.contains_key(v) {
                optscope.insert(v.clone(), Value::Object(Map::new()));
            }
        }
        let mut types = Map::new();
        let mut tn: Vec<(&String, &String)> = self.types.iter().collect();
        tn.sort();
        for (k, v) in tn {
            types.insert(k.clone(), v.clone().into());
        }
        let mut m = Map::new();
        m.insert("optscope".into(), Value::Object(optscope));
        m.insert("opttype".into(), Value::Object(types));
        m.insert("blxoptions".into(), Value::Object(blxopts));
        Value::Object(m)
    }
}

/// `map_boolean(…, 'tonum')` for a boolean-typed option.
pub fn map_boolean_tonum_raw(val: Option<&str>) -> Option<String> {
    let b = lc(val.unwrap_or(""));
    if looks_like_number(&b) {
        return Some(b);
    }
    match b.as_str() {
        "true" => Some("1".into()),
        "false" => Some("0".into()),
        _ => None,
    }
}

/// `map_boolean(…, 'tostring')` for a boolean-typed option.
pub fn map_boolean_tostring_raw(val: Option<&str>) -> Option<String> {
    let b = lc(val.unwrap_or(""));
    if !looks_like_number(&b) {
        return Some(b);
    }
    // %map = reverse (true => 1, false => 0): keys "1" and "0"
    match b.as_str() {
        "1" => Some("true".into()),
        "0" => Some("false".into()),
        _ => None,
    }
}

/// A biblatex option value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptValue {
    /// `undef` (e.g. `map_boolean` of an unknown string). Treated as "not
    /// defined" by `getblxoption`.
    Undef,
    /// A scalar.
    Str(String),
    /// A multivalued `.bcf` option: the value hashes (`{content, type, …}`
    /// minus `order`), sorted by `order`.
    Multi(Vec<XHash>),
    /// A raw `XMLin` structure (inheritance, labelalphatemplate, translit).
    Xml(XVal),
    /// The data model list (`[bcf_dm]`, or tool + user DMs).
    Datamodel(Vec<XVal>),
    /// `labelalphanametemplate`.
    LabelAlphaNameTemplates(Named<Vec<LabelAlphaNamePart>>),
    /// `uniquenametemplate`.
    UniquenameTemplates(Named<Vec<UniquenamePart>>),
    /// `namehashtemplate`.
    NamehashTemplates(Named<Vec<NamehashPart>>),
    /// `sortingnamekeytemplate`.
    SortingNameKeyTemplates(Named<SortingNameKeyTemplate>),
    /// `sortingtemplate`.
    SortingTemplates(Named<SortingTemplate>),
    /// `extradatespec`: scopes, each a list of fields in order.
    ExtradateSpec(Vec<Vec<String>>),
    /// `sortexclusion`/`sortinclusion`: a set of field names.
    FieldSet(BTreeSet<String>),
}

impl OptValue {
    /// The scalar string, if this is a scalar.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            OptValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Perl truthiness (references are true).
    pub fn truthy(&self) -> bool {
        match self {
            OptValue::Undef => false,
            OptValue::Str(s) => truthy(Some(s)),
            _ => true,
        }
    }

    /// Is this defined?
    pub fn is_defined(&self) -> bool {
        !matches!(self, OptValue::Undef)
    }

    /// JSON in biber's shape (scalars as strings).
    pub fn to_json(&self) -> Value {
        use crate::bcf::templates as t;
        match self {
            OptValue::Undef => Value::Null,
            OptValue::Str(s) => s.clone().into(),
            OptValue::Multi(v) => {
                Value::Array(v.iter().map(|h| XVal::Hash(h.clone()).to_json()).collect())
            }
            OptValue::Xml(x) => x.to_json(),
            OptValue::Datamodel(v) => Value::Array(v.iter().map(XVal::to_json).collect()),
            OptValue::LabelAlphaNameTemplates(m) => t::labelalphaname_json(m),
            OptValue::UniquenameTemplates(m) => t::uniquename_json(m),
            OptValue::NamehashTemplates(m) => t::namehash_json(m),
            OptValue::SortingNameKeyTemplates(m) => t::sortingnamekey_json(m),
            OptValue::SortingTemplates(m) => t::sortingtemplates_json(m),
            OptValue::ExtradateSpec(v) => Value::Array(
                v.iter()
                    .map(|s| {
                        if s.is_empty() {
                            Value::Null
                        } else {
                            Value::Array(s.iter().map(|x| x.clone().into()).collect())
                        }
                    })
                    .collect(),
            ),
            OptValue::FieldSet(s) => {
                Value::Object(s.iter().map(|k| (k.clone(), Value::from("1"))).collect())
            }
        }
    }
}

/// A section number as biber keys per-entry options (`{citekey}{secnum}`).
pub type SecNum = String;

/// The biblatex option store.
#[derive(Clone, Debug, Default)]
pub struct BlxOptions {
    /// The scope registry.
    pub registry: Registry,
    global: HashMap<String, OptValue>,
    entrytype: HashMap<String, HashMap<String, OptValue>>,
    entry: HashMap<(String, SecNum), HashMap<String, OptValue>>,
}

impl BlxOptions {
    /// `map_boolean($opt, $val, 'tonum')` for any value: scalars are mapped;
    /// a non-scalar given to a boolean option becomes undef (Perl `lc` of a
    /// reference).
    pub fn map_boolean(&self, opt: &str, val: OptValue) -> OptValue {
        if self.registry.datatype(opt) != Some("boolean") {
            return val;
        }
        match val {
            OptValue::Str(s) => self
                .registry
                .map_boolean_tonum(opt, Some(&s))
                .map_or(OptValue::Undef, OptValue::Str),
            OptValue::Undef => self
                .registry
                .map_boolean_tonum(opt, None)
                .map_or(OptValue::Undef, OptValue::Str),
            _ => OptValue::Undef,
        }
    }

    /// `setblxoption($secnum, $opt, $val)` at GLOBAL scope. Undeclared
    /// options are silently dropped.
    pub fn set_global(&mut self, opt: &str, val: OptValue) {
        self.registry.touch(opt);
        let val = self.map_boolean(opt, val);
        if self.registry.has_scope(opt, Scope::Global) {
            self.global.insert(opt.to_string(), val);
        }
    }

    /// `setblxoption(…, 'ENTRYTYPE', $type)`.
    pub fn set_entrytype(&mut self, entrytype: &str, opt: &str, val: OptValue) {
        self.registry.touch(opt);
        let val = self.map_boolean(opt, val);
        if self.registry.has_scope(opt, Scope::EntryType) {
            self.entrytype
                .entry(entrytype.to_string())
                .or_default()
                .insert(opt.to_string(), val);
        }
    }

    /// `setblxoption($secnum, …, 'ENTRY', $citekey)`.
    pub fn set_entry(&mut self, secnum: &str, citekey: &str, opt: &str, val: OptValue) {
        self.registry.touch(opt);
        let val = self.map_boolean(opt, val);
        if self.registry.has_scope(opt, Scope::Entry) {
            self.entry
                .entry((citekey.to_string(), secnum.to_string()))
                .or_default()
                .insert(opt.to_string(), val);
        }
    }

    /// Set a GLOBAL value without scope check or boolean mapping (the
    /// biblatex half of `setconfigfileoption`, Config.pm:891).
    pub fn set_global_raw(&mut self, opt: &str, val: OptValue) {
        self.global.insert(opt.to_string(), val);
    }

    /// `addtoblxoption` (only for the `datamodel` list).
    pub fn add_datamodel(&mut self, dm: XVal) {
        self.registry.touch("datamodel");
        if !self.registry.has_scope("datamodel", Scope::Global) {
            return;
        }
        match self.global.get_mut("datamodel") {
            Some(OptValue::Datamodel(v)) => v.push(dm),
            _ => {
                self.global
                    .insert("datamodel".to_string(), OptValue::Datamodel(vec![dm]));
            }
        }
    }

    /// `getblxoption($secnum, $opt, $entrytype, $citekey)` with biber's exact
    /// semantics: an undef `secnum` never matches a per-entry option (BB-021).
    pub fn get(
        &self,
        secnum: Option<&str>,
        opt: &str,
        entrytype: Option<&str>,
        citekey: Option<&str>,
    ) -> Option<&OptValue> {
        if let (Some(sec), Some(key)) = (secnum, citekey)
            && self.registry.has_scope(opt, Scope::Entry)
            && let Some(v) = self
                .entry
                .get(&(key.to_string(), sec.to_string()))
                .and_then(|m| m.get(opt))
            && v.is_defined()
        {
            return Some(v);
        }
        if let Some(et) = entrytype
            && self.registry.has_scope(opt, Scope::EntryType)
            && let Some(v) = self.entrytype.get(&lc(et)).and_then(|m| m.get(opt))
            && v.is_defined()
        {
            return Some(v);
        }
        if self.registry.has_scope(opt, Scope::Global) {
            return self.global.get(opt);
        }
        None
    }

    /// A lookup at a call site where biber passes an undef section number
    /// (`getblxoption(undef, …)`). With the `entry-scoped-options-ignored`
    /// quirk (BB-021, default) per-entry options are invisible; with it fixed
    /// the current section's per-entry options are used.
    pub fn get_undef_secnum(
        &self,
        quirks: Quirks,
        current_secnum: &str,
        opt: &str,
        entrytype: Option<&str>,
        citekey: Option<&str>,
    ) -> Option<&OptValue> {
        if quirks.on(Quirk::EntryScopedOptionsIgnored) {
            self.get(None, opt, entrytype, citekey)
        } else {
            self.get(Some(current_secnum), opt, entrytype, citekey)
        }
    }

    /// Mutable access to a stored GLOBAL value.
    pub fn global_mut(&mut self, opt: &str) -> Option<&mut OptValue> {
        self.global.get_mut(opt)
    }

    /// The GLOBAL value of `opt` (`getblxoption(undef, $opt)`).
    pub fn global(&self, opt: &str) -> Option<&OptValue> {
        self.get(None, opt, None, None)
    }

    /// `getblxentryoptions($secnum, $key)`: names of the per-entry options.
    pub fn entry_options(&self, secnum: &str, citekey: &str) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .entry
            .get(&(citekey.to_string(), secnum.to_string()))
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        v.sort_unstable();
        v
    }

    /// JSON dump (`GLOBAL`, `ENTRYTYPE`, `ENTRY`).
    pub fn to_json(&self) -> Value {
        let sorted = |m: &HashMap<String, OptValue>| {
            let mut o = Map::new();
            let mut ks: Vec<&String> = m.keys().collect();
            ks.sort();
            for k in ks {
                o.insert(k.clone(), m[k].to_json());
            }
            Value::Object(o)
        };
        let mut et = Map::new();
        let mut ets: Vec<&String> = self.entrytype.keys().collect();
        ets.sort();
        for t in ets {
            et.insert(t.clone(), sorted(&self.entrytype[t]));
        }
        let mut en = Map::new();
        for ((key, sec), opts) in &self.entry {
            let e = en
                .entry(key.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(m) = e {
                m.insert(sec.clone(), sorted(opts));
            }
        }
        let mut m = Map::new();
        m.insert("GLOBAL".into(), sorted(&self.global));
        m.insert("ENTRYTYPE".into(), Value::Object(et));
        m.insert("ENTRY".into(), Value::Object(en));
        Value::Object(m)
    }
}

/// `expand_option_input` (Utils.pm:1353): expand a (possibly meta-) option
/// into the options to set, given its `backendin` declaration.
pub fn expand_option_input(
    registry: &Registry,
    opt: &str,
    val: Option<&str>,
    cfopt: Option<&BackendIn>,
) -> Vec<(String, Option<String>)> {
    let val = registry.map_boolean_tonum(opt, val);
    match cfopt {
        None => vec![(opt.to_string(), val)],
        Some(BackendIn::List(ks)) => ks.iter().map(|k| (k.clone(), val.clone())).collect(),
        Some(BackendIn::Map(m)) => {
            let mut out = Vec::new();
            for (k, v) in m {
                let subval = registry.map_boolean_tonum(k, Some(v));
                if !truthy(val.as_deref()) {
                    if registry.datatype(k) == Some("boolean") {
                        let inv = if truthy(subval.as_deref()) { "0" } else { "1" };
                        out.push((k.clone(), Some(inv.to_string())));
                    }
                } else {
                    out.push((k.clone(), subval));
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_boolean_vectors() {
        // utils.t 74-78
        let mut r = Registry::default();
        r.set_datatype("test", "boolean");
        assert_eq!(
            r.map_boolean_tonum("test", Some("true")).as_deref(),
            Some("1")
        );
        assert_eq!(
            r.map_boolean_tonum("test", Some("False")).as_deref(),
            Some("0")
        );
        assert_eq!(map_boolean_tostring_raw(Some("1")).as_deref(), Some("true"));
        assert_eq!(
            map_boolean_tostring_raw(Some("0")).as_deref(),
            Some("false")
        );
        assert_eq!(r.map_boolean_tonum("test", Some("0")).as_deref(), Some("0"));
        assert_eq!(r.map_boolean_tonum("test", Some("maybe")), None);
        assert_eq!(
            r.map_boolean_tonum("other", Some("maybe")).as_deref(),
            Some("maybe")
        );
    }

    #[test]
    fn backendin() {
        assert_eq!(
            process_backendin(Some("a, b,c")),
            Some(BackendIn::List(vec!["a".into(), "b".into(), "c".into()]))
        );
        let Some(BackendIn::Map(m)) = process_backendin(Some("skipbib=true,uniquename=false"))
        else {
            panic!()
        };
        assert_eq!(m["skipbib"], "true");
        assert_eq!(m["uniquename"], "false");
        assert_eq!(process_backendin(Some("")), None);
    }

    #[test]
    fn scoped_lookup() {
        let mut b = BlxOptions::default();
        b.registry
            .declare(Scope::Global, "x", None, None, Some("string"), true);
        b.registry
            .declare(Scope::EntryType, "x", None, None, Some("string"), true);
        b.registry
            .declare(Scope::Entry, "x", None, None, Some("string"), true);
        b.set_global("x", OptValue::Str("g".into()));
        b.set_entrytype("book", "x", OptValue::Str("t".into()));
        b.set_entry("0", "k", "x", OptValue::Str("e".into()));
        b.set_global("undeclared", OptValue::Str("1".into()));
        assert_eq!(
            b.get(Some("0"), "x", Some("Book"), Some("k"))
                .unwrap()
                .as_str(),
            Some("e")
        );
        assert_eq!(
            b.get(None, "x", Some("Book"), Some("k")).unwrap().as_str(),
            Some("t")
        );
        assert_eq!(
            b.get(None, "x", Some("article"), Some("k"))
                .unwrap()
                .as_str(),
            Some("g")
        );
        assert!(b.global("undeclared").is_none());
        let q = Quirks::default();
        assert_eq!(
            b.get_undef_secnum(q, "0", "x", None, Some("k"))
                .unwrap()
                .as_str(),
            Some("g")
        );
        let mut q = Quirks::default();
        q.fix(Quirk::EntryScopedOptionsIgnored);
        assert_eq!(
            b.get_undef_secnum(q, "0", "x", None, Some("k"))
                .unwrap()
                .as_str(),
            Some("e")
        );
    }
}

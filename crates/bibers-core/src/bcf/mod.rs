//! The control file (`.bcf`): `parse_ctrlfile` (Biber.pm:345-1170) and the
//! parse-time parts of `process_setup` (Biber.pm:1178-1232).
//!
//! The `.bcf` is read through an exact emulation of biber's `XMLin` call
//! ([`crate::xmlin`]) and then interpreted the way biber's Perl does, so the
//! resulting options, templates, sections and data lists are identical
//! (checked differentially against biber over the whole corpus).

pub mod sourcemap;
pub mod templates;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::{Map, Value};

use crate::config::biber_opts::{BiberValue, PatternOpt};
use crate::config::blx::{OptValue, Scope};
use crate::config::{Config, DfsMember};
use crate::datamodel::DataModel;
use crate::env::Env;
use crate::files::{Lookup, locate_data_file};
use crate::log::{Fatal, Logger};
use crate::perlish::{nfd, sort_by_order, split_comma, truthy};
use crate::quirks::Quirk;
use crate::xmlin::{XHash, XVal, parse_document, xmlin_document};
use crate::{biber_error, debug, info};
use sourcemap::SourceMaps;
use templates::{
    labelalphaname_template, namehash_template, parse_sort, sortingnamekey_template,
    uniquename_template,
};

/// The `.bcf` format version this release expects (`$BCF_VERSION`).
pub const BCF_VERSION: &str = "3.11";

/// The `ForceArray` list of `parse_ctrlfile` (Biber.pm:432-488).
pub fn bcf_force_array(n: &str) -> bool {
    // qr/\A(?:no)*citekey(?:count)?\z/
    let mut t = n;
    while let Some(r) = t.strip_prefix("no") {
        t = r;
    }
    if t == "citekey" || t == "citekeycount" {
        return true;
    }
    matches!(
        n,
        "option"
            | "options"
            | "value"
            | "sortitem"
            | "bibdata"
            | "datasource"
            | "constant"
            | "section"
            | "sortexclusion"
            | "sortinclusion"
            | "exclusion"
            | "inclusion"
            | "sort"
            | "mode"
            | "maps"
            | "map"
            | "map_step"
            | "per_type"
            | "per_nottype"
            | "keypart"
            | "part"
            | "sortingnamekeytemplate"
            | "sortingtemplate"
            | "per_datasource"
            | "nosort"
            | "nonamestring"
            | "member"
            | "noinit"
            | "nolabel"
            | "nolabelwidthcount"
            | "presort"
            | "type_pair"
            | "inherit"
            | "namepart"
            | "fieldor"
            | "fieldxor"
            | "field"
            | "scope"
            | "transliteration"
            | "translit"
            | "alias"
            | "alsoset"
            | "constraints"
            | "constraint"
            | "entryfields"
            | "entrytype"
            | "datetype"
            | "datalist"
            | "labelpart"
            | "labelelement"
            | "labelalphatemplate"
            | "labelalphanametemplate"
            | "uniquenametemplate"
            | "namehashtemplate"
            | "condition"
            | "filter"
            | "filteror"
            | "optionscope"
    )
}

/// A data source of a section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataSource {
    /// `type` (file).
    pub type_: Option<String>,
    /// The file name (kept in OS normalisation form, not NFD).
    pub name: Option<String>,
    /// `datatype` (bibtex, biblatexml).
    pub datatype: Option<String>,
    /// `encoding` (defaults to `input_encoding`).
    pub encoding: Option<String>,
    /// `glob` (defaults to `glob_datasources`).
    pub glob: Option<String>,
}

/// A refsection as known after parsing the `.bcf` (the parse-time parts of
/// `Biber::Section`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Section {
    /// The section number as written in the `.bcf`.
    pub number: String,
    /// Data sources (`None` if the section has none).
    pub datasources: Option<Vec<DataSource>>,
    /// Cited keys in citation order (including dynamic set keys).
    pub citekeys: Vec<String>,
    /// `orig_order_citekeys`.
    pub orig_order_citekeys: Vec<String>,
    /// `\nocite{*}` / `\cite{*}`.
    pub allkeys: bool,
    /// `*` came via `\nocite`.
    pub allkeys_nocite: bool,
    /// Keys cited with `\cite`.
    pub cite: BTreeSet<String>,
    /// Keys cited with `\nocite` only.
    pub nocite: BTreeSet<String>,
    /// Dynamic sets: set key → member keys.
    pub dynamic_sets: BTreeMap<String, Vec<String>>,
    /// `citecount` from `<citekeycount>`.
    pub citecount: BTreeMap<String, Option<String>>,
    /// `seenkeys` counts.
    pub seenkeys: BTreeMap<String, u32>,
    citekeys_h: HashSet<String>,
}

impl Section {
    /// An empty section.
    pub fn new(number: &str) -> Self {
        Section {
            number: number.to_string(),
            ..Section::default()
        }
    }

    /// `add_citekeys`.
    pub fn add_citekeys(&mut self, keys: &[String]) {
        for k in keys {
            if self.citekeys_h.contains(k) {
                continue;
            }
            self.citekeys_h.insert(k.clone());
            self.citekeys.push(k.clone());
            self.orig_order_citekeys.push(k.clone());
        }
    }

    /// `del_citekeys`.
    pub fn del_citekeys(&mut self) {
        self.citekeys.clear();
        self.citekeys_h.clear();
        self.orig_order_citekeys.clear();
    }

    /// `has_citekey` (no aliases at parse time).
    pub fn has_citekey(&self, k: &str) -> bool {
        self.citekeys_h.contains(k)
    }

    /// `get_citecount`: `-1` for a zero/undefined count.
    pub fn get_citecount(&self, k: &str) -> String {
        match self.citecount.get(k).and_then(|c| c.as_deref()) {
            Some(c) if truthy(Some(c)) => c.to_string(),
            _ => "-1".to_string(),
        }
    }

    fn to_json(&self) -> Value {
        let strs = |v: &mut dyn Iterator<Item = &String>| {
            Value::Array(v.map(|x| Value::from(x.clone())).collect())
        };
        let mut m = Map::new();
        m.insert("number".into(), self.number.clone().into());
        m.insert(
            "datasources".into(),
            match &self.datasources {
                None => Value::Null,
                Some(ds) => Value::Array(
                    ds.iter()
                        .map(|d| {
                            let o =
                                |x: &Option<String>| x.clone().map_or(Value::Null, Value::String);
                            let mut h = Map::new();
                            h.insert("type".into(), o(&d.type_));
                            h.insert("name".into(), o(&d.name));
                            h.insert("datatype".into(), o(&d.datatype));
                            h.insert("encoding".into(), o(&d.encoding));
                            h.insert("glob".into(), o(&d.glob));
                            Value::Object(h)
                        })
                        .collect(),
                ),
            },
        );
        m.insert("citekeys".into(), strs(&mut self.citekeys.iter()));
        m.insert(
            "orig_order_citekeys".into(),
            strs(&mut self.orig_order_citekeys.iter()),
        );
        m.insert(
            "allkeys".into(),
            (if self.allkeys { "1" } else { "0" }).into(),
        );
        m.insert(
            "allkeys_nocite".into(),
            (if self.allkeys_nocite { "1" } else { "0" }).into(),
        );
        m.insert("cite".into(), strs(&mut self.cite.iter()));
        m.insert("nocite".into(), strs(&mut self.nocite.iter()));
        m.insert(
            "dkeys".into(),
            Value::Object(
                self.dynamic_sets
                    .iter()
                    .map(|(k, v)| (k.clone(), strs(&mut v.iter())))
                    .collect(),
            ),
        );
        m.insert(
            "citecount".into(),
            Value::Object(
                self.citecount
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone().map_or(Value::Null, Value::String)))
                    .collect(),
            ),
        );
        m.insert(
            "seenkeys".into(),
            Value::Object(
                self.seenkeys
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::from(v.to_string())))
                    .collect(),
            ),
        );
        Value::Object(m)
    }
}

/// A data list filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    /// One `<filter type=…>value</filter>`.
    One {
        /// type, nottype, subtype, notsubtype, keyword, notkeyword, field,
        /// notfield.
        type_: Option<String>,
        /// The value.
        value: Option<String>,
    },
    /// A `<filteror>`: any of these.
    Or(Vec<(Option<String>, Option<String>)>),
}

/// A data list (refcontext / bibliography list) declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataList {
    /// The section.
    pub section: Option<String>,
    /// The list name.
    pub name: String,
    /// `entry` or `list`.
    pub type_: String,
    /// `sortingtemplatename`.
    pub sortingtemplatename: Option<String>,
    /// `sortingnamekeytemplatename`.
    pub sortingnamekeytemplatename: Option<String>,
    /// `uniquenametemplatename`.
    pub uniquenametemplatename: Option<String>,
    /// `labelalphanametemplatename`.
    pub labelalphanametemplatename: Option<String>,
    /// `namehashtemplatename`.
    pub namehashtemplatename: Option<String>,
    /// `labelprefix`.
    pub labelprefix: Option<String>,
    /// Filters (AND across entries; OR within [`Filter::Or`]).
    pub filters: Vec<Filter>,
    /// Locale of the level-1 collator used for `sortinithash` (`None` if no
    /// collator was set up).
    pub sortinit_locale: Option<Option<String>>,
}

impl DataList {
    /// A default `entry` list with the given sorting template and all
    /// `global` templates.
    pub fn default_entry(section: &str, sortingtemplatename: &str) -> Self {
        DataList {
            section: Some(section.to_string()),
            name: format!("{sortingtemplatename}/global//global/global/global"),
            type_: "entry".into(),
            sortingtemplatename: Some(sortingtemplatename.to_string()),
            sortingnamekeytemplatename: Some("global".into()),
            uniquenametemplatename: Some("global".into()),
            labelalphanametemplatename: Some("global".into()),
            namehashtemplatename: Some("global".into()),
            labelprefix: Some(String::new()),
            filters: Vec::new(),
            sortinit_locale: None,
        }
    }

    /// `get_attrs`: `stn/snktn/prefix/untn/lantn/nhtn`.
    pub fn attrs(&self) -> String {
        let o = |x: &Option<String>| x.clone().unwrap_or_default();
        [
            o(&self.sortingtemplatename),
            o(&self.sortingnamekeytemplatename),
            o(&self.labelprefix),
            o(&self.uniquenametemplatename),
            o(&self.labelalphanametemplatename),
            o(&self.namehashtemplatename),
        ]
        .join("/")
    }

    fn to_json(&self) -> Value {
        let o = |x: &Option<String>| x.clone().map_or(Value::Null, Value::String);
        let mut m = Map::new();
        m.insert("section".into(), o(&self.section));
        m.insert("name".into(), self.name.clone().into());
        m.insert("type".into(), self.type_.clone().into());
        m.insert("sortingtemplatename".into(), o(&self.sortingtemplatename));
        m.insert(
            "sortingnamekeytemplatename".into(),
            o(&self.sortingnamekeytemplatename),
        );
        m.insert(
            "uniquenametemplatename".into(),
            o(&self.uniquenametemplatename),
        );
        m.insert(
            "labelalphanametemplatename".into(),
            o(&self.labelalphanametemplatename),
        );
        m.insert("namehashtemplatename".into(), o(&self.namehashtemplatename));
        m.insert("labelprefix".into(), o(&self.labelprefix));
        let f = |t: &Option<String>, v: &Option<String>| {
            let mut h = Map::new();
            h.insert("type".into(), o(t));
            h.insert("value".into(), o(v));
            Value::Object(h)
        };
        m.insert(
            "filters".into(),
            Value::Array(
                self.filters
                    .iter()
                    .map(|x| match x {
                        Filter::One { type_, value } => f(type_, value),
                        Filter::Or(v) => Value::Array(v.iter().map(|(t, v)| f(t, v)).collect()),
                    })
                    .collect(),
            ),
        );
        m.insert(
            "sortinit_locale".into(),
            match &self.sortinit_locale {
                None => Value::from("<none>"),
                Some(l) => o(l),
            },
        );
        Value::Object(m)
    }
}

/// Sections and data lists after parsing.
#[derive(Clone, Debug, Default)]
pub struct Document {
    /// Sections in first-appearance order.
    pub sections: Vec<Section>,
    /// Data lists in declaration order.
    pub datalists: Vec<DataList>,
}

impl Document {
    /// `get_section`.
    pub fn section(&self, number: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.number == number)
    }

    fn section_mut(&mut self, number: &str) -> Option<&mut Section> {
        self.sections.iter_mut().find(|s| s.number == number)
    }

    /// `get_sections`: sorted numerically.
    pub fn sorted_sections(&self) -> Vec<&Section> {
        let mut v: Vec<&Section> = self.sections.iter().collect();
        v.sort_by(|a, b| crate::perlish::num_cmp(Some(&a.number), Some(&b.number)));
        v
    }

    /// Data lists of a section (numeric section compare).
    pub fn lists_for_section(&self, number: &str) -> Vec<&DataList> {
        let n = crate::perlish::numify(number);
        self.datalists
            .iter()
            .filter(|l| crate::perlish::numify(l.section.as_deref().unwrap_or("")) == n)
            .collect()
    }

    /// JSON dump.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert(
            "sections".into(),
            Value::Array(
                self.sorted_sections()
                    .into_iter()
                    .map(Section::to_json)
                    .collect(),
            ),
        );
        m.insert(
            "datalists".into(),
            Value::Array(self.datalists.iter().map(DataList::to_json).collect()),
        );
        Value::Object(m)
    }
}

/// The result of `parse_ctrlfile`.
#[derive(Debug)]
pub enum Parsed {
    /// Parsed; carry on.
    Document(Document),
    /// "No data sources defined!": biber exits 0 here without printing
    /// anything (BB-070).
    NoDataSources,
}

/// Inputs for [`parse_ctrlfile`] beyond the configuration.
pub struct ParseArgs<'a> {
    /// The control file as given (`$bcf`).
    pub ctrl_file: &'a str,
    /// The output target, deleted if the `.bcf` is malformed (`-` is never
    /// deleted).
    pub output_target: Option<&'a str>,
}

fn multi_contents(opt: &XHash) -> Vec<Option<String>> {
    let mut vals: Vec<&XHash> = opt.list("value").hashes().collect();
    sort_by_order(&mut vals, |v| v.str("order"));
    vals.into_iter()
        .map(|v| v.content().map(str::to_string))
        .collect()
}

fn multi_hashes(opt: &XHash) -> Vec<XHash> {
    let mut vals: Vec<&XHash> = opt.list("value").hashes().collect();
    sort_by_order(&mut vals, |v| v.str("order"));
    vals.into_iter()
        .map(|v| {
            let mut h = v.clone();
            h.remove("order");
            h
        })
        .collect()
}

/// Read the control file text as biber does: UTF-8, falling back to latin1
/// (and then rewriting the file as UTF-8).
fn read_ctrlfile(env: &Env, path: &str, log: &mut Logger) -> Result<String, Fatal> {
    let bytes = match std::fs::read(env.resolve(path)) {
        Ok(b) => b,
        Err(e) => {
            biber_error!(log, "Cannot read control file '{path}': {e}")?;
            return Err(Fatal);
        }
    };
    match String::from_utf8(bytes) {
        Ok(s) if truthy(Some(&s)) => Ok(s),
        Ok(s) => {
            // An empty (or "0") buffer is false in Perl: biber falls through
            // to the latin1 read, which is false too.
            let _ = s;
            biber_error!(
                log,
                "{path} is not UTF-8 or even latin1, please delete it and run latex again or check that biblatex is writing a valid .bcf file."
            )?;
            Err(Fatal)
        }
        Err(e) => {
            let latin1: String = e.into_bytes().iter().map(|&b| b as char).collect();
            if !truthy(Some(&latin1)) {
                biber_error!(
                    log,
                    "{path} is not UTF-8 or even latin1, please delete it and run latex again or check that biblatex is writing a valid .bcf file."
                )?;
                return Err(Fatal);
            }
            // Write ctrl file as UTF-8 (a side effect on the user's file).
            let _ = std::fs::write(env.resolve(path), latin1.as_bytes());
            Ok(latin1)
        }
    }
}

/// NFD every line except those naming a data source
/// (`m/<bcf:datasource.+>([^<]+)/`), normalising line breaks to `\n`.
fn nfd_except_datasources(buf: &str) -> Cow<'_, str> {
    // Nothing to change in ASCII text with `\n` line ends (`\R` also
    // matches U+0085, U+2028 and U+2029, which are not ASCII).
    if buf.is_ascii() && !buf.bytes().any(|b| matches!(b, b'\r' | b'\x0b' | b'\x0c')) {
        return Cow::Borrowed(buf);
    }
    let mut out = String::with_capacity(buf.len());
    let mut first = true;
    for line in split_lines_r(buf) {
        if !first {
            out.push('\n');
        }
        first = false;
        if line.is_ascii() || is_datasource_line(line) {
            out.push_str(line);
        } else {
            out.push_str(&nfd(line));
        }
    }
    Cow::Owned(out)
}

/// `split(/\R/, $buf, -1)`.
fn split_lines_r(buf: &str) -> Vec<&str> {
    let mut v = Vec::new();
    let mut start = 0;
    let mut chars = buf.char_indices().peekable();
    while let Some((pos, c)) = chars.next() {
        let is_break = matches!(
            c,
            '\n' | '\x0b' | '\x0c' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
        );
        if is_break {
            v.push(&buf[start..pos]);
            let mut end = pos + c.len_utf8();
            if c == '\r' && chars.next_if(|&(_, n)| n == '\n').is_some() {
                end += 1;
            }
            start = end;
        }
    }
    v.push(&buf[start..]);
    v
}

fn is_datasource_line(line: &str) -> bool {
    const TAG: &str = "<bcf:datasource";
    let Some(p) = line.find(TAG) else {
        return false;
    };
    // `.+` (at least one char) then `>` then at least one non-`<` char.
    let after = &line[p + TAG.len()..];
    let mut chars = after.char_indices();
    if chars.next().is_none() {
        return false;
    }
    let rest_start = after.chars().next().map_or(0, char::len_utf8);
    let rest = &after[rest_start..];
    rest.char_indices()
        .any(|(i, c)| c == '>' && rest[i + 1..].chars().next().is_some_and(|n| n != '<'))
}

/// The locale for a list's sortinit collator:
/// `getblxoption(undef, 'sortingtemplate')->{$name}->{locale}`. Like Perl,
/// looking up a missing template name autovivifies an empty template.
fn sortinit_locale(cfg: &mut Config, stn: Option<&str>) -> Option<String> {
    match cfg.blx.global_mut("sortingtemplate") {
        Some(OptValue::SortingTemplates(m)) => m
            .entry(stn.unwrap_or("").to_string())
            .or_insert_with(templates::SortingTemplate::vivified)
            .locale
            .clone(),
        _ => None,
    }
}

/// `parse_ctrlfile`: read the `.bcf` into `cfg` and return the sections and
/// data lists.
pub fn parse_ctrlfile(
    cfg: &mut Config,
    log: &mut Logger,
    env: &Env,
    args: &ParseArgs<'_>,
) -> Result<Parsed, Fatal> {
    let fetched;
    let ctrl_file = match crate::remote::remote_scheme(args.ctrl_file) {
        // locate_data_file fetches a remote control file too.
        Some(tls) => {
            fetched = crate::remote::locate(cfg, log, args.ctrl_file, tls)?;
            fetched.as_str()
        }
        None => args.ctrl_file,
    };
    let lookup = Lookup {
        input_directory: cfg.biber.get_str("input_directory"),
        output_directory: cfg.biber.get_str("output_directory"),
        ctrlfile_path: None,
    };
    let path = match locate_data_file(env, &lookup, ctrl_file) {
        Some(p) => p,
        None => {
            biber_error!(log, "Cannot find '{ctrl_file}'!")?;
            // BB-071: with --nodieonerror biber reaches its friendlier
            // message and then crashes; we stop after it.
            log.biber_error_nodie(
                crate::loc!(),
                &format!(
                    "Cannot find control file '{ctrl_file}'! - Did latex run successfully on your .tex file before you ran biber?"
                ),
            );
            return Err(Fatal);
        }
    };
    cfg.ctrlfile_path = Some(path.clone());

    let raw = read_ctrlfile(env, &path, log)?;
    cfg.names_rawdata |= raw.contains("rawdata");
    // biber checks the NFD of the whole file, then reads a copy where data
    // source lines are not normalised. The two texts are usually the same
    // (ASCII or already-NFD data source names, `\n` line ends), and then
    // one parse serves both.
    let checked = if raw.is_ascii() {
        Cow::Borrowed(raw.as_str())
    } else {
        perl_compat::unicode::nfd(&raw)
    };
    let wf_doc = parse_document(&checked);
    if let Err(e) = &wf_doc {
        let output = args.output_target.unwrap_or("");
        if output != "-" && !output.is_empty() {
            let _ = std::fs::remove_file(env.resolve(output));
        }
        debug!(log, "XML error: {e}");
        biber_error!(
            log,
            "{path} is malformed, last biblatex run probably failed. Deleted {output}"
        )?;
        return Err(Fatal);
    }

    // Biber.pm:375-378 (DC-033).
    if cfg.biber.is_true("validate_control") {
        crate::xmlcheck::validate_biber_xml(
            log,
            env,
            &path,
            crate::xmlcheck::SchemaKind::Control,
            cfg.quirks,
        )?;
    }
    // Biber.pm:380-417.
    if cfg.biber.is_true("convert_control") {
        crate::bcfhtml::convert_control(log, env, &path)?;
    }

    info!(log, "Reading '{path}'");
    let buf = nfd_except_datasources(&raw);
    let reparsed;
    let doc = match wf_doc {
        Ok(ref d) if *checked == *buf => d,
        _ => {
            reparsed = parse_document(&buf);
            match &reparsed {
                Ok(d) => d,
                Err(e) => {
                    biber_error!(log, "{path} is malformed: {e}")?;
                    return Err(Fatal);
                }
            }
        }
    };
    let root = match xmlin_document(doc, bcf_force_array) {
        XVal::Hash(h) => h,
        _ => XHash::default(),
    };
    apply_bcf(cfg, log, root, &path)
}

/// Interpret a parsed `.bcf` tree (everything after `XMLin` in
/// `parse_ctrlfile`). The raw subtrees that are stored as options (data
/// model, inheritance) are moved out of `bcf`.
pub fn apply_bcf(
    cfg: &mut Config,
    log: &mut Logger,
    mut bcf: XHash,
    ctrl_file_path: &str,
) -> Result<Parsed, Fatal> {
    let controlversion = bcf.str("version").unwrap_or("").to_string();
    let bltxversion = bcf.str("bltxversion").unwrap_or("").to_string();
    cfg.blx
        .set_global("controlversion", OptValue::Str(controlversion.clone()));
    if controlversion != BCF_VERSION {
        biber_error!(
            log,
            "Error: Found biblatex control file version {controlversion}, expected version {BCF_VERSION}.\nThis means that your biber ({}) and biblatex ({bltxversion}) versions are incompatible.\nSee compat matrix in biblatex or biber PDF documentation.",
            crate::BIBER_VERSION
        )?;
    }

    // Option scope
    for os in bcf.list("optionscope").hashes() {
        let scope = os.str("type").and_then(Scope::from_name);
        for o in os.list("option").hashes() {
            let Some(scope) = scope else { continue };
            let opt = o.content().unwrap_or("");
            if let Some(w) = cfg.blx.registry.declare(
                scope,
                opt,
                o.str("backendout"),
                o.str("backendin"),
                o.str("datatype"),
                true,
            ) {
                log.biber_warn(w);
            }
        }
    }

    // Options
    for opts in bcf.list("options").hashes() {
        let component = opts.str("component").unwrap_or("");
        let otype = opts.str("type").unwrap_or("");
        if component == "biber" && otype == "global" {
            for o in opts.list("option").hashes() {
                let key = o.hash("key").and_then(|k| k.content()).unwrap_or("");
                if cfg.biber.is_explicit(key) {
                    continue;
                }
                match o.str("type") {
                    Some("singlevalued") => {
                        let v = o
                            .first("value")
                            .and_then(XVal::as_hash)
                            .and_then(|h| h.content())
                            .map(str::to_string);
                        cfg.biber.set(key, BiberValue::from(v));
                    }
                    Some("multivalued") => cfg.biber.set(key, BiberValue::List(multi_contents(o))),
                    _ => {}
                }
            }
        }
        if component == "biblatex" {
            for o in opts.list("option").hashes() {
                let key = o.hash("key").and_then(|k| k.content()).unwrap_or("");
                let val = match o.str("type") {
                    Some("singlevalued") => o
                        .first("value")
                        .and_then(XVal::as_hash)
                        .and_then(|h| h.content())
                        .map_or(OptValue::Undef, |c| OptValue::Str(c.to_string())),
                    Some("multivalued") => OptValue::Multi(multi_hashes(o)),
                    _ => continue,
                };
                if otype == "global" {
                    cfg.blx.set_global(key, val);
                } else {
                    cfg.blx.set_entrytype(otype, key, val);
                }
            }
        }
    }

    // Datafield sets
    for s in bcf.list("datafieldset").hashes() {
        let name = crate::perlish::lc(s.str("name").unwrap_or(""));
        for m in s.list("member").hashes() {
            let member = match m.first_str("field").filter(|f| truthy(Some(f))) {
                Some(f) => DfsMember::Field(f.to_string()),
                None => DfsMember::Types {
                    fieldtype: m.str("fieldtype").map(str::to_string),
                    datatype: m.str("datatype").map(str::to_string),
                },
            };
            cfg.datafield_sets
                .entry(name.clone())
                .or_default()
                .push(member);
        }
    }

    // Source maps
    if let Some(sm) = bcf.get("sourcemap") {
        let docmaps: Vec<SourceMaps> = sm
            .as_hash()
            .map(|h| h.list("maps").hashes().map(SourceMaps::from_xval).collect())
            .unwrap_or_default();
        let mut usms = cfg.biber.sourcemap().to_vec();
        if !usms.is_empty() {
            for m in &mut usms {
                m.level = Some("user".into());
            }
            let user_doc: Vec<&SourceMaps> = docmaps
                .iter()
                .filter(|m| m.level.as_deref().unwrap_or("") == "user")
                .collect();
            if !user_doc.is_empty() {
                let prepend: Vec<_> = user_doc
                    .iter()
                    .flat_map(|m| m.maps.iter().cloned())
                    .collect();
                let first = &mut usms[0];
                let mut maps = prepend;
                maps.append(&mut first.maps);
                first.maps = maps;
            }
            let dm: Vec<SourceMaps> = docmaps
                .iter()
                .filter(|m| matches!(m.level.as_deref(), Some("driver" | "style")))
                .cloned()
                .collect();
            if !dm.is_empty() {
                let mut all = usms;
                all.extend(dm);
                cfg.biber.set("sourcemap", BiberValue::Sourcemap(all));
            } else if cfg.quirks.on(Quirk::ConfSourcemapsDropped) {
                // BB-052: the config file's maps are dropped.
                cfg.biber.set("sourcemap", BiberValue::Sourcemap(docmaps));
            } else {
                cfg.biber.set("sourcemap", BiberValue::Sourcemap(usms));
            }
        } else {
            cfg.biber.set("sourcemap", BiberValue::Sourcemap(docmaps));
        }
    }

    // Labelalpha name templates
    let mut lants = BTreeMap::new();
    for t in bcf.list("labelalphanametemplate").hashes() {
        let (name, parts) = labelalphaname_template(t);
        lants.insert(name, parts);
    }
    cfg.blx.set_global(
        "labelalphanametemplate",
        if lants.is_empty() {
            OptValue::Undef
        } else {
            OptValue::LabelAlphaNameTemplates(lants)
        },
    );

    // Labelalpha templates (stored raw)
    for t in bcf.list("labelalphatemplate").hashes() {
        let latype = t.str("type").unwrap_or("");
        let v = OptValue::Xml(XVal::Hash(t.clone()));
        if latype == "global" {
            cfg.blx.set_global("labelalphatemplate", v);
        } else {
            cfg.blx.set_entrytype(latype, "labelalphatemplate", v);
        }
    }

    // Extradate specification
    let mut ed = Vec::new();
    if let Some(eds) = bcf.hash("extradatespec") {
        for scope in eds.list("scope").hashes() {
            let mut fs: Vec<&XHash> = scope.list("field").hashes().collect();
            sort_by_order(&mut fs, |f| f.str("order"));
            ed.push(
                fs.into_iter()
                    .map(|f| f.content().unwrap_or("").to_string())
                    .collect(),
            );
        }
    }
    cfg.blx.set_global(
        "extradatespec",
        if ed.is_empty() {
            OptValue::Undef
        } else {
            OptValue::ExtradateSpec(ed)
        },
    );

    // Inheritance (raw)
    cfg.blx.set_global(
        "inheritance",
        bcf.remove("inheritance")
            .map_or(OptValue::Undef, OptValue::Xml),
    );

    // noinit / nolabel / nolabelwidthcount / nosort / nonamestring
    for (outer, inner, opt, named) in [
        ("noinits", "noinit", "noinit", false),
        ("nolabels", "nolabel", "nolabel", false),
        (
            "nolabelwidthcounts",
            "nolabelwidthcount",
            "nolabelwidthcount",
            false,
        ),
        ("nosorts", "nosort", "nosort", true),
        ("nonamestrings", "nonamestring", "nonamestring", true),
    ] {
        let mut pats = Vec::new();
        if let Some(o) = bcf.hash(outer) {
            for n in o.list(inner).hashes() {
                pats.push(PatternOpt {
                    name: if named {
                        n.first_str("field").map(str::to_string)
                    } else {
                        None
                    },
                    value: n.first_str("value").map(str::to_string),
                    has_name: named,
                });
            }
        }
        if !pats.is_empty() {
            cfg.biber.set(opt, BiberValue::Patterns(pats));
        }
    }

    // Uniquename templates
    let mut unts = BTreeMap::new();
    let mut checkbase = false;
    let mut all_have_base = true;
    for t in bcf.list("uniquenametemplate").hashes() {
        let (name, parts, has_base) = uniquename_template(t);
        checkbase |= has_base;
        all_have_base &= has_base;
        unts.insert(name, parts);
    }
    let base_ok = if cfg.quirks.on(Quirk::UniquenameBaseCheckGlobal) {
        checkbase
    } else {
        checkbase && all_have_base
    };
    if !base_ok {
        biber_error!(
            log,
            "The uniquenametemplate must contain at least one 'base' part otherwise name disambiguation is impossible"
        )?;
    }
    cfg.blx.set_global(
        "uniquenametemplate",
        if unts.is_empty() {
            OptValue::Undef
        } else {
            OptValue::UniquenameTemplates(unts)
        },
    );

    // Namehash templates
    let mut nhts = BTreeMap::new();
    for t in bcf.list("namehashtemplate").hashes() {
        let (name, parts) = namehash_template(t);
        nhts.insert(name, parts);
    }
    cfg.blx.set_global(
        "namehashtemplate",
        if nhts.is_empty() {
            OptValue::Undef
        } else {
            OptValue::NamehashTemplates(nhts)
        },
    );

    // Sorting name key templates
    let mut snss = BTreeMap::new();
    for t in bcf.list("sortingnamekeytemplate").hashes() {
        let (name, tpl) = sortingnamekey_template(t);
        snss.insert(name, tpl);
    }
    cfg.blx.set_global(
        "sortingnamekeytemplate",
        if snss.is_empty() {
            OptValue::Undef
        } else {
            OptValue::SortingNameKeyTemplates(snss)
        },
    );

    // Transliteration
    for tr in bcf.list("transliteration").hashes() {
        let v = tr
            .get("translit")
            .cloned()
            .map_or(OptValue::Undef, OptValue::Xml);
        match tr.first_str("entrytype") {
            Some("*") => cfg.blx.set_global("translit", v),
            et => cfg.blx.set_entrytype(et.unwrap_or(""), "translit", v),
        }
    }

    // Sort exclusions / inclusions
    for (outer, inner) in [
        ("sortexclusion", "exclusion"),
        ("sortinclusion", "inclusion"),
    ] {
        for sx in bcf.list(outer).hashes() {
            let set: BTreeSet<String> = sx
                .list(inner)
                .hashes()
                .map(|e| e.content().unwrap_or("").to_string())
                .collect();
            let v = if set.is_empty() {
                OptValue::Undef
            } else {
                OptValue::FieldSet(set)
            };
            cfg.blx
                .set_entrytype(sx.str("type").unwrap_or(""), outer, v);
        }
    }

    // Presort
    for p in bcf.list("presort").hashes() {
        let v = p
            .content()
            .map_or(OptValue::Undef, |c| OptValue::Str(c.to_string()));
        if p.contains("type") {
            cfg.blx
                .set_entrytype(p.str("type").unwrap_or(""), "presort", v);
        } else {
            cfg.blx.set_global("presort", v);
        }
    }

    // Sorting templates
    let gl = cfg
        .blx
        .global("sortlocale")
        .and_then(OptValue::as_str)
        .map(str::to_string);
    let mut sts = BTreeMap::new();
    for ss in bcf.list("sortingtemplate").hashes() {
        sts.insert(
            ss.str("name").unwrap_or("").to_string(),
            parse_sort(ss, gl.as_deref()),
        );
    }
    cfg.blx.set_global(
        "sortingtemplate",
        if sts.is_empty() {
            OptValue::Undef
        } else {
            OptValue::SortingTemplates(sts)
        },
    );

    // Data model
    cfg.blx
        .set_global("datamodel", OptValue::Datamodel(Vec::new()));
    cfg.blx.add_datamodel(
        bcf.remove("datamodel")
            .unwrap_or(XVal::Hash(XHash::default())),
    );

    // Data sources
    let input_encoding = cfg.biber.get_str("input_encoding").map(str::to_string);
    let glob_default = cfg.biber.get_str("glob_datasources").map(str::to_string);
    let mut bibdatasources: Vec<(String, Vec<DataSource>)> = Vec::new();
    for data in bcf.list("bibdata").hashes() {
        let sec = data.first_str("section").unwrap_or("").to_string();
        for ds in data.list("datasource").hashes() {
            let d = DataSource {
                type_: ds.str("type").map(str::to_string),
                name: ds.content().map(str::to_string),
                datatype: ds.str("datatype").map(str::to_string),
                encoding: ds
                    .str("encoding")
                    .map(str::to_string)
                    .or_else(|| input_encoding.clone()),
                glob: ds
                    .str("glob")
                    .map(str::to_string)
                    .or_else(|| glob_default.clone()),
            };
            let idx = match bibdatasources.iter().position(|(s, _)| *s == sec) {
                Some(i) => i,
                None => {
                    bibdatasources.push((sec.clone(), Vec::new()));
                    bibdatasources.len() - 1
                }
            };
            let list = &mut bibdatasources[idx].1;
            let e = |x: &Option<String>| x.clone().unwrap_or_default();
            if !list.iter().any(|x| {
                e(&x.type_) == e(&d.type_)
                    && e(&x.datatype) == e(&d.datatype)
                    && e(&x.name) == e(&d.name)
            }) {
                list.push(d);
            }
        }
    }
    if bibdatasources.is_empty() {
        // BB-070: biber_warn then exit 0 without display_end.
        log.biber_warn("No data sources defined!");
        return Ok(Parsed::NoDataSources);
    }
    let ds_for = |sec: &str| {
        bibdatasources
            .iter()
            .find(|(s, _)| s == sec)
            .map(|(_, v)| v.clone())
    };

    let mut doc = Document::default();
    let mut key_flag = false;
    for section in bcf.list("section").hashes() {
        let secnum = section.str("number").unwrap_or("").to_string();
        if doc.section(&secnum).is_none() {
            doc.sections.push(Section::new(&secnum));
        }
        // Pre-process: \cite beats \nocite of the same key.
        let mut prekeys: Vec<XHash> = Vec::new();
        for keyc in section.list("citekey").hashes() {
            let key = nfd(keyc.content().unwrap_or(""));
            let kc = |x: &XHash| nfd(x.content().unwrap_or(""));
            if truthy(keyc.str("nocite")) {
                if !prekeys.iter().any(|p| kc(p) == key) {
                    prekeys.push(keyc.clone());
                }
            } else if prekeys
                .iter()
                .any(|p| kc(p) == key && truthy(p.str("nocite")))
            {
                for p in prekeys.iter_mut() {
                    if kc(p) == key {
                        p.remove("nocite");
                    }
                }
            } else {
                prekeys.push(keyc.clone());
            }
        }
        let bs = doc.section_mut(&secnum).expect("section exists");
        if bs.datasources.is_none() {
            bs.datasources = ds_for(&secnum);
        }
        let mut keys: Vec<String> = Vec::new();
        for keyc in &prekeys {
            let key = nfd(keyc.content().unwrap_or(""));
            if key == "*" {
                bs.allkeys = true;
                cfg.set_keyorder(&secnum, &key, keyc.str("order"));
                if truthy(keyc.str("nocite")) {
                    bs.allkeys_nocite = true;
                }
                key_flag = true;
                *bs.seenkeys.entry(key).or_default() += 1;
            } else if bs.seenkeys.get(&key).copied().unwrap_or(0) == 0 {
                if keyc.str("type") == Some("set") {
                    let members = split_comma(keyc.str("members").unwrap_or(""));
                    bs.dynamic_sets.insert(key.clone(), members);
                    keys.push(key);
                    key_flag = true;
                } else {
                    if truthy(keyc.str("nocite")) {
                        bs.nocite.insert(key.clone());
                    } else {
                        bs.cite.insert(key.clone());
                    }
                    cfg.set_keyorder(&secnum, &key, keyc.str("order"));
                    if let Some(io) = keyc.str("intorder").filter(|i| truthy(Some(i))) {
                        cfg.internal_keyorder
                            .entry(secnum.clone())
                            .or_default()
                            .insert(key.clone(), io.to_string());
                    }
                    keys.push(key.clone());
                    key_flag = true;
                    *bs.seenkeys.entry(key).or_default() += 1;
                }
            }
        }
        for kc in section.list("citekeycount").hashes() {
            let key = nfd(kc.content().unwrap_or(""));
            bs.citecount
                .insert(key, kc.str("count").map(str::to_string));
        }
        if bs.allkeys {
            bs.del_citekeys();
            info!(log, "Using all citekeys in bib section {secnum}");
        } else {
            info!(log, "Found {} citekeys in bib section {secnum}", keys.len());
            let mut sorted = keys.clone();
            sorted.sort();
            debug!(
                log,
                "The citekeys for section {secnum} are: {}\n",
                sorted.join(", ")
            );
            let bs = doc.section_mut(&secnum).expect("section exists");
            bs.add_citekeys(&keys);
        }
    }

    // Data lists
    for list in bcf.list("datalist").hashes() {
        let s = |k: &str| list.str(k).map(str::to_string);
        let stn = s("sortingtemplatename");
        let snktn = s("sortingnamekeytemplatename");
        let untn = s("uniquenametemplatename");
        let lantn = s("labelalphanametemplatename");
        let nhtn = s("namehashtemplatename");
        let lpn = s("labelprefix");
        let lname = s("name");
        let ltype = s("type");
        let o = |x: &Option<String>| x.clone().unwrap_or_default();
        let name = lname
            .clone()
            .filter(|n| truthy(Some(n)))
            .unwrap_or_else(|| {
                format!(
                    "{}/{}/{}/{}/{}/{}",
                    o(&stn),
                    o(&snktn),
                    o(&lpn),
                    o(&untn),
                    o(&lantn),
                    o(&nhtn)
                )
            });
        let mut filters = Vec::new();
        for f in list.list("filter").hashes() {
            filters.push(Filter::One {
                type_: f.str("type").map(str::to_string),
                value: f.content().map(str::to_string),
            });
        }
        for fo in list.list("filteror").hashes() {
            filters.push(Filter::Or(
                fo.list("filter")
                    .hashes()
                    .map(|f| {
                        (
                            f.str("type").map(str::to_string),
                            f.content().map(str::to_string),
                        )
                    })
                    .collect(),
            ));
        }
        let dl = DataList {
            section: list.first_str("section").map(str::to_string),
            name,
            type_: ltype
                .filter(|t| truthy(Some(t)))
                .unwrap_or_else(|| "entry".into()),
            sortinit_locale: Some(sortinit_locale(cfg, stn.as_deref())),
            sortingtemplatename: stn,
            sortingnamekeytemplatename: snktn,
            uniquenametemplatename: untn,
            labelalphanametemplatename: lantn,
            namehashtemplatename: nhtn,
            labelprefix: lpn,
            filters,
        };
        // BB-054: biber's de-duplication never matches.
        if !cfg.quirks.on(Quirk::DuplicateDatalists) && doc.datalists.contains(&dl) {
            continue;
        }
        doc.datalists.push(dl);
    }

    // Every section gets an entry list with the global sorting template.
    let globalss = cfg
        .blx
        .global("sortingtemplatename")
        .and_then(OptValue::as_str)
        .unwrap_or("")
        .to_string();
    for section in bcf.list("section").hashes() {
        let secnum = section.str("number").unwrap_or("").to_string();
        let want = DataList::default_entry(&secnum, &globalss);
        let o = |x: &Option<String>| x.clone().unwrap_or_default();
        let exists = doc.datalists.iter().any(|l| {
            o(&l.section) == secnum
                && l.type_ == "entry"
                && o(&l.sortingtemplatename) == globalss
                && o(&l.sortingnamekeytemplatename) == "global"
                && o(&l.uniquenametemplatename) == "global"
                && o(&l.labelalphanametemplatename) == "global"
                && o(&l.namehashtemplatename) == "global"
                && o(&l.labelprefix).is_empty()
                && l.name == want.name
        });
        if !exists {
            let mut dl = want;
            dl.sortinit_locale = Some(sortinit_locale(cfg, Some(&globalss)));
            doc.datalists.push(dl);
        }
    }

    if !key_flag {
        log.biber_warn(format!(
            "The file '{ctrl_file_path}' does not contain any citations!"
        ));
    }

    cfg.normalise_utf8();

    // bibtex output outside tool mode: a pseudo-section with everything.
    if cfg.biber.output_format() == "bibtex" {
        cfg.biber.set("tool", "1");
        cfg.biber.set("mincrossrefs", "1");
        cfg.biber.set("minxrefs", "1");
        let mut bs = Section::new("99999");
        for s in doc.sorted_sections() {
            if s.allkeys {
                bs.allkeys = true;
            } else {
                bs.add_citekeys(&s.citekeys);
            }
            if let Some(ds) = &s.datasources {
                bs.datasources
                    .get_or_insert_with(Vec::new)
                    .extend(ds.iter().cloned());
            }
        }
        doc.sections.push(bs);
        let mut dl = DataList::default_entry("99999", &globalss);
        dl.sortinit_locale = None;
        doc.datalists.push(dl);
    }

    Ok(Parsed::Document(doc))
}

/// The parse-time parts of `process_setup` (Biber.pm:1178-1232): drop
/// sections for bibtex output, add missing default entry lists, build the
/// data model and resolve datafield sets, force `output_safechars` for
/// ASCII output.
pub fn process_setup(cfg: &mut Config, log: &mut Logger, doc: &mut Document) -> Result<(), Fatal> {
    if cfg.biber.output_format() == "bibtex" {
        doc.sections.retain(|s| s.number == "99999");
    }
    let globalss = cfg
        .blx
        .global("sortingtemplatename")
        .and_then(OptValue::as_str)
        .unwrap_or("")
        .to_string();
    let numbers: Vec<String> = doc
        .sorted_sections()
        .iter()
        .map(|s| s.number.clone())
        .collect();
    for secnum in numbers {
        let n = crate::perlish::numify(&secnum);
        let has = doc.datalists.iter().any(|l| {
            l.type_ == "entry" && crate::perlish::numify(l.section.as_deref().unwrap_or("")) == n
        });
        if !has {
            let mut dl = DataList::default_entry(&secnum, &globalss);
            dl.sortinit_locale = Some(sortinit_locale(cfg, Some(&globalss)));
            doc.datalists.push(dl);
        }
    }

    setup_datamodel(cfg, log)
}

/// The tail of `process_setup` (Biber.pm:1209-1225), which is all of
/// `process_setup_tool` (Biber.pm:1237): build the data model, resolve
/// datafield sets, force `output_safechars` for ASCII output.
pub fn setup_datamodel(cfg: &mut Config, log: &mut Logger) -> Result<(), Fatal> {
    let dms = match cfg.blx.global("datamodel") {
        Some(OptValue::Datamodel(v)) => v.as_slice(),
        _ => &[],
    };
    let (dm, errors) = DataModel::new(dms, cfg.biber.is_true("no_default_datamodel"));
    for e in errors {
        biber_error!(log, "{}", e.0)?;
    }
    cfg.dm = Some(dm);
    cfg.resolve_datafieldsets();

    let ascii = |s: Option<&str>| s.is_some_and(|s| s.to_ascii_lowercase().contains("ascii"));
    if ascii(cfg.biber.output_encoding()) && !ascii(cfg.biber.input_encoding()) {
        cfg.biber.set("output_safechars", "1");
    }
    Ok(())
}

/// A canonical JSON dump of the post-setup configuration state, matching
/// the oracle probe `tools/oracle/probes/bibers-core/dump_config.pl`.
pub fn dump_state(cfg: &Config, doc: Option<&Document>, warnings: &[String]) -> Value {
    let mut m = Map::new();
    let mut biber = cfg.biber_json();
    if let Value::Object(b) = &mut biber {
        // Paths that differ between the probe and the test harness.
        b.remove("bcf");
    }
    m.insert("biber".into(), biber);
    let mut blx = cfg.blx.to_json();
    if let Value::Object(b) = &mut blx
        && let Some(Value::Object(g)) = b.get_mut("GLOBAL")
    {
        // biber mutates the stored data model in place; compared via "dm".
        g.remove("datamodel");
    }
    m.insert("blx".into(), blx);
    m.insert("registry".into(), cfg.blx.registry.to_json());
    m.insert(
        "datafieldsets".into(),
        Value::Object(
            cfg.datafield_sets_resolved
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        Value::Array(v.iter().map(|x| x.clone().into()).collect()),
                    )
                })
                .collect(),
        ),
    );
    let ko = |src: &HashMap<String, HashMap<String, String>>| {
        let mut o = Map::new();
        let mut secs: Vec<&String> = src.keys().collect();
        secs.sort();
        for s in secs {
            let mut keys: Vec<(&String, &String)> = src[s].iter().collect();
            keys.sort();
            o.insert(
                s.clone(),
                Value::Object(
                    keys.into_iter()
                        .map(|(k, v)| (k.clone(), v.clone().into()))
                        .collect(),
                ),
            );
        }
        Value::Object(o)
    };
    m.insert("keyorder".into(), ko(&cfg.keyorder));
    m.insert("internalkeyorder".into(), ko(&cfg.internal_keyorder));
    if let Some(doc) = doc {
        m.insert("doc".into(), doc.to_json());
    }
    if let Some(dm) = &cfg.dm {
        m.insert("dm".into(), dm.to_json());
    }
    m.insert(
        "warnings".into(),
        Value::Array(warnings.iter().map(|w| w.clone().into()).collect()),
    );
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datasource_lines() {
        assert!(is_datasource_line(
            r#"    <bcf:datasource type="file" datatype="bibtex">x.bib</bcf:datasource>"#
        ));
        assert!(!is_datasource_line("<bcf:datasource>"));
        assert!(!is_datasource_line("<bcf:section number=\"0\">"));
    }

    #[test]
    fn line_split() {
        assert_eq!(split_lines_r("a\r\nb\nc\r"), vec!["a", "b", "c", ""]);
    }

    #[test]
    fn force_array() {
        assert!(bcf_force_array("citekey"));
        assert!(bcf_force_array("nonocitekeycount"));
        assert!(!bcf_force_array("noinits"));
        assert!(bcf_force_array("noinit"));
    }
}

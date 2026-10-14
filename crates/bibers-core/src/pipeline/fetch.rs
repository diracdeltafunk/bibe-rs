//! Section initialisation and data fetching: `fetch_data` (Biber.pm:4574),
//! data source parsing (`Biber::Input::file::bibtex`), source maps and field
//! handlers.

pub mod biblatexml;
mod bibtex;
pub mod dates;
pub mod encoding;
pub mod names;

pub use bibtex::{BibCache, FetchOpts};
pub(crate) use bibtex::{collapse_eq, process_entry_options};

use super::Ctx;
use crate::log::Fatal;
use crate::model::{DataList, PerlData, Section, SectionState, Value};
use crate::{biber_error, info};
use serde_json::{Map, Value as J, json};

/// Set up a section's state as biber has it just before `fetch_data`: the
/// `Biber::Section` built while parsing the `.bcf`, its data lists, and
/// `$CONFIG->{state}` as reset by `Biber::Config::_init` (biber's `init`
/// phase dump).
pub fn init_section(ctx: &mut Ctx, secnum: &str) -> Result<SectionState, Fatal> {
    let bs = ctx
        .doc
        .section(secnum)
        .expect("prepare only processes sections from the .bcf");
    let mut section = Section {
        number: bs.number.clone(),
        citekeys: bs.citekeys.clone(),
        citekeys_h: bs.citekeys.iter().cloned().collect(),
        orig_order_citekeys: bs.orig_order_citekeys.clone(),
        allkeys: bs.allkeys,
        allkeys_nocite: bs.allkeys_nocite,
        cite_citekeys: bs.cite.clone(),
        nocite_citekeys: bs.nocite.clone(),
        ..Section::default()
    };
    let empty = || J::Object(Map::new());
    for k in [
        "bcfkeycache",
        "everykey",
        "everykey_lc",
        "keytods",
        "keytorelclone",
        "labelcache_l",
        "labelcache_v",
        "namepartlengths",
        "relclonetokey",
        "relkeys",
        "state",
        "static_keys",
    ] {
        section.slots.insert(k.into(), empty());
    }
    section.slots.insert(
        "dkeys".into(),
        J::Object(
            bs.dynamic_sets
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect(),
        ),
    );
    section.slots.insert(
        "citecount".into(),
        J::Object(
            bs.citecount
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().map_or(J::Null, J::from)))
                .collect(),
        ),
    );
    section.slots.insert(
        "seenkeys".into(),
        J::Object(
            bs.seenkeys
                .iter()
                .map(|(k, v)| (k.clone(), J::from(v.to_string())))
                .collect(),
        ),
    );
    if let Some(ds) = &bs.datasources {
        section.slots.insert(
            "datasources".into(),
            J::Array(
                ds.iter()
                    .map(|d| {
                        let mut o = Map::new();
                        for (k, v) in [
                            ("type", &d.type_),
                            ("name", &d.name),
                            ("datatype", &d.datatype),
                            ("encoding", &d.encoding),
                            ("glob", &d.glob),
                        ] {
                            if let Some(v) = v {
                                o.insert(k.into(), J::from(v.clone()));
                            }
                        }
                        J::Object(o)
                    })
                    .collect(),
            ),
        );
    }

    let datalists = ctx
        .doc
        .lists_for_section(secnum)
        .into_iter()
        .map(datalist_from_bcf)
        .collect();

    Ok(SectionState {
        section,
        datalists,
        config_state: initial_config_state(ctx),
        entry_options: Default::default(),
        // Carried over from the previous section by `prepare`.
        preamble: None,
        // No entries yet, so no annotations.
        annotations: Some(serde_json::Value::Object(Default::default())),
        shared_lists: Some(Vec::new()),
    })
}

fn datalist_from_bcf(d: &crate::bcf::DataList) -> DataList {
    let o = |x: &Option<String>| x.clone().unwrap_or_default();
    let mut dl = DataList {
        name: d.name.clone(),
        type_: d.type_.clone(),
        section: o(&d.section),
        sortingtemplatename: o(&d.sortingtemplatename),
        sortingnamekeytemplatename: o(&d.sortingnamekeytemplatename),
        uniquenametemplatename: o(&d.uniquenametemplatename),
        labelalphanametemplatename: o(&d.labelalphanametemplatename),
        namehashtemplatename: o(&d.namehashtemplatename),
        labelprefix: o(&d.labelprefix),
        ..DataList::default()
    };
    if !d.filters.is_empty() {
        let f = d
            .filters
            .iter()
            .map(|f| match f {
                crate::bcf::Filter::One { type_, value } => filter_json(type_, value),
                crate::bcf::Filter::Or(alts) => {
                    J::Array(alts.iter().map(|(t, v)| filter_json(t, v)).collect())
                }
            })
            .collect();
        dl.slots.insert("filters".into(), J::Array(f));
    }
    if d.sortinit_locale.is_some() {
        // An opaque Unicode::Collate::Locale object in biber's dump.
        dl.slots.insert(
            "sortinitcollator".into(),
            json!({"__class": "Unicode::Collate::Locale"}),
        );
    }
    dl
}

fn filter_json(t: &Option<String>, v: &Option<String>) -> PerlData {
    let mut o = Map::new();
    o.insert("type".into(), t.clone().map_or(J::Null, J::from));
    o.insert("value".into(), v.clone().map_or(J::Null, J::from));
    J::Object(o)
}

/// `$CONFIG->{state}` after `Biber::Config::_init` (Config.pm:66-112).
fn initial_config_state(ctx: &Ctx) -> PerlData {
    let orders =
        |m: &std::collections::HashMap<String, std::collections::HashMap<String, String>>| {
            J::Object(
                m.iter()
                    .map(|(s, keys)| {
                        (
                            s.clone(),
                            J::Object(
                                keys.iter()
                                    .map(|(k, v)| (k.clone(), J::from(v.clone())))
                                    .collect(),
                            ),
                        )
                    })
                    .collect(),
            )
        };
    json!({
        "citkey_aliases": {},
        "control_file_location": "",
        "crossref": [],
        "crossrefkeys": {},
        "datafiles": [],
        "graph": {},
        "internalkeyorder": orders(&ctx.cfg.internal_keyorder),
        "keyorder": orders(&ctx.cfg.keyorder),
        "uniqignore": {},
        "xdata": [],
        "xrefkeys": {},
    })
}

/// `Biber::Section` methods used while fetching data, over the typed
/// fields and the `slots` of [`Section`].
pub trait SecExt {
    fn slot_obj(&mut self, name: &str) -> &mut Map<String, J>;
    fn slot_get(&self, name: &str, key: &str) -> Option<&J>;
    /// `set_keytods`.
    fn set_keytods(&mut self, key: &str, ds: &str);
    /// `get_keytods`.
    fn get_keytods(&self, key: &str) -> Option<String>;
    /// `has_everykey`.
    fn has_everykey(&self, key: &str) -> bool;
    /// `add_everykey`.
    fn add_everykey(&mut self, key: &str);
    /// `has_badcasekey`.
    fn has_badcasekey(&self, key: &str) -> Option<String>;
    /// `has_citekey` (understands aliases).
    fn has_citekey_alias(&self, key: &str) -> bool;
    /// `del_citekey`.
    fn del_citekey(&mut self, key: &str);
    /// `add_citekeys`.
    fn add_citekeys(&mut self, keys: &[String]);
    /// `dynamic_set_keys` (sorted; callers needing biber's hash order use .bcf order, DC-015).
    fn dynamic_set_keys(&self) -> Vec<String>;
    /// `get_dynamic_set`.
    fn get_dynamic_set(&self, key: &str) -> Vec<String>;
    /// `set_dynamic_set`.
    fn set_dynamic_set(&mut self, key: &str, members: Vec<String>);
    /// `set_np_length`.
    fn set_np_length(&mut self, np: &str, len: usize);
    /// `is_related`.
    fn is_related(&self, key: &str) -> bool;
    /// `add_related`.
    fn add_related(&mut self, key: &str);
}

impl SecExt for Section {
    fn slot_obj(&mut self, name: &str) -> &mut Map<String, J> {
        // (no key allocation when the slot exists, the common case)
        if !self.slots.get(name).is_some_and(J::is_object) {
            self.slots.insert(name.to_string(), J::Object(Map::new()));
        }
        self.slots
            .get_mut(name)
            .and_then(J::as_object_mut)
            .expect("object")
    }

    fn slot_get(&self, name: &str, key: &str) -> Option<&J> {
        self.slots.get(name)?.as_object()?.get(key)
    }

    fn set_keytods(&mut self, key: &str, ds: &str) {
        self.slot_obj("keytods").insert(key.into(), J::from(ds));
    }

    fn get_keytods(&self, key: &str) -> Option<String> {
        self.slot_get("keytods", key)
            .and_then(J::as_str)
            .map(str::to_string)
    }

    fn has_everykey(&self, key: &str) -> bool {
        self.slot_get("everykey", key)
            .and_then(J::as_str)
            .is_some_and(perl_compat::perl_truthy)
    }

    fn add_everykey(&mut self, key: &str) {
        self.slot_obj("everykey").insert(key.into(), J::from("1"));
        self.slot_obj("everykey_lc")
            .insert(crate::perlish::lc(key), J::from(key));
    }

    fn has_badcasekey(&self, key: &str) -> Option<String> {
        let ck = self
            .slot_get("everykey_lc", &crate::perlish::lc(key))
            .and_then(J::as_str)?;
        (perl_compat::perl_truthy(ck) && ck != key).then(|| ck.to_string())
    }

    fn has_citekey_alias(&self, key: &str) -> bool {
        let k = self
            .citekey_alias
            .get(key)
            .filter(|r| perl_compat::perl_truthy(r))
            .map_or(key, String::as_str);
        self.citekeys_h.contains(k)
    }

    fn del_citekey(&mut self, key: &str) {
        if !self.has_citekey_alias(key) {
            return;
        }
        self.citekeys.retain(|k| k != key);
        self.orig_order_citekeys.retain(|k| k != key);
        self.citekeys_h.remove(key);
    }

    fn add_citekeys(&mut self, keys: &[String]) {
        for k in keys {
            if self.has_citekey_alias(k) {
                continue;
            }
            self.citekeys_h.insert(k.clone());
            self.citekeys.push(k.clone());
            self.orig_order_citekeys.push(k.clone());
        }
    }

    fn dynamic_set_keys(&self) -> Vec<String> {
        match self.slots.get("dkeys") {
            Some(J::Object(m)) => m.keys().cloned().collect(),
            _ => Vec::new(),
        }
    }

    fn get_dynamic_set(&self, key: &str) -> Vec<String> {
        match self.slot_get("dkeys", key) {
            Some(J::Array(a)) => a
                .iter()
                .map(|v| v.as_str().unwrap_or("").to_string())
                .collect(),
            _ => Vec::new(),
        }
    }

    fn set_dynamic_set(&mut self, key: &str, members: Vec<String>) {
        self.slot_obj("dkeys").insert(
            key.into(),
            J::Array(members.into_iter().map(J::from).collect()),
        );
    }

    fn set_np_length(&mut self, np: &str, len: usize) {
        let m = self.slot_obj("namepartlengths");
        let cur = m
            .get(np)
            .and_then(J::as_str)
            .map_or(0.0, perl_compat::truth::numify);
        if len as f64 > cur {
            m.insert(np.into(), J::from(len.to_string()));
        }
    }

    fn is_related(&self, key: &str) -> bool {
        self.slot_get("relkeys", key)
            .and_then(J::as_str)
            .is_some_and(perl_compat::perl_truthy)
    }

    fn add_related(&mut self, key: &str) {
        self.slot_obj("relkeys").insert(key.into(), J::from("1"));
    }
}

/// One data source of a section (after globbing).
#[derive(Clone, Debug)]
struct Datasource {
    type_: Option<String>,
    name: Option<String>,
    datatype: Option<String>,
    encoding: Option<String>,
    glob: Option<String>,
}

fn datasources(st: &SectionState) -> Option<Vec<Datasource>> {
    let J::Array(a) = st.section.slots.get("datasources")? else {
        return None;
    };
    Some(
        a.iter()
            .map(|d| {
                let g = |k: &str| d.get(k).and_then(J::as_str).map(str::to_string);
                Datasource {
                    type_: g("type"),
                    name: g("name"),
                    datatype: g("datatype"),
                    encoding: g("encoding"),
                    glob: g("glob"),
                }
            })
            .collect(),
    )
}

/// `_bool_norm` (Utils.pm:1986).
fn bool_norm(b: Option<&str>) -> bool {
    let Some(b) = b.filter(|b| perl_compat::perl_truthy(b)) else {
        return false;
    };
    let l = b.to_ascii_lowercase();
    l.contains("true") || l.contains('1')
}

/// Match one path component against a glob pattern (`*`, `?`, `[...]`).
fn glob_match(pat: &[char], s: &[char]) -> bool {
    match pat.first() {
        None => s.is_empty(),
        Some('*') => (0..=s.len()).any(|i| glob_match(&pat[1..], &s[i..])),
        Some('?') => !s.is_empty() && glob_match(&pat[1..], &s[1..]),
        Some('[') => {
            let Some(end) = pat.iter().skip(2).position(|&c| c == ']').map(|p| p + 2) else {
                return !s.is_empty() && s[0] == '[' && glob_match(&pat[1..], &s[1..]);
            };
            if s.is_empty() {
                return false;
            }
            let (neg, set) = match pat[1] {
                '!' | '^' => (true, &pat[2..end]),
                _ => (false, &pat[1..end]),
            };
            let mut hit = false;
            let mut i = 0;
            while i < set.len() {
                if i + 2 < set.len() && set[i + 1] == '-' {
                    hit |= set[i] <= s[0] && s[0] <= set[i + 2];
                    i += 3;
                } else {
                    hit |= set[i] == s[0];
                    i += 1;
                }
            }
            hit != neg && glob_match(&pat[end + 1..], &s[1..])
        }
        Some('\\') if pat.len() > 1 => {
            !s.is_empty() && s[0] == pat[1] && glob_match(&pat[2..], &s[1..])
        }
        Some(c) => !s.is_empty() && s[0] == *c && glob_match(&pat[1..], &s[1..]),
    }
}

/// Expand `{a,b}` alternatives (GLOB_BRACE).
fn brace_expand(p: &str) -> Vec<String> {
    let Some(open) = p.find('{') else {
        return vec![p.to_string()];
    };
    let mut depth = 0;
    let mut close = None;
    let mut commas = Vec::new();
    for (i, c) in p[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            ',' if depth == 1 => commas.push(open + i),
            _ => {}
        }
    }
    let Some(close) = close else {
        return vec![p.to_string()];
    };
    let mut bounds = vec![open];
    bounds.extend(commas);
    bounds.push(close);
    let mut out = Vec::new();
    for w in bounds.windows(2) {
        let alt = &p[w[0] + 1..w[1]];
        out.extend(brace_expand(&format!(
            "{}{}{}",
            &p[..open],
            alt,
            &p[close + 1..]
        )));
    }
    out
}

/// Perl's `glob` (File::Glob `csh_glob`) on one pattern, relative to `cwd`.
fn glob1(env: &crate::env::Env, pattern: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in pattern.split_whitespace() {
        for p in brace_expand(word) {
            if !p.contains(['*', '?', '[']) {
                out.push(p);
                continue;
            }
            let abs = p.starts_with('/');
            let comps: Vec<&str> = p.split('/').filter(|c| !c.is_empty()).collect();
            let mut cur: Vec<String> = vec![if abs { "/".into() } else { String::new() }];
            for (ci, comp) in comps.iter().enumerate() {
                let last = ci + 1 == comps.len();
                let mut next = Vec::new();
                for base in &cur {
                    let join = |n: &str| {
                        if base.is_empty() {
                            n.to_string()
                        } else if base.ends_with('/') {
                            format!("{base}{n}")
                        } else {
                            format!("{base}/{n}")
                        }
                    };
                    if !comp.contains(['*', '?', '[']) {
                        next.push(join(comp));
                        continue;
                    }
                    let dir = env.resolve(if base.is_empty() { "." } else { base });
                    let Ok(rd) = std::fs::read_dir(dir) else {
                        continue;
                    };
                    let pc: Vec<char> = comp.chars().collect();
                    let mut names: Vec<String> = rd
                        .filter_map(|e| e.ok()?.file_name().into_string().ok())
                        .filter(|n| !n.starts_with('.') || comp.starts_with('.'))
                        .filter(|n| glob_match(&pc, &n.chars().collect::<Vec<_>>()))
                        .collect();
                    names.sort_by(|a, b| {
                        a.to_lowercase()
                            .cmp(&b.to_lowercase())
                            .then_with(|| a.cmp(b))
                    });
                    next.extend(names.iter().map(|n| join(n)));
                }
                cur = next;
                if !last {
                    cur.retain(|d| env.resolve(d).is_dir());
                }
            }
            out.extend(cur.into_iter().filter(|f| env.exists(f)));
        }
    }
    out
}

/// `glob_data_file` (Utils.pm:127).
fn glob_data_file(ctx: &mut Ctx, source: &str, globflag: Option<&str>) -> Vec<String> {
    if crate::files::is_remote(source) || !bool_norm(globflag) {
        return vec![source.to_string()];
    }
    info!(ctx.log, "Globbing data source '{source}'");
    // globU: raw, then NFC and NFD variants of the pattern if they differ
    let mut sources = glob1(ctx.env, source);
    let c = crate::perlish::nfc(source);
    if c != source {
        sources.extend(glob1(ctx.env, &c));
    }
    let d = crate::perlish::nfd(source);
    if d != source {
        sources.extend(glob1(ctx.env, &d));
    }
    info!(
        ctx.log,
        "Globbed data source '{source}' to '{}'",
        sources.join(",")
    );
    sources
}

/// `locate_data_file` (Utils.pm:226). `Ok(None)` after a non-fatal error.
fn locate(ctx: &mut Ctx, name: &str) -> Result<Option<String>, Fatal> {
    let fetched;
    let name = if let Some(tls) = crate::remote::remote_scheme(name) {
        fetched = fetch_remote(ctx, name, tls)?;
        fetched.as_str()
    } else {
        name
    };
    let lookup = crate::files::Lookup {
        input_directory: ctx.cfg.biber.get_str("input_directory"),
        output_directory: ctx.cfg.biber.get_str("output_directory"),
        ctrlfile_path: ctx.cfg.ctrlfile_path.as_deref(),
    };
    match crate::files::locate_data_file(ctx.env, &lookup, name) {
        Some(f) => Ok(Some(f)),
        None => {
            biber_error!(ctx.log, "Cannot find '{name}'!")?;
            Ok(None)
        }
    }
}

/// The remote branch of `locate_data_file` (see [`crate::remote::locate`]).
fn fetch_remote(ctx: &mut Ctx, source: &str, tls: bool) -> Result<String, Fatal> {
    crate::remote::locate(ctx.cfg, ctx.log, source, tls)
}

/// Check a data source's type/datatype is supported. `Ok(false)` to skip it.
fn check_datasource(ctx: &mut Ctx, ds: &Datasource) -> Result<bool, Fatal> {
    let t = ds.type_.as_deref().unwrap_or("");
    let dt = ds.datatype.as_deref().unwrap_or("");
    if t != "file" || (dt != "bibtex" && dt != "biblatexml") {
        biber_error!(
            ctx.log,
            "Error loading data source package 'Biber::Input::{t}::{dt}' for '{dt}' '{t}' datasource. Valid type/datatypes are: file/biblatexml,file/bibtex"
        )?;
        return Ok(false);
    }
    Ok(true)
}

/// Read one data source looking for `keys`; returns the keys not found.
fn read_datasource(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    ds: &Datasource,
    keys: &[String],
) -> Result<Vec<String>, Fatal> {
    let name = ds.name.clone().unwrap_or_default();
    let Some(file) = locate(ctx, &name)? else {
        ctx.log
            .biber_warn("Data source '' is empty, ignoring".to_string());
        return Ok(keys.to_vec());
    };
    if ds.datatype.as_deref() == Some("biblatexml") {
        // the encoding is ignored: XML is always read as UTF-8
        return biblatexml::extract_entries(ctx, st, opts, &file, keys);
    }
    let enc = ds.encoding.clone().unwrap_or_else(|| "UTF-8".into());
    bibtex::extract_entries(ctx, st, opts, &file, &enc, keys)
}

/// `Biber::fetch_data` (Biber.pm:4574): read every data source of the
/// section, create entries for the cited keys (or all keys), and collect
/// dependent keys (`get_dependents`).
pub fn fetch_data(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    let secnum = st.section.number.clone();
    let opts = FetchOpts::new(ctx);
    let dkeys = st.section.dynamic_set_keys();
    let citekeys = perl_compat::sort::reduce_array(&st.section.citekeys, &dkeys);

    if ctx.cfg.biber.is_true("clrmacros") {
        ctx.run.bt.macros.clear();
    }
    if !opts.nostdmacros {
        ctx.run.bt.macros.install_biber_months();
    }
    let mut remaining = citekeys;

    // Datasource globbing
    let mut dss: Vec<Datasource> = Vec::new();
    for d in datasources(st).unwrap_or_default() {
        if d.type_.as_deref() != Some("file") {
            dss.push(d.clone());
        }
        for g in glob_data_file(ctx, d.name.as_deref().unwrap_or(""), d.glob.as_deref()) {
            dss.push(Datasource {
                name: Some(g),
                glob: None,
                ..d.clone()
            });
        }
    }
    if st.section.slots.contains_key("datasources") {
        let arr = dss
            .iter()
            .map(|d| {
                let mut o = Map::new();
                let v = |x: &Option<String>| x.clone().map_or(J::Null, J::from);
                o.insert("type".into(), v(&d.type_));
                o.insert("name".into(), v(&d.name));
                o.insert("datatype".into(), v(&d.datatype));
                o.insert("encoding".into(), v(&d.encoding));
                J::Object(o)
            })
            .collect();
        st.section.slots.insert("datasources".into(), J::Array(arr));
    }

    for ds in &dss {
        if remaining.is_empty() && !st.section.allkeys {
            break;
        }
        if ds.datatype.as_deref() == Some("biblatexml") {
            biblatexml_schema(ctx, ds.name.as_deref().unwrap_or(""))?;
        }
        if !check_datasource(ctx, ds)? {
            continue;
        }
        // Slightly different message for tool mode (Biber.pm:4676).
        let sec = if ctx.cfg.biber.tool() {
            String::new()
        } else {
            format!(" for section {secnum}")
        };
        info!(
            ctx.log,
            "Looking for {} {} '{}'{sec}",
            ds.datatype.as_deref().unwrap_or(""),
            ds.type_.as_deref().unwrap_or(""),
            ds.name.as_deref().unwrap_or("")
        );
        remaining = read_datasource(ctx, st, &opts, ds, &remaining)?;
    }

    for k in &remaining {
        ctx.log.biber_warn(format!(
            "I didn't find a database entry for '{k}' (section {secnum})"
        ));
        st.section.del_citekey(k);
        st.section.undef_citekeys.push(k.clone());
    }

    let keys = st.section.citekeys.clone();
    let mut kwd = Vec::new();
    get_dependents(ctx, st, &opts, &dss, keys, &mut kwd)?;
    super::sync_annotations(ctx, st);
    Ok(())
}

/// Biber.pm:4629-4646: write the bltxml RNG schema (unless
/// `--no-bltxml-schema`, once per run) and, with `--validate-bltxml`,
/// validate the data source `name` against it (biber validates against the
/// schema path *before* `--output-directory` is applied, so with an output
/// directory the schema is not found and validation is skipped with a
/// warning).
fn biblatexml_schema(ctx: &mut Ctx, name: &str) -> Result<(), Fatal> {
    let outfile = if ctx.cfg.biber.is_true("tool") {
        // tool mode: next to the data source (DC-010: never the input itself)
        let dsn = ctx.cfg.biber.get_str("dsn").unwrap_or("");
        crate::output::biblatexml::schema_file_for_dsn(dsn)
    } else {
        let bcf = ctx.cfg.biber.get_str("bcf").unwrap_or("").to_string();
        biblatexml::schema_file(&bcf)
    };
    if !ctx.cfg.biber.is_true("no_bltxml_schema") {
        write_bltxml_schema(ctx, &outfile)?;
    }
    if ctx.cfg.biber.is_true("validate_bltxml") {
        crate::xmlcheck::validate_biber_xml(
            ctx.log,
            ctx.env,
            name,
            crate::xmlcheck::SchemaKind::Bltxml(&outfile),
            ctx.cfg.quirks,
        )?;
    }
    Ok(())
}

/// `generate_bltxml_schema($outfile)` (DataModel.pm:1021): written into
/// `--output-directory` if given, once per run.
pub fn write_bltxml_schema(ctx: &mut Ctx, outfile: &str) -> Result<(), Fatal> {
    if ctx.run.bltxml.schema_done {
        return Ok(());
    }
    let mut outfile = outfile.to_string();
    if let Some(outdir) = ctx
        .cfg
        .biber
        .get_str("output_directory")
        .filter(|d| perl_compat::perl_truthy(d))
    {
        outfile = crate::env::catfile(outdir, crate::env::basename(&outfile));
    }
    info!(
        ctx.log,
        "Writing BibLaTeXML RNG schema '{outfile}' for datamodel"
    );
    let rng = crate::bltxml_schema::generate_bltxml_schema(ctx.dm);
    if let Err(e) = std::fs::write(ctx.env.resolve(&outfile), rng) {
        return Err(biblatexml::die(
            ctx,
            &format!("Cannot write BibLaTeXML RNG schema '{outfile}': {e}"),
        ));
    }
    ctx.run.bltxml.schema_done = true;
    Ok(())
}

fn push_uniq(v: &mut Vec<String>, k: &str) {
    if !v.iter().any(|x| x == k) {
        v.push(k.to_string());
    }
}

fn field_list(e: &crate::model::Entry, f: &str) -> Option<Vec<String>> {
    match e.get_field(f)? {
        Value::List(l) => Some(l.clone()),
        Value::Str(s) => Some(vec![s.clone()]),
        _ => Some(Vec::new()),
    }
}

/// `get_dependents` (Biber.pm:4723).
fn get_dependents(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    dss: &[Datasource],
    keys: Vec<String>,
    keyswithdeps: &mut Vec<String>,
) -> Result<(), Fatal> {
    let mut new_deps: Vec<String> = Vec::new();
    let sec = &mut st.section;
    for citekey in &keys {
        if let Some(real) = sec
            .citekey_alias
            .get(citekey)
            .filter(|r| perl_compat::perl_truthy(r))
            .cloned()
        {
            new_deps.push(real.clone());
            push_uniq(keyswithdeps, &real);
            continue;
        }
        let dmems = sec.get_dynamic_set(citekey);
        if !dmems.is_empty() {
            for dm in &dmems {
                if !sec.entries.contains_key(dm) {
                    new_deps.push(dm.clone());
                    push_uniq(keyswithdeps, citekey);
                }
            }
            continue;
        }
        let Some(be) = sec.entries.get(citekey) else {
            continue;
        };
        if let Some(J::Array(refs)) = be.slots.get("xdatarefs") {
            for r in refs {
                if let Some(J::Array(xes)) = r.get("xdataentries") {
                    for x in xes {
                        let x = x.as_str().unwrap_or("");
                        if !sec.entries.contains_key(x) {
                            new_deps.push(x.to_string());
                        }
                        push_uniq(keyswithdeps, citekey);
                    }
                }
            }
        }
        for f in ["xref", "crossref"] {
            if let Some(r) = be.get_str(f).filter(|r| perl_compat::perl_truthy(r)) {
                if !sec.entries.contains_key(r) {
                    new_deps.push(r.to_string());
                }
                push_uniq(keyswithdeps, citekey);
            }
        }
        let mut related_new = Vec::new();
        if be.entrytype() == "set" {
            for sm in field_list(be, "entryset").unwrap_or_default() {
                if !sec.has_citekey_alias(&sm) {
                    new_deps.push(sm);
                    push_uniq(keyswithdeps, citekey);
                }
            }
        }
        if let Some(rel) = field_list(be, "related") {
            for rm in rel {
                if !(sec.has_citekey_alias(&rm) || sec.is_related(&rm) || related_new.contains(&rm))
                {
                    related_new.push(rm.clone());
                    new_deps.push(rm);
                }
                push_uniq(keyswithdeps, citekey);
            }
        }
        for rm in related_new {
            sec.add_related(&rm);
        }
    }
    // uniq
    let mut seen = std::collections::HashSet::new();
    new_deps.retain(|k| seen.insert(k.clone()));

    let mut missing: Vec<String> = Vec::new();
    if !new_deps.is_empty() {
        if st.section.allkeys {
            for dk in &new_deps {
                if !st.section.has_citekey_alias(dk) {
                    missing.push(dk.clone());
                }
            }
        } else {
            missing = new_deps.clone();
            for ds in dss {
                if missing.is_empty() {
                    break;
                }
                if !check_datasource(ctx, ds)? {
                    continue;
                }
                missing = read_datasource(ctx, st, opts, ds, &missing)?;
            }
        }
        new_deps.retain(|k| !missing.contains(k));
    }
    if !new_deps.is_empty() {
        get_dependents(ctx, st, opts, dss, new_deps, keyswithdeps)?;
    }
    for kwd in keyswithdeps.clone() {
        for mk in &missing {
            remove_undef_dependent(ctx, st, &kwd, mk);
        }
    }
    Ok(())
}

/// `remove_undef_dependent` (Biber.pm:4893).
fn remove_undef_dependent(ctx: &mut Ctx, st: &mut SectionState, citekey: &str, missing: &str) {
    let secnum = st.section.number.clone();
    let noremove = ctx.cfg.biber.is_true("tool_noremove_missing_dependants");
    let sec = &mut st.section;
    let dmems = sec.get_dynamic_set(citekey);
    if !dmems.is_empty() {
        if dmems.iter().any(|m| m == missing) {
            let keep = dmems.into_iter().filter(|m| m != missing).collect();
            sec.set_dynamic_set(citekey, keep);
            ctx.log.biber_warn(format!(
                "I didn't find a database entry for dynamic set member '{missing}' - ignoring (section {secnum})"
            ));
        }
        return;
    }
    let Some(be) = sec.entries.get_mut(citekey) else {
        return;
    };
    for f in ["xref", "crossref"] {
        if be
            .get_str(f)
            .is_some_and(|v| perl_compat::perl_truthy(v) && v == missing)
        {
            ctx.log.biber_warn(format!(
                "I didn't find a database entry for {f} '{missing}' in entry '{citekey}' - ignoring (section {secnum})"
            ));
            if !noremove {
                be.del_field(f);
            }
        }
    }
    if let Some(xd) = field_list(be, "xdata") {
        if xd.iter().any(|x| x == missing) {
            ctx.log.biber_warn(format!(
                "I didn't find a database entry for xdata entry '{missing}' in entry '{citekey}' - ignoring (section {secnum})"
            ));
        }
        if !noremove {
            let keep = xd.into_iter().filter(|x| x != missing).collect();
            be.set_datafield("xdata", Some(Value::List(keep)));
        }
    }
    if be.entrytype() == "set"
        && let Some(sm) = field_list(be, "entryset")
        && sm.iter().any(|x| x == missing)
    {
        let keep = sm.into_iter().filter(|x| x != missing).collect();
        be.set_datafield("entryset", Some(Value::List(keep)));
        ctx.log.biber_warn(format!(
            "I didn't find a database entry for static set member '{missing}' in entry '{citekey}' - ignoring (section {secnum})"
        ));
    }
    if let Some(rel) = field_list(be, "related")
        && rel.iter().any(|x| x == missing)
    {
        let keep: Vec<String> = rel.into_iter().filter(|x| x != missing).collect();
        let empty = keep.is_empty();
        be.set_datafield("related", Some(Value::List(keep)));
        if empty {
            be.del_field("relatedtype");
            be.del_field("relatedstring");
        }
        ctx.log.biber_warn(format!(
            "I didn't find a database entry for related entry '{missing}' in entry '{citekey}' - ignoring (section {secnum})"
        ));
    }
}

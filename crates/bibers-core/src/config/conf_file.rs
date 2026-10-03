//! biber.conf (and biber-tool.conf): discovery (`config_file`,
//! Config.pm:613-662) and application (`_config_file_set`,
//! Config.pm:357-583).

use std::collections::BTreeMap;

use crate::bcf::sourcemap::SourceMaps;
use crate::bcf::templates::{
    labelalphaname_template, namehash_template, parse_sort, sortingnamekey_template,
    uniquename_template,
};
use crate::config::biber_opts::{BiberValue, PatternOpt};
use crate::config::blx::{OptValue, Scope};
use crate::config::{Config, DfsMember};
use crate::env::{Env, catfile};
use crate::perlish::{lc, nfd};
use crate::quirks::Quirk;
use crate::xmlin::{XHash, XVal, xmlin};

/// The config file name.
pub const BIBER_CONF_NAME: &str = "biber.conf";

/// `config_file()`: find the user's biber.conf.
pub fn config_file(env: &Env) -> Option<String> {
    let name = BIBER_CONF_NAME;
    let abs = |p: &str| {
        std::fs::canonicalize(env.resolve(p))
            .map(|p| p.to_string_lossy().into_owned())
            .ok()
    };
    if env.is_file(name) {
        return abs(name);
    }
    let dotname = format!(".{name}");
    if env.is_file(&dotname) {
        return abs(&dotname);
    }
    let home = env.home.clone().unwrap_or_default();
    let p = catfile(&home, &dotname);
    if env.is_file(&p) {
        return Some(p);
    }
    if let Some(xdg) = &env.xdg_config_home {
        let p = catfile(&catfile(xdg, "biber"), name);
        if env.is_file(&p) {
            return Some(p);
        }
    }
    let p = catfile(&catfile(&catfile(&home, ".config"), "biber"), name);
    if env.is_file(&p) {
        return Some(p);
    }
    if cfg!(target_os = "macos") {
        let p = catfile(&catfile(&catfile(&home, "Library"), "biber"), name);
        if env.is_file(&p) {
            return Some(p);
        }
    }
    if cfg!(windows)
        && let Some(appdata) = &env.appdata
    {
        let p = catfile(&catfile(appdata, "biber"), name);
        if env.is_file(&p) {
            return Some(p);
        }
    }
    env.kpsewhich(name)
}

/// The `ForceArray` list `_config_file_set` uses (Config.pm:363-403).
pub fn conf_force_array(n: &str) -> bool {
    matches!(
        n,
        "option"
            | "maps"
            | "map"
            | "map_step"
            | "per_type"
            | "per_nottype"
            | "per_datasource"
            | "type_pair"
            | "inherit"
            | "fieldor"
            | "fieldxor"
            | "field"
            | "alias"
            | "keypart"
            | "part"
            | "member"
            | "noinit"
            | "nolabel"
            | "alsoset"
            | "constraints"
            | "constraint"
            | "entrytype"
            | "entryfields"
            | "datetype"
            | "datafieldset"
            | "condition"
            | "filter"
            | "orfilter"
            | "sortexclusion"
            | "exclusion"
            | "sortingtemplate"
            | "constant"
            | "sort"
            | "labelalphatemplate"
            | "labelalphanametemplate"
            | "sortitem"
            | "uniquenametemplate"
            | "namehashtemplate"
            | "presort"
            | "optionscope"
            | "sortingnamekeytemplate"
    )
}

/// An error reading a config file (biber dies with a Perl error here,
/// before logging is set up).
#[derive(Debug, Clone)]
pub struct ConfError(pub String);

/// Read and parse a config file into its `XMLin` tree.
pub fn read_conf(env: &Env, path: &str) -> Result<XHash, ConfError> {
    let bytes = std::fs::read(env.resolve(path))
        .map_err(|e| ConfError(format!("Failed to read biber config file '{path}'\n {e}")))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        ConfError(format!(
            "Failed to read biber config file '{path}'\n file is not valid UTF-8"
        ))
    })?;
    let buf = nfd(&text);
    match xmlin(&buf, conf_force_array) {
        Ok(XVal::Hash(h)) => Ok(h),
        Ok(_) => Ok(XHash::default()),
        Err(e) => Err(ConfError(format!(
            "Failed to read biber config file '{path}'\n {e}"
        ))),
    }
}

/// Messages produced while applying a config file (biber `carp`s these to
/// STDERR).
pub type ConfNotes = Vec<String>;

/// `_config_file_set`: apply a parsed config file to `cfg`.
pub fn apply_conf(cfg: &mut Config, conf: &XHash) -> ConfNotes {
    let mut notes = Vec::new();
    // Option scope has to be set first. biber.conf does not record datatypes.
    for os in conf.list("optionscope").hashes() {
        let Some(scope) = os.str("type").and_then(Scope::from_name) else {
            continue;
        };
        for o in os.list("option").hashes() {
            let Some(opt) = o.content() else { continue };
            cfg.blx.registry.declare(
                scope,
                opt,
                o.str("backendout"),
                o.str("backendin"),
                None,
                false,
            );
        }
    }
    // Datafield sets (names are not lowercased here, unlike the .bcf).
    for s in conf.list("datafieldset").hashes() {
        let name = s.str("name").unwrap_or("").to_string();
        for m in s.list("member").hashes() {
            let member = match m
                .first_str("field")
                .filter(|f| crate::perlish::truthy(Some(f)))
            {
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
    for (k, v) in conf.iter() {
        if k == "optionscope" || k == "datafieldset" {
            continue;
        }
        let lk = lc(k);
        match lk.as_str() {
            "labelalphatemplate" => {
                for t in XHashList::of(v) {
                    let latype = t.str("type").unwrap_or("");
                    if latype == "global" {
                        cfg.blx
                            .set_global("labelalphatemplate", OptValue::Xml(XVal::Hash(t.clone())));
                    } else {
                        cfg.blx.set_entrytype(
                            latype,
                            "labelalphatemplate",
                            OptValue::Xml(XVal::Hash(t.clone())),
                        );
                    }
                }
            }
            "labelalphanametemplate" => {
                // BB-083: each template replaces the whole option, so only
                // the last survives. With the quirk fixed they accumulate.
                let mut acc = BTreeMap::new();
                for t in XHashList::of(v) {
                    let (name, parts) = labelalphaname_template(t);
                    if cfg.quirks.on(Quirk::ConfLabelalphanameLastWins) {
                        acc = BTreeMap::new();
                    }
                    acc.insert(name, parts);
                    cfg.blx.set_global(
                        "labelalphanametemplate",
                        OptValue::LabelAlphaNameTemplates(acc.clone()),
                    );
                }
            }
            "uniquenametemplate" => {
                let mut acc = BTreeMap::new();
                for t in XHashList::of(v) {
                    let (name, parts, _) = uniquename_template(t);
                    acc.insert(name, parts);
                }
                cfg.blx.set_global(
                    "uniquenametemplate",
                    if acc.is_empty() {
                        OptValue::Undef
                    } else {
                        OptValue::UniquenameTemplates(acc)
                    },
                );
            }
            "namehashtemplate" => {
                let mut acc = BTreeMap::new();
                for t in XHashList::of(v) {
                    let (name, parts) = namehash_template(t);
                    acc.insert(name, parts);
                }
                cfg.blx.set_global(
                    "namehashtemplate",
                    if acc.is_empty() {
                        OptValue::Undef
                    } else {
                        OptValue::NamehashTemplates(acc)
                    },
                );
            }
            "sortingnamekeytemplate" => {
                let mut acc = BTreeMap::new();
                for t in XHashList::of(v) {
                    let (name, tpl) = sortingnamekey_template(t);
                    acc.insert(name, tpl);
                }
                cfg.blx.set_global(
                    "sortingnamekeytemplate",
                    if acc.is_empty() {
                        OptValue::Undef
                    } else {
                        OptValue::SortingNameKeyTemplates(acc)
                    },
                );
            }
            "transliteration" => {
                for tr in XHashList::of(v) {
                    let val = tr
                        .get("translit")
                        .cloned()
                        .map_or(OptValue::Undef, OptValue::Xml);
                    match tr.first_str("entrytype") {
                        Some("*") => cfg.blx.set_global("translit", val),
                        Some(et) => cfg.blx.set_entrytype(et, "translit", val),
                        None => cfg.blx.set_entrytype("", "translit", val),
                    }
                }
            }
            "nosort" | "noinit" | "nolabel" => {
                // DC-005 (BB-005): biber crashes on the documented
                // `<noinit><option value=".."/></noinit>` form because
                // `noinit`/`nolabel` are force-arrayed; we parse it.
                let mut pats = Vec::new();
                for h in XHashList::of(v) {
                    for o in h.list("option").hashes() {
                        pats.push(PatternOpt {
                            name: o.str("name").map(str::to_string),
                            value: o.str("value").map(str::to_string),
                            has_name: o.contains("name"),
                        });
                    }
                }
                let val = if pats.is_empty() {
                    BiberValue::Undef
                } else {
                    BiberValue::Patterns(pats)
                };
                cfg.set_configfile_option(k, val);
            }
            "collate_options" => {
                let mut coll = cfg.biber.collate_options().cloned().unwrap_or_default();
                for h in XHashList::of(v) {
                    for o in h.list("option").hashes() {
                        coll.insert(
                            o.str("name").unwrap_or("").to_string(),
                            o.str("value").unwrap_or("").to_string(),
                        );
                    }
                }
                cfg.set_configfile_option(k, BiberValue::Map(coll));
            }
            "sourcemap" => {
                let mut sms = Vec::new();
                for h in XHashList::of(v) {
                    for sm in h.list("maps").hashes() {
                        match sm.str("level") {
                            Some("driver") => notes.push(
                                "You can't set driver level sourcemaps via biber - use \\DeclareDriverSourcemap in biblatex. Ignoring map.".to_string(),
                            ),
                            Some("style") => notes.push(
                                "You can't set style level sourcemaps via biber - use \\DeclareStyleSourcemap in biblatex. Ignoring map.".to_string(),
                            ),
                            _ => sms.push(SourceMaps::from_xval(sm)),
                        }
                    }
                }
                let val = if sms.is_empty() {
                    BiberValue::Undef
                } else {
                    BiberValue::Sourcemap(sms)
                };
                cfg.set_configfile_option(k, val);
            }
            "inheritance" => {
                cfg.blx.set_global(k, OptValue::Xml(v.clone()));
            }
            "sortexclusion" | "sortinclusion" => {
                // DC-005 (BB-006): biber crashes on a single <sortinclusion>
                // (missing from ForceArray); we accept it.
                let inner = if lk == "sortexclusion" {
                    "exclusion"
                } else {
                    "inclusion"
                };
                for sx in XHashList::of(v) {
                    let mut set = std::collections::BTreeSet::new();
                    for e in sx.list(inner).iter() {
                        let c = match e {
                            XVal::Hash(h) => h.content().unwrap_or("").to_string(),
                            XVal::Str(s) => s.clone(),
                            XVal::Array(_) => continue,
                        };
                        set.insert(c);
                    }
                    let val = if set.is_empty() {
                        OptValue::Undef
                    } else {
                        OptValue::FieldSet(set)
                    };
                    cfg.blx
                        .set_entrytype(sx.str("type").unwrap_or(""), &lk, val);
                }
            }
            "presort" => {
                for p in XHashList::of(v) {
                    let val = p
                        .content()
                        .map_or(OptValue::Undef, |c| OptValue::Str(c.to_string()));
                    if p.contains("type") {
                        cfg.blx
                            .set_entrytype(p.str("type").unwrap_or(""), "presort", val);
                    } else {
                        cfg.blx.set_global("presort", val);
                    }
                }
            }
            "sortingtemplate" => {
                let gl = cfg
                    .blx
                    .global("sortlocale")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let mut acc = BTreeMap::new();
                for ss in XHashList::of(v) {
                    acc.insert(
                        ss.str("name").unwrap_or("").to_string(),
                        parse_sort(ss, gl.as_deref()),
                    );
                }
                cfg.blx.set_global(
                    "sortingtemplate",
                    if acc.is_empty() {
                        OptValue::Undef
                    } else {
                        OptValue::SortingTemplates(acc)
                    },
                );
            }
            "datamodel" => {
                cfg.blx.add_datamodel(v.clone());
            }
            _ => {
                // A simple option: an element with content.
                if let XVal::Hash(h) = v
                    && h.contains("content")
                {
                    cfg.set_configfile_option(k, BiberValue::from(h.content().map(str::to_string)));
                }
            }
        }
    }
    notes
}

/// Iterate over the hashes of a value that may be a single hash or an array
/// of hashes.
struct XHashList;

impl XHashList {
    fn of(v: &XVal) -> Vec<&XHash> {
        match v {
            XVal::Hash(h) => vec![h],
            XVal::Array(a) => a.iter().filter_map(XVal::as_hash).collect(),
            XVal::Str(_) => Vec::new(),
        }
    }
}

//! Data inheritance: set parents from their first member
//! (`Biber::Entry::set_inherit_from`, Entry.pm:674) and crossref
//! inheritance under `\DeclareDataInheritance` (`Biber::process_interentry`,
//! Biber.pm:1654; `Biber::Entry::inherit_from`, Entry.pm:931).

use super::graph::InhGraph;
use super::{
    SharedLists, copy_field, datefield_stems, datefields_now, entry_key, true_str, truthy,
    warn_entry,
};
use crate::bcf::templates::Inheritance;
use crate::biber_error;
use crate::config::blx::OptValue;
use crate::log::Fatal;
use crate::model::SectionState;
use crate::pipeline::Ctx;
use crate::quirks::Quirk;
use crate::xmlin::XVal;
use perl_compat::{fc, perl_truthy};
use serde_json::{Map, Value as J};
use std::collections::HashSet;

/// The derived date fields crossref inheritance copies (Entry.pm:1072).
const INHERIT_DATE_META: [&str; 12] = [
    "dateunspecified",
    "datesplit",
    "datejulian",
    "enddatejulian",
    "dateapproximate",
    "enddateapproximate",
    "dateuncertain",
    "enddateuncertain",
    "yeardivision",
    "endyeardivision",
    "era",
    "endera",
];

const DATEPARTS: [&str; 8] = [
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "yeardivision",
    "timezone",
];

/// `$field =~ s/(?:end)?(?:year|month|day|hour|minute|second|yeardivision|timezone)$//r`
fn date_stem(field: &str) -> &str {
    for (i, _) in field.char_indices() {
        let rest = &field[i..];
        let r = rest.strip_prefix("end").unwrap_or(rest);
        if DATEPARTS.contains(&r) || DATEPARTS.contains(&rest) {
            return &field[..i];
        }
    }
    field
}

/// `$entry->set_inherit_from($parent)` (Entry.pm:674): a set inherits the
/// data fields it lacks from its first member.
pub fn set_inherit_from(
    ctx: &mut Ctx,
    st: &mut SectionState,
    sh: &mut SharedLists,
    key: &str,
    parent: &str,
) {
    let fields: Vec<String> = st.section.entries[parent]
        .datafields
        .keys()
        .map(|k| k.to_string())
        .collect();
    for f in fields {
        if st.section.entries[key].field_exists(&f) {
            continue;
        }
        if fc(&f) == "annotation" || fc(&f) == "shorthand" {
            continue;
        }
        copy_field(ctx, st, sh, parent, &f, key, &f);
    }
    for df in datefield_stems(ctx) {
        let f = format!("{df}datesplit");
        let v = st.section.entries[parent].get_field(&f).cloned();
        if truthy(v.as_ref()) {
            st.section
                .entries
                .get_mut(key)
                .expect("entry")
                .set_field(f, v);
        }
    }
}

struct Inh<'a> {
    g: InhGraph,
    sh: SharedLists,
    rules: &'a Inheritance,
    /// `$dmh->{dateparts}`.
    dateparts: HashSet<&'a str>,
}

/// `Biber::process_interentry` (without the tool-mode guard).
pub fn process_interentry(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    let dm = ctx.dm;
    let rules = match ctx.cfg.blx.global("inheritance") {
        Some(OptValue::Xml(XVal::Hash(h))) => Inheritance::from_xval(h),
        _ => Inheritance::default(),
    };
    let mut inh = Inh {
        g: InhGraph::load(st, "crossref"),
        sh: SharedLists::load(st),
        rules: &rules,
        dateparts: dm.helpers.dateparts.iter().map(String::as_str).collect(),
    };
    let mut res = Ok(());
    for citekey in st.section.citekeys.clone() {
        let Some(key) = entry_key(&st.section, &citekey) else {
            continue;
        };
        let Some(cr) = true_str(&st.section.entries[&key], "crossref") else {
            continue;
        };
        let ck = st.section.entries[&key].citekey().to_string();
        if inh.g.get_inheritance(&cr, &ck) {
            continue;
        }
        match entry_key(&st.section, &cr) {
            None => warn_entry(
                ctx,
                st,
                &key,
                format!("Cannot inherit from crossref key '{cr}' - does it exist?"),
            ),
            Some(parent) => {
                res = inherit_from(ctx, st, &mut inh, &key, &parent);
                if res.is_err() {
                    break;
                }
            }
        }
    }
    inh.sh.store(st);
    res
}

/// `Biber::Config->add_uniq_ignore($key, $field, $uniqs)`.
fn add_uniq_ignore(st: &mut SectionState, key: &str, field: &str, uniqs: Option<&str>) {
    let Some(u) = uniqs.filter(|u| perl_truthy(u)) else {
        return;
    };
    // split(/\s*,\s*/, …): trailing empty fields are dropped.
    let mut parts: Vec<&str> = u.split(',').collect();
    let n = parts.len();
    for (i, p) in parts.iter_mut().enumerate() {
        let mut s = *p;
        if i > 0 {
            s = s.trim_start_matches(perl_compat::unicode::is_perl_space);
        }
        if i + 1 < n {
            s = s.trim_end_matches(perl_compat::unicode::is_perl_space);
        }
        *p = s;
    }
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    let h = super::config_hash(st, "uniqignore");
    let k = h.entry(key).or_insert_with(|| J::Object(Map::new()));
    if !k.is_object() {
        *k = J::Object(Map::new());
    }
    let k = k.as_object_mut().expect("object");
    for p in parts {
        let a = k.entry(p).or_insert_with(|| J::Array(Vec::new()));
        if let J::Array(a) = a {
            a.push(J::from(field));
        }
    }
}

/// `$entry->inherit_from($parent)` (Entry.pm:931).
fn inherit_from(
    ctx: &mut Ctx,
    st: &mut SectionState,
    inh: &mut Inh,
    key: &str,
    parent: &str,
) -> Result<(), Fatal> {
    let secnum = st.section.number.clone();
    let target_key = st.section.entries[key].citekey().to_string();
    let source_key = st.section.entries[parent].citekey().to_string();
    inh.g.set_inheritance(st, &source_key, &target_key);
    if !inh.g.is_path_after_set(&target_key, &source_key) {
        // Cascading crossrefs: the parent inherits from its parent first,
        // every time (biber's records grow accordingly).
        if let Some(pp) = true_str(&st.section.entries[parent], "crossref")
            && let Some(ppk) = entry_key(&st.section, &pp)
        {
            inherit_from(ctx, st, inh, parent, &ppk)?;
        }
    } else {
        biber_error!(
            ctx.log,
            "Circular inheritance between '{source_key}'<->'{target_key}'"
        )?;
    }

    let bee = st.section.entries[key].entrytype().to_string();
    let parenttype = st.section.entries[parent].entrytype().to_string();
    let rules = inh.rules;
    let mut inherit_all = rules.inherit_all.clone().unwrap_or_default();
    let mut override_target = rules.override_target.clone().unwrap_or_default();
    let mut dignore = rules.ignore.clone();
    let matches = |s: &Option<String>, t: &Option<String>| {
        let s = s.as_deref().unwrap_or("");
        let t = t.as_deref().unwrap_or("");
        (s == "*" || s == parenttype) && (t == "*" || t == bee)
    };
    for tp in &rules.default_pairs {
        if matches(&tp.source, &tp.target) {
            if let Some(v) = tp.inherit_all.as_ref().filter(|v| perl_truthy(v)) {
                inherit_all = v.clone();
            }
            if let Some(v) = tp.override_target.as_ref().filter(|v| perl_truthy(v)) {
                override_target = v.clone();
            }
            if tp.ignore.is_some() {
                dignore = tp.ignore.clone();
            }
        }
    }
    let niset_fields: Option<Vec<String>> = ctx
        .cfg
        .blx
        .get(Some(&secnum), "noinherit", None, Some(&target_key))
        .and_then(OptValue::as_str)
        .filter(|s| perl_truthy(s))
        .map(|s| {
            ctx.cfg
                .datafield_sets_resolved
                .get(s)
                .cloned()
                .unwrap_or_default()
        });
    let noinherit = |f: &str| {
        niset_fields
            .as_ref()
            .is_some_and(|l| l.iter().any(|x| x == f))
    };
    let mut processed: HashSet<String> = HashSet::new();

    // Fields with special treatment.
    for rule in &rules.inherits {
        for tp in &rule.type_pairs {
            if !matches(&tp.source, &tp.target) {
                continue;
            }
            for field in &rule.fields {
                if let Some(t) = &field.target
                    && noinherit(t)
                {
                    continue;
                }
                let Some(source) = &field.source else {
                    continue;
                };
                if !st.section.entries[parent].field_exists(source) {
                    continue;
                }
                processed.insert(source.clone());
                let fot = field.override_target.as_deref().unwrap_or("false");
                let target = field.target.clone().unwrap_or_default();
                if field.skip.as_deref().is_some_and(perl_truthy) {
                    continue;
                }
                if !st.section.entries[key].field_exists(&target) || fot == "true" {
                    copy_field(ctx, st, &mut inh.sh, parent, source, key, &target);
                    ctx.run.annotations.inherit_annotations(
                        &source_key,
                        &target_key,
                        source,
                        &target,
                        None,
                    );
                    let ignore = rule
                        .ignore
                        .as_deref()
                        .filter(|i| perl_truthy(i))
                        .or(dignore.as_deref());
                    add_uniq_ignore(st, &target_key, &target, ignore);
                }
            }
        }
    }

    // The rest of the (data) fields.
    if inherit_all == "true" {
        let fields: Vec<String> = st.section.entries[parent]
            .datafields
            .keys()
            .map(|k| k.to_string())
            .collect();
        let quirk = ctx.cfg.quirks.on(Quirk::CrossrefDateMeta);
        let mut filtered = Vec::new();
        let mut removed: Vec<String> = Vec::new();
        for f in fields {
            if inh.dateparts.contains(f.as_str()) {
                // BB-023: only the main date's datesplit is checked.
                let ds = if quirk {
                    "datesplit".to_string()
                } else {
                    format!("{}datesplit", date_stem(&f))
                };
                let p = &st.section.entries[parent];
                let c = &st.section.entries[key];
                if truthy(p.get_field(&ds))
                    && truthy(c.get_field(&ds))
                    && date_fields_exist(st, key, &f)
                {
                    if override_target == "true" {
                        delete_date_fields(st, &mut inh.sh, key, &f);
                    } else {
                        removed.push(f);
                        continue;
                    }
                }
            }
            filtered.push(f);
        }
        // Derived date fields are technically data: the parent's true
        // `<df><meta>` fields for the date fields not skipped.
        let datefields = datefields_now(ctx);
        let stems: Vec<&str> = datefields
            .iter()
            .filter(|datefield| {
                let df = datefield.strip_suffix("date").unwrap_or(datefield);
                // BB-022: compares date field names with datepart names, so
                // never skips.
                let skip = if quirk {
                    removed.contains(datefield)
                } else {
                    removed.iter().any(|r| date_stem(r) == df)
                };
                !skip
            })
            .map(|d| d.strip_suffix("date").unwrap_or(d))
            .collect();
        let copies: Vec<(crate::model::Sym, crate::model::Value)> = st.section.entries[parent]
            .derivedfields
            .iter()
            .filter_map(|(f, v)| {
                let v = v.as_ref().filter(|v| truthy(Some(v)))?;
                let meta = stems.iter().any(|df| {
                    f.strip_prefix(df)
                        .is_some_and(|dsf| INHERIT_DATE_META.contains(&dsf))
                });
                meta.then(|| (*f, v.clone()))
            })
            .collect();
        let child = st.section.entries.get_mut(key).expect("entry");
        for (f, v) in copies {
            child.derivedfields.insert(f, Some(v));
        }
        for f in filtered {
            if noinherit(&f) || processed.contains(&f) {
                continue;
            }
            if !st.section.entries[key].field_exists(&f) || override_target == "true" {
                copy_field(ctx, st, &mut inh.sh, parent, &f, key, &f);
                ctx.run
                    .annotations
                    .inherit_annotations(&source_key, &target_key, &f, &f, None);
                add_uniq_ignore(st, &target_key, &f, dignore.as_deref());
            }
        }
    }
    for df in datefield_stems(ctx) {
        let f = format!("{df}datesplit");
        let v = st.section.entries[parent].get_field(&f).cloned();
        if truthy(v.as_ref()) {
            st.section
                .entries
                .get_mut(key)
                .expect("entry")
                .set_field(f, v);
        }
    }
    Ok(())
}

/// `date_fields_exist` (Entry.pm:565).
fn date_fields_exist(st: &SectionState, key: &str, field: &str) -> bool {
    let t = date_stem(field);
    let e = &st.section.entries[key];
    DATEPARTS.iter().any(|dp| {
        e.datafields.contains_key(&format!("{t}{dp}"))
            || e.datafields.contains_key(&format!("{t}end{dp}"))
    })
}

/// `delete_date_fields` (Entry.pm:581).
fn delete_date_fields(st: &mut SectionState, sh: &mut SharedLists, key: &str, field: &str) {
    let t = date_stem(field).to_string();
    for dp in DATEPARTS {
        for f in [format!("{t}{dp}"), format!("{t}end{dp}")] {
            sh.unshare(key, &f);
            if let Some(e) = st.section.entries.get_mut(key) {
                e.datafields.remove(&f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::date_stem;

    #[test]
    fn date_stems() {
        assert_eq!(date_stem("year"), "");
        assert_eq!(date_stem("endyear"), "");
        assert_eq!(date_stem("origendmonth"), "orig");
        assert_eq!(date_stem("yeardivision"), "");
        assert_eq!(date_stem("eventyeardivision"), "event");
        assert_eq!(date_stem("title"), "title");
    }
}

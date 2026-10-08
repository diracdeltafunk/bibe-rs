//! XDATA: `Biber::resolve_alias_refs`'s XDATA part (Biber.pm:1316),
//! `Biber::resolve_xdata` (Biber.pm:1433) and `Biber::Entry::resolve_xdata`
//! (Entry.pm:714).
//!
//! An entry's XDATA references (`xdatarefs`, a list of hashes) are one Perl
//! array shared with its related entry clones (`clone` copies the
//! reference), so changes made while processing one show in all. We keep
//! the references of the root of each clone chain as the live copy and copy
//! them to the clones at the end.

use super::graph::InhGraph;
use super::{
    SharedLists, alias, copy_field, entry_key, mutate_list, num, truthy, warn_entry,
};
use crate::biber_error;
use crate::log::Fatal;
use crate::model::{Section, SectionState, Value};
use crate::pipeline::Ctx;
use crate::quirks::Quirk;
use serde_json::{Value as J, json};
use std::collections::HashSet;

/// The entry holding the live copy of `key`'s XDATA references: follow
/// `clonesourcekey` while the source exists and has references.
fn owner(sec: &Section, key: &str) -> String {
    let mut k = key.to_string();
    let mut guard = 0;
    while let Some(e) = sec.entries.get(&k) {
        let Some(src) = e.derivedfields.get("clonesourcekey").cloned().flatten() else {
            break;
        };
        let Value::Str(src) = src else { break };
        match sec.entries.get(&src) {
            Some(s) if s.slots.get("xdatarefs").is_some_and(|x| !x.is_null()) => k = src,
            _ => break,
        }
        guard += 1;
        if guard > sec.entries.len() {
            break;
        }
    }
    k
}

/// Copy every owner's references to the clones sharing them.
fn propagate(sec: &mut Section) {
    let keys: Vec<String> = sec
        .entries
        .iter()
        .filter(|(_, e)| e.slots.get("xdatarefs").is_some_and(|x| !x.is_null()))
        .map(|(k, _)| k.clone())
        .collect();
    for k in keys {
        let o = owner(sec, &k);
        if o != k {
            let v = sec.entries[&o].slots.get("xdatarefs").cloned();
            if let Some(v) = v {
                sec.entries.get_mut(&k).expect("entry").slots.insert("xdatarefs".into(), v);
            }
        }
    }
}

/// The XDATA part of `resolve_alias_refs` for one citekey (BB-091: the
/// resolved whole-entry keys accumulate in one array shared by all
/// whole-entry references of the entry).
pub fn resolve_ref_aliases(st: &mut SectionState, key: &str) {
    let o = owner(&st.section, key);
    let Some(J::Array(refs)) = st.section.entries[&o].slots.get("xdatarefs").cloned() else {
        return;
    };
    let mut refs = refs;
    let mut resolved: Vec<J> = Vec::new();
    let mut whole: Vec<usize> = Vec::new();
    for (i, r) in refs.iter_mut().enumerate() {
        let entries: Vec<String> = r
            .get("xdataentries")
            .and_then(J::as_array)
            .map(|a| a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect())
            .unwrap_or_default();
        let real = |k: &String| alias(&st.section, k).map_or(k.clone(), str::to_string);
        if r.get("xdatafield").is_none_or(J::is_null) {
            for k in &entries {
                resolved.push(J::from(real(k)));
            }
            whole.push(i);
        } else if let Some(o) = r.as_object_mut() {
            let first = entries.first().cloned().unwrap_or_default();
            o.insert("xdataentries".into(), json!([real(&first)]));
        }
    }
    for i in whole {
        if let Some(o) = refs[i].as_object_mut() {
            o.insert("xdataentries".into(), J::Array(resolved.clone()));
        }
    }
    st.section
        .entries
        .get_mut(&o)
        .expect("entry")
        .slots
        .insert("xdatarefs".into(), J::Array(refs));
    propagate(&mut st.section);
}

struct Xd {
    g: InhGraph,
    sh: SharedLists,
    /// XDATA entries whose references were resolved (for the
    /// `xdata-rerefs` quirk fixed).
    done: HashSet<String>,
}

/// `Biber::resolve_xdata` over all entries except XDATA ones.
///
/// biber walks the entries in Perl hash order, so which XDATA loop is
/// reported varies (BB-112); we walk citekeys in order, then the other
/// entries in key order (DC-017).
pub fn resolve_xdata(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    let mut x = Xd {
        g: InhGraph::load(st, "xdata"),
        sh: SharedLists::load(st),
        done: HashSet::new(),
    };
    let mut order: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for k in &st.section.citekeys {
        if st.section.entries.contains_key(k) && seen.insert(k.clone()) {
            order.push(k.clone());
        }
    }
    for k in st.section.entries.keys() {
        if seen.insert(k.clone()) {
            order.push(k.clone());
        }
    }
    let mut res = Ok(());
    for k in order {
        let e = &st.section.entries[&k];
        if e.entrytype() == "xdata" {
            continue;
        }
        if !e.slots.get("xdatarefs").is_some_and(J::is_array) {
            continue;
        }
        res = resolve_entry(ctx, st, &mut x, &k);
        if res.is_err() {
            break;
        }
    }
    propagate(&mut st.section);
    x.sh.store(st);
    res
}

/// Set `resolved` on the `i`-th reference of `o`.
fn set_resolved(st: &mut SectionState, o: &str, i: usize, v: &str) {
    if let Some(J::Array(a)) = st
        .section
        .entries
        .get_mut(o)
        .and_then(|e| e.slots.get_mut("xdatarefs"))
        && let Some(J::Object(r)) = a.get_mut(i)
    {
        r.insert("resolved".into(), J::from(v));
    }
}

/// `$entry->resolve_xdata($xdata)` (Entry.pm:714).
fn resolve_entry(ctx: &mut Ctx, st: &mut SectionState, x: &mut Xd, key: &str) -> Result<(), Fatal> {
    let o = owner(&st.section, key);
    let Some(J::Array(refs)) = st.section.entries[&o].slots.get("xdatarefs").cloned() else {
        return Ok(());
    };
    let secnum = st.section.number.clone();
    let entry_key_s = st.section.entries[key].citekey().to_string();
    let bee = st.section.entries[key].entrytype().to_string();
    let s = |r: &J, f: &str| r.get(f).and_then(J::as_str).map(str::to_string);
    for (i, xdatum) in refs.iter().enumerate() {
        let xentries: Vec<String> = xdatum
            .get("xdataentries")
            .and_then(J::as_array)
            .map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect())
            .unwrap_or_default();
        for xdref in xentries {
            let Some(xkey) = entry_key(&st.section, &xdref) else {
                warn_entry(ctx, st, key, format!(
                    "{bee} entry '{entry_key_s}' references XDATA entry '{xdref}' which does not exist, not resolving (section {secnum})"
                ));
                set_resolved(st, &o, i, "0");
                continue;
            };
            if st.section.entries[&xkey].entrytype() != "xdata" {
                warn_entry(ctx, st, key, format!(
                    "{bee} entry '{entry_key_s}' references XDATA entry '{xdref}' which is not an XDATA entry, not resolving (section {secnum})"
                ));
                set_resolved(st, &o, i, "0");
                continue;
            }
            x.g.set_inheritance(st, &xdref, &entry_key_s);
            if x.g.is_path_after_set(&entry_key_s, &xdref) {
                biber_error!(
                    ctx.log,
                    "Circular XDATA inheritance between '{xdref}'<->'{entry_key_s}'"
                )?;
                continue;
            }
            // Recurse into the XDATA entry's own references, every time it
            // is referenced (BB-025).
            let has_refs = st.section.entries[&xkey]
                .slots
                .get("xdatarefs")
                .is_some_and(J::is_array);
            if has_refs
                && (ctx.cfg.quirks.on(Quirk::XdataRerefs) || x.done.insert(xkey.clone()))
            {
                resolve_entry(ctx, st, x, &xkey)?;
            }
            let xcite = st.section.entries[&xkey].citekey().to_string();
            match s(xdatum, "xdatafield") {
                None => {
                    // Whole-entry reference: inherit every field.
                    let fields: Vec<String> =
                        st.section.entries[&xkey].datafields.keys().cloned().collect();
                    for f in fields {
                        if f == "ids" {
                            continue;
                        }
                        copy_field(ctx, st, &mut x.sh, &xkey, &f, key, &f);
                        ctx.run
                            .annotations
                            .inherit_annotations(&xcite, &entry_key_s, &f, &f, None);
                    }
                }
                Some(xdatafield) => {
                    let reffield = s(xdatum, "reffield").unwrap_or_default();
                    let xdataposition = s(xdatum, "xdataposition").unwrap_or_default();
                    let refposition = s(xdatum, "refposition").map_or(0, |p| num(&p));
                    let ok = granular(
                        ctx,
                        st,
                        x,
                        key,
                        &xkey,
                        &GranularRef {
                            bee: &bee,
                            entry_key: &entry_key_s,
                            xdref: &xdref,
                            xcite: &xcite,
                            reffield: &reffield,
                            xdatafield: &xdatafield,
                            xdataposition: &xdataposition,
                            refposition,
                        },
                    );
                    if !ok {
                        set_resolved(st, &o, i, "0");
                        continue;
                    }
                }
            }
            set_resolved(st, &o, i, "1");
        }
    }
    Ok(())
}

struct GranularRef<'a> {
    bee: &'a str,
    entry_key: &'a str,
    xdref: &'a str,
    xcite: &'a str,
    reffield: &'a str,
    xdatafield: &'a str,
    xdataposition: &'a str,
    refposition: i64,
}

/// A granular XDATA reference (Entry.pm:791-905). Returns whether it was
/// resolved.
fn granular(
    ctx: &mut Ctx,
    st: &mut SectionState,
    x: &mut Xd,
    key: &str,
    xkey: &str,
    r: &GranularRef,
) -> bool {
    let secnum = st.section.number.clone();
    let GranularRef {
        bee,
        entry_key,
        xdref,
        xcite,
        reffield,
        xdatafield,
        xdataposition,
        refposition,
    } = *r;
    let dm = ctx.dm;
    let ft = |f: &str| dm.get_fieldtype(f).unwrap_or("").to_string();
    let dt = |f: &str| dm.get_datatype(f).unwrap_or("").to_string();
    if ft(reffield) != ft(xdatafield) || dt(reffield) != dt(xdatafield) {
        warn_entry(ctx, st, key, format!(
            "Field '{reffield}' in {bee} entry '{entry_key}' which xdata references field '{xdatafield}' in entry '{xdref}' are not the same types, not resolving (section {secnum})"
        ));
        return false;
    }
    let xval = st.section.entries[xkey].get_field(xdatafield).cloned();
    if !truthy(xval.as_ref()) {
        warn_entry(ctx, st, key, format!(
            "Field '{reffield}' in {bee} entry '{entry_key}' references XDATA field '{xdatafield}' in entry '{xdref}' and this field does not exist, not resolving (section {secnum})"
        ));
        return false;
    }
    let xval = xval.expect("checked");
    let no_position = |ctx: &mut Ctx, st: &mut SectionState| {
        warn_entry(ctx, st, key, format!(
            "Field '{reffield}' in {bee} entry '{entry_key}' references field '{xdatafield}' position {xdataposition} in entry '{xdref}' and this position does not exist, not resolving (section {secnum})"
        ));
    };
    let quirk_index = ctx.cfg.quirks.on(Quirk::XdataNameIndex);
    let refidx = (refposition - 1).max(0) as usize;
    if dm.field_is_type("list", "name", reffield) {
        let target = st.section.entries[key].get_field(reffield).and_then(Value::as_names);
        let (Some(target), Value::Names(src)) = (target, &xval) else {
            return false;
        };
        let src = *src;
        let arena = &mut st.section.names;
        if xdataposition == "*" {
            // Names::splice (Names.pm:101)
            let srcnames = arena.list(src).names.clone();
            let tl = arena.list_mut(target);
            let at = refidx.min(tl.names.len());
            let end = (at + 1).min(tl.names.len());
            tl.names.splice(at..end, srcnames);
            let ids = tl.names.clone();
            // BB-024: 0-based, and the last name is not re-indexed.
            let n = if quirk_index { ids.len().saturating_sub(1) } else { ids.len() };
            for (i, id) in ids.iter().take(n).enumerate() {
                let idx = if quirk_index { i } else { i + 1 };
                arena.name_mut(*id).index = Some(idx.to_string());
            }
            let count = arena.list(src).names.len();
            for i in 1..=count {
                let ti = (refposition + i as i64 - 1).to_string();
                ctx.run.annotations.inherit_annotations(
                    xcite,
                    entry_key,
                    xdatafield,
                    reffield,
                    Some((&i.to_string(), &ti)),
                );
            }
        } else {
            let n = num(xdataposition);
            let names = arena.list(src).names.clone();
            let size = names.len() as i64;
            // is_nth_name: $namelist[$n == 0 ? 0 : $n-1]
            let exists = if n == 0 { size > 0 } else { n - 1 < size };
            if !exists {
                no_position(ctx, st);
                return false;
            }
            // nth_name: $namelist[$n > $size ? $size-1 : $n-1] (-1: last)
            let i = if n > size { size - 1 } else { n - 1 };
            let i = if i < 0 { size + i } else { i };
            let name = names[i as usize];
            arena.name_mut(name).index = Some(if quirk_index {
                (refposition - 1).to_string()
            } else {
                refposition.to_string()
            });
            let tl = arena.list_mut(target);
            if refidx < tl.names.len() {
                tl.names[refidx] = name;
            } else {
                tl.names.push(name);
            }
            ctx.run.annotations.inherit_annotations(
                xcite,
                entry_key,
                xdatafield,
                reffield,
                Some((xdataposition, &refposition.to_string())),
            );
        }
    } else if dm.field_is_fieldtype("list", reffield) {
        let src: Vec<String> = match &xval {
            Value::List(l) => l.clone(),
            _ => Vec::new(),
        };
        if xdataposition == "*" {
            mutate_list(st, &x.sh, key, reffield, |l| {
                let at = refidx.min(l.len());
                let end = (at + 1).min(l.len());
                l.splice(at..end, src.iter().cloned());
            });
            for i in 1..=src.len() {
                let ti = (refposition + i as i64 - 1).to_string();
                ctx.run.annotations.inherit_annotations(
                    xcite,
                    entry_key,
                    xdatafield,
                    reffield,
                    Some((&i.to_string(), &ti)),
                );
            }
        } else {
            // $list->[$xdataposition-1] (-1: last)
            let p = num(xdataposition) - 1;
            let p = if p < 0 { src.len() as i64 + p } else { p };
            let v = (p >= 0).then(|| src.get(p as usize)).flatten().cloned();
            if !v.as_deref().is_some_and(perl_compat::perl_truthy) {
                no_position(ctx, st);
                return false;
            }
            let v = v.expect("checked");
            mutate_list(st, &x.sh, key, reffield, |l| {
                if refidx < l.len() {
                    l[refidx] = v.clone();
                } else {
                    l.resize(refidx, String::new());
                    l.push(v.clone());
                }
            });
            ctx.run.annotations.inherit_annotations(
                xcite,
                entry_key,
                xdatafield,
                reffield,
                Some((xdataposition, &refposition.to_string())),
            );
        }
    } else {
        copy_field(ctx, st, &mut x.sh, xkey, xdatafield, key, reffield);
        ctx.run
            .annotations
            .inherit_annotations(xcite, entry_key, xdatafield, reffield, None);
    }
    true
}

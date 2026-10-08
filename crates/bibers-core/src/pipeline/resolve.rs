//! Dependency resolution: citekey aliases, dynamic sets and related
//! entries, XDATA, sets, crossref/xref inheritance, data model validation.
//!
//! A port of `Biber::process_citekey_aliases` … `Biber::postprocess_sets`
//! (Biber.pm:1286-1957) and the `Biber::Entry` methods they use
//! (`relclone`, `clone`, `resolve_xdata`, `set_inherit_from`,
//! `inherit_from`).

mod graph;
mod inherit;
mod related;
mod shared;
mod validate;
mod xdata;

use super::Ctx;
use super::fetch::SecExt;
use crate::log::Fatal;
use crate::model::{Entry, Section, SectionState, Value};
use crate::quirks::Quirk;
use perl_compat::perl_truthy;
use serde_json::{Map, Value as J};

pub use shared::SharedLists;
pub use validate::validate_datamodel;

/// The options biber forces on set members and related clones
/// ("dataonly", Biber.pm:2497, Entry.pm:118).
const DATAONLY: [&str; 5] = [
    "skipbib",
    "skiplab",
    "skipbiblist",
    "uniquename=false",
    "uniquelist=false",
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `get_citekey_alias($key)` in a boolean context.
fn alias<'a>(sec: &'a Section, key: &str) -> Option<&'a str> {
    sec.citekey_alias
        .get(key)
        .map(String::as_str)
        .filter(|r| perl_truthy(r))
}

/// `$section->bibentry($key)` (alias-aware): the key of the entry returned.
fn entry_key(sec: &Section, key: &str) -> Option<String> {
    let k = alias(sec, key).unwrap_or(key);
    sec.entries.contains_key(k).then(|| k.to_string())
}

/// Perl truthiness of a field value (references are true).
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None => false,
        Some(Value::Str(s)) => perl_truthy(s),
        Some(_) => true,
    }
}

/// `get_field($f)` as a string, if it is a true scalar.
fn true_str(e: &Entry, f: &str) -> Option<String> {
    match e.get_field(f) {
        Some(Value::Str(s)) if perl_truthy(s) => Some(s.clone()),
        _ => None,
    }
}

/// `biber_warn($msg, $be)`: a warning on the entry and in the global list.
fn warn_entry(ctx: &mut Ctx, st: &mut SectionState, key: &str, msg: String) {
    if let Some(e) = st.section.entries.get_mut(key) {
        let w = e
            .derivedfields
            .entry("warnings".into())
            .or_insert_with(|| Some(Value::List(Vec::new())));
        match w {
            Some(Value::List(l)) => l.push(msg.clone()),
            other => *other = Some(Value::List(vec![msg.clone()])),
        }
    }
    ctx.log.biber_warn(msg);
}

/// `$dmh->{datefields}` after one more `foreach my $df (…) { $df =~
/// s/date$// }` (Entry.pm:175, 696, 1117; bbl.pm:457): the loop variable
/// aliases the shared helper array, which is stripped in place once per
/// loop (BB-028, quirk `datefield-helper-mutation`). Returns the stems the
/// loop sees.
pub fn datefield_stems(ctx: &mut Ctx) -> Vec<String> {
    let strip = |d: &mut String| {
        if d.ends_with("date") {
            d.truncate(d.len() - 4);
        }
    };
    if ctx.cfg.quirks.on(Quirk::DatefieldHelperMutation) {
        let v = ctx
            .run
            .datefields
            .get_or_insert_with(|| ctx.dm.helpers.datefields.clone());
        v.iter_mut().for_each(strip);
        v.clone()
    } else {
        let mut v = ctx.dm.helpers.datefields.clone();
        v.iter_mut().for_each(strip);
        v
    }
}

/// `$dmh->{datefields}` as it currently is (not stripped again).
fn datefields_now(ctx: &Ctx) -> Vec<String> {
    match (&ctx.run.datefields, ctx.cfg.quirks.on(Quirk::DatefieldHelperMutation)) {
        (Some(v), true) => v.clone(),
        _ => ctx.dm.helpers.datefields.clone(),
    }
}

/// `$section->{state}` (a Perl hash, autovivified).
fn sec_state(sec: &mut Section) -> &mut Map<String, J> {
    sec.slot_obj("state")
}

/// `$section->{state}{set}{$which}` (autovivified).
fn set_state<'a>(sec: &'a mut Section, which: &str) -> &'a mut Map<String, J> {
    let set = sec_state(sec)
        .entry("set")
        .or_insert_with(|| J::Object(Map::new()));
    if !set.is_object() {
        *set = J::Object(Map::new());
    }
    let w = set
        .as_object_mut()
        .expect("object")
        .entry(which)
        .or_insert_with(|| J::Object(Map::new()));
    if !w.is_object() {
        *w = J::Object(Map::new());
    }
    w.as_object_mut().expect("object")
}

/// `set_set_pc`/`set_set_cp`: `$section->{state}{set}{$which}{$a}{$b} = 1`.
fn set_set(sec: &mut Section, which: &str, a: &str, b: &str) {
    let m = set_state(sec, which)
        .entry(a)
        .or_insert_with(|| J::Object(Map::new()));
    if !m.is_object() {
        *m = J::Object(Map::new());
    }
    m.as_object_mut()
        .expect("object")
        .insert(b.into(), J::from("1"));
}

/// `get_set_children`/`get_set_parents` (`keys`, in sorted order here).
fn get_set(sec: &mut Section, which: &str, k: &str) -> Vec<String> {
    match set_state(sec, which).get(k) {
        Some(J::Object(m)) => m.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// `$CONFIG->{state}{$name}` as a hash (autovivified).
fn config_hash<'a>(st: &'a mut SectionState, name: &str) -> &'a mut Map<String, J> {
    if !st.config_state.is_object() {
        st.config_state = J::Object(Map::new());
    }
    let v = st
        .config_state
        .as_object_mut()
        .expect("object")
        .entry(name)
        .or_insert_with(|| J::Object(Map::new()));
    if !v.is_object() {
        *v = J::Object(Map::new());
    }
    v.as_object_mut().expect("object")
}

/// `get_keyorder($secnum, $key)`.
fn get_keyorder(st: &SectionState, key: &str) -> Option<String> {
    st.config_state
        .get("keyorder")?
        .get(&st.section.number)?
        .get(key)?
        .as_str()
        .map(str::to_string)
}

/// `set_keyorder($secnum, $key, $order)`.
fn set_keyorder(st: &mut SectionState, key: &str, order: &str) {
    let sec = st.section.number.clone();
    let m = config_hash(st, "keyorder")
        .entry(sec)
        .or_insert_with(|| J::Object(Map::new()));
    if !m.is_object() {
        *m = J::Object(Map::new());
    }
    m.as_object_mut()
        .expect("object")
        .insert(key.into(), J::from(order));
}

/// `set_datafield($f, $v)` with a value of the entry's own.
fn set_own_datafield(
    st: &mut SectionState,
    sh: &mut SharedLists,
    key: &str,
    f: &str,
    v: Option<Value>,
) {
    sh.unshare(key, f);
    if let Some(e) = st.section.entries.get_mut(key) {
        e.set_datafield(f, v);
    }
}

/// `del_field($f)`.
fn del_field(st: &mut SectionState, sh: &mut SharedLists, key: &str, f: &str) {
    sh.unshare(key, f);
    if let Some(e) = st.section.entries.get_mut(key) {
        e.del_field(f);
    }
}

/// `$dst->set_datafield($df, $src->get_field($sf))`: the value is copied
/// by reference (lists and name lists are shared, BB-026/B21).
fn copy_field(
    ctx: &Ctx,
    st: &mut SectionState,
    sh: &mut SharedLists,
    src: &str,
    sf: &str,
    dst: &str,
    df: &str,
) {
    let Some(e) = st.section.entries.get(src) else {
        return;
    };
    let (val, from_data) = match e.datafields.get(sf) {
        Some(Some(v)) => (Some(v.clone()), true),
        _ => (e.derivedfields.get(sf).cloned().flatten(), false),
    };
    let val = match val {
        Some(Value::Names(id)) if !ctx.cfg.quirks.on(Quirk::SharedNamelists) => Some(
            Value::Names(shared::clone_names(&mut st.section.names, id)),
        ),
        v => v,
    };
    match &val {
        Some(v) if from_data && shared::is_array(v) => sh.share(dst, df, src, sf),
        _ => sh.unshare(dst, df),
    }
    if let Some(d) = st.section.entries.get_mut(dst) {
        d.set_datafield(df, val);
    }
}

/// Change a list datafield in place (`splice`, element assignment, `s///`
/// over the elements): the change shows in every field sharing the array.
fn mutate_list(
    st: &mut SectionState,
    sh: &SharedLists,
    key: &str,
    f: &str,
    op: impl Fn(&mut Vec<String>),
) {
    for (k, fl) in sh.group(key, f) {
        if let Some(Some(Value::List(l))) = st
            .section
            .entries
            .get_mut(&k)
            .and_then(|e| e.datafields.get_mut(&fl))
        {
            op(l);
        }
    }
}

/// Perl numification of an integer-like string (positions, counts).
fn num(s: &str) -> i64 {
    perl_compat::truth::numify(s) as i64
}

/// Is a biber option (tool mode guard) true?
fn opt(ctx: &Ctx, o: &str) -> bool {
    ctx.cfg.biber.is_true(o)
}

// ---------------------------------------------------------------------------
// Phases
// ---------------------------------------------------------------------------

/// `Biber::process_citekey_aliases` (Biber.pm:1344): remove citekey
/// aliases from the citekeys.
pub fn process_citekey_aliases(_ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    let keys = st.section.citekeys.clone();
    for k in keys {
        if alias(&st.section, &k).is_some() {
            st.section.del_citekey(&k);
        }
    }
    Ok(())
}

/// `Biber::instantiate_dynamic` (Biber.pm:1368): create the entries of
/// dynamic sets, then the related entry clones of dynamic set members and of
/// every citekey.
pub fn instantiate_dynamic(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    let mut sh = SharedLists::load(st);
    // `dynamic_set_keys` is Perl hash order; we use .bcf declaration order
    // (DC-015).
    let mut dkeys = st.section.dynamic_set_keys();
    if let Some(bs) = ctx.doc.section(&st.section.number) {
        let pos = |k: &String| {
            bs.citekeys
                .iter()
                .position(|c| c == k)
                .unwrap_or(usize::MAX)
        };
        dkeys.sort_by_key(pos);
    }
    for dset in dkeys {
        let members: Vec<String> = st
            .section
            .get_dynamic_set(&dset)
            .into_iter()
            .map(|m| alias(&st.section, &m).map_or(m.clone(), str::to_string))
            .collect();
        st.section.set_dynamic_set(&dset, members.clone());
        let mut be = Entry::default();
        be.set_field("entrytype", Some(Value::Str("set".into())));
        be.set_field("entryset", Some(Value::List(members.clone())));
        be.set_field("citekey", Some(Value::Str(dset.clone())));
        be.set_field("datatype", Some(Value::Str("dynamic".into())));
        if let Some(old) = st.section.entries.insert(dset.clone(), be) {
            sh.unshare_entry(&dset, &old);
        }
        // min grep {defined} map {get_keyorder} @members
        let minorder = members
            .iter()
            .filter_map(|m| get_keyorder(st, m))
            .min_by(|a, b| {
                perl_compat::truth::num_cmp_str(a, b).unwrap_or(std::cmp::Ordering::Equal)
            });
        for m in &members {
            if let Some(k) = entry_key(&st.section, m) {
                related::relclone(ctx, st, &mut sh, &k);
            }
            if let Some(o) = minorder.as_deref().filter(|o| perl_truthy(o)) {
                set_keyorder(st, &dset, o);
            }
        }
    }
    for k in st.section.citekeys.clone() {
        if let Some(k) = entry_key(&st.section, &k) {
            related::relclone(ctx, st, &mut sh, &k);
        }
    }
    sh.store(st);
    super::sync_annotations(ctx, st);
    Ok(())
}

/// `Biber::resolve_alias_refs` (Biber.pm:1286): rewrite alias keys in
/// `xref`, `crossref` and XDATA references to the real keys.
pub fn resolve_alias_refs(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    if ctx.cfg.biber.tool()
        && !(opt(ctx, "output_resolve_crossrefs") || opt(ctx, "output_resolve_xdata"))
    {
        return Ok(());
    }
    for citekey in st.section.citekeys.clone() {
        let Some(key) = entry_key(&st.section, &citekey) else {
            continue;
        };
        for f in ["xref", "crossref"] {
            let real = true_str(&st.section.entries[&key], f)
                .and_then(|r| alias(&st.section, &r).map(str::to_string));
            if let Some(real) = real {
                st.section
                    .entries
                    .get_mut(&key)
                    .expect("entry")
                    .set_datafield(f, Some(Value::Str(real)));
            }
        }
        xdata::resolve_ref_aliases(st, &key);
    }
    Ok(())
}

/// `Biber::resolve_xdata` (Biber.pm:1433).
pub fn resolve_xdata(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    if ctx.cfg.biber.tool() && !opt(ctx, "output_resolve_xdata") {
        return Ok(());
    }
    let r = xdata::resolve_xdata(ctx, st);
    super::sync_annotations(ctx, st);
    r
}

/// `Biber::cite_setmembers` (Biber.pm:1467): promote set members to
/// citekeys; set parents inherit their first member's data.
pub fn cite_setmembers(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    let mut sh = SharedLists::load(st);
    for citekey in st.section.citekeys.clone() {
        let Some(key) = entry_key(&st.section, &citekey) else {
            continue;
        };
        let be = &st.section.entries[&key];
        if be.entrytype() != "set" || !truthy(be.get_field("entryset")) {
            continue;
        }
        let inset: Vec<String> = match be.get_field("entryset") {
            Some(Value::List(l)) => l.clone(),
            _ => continue,
        };
        if inset.is_empty() {
            continue;
        }
        let realmems: Vec<String> = inset
            .iter()
            .map(|m| alias(&st.section, m).map_or(m.clone(), str::to_string))
            .collect();
        set_own_datafield(
            st,
            &mut sh,
            &key,
            "entryset",
            Some(Value::List(realmems.clone())),
        );
        for m in &realmems {
            st.section.add_citekeys(std::slice::from_ref(m));
        }
        if let Some(first) = entry_key(&st.section, &realmems[0]) {
            inherit::set_inherit_from(ctx, st, &mut sh, &key, &first);
        }
        if true_str(&st.section.entries[&key], "crossref").is_some() {
            warn_entry(
                ctx,
                st,
                &key,
                format!(
                    "Field 'crossref' is no longer needed in set entries in Biber - ignoring in entry '{citekey}'"
                ),
            );
            del_field(st, &mut sh, &key, "crossref");
        }
    }
    sh.store(st);
    Ok(())
}

/// `Biber::preprocess_sets` (Biber.pm:1529): record set parent/child
/// relations; related clones of static set members.
pub fn preprocess_sets(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    if ctx.cfg.biber.tool() && !opt(ctx, "output_resolve_sets") {
        return Ok(());
    }
    let mut sh = SharedLists::load(st);
    for citekey in st.section.citekeys.clone() {
        let Some(key) = entry_key(&st.section, &citekey) else {
            continue;
        };
        if st.section.entries[&key].entrytype() != "set" {
            continue;
        }
        let es = st.section.entries[&key].get_field("entryset").cloned();
        if !truthy(es.as_ref()) {
            warn_entry(
                ctx,
                st,
                &key,
                format!("Set entry '{citekey}' has no entryset field, ignoring"),
            );
            continue;
        }
        let Some(Value::List(members)) = es else {
            continue;
        };
        for m in members {
            set_set(&mut st.section, "pc", &citekey, &m);
            set_set(&mut st.section, "cp", &m, &citekey);
            if let Some(k) = entry_key(&st.section, &m) {
                related::relclone(ctx, st, &mut sh, &k);
            }
        }
    }
    sh.store(st);
    super::sync_annotations(ctx, st);
    Ok(())
}

/// `Biber::calculate_interentry` (Biber.pm:1577): count crossref/xref
/// targets and promote those referenced at least `mincrossrefs`/`minxrefs`
/// times to citekeys.
///
/// biber promotes in Perl hash order (`keys %crossrefkeys`, BB-061); we
/// promote in first-reference order (DC-014).
pub fn calculate_interentry(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    let incr = |st: &mut SectionState, name: &str, k: &str, order: &mut Vec<String>| {
        let h = config_hash(st, name);
        let n = h
            .get(k)
            .and_then(J::as_str)
            .map_or(0, |s| perl_compat::truth::numify(s) as i64);
        if !h.contains_key(k) {
            order.push(k.to_string());
        }
        h.insert(k.into(), J::from((n + 1).to_string()));
    };
    // Keys counted before this phase (none in a normal run) come first.
    let existing = |st: &mut SectionState, name: &str| -> Vec<String> {
        config_hash(st, name).keys().cloned().collect()
    };
    let mut cr_order = existing(st, "crossrefkeys");
    let mut xr_order = existing(st, "xrefkeys");
    for citekey in st.section.citekeys.clone() {
        let Some(key) = entry_key(&st.section, &citekey) else {
            continue;
        };
        let be = &st.section.entries[&key];
        let cr = true_str(be, "crossref");
        let xr = true_str(be, "xref");
        if let Some(r) = cr
            && entry_key(&st.section, &r).is_some()
        {
            incr(st, "crossrefkeys", &r, &mut cr_order);
        }
        if let Some(r) = xr {
            incr(st, "xrefkeys", &r, &mut xr_order);
        }
    }
    for (name, order, min, flag) in [
        (
            "crossrefkeys",
            cr_order,
            ctx.cfg.biber.mincrossrefs(),
            "crossrefsource",
        ),
        ("xrefkeys", xr_order, ctx.cfg.biber.minxrefs(), "xrefsource"),
    ] {
        for k in order {
            let n = config_hash(st, name)
                .get(&k)
                .and_then(J::as_str)
                .map_or(0.0, perl_compat::truth::numify);
            if n >= min {
                if !st.section.has_citekey_alias(&k)
                    && let Some(ek) = entry_key(&st.section, &k)
                {
                    st.section
                        .entries
                        .get_mut(&ek)
                        .expect("entry")
                        .set_field(flag, Some(Value::Str("1".into())));
                }
                st.section.add_citekeys(std::slice::from_ref(&k));
            }
        }
    }
    Ok(())
}

/// `Biber::process_interentry` (Biber.pm:1654): crossref inheritance.
pub fn process_interentry(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    if ctx.cfg.biber.tool() && !opt(ctx, "output_resolve_crossrefs") {
        return Ok(());
    }
    let r = inherit::process_interentry(ctx, st);
    super::sync_annotations(ctx, st);
    r
}

/// `Biber::postprocess_sets` → `process_sets` (Biber.pm:1946, 2487): force
/// the "dataonly" options on set members and point them to their set.
pub fn postprocess_sets(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    let mut sh = SharedLists::load(st);
    let secnum = st.section.number.clone();
    let dataonly: Vec<String> = DATAONLY.iter().map(|s| s.to_string()).collect();
    for citekey in st.section.citekeys.clone() {
        // `keys` order in biber (irrelevant to the result); sorted here.
        let children = get_set(&mut st.section, "pc", &citekey);
        if !children.is_empty() {
            for m in children {
                super::fetch::process_entry_options(ctx, &m, &dataonly, &secnum);
                let Some(mk) = entry_key(&st.section, &m) else {
                    continue;
                };
                if truthy(st.section.entries[&mk].get_datafield("entryset")) {
                    warn_entry(
                        ctx,
                        st,
                        &mk,
                        format!(
                            "Field 'entryset' is no longer needed in set member entries in Biber - ignoring in entry '{m}'"
                        ),
                    );
                    del_field(st, &mut sh, &mk, "entryset");
                }
                st.section
                    .entries
                    .get_mut(&mk)
                    .expect("entry")
                    .set_field("entryset", Some(Value::List(vec![citekey.clone()])));
            }
        } else if !get_set(&mut st.section, "cp", &citekey).is_empty() {
            super::fetch::process_entry_options(ctx, &citekey, &dataonly, &secnum);
        }
    }
    sh.store(st);
    Ok(())
}

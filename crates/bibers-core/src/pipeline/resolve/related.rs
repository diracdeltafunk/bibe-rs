//! Related entries: `Biber::Entry::relclone` and `clone` (Entry.pm:63-207).

use super::{DATAONLY, SharedLists, alias, datefield_stems, entry_key, set_own_datafield, shared};
use crate::model::{SectionState, Value};
use crate::pipeline::Ctx;
use crate::pipeline::fetch::{SecExt, collapse_eq, process_entry_options};
use md5::{Digest, Md5};
use perl_compat::perl_truthy;
use serde_json::Value as J;

/// The derived date fields `clone` copies (Entry.pm:177-180): `yeardivision`
/// is listed twice and `endyeardivision` is missing (BB-093, quirk
/// `clone-endyeardivision`), so clones lose it. With the quirk fixed the
/// second `yeardivision` is read as `endyeardivision`.
const CLONE_DATE_META: [&str; 12] = [
    "dateunspecified",
    "datesplit",
    "datejulian",
    "enddatejulian",
    "dateapproximate",
    "enddateapproximate",
    "dateuncertain",
    "enddateuncertain",
    "yeardivision",
    "yeardivision",
    "era",
    "endera",
];

/// `md5_hex(encode_utf8($key))`: the key of a related entry clone.
pub fn clone_key(key: &str) -> String {
    let d = Md5::digest(key.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// `merge_entry_options($opts, $overrideopts)` (Utils.pm:1312).
pub fn merge_entry_options(opts: &[String], over: &[String]) -> Vec<String> {
    // m/^([^=]+)(=?)(.*)$/ (no match: all parts undef, i.e. empty)
    fn split(s: &str) -> (String, String, String) {
        if s.contains('\n') || s.is_empty() || s.starts_with('=') {
            return Default::default();
        }
        match s.find('=') {
            Some(p) => (s[..p].into(), "=".into(), s[p + 1..].into()),
            None => (s.into(), String::new(), String::new()),
        }
    }
    let mut merged = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for ov in opts {
        let (o, e, v) = split(ov);
        let mut overridden = false;
        for oov in over {
            let (oo, eo, vo) = split(oov);
            if o == oo {
                overridden = true;
                let x = format!("{oo}{eo}{vo}");
                merged.push(x.clone());
                used.push(x);
                break;
            }
        }
        if !overridden {
            merged.push(format!("{o}{e}{v}"));
        }
    }
    for oov in over {
        if !used.iter().any(|u| u == oov) {
            merged.push(oov.clone());
        }
    }
    merged
}

/// `$entry->relclone` (Entry.pm:63): create (recursively) clones of the
/// entries in `key`'s `related` field, make them citekeys and point
/// `related` to them.
pub fn relclone(ctx: &mut Ctx, st: &mut SectionState, sh: &mut SharedLists, key: &str) {
    let Some(e) = st.section.entries.get(key) else {
        return;
    };
    let relkeys = match e.get_field("related") {
        Some(Value::List(l)) => l.clone(),
        _ => return,
    };
    let secnum = st.section.number.clone();
    let mut clonekeys = Vec::new();
    for relkey in relkeys {
        let relkey = alias(&st.section, &relkey).map_or(relkey.clone(), str::to_string);
        // Loop avoidance: a clone of this key exists already.
        let existing = st
            .section
            .slot_get("keytorelclone", &relkey)
            .and_then(J::as_str)
            .filter(|c| perl_truthy(c))
            .map(str::to_string);
        if let Some(ck) = existing {
            clonekeys.push(ck);
            continue;
        }
        // biber dies on a missing related entry (they are pruned earlier).
        let Some(relentry) = entry_key(&st.section, &relkey) else {
            continue;
        };
        let ck = clone_key(&relkey);
        clonekeys.push(ck.clone());
        clone(ctx, st, sh, &relentry, &ck);
        let relopts = st.section.entries[key].get_field("relatedoptions").cloned();
        if let Some(relopts) = relopts {
            let ro = match relopts {
                Value::List(l) => l,
                Value::Str(s) => vec![s],
                _ => Vec::new(),
            };
            // A directly cited related entry must not appear twice.
            if st.section.has_citekey_alias(&relkey) {
                let mut m = merge_entry_options(&ro, &["skipbib".into(), "skipbiblist".into()]);
                process_entry_options(ctx, &ck, &m, &secnum);
                m.iter_mut().for_each(|o| *o = collapse_eq(o));
                set_own_datafield(st, sh, &ck, "options", Some(Value::List(m)));
            } else {
                // process_entry_options rewrites the elements of the
                // parent's own relatedoptions array, which becomes the
                // clone's options.
                process_entry_options(ctx, &ck, &ro, &secnum);
                super::mutate_list(st, sh, key, "relatedoptions", |l| {
                    l.iter_mut().for_each(|o| *o = collapse_eq(o))
                });
                let v = st.section.entries[key].get_field("relatedoptions").cloned();
                let from_data = st.section.entries[key]
                    .datafields
                    .get("relatedoptions")
                    .is_some_and(Option::is_some);
                if from_data {
                    sh.share(&ck, "options", key, "relatedoptions");
                } else {
                    sh.unshare(&ck, "options");
                }
                if let Some(c) = st.section.entries.get_mut(&ck) {
                    c.set_datafield("options", v);
                }
            }
        } else {
            let relentry_opts = match st.section.entries[&relentry].get_field("options") {
                Some(Value::List(l)) => Some(l.clone()),
                Some(Value::Str(s)) => Some(vec![s.clone()]),
                _ => None,
            };
            let dataonly: Vec<String> = DATAONLY.iter().map(|s| s.to_string()).collect();
            let mut m = match relentry_opts {
                Some(o) => merge_entry_options(&dataonly, &o),
                None => dataonly,
            };
            process_entry_options(ctx, &ck, &m, &secnum);
            m.iter_mut().for_each(|o| *o = collapse_eq(o));
            set_own_datafield(st, sh, &ck, "options", Some(Value::List(m)));
        }
        // keytorelclone($relkey, $clonekey)
        st.section
            .slot_obj("keytorelclone")
            .insert(relkey.clone(), J::from(ck.clone()));
        st.section
            .slot_obj("relclonetokey")
            .insert(ck.clone(), J::from(relkey.clone()));
        // Cascading related entries.
        relclone(ctx, st, sh, &ck);
    }
    st.section.add_citekeys(&clonekeys);
    set_own_datafield(st, sh, key, "related", Some(Value::List(clonekeys)));
}

/// `$entry->clone($newkey)` (Entry.pm:159), added to the section as
/// `newkey` (`add_entry`). Data fields are copied by reference.
fn clone(ctx: &mut Ctx, st: &mut SectionState, sh: &mut SharedLists, src: &str, newkey: &str) {
    let stems = datefield_stems(ctx);
    // `each %{$self->{origfields}}` autovivifies the source's origfields.
    st.section
        .entries
        .get_mut(src)
        .expect("entry")
        .slots
        .entry("origfields".into())
        .or_insert_with(|| J::Object(Default::default()));
    let s = &st.section.entries[src];
    let mut new = crate::model::Entry {
        datafields: s.datafields.clone(),
        ..Default::default()
    };
    if let Some(J::Object(o)) = s.slots.get("origfields")
        && !o.is_empty()
    {
        new.slots.insert("origfields".into(), J::Object(o.clone()));
    }
    new.slots.insert(
        "xdatarefs".into(),
        s.slots.get("xdatarefs").cloned().unwrap_or(J::Null),
    );
    for df in &stems {
        for (i, dsf) in CLONE_DATE_META.into_iter().enumerate() {
            // The duplicated `yeardivision` (index 9) is biber's typo for
            // `endyeardivision`.
            let dsf = if i == 9
                && !ctx
                    .cfg
                    .quirks
                    .on(crate::quirks::Quirk::CloneEndyeardivision)
            {
                "endyeardivision"
            } else {
                dsf
            };
            let f = format!("{df}{dsf}");
            if let Some(Some(v)) = s.derivedfields.get(&f)
                && super::truthy(Some(v))
            {
                new.derivedfields.insert(f, Some(v.clone()));
            }
        }
    }
    for f in ["entrytype", "datatype"] {
        new.derivedfields
            .insert(f.into(), s.derivedfields.get(f).cloned().flatten());
    }
    if perl_truthy(newkey) {
        new.derivedfields
            .insert("citekey".into(), Some(Value::Str(newkey.into())));
    }
    let srckey = s.get_field("citekey").cloned();
    let srccite = s.citekey().to_string();
    new.derivedfields.insert("clonesourcekey".into(), srckey);
    let shared_fields: Vec<String> = new
        .datafields
        .iter()
        .filter(|(_, v)| v.as_ref().is_some_and(shared::is_array))
        .map(|(k, _)| k.clone())
        .collect();
    ctx.run.annotations.copy_annotations(&srccite, newkey);
    if let Some(old) = st.section.entries.insert(newkey.to_string(), new) {
        sh.unshare_entry(newkey, &old);
    }
    for f in shared_fields {
        sh.share(newkey, &f, src, &f);
    }
}

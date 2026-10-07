//! The per-list driver (`Biber::process_lists`), list filters, and context
//! data (`generate_contextdata`: extra*, singletitle, uniquetitle, …).

use super::{Ctx, LIST_PHASES};
use crate::model::{Entry, Value};
use crate::perlish::{lc, truthy};
use crate::quirks::Quirk;
use crate::{log::Fatal, model::SectionState};

/// `Biber::process_lists` (Biber.pm:3085-3183): for every data list of the
/// section, reset its state, set its keys to the section's citekeys, then run
/// the per-list phases, the list filters, sorting and context data.
///
/// Tool mode (deferred, DC-030) skips the uniqueness/entry passes; it is not
/// supported yet, so every phase runs.
pub fn process_lists(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    for i in 0..st.datalists.len() {
        init_list(st, i);
        for (name, phase) in LIST_PHASES {
            if *name == "generate_sortdataschema" {
                // Filtering must come before sorting/labelling so that there
                // are no gaps in e.g. extradate (Biber.pm:3134).
                filter_list(ctx, st, i)?;
            }
            phase(ctx, st, i)?;
        }
    }
    Ok(())
}

/// The start of each iteration of `process_lists` (Biber.pm:3091-3113):
/// reset the list's state, default its template names, set its keys to the
/// section's citekeys, and mark uniqueness as changed.
pub fn init_list(st: &mut SectionState, i: usize) {
    let citekeys = st.section.citekeys.clone();
    let dl = &mut st.datalists[i];
    // `reset_state`: essential when a section is processed again.
    dl.state = Some(serde_json::Value::Object(Default::default()));
    for t in [
        &mut dl.sortingnamekeytemplatename,
        &mut dl.uniquenametemplatename,
        &mut dl.labelalphanametemplatename,
        &mut dl.namehashtemplatename,
    ] {
        if t.is_empty() {
            *t = "global".into();
        }
    }
    dl.keys = Some(citekeys);
    // `set_unul_changed(1)`: starts the uniqueness iteration.
    dl.slots
        .insert("unulchanged".into(), serde_json::Value::from("1"));
}

/// The list filtering block of `process_lists` (Biber.pm:3134-3168),
/// including `check_list_filter` (Biber.pm:3190) and the set-member re-add
/// (BB-018, `filter-readds-sets`).
pub fn filter_list(ctx: &mut Ctx, st: &mut SectionState, list: usize) -> Result<(), Fatal> {
    let dl = &st.datalists[list];
    // `if (my $filters = $list->get_filters)`: any array is true.
    let Some(serde_json::Value::Array(filters)) = dl.slots.get("filters") else {
        return Ok(());
    };
    let sec = &st.section;
    let keys = dl.keys.clone().unwrap_or_default();
    let one = |k: &str, f: &serde_json::Value, be: &Entry| -> bool {
        let g = |x: &str| f.get(x).and_then(serde_json::Value::as_str).unwrap_or("");
        check_list_filter(k, g("type"), g("value"), be)
    };
    let mut flist: Vec<String> = Vec::new();
    'keys: for k in &keys {
        let Some(be) = sec.bibentry(k) else { continue };
        for f in filters {
            let ok = match f {
                serde_json::Value::Array(alts) => alts.iter().any(|a| one(k, a, be)),
                _ => one(k, f, be),
            };
            if !ok {
                continue 'keys;
            }
        }
        flist.push(k.clone());
    }
    // Re-add set members of sets which survived the filter.
    let sets: Vec<&String> = flist
        .iter()
        .filter(|k| sec.bibentry(k).is_some_and(|e| e.entrytype() == "set"))
        .collect();
    let has_entryset = |e: &Entry| match e.get_field("entryset") {
        Some(Value::Str(s)) => truthy(Some(s)),
        Some(_) => true,
        None => false,
    };
    let mut readd = Vec::new();
    for k in &keys {
        let Some(be) = sec.bibentry(k) else { continue };
        let add = if ctx.cfg.quirks.on(Quirk::FilterReaddsSets) {
            // `first {$be->get_field('entryset')} @sets` ignores `$_`: the
            // first set key is returned (and tested for truth) whenever the
            // entry has an `entryset` at all.
            has_entryset(be) && sets.first().is_some_and(|s| truthy(Some(s)))
        } else {
            sets.iter().any(|s| {
                sec.bibentry(s)
                    .and_then(|e| e.get_field("entryset"))
                    .and_then(Value::as_list)
                    .is_some_and(|m| m.iter().any(|x| x == k))
            })
        };
        if add {
            readd.push(k.clone());
        }
    }
    flist.extend(readd);
    // `uniq`
    let mut seen = std::collections::HashSet::new();
    flist.retain(|k| seen.insert(k.clone()));
    st.datalists[list].keys = Some(flist);
    Ok(())
}

/// `check_list_filter` (Biber.pm:3190).
fn check_list_filter(_k: &str, t: &str, fs: &str, be: &Entry) -> bool {
    let lfs = lc(fs);
    let subtype_is = || be.field_exists("entrysubtype") && be.get_str("entrysubtype") == Some(&lfs);
    let has_keyword = || {
        // `first {$_ eq $keyword} @$keywords`: a found keyword `0` is false.
        be.datafields
            .get("keywords")
            .and_then(Option::as_ref)
            .and_then(Value::as_list)
            .is_some_and(|kw| kw.iter().any(|x| x == fs) && truthy(Some(fs)))
    };
    match t {
        "type" => be.entrytype() == lfs,
        "nottype" => be.entrytype() != lfs,
        "subtype" => subtype_is(),
        "notsubtype" => !subtype_is(),
        "keyword" => has_keyword(),
        "notkeyword" => !has_keyword(),
        "field" => be.field_exists(fs),
        "notfield" => !be.field_exists(fs),
        _ => true,
    }
}

/// `Biber::generate_contextdata` (Biber.pm:3972), for the data list
/// `st.datalists[list]`. Not yet implemented: a no-op.
pub fn generate_contextdata(
    _ctx: &mut Ctx,
    _st: &mut SectionState,
    _list: usize,
) -> Result<(), Fatal> {
    Ok(())
}

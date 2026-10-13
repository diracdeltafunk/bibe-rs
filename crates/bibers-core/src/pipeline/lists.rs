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
/// In tool mode (also set for `--output-format=bibtex`, Biber.pm:1133) only
/// `process_visible_names`, filtering and sorting run (Biber.pm:3112-3177).
pub fn process_lists(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    let tool = ctx.cfg.biber.tool();
    for i in 0..st.datalists.len() {
        init_list(st, i);
        if tool {
            // `set_unul_changed(1)` is inside `unless tool`.
            st.datalists[i].slots.remove("unulchanged");
        }
        for (name, phase) in LIST_PHASES {
            if tool
                && matches!(
                    *name,
                    "process_entries_pre"
                        | "uniqueness"
                        | "process_entries_post"
                        | "process_entries_final"
                        | "generate_contextdata"
                )
            {
                continue;
            }
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
/// `st.datalists[list]`: in sorted list order, sorted set members, the
/// extraname/extradate/extratitle/extratitleyear/extraalpha numbers, the
/// labelalpha copy, and the per-namepart uniquename values of the labelname.
pub fn generate_contextdata(
    ctx: &mut Ctx,
    st: &mut SectionState,
    list: usize,
) -> Result<(), Fatal> {
    use super::entries::{H, info_slot, ls, uniquename_value};
    use serde_json::Value as J;
    let secnum = st.section.number.clone();
    let h = H::new(ctx.cfg, ctx.dm, &secnum);
    let opt_undef = |opt: &str, et: &str, key: &str| {
        h.cfg
            .blx
            .get_undef_secnum(h.cfg.quirks, &secnum, opt, Some(et), Some(key))
            .is_some_and(crate::config::OptValue::truthy)
    };
    let sortsets = h
        .cfg
        .blx
        .global("sortsets")
        .is_some_and(crate::config::OptValue::truthy);
    let sec = &st.section;
    let dl = &mut st.datalists[list];
    let keys = dl.keys.clone().unwrap_or_default();
    let jstr = |s: &str| J::String(s.to_string());
    for key in &keys {
        let Some(be) = sec.bibentry(key) else {
            continue;
        };
        let et = be.entrytype();
        let lni = info_slot(be, "labelnameinfo");

        if et == "set" {
            let members: Vec<String> = be
                .get_field("entryset")
                .and_then(Value::as_list)
                .map(<[String]>::to_vec)
                .unwrap_or_default();
            let es: Vec<String> = if sortsets {
                // `first_index` gives -1 for a missing member, and the
                // slice then takes the last key.
                let mut idx: Vec<i64> = members
                    .iter()
                    .map(|m| keys.iter().position(|k| k == m).map_or(-1, |i| i as i64))
                    .collect();
                idx.sort_unstable();
                idx.iter()
                    .filter_map(|&i| {
                        if i < 0 {
                            keys.len().checked_sub(i.unsigned_abs() as usize)
                        } else {
                            Some(i as usize)
                        }
                    })
                    .filter_map(|i| keys.get(i).cloned())
                    .collect()
            } else {
                members
            };
            ls::set_entryfield(
                dl,
                key,
                "entryset",
                J::Array(es.iter().map(|s| jstr(s)).collect()),
            );
        }

        let skiplab = h
            .cfg
            .blx
            .get(Some(&secnum), "skiplab", Some(et), Some(key))
            .is_some_and(crate::config::OptValue::truthy);
        if !skiplab {
            // extraname
            let lnh = ls::entryfield(dl, key, "labelnamehash");
            if ls::truthy(&lnh) {
                let lnh = ls::str_of(&lnh).unwrap_or_default();
                if ls::get(dl, &["seen_labelname", &lnh]).map_or(0.0, |v| ls::num(&v)) > 1.0 {
                    let v = ls::incr(dl, &["seen_extraname", &lnh]);
                    ls::set(dl, &["extranamedata", key], J::String(v.to_string()));
                }
            }
            for (opt, field, seen, extra, data) in [
                (
                    "labeldateparts",
                    "nametitledateparts",
                    "seen_nametitledateparts",
                    "seen_extradate",
                    "extradatedata",
                ),
                (
                    "labeltitle",
                    "nametitle",
                    "seen_nametitle",
                    "seen_extratitle",
                    "extratitledata",
                ),
                (
                    "labeltitleyear",
                    "titleyear",
                    "seen_titleyear",
                    "seen_extratitleyear",
                    "extratitleyeardata",
                ),
            ] {
                if !opt_undef(opt, et, key) {
                    continue;
                }
                let v = ls::str_of(&ls::entryfield(dl, key, field)).unwrap_or_default();
                if ls::get(dl, &[seen, &v]).map_or(0.0, |x| ls::num(&x)) > 1.0 {
                    let n = ls::incr(dl, &[extra, &v]);
                    ls::set(dl, &[data, key], J::String(n.to_string()));
                }
            }
            if opt_undef("labelalpha", et, key) {
                let la = ls::entryfield(dl, key, "labelalpha");
                ls::set(dl, &["labelalphadata", key], la.clone().unwrap_or(J::Null));
                let la = ls::str_of(&la).unwrap_or_default();
                if ls::get(dl, &["ladisambiguation", &la]).map_or(0.0, |x| ls::num(&x)) > 1.0 {
                    let n = ls::incr(dl, &["seen_extraalpha", &la]);
                    ls::set(dl, &["extraalphadata", key], J::String(n.to_string()));
                }
            }
        }

        // uniquename: per-namepart values for the labelname only
        for nf in &h.dm.helpers.namelists {
            let Some(Value::Names(nlid)) = be.get_field(nf) else {
                continue;
            };
            if lni != Some(nf.as_str()) {
                continue;
            }
            let nlk = nlid.key();
            for nid in &sec.names.list(*nlid).names {
                let nk = nid.key();
                let un = ls::get(dl, &["namelistdata", &nlk, &nk, "un"]);
                if !ls::truthy(&un) {
                    continue;
                }
                let schema = ls::get(dl, &["namelistdata", &nlk, &nk, "namedisschema"]);
                let schema: Vec<J> = schema
                    .and_then(|s| s.as_array().cloned())
                    .unwrap_or_default();
                let mut pnun: std::collections::BTreeMap<String, String> = Default::default();
                for (i, nss) in schema.iter().enumerate() {
                    if Some(nss) == un.as_ref() {
                        let mut dis: Vec<&J> = schema
                            .get(1..i.max(1))
                            .unwrap_or(&[])
                            .iter()
                            .filter(|e| {
                                e.get(0).and_then(J::as_str) != Some("base")
                                    && e.get(1).and_then(J::as_str) != Some("full")
                            })
                            .collect();
                        dis.push(nss);
                        pnun = dis
                            .iter()
                            .filter_map(|e| {
                                let np = e.get(0).and_then(J::as_str)?;
                                let v = e.get(1).and_then(J::as_str).unwrap_or("");
                                Some((
                                    np.to_string(),
                                    if v == "fullonly" { "full" } else { v }.to_string(),
                                ))
                            })
                            .collect();
                        break;
                    }
                }
                for np in sec.names.name(*nid).nameparts.keys() {
                    let v =
                        uniquename_value(pnun.get(np).map_or("none", String::as_str)).unwrap_or(0);
                    ls::set(
                        dl,
                        &["namelistdata", &nlk, &nk, "unparts", np],
                        J::String(v.to_string()),
                    );
                }
            }
        }
    }
    Ok(())
}

//! The per-list driver (`Biber::process_lists`), list filters, and context
//! data (`generate_contextdata`: extra*, singletitle, uniquetitle, …).

use super::{Ctx, LIST_PHASES};
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
/// (BB-018 quirk). Not yet implemented: a no-op (lists keep all keys).
pub fn filter_list(_ctx: &mut Ctx, _st: &mut SectionState, _list: usize) -> Result<(), Fatal> {
    Ok(())
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

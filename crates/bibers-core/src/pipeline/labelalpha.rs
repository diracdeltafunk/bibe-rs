//! Alphabetic labels: `process_labelalpha` / `process_extraalpha`
//! (Biber.pm:3020, 3043) and the label template machinery of
//! `Biber::Internals` (`_genlabel`, `_labelpart`, `_label_name`,
//! `_process_label_attributes`, `_label_listdisambiguation`, …). Called from
//! `entries::process_entries_post`.

use super::Ctx;
use crate::{log::Fatal, model::SectionState};

/// `Biber::process_labelalpha` (Biber.pm:3020) for entry `citekey` in the
/// data list `st.datalists[list]`. Not yet implemented: a no-op.
pub fn process_labelalpha(
    _ctx: &mut Ctx,
    _st: &mut SectionState,
    _list: usize,
    _citekey: &str,
) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::process_extraalpha` (Biber.pm:3043) for entry `citekey` in the
/// data list `st.datalists[list]`. Not yet implemented: a no-op.
pub fn process_extraalpha(
    _ctx: &mut Ctx,
    _st: &mut SectionState,
    _list: usize,
    _citekey: &str,
) -> Result<(), Fatal> {
    Ok(())
}

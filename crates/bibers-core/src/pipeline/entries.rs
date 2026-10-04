//! Per-entry processing: `process_entries_static` (labelname, labeldate,
//! labeltitle, …), the per-list `process_entries_pre`/`post`/`final`
//! passes (hashes, labelalpha, extra* bookkeeping, …) and visible names.

use super::Ctx;
use crate::{log::Fatal, model::SectionState};

/// `Biber::process_entries_static` (Biber.pm:1966). Not yet implemented: a no-op.
pub fn process_entries_static(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::process_entries_pre` (Biber.pm:2000), for the data list `st.datalists[list]`.
/// Not yet implemented: a no-op.
pub fn process_entries_pre(
    _ctx: &mut Ctx,
    _st: &mut SectionState,
    _list: usize,
) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::process_visible_names` (Biber.pm:2892), for the data list `st.datalists[list]`.
/// Not yet implemented: a no-op.
pub fn process_visible_names(
    _ctx: &mut Ctx,
    _st: &mut SectionState,
    _list: usize,
) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::process_entries_post` (Biber.pm:2049), for the data list `st.datalists[list]`.
/// Not yet implemented: a no-op.
pub fn process_entries_post(
    _ctx: &mut Ctx,
    _st: &mut SectionState,
    _list: usize,
) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::process_entries_final` (Biber.pm:2107), for the data list `st.datalists[list]`.
/// Not yet implemented: a no-op.
pub fn process_entries_final(
    _ctx: &mut Ctx,
    _st: &mut SectionState,
    _list: usize,
) -> Result<(), Fatal> {
    Ok(())
}

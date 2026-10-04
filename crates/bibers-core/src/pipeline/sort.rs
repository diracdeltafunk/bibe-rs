//! Sorting: the sort data schema, per-entry sort keys, the multi-key
//! sort, and sortinit/sortinithash.

use super::Ctx;
use crate::{log::Fatal, model::SectionState};

/// `Biber::generate_sortdataschema` (Biber.pm:3304), for the data list `st.datalists[list]`.
/// Not yet implemented: a no-op.
pub fn generate_sortdataschema(
    _ctx: &mut Ctx,
    _st: &mut SectionState,
    _list: usize,
) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::generate_sortinfo` (Biber.pm:3347), for the data list `st.datalists[list]`.
/// Not yet implemented: a no-op.
pub fn generate_sortinfo(
    _ctx: &mut Ctx,
    _st: &mut SectionState,
    _list: usize,
) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::sort_list` (Biber.pm:4249), for the data list `st.datalists[list]`.
/// Not yet implemented: a no-op.
pub fn sort_list(_ctx: &mut Ctx, _st: &mut SectionState, _list: usize) -> Result<(), Fatal> {
    Ok(())
}

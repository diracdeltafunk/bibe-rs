//! uniquename / uniquelist (the iterative `Biber::uniqueness` loop).

use super::Ctx;
use crate::{log::Fatal, model::SectionState};

/// `Biber::uniqueness` (Biber.pm:3362), for the data list `st.datalists[list]`.
/// Not yet implemented: a no-op.
pub fn uniqueness(_ctx: &mut Ctx, _st: &mut SectionState, _list: usize) -> Result<(), Fatal> {
    Ok(())
}

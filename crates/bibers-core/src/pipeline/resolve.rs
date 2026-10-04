//! Dependency resolution: citekey aliases, dynamic sets and related
//! entries, XDATA, sets, crossref/xref inheritance, data model validation.

use super::Ctx;
use crate::{log::Fatal, model::SectionState};

/// `Biber::process_citekey_aliases` (Biber.pm:1344). Not yet implemented: a no-op.
pub fn process_citekey_aliases(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::instantiate_dynamic` (Biber.pm:1368). Not yet implemented: a no-op.
pub fn instantiate_dynamic(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::resolve_alias_refs` (Biber.pm:1286). Not yet implemented: a no-op.
pub fn resolve_alias_refs(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::resolve_xdata` (Biber.pm:1433). Not yet implemented: a no-op.
pub fn resolve_xdata(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::cite_setmembers` (Biber.pm:1467). Not yet implemented: a no-op.
pub fn cite_setmembers(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::preprocess_sets` (Biber.pm:1529). Not yet implemented: a no-op.
pub fn preprocess_sets(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::calculate_interentry` (Biber.pm:1577). Not yet implemented: a no-op.
pub fn calculate_interentry(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::process_interentry` (Biber.pm:1654). Not yet implemented: a no-op.
pub fn process_interentry(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::validate_datamodel` (Biber.pm:1702). Not yet implemented: a no-op.
pub fn validate_datamodel(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

/// `Biber::postprocess_sets` (Biber.pm:1946). Not yet implemented: a no-op.
pub fn postprocess_sets(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

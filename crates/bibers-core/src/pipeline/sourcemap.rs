//! Source maps (`\DeclareSourcemap`, style/driver maps, biber.conf
//! `<sourcemap>`): the map engine of `Biber::Input::file::bibtex::create_entry`
//! (bibtex.pm:356-937), applied to a data source entry before field handlers
//! run. Used by [`super::fetch`].
//!
//! Interface contract between `fetch` and this module: `fetch` turns each
//! parsed data source entry into a [`RawEntry`] (as Text::BibTeX exposes it to
//! biber after its second parse pass), calls [`apply_maps`], then runs field
//! handlers on the (possibly modified) entry and on any new entries the maps
//! created. This module owns [`RawEntry`]'s exact semantics and may extend it.

use super::Ctx;
use crate::log::Fatal;
use crate::model::SectionState;

/// A data source entry during source mapping (biber's `Text::BibTeX::Entry`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawEntry {
    /// Entry type as Text::BibTeX reports it.
    pub entrytype: String,
    /// The citation key.
    pub key: String,
    /// Fields in source order: (name as Text::BibTeX reports it, value).
    pub fields: Vec<(String, String)>,
}

/// What mapping did to an entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapOutcome {
    /// The entry was deleted (`entry_null`) and must not be created.
    pub skip: bool,
    /// Entries created by `entry_new`/`entry_clone` steps.
    pub new_entries: Vec<RawEntry>,
}

/// Apply all applicable source maps (user, style, then the first driver
/// map for the data source's datatype) to `entry`. Not yet implemented: maps
/// nothing.
pub fn apply_maps(
    _ctx: &mut Ctx,
    _st: &mut SectionState,
    _datatype: &str,
    _entry: &mut RawEntry,
) -> Result<MapOutcome, Fatal> {
    Ok(MapOutcome::default())
}

//! The `.bbl` writer (`Biber::Output::bbl`, bbl.pm), producing exactly
//! biber's text: see `docs/research/05-output-and-testing.md` §2 for the
//! format specification.

use crate::log::Fatal;
use crate::pipeline::{Ctx, Processed};

/// Render the whole `.bbl` (header, `\preamble`, one `\refsection` per
/// processed section, `\endinput`). Returns `None` while the writer is not
/// implemented.
pub fn render(_ctx: &mut Ctx, _processed: &Processed) -> Result<Option<String>, Fatal> {
    Ok(None)
}

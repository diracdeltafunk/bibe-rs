//! Output writers: the `.bbl` ([`bbl`]) and bibtex data output ([`bibtex`],
//! tool mode and `--output-format=bibtex`). `bblxml`/`dot` are dropped and
//! biblatexml output is deferred (docs/compat.md DC-020…DC-031).

pub mod bbl;
pub mod bibtex;

use crate::log::Fatal;
use crate::pipeline::{Ctx, Processed};

/// An output format (`--output-format`) bibers can write. Data formats
/// (written in tool mode) render the `99999` pseudo-section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    /// `bbl` (`Biber::Output::bbl`).
    Bbl,
    /// `bibtex` (`Biber::Output::bibtex`).
    Bibtex,
}

impl OutputFormat {
    /// The format named by `--output-format`, if bibers writes it.
    pub fn from_name(name: &str) -> Option<OutputFormat> {
        match name {
            "bbl" => Some(OutputFormat::Bbl),
            "bibtex" => Some(OutputFormat::Bibtex),
            _ => None,
        }
    }

    /// Whether the output file is opened (and so truncated) before the
    /// control file is read (`set_output_target_file`, bin/biber:397): the
    /// `.bbl` writer opens it early (base.pm:56); the data writers open it
    /// only in `output` (bibtex.pm:38-43, 422-434), so that tool mode can
    /// write over its own input.
    pub fn opens_output_early(self) -> bool {
        matches!(self, OutputFormat::Bbl)
    }

    /// The message logged with `--output-safechars` (bbl.pm:779,
    /// bibtex.pm:441).
    pub fn safechars_message(self) -> &'static str {
        match self {
            OutputFormat::Bbl => "Converting UTF-8 to TeX macros on output to .bbl",
            OutputFormat::Bibtex => "Converting UTF-8 to TeX macros on output",
        }
    }

    /// Render the processed sections in this format, before encoding.
    /// `Ok(None)` if the writer cannot produce output.
    pub fn render(self, ctx: &mut Ctx, processed: &Processed) -> Result<Option<String>, Fatal> {
        match self {
            OutputFormat::Bbl => bbl::render(ctx, processed),
            OutputFormat::Bibtex => bibtex::render(ctx, processed).map(Some),
        }
    }
}

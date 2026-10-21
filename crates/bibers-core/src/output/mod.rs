//! Output writers: the `.bbl` ([`bbl`]), bibtex data output ([`bibtex`],
//! tool mode and `--output-format=bibtex`) and biblatexml data output
//! ([`biblatexml`], tool mode). `bblxml`/`dot` are dropped (docs/compat.md
//! DC-020/DC-021).

pub mod bbl;
pub mod biblatexml;
pub mod bibtex;

use crate::log::Fatal;
use crate::pipeline::{Ctx, Processed};

/// Output text in pieces: the output is their concatenation. A large
/// `.bbl` is rendered entry by entry, and keeping the entries' pieces
/// instead of copying them into one string halves its memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pieces: Vec<String>,
}

impl Text {
    /// Append a piece.
    pub fn push(&mut self, s: String) {
        if !s.is_empty() {
            self.pieces.push(s);
        }
    }

    /// The pieces, in order.
    pub fn pieces(&self) -> &[String] {
        &self.pieces
    }

    /// The pieces, in order.
    pub fn into_pieces(self) -> Vec<String> {
        self.pieces
    }

    /// The length in bytes.
    pub fn len(&self) -> usize {
        self.pieces.iter().map(String::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// The whole text as one string.
    pub fn into_string(self) -> String {
        if self.pieces.len() == 1 {
            return self.pieces.into_iter().next().expect("one piece");
        }
        let mut s = String::with_capacity(self.len());
        for p in self.pieces {
            s.push_str(&p);
        }
        s
    }
}

impl From<String> for Text {
    fn from(s: String) -> Text {
        let mut t = Text::default();
        t.push(s);
        t
    }
}

/// An output format (`--output-format`) bibers can write. Data formats
/// (written in tool mode) render the `99999` pseudo-section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    /// `bbl` (`Biber::Output::bbl`).
    Bbl,
    /// `bibtex` (`Biber::Output::bibtex`).
    Bibtex,
    /// `biblatexml` (`Biber::Output::biblatexml`, tool mode only).
    Biblatexml,
}

impl OutputFormat {
    /// The format named by `--output-format`, if bibers writes it.
    pub fn from_name(name: &str) -> Option<OutputFormat> {
        match name {
            "bbl" => Some(OutputFormat::Bbl),
            "bibtex" => Some(OutputFormat::Bibtex),
            "biblatexml" => Some(OutputFormat::Biblatexml),
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
            OutputFormat::Bibtex | OutputFormat::Biblatexml => {
                "Converting UTF-8 to TeX macros on output"
            }
        }
    }

    /// Render the processed sections in this format, before encoding.
    /// `Ok(None)` if the writer cannot produce output.
    /// The `.bbl` writer frees the entries it has written: `processed` is
    /// only good for writing after this.
    pub fn render(self, ctx: &mut Ctx, processed: &mut Processed) -> Result<Option<Text>, Fatal> {
        match self {
            OutputFormat::Bbl => bbl::render_text(ctx, processed),
            OutputFormat::Bibtex => bibtex::render(ctx, processed).map(|t| Some(t.into())),
            OutputFormat::Biblatexml => biblatexml::render(ctx, processed).map(|t| Some(t.into())),
        }
    }

    /// Whether the text is encoded with `--output-encoding` (and
    /// `--output-safechars` applies). The biblatexml writer always writes
    /// UTF-8 and never converts to TeX macros (Output/biblatexml.pm:69-72).
    pub fn encodes_output(self) -> bool {
        !matches!(self, OutputFormat::Biblatexml)
    }

    /// What a writer does after the output file is written: the biblatexml
    /// writer's schema and `--validate-bltxml` (Output/biblatexml.pm:571-590).
    pub fn after_write(self, ctx: &mut Ctx, outfile: &str) -> Result<(), Fatal> {
        match self {
            OutputFormat::Biblatexml => biblatexml::after_write(ctx, outfile),
            _ => Ok(()),
        }
    }
}

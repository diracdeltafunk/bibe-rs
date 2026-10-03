//! Diagnostics in btparse's exact textual form (spec §8).

use std::fmt;

/// How btparse classifies a message. The rendered prefix depends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagKind {
    /// `<file>, line <n>, warning: <msg>`
    Warning,
    /// `<file>, line <n>, error: <msg>`
    Error,
    /// `<file>, line <n>, syntax error: <msg>`
    SyntaxError,
    /// `invalid token near line <n> (text was '<text>')` (no file name).
    /// `message` holds the token text.
    InvalidToken,
    /// A location-less message printed verbatim, e.g. `warning: unmatched '{' (ignoring)`
    /// from name splitting, or `substring 2, warning: empty substring` from `split_list`.
    Bare,
}

/// One diagnostic line as libbtparse would print it on stderr.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Diag {
    pub kind: DiagKind,
    /// 1-based line number (meaningless for [`DiagKind::Bare`]).
    pub line: u32,
    pub message: String,
}

impl Diag {
    pub(crate) fn new(kind: DiagKind, line: u32, message: impl Into<String>) -> Self {
        Diag {
            kind,
            line,
            message: message.into(),
        }
    }

    /// `true` for the kinds biber turns into errors (its stderr filter matches `error:`).
    pub fn is_error(&self) -> bool {
        match self.kind {
            DiagKind::Error | DiagKind::SyntaxError => true,
            DiagKind::Bare => self.message.contains("error:"),
            _ => false,
        }
    }

    /// `true` if biber would report this line as a warning (`warning:` and not `error:`).
    pub fn is_warning(&self) -> bool {
        !self.is_error() && self.render("").contains("warning:")
    }

    /// Render exactly as btparse prints it, naming the input `file`.
    pub fn render(&self, file: &str) -> String {
        match self.kind {
            DiagKind::Warning => format!("{file}, line {}, warning: {}", self.line, self.message),
            DiagKind::Error => format!("{file}, line {}, error: {}", self.line, self.message),
            DiagKind::SyntaxError => {
                format!("{file}, line {}, syntax error: {}", self.line, self.message)
            }
            DiagKind::InvalidToken => {
                format!(
                    "invalid token near line {} (text was '{}')",
                    self.line, self.message
                )
            }
            DiagKind::Bare => self.message.clone(),
        }
    }
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render("<input>"))
    }
}

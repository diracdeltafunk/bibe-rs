//! A btparse-compatible BibTeX parser: lexing and parsing of `.bib` files,
//! the macro table, `split_list`, name splitting and name formatting, and
//! btparse-style diagnostics.
//!
//! **Licensing rule:** this crate is a *spec-first* reimplementation (see
//! `docs/PLAN.md` §9). It is written from `docs/spec/btparse.md` and black-box
//! test results only. Do not translate or paraphrase btparse's (LGPL) C source.
//!
//! Overview (section numbers refer to `docs/spec/btparse.md`):
//! - [`BtState`] holds libbtparse's process-global state (macro table and
//!   error-recovery flag); [`BtState::parse`] / [`Parser`] read one file the
//!   way biber's `while (Text::BibTeX::Entry->new($bib))` loop does (§2–§4).
//! - [`split_list`] (§5), [`split_name`] and [`format_tokens`] (§6–§7).
//! - [`compat`] reproduces biber's pass-1 re-serialisation (§9).
//!
//! The crate does no Unicode normalisation: biber feeds btparse NFD text for
//! files and NFC text for `split_list`/names; callers do the same.

mod bib;
pub mod compat;
mod diag;
mod lower_table;
mod macros;
mod names;
mod split_list;

pub use bib::{BtState, Entry, Field, Item, Metatype, ParsedFile, Parser, collapse_spaces};
pub use diag::{Diag, DiagKind};
pub use macros::{BIBER_MONTHS, MacroTable, TEXT_BIBTEX_MONTHS};
pub use names::{
    NameAnomaly, NamePart, NameParts, SplitName, format_tokens, is_lower_char, is_lowercase_token,
    split_name, token_length,
};
pub use split_list::{SplitList, split_list, split_list_desc};

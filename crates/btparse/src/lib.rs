//! A btparse-compatible BibTeX parser: lexing and parsing of `.bib` files,
//! the macro table, `split_list`, name splitting and name formatting, and
//! btparse-style diagnostics.
//!
//! **Licensing rule:** this crate is a *spec-first* reimplementation (see
//! `docs/PLAN.md` §9). It is written from `docs/spec/btparse.md` and black-box
//! test results only. Do not translate or paraphrase btparse's (LGPL) C source.

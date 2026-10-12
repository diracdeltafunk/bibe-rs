//! Perl-isms and small CPAN-library behaviours that leak into biber's
//! observable output, gathered in one place so every call site is explicit
//! and grep-able.
//!
//! | module | replicates | Unicode |
//! |---|---|---|
//! | [`truth`] | perl truthiness (`"0"` is false), `Scalar::Util::looks_like_number`, string→number conversion and `%.15g` output | – |
//! | [`case`] | `lc`, `uc`, `fc`, `ucfirst`, `lcfirst` (full mappings, no Final_Sigma) | 16.0 |
//! | [`unicode`] | `Unicode::Normalize` NFD/NFC/NFKD, `Unicode::UCD::num`, regex classes (`\s`, `\w`, `\p{Dash}`, …) | 16.0 |
//! | [`grapheme`] | `Unicode::GCString` (sombok, Unicode **8.0**) and perl `\X`/`\b{gcb}` (16.0) | 8.0 / 16.0 |
//! | [`regex`] | user regexes (sourcemaps, `nosort`, `noinit`, …) via PCRE2, perl `split`, `m//g` list context, `s///eg` with `qq` replacement interpolation | 16.0 (PCRE2 10.46) |
//! | [`isbn`] | `Business::ISBN` 3.012 (+ RangeMessage.xml), `Business::ISSN` 1.008, `Business::ISMN` 1.205 | – |
//! | [`uri`] | `URI->new($s)->as_string` (URI 5.34), `URI::Escape` | – |
//! | [`roman`] | `Text::Roman` 3.5 `isroman`/`roman2int` | – |
//! | [`encoding`] | `Encode` 3.21: `find_encoding` name resolution, every table-driven encoding (single-byte, CJK, EBCDIC, Mac), UTF-16/32, ISO-2022-JP/KR, HZ, `utf8`; the PerlIO `:encoding` layer | – |
//! | [`textwrap`] | `Text::Wrap::wrap` 2024.001, `Text::Tabs` `expand`/`unexpand` | – |
//! | [`sort`] | perl `cmp`/`sort` (code point order), `reduce_array` | – |
//!
//! Every function documents the biber 2.22 call sites it serves. Behaviour is
//! verified differentially against oracle O3 (perl 5.42.0 with the exact CPAN
//! versions biber 2.22 bundles): committed vectors and digests make `cargo
//! test` meaningful without perl, and `#[ignore]`d tests re-run the full
//! comparisons against the oracle (`cargo test -p perl-compat -- --ignored`).

pub mod case;
pub mod encoding;
pub mod grapheme;
pub mod isbn;
mod isbn_ranges;
pub mod regex;
pub mod roman;
pub mod sort;
mod tables;
pub mod textwrap;
pub mod truth;
pub mod unicode;
pub mod uri;

pub use case::{fc, lc, lcfirst, uc, ucfirst};
pub use truth::{looks_like_number, perl_truthy};

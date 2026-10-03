//! A Unicode Collation Algorithm implementation that reproduces the sort keys
//! of Perl's `Unicode::Collate` 1.31 (UCA 43, DUCET 13.0.0) and
//! `Unicode::Collate::Locale` 1.31 tailorings byte for byte, including
//! `viewSortKey` output. This is the collation engine of bibers (biber 2.22 bundles
//! exactly these module versions).
//!
//! Implemented from UTS #10 and the `Unicode::Collate` documentation, verified
//! by exhaustive differential testing against the Perl module (oracle O3, see
//! `tools/oracle/probes/ucollate/`).
//!
//! ```
//! use ucollate::{Collator, CollateOptions};
//!
//! // biber's default sort collator (en-US → no tailoring; sortcase, sortupper on)
//! let c = Collator::new(Some("en-US"), &CollateOptions::biber_sort(4, true)).unwrap();
//! assert_eq!(c.view_sort_key("a"), "[1FA2 | 0020 | 0008 | |]");
//! assert!(c.sort_key("Apple") < c.sort_key("apple"));
//!
//! // biber's sortinithash collator
//! let s = Collator::new(Some("en-US"), &CollateOptions::biber_sortinit()).unwrap();
//! assert_eq!(s.view_sort_key("\u{C5}"), "[1FA2 | | |]");
//! ```
//!
//! Locale data is loaded lazily and cached per process; constructing a collator for a
//! locale that is already loaded is cheap.

mod cjk;
mod collator;
mod locale;
mod options;
mod tables;
mod tailoring;

pub use collator::{Collator, view_key};
pub use locale::{file_stem, resolve_locale, tailored_locales};
pub use options::{CollateOptions, Normalization, Variable};

/// Errors from building a collator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// An invalid option value (message as `Unicode::Collate` croaks it).
    InvalidOption(String),
    /// An option `Unicode::Collate::change()` refuses
    /// (message: `change of KEY via change() is not allowed!`).
    ChangeNotAllowed(String),
    /// A valid `Unicode::Collate` option or value that this crate does not implement.
    Unsupported(String),
    /// A malformed `entry` line.
    InvalidEntry(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidOption(m) | Error::Unsupported(m) => f.write_str(m),
            Error::ChangeNotAllowed(k) => write!(f, "change of {k} via change() is not allowed!"),
            Error::InvalidEntry(m) => write!(f, "invalid collation entry: {m}"),
        }
    }
}

impl std::error::Error for Error {}

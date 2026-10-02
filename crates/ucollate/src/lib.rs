//! A Unicode Collation Algorithm implementation that reproduces the sort keys
//! of Perl's `Unicode::Collate` 1.31 (DUCET 13.0.0) and
//! `Unicode::Collate::Locale` tailorings, including `viewSortKey` output.
//!
//! Implemented from UTS #10 and the `Unicode::Collate` documentation, verified
//! by exhaustive differential testing against the Perl module (oracle O2/O3).

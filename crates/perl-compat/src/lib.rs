//! Perl-isms that leak into biber's observable behaviour, gathered in one place
//! so every call site is explicit: string truthiness (`"0"` is false), case
//! mapping, `looks_like_number`, user regexes with Perl semantics (via PCRE2),
//! Perl `qq` replacement-string interpolation, Unicode 8.0 grapheme clusters
//! (as used by `Unicode::GCString`), URI escaping, ISBN/ISSN/ISMN, and roman
//! numerals.

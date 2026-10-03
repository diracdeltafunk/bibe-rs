# ucollate

A Unicode Collation Algorithm engine that reproduces Perl's **Unicode::Collate 1.31**
(UCA 43, DUCET 13.0.0) and **Unicode::Collate::Locale 1.31** byte for byte:
`getSortKey` bytes, `viewSortKey` text and `getlocale` names. It is the collator of
bibers, because biber 2.22 sorts with exactly these module versions.

```rust
use ucollate::{CollateOptions, Collator};

// biber: Biber::UCollate->new('en-US', level => 4, variable => 'non-ignorable',
//        normalization => 'prenormalized', upper_before_lower => 1)
let c = Collator::new(Some("en-US"), &CollateOptions::biber_sort(4, true))?;
let key: Vec<u8> = c.sort_key("Doe     John  ");   // compare keys bytewise

// biber's sortinithash: md5_hex(viewSortKey(sortinit)) with
// Unicode::Collate::Locale->new(locale => $template_locale, level => 1)
let s = Collator::new(Some("en-US"), &CollateOptions::biber_sortinit())?;
assert_eq!(s.view_sort_key("Å"), "[1FA2 | | |]");
```

## API

* `Collator::new(locale: Option<&str>, &CollateOptions) -> Result<Collator, Error>`
  behaves like `Unicode::Collate::Locale->new(locale => L)->change(%opts)`.
  Locale names are resolved exactly like `_locale`: case-insensitive, `-`, `.` and space
  act as `_`, variant aliases apply, the fallback chain is followed, and anything else
  becomes `default`.
* `sort_key`, `sort_key_into` (reuses a buffer), `sort_key_from_code_points` (accepts
  surrogates and values above U+10FFFF, as Perl strings can), `cmp`, `view_sort_key`,
  `view_key(&[u8])`.
* `locale()` gives `getlocale`, plus `locale_version()`, `version()` and `uca_version()`.
* `option_value(key)` gives what `change()` reports for an option. Biber uses it for its
  "Overriding locale … defaults" INFO line.
* `CollateOptions` has typed fields. `CollateOptions::set_perl(key, value)` interprets a
  Perl key/value pair the way `change()` does: Perl numification and truthiness,
  case-insensitive `variable`, the same croak messages, `ChangeNotAllowed` for `entry`,
  `table`, `ignoreChar` and the like, and unknown keys silently ignored. Use it for
  biber's `--collate-options`.
* `resolve_locale`, `tailored_locales` and `file_stem` are exposed for diagnostics.

Supported options: `level` 1–4, `variable`/`alternate` (all four modes), `normalization`
(NFD/NFC/NFKD/NFKC under all their Perl spellings, `prenormalized`, `undef`),
`upper_before_lower`, `katakana_before_hiragana`, `backwards` (a level or a list),
`ignore_level2`, `hangul_terminator`, `identical`, `highestFFFF`, `minimalFFFE`,
`entry` (constructor only, added after the locale's entries) and `suppress`
(constructor only, replaces the locale's list).

**Not supported** (`Error::Unsupported`): `UCA_Version` other than 43,
`long_contraction => 1`, normalization forms `FCD`/`FCC`, and the code-reference
options `preprocess`, `rearrange`, `overrideCJK`, `overrideHangul` and `overrideOut`.
The ja/ko/zh `overrideCJK` tables *are* built in, so the locales work. `table`,
`ignoreChar`/`ignoreName` and `undefChar`/`undefName` are not supported either. Biber
cannot pass any of these: `change()` refuses them, which is BB B8.

## Verification

Run `cargo test -p ucollate --release --test differential -- --ignored`. It needs the
O3 perl, which it finds itself or takes from `UCOLLATE_PERL`. The test compares full
key bytes against Unicode::Collate 1.31:

* every code point U+0000..U+10FFFF, surrogates included, as a one-character string,
  for three collators: biber's sort collator, biber's sortinithash collator and the
  module defaults. That is 3,342,336 keys;
* every one of the 87 tailorings (90 locale names, plus the default with all 939 DUCET
  contractions): each tailoring key on its own, with combining marks appended or
  inserted (discontiguous contractions, blocked and unblocked), and in context, under
  five option sets biber uses. That is 953,440 keys;
* an option matrix on 12 locales over targeted and random strings. It covers 4 levels ×
  4 variable modes × upper_before_lower, then each of backwards, kbh, ignore_level2,
  identical, hangul_terminator, FFFE/FFFF, every normalization form, user `entry`
  (jamo contractions in Hangul syllables, 5-character contractions) and NFD
  (Unicode 16) input with `prenormalized`. That is 1,322,544 keys;
* 8059 locale names (`getlocale`), and `option_value` for every locale.

All of these match exactly. Plain `cargo test` needs no Perl: it checks the same suites
against the committed md5 digests (`tests/data/`). It also checks every
(sortinit, sortinithash) pair found in the biber-generated corpus goldens (74 pairs,
all reproduced).

## Performance

Measured on an Apple M-series machine with `cargo run --release --example bench`:

* ~120 ns per key for a short NFD name. A typical 59-character biber sort string takes
  ~0.7–0.8 µs (en-US) or ~0.95 µs (sv). That is roughly 13 ns per character.
* Collator construction: about 50 µs for `default`, about 0.2–0.3 ms the first time a
  tailoring is used (then cached, ~1 µs), and 1–2.5 ms the first time ja/ko/zh* is used
  (the CJK table is parsed once).
* Memory: the DUCET is about 575 KB of static read-only tables (39,115 collation
  elements, a two-stage 17 KB + 155 KB index, and 939 contractions), with no start-up
  cost. Each loaded CJK table takes about 240 KB. A tailoring takes a few KB, or about
  100 KB for ja/zh.

Data provenance and licences: [DATA.md](DATA.md).

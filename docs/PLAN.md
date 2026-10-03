# bibe-rs: plan for a fast, behaviour-compatible Rust port of biber

Status: v1, decisions recorded (2026-10-02; see §10). It is based on reading biber 2.22, five subsystem deep-dives (`docs/research/01…05`) and measurements of the TeX Live 2026 biber 2.22 binary. Upstream bugs found along the way are in `docs/biber-bugs.md`.

---

## 1. Goals, non-goals, success criteria

**Goal.** A drop-in replacement for `biber`, shipped as the binary **`bibers`**, (the biblatex backend) that:

1. produces the **same `.bbl` byte for byte** as biber 2.22 for the same `.bcf`/data/argv, except in the deliberate, documented deviations of §6.4;
2. is **≥50× faster** end to end on typical documents, and asymptotically better on the pathological shapes where biber is super-linear;
3. accepts biber's CLI and `biber.conf`, and writes a `.blg`/stdout that latexmk, arara and editors keep parsing correctly.

**Non-goals (initially):** new features; a library API stable for third parties; GUI/LSP integration; bit-exact `.blg` timestamps or Perl source-line tags.

**Measured baseline** (biber 2.22, Apple M-series, `scratch/perf`, `docs/research/05` §6):

| Workload | biber 2.22 | Target |
|---|---|---|
| `biber --version` (PAR unpack + Perl startup) | 0.55–0.7 s | < 5 ms |
| biblatex-examples (92–102 entries, authoryear) | 1.3–1.5 s | < 20 ms |
| 4,600 entries | 18.5–20.7 s (any style) | < 250 ms |
| 18,400 entries | 77 s, 742 MB RSS | < 1 s, < 200 MB |
| uniquelist with 4,000 identical-list entries (quadratic in biber) | 103 s | < 200 ms |
| 2,000 crossref children → parent → grandparent (cubic in biber) | 245 s | < 100 ms |

Scaling in biber is linear at about 3.5–4.3 ms per entry. In normal documents most of that is spent in per-entry field handling: `_create_entry` name/date parsing, sourcemap steps and LaTeX decoding, plus parsing every `.bib` twice. The big wins therefore come from straightforward efficient code (one parse, interned strings, no regex-per-field, no string-`eval`), not from clever algorithms. Parallelism across sections, lists and entries is a bonus.

---

## 2. What biber does (one-paragraph map)

`bin/biber` parses the CLI and finds/merges `biber.conf`. It opens the output (truncating it), then parses the `.bcf`: options with scoping, the data model, templates, sourcemaps, sections, datalists and citekeys. For each refsection it:
1. **fetches** data: parses `.bib` with btparse, LaTeX→Unicode decodes, applies sourcemaps, and runs field handlers into typed values (names, dates, ranges, lists, verbatim, …);
2. resolves **dependencies**: citekey aliases, crossref/xref/xdata (including granular XDATA), related-entry clones, static/dynamic/legacy sets, and mincrossrefs promotion;
3. applies **inheritance** (`\DeclareDataInheritance`) and validates against the data model;
4. computes per-entry statics (labelname/labeldate/labeltitle, hashes);
5. then, **per datalist** (refcontext): visible names, the iterative uniquename/uniquelist loop, labelalpha, filters, sort keys (Unicode::Collate), the stable multi-key sort, extra* numbering in sorted order, and singletitle/uniquework etc.;
6. finally the writer emits `\refsection`/`\datalist`/`\entry` blocks.

Details: `docs/research/02` §2, `03` §2.

---

## 3. Feature inventory and priorities

**P0** is needed for any real document. **P1** is needed for broad compatibility. **P2** covers less common features. **P3** is rare and deferred; deprecated features are dropped (§10).

| Area | Features | Prio | Difficulty / risk | Research |
|---|---|---|---|---|
| CLI & config | All `bin/biber` options incl. Getopt prefix abbreviation and aliases; `biber.conf` discovery (cwd, `$XDG`, `~/.biber.conf`, kpsewhich); option precedence (CLI > conf > bcf > defaults for biber options; bcf > defaults > conf for biblatex options); exit codes 0/2; output filename derivation | P0 | M / M | 02 §3.1, 3.3, 3.4 |
| `.blg` / stdout | `[ms] File:line> LEVEL - msg` format; deferred WARN emission; `WARNINGS:`/`ERRORS:` summary; ~15 message texts that latexmk greps | P0 | L / **H** (ecosystem) | 02 §3.1.4 |
| `.bcf` | All elements (bcf.rnc 3.11); optionscope registry; NFD of values; `order` attributes; merged repeated `<section>`; version check; latin1 fallback | P0 | M / M | 02 §3.2 |
| Data model | Entry types, fields, datatypes and formats, aliases, constraints (mandatory, conditional, data incl. ISBN/ISSN/ISMN/date/pattern/range), `--validate-datamodel`, user DM merge | P0 (DM), P1 (validation) | M / M | 02 §3.5 |
| Option scoping | GLOBAL / ENTRYTYPE / ENTRY / NAMELIST / NAME, `map_boolean` semantics, silent drop of undeclared options | P0 | L / **H** (silent semantics) | 02 §3.4, 03 §3.13 |
| BibTeX parsing | btparse-compatible lexer/parser: `%` comments, `#` concatenation, macros (global, accumulating across sections; month macros; `--clrmacros`), error recovery, last-duplicate-field-wins, case folding, exact diagnostics | P0 | H / **H** | 01 §2.2 |
| LaTeX recode | Decode (LaTeX → Unicode) from `recode_data.xml` sets (base/full/null); encode for `--output-safechars` / non-UTF-8 | P0 decode, P1 encode | M / **H** | 01 §2.6 |
| Names | btparse `split_name` (von = first lowercase run, `~`/`-` not separators, Unicode case detection), `and others`, extended name format, per-name options, initials (grapheme-based), `format_name` ties | P0 | H / **H** | 01 §2.7 |
| Dates | ISO 8601-2/EDTF levels used by biblatex: ranges, open ends, circa/uncertain/unknown (`X`), seasons/divisions 21–41, eras/BCE, times and zones, Julian/Gregorian switch, legacy year/month | P0 | M / M | 01 §2.9 |
| Other field handlers | ranges (roman numerals, abbreviated ends), xsv lists, literal/key/name lists, verbatim/URI (URI::Escape rules), ISBN normalisation, annotations (`+an` fields) | P0/P1 | M / M | 01 §2.5, 2.10 |
| Sourcemaps | All step types (map_field_source/target, match/matchi/notmatch/notmatchi, replace, append/appendstrict, entry_new/clone/null, final, overwrite, foreach loops, `$MAPLOOP`/`$MAPUNIQ`, …); driver/style/user levels; **Perl regex semantics**; Perl replace-string interpolation | P0 (driver defaults), P1 (user maps) | H / **H** | 01 §2.4 |
| Encodings | Input/output encodings by Perl name, bibencoding, BOM, strict decoding | P1 | L / L | 01 §2.1 |
| Dependencies | crossref, xref, xdata (incl. granular `xdata=key-field[-n]`), mincrossrefs/minxrefs, related + relatedtype/relatedoptions clones (md5 keys), ids aliases, skip/omit, nocite, `*` | P0 | H / **H** | 03 §3.1–3.4 |
| Sets | static `@set`/entryset, dynamic `\defbibentryset`, legacy crossref sets, `\inset` | P0 | M / M | 03 §3.3 |
| Inheritance | `\DeclareDataInheritance` rules, defaults, override/ignore/suppress, date special cases | P0 | H / **H** | 03 §3.5 |
| Labels | labelname/labeldate/labeltitle, labelalpha templates incl. `v`/`l`/`f` disambiguation, padding, compound, ranges, labelalphaname templates, extraalpha | P0 | H / **H** | 03 §3.9–3.10 |
| Uniqueness | iterative uniquename (all modes incl. minyear*, templates) / uniquelist (incl. minyear); visible names (max/min × cite/bib/sort/alpha, per-entry overrides) | P0 | H / **H** | 03 §3.6–3.7 |
| Hashes | namehash/fullhash/fullhashraw/bibnamehash/per-name hashes: `md5_hex(utf8(NFC(normalise_string_hash(key))))` | P0 | M / M (bit-exact) | 03 §3.8 |
| extra* | extradate/extraname/extratitle/extratitleyear/extraalpha in sorted order; singletitle/uniquetitle/uniquebaretitle/uniquework/uniqueprimaryauthor | P0 | M / M | 03 §3.11 |
| Sorting | sort templates, sorting-name-key templates, padding/substring/final/descending, citeorder/intciteorder, nosort/sortexclusion/inclusion, presort, sortcase/sortupper, per-set locale, number vs string columns, stable `multikeysorter` semantics | P0 | M / **H** | 04 §2 |
| Collation | Port of **Unicode::Collate 1.31**: DUCET 13.0.0, 87 locale tailorings, CJK tables, levels, variable weighting, upper_before_lower, backwards, `viewSortKey` (for sortinithash) | P0 (root + common locales), P1 (all) | H / **H** | 04 §4–5 |
| sortinit / sortinithash | first grapheme (sombok, **Unicode 8** rules) of the normalised sort string; md5 of level-1 `viewSortKey` | P0 | M / **H** | 04 §3.1 |
| Transliteration | `\DeclareSortTranslit` (Lingua::Translit tables used by biblatex: iast→devanagari, etc.) | P2 | M / M | 04 |
| Datalists | multiple lists per section, refcontexts, filters (type/nottype/keyword/category/…), labelprefix, sortinghead per list | P0/P1 | M / M | 02 §3.6, 03 §3.12 |
| `.bbl` writer | exact layout, field-group ordering, `\name` blocks, `\strng`, `\range`, `\verb`, `\annotation`, `\keyalias`, `\missing`, set handling, NFC, `\endinput` | P0 | M / M | 05 §2 |
| Tool mode | `--tool`, biber-tool.conf, `.bib` output formatting (align, indent, fieldcase, field order, resolve, xname, legacy dates) | P2 | M / L | 02 §3.1.5, 05 §3.1 |
| biblatexml | input and output | P2 | M / M | 01 §2.11, 05 §3.3 |
| ~~bblxml, dot outputs~~ | deprecated upstream: **dropped** (§10). Selecting them prints biber's own deprecation message and exits with an error | — | — | 05 §3.2, 3.4 |
| Remote datasources | http(s)/ftp `.bib` URLs | P2 | L / L | 01 §2.1 |
| RNG validation | `--validate-control`, `--validate-config`, bltxml schema generation/validation | P3 (M4) | M / L | 02 |

---

## 4. Risk register: where exact compatibility is hard

These drive the order of work. Each has an owner module, a dedicated oracle (§6) and early spike tests.

1. **Collation must match Unicode::Collate 1.31 byte for byte.** That means DUCET **13.0.0** (not current Unicode), its pre-resolved tailoring tables, `upper_before_lower` tertiary remapping, the implicit-weight ranges of UCA 13, and `viewSortKey` text for `sortinithash`. ICU4X and `feruca` use CLDR root and current Unicode, so neither is usable. **Decision: write our own UCA engine (`ucollate` crate) that loads U::C 1.31's data.** Est. 1.5–2.5 kLOC plus a data extractor. Oracle: macOS system Perl's Unicode::Collate 1.29 has identical tables and is available today.
2. **Three Unicode versions coexist in biber.** These are Perl 5.42's **Unicode 16.0.0** (NFD/NFC, `\p{…}`, case mapping; verified from the PAR cache `unicore/version`), DUCET **13.0.0** (collation), and sombok's **Unicode 8.0** grapheme rules (`Unicode::GCString` 2013.10: sortinit, initials, label substrings, padding widths). We pin `unicode-normalization` to a Unicode-16 release, and implement legacy UAX#29 v8 segmentation ourselves (~150 LOC plus a generated table) rather than using `unicode-segmentation`.
3. **btparse semantics.** Name splitting, macro expansion, whitespace collapsing, error recovery and diagnostics. Mitigation: a spec-first reimplementation (btparse is LGPL, §9). Write `docs/spec/btparse.md` from the docs and black-box experiments, implement from it, and fuzz differentially against biber `--tool` output, `.bbl` names, and O3's direct calls into Text::BibTeX.
4. **LaTeX decoding** is a cascade of large Perl regexes that Rust regex engines can't express identically. Mitigation: a hand-written tokenizer driven by tables generated from `recode_data.xml`, plus a differential corpus generated by running biber over synthetic TeX-ish strings.
5. **User-supplied Perl regexes** in sourcemaps and in `noinit`/`nosort`/`nolabel`/`nonamestring`/`xsvsep`. Users rely on look-around, backrefs, `\p{…}`, `\x{…}`, `\N{U+…}` and inline flags. **Decision: PCRE2 (`pcre2` crate, UTF + UCP) for user regexes.** It is closest to Perl; `regex` lacks look-around/backrefs and `fancy-regex` lacks `\X`/`\N{U+}`. Translate `/xms`. Our built-in patterns use `regex` or hand-written code. Biber's own regexes run under Perl's Unicode 16 tables, while PCRE2's tables follow its own version, so the version gap for `\p{…}` is a known small risk; test it.
6. **Replace strings are Perl `qq` strings evaluated in Safe** (`$1`, `\u\L…\E`, even `@{[ code ]}`). We implement the interpolation subset (captures, case-modification escapes, standard escapes) exactly. For embedded code we emit a warning and emulate biber's error fallback, which inserts the stringified regex `(?^u:…)`. This is a deliberate, documented gap.
7. **Perl-isms with observable effects.** These include the truthiness of `"0"` (BB-055…059), `||` vs `//`, `lc`/`uc`/`fc` full case mapping, Perl string sort order, `looks_like_number`, `\s` semantics, and string-eval quirks (BB-017). Mitigation: a `perl_compat` module with named helpers (`perl_truthy(&str)`, `perl_fc`, …), so every Perl-ism is explicit and grep-able at call sites.
8. **Nondeterminism in biber itself** (BB-060…065). The oracle has to normalise these, and we pick one canonical deterministic order (§6.4).
9. **The uniqueness fixpoint and order-dependent state.** This covers last-writer-wins on shared name lists (BB-026), per-name option leakage (BB-019), and citekey ordering rules. We keep biber's iteration structure exactly and only change the data structures (prefix maps instead of rescans).
10. **Ecosystem coupling.** latexmk parses `.blg` messages, and biblatex checks the `.bcf` version. We keep message texts verbatim and test them with a latexmk smoke test.

---

## 5. Rust architecture

### 5.1 Workspace layout

```
bibe-rs/
├── Cargo.toml                  # workspace
├── crates/
│   ├── bibers/                 # the `bibers` binary (biber-compatible CLI): CLI parsing, conf discovery, logging, exit codes
│   ├── bibers-core/            # the pipeline (no I/O policy): see 5.2
│   ├── btparse/                # btparse-compatible (spec-first, §9) .bib lexer/parser, macro table, split_list,
│   │                           #   split_name, format_name, diagnostics (independently fuzzable)
│   ├── latex-recode/           # LaTeX↔Unicode; tables generated from recode_data.xml at build time
│   ├── ucollate/               # port of Unicode::Collate 1.31 (+ Locale, CJK); data generated by xtask
│   └── perl-compat/            # Perl-isms in one place: truthiness, fc/lc/uc, looks_like_number,
│                               #   PCRE2 wrapper for user regexes + qq-replacement interpreter,
│                               #   Unicode-8 grapheme segmentation, Unicode::UCD num, URI::Escape rules,
│                               #   ISBN/ISSN/ISMN (Business::* incl. RangeMessage.xml), Text::Roman
├── xtask/                      # dev tooling: data extraction (DUCET, tailorings, recode tables),
│                               #   golden generation, corpus building, delta-debug minimiser, bench
├── tests/
│   ├── corpus/                 # cases: {case.bcf, *.bib, argv, expected.bbl, expected.blg.msgs, exit}
│   └── ported/                 # assertions converted from biber's t/*.t
├── vendor/biber/               # upstream, pinned (git submodule)
└── docs/                       # PLAN.md, biber-bugs.md, research/, quirks.md, compat.md
```

The parsers, recoder, collator and Perl-compat layer are separate crates for three reasons. Each has its *own* oracle and fuzz target. Compile times stay low. And they are reusable on their own: a faithful Unicode::Collate port and a btparse port are useful outside biber.

### 5.2 `bibers-core` modules and core types

```
bibers-core/src/
  config/      options.rs (typed BiberOptions + provenance), blx_options.rs (OptionRegistry from
               <optionscope>, OptValue enum, scoped lookup incl. biber's undef-secnum quirk),
               conf_file.rs (biber.conf), precedence.rs (pure resolve())
  bcf/         typed model (quick-xml), templates (sorting, sortingnamekey, uniquename, labelalpha,
               labelalphaname, namehash, inheritance, translit, sourcemap, datafieldsets)
  datamodel/   DataModel (interned FieldId/TypeId, bitsets of legal fields, constraints, validate)
  input/       bibtex.rs (btparse AST → RawEntry), biblatexml.rs, sourcemap/ (engine + steps),
               handlers/ (names, dates, ranges, lists, verbatim/uri, annotations, isbn…)
  model/       Entry, Value, NameList/Name arenas, Section, DataList, Sets
  resolve/     aliases, dependencies (crossref/xref/xdata/related/sets), inheritance
  process/     labels (labelname/date/title), hashes, visibility, uniqueness, labelalpha,
               extras, workuniqueness, contextdata, filters
  sort/        sort-object construction, column typing, collator cache, final sort
  output/      bbl.rs (primary), bibtex.rs (tool), biblatexml.rs
  log.rs       Logger: .blg + stdout formats, deferred warnings, error count, message catalogue
  quirks.rs    Quirk registry (see 6.4)
```

Key representation choices:
- **Interning everywhere.** Field names, entry types and option names become small integer ids fixed by the data model and option registry. Citekeys become `KeyId`. Biber does string-keyed hash lookups on every access; this alone is a large constant-factor win.
- **Arenas + ids** for entries, name lists and names. Biber *shares* `Names` objects between parent and child on inheritance/XDATA, and per-list state is keyed by name-list id (BB-026). We reproduce this sharing explicitly by copying the `NameListId`, not by deep-cloning. Then last-writer-wins is a deliberate, testable behaviour instead of an aliasing accident.
- **Typed values:** `Value::{Literal, Integer, Range(Vec<(String, Option<String>)>), Names(NameListId), List(Vec<String>), Date(DateParts), Verbatim, Uri, Key, …}` with the data-model datatype attached.
- **Strings:** internal form is NFD (as in biber) and output is NFC. Hot paths use `CompactString`/`SmolStr` and `Cow`.
- **Citekey order** in an `IndexSet<KeyId>` (O(1) delete, insertion-ordered), replacing biber's O(N) array rebuilds.
- **Options:** one immutable `RunConfig` after setup; per-section `SectionState`; per-list `ListState`. No globals, so sections and lists can run in parallel.
- **The `.bbl` writer** renders each (entry, list) pair directly from structured data. Biber instead builds a string with `<BDS>` placeholders and then runs ~40 regex substitutions per entry per list. The placeholder side effects that leak into output (BB-015) are reproduced as explicit quirks.

### 5.3 Pipeline and parallelism

1. **Setup (serial):** CLI → conf → `.bcf` → `RunConfig`, DataModel, templates and compiled sourcemaps.
2. **Parse data sources once per file** (parallel across files) into an immutable `ParsedFile` cache keyed by (path, datatype, encoding). The macro table must keep biber's semantics: definitions accumulate in read order across sections. If files define conflicting macros, fall back to serial order.
3. **Per section** (sections are semantically independent; run in parallel with rayon, then merge output and warnings in section order): fetch (cache → sourcemap → handlers, parallel per entry) → resolve dependencies and inheritance (serial graph work, linear-time with visited sets) → static per-entry processing (parallel).
4. **Per datalist:** visibility/uniqueness counting is map-reduce over entries. Sort objects and collation keys are computed in parallel with a memoised key cache per collator. The sort is a single stable `sort_by` over rank tuples. extra* and labelalpha `l`/`v` numbering are sequential passes.
5. **Output** (serial write, parallel per-entry rendering).

Parallelism is an optimisation, not a requirement. Phase 1 of the port is single-threaded, and every parallel stage must give identical bytes to the serial path (a test asserts this).

---

## 6. Correctness strategy

Exact output equality with a real biber is the bar, so the project is built around **differential testing against oracles**. Unit tests alone are not enough.

### 6.1 Oracles

| Oracle | What it answers | Availability |
|---|---|---|
| **O1: biber 2.22 binary** (TeX Live 2026, sha256 recorded) | End-to-end `.bbl`/`.blg`/exit code; `--tool` output (parsed+mapped `.bib`) as a parser-level probe; `--debug` "Sorting object for key …" lines to diff pre-collation sort data; `--sortdebug` | Installed. HEAD `lib/` == v2.22, and `.blg` file:line tags point straight into `vendor/biber` |
| **O2: system Perl Unicode::Collate 1.29** (`/usr/bin/perl`) | Sort-key bytes and `viewSortKey` for any string/locale/option combination; same DUCET 13 and tailorings as biber's 1.31 | Installed |
| **O3: source-built biber** (perl 5.42.0 + CPAN deps pinned to the PAR cache versions, under `vendor/perl`; `tools/oracle/`) | Function-level differential fuzzing (`parsename`, `latex_decode`, `normalise_string_*`, date parsing, `rangelen`, sourcemap step evaluation); running biber's own `t/*.t`; instrumenting intermediate state; trying candidate upstream fixes | **Built** (`tools/oracle/biber-src`). All 1,275 of biber's own tests pass. Byte-identical `.bbl` to O1 on 60/60 parity cases. Details: `tools/oracle/README.md` |

### 6.2 Test layers

1. **Crate-level differential tests** against O2/O3:
   - `ucollate`: exhaustive single-code-point keys (all of 0..0x10FFFF) for root at the default options; sampled option and locale matrices (87 tailorings × levels × variable × upper_before_lower × backwards); every contraction (+ trailing combining mark, for discontiguous matching); Hangul; random multi-script strings. Compare **key bytes**, not just order, plus md5(viewSortKey) for sortinithash.
   - `btparse`: grammar fuzzing (braces, quotes, `#`, `%`, junk, recovery), `split_name` on generated names, and diagnostics text.
   - `latex-recode`: generated TeX-ish strings, decode and encode, compared with biber.
   - `perl-compat`: PCRE2 vs Perl on a regex/replacement corpus; Unicode-8 grapheme breaks vs `Unicode::GCString` (O3); `fc`/`lc` vs Perl.
2. **Ported biber tests (`tests/ported`).** biber has 1,275 planned tests (~1,100 assertions). About 750–800 can be converted mechanically into black-box checks on `.bbl` text and `--sortdebug` output, and about 130 into table-driven unit tests (`xtask port-tests` extracts the expected strings from the `.t` files). The rest poke Perl internals and get rewritten by hand or dropped.
3. **Golden end-to-end corpus (`tests/corpus`).** Each case is `.bcf` + data + argv, with expected `.bbl`, normalised `.blg` messages and exit code, all generated by O1 and committed so CI needs no TeX. Sources:
   - the 53 `t/tdata` configurations;
   - biblatex's 52 example documents;
   - a **style × option matrix**: all standard styles plus the contributed styles in TeX Live (apa, chicago, ieee, mla, gb7714, ext, …; apa ships its own sourcemaps and datamodel, so it is a good stress test), crossed with uniquename/uniquelist/maxnames/labeldateparts/sorting/sortlocale/sortcase/sortupper/useprefix;
   - the 488 `.bib` files in TeX Live;
   - the Unicode zips in `testfiles/`;
   - the synthetic scaled files.

   A `.bcf` needs one `pdflatex -draftmode` pass. After that the `.bcf` can be edited to swap data sources and options without TeX.
4. **Differential fuzzing.** A grammar-based generator produces `.bib` entries (names with prefixes, braces, `and others`, extended names, per-name options; ISO 8601-2 dates; ranges; verbatim/URI; crossref/xdata/xref/set/related graphs; `@string`s) and runs O1 against bibers on a small set of `.bcf` templates. Failures are **auto-minimised** by delta-debugging over entries, then fields, with O1 in the loop. The minimised case is committed to the corpus.
5. **Properties:** no panics on arbitrary bytes (cargo-fuzz on btparse, recode, dates, the `.bcf` parser); serial output == parallel output; output is identical across runs; idempotence of tool-mode round-trips.
6. **Ecosystem smoke tests:** latexmk and arara on sample documents with bibers as `$biber`, checking the rerun detection and warning/error reporting.

### 6.3 Normalisation before comparing with O1

- **`.bbl`:** sort the option lists inside `\name{…}{n}{<opts>}` and `{{<opts>}{%` (BB-060). For `sorting=none` runs with mincrossrefs promotion or `\nocite{*}` dynamic sets, compare the affected entry runs as multisets (BB-061/062). Nothing else varies (verified: 2× runs over 5 styles and 53 fixtures were byte-identical).
- **`.blg`/stdout:** strip `[ms]`, the date line, `File.pm:NNN>` tags and temp paths. Compare WARN/ERROR as a multiset (order is nondeterministic, BB-065), and INFO as advisory.
- **Other formats:** compare biblatexml as XML.

### 6.4 Compatibility policy and the quirk registry

Every known biber bug that affects output gets a `Quirk` variant in `bibers-core::quirks`, cross-referenced to its `BB-NNN` id. The code path that reproduces it is written explicitly, e.g. `if quirks.on(Quirk::RangelenZeroDigit /* BB-016 */) { … }`. Defaults (decided, §10):

- **Reproduced by default:** silent output-affecting bugs on valid input (BB-015, 016, 017, 018, 019, 022, 023, 026, 028, 042, 055, …). Users switching to bibers then get identical documents, and differential testing stays clean.
- **Always fixed, never reproduced:**
  - crashes (BB-001…011): a sensible error or the obviously intended output;
  - data loss (BB-012/013/014): for 014 we keep biber's truncate-on-error behaviour only if latexmk depends on it, and otherwise write via temp file + rename;
  - nondeterminism (BB-060…065): one canonical order that is *one of biber's possible outputs*, e.g. per-name options in data-model declaration order, promoted crossrefs in first-reference order.
- **`--fix-quirks[=list]`** turns reproduced quirks off individually or all together. When upstream fixes a bug in 2.23+, the default flips with a `--compat=2.22` escape hatch.

Every deliberate deviation from O1 is listed in `docs/compat.md`, and the test harness has a per-case allow-list keyed by quirk/bug id. An unexplained diff is always a test failure.

### 6.5 Reporting upstream bugs

Any bug found while porting (including in the harness, fuzzing and minimisation) is added to `docs/biber-bugs.md` with a `BB-NNN` id, status V/C/R and a minimal reproducer. The fuzzer's minimiser yields reproducers almost for free. Before filing upstream we re-verify against the latest biber release and HEAD, ideally with O3 so a patch can be proposed alongside.

---

## 7. Performance plan

- Single parse per `.bib` with a zero-copy lexer; macro lookup via a hash map; LaTeX decoding via an Aho–Corasick/trie tokenizer. No intermediate re-serialisation.
- Interned ids and bitsets instead of string hashes, and no string-`eval` anywhere.
- Collators built once per (locale, options), with collation keys memoised per (collator, string). Sort via rank tuples.
- Linear-time replacements for the super-linear spots in biber (`docs/research/03` §5):
  - prefix maps for `namelist_differs_*` (BB-109);
  - an edge set + visited-set DFS for crossref inheritance (BB-110);
  - key→index maps for `first_index` scans;
  - `IndexSet` citekeys.
- Rayon parallelism across files, sections, lists and entries where state allows (§5.3).
- Startup: a native binary, embedded data tables (DUCET as a compact `phf`/sorted array generated at build time, tailorings loaded lazily per locale), and no kpsewhich spawn unless a file isn't found locally. Cache kpsewhich results.
- `xtask bench` tracks both implementations on the scaled corpus and the pathological shapes. CI fails on >10% regressions.

---

## 8. Milestones

Each milestone ends with a corpus subset that passes byte-for-byte. Work inside a milestone is split into small PRs, each gated by the golden tests it turns green.

**M0: Infrastructure**: *done (2026-10-02).*
- Harness: `crates/testkit`, `cargo xtask`. Corpus: 475 cases in 6 suites (`tdata` 60, `biblatex-examples` 51, `styles` 129, `bugs` 66, `scale` 1, `ported` 168). All 475 pass on O1. O3 reproduces all but the 2 CJK cases (BB-011).
- biber's 1,275 tests: 948 covered by corpus cases (fidelity-checked), 322 extracted as unit vectors in `tests/ported/`, and 5 not portable.
- Open harness items: hand-maintained expectations for DC cases (always-fixed bugs, nondeterminism BB-112/113); comparing normalised stderr for crash cases; suite-level shared inputs to shrink `ported/` (33 MB on disk, under 1 MB packed).

Original M0 scope:
- Cargo workspace, CI (fmt, clippy, test, fuzz smoke), `vendor/biber` as a pinned submodule.
- O1 harness: run biber on a case and normalise outputs. `xtask golden` builds the corpus (`.bcf` generation via pdflatex, then O1 runs, twice for determinism).
- O2 harness for collation keys. O3 environment, if approved.
- `xtask port-tests`: extract expected strings from `t/*.t`.
- Initial corpus: t/tdata, biblatex examples, a 5-style matrix over biblatex-examples.bib.

**M1: Foundations**: *done (2026-10-03).* Each crate was differentially tested against the Perl oracle, and compact vectors are committed so CI needs no Perl.
- `ucollate`: ~5.6M sort keys identical to Unicode::Collate 1.31 (every code point × biber's collators, 87 tailorings, option matrix); ~120 ns per short key.
- `btparse` (clean-room; spec in `docs/spec/btparse.md`): 100k random differential cases with no unexplained mismatches; 1,181/1,182 TeX Live `.bib` files identical (the remaining one hits a NUL-byte crash in biber); 133 MB/s.
- `latex-recode`: ~2.2M random cases, 0 mismatches; ~300× faster than biber's decoding.
- `perl-compat`: case mapping, normalisation, 44 regex classes and both grapheme segmenters exhaustive over all code points; 17,933 regex cases via PCRE2 (gaps documented as DC-036); ISBN/ISSN/ISMN, URI, roman numerals.
- `bibers-core` config front end and the `bibers` CLI: post-parse state identical to biber on all 463 runnable corpus `.bcf` files; 126 CLI argv cases identical to Getopt::Long. The binary reaches the data model in about 8 ms.

Original M1 scope:
- `ucollate` (DUCET root first, then tailorings, then CJK), with exhaustive key diff vs O2.
- `btparse` (parser, macros, split_name/format_name, diagnostics).
- `latex-recode` (decode first).
- `perl-compat` (truthiness/case helpers, Unicode-8 graphemes, PCRE2 wrapper + replacement interpreter, URI escape, ISBN).
- `bibers-core::{config, bcf, datamodel}` and `log`.

**M2: Vertical slice.** biblatex-examples.bib under numeric, authoryear, alphabetic and authortitle produces byte-identical `.bbl`. This needs:
- driver sourcemaps and the field handlers;
- names and dates;
- crossref/xdata/sets/related plus inheritance;
- label*, hashes and visible names;
- uniquename/uniquelist, labelalpha and extra*;
- sorting with root collation and sortinit/sortinithash;
- the `.bbl` writer;
- the CLI and `.blg`.

This is the largest milestone. Order inside it: parse → handlers → writer for `sorting=none` with no uniqueness → sorting → labels/hashes → uniqueness → extra*/labelalpha → dependency features.

**M3: Breadth.**
- All of t/tdata and the ported assertions; the full style × option matrix including contributed styles.
- Multiple sections, datalists and filters; user sourcemaps (PCRE2 and the replacement interpreter); all locales and tailorings; transliteration.
- Option scoping edge cases, `--validate-datamodel`, encodings.
- Fuzzing campaign running continuously with the minimiser.

**M4: Secondary features** (deferred by decision, §10). These are tracked here so they aren't forgotten:
- tool mode (`--tool`, biber-tool.conf) and bibtex output with all `--output-*` formatting options;
- non-tool `--output-format=bibtex`;
- biblatexml input and output, and bltxml schema generation;
- remote (http/https/ftp) datasources;
- RNG validation (`--validate-control`, `--validate-config`, `--validate-bltxml`);
- `--output-encoding` other than UTF-8 and `--output-safechars` (LaTeX *encoding*), unless the corpus shows they're common enough for M3.

Not planned at all: the deprecated `bblxml` and `dot` outputs (with `--dot-include`), and `bibjson` (unimplemented upstream).

**M5: Performance and release.**
- Parallel pipeline with the serial == parallel tests; benchmark targets of §1 met; memory profile.
- Packaging: static binaries for macOS, Linux and Windows, and drop-in instructions (latexmk `$biber`, TeXstudio/Overleaf-style setups).
- `docs/compat.md` completed; upstream bug reports filed.

---

## 9. Licensing

*Not legal advice. This is a careful reading of the licence text in `vendor/biber/LICENSE`.*

**Assumption.** We treat bibers as a **Modified Version** of biber. A translation that follows biber's code structure and logic closely, which a byte-compatible port has to do, is very likely a derivative work. Rewriting in another language doesn't change that. Behaviour, file formats and algorithms aren't copyrightable as such, but code expression is.

**What the Artistic License 2.0 requires:**
- **§1:** creating and using Modified Versions privately is unrestricted. Conditions only apply once we *distribute*.
- **§4** (distributing a Modified Version as source): we must **clearly document how it differs** from biber, and do at least one of:
  - (a) make our modifications available to the biber copyright holders under the Artistic License 2.0, so they could take them upstream;
  - (b) give it a **different name** and make sure installing it doesn't stop anyone installing or running real biber;
  - (c) let recipients redistribute our source under the Artistic License 2.0, or a copyleft licence that keeps it free.
- **§5–6** (binaries): ship complete instructions for getting the corresponding source.
- **§12:** no right to use the copyright holders' trademarks or logos. We must not present bibers as biber or as endorsed by its authors.
- **§10:** the biber-derived parts stay under the Artistic License 2.0 whatever licence we put on the project, so we can't truly relicense them as MIT/Apache.

**Are we allowed?** Yes. The licence explicitly permits modified versions, compiled distributions and embedding (§8). Our plan already meets §4 three times over:
- the name `bibers` and a separate binary satisfy (b);
- `docs/compat.md` plus the quirk registry document the differences;
- releasing under the Artistic License 2.0 satisfies both (a) and (c).

**Decision: bibers is licensed under the Artistic License 2.0** (SPDX `Artistic-2.0`), keeping the copyright notices of François Charette and Philip Kime alongside ours. This is the only choice with no grey area: it matches upstream, lets the maintainers take our fixes, and is still permissive (commercial use and embedding allowed). MIT OR Apache-2.0 is the Rust norm, but because of §10 it would be misleading for the biber-derived crates.

**Third-party-derived crates keep their upstream terms,** each crate with its own `license` field and NOTICE:
- `ucollate` ports Unicode::Collate and its tables, which are under Perl 5 terms (Artistic 1.0 or GPL-1.0+). DUCET is under the Unicode License v3, which is permissive.
- **btparse is LGPL-2.0-or-later** (verified from the Text-BibTeX 0.91 dist; its PCCTS-generated parts are public domain). **Decision: spec-first reimplementation (§10 #8).** We do *not* translate btparse's C code (`names.c`, `format_name.c`, the lexer/parser). Instead we:
  1. write a behavioural specification (`docs/spec/btparse.md`) from btparse's documentation, BibTeX's documented rules, and black-box experiments against O1/O3;
  2. implement the `btparse` crate from that spec;
  3. verify it by differential fuzzing against O1/O3.

  Contributors to the `btparse` crate work from the spec and test results, not from the LGPL C sources. The spec may describe *behaviour* in any detail, but must not paraphrase code. The crate is Artistic-2.0 like the rest of bibers.
- `ucollate`'s tailoring data (`Locale/*.pl`) is itself derived from CLDR (Unicode licence, permissive).
- Lingua::Translit tables are under Perl terms.
- ISBN ranges: `data/isbn/RangeMessage.xml` is vendored verbatim from Business-ISBN-Data 20260218.001 (Artistic-2.0), byte-identical to biber 2.22's bundled copy. It is compiled into the binary at build time, so there is no network dependency, and `ISBN_RANGE_MESSAGE` overrides it at run time. See `data/isbn/README.md`.
- biber's `recode_data.xml` is Artistic-2.0, same as us.

---

## 10. Decisions log

| # | Question | Decision (2026-10-02) |
|---|---|---|
| 1 | Bug compatibility | **Keep biber's logic.** Reproduce output-affecting biber bugs by default, each behind a named `Quirk` switch (§6.4). Always fix crashes, data loss and nondeterminism. |
| 2 | Scope | P2 features are **deferred to M4** (listed in §8). **Deprecated features are dropped:** `bblxml` and `dot` outputs, and `--dot-include`. Legacy CLI aliases (`--bibencoding`, `--outfile`, `--tool-align`, …) are kept because they are cheap and scripts use them. |
| 3 | Oracle O3 | **Approved.** Source-built perl 5.42.0 + pinned CPAN deps under `vendor/perl` (gitignored), built by `tools/oracle/build-perl-oracle.sh`. |
| 4 | Version target | **biber 2.22** (submodule pinned at tag `v2.22`), then follow 2.23. |
| 5 | Name / licence | Binary and crate name **`bibers`**. Licence: **Artistic-2.0** (§9; top-level `LICENSE`). |
| 6 | Git | Repo initialised, biber added as a submodule at `vendor/biber` (v2.22), docs committed. |
| 7 | Behaviour differences | Every difference from biber is recorded in **`docs/compat.md`** (DC-NNN ids, referenced from code and from the test-harness allow-list). |
| 8 | btparse licence (LGPL) | **Spec-first reimplementation** (§9): no translation of btparse C code. |
| 9 | ISBN range data | **Vendor** the exact `RangeMessage.xml` biber 2.22 bundles (`data/isbn/`), compiled in at build time. No network needed; `ISBN_RANGE_MESSAGE` override supported. |

# Behavioural differences between bibers and biber

This is the authoritative list of every way **bibers** behaves differently from **biber 2.22** (the reference, `vendor/biber` @ `v2.22`). It also serves as the "clearly document how it differs from the Standard Version" record required by §4 of the Artistic License 2.0.

**Rules**
1. Any observable difference from biber 2.22 must have an entry here *before* it is merged: `.bbl` content, other output files, `.blg`/stdout/stderr messages, exit codes, CLI behaviour, or file side effects. Performance is the exception.
2. Each entry gets a stable id `DC-NNN`. Code that implements a difference cites the id in a comment. The differential-test harness allow-list is keyed by these ids, so an unexplained diff against biber is always a test failure.
3. If the difference comes from a biber bug, link its `BB-NNN` entry in [biber-bugs.md](biber-bugs.md).
4. *Status* is one of: **planned** (decided, not implemented), **implemented**, or **withdrawn** (kept for history).

**Default policy** (PLAN.md §6.4, §10): bibers reproduces biber's logic, *including* bugs that silently affect output. Each reproduced bug sits behind a named `Quirk` switch (§2), so the default output matches biber. Crashes, data loss and nondeterminism are always fixed (§1). Deprecated features are dropped (§3).

---

## 1. Always-on differences (biber bugs we never reproduce)

### 1a. Crashes on valid input

Biber dies with a Perl error and usually leaves an empty `.bbl`. bibers produces the intended output, or a proper `ERROR` with exit code 2.

| Id | biber behaviour | bibers behaviour | Bug | Status |
|---|---|---|---|---|
| DC-001 | `uniquelist` crashes when a labelname family name is literally `final` | Processes normally (name keys can't collide with internal bookkeeping) | BB-001 | planned |
| DC-002 | Several macros in one `@string{a="x", b="y"}` → fatal syntax error | All macros defined, as in BibTeX | BB-002 | planned |
| DC-003 | `\char"7B`/`\char123` in a field → fatal syntax error, and the next entry is swallowed | Decodes to a literal brace character without breaking parsing. *Exact output TBD when implemented.* | BB-003 | planned |
| DC-004 | A regex metacharacter in `presort` (e.g. `a(b`) → fatal | `presort` is treated as a literal string | BB-004 | planned |
| DC-005 | biber.conf `<noinit>`/`<nolabel>` in the documented form → crash; `<sortinclusion>` alone → crash | Parsed as documented | BB-005, BB-006 | planned |
| DC-006 | A section with citations but no data sources (while other sections have some) → crash | Warns that keys were not found, as for any missing key | BB-007 | planned |
| DC-007 | `--output-fieldcase=Upper` (non-lowercase value) → crash | Value accepted case-insensitively | BB-008 | planned |
| DC-008 | `-c ignoreChar=…` and other construction-only collation options → crash | Applied when the collator is constructed | BB-010 | planned |
| DC-009 | Packaged binary dies for ja/ko/zh sort locales | Supported | BB-011 | planned |
| DC-024 | `-c level=5` (out-of-range collation level) crashes with out-of-memory, leaving an empty `.bbl` | ERROR naming the invalid option value, exit 2 | BB-120 | implemented (ucollate returns an error) |

### 1b. Data loss

| Id | biber behaviour | bibers behaviour | Bug | Status |
|---|---|---|---|---|
| DC-010 | Tool mode with biblatexml input lacking a known extension overwrites the input file with the generated `.rng` schema | Schema file name always differs from the input | BB-012 | planned (M4) |
| DC-011 | Tool mode on an input file with no extension overwrites the input with the output | Output named `<input>_bibertool.bib` | BB-013 | planned (M4) |
| DC-012 | The `.bbl` is truncated before processing, so any error leaves a 0-byte `.bbl` | *Undecided:* keep biber's behaviour if latexmk/biblatex rely on it (an empty `.bbl` forces a rerun); otherwise write to a temp file and rename on success. Decide in M2 with the latexmk smoke test. | BB-014 | open |

### 1c. Nondeterministic output

Biber's output varies between identical runs because of Perl hash ordering. bibers always emits **one fixed order that is among biber's possible outputs**, so the differential harness only needs order-insensitive comparison at these spots.

| Id | Where | bibers order | Bug | Status |
|---|---|---|---|---|
| DC-013 | Per-name / per-namelist option lists in `\name{…}{n}{<opts>}` and `{{<opts>}{%` | Option declaration order from the `.bcf` `<optionscope>`. *Confirm against biblatex parsing during M2.* | BB-060 | planned |
| DC-014 | Order in which mincrossrefs/minxrefs-promoted entries are added (visible with `sorting=none` or sort ties) | First-reference order | BB-061 | planned |
| DC-015 | Order of dynamic sets under `\nocite{*}` | `.bcf` declaration order | BB-062 | planned |
| DC-016 | Order of deferred WARN messages in `.blg`/stdout | Emission order (deterministic) | BB-065 | planned |
| DC-017 | Which cycle is named in the fatal "Circular XDATA inheritance" error, and which warnings come before it | Cycles are detected walking entries in citekey/`.bib` order, so the first cycle found is always reported | BB-112 | planned |
| DC-018 | `$MAPUNIQ` values in sourcemaps (time-based in biber, so the `.bbl` differs on every run) | Deterministic values: a per-run counter rendered in `suniqid`'s alphabet, so keys are unique within a run and identical across runs. *Exact format TBD when implemented.* | BB-113 | planned |

### 1d. Unavoidable representation differences

| Id | biber behaviour | bibers behaviour | Bug | Status |
|---|---|---|---|---|
| DC-019 | `\char` with a surrogate (U+D800–DFFF) or a value above U+10FFFF yields that raw code point (invalid as UTF-8 text) | U+FFFD. Rust strings can't hold these, and biber's output in these cases isn't valid UTF-8 anyway. | — | implemented (latex-recode) |

### 1e. CLI and file-handling fixes

| Id | biber behaviour | bibers behaviour | Bug | Status |
|---|---|---|---|---|
| DC-025 | Non-tool `--output-format=bibtex` names its output by stripping from the first `.` in the *whole path* (`./t1` → `_biber.bib`; `sub.dir/t1` → `sub_biber.bib`) | `<bcf-stem>_biber.bib` next to the `.bcf` (or in `--output-directory`) | BB-068 | implemented (output itself deferred, DC-030) |
| DC-026 | `-O -` leaves an empty file named `-` (and with `--output-directory` writes `<dir>/-` instead of stdout) | Writes to stdout; no stray file | BB-069 | implemented |
| DC-027 | With `--nodieonerror`, a missing control file cascades into a libxml crash | Stops after the "Cannot find control file" error, exit 2 | BB-071 | implemented |
| DC-028 | `--fix-quirks` doesn't exist | Accepted only when spelt in full, so it never makes biber's abbreviations of other options ambiguous | — | implemented |

## 2. Reproduced biber bugs (Quirk switches)

These are **not** differences by default: bibers reproduces biber's output. Each can be switched off with `--fix-quirks=<name>[,<name>…]` (or `--fix-quirks=all`), and the fixed behaviour is then a documented difference. Switch names are provisional until implemented.

| Quirk (switch) | Reproduced biber behaviour | Behaviour with the quirk fixed | Bug | Status |
|---|---|---|---|---|
| `bbl-comma-cleanup` | `,}}` inside field data is rewritten to `}}` (`{Foo {bar,}}` → `{Foo {bar}}`) | Field data preserved | BB-015 | planned |
| `rangelen-zero-digit` | `pages={105-10}` → `\range{pages}{11}` | `\range{pages}{6}` | BB-016 | planned |
| `sortkey-eval` | Backslashes/braces in sort values can produce an empty key (entry sorts first) | Sort value used verbatim | BB-017 | planned |
| `filter-readds-sets` | A filtered list re-adds every set and set member if any set passes | Only members of passing sets are added | BB-018 | planned |
| `per-name-option-leak` | Per-name/per-namelist overrides leak to following names | Overrides apply only to their own name/namelist | BB-019 | planned |
| `uniquename-false-skips-entry` | A per-name `uniquename=false` drops the whole entry from uniquename | Only that name is excluded | BB-020 | planned |
| `entry-scoped-options-ignored` | Per-entry `labelalpha`, `labeldateparts`, `singletitle`, … are ignored | Honoured | BB-021 | implemented (lookup; call sites in M2) |
| `crossref-date-meta` | Parent's circa/uncertain/era flags are inherited even when the child has its own date; date parts of different date fields are mixed | Date metadata inherited only together with the date | BB-022, BB-023 | planned |
| `xdata-name-index` | Granular-XDATA names get 0-based indices / last name skipped | Consistent 1-based indices | BB-024 | planned |
| `xdata-rerefs` | Re-resolving XDATA refs duplicates names | Resolved once | BB-025 | planned |
| `shared-namelists` | Inherited name lists share per-list state with the parent (last processed wins) | Per-entry state | BB-026 | planned |
| `workuniq-skipbib-token` | Only the literal `skipbib` token suppresses work uniqueness | `skipbib=true`, `dataonly` and set members respected | BB-027 | planned |
| `datefield-helper-mutation` | Custom date fields whose stem ends in `date` lose flags after the first entry | Flags on every entry | BB-028 | planned |
| `sort-column-type` | `labelyear`/`labelmonth`/`labelday` columns sort as strings; mixed columns typed by first item | Typed per item | BB-042 | planned |
| `zero-is-empty` | Values equal to `0` treated as missing (year 0, `volume={0}`, …) | `0` is a value | BB-055…059 | planned |
| `blx-defaults-override-conf` | Hard-coded biblatex defaults overwrite biber.conf values (maxbibnames, useprefix, …) | biber.conf values win | BB-053 | implemented |
| `conf-labelalphanametemplate-last` | Only the last biber.conf `labelalphanametemplate` survives | All are kept | BB-083 | implemented |
| `conf-sourcemaps-dropped` | biber.conf sourcemaps are dropped when the `.bcf` has no driver/style maps | They are merged | BB-052 | implemented |
| `uniquename-base-check-global` | The "uniquenametemplate needs a base part" check is global across templates | Checked per template | BB-082 | implemented |
| `duplicate-datalists` | Repeated identical `<datalist>`s are emitted twice | De-duplicated | BB-054 | implemented |
| *(more to be added as porting proceeds: every reproduced bug gets a row)* | | | | |

Some silent biber behaviours are judged to be design decisions rather than bugs and are reproduced with no switch. Examples: `\textdollar` decoding to a raw `$` (BB-029, pinned by biber's own tests), and the btparse name-splitting rules. They are listed in biber-bugs.md but not here.

## 3. Dropped features

| Id | Feature | bibers behaviour | Status |
|---|---|---|---|
| DC-020 | `--output-format=bblxml` (deprecated upstream) | Error: unsupported output format, exit 2 | planned |
| DC-021 | `--output-format=dot`, `--dot-include` (deprecated upstream) | Error: unsupported output format, exit 2 | planned |
| DC-022 | `--output-format=bibjson` (accepted but unimplemented upstream) | Error: unsupported output format, exit 2 | planned |
| DC-023 | Perl code in sourcemap replacement strings (`@{[ … ]}`, `${\ … }`). biber evaluates these in a Safe compartment. | Not executed. bibers warns, and inserts the stringified regex `(?^u:…)` exactly as biber does when evaluation fails. Plain interpolation (`$1`, `\u`, `\L…\E`, escapes) is fully supported. | planned |

## 4. Temporarily unsupported (deferred to milestone M4)

Until implemented, bibers exits with a clear "not yet supported" ERROR (exit 2) instead of silently doing something different. Each row is deleted when the feature lands.

| Id | Feature | Status |
|---|---|---|
| DC-030 | Tool mode (`--tool`), bibtex output (`--output-format=bibtex`) and its `--output-*` formatting options | deferred |
| DC-031 | biblatexml input and output; bltxml schema generation | deferred |
| DC-032 | Remote (http/https/ftp) data sources | deferred |
| DC-033 | RNG validation (`--validate-control`, `--validate-config`, `--validate-bltxml`) | deferred |
| DC-034 | Non-UTF-8 `--output-encoding` and `--output-safechars` | deferred |

## 5. Cosmetic differences in logs and messages

The `.blg` keeps biber's line format (`[ms] location> LEVEL - message`) and message texts, because latexmk, arara and editors parse them. These parts necessarily differ:

| Id | biber | bibers | Status |
|---|---|---|---|
| DC-040 | `This is Biber 2.22` banner; `--version` prints `biber version: 2.22` | `This is bibers <version> (biber 2.22 compatible)`; `--version` prints `bibers version: <version> (biber 2.22 compatible)`. *Check latexmk/arara don't parse these.* | implemented |
| DC-041 | Location tag `Biber.pm:4487>` (Perl file:line) | A Rust module/location tag. The format is kept and the contents are not meaningful across implementations. | planned |
| DC-042 | btparse diagnostics name a temp file (`<md5>_<pid>.utf8`) and pass-2 line numbers | Name the user's `.bib` file and its real line numbers | BB-076 | planned |
| DC-043 | `INFO` lines specific to biber internals (PAR cache, Perl modules, …) | Omitted or replaced | planned |
| DC-044 | `--debug`/`--trace` output (Perl data dumps) | Different content. Also, `--debug` does **not** stop btparse errors being fatal (BB-075). | planned |

## 6. Extensions (things bibers adds)

| Id | Extension | Status |
|---|---|---|
| DC-050 | `--fix-quirks` option (§2) | planned |
| DC-051 | `--compat=2.22` (pin quirk defaults to a biber version once upstream fixes change them) | planned |

# Biber upstream bug log

Bugs, doc/code mismatches and latent hazards found in **biber 2.22** (HEAD `02f32dae`, whose `lib/` is identical to the v2.22 release apart from the version string) while planning bibe-rs. The goal is to report these to the maintainers (https://github.com/plk/biber/issues).

**Process:** every new finding gets the next free `BB-NNN` id. Nothing is filed upstream until it has a minimal reproducer that has been checked against the released binary. Once filed, record the upstream issue link in the *Upstream* column. When the Rust port deliberately reproduces a bug for output compatibility, the `Quirk` it uses is named here (see PLAN.md §6.4).

**Status key**
- **V**: independently re-verified by the coordinator with a fresh reproducer (`scratch/verify/`).
- **C**: confirmed by running TeX Live 2026's biber 2.22. Reproducers are under `scratch/<area>/` as cited in the research reports.
- **R**: found by reading the code only. Needs a reproducer before filing.

**Source** says which research report has the full details: `01` input, `02` config/datamodel, `03` processing, `04` sorting, `05` output. `01-B3` means bug B3 in `docs/research/01-input-parsing.md`.

---

## 1. Crashes and fatal errors on valid input

| Id | St | Location | Summary | Source |
|---|---|---|---|---|
| BB-001 | V | DataList.pm:177,190 | `uniquelist` crashes ("Can't use string ("1") as a HASH ref") when the first labelname family name is literally `final`. One hash holds both the prefix counts and the `{final}` sub-hash. Repro: `author={final, Ann and Bob, B}` twice with `uniquelist=true,maxnames=1`. | 03-B1 |
| BB-002 | C | bibtex.pm:1739-1745 | `@string{a="x", b="y"}` (several macros in one `@string`, valid BibTeX) is re-serialised without commas in pass 1, and pass 2 then hits a fatal syntax error. | 01-B1 |
| BB-003 | C | bibtex.pm:1725, Recode.pm:216 | `\char"7B` / `\char123` decodes to a bare `{` inside the pass-1 re-serialisation. The result is a fatal syntax error, and the following entry is swallowed. | 01-B2 |
| BB-004 | C | Internals.pm:1267 | The `presort` value is interpolated into a regex unescaped: `presort={a(b}` is fatal ("Unmatched ("), and `.`/`+` mis-strip silently. | 04-B2 |
| BB-005 | C | Config.pm:557-561, 363-403 | biber.conf `<noinit>`/`<nolabel>` in the *documented* form crashes ("Not a HASH reference"). The schema form `<noinits>`/`<nolabels>` is silently ignored, as are `nolabelwidthcount` and `nonamestring`. t/configfile.t passes only because the broken block is ignored. | 02-B1 |
| BB-006 | C | Config.pm:363-403 | A single `<sortinclusion>` in biber.conf crashes ("Not an ARRAY reference"). It is missing from ForceArray, unlike `sortexclusion`. | 02-B2 |
| BB-007 | C | Biber.pm:885-896, 4605-4619 | If some refsection has data sources but a section with citations has none, biber crashes ("Can't use an undefined value as an ARRAY reference", exit 102, empty .bbl). | 02-B11 |
| BB-008 | C | bin/biber:294 vs Output/bibtex.pm:56-66 | `--output-fieldcase` is validated case-insensitively but compared case-sensitively, so `--output-fieldcase=Upper` crashes (exit 25). | 05-B11 |
| BB-009 | C | Output/bblxml.pm:561,576,594 | Calls the undefined `bblxml_norm` (should be `_bblxml_norm`), so any entry with an annotation kills `--output-format=bblxml` (exit 102). Masks BB-103. | 05-B12 |
| BB-010 | C | UCollate.pm:64 | `-c ignoreChar=…` (and `entry`, `suppress`, `rewrite`, `undefChar`) dies: "change of ignoreChar via change() is not allowed". Options are applied with `change()`, not `new()`. | 04-B8 |
| BB-011 | C | PAR packaging | The binary dies for `sortlocale` ja/ko/zh/zh_*: the `Unicode::Collate::CJK::*` modules are packed under a bogus path (`Unicode/Collate/CJK;lib/...`). Report against the binary build scripts, not the Perl source. | 04-B7 |

| BB-120 | V | UCollate.pm / Unicode::Collate 1.31 `change()` | `--collate-options level=5` (any level outside 1–4, probably also non-numeric) is passed to Unicode::Collate unchecked, which dies with "Out of memory in perl:util:safesysmalloc": exit 25, empty `.bbl`, no ERROR in the log. Also, after `change(level=>0)` croaks the collator is left half-changed (empty keys). Report to both biber (validate `-c` values) and Unicode::Collate (CPAN). | M1 ucollate |

## 2. Data loss / destructive behaviour

| Id | St | Location | Summary | Source |
|---|---|---|---|---|
| BB-012 | C | Biber.pm:4631-4634 | Tool mode with biblatexml input whose name lacks a known extension (e.g. `data.xml`): the generated `.rng` schema path equals the input path, so the **user's data file is overwritten** before it is read. | 02-B6a |
| BB-013 | C | bin/biber:356-364 | Tool mode on an input file with no extension: the output name equals the input name, so the **input .bib is overwritten**. | 02-B6b |
| BB-014 | C | bin/biber:415, Output/base.pm:57-71 | The `.bbl` is opened and truncated before processing, so any error (e.g. a missing .bib) leaves a 0-byte `.bbl` and destroys the previous good one. No temp-file+rename. Possibly intentional, but undocumented. | 05-B16 |

## 3. Wrong output (silent)

| Id | St | Location | Summary | Source |
|---|---|---|---|---|
| BB-015 | V | DataList.pm:2222 | The "dangling comma" cleanup `s/,(?:\n\s+)?\}\}/}}/g` runs over the whole instantiated entry, including user data: `title={Foo {bar,}}` → `\field{title}{Foo {bar}}`. | 05-B1 |
| BB-016 | V | Utils.pm:1659 | `rangelen` treats a `0` digit in an abbreviated range end as missing: `pages={105-10}` → `\range{pages}{11}` (should be 6). | 05-B3 |
| BB-017 | V | Biber.pm:4392-4409 | Sort keys are built by string-`eval` of `getSortKey(q{$value})`. Backslashes and braces in data break the eval silently, giving an `undef` key that sorts first: `title={The Foo\\Bar}` sorts before `Alpha`. `\\` and `\{` can also be altered. | 04-B1 |
| BB-018 | C | Biber.pm:3155-3162 | After list filtering, `first {$be->get_field('entryset')} @sets` ignores `$_`: if *any* set passes the filter, **every** set and set member is re-added to the list. | 02-B14, 03-B2 |
| BB-019 | C | Internals.pm:600-610, 1640-1650; Biber.pm:1816-1837, 3537-3545, 3694-3702; Internals.pm:62-123 | Per-name and per-namelist option overrides (`useprefix`, sorting name-key template, uniquename template, …) are assigned to loop-outer variables and **leak to all following names**, in labelalpha, sorting, namedis, uniquename and hashes. Results depend on name order. | 03-B5, 04-B9 |
| BB-020 | C | Biber.pm:3545, 3702 | A per-name `uniquename=false` on any one name `next MAIN`s the whole entry out of uniquename processing. | 03-B5b |
| BB-021 | C | Biber.pm (2148, 2203, 2271, 2385, 2448, 3035, 3051, 4020-4052), Config.pm:1017-1024 | `getblxoption(undef, …)` cannot see per-entry options (stored per `{key}{secnum}`), so ENTRY-scoped `labelalpha`, `labeldateparts`, `labeltitle`, `labeltitleyear`, `singletitle`, `uniquetitle`, `uniquebaretitle`, `uniquework` and `uniqueprimaryauthor` set via `options={…}` are silently ignored. | 03-B6 |
| BB-022 | C | Entry.pm:1067-1080 | Crossref inheritance copies the parent's derived date metadata (`datecirca`, `dateuncertain`, era, julian, …) even when the child has its own date. The guard compares date *field* names with date*part* names and never matches. | 03-B3, 05-B18 |
| BB-023 | C | Entry.pm:1048-1060 | The date-part inheritance guard tests only the main `datesplit`, so `origdate`/`eventdate`/`urldate` parts mix: child `origdate=2005` plus the parent's `origmonth`. | 03-B4 |
| BB-024 | C | Names.pm:88-110 | `replace_name`/`splice` (granular XDATA) set 0-based name indices while `add_name` uses 1-based ones, and `splice` skips the last name. The off-by-one changes uniquename visibility. | 03-B8, 01-S1 |
| BB-025 | C | Entry.pm:765-768, 818-875 | A referenced XDATA entry's own references are re-resolved on every reference, so granular name-list refs splice again and duplicate names: 3 referrers give 5 names instead of 3. | 03-B9 |
| BB-026 | C | Entry.pm:1036,1098,798; per-nlid state in DataList.pm | Inherited/XDATA `Names` objects are shared by reference with the parent. Visibility, uniquelist and uniquename are keyed per name-list id, so the last entry processed wins: identical children get different `namehash` values depending on order. | 03-B10 |
| BB-027 | C | Biber.pm:2194-2198 | Work-uniqueness skip looks for the literal token `skipbib` in the `options` field. `skipbib=true` and `dataonly` are not recognised, and set members are never skipped. | 03-B11 |
| BB-028 | C | bbl.pm:457-458; bibtex.pm:234-235; Entry.pm:175-176, 696-697, 1117-1118 | `foreach my $d ($dmh->{datefields}->@*) { $d =~ s/date$// }` mutates the shared data-model helper array in place. For a custom date field whose stem ends in `date` (e.g. `updatedate`), only the first entry gets its circa/era/unknown flags. In tool mode the `dates` class of `--output-field-order` never matches and **URL** is placed instead. biber's own fixture `full-bibtex_biber.bib` encodes the wrong behaviour, and the manual contradicts it. | 05-B4, 05-B5, 03-B17 |
| BB-029 | C | Recode.pm:244; recode_data.xml | (a) `\textdollar`, `\textpercent` and `\textampersand` decode to raw `$ % &`, which breaks the `.bbl` under UTF-8 output (LaTeX error). This is pinned by utils.t:78, so it may be intentional, but it is harmful. (b) Symbol/punctuation/greek macros use `(?: \{\}|\s+|\b)` with a literal space under `/x`-less matching, so `\textdollar{}5` → `${}5`. | 01-B4 |
| BB-030 | C | bibtex.pm:838-839 | Sourcemap `map_field_source`→`map_field_target` with `map_entrytarget` reads the value from the current entry, not the target. | 01-B5 |
| BB-031 | C | bibtex.pm:678-694 | `map_entrykey_starnocited` applies to *every* entry (including `\cite`d ones) when there is no `\nocite{*}`. | 01-B6 |
| BB-032 | C | Utils.pm:1157-1161; bibtex.pm:792 | Sourcemap capture handling: `$0` means the *last* capture; after a `notmatch` step `$1` is `"1"`; captures are interpolated unescaped into later match patterns; and the `$N` digit rules differ between contexts. | 01-B9 |
| BB-033 | C | Utils.pm:1173-1185 | Sourcemap replace strings are evaluated as (Safe-sandboxed) Perl code. On *any* error the match is silently replaced by the stringified regex (`(?^u:…)`). Code execution in replace strings is undocumented. | 01-B8 |
| BB-034 | C | Utils.pm:1778 | `$MAPUNIQ`/`$MAPUNIQVAL`/`$MAPLOOP` are substituted only inside `map_foreach`; elsewhere they stay literal. Intent unclear from the docs. | 01-B7 |
| BB-035 | C | bibtex.pm:1129-1139 | `_xsv` passes an arrayref to `add_xdata_ref`, so granular XDATA references in xsv fields (`keywords={xdata=xd-keywords}`) are never resolved and are output literally. | 01-B10 |
| BB-036 | C | bibtex.pm:1020-1036 | Annotations are split on `;` before literal-quote handling: `="literal; x"` loses its literal flag, and the fragment overwrites the annotation with undef. | 01-B11 |
| BB-037 | C | Utils.pm:1417-1479; bibtex.pm:1384-1386 | Unspecified ISO 8601-2 dates outside the supported `X` patterns (`199X-05`, `XXXX`) silently become an open-ended empty year, with no warning. | 01-B12 |
| BB-038 | C | bibtex.pm:1913-1930 | Extended name format (`family={Barnes Noble}`) strips protecting braces without recording it, so the protection is lost in the `.bbl`. | 01-B13 |
| BB-039 | C | bibtex.pm:1152-1198 | The range parser contradicts its comments: `-5` and `-` are rejected as malformed; `{12-15}` is split into `{12` and `15}`; `12~` keeps a trailing space. | 01-B3 |
| BB-040 | C | Biber.pm:4622-4625 | Datasource reading stops once all cited keys are found, so `@preamble`s (and `@string`s) in later files are dropped. `\preamble` then depends on which keys are cited. | 01-B18 |
| BB-041 | C | bibtex.pm:1266 | Comma sanity checks are skipped for any name that starts with `{` and ends with `}`, even if not wholly braced (`{Doe}, J, K, {L}`). | 01-B19 |
| BB-042 | C | Biber.pm:3332, DataModel.pm:337 | A sort column's type comes from its first item only: `labelyear`/`labelmonth`/`labelday` sort as strings (−50, 1000, 2000, 80, 999), and `\sort{\field{year}\field{title}}` makes the title fallback numeric. | 04-B10 |
| BB-043 | C | Biber.pm:4283, 4333-4337 vs doc/biber.tex:1923-1927 | `--sortlocale` does not override a per-sort-set `locale`, contrary to the manual. | 04-B4 |
| BB-044 | C | Biber.pm:1076/1111/1211; DataList.pm:2011 | `sortinithash` uses only the template/global locale and ignores `--sortlocale`, `-c locale` and per-set locales, so headings hash inconsistently with the sort. | 04-B6 |
| BB-045 | C | bin/biber:43 vs doc/biber.tex:1974-2010 | The documented `--collate-options="variable => 'shifted'"` syntax creates a bogus key that is silently ignored, and breaks the eval for per-locale columns: the list comes out **unsorted**. | 04-B3 |
| BB-046 | C | Output/bbl.pm:731-746; Output/bibtex.pm:389-404 | `--output-safechars` (and forced recoding for non-UTF-8 output) is applied to the whole entry string, including verbatim fields and keys: `\verb http://example.org/caf\'{e}`. | 05-B15 |
| BB-047 | C | Output/bibtex.pm:665; biblatexml.pm:395 | Year-division map typo `'WinterS'` (Constants uses `winterS`): tool output of `date={2020-32}` loses the season. | 05-B9 |
| BB-048 | C | Output/bibtex.pm:401; Output/base.pm:313 | Non-UTF-8 output check is `encode(...) =~ /\?/`: any literal `?` triggers a spurious "cannot be encoded" warning and full-entry LaTeX recoding. | 05-B10 |
| BB-049 | C | bbl.pm:681 | Sections are enumerated from found `ENTRIES`, so a refsection with no found entries emits no `\refsection` block and loses its `\missing`/`\keyalias` lines. | 05-B22 |
| BB-050 | C | biblatexml.pm:922 | Misplaced parenthesis `fc(_norm($f) eq "$NS:eprint")` (and `_norm` already strips the prefix), so biblatexml `eprint` type/class attributes are ignored. | 01-B16 |
| BB-051 | C | biblatexml.pm:968-975 vs Output/bbl.pm:553-557 | biblatexml URIs are percent-escaped at input from NFD text (`é` → `e%CC%81`), and `urlraw` is escaped too. BibTeX input escapes at output from NFC. | 01-B17 |
| BB-052 | C | Biber.pm:613-640 | If biber.conf has user sourcemaps and the `.bcf` `<sourcemap>` has no driver/style maps, the biber.conf maps are silently dropped. | 02-B13 |
| BB-053 | C | Config.pm:153-158 | `%CONFIG_DEFAULT_BIBLATEX` is applied *after* the user config file, overwriting biber.conf values (maxbibnames, maxsortnames, useprefix, …). Visible in tool mode. | 02-B17 |
| BB-054 | C | Biber.pm:1032-1044 vs DataLists.pm `get_list` | Datalist de-duplication calls a positional sub with named args, so it never matches and duplicate `<bcf:datalist>`s are emitted twice. Latent: biblatex de-dups first. | 02-B12 |
| BB-114 | C | Output/biblatexml.pm:622 (`output` iterates `$section->get_citekeys`) vs Output/bibtex.pm:460-468 | In tool mode the biblatexml writer emits entries in citekey (input) order and ignores the `tool` sorting template, which the bibtex writer applies (the manual says tool-mode output is sorted by the `tool` scheme). t/tool-bltxml.t asserts the sorted list order (`get_keys`), which never reaches the file. Repro: in a copy of `t/tdata`, `biber --tool --configfile=tool-testsort.conf --output-format=biblatexml tool.bib` writes `i3Š, xd1, macmillan, …` (file order) while `--output-format=bibtex` writes `b1, macmillan, dt1, …` (sorted). Corpus: `tests/corpus/ported/tool-bltxml-1` vs `tool-1`. | port-tests |

| BB-115 | V | Recode.pm (`latex_decode`) | With `--decodecharsset=null`, decoding still strips braces (`{I} x` → `I x`) and still interprets `\char`/`\hbox`, so "null" is not a no-op. | M1 latex-recode |
| BB-116 | V | Recode.pm:~218 (`\char` handled with `s///ee`) | `\char` values are string-eval'd: `\char012` is read as **octal** (→ newline), and `\char09`, `\char'89`, non-ASCII digits and values above IV_MAX **silently vanish**, with "Use of uninitialized value" on stderr. Related: BB-003. | M1 latex-recode |
| BB-117 | C | recode_data.xml / Recode.pm | In the `full` decode set, `\ding{99}` silently disappears. | M1 latex-recode |
| BB-118 | C | Recode.pm accent handling | `\' {a}` (space before the braced argument) is not decoded; `\c  ,` puts the cedilla on a space; `\t{oo}` gives `{̑oo}` (tie accent misplaced, braces kept). | M1 latex-recode |
| BB-119 | V | Recode.pm dash handling | `\--`/`\---` are decoded only when followed by a word character: `\--x` → `–x`, but a trailing `y\---` stays literal. | M1 latex-recode |

### 3a. The truthiness of `"0"` (one family, several sites)

Perl treats the string `"0"` as false. Many `if (my $x = …)` / `||` / `return '' unless $s` tests therefore treat a legitimate value `0` as missing. Fixing these as one sweep is probably easiest upstream.

| Id | St | Location | Symptom | Source |
|---|---|---|---|---|
| BB-055 | C | Internals.pm:1342,1472,1487,1676 | `year={0}`, `date={0000}` (1 BCE) and `volume={0}` are treated as missing for sorting and sort after every dated entry. | 04-B5 |
| BB-056 | C | Output/bibtex.pm:273-283, 236, 711, 745, 793 | Tool output drops fields whose value is `0`, drops year-0 dates, and drops midnight times (`T00:00:00`). | 05-B6, B7, B8 |
| BB-057 | R | Internals.pm:1548 | A sort value of `"0"` skips padding/substring, and `final` can copy the `BIBERZERO` sentinel into later columns. | 04-B11 |
| BB-058 | R | DataModel.pm:932-950 | Range constraints with `rangemin`/`rangemax` of `0`, and values of `0`, are skipped. | 02-B22 |
| BB-059 | R | Utils.pm:683 | `normalise_string_bblxml("0")` returns `''`. | 05-B20 |

## 4. Nondeterministic output (Perl hash order)

| Id | St | Location | Summary | Source |
|---|---|---|---|---|
| BB-060 | C | Name.pm:468; bbl.pm:310; biblatexml.pm:180 | Per-name / per-namelist options in `\name{…}{n}{<opts>}` and `{{<opts>}{%` are emitted in random hash order. Six runs gave four different `.bbl` md5s. This breaks reproducible builds and latexmk's "did the .bbl change" checks. | 05-B2 |
| BB-061 | C | Config.pm:1254-1270; Biber.pm:1617-1646 | Auto-promoted `mincrossrefs`/`minxrefs` targets are added to citekeys in hash order. With `sorting=none` (or sort ties) entry order and labelnumbers change between runs. | 03-B7 |
| BB-062 | C | Section.pm `dynamic_set_keys`; bibtex.pm:229-234 | Dynamic set keys under `\nocite{*}` are added in hash order (same effect). | 03-B7b |
| BB-063 | C | bblxml.pm:705-708 | `<bbl:keyalias>` order is random (the `.bbl` writer sorts them; bblxml does not). | 05-B14 |
| BB-064 | R | dot.pm:266 | `sort values %$v` sorts hashrefs by memory address, so `.dot` edge order can vary. | 05-B17 |
| BB-065 | C | (logging) | Warning order in `.blg`/stdout differs between identical runs (observed with `-V`). Minor, but it defeats log diffing. | 02 §6 |
| BB-112 | V | Biber.pm:1450 (`resolve_xdata` over `values %$self`, Entries.pm:68); Entry.pm:913 | With circular XDATA references, the fatal error names a random cycle, and the number of warnings before the exit varies. Repro: in a copy of `t/tdata`, run `biber --noconf xdata.bcf` repeatedly. The ERROR alternates between `'loop'<->'loop:3'` and `'lxd1:loop'<->'lxd2:loop'` (2 vs 6 times in 8 runs). | O3 build |
| BB-113 | C | Utils.pm:1781-1784 (`maploopreplace`: `suniqid`) | `$MAPUNIQ` is generated with `Data::Uniqid::suniqid`, i.e. from the wall-clock time, so a sourcemap that keys new entries with it (`\step[entrynew=\regexp{$MAPUNIQ}]`, as biblatex's own example `95-customlists.tex` does) gives a different `.bbl` on **every** run: the entry keys change, and so does the order of entries whose sort keys tie. Documented as "random", but it defeats reproducible builds and latexmk's "did the .bbl change" check (each run looks like a change). Repro: run `biber --noconf 95-customlists.bcf` twice on the example's `.bcf` (6 runs gave 6 md5s). | corpus generation |

## 5. CLI, config and logging issues

| Id | St | Location | Summary | Source |
|---|---|---|---|---|
| BB-066 | C | Config.pm:125-127 | `--configfile` pointing to a non-existent file is silently ignored. | 02-B3 |
| BB-067 | C | Biber.pm:1710, Utils.pm:478 | `--dieondatamodel` never exits early: the entry is passed in `biber_error`'s `$nodie` slot. | 02-B4 |
| BB-068 | C | bin/biber:368 | Non-tool `--output-format=bibtex` default name uses `s/\..+$/_biber.bib/` on the whole path: `./t1` writes `_biber.bib`, and `sub.dir/t1` writes `sub_biber.bib` in the cwd. | 02-B5 |
| BB-069 | C | bin/biber:389-392; Output/base.pm:56-73 | `-O -` with `--output-directory` writes a file literally named `-`; plain `-O -` also leaves an empty `-` file in the cwd. | 02-B7, B8 |
| BB-070 | C | Biber.pm:894-898 | "No data sources defined!" is warned and then `exit 0` runs without `display_end`, so the message is never shown and the `.bbl` is left empty. | 02-B9 |
| BB-071 | C | Biber.pm:348-351; Utils.pm:368 | The friendlier "Cannot find control file" message (which latexmk special-cases) is unreachable. With `--nodieonerror` the error cascades into a libxml crash. | 02-B10 |
| BB-072 | C | bin/biber:52 | The `--input-encoding` alias is misspelt `input-encodinge` (works only via prefix abbreviation), and the documented `-e` short option does not exist. | 02-B27, 01-S5 |
| BB-073 | C | Biber.pm:4288-4290 | `--fastsort`, documented as a no-op, emits a WARN per sorted list, inflating warning counts. | 02-B29 |
| BB-074 | C | bin/biber:375-378 | Non-tool `--output-format=biblatexml|bibjson` fails late, with the message on stdout rather than as an ERROR. | 02-B30 |
| BB-075 | C | bibtex.pm:165-171, 304-329 | With `--debug`/`--trace`, btparse errors are neither counted nor fatal, so behaviour changes with the log level. The two branches of the if/else at 316-321 are identical. | 01-B15 |
| BB-076 | C | bibtex.pm:1648-1694 | btparse diagnostics name the temp file (`<md5>_<pid>.utf8`) and pass-2 line numbers instead of the user's `.bib` and line. | 01-B20 |
| BB-077 | C | bibtex.pm:127 vs 1667-1670 | The ascii→UTF-8 leniency in `preprocess_file` is dead code: the earlier strict slurp already fails. | 01-B14 |
| BB-078 | C | bibtex.pm:1061-1066 | The misleading "not an integer" warning for `month={February}` is followed by a successful conversion to 2. | 01-B22 |
| BB-079 | C | UCollate.pm:61-70 | Locale-mandated collation options (Thai `variable=shifted`, Danish upper-first) are overridden with only an INFO message; the manual says biber warns. | 04-B13 |
| BB-080 | C | Constants.pm:166-182 | The `isbn` data constraint only checks that `Business::ISBN->new` succeeds, so invalid checksums pass. A separate warning says to "run with --validate_datamodel for details", which then gives none. | 02-B21 |

## 6. Latent bugs, dead code, and doc/schema drift (lower priority; need reproducers)

| Id | St | Location | Summary | Source |
|---|---|---|---|---|
| BB-081 | R | Constants.pm:192,201 | ISSN/ISMN validators read `$_` instead of their argument; this works only because of the caller's loop. | 02-B21, 01-S4 |
| BB-082 | R | Biber.pm:743-760 | The "uniquenametemplate must contain a base part" check is global across templates. | 02-B15 |
| BB-083 | R | Config.pm:466-481 | In biber.conf only the last `labelalphanametemplate` survives. | 02-B16 |
| BB-084 | R | DataModel.pm:66-95 | User data-model merge loses `<entrytype>` scoping of user constraints/entryfields. | 02-B18 |
| BB-085 | R | DataModel.pm:146-151 | A redefined field is removed from the format list keyed by the *new* format. | 02-B19 |
| BB-086 | R | DataModel.pm:136-138, 404-407 | `is_multiscript` is broken twice (dead code). | 02-B20 |
| BB-087 | R | Config.pm:647-651 | The Windows config path in code (`%APPDATA%\biber\biber.conf`) differs from the docs. | 02-B23 |
| BB-088 | R | config.rnc, bcf.rnc, doc/biber.tex | Schema/doc/code drift: `noinits` vs `noinit`; no `nolabelwidthcount` in the schemas; `output_format` enum incomplete; `datepart` vs `date` constraint datatypes; the manual's sample biber.conf is malformed. | 02-B24 |
| BB-089 | R | LangTags.pm | Grammar `[a-zA-z]` matches `[\]^_\``. The parser is unused but compiled at every startup. | 02-B25 |
| BB-090 | R | Constants.pm `%LOCALE_MAP_R` | `'sr-Latn' => 'latin'`, duplicate keys, invalid `en-UK`. | 02-B26 |
| BB-091 | R | Biber.pm:1316-1334 | `$resolved_keys` is declared outside the XDATA-ref loop, so keys can accumulate. | 02-B28 |
| BB-092 | R | Biber.pm:1178-1188 | The bibtex-output pseudo-section reads a shared `.bib` once per refsection. | 02-B31 |
| BB-093 | R | Entry.pm:179 | `clone()` lists `yeardivision` twice and omits `endyeardivision`, so related clones lose it. | 03-B12 |
| BB-094 | R | Internals.pm:714,797 | The labelalpha `v`/`l` caches are keyed by field name only, per section, which is wrong across datalists or templates. | 03-B13 |
| BB-095 | R | Biber.pm:3565 vs 3704 | Name visibility for minyear* uniquename differs between counting and generation. | 03-B14 |
| BB-096 | R | Internals.pm:719,785 | `globalindices || index` ignores a computed global index of 0. | 03-B15 |
| BB-097 | R | Biber.pm:3857-3862 | `uniquelist=minyear` increments `final{list}` twice. | 03-B16 |
| BB-098 | R | Biber.pm:3995-4000 | sortsets `first_index = -1` picks the last key (unreachable while BB-018 exists). | 03-B18 |
| BB-099 | R | doc/biber.tex:459-461 | The manual's description of mincrossrefs doesn't match the code: crossrefs and xrefs are counted separately, and promotion cascades only one level. | 03-B20 |
| BB-100 | R | Biber.pm:2943-2950 | `pluralothers` adjusts only cite visibility, not bib/sort/alpha visibility. Needs a biblatex-side ruling. | 03-B21 |
| BB-101 | R | Biber.pm:4296-4300 | `sort_list` mutates the *global* collate options: `-c upper_before_lower` is ignored, and `-c level` is overridden by sortcase. | 04-B12 |
| BB-102 | R | Lingua::Translit use | The transliterator object is rebuilt and the warning re-emitted on every call (perf plus noise). | 04-B14 |
| BB-103 | R | bblxml.pm:570-572, 586-588 | Annotation arguments are passed in the wrong slots (masked by BB-009). | 05-B13 |
| BB-104 | R | bblxml.pm:683-702 | A globally sorted list with a labelprefix can be emitted twice in bblxml. | 05-B19 |
| BB-105 | R | Name.pm:75-76 | `rawstring` joins nameparts in hash order (harmless today). | 01-S2 |
| BB-106 | R | bibtex.pm:1413 | Legacy field typo `endseaason`. | 01-S3 |
| BB-107 | R | Utils.pm:112-118 | `globU` concatenates raw/NFC/NFD glob results without dedup, so a file may be read twice on APFS. | 01-S7 |
| BB-108 | R | bibtex.pm:1887 | The extended-name part regex is greedy and splits at the *last* `=`. | 01-S8 |
| BB-121 | C | Config.pm `setblxoption` | Setting an undeclared biblatex option autovivifies it into the option-scope table (`%CONFIG_SCOPEOPT_BIBLATEX`), so a later biber.conf can set an option the `.bcf` never declared. Harmless today, but it contradicts the "undeclared options are dropped" rule. | M1 config |
| BB-122 | C | Biber.pm (datalist setup) | A datalist naming a non-existent sorting template silently gets an empty `{}` template (and undef sort-pass options become `{}` at run time) instead of a warning. | M1 config |

## 7. Performance bugs (output-neutral; worth reporting with numbers)

| Id | St | Location | Summary | Source |
|---|---|---|---|---|
| BB-109 | C | DataList.pm:2251-2324 | `namelist_differs_index`/`_nth` rescan every distinct final list per entry, so uniquelist is **quadratic** on documents with many identical author lists (1000/2000/4000 entries: 13.5 s/33.5 s/103 s). | 03 §5 |
| BB-110 | C | Config.pm:1119-1165; Entry.pm:941-950 | Crossref cascade bookkeeping is **cubic** for N children → parent → grandparent (1000 children: 26.7 s; 2000: 244.7 s). Realistic for proceedings → mvproceedings. | 03 §5 |
| BB-111 | C | Biber.pm:4392 (`sort_list`) | A per-sort-set `locale` constructs a new `Unicode::Collate::Locale` per key: 5000 entries take ~40 s versus ~2 s. | 04 §5.5 |

## Not bugs, but worth mentioning upstream

- btparse rejects `@misc{key}` (no fields) and brace-less `@comment text`, both of which BibTeX 0.99 accepts (01-B21).
- `\true{more<list>}` handling `pop`s `others` from live entry data during output (05-B21). Harmless today, but fragile.

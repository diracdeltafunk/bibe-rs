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
| BB-123 | V | btparse (Text::BibTeX 0.91) name splitting | `author = {, John}` (leading comma) kills biber with `FATAL - Caught signal: SEGV`, empty `.bbl`. `Doe,,` and `Doe, ,` also segfault in Text::BibTeX. Report to Text::BibTeX; biber could pre-check. | M1 btparse |
| BB-124 | V | btparse name splitting | `author = {Doe, , John}` silently puts `John` in **suffix** (`suffix={John}`) instead of given. | M1 btparse |
| BB-125 | V | btparse error reporting | **Uncontrolled format string:** btparse passes the offending input token to printf as part of the format string. `title {50%s}` gives the mangled error text `found "{50u}"`, and the agent saw SEGVs with other inputs. User `.bib` data controls a printf format: this is security-relevant, so report it privately to the Text::BibTeX maintainer. | M1 btparse |
| BB-126 | V | btparse / biber input | A NUL byte in a field value aborts biber (exit 134, no message, empty `.bbl`). Real-world trigger: TeX Live's `doc/latex/umthesis/felty.bib`. | M1 btparse |
| BB-127 | C | btparse | `@misc"x"` makes libbtparse exit the whole process. | M1 btparse |
| BB-128 | C | Text::BibTeX `split_list` | Aborts when a piece would be empty (`" and B"`, `"A and "`, two delimiters two spaces apart). Unreachable from normal fields but reachable through sourcemaps. | M1 btparse |
| BB-130 | V | Output/bbl.pm / Business::ISBN | `--isbn10` with any 978-incompatible ISBN (979 prefix) crashes biber (exit 102/25, empty `.bbl`): `as_isbn10` returns undef and biber calls `->isbn` on it. | M1 perl-compat |
| BB-133 | C | Biber.pm:4756 (`get_dependents`) with sourcemap `entry_null` | If a sourcemap creates an entry that ends up in citekeys (via `entry_nocite`, or in any `\nocite{*}` section) and a later step drops that entry with `entry_null`, biber crashes: "Can't call method "get_field" on an undefined value at Biber.pm line 4756". | M2 sourcemap Second trigger (fuzzer): `\cite{Knuth1992q}` where `Knuth1992q` has `related={k9}`, `k9` has `crossref={SmytheJones1992r}`, and that entry has `editor={,,}`. |
| BB-149 | V | Biber.pm:3387-3414 (`uniqueness` loop) | **Endless loop.** A name with no name parts makes the uniquename/uniquelist iteration cycle forever (no bound on the loop). Minimal repro: `@misc{k,author={xdata=xd1-author}}` with no entry `xd1` (or `xd1` lacking `author`/that position), or `author={family=Doe, given="Ja}`. biber 2.22 was still running when killed after 60 s on a one-entry `.bib`. A second trigger (XDATA names with per-entry `maxnames=1`) minimises to 3 entries. **Filed: [plk/biber#520](https://github.com/plk/biber/issues/520).** | M3 fuzzer; verified |
| BB-150 | C | Biber.pm:3603 (`create_uniquename_info`) | Crash with `uniquename=full`: `@proceedings{100,editor={prefix=,uniquename=false}}`, `@inproceedings{k2,shortauthor={á},crossref={100}}`, `\cite{k2}`. | M3 fuzzer (O3) |
| BB-159 | C | Output/bibtex.pm:145; bin/biber:294 | An invalid `output_fieldcase` in biber.conf (`<output_fieldcase>bogus</output_fieldcase>`) crashes ("Can't use an undefined value as a subroutine reference", exit 25): only the command line validates it. Extends BB-008. bibers: ERROR, exit 2 (DC-007). | M4 tool |
| BB-165 | C | Output/bbl.pm:741 (and the other writers) | An `--output-encoding` Encode doesn't know (e.g. `gb18030`: biber does not load Encode::HanExtra) logs "Cannot find encoding", then dies with "Unknown encoding … at bbl.pm line 741": empty `.bbl`, exit 25. bibers: ERROR, exit 2. | M4 io |
| BB-166 | C | Encode / PerlIO `:encoding` (input) | With `--input-encoding=iso-2022-jp`, `iso-2022-jp-1` or `7bit-jis`, any 8-bit byte in the data file makes biber loop forever at end of file; an unrecognised escape sequence does the same (also `iso-2022-kr`). A Perl Encode/PerlIO bug that biber exposes. bibers: ERROR naming the file. | M4 io |
| BB-167 | C | LaTeX/Recode.pm (`--recodedata`) | A `--recodedata` file that kpsewhich can't find logs "Using user-defined recode data file ''" and dies with "Couldn't open : No such file or directory" (exit 2), without naming the file. bibers: ERROR naming the requested file. | M4 io |
| BB-168 | C | Input/file/bibtex.pm (PerlIO `:encoding(UTF-16)` reads) | A UTF-16 `.bib` whose surrogate pair straddles Perl's 1024-byte read chunks (high surrogate at byte 1022) fails to read with "UTF-16:Partial character". Quirk `utf16-chunk-surrogate`; corpus `io/enc-in-utf-16-surrogate-split`. | M4 io |
| BB-176 | C | Input/file/biblatexml.pm (`_datetime`) | Any unparseable biblatexml date (`abc`, `/2001`, `-380`, ` 2001 `, an invalid range start) makes biber die on `->year` of undef: the "Invalid format" warning branch is unreachable because `if (my (…) = list)` is always true. bibers emits the intended warning. | M4 bltxml |
| BB-177 | C | Input/file/biblatexml.pm (field dispatch) | A data-model field with no biblatexml handler (e.g. `<bltx:year>`) dies calling an undefined sub reference. bibers warns and ignores the field. | M4 bltxml |
| BB-178 | C | Input/file/biblatexml.pm | biber dies (Perl/libxml2 errors) on: malformed XML; a file whose biblatexml namespace is not bound to the `bltx` prefix (default namespace or another prefix); an invalid XPath in a map; a map creating a node with an invalid name (`bltx:name[2]` when absent, names with spaces). bibers: ERROR naming the file, exit 2. | M4 bltxml |
| BB-179 | C | Input/file/biblatexml.pm (sourcemaps) | Several biblatexml map steps die: `per_type` (`$entry->type` doesn't exist); `map_notfield` without `map_field_source` (empty XPath); `map_matches` (`$etarget->set` doesn't exist); `map_null` on a missing field; `map_entry_null` (the key stays cited and `get_dependents` dies); `map_entry_clone` (the original is re-created under its own id and biber dies on the clone key). bibers does what each step intends. | M4 bltxml |
| BB-180 | C | Input/file/biblatexml.pm (cited-key lookup) | A cited key containing `'` is interpolated into an XPath, which then fails and biber dies. bibers compares the `id` attribute directly. | M4 bltxml |
| BB-181 | C | Input/file/biblatexml.pm (`_changenode` calls in match/replace steps) | A misplaced parenthesis makes the error paths of `_changenode` die instead of warning. bibers warns. | M4 bltxml |
| BB-185 | C | Output/biblatexml.pm (range fields) | An unresolved XDATA reference in a range field (read from biblatexml as a one-item list) makes the writer die: `xdatarefcheck` tests the array reference and misses it, then the string is dereferenced as a range. bibers writes the reference. | M4 bltxml |

## 2. Data loss / destructive behaviour

| Id | St | Location | Summary | Source |
|---|---|---|---|---|
| BB-012 | C | Biber.pm:4631-4634 | Tool mode with biblatexml input whose name lacks a known extension (e.g. `data.xml`): the generated `.rng` schema path equals the input path, so the **user's data file is overwritten** before it is read. **Filed: [plk/biber#522](https://github.com/plk/biber/issues/522).** | 02-B6a |
| BB-013 | C | bin/biber:356-364 | Tool mode on an input file with no extension: the output name equals the input name, so the **input .bib is overwritten**. **Filed: [plk/biber#522](https://github.com/plk/biber/issues/522).** | 02-B6b |
| BB-014 | C | bin/biber:415, Output/base.pm:57-71 | The `.bbl` is opened and truncated before processing, so any error (e.g. a missing .bib) leaves a 0-byte `.bbl` and destroys the previous good one. No temp-file+rename. Possibly intentional, but undocumented. | 05-B16 |
| BB-155 | C | Input/file/bibtex.pm:338-342 | Tool mode with several data files: `$Biber::MASTER->{comments}` is replaced per file, so only the last file's `@comment`s are written. Repro: `biber --tool a.bib b.bib`, both with `@comment`s. Quirk `tool-comments-last-file`. | M4 tool |
| BB-156 | C | Output/bibtex.pm | Tool-mode and `--output-format=bibtex` output never write the data's `@preamble`s, so a round trip loses them. Quirk `tool-drops-preambles`. | M4 tool |
| BB-182 | C | Output/biblatexml.pm:292 | biblatexml output never writes xsv fields (`keywords`, `entryset`, `related`, the `options` field, …): `get_fields_of_type('field', 'xsv')` asks for a *datatype* `xsv`, but xsv is a format. Quirk `bltxml-xsv-fields`. | M4 bltxml |
| BB-183 | C | Output/biblatexml.pm:115-123 vs Input/file/biblatexml.pm:156 | biblatexml output writes aliases as `<ids><list><item>`, but the reader only reads `<ids><key>`, so aliases are lost on a round trip. Quirk `bltxml-ids-output`. | M4 bltxml |
| BB-184 | C | Output/biblatexml.pm (dates) | biblatexml output writes dates biber cannot read back: unpadded years (`380`, `-99-07-11`) and `1934~-02` make biber die on re-reading its own output (BB-176). Unspecified dates are written as `1999-00` (as BB-153). Reproduced. | M4 bltxml |
| BB-186 | C | Output/biblatexml.pm:471,503 | biblatexml output drops a time whose hour is `0` (`if (my $sh = …)`), e.g. `T00:30:00`. Same family as BB-055…059; quirk `zero-is-empty`. | M4 bltxml |
| BB-187 | C | Entry/Name.pm:245,273 (`name_to_biblatexml`) | Name and name-part annotations are looked up with the name's index as the annotation *name* (`get_annotation('item', $key, $field, $count)`), so they are never written to biblatexml output. Quirk `bltxml-name-annotations`. | M4 bltxml |

## 3. Wrong output (silent)

| Id | St | Location | Summary | Source |
|---|---|---|---|---|
| BB-015 | V | DataList.pm:2222 | The "dangling comma" cleanup `s/,(?:\n\s+)?\}\}/}}/g` runs over the whole instantiated entry, including user data: `title={Foo {bar,}}` → `\field{title}{Foo {bar}}`. | 05-B1 |
| BB-016 | V | Utils.pm:1659 | `rangelen` treats a `0` digit in an abbreviated range end as missing: `pages={105-10}` → `\range{pages}{11}` (should be 6). **Filed: [plk/biber#523](https://github.com/plk/biber/issues/523).** | 05-B3 |
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
| BB-048 | C | Output/bibtex.pm:401; Output/base.pm:313 | Non-UTF-8 output check is `encode(...) =~ /\?/`: any literal `?` triggers a spurious "cannot be encoded" warning and full-entry LaTeX recoding. **Filed: [plk/biber#521](https://github.com/plk/biber/issues/521).** | 05-B10 |
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

| BB-135 | C | Internals.pm `_dispatch_sorting` / `_sort_list_verbatim` | `_sort_list_verbatim` is unreachable: verbatim/URI *list* fields dispatch to `_sort_verbatim`, which stringifies the array ref, so such entries sort by memory address (`ARRAY(0x…)`), arbitrarily and differently on every run. | M2 sort |
| BB-136 | C | Biber.pm `sort_list` (collator reuse) | The main collator is mutated with `change()` per column and keeps that state: a column with only a per-set `sortcase` override inherits `upper_before_lower` from whichever column last computed a key, and cache hits skip `change()` entirely, so collation depends on evaluation order. | M2 sort |
| BB-138 | C | bbl.pm:335-337, 567-569 (05-B21) | **Observable:** list fields are inherited by reference, so popping `others` while writing one entry removes `\true{more<list>}` from every other entry sharing that list (e.g. crossref children). The URI encoding of URI lists is likewise written back into the shared list. Corpus case `bugs/others-pop-shared-list`. | M2 output |
| BB-139 | R | Name.pm `name_to_bbl` | Per-name option lists keep a trailing comma when the name hash is missing (`{{un=0,uniquepart=base,}{%`). Reproduced by bibers; not seen in the corpus. | M2 output |
| BB-140 | C | bibtex.pm `parsename` | Only *leading* whitespace of a name is trimmed (the trim regex lacks `/g`). | M2 fetch |
| BB-141 | C | bibtex.pm `parsename_x` | `family = Doe` (spaces around `=`) is rejected as an invalid name part: the greedy `(.+)` keeps the trailing space in the part name (cf. BB-108). | M2 fetch |
| BB-142 | C | Utils.pm date preprocessing | If a date mixes ASCII digit runs with non-ASCII digit runs, the ASCII runs are deleted. | M2 fetch |
| BB-143 | C | Utils.pm date parsing | A year-division suffix of `-20` silently turns the date into a bare year. | M2 fetch |
| BB-144 | C | bibtex.pm `cache_data` | The parse cache is per file and per run, so duplicate-key warnings and `everykey` bookkeeping only happen in the first refsection that reads a file. | M2 fetch |
| BB-145 | R | Biber.pm `fetch_data` globbing | A non-`file` data source is pushed onto the datasource list twice. | M2 fetch |

| BB-146 | C | Biber.pm process_extratitle/extratitleyear, generate_uniquetitle/uniquework | When a name list is the `labeltitle` (chicago-authordate via a per-type labeltitlespec), the tracking keys are built by string interpolation of the `Names` object, i.e. its memory address `Biber::Entry::Names=HASH(0x…)`: the values compare object identity, not names, and the keys differ on every run. Seen in the chicago phase dumps. | M2 entries |
| BB-147 | C | Biber.pm:1855-1857 (`process_namedis`) | For a name without a base part (e.g. only `given=Plato`) the namestrings are `["", "P"]` but the schema is `[["given","init"]]`: the arrays are misaligned by one, so `un`/`unall` report `given=init` for index 0. Reproduced by bibers. | M2 uniqueness |
| BB-148 | C | Biber.pm `process_workuniqueness` | Autovivifies empty `uniqignore{key}{uniquetitle|uniquework|…} = []` entries in the global config state. No output effect. | M2 entries |

### 3a. The truthiness of `"0"` (one family, several sites)

Perl treats the string `"0"` as false. Many `if (my $x = …)` / `||` / `return '' unless $s` tests therefore treat a legitimate value `0` as missing. Fixing these as one sweep is probably easiest upstream.

| Id | St | Location | Symptom | Source |
|---|---|---|---|---|
| BB-055 | C | Internals.pm:1342,1472,1487,1676 | `year={0}`, `date={0000}` (1 BCE) and `volume={0}` are treated as missing for sorting and sort after every dated entry. | 04-B5 |
| BB-056 | C | Output/bibtex.pm:273-283, 236, 711, 745, 793 | Tool output drops fields whose value is `0`, drops year-0 dates, and drops midnight times (`T00:00:00`). | 05-B6, B7, B8 |
| BB-057 | R | Internals.pm:1548 | A sort value of `"0"` skips padding/substring, and `final` can copy the `BIBERZERO` sentinel into later columns. | 04-B11 |
| BB-058 | R | DataModel.pm:932-950 | Range constraints with `rangemin`/`rangemax` of `0`, and values of `0`, are skipped. | 02-B22 |
| BB-059 | R | Utils.pm:683 | `normalise_string_bblxml("0")` returns `''`. | 05-B20 |
| BB-137 | C | Entry.pm `has_keyword`; Biber.pm:3155 | `has_keyword` treats a keyword `0` as not found, and BB-018's `first {…}` is false when the first set key is `0`. | M2 sort |
| BB-153 | C | Output/bibtex.pm:730,736 (`construct_datetime`) | ISO 8601-2 unspecified dates are corrupted: `date={1999-XX}` is written as `DATE = {1999-00}` (`1999-01-XX` → `1999-01-00`, `1999-XX-XX` → `1999-00-00`), because `'XX'` goes through `sprintf('%.2d')` (with Perl "isn't numeric" warnings). Quirk `bibtex-unspecified-dates`. | M4 tool | *Also in biblatexml output (BB-184).*
| BB-154 | C | Output/bibtex.pm (`construct_annotation`) | Literal annotations lose their quotes on output: `author+an={1="Lit"}` is written as `{1=Lit}`, which reads back as a non-literal annotation. Quirk `bibtex-literal-annotations`. | M4 tool |
| BB-158 | C | Output/bibtex.pm (`%RSTRINGS`); Text::BibTeX | A `@string` whose name has a decomposable character (`@string{GÖTTINGEN={…}}`) loses its text: the name from `fieldlist` is NFD, `$entry->get`/`macro_text` with it return undef, so `--output-all-macrodefs` writes `@STRING{GÖTTINGEN = ""}` and the macro is not recognised in fields. Reproduced by bibers as Text::BibTeX semantics. | M4 tool |
| BB-169 | C | Output/base.pm:313; Output/bibtex.pm:401 | With UTF-16/32 or UCS-2 output, *every* entry is judged unencodable: the test is `encode($enc,$s,sub{"\0"}) =~ /\0/`, and these encodings always contain NUL bytes. Every entry gets a WARN and full LaTeX recoding. Quirk `unicode-output-nul-check`. **Filed: [plk/biber#521](https://github.com/plk/biber/issues/521).** | M4 io |
| BB-170 | C | Input/file/biblatexml.pm (names) | On a simple `<bltx:namepart initial="X">`, the `initial` attribute is read from the enclosing `<bltx:name>` instead, so `<bltx:namepart type="given" initial="X">Anders</bltx:namepart>` gets initial `A`. Quirk `bltxml-namepart-initial`. | M4 bltxml |
| BB-171 | C | Input/file/biblatexml.pm:893 area (related) | `relatedoptions` is read from a `relatedoptions` attribute on `<bltx:item>` instead of `options`, so it is always empty. Quirk `bltxml-relatedoptions`. | M4 bltxml |
| BB-172 | C | Input/file/biblatexml.pm (sourcemaps) | In biblatexml maps, `map_matchi`/`map_notmatchi` without a replacement are case-sensitive (`imatch` is called without the flag), and `map_notmatchi` is not negated. Quirk `bltxml-smap-matchi`. | M4 bltxml |
| BB-173 | C | Input/file/biblatexml.pm (sourcemaps) | Mapping a field to a new non-name field (`\step[fieldsource=title, fieldtarget=subtitle]`), or using a node value as a string, stores the serialised XML (`<bltx:title xmlns:bltx="…">Title</bltx:title>`) instead of its text. Quirk `bltxml-smap-node-string`. | M4 bltxml |
| BB-174 | C | Input/file/biblatexml.pm:250-268 | In cited-key (non-allkeys) mode biblatexml aliases are never consulted: biber looks for a non-existent `<bltx:id>` element instead of `<bltx:ids><bltx:key>`, so citing an alias gives "I didn't find a database entry". Quirk `bltxml-cited-aliases`. | M4 bltxml |
| BB-175 | C | Input/file/biblatexml.pm:380 | `map_entry_new` creates `XML::LibXML::Element->new("bltx:entry")`: a local name with a colon and no namespace. XPaths on it fail ("XPath error : Undefined namespace prefix" on stderr), `exists()` treats the error (-1) as true, so map steps never set fields on new entries. Quirk `bltxml-smap-new-entry-ns`. | M4 bltxml |

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
| BB-151 | C | Biber.pm `resolve_xdata` / granular XDATA | Beyond BB-112's message order, the *output* depends on Perl hash order: with `@patent{a,xdata={xd1, xd1},location={xdata=xd1-location-2 and Berlin}}`, `@article{b,location={xdata=xd1-location-1 and Berlin}}`, `@xdata{xd1,location={Paris and others}}`, entry `a` gets `Paris` or `others` from run to run. | M3 fuzzer (O3) |
| BB-152 | C | Output/base.pm (PerlIO `:encoding` layer) | With a non-UTF-8 `--output-encoding`, Perl's `:encoding` layer encodes in 1024-byte chunks: a multi-byte character straddling a chunk boundary is output as `\x{fffd}` unless the target encoding has a character starting with the cut-off bytes, in which case the remainder is carried over and later boundaries shift. Output therefore depends on where characters fall in the file. Reproduced by bibers behind `perlio-encoding-split`; cases `fuzz/perlio-encoding-split-{ascii,latin1}`. Arguably a Perl (PerlIO::encoding) bug. | M3 fuzzer | *M4: also applies to multi-byte encodings; for iso-2022-jp, HZ and iso-2022-kr the cut-off part is dropped or `\x{fffd}`, and iso-2022-kr re-emits its escape at the start of every line.*
| BB-162 | C | Output/bibtex.pm (`getblxentryoptions` uses `keys`) | The options in the `OPTIONS` field of tool/bibtex output are written in Perl hash order, so the field changes between identical runs. bibers sorts them by name (DC-054). | M4 tool |
| BB-188 | C | DataModel.pm `generate_bltxml_schema`; Output/biblatexml.pm | The generated `.rng` schema lists entry types, field types and datatypes in Perl hash order, and biblatexml output writes per-namelist/per-name option attributes in hash order, so both change between identical runs. bibers: entry types in declaration order, field and data types sorted; option attributes in `.bcf` declaration order (DC-066). | M4 bltxml |


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
| BB-157 | C | Biber.pm (`normalise_utf8` only in `parse_ctrlfile`) | In tool mode the encoding name is not normalised, so `--tool -E utf8` (Perl's lax alias) makes every entry containing `?` hit the "cannot be encoded in 'utf8'" path (BB-048) and full LaTeX recoding. Reproduced by bibers (corpus `tool/data-utf8-alias`). | M4 tool |
| BB-160 | C | bin/biber:356-364 | Tool output naming runs `s/\.[^.]+$/_bibertool.bib/` on the whole path: an extension-less input in a dotted directory (`my.dir/refs`) gives `my_bibertool.bib` in the current directory. Variant of BB-013; bibers writes `my.dir/refs_bibertool.bib` (DC-011). **Filed: [plk/biber#522](https://github.com/plk/biber/issues/522).** | M4 tool |
| BB-163 | C | data/schemata/bcf.rnc | `bcf.rng` lacks the `map_entry_nocite` attribute that biber implements (Input/file/bibtex.pm:475) and biblatex 3.21 writes for `\step[entrynocite]`, so `--validate-control` rejects valid control files (chicago-notes, sbl styles) with a fatal ERROR, exit 2. Corpus `xml/validate-control-chicago-notes`. | M4 xml |
| BB-164 | C | DataModel.pm `generate_bltxml_schema` | The generated biblatexml schema rejects biber's own `t/tdata/biblatexml.bltxml` under `--validate-bltxml`: no `bltx:key` content for `bltx:ids`, no `id` attribute on `bltx:name`, no `field`/`name`/`literal` attributes on `bltx:annotation`. biber reads the file fine, but validation fails with ERROR, exit 2. | M4 xml |

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
| BB-091 | R | Biber.pm:1316-1334 | `$resolved_keys` is declared outside the XDATA-ref loop, so keys can accumulate. bibers reproduces this (resolve), but no input with an output effect has been found yet. | 02-B28 |
| BB-092 | R | Biber.pm:1178-1188 | The bibtex-output pseudo-section reads a shared `.bib` once per refsection. | 02-B31 |
| BB-093 | V | Entry.pm:179 | `clone()` lists `yeardivision` twice and omits `endyeardivision`, so related-entry clones lose the end date's season: `date={2001-21/2002-23}` gives the original `endyeardivision=autumn`, the clone only `yeardivision=spring`. Corpus case `bugs/bb-093-clone-endyeardivision`. | 03-B12; verified in M2 |
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
| BB-129 | C | Text::BibTeX quirks passed through by biber | (a) In "First von Last" splitting the run of lowercase words ignores commas (`a, b` → von `a b`, given `b`); (b) a `@string` whose value refers to itself expands twice (`m = "a" # m # "b"` → `aabb`); (c) the lowercase test for von detection uses Unicode 6.1 tables; (d) any `@` followed by no type name (`@{`, `@,`, `@123`) silently ends reading of that file. | M1 btparse |
| BB-131 | C | Constants.pm ISMN constraint / Business::ISMN | The ISMN data-model check never fails: `is_valid` returns -1/-3 for invalid input, both Perl-true. Also, Business::ISMN ranges with an undef upper bound never match. | M1 perl-compat |
| BB-132 | C (library) | Text::Roman `isroman` (used by `rangelen`) | `isroman(" ")` and `isroman("X I")` are true (its class `[@RSN]` accepts spaces), but `roman2int` returns undef for them, so a range like `X I-XV` likely yields an undef length. Effect on biber output suspected, not yet shown. | M1 perl-compat |
| BB-134 | C | bibtex.pm `map_foreach` | `map_foreach="<field>"` on an entry lacking that field loops over the literal field *name* as if it were the value; and a `$MAPLOOP` value of `"0"` is never substituted (Perl truthiness). Both reproduced by bibers. | M2 sourcemap |
| BB-161 | R | Output/bibtex.pm `output()` | Dies (`->[0]->get_keys` on undef) if the global `sortingtemplatename` differs from that of the section-99999 list. bibers falls back to the section's first entry list. | M4 tool |

## 7. Performance bugs (output-neutral; worth reporting with numbers)

| Id | St | Location | Summary | Source |
|---|---|---|---|---|
| BB-109 | C | DataList.pm:2251-2324 | `namelist_differs_index`/`_nth` rescan every distinct final list per entry, so uniquelist is **quadratic** on documents with many identical author lists (1000/2000/4000 entries: 13.5 s/33.5 s/103 s). | 03 §5 |
| BB-110 | C | Config.pm:1119-1165; Entry.pm:941-950 | Crossref cascade bookkeeping is **cubic** for N children → parent → grandparent (1000 children: 26.7 s; 2000: 244.7 s). Realistic for proceedings → mvproceedings. | 03 §5 |
| BB-111 | C | Biber.pm:4392 (`sort_list`) | A per-sort-set `locale` constructs a new `Unicode::Collate::Locale` per key: 5000 entries take ~40 s versus ~2 s. | 04 §5.5 |

## Not bugs, but worth mentioning upstream

- **libxml2 (2.9.x) RELAX NG interleave bug, which biber inherits:** when an operand of an `interleave` is itself an `interleave`, libxml2 rejects some valid documents. Example: `interleave(interleave(a?,b?), interleave(c?,d?))` rejects `<r><a/><c/><b/><d/></r>` but accepts `acdb`. In biber this only affects `--validate-bltxml` against the generated biblatexml schema. bibers reproduces it (quirk `libxml2-interleave`). Report to libxml2, not biber.
- btparse rejects `@misc{key}` (no fields) and brace-less `@comment text`, both of which BibTeX 0.99 accepts (01-B21).
- `\true{more<list>}` handling `pop`s `others` from live entry data during output (05-B21). Harmless today, but fragile.

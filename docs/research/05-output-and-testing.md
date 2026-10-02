# 05 — Output writers, the biber test suite as an oracle, and a differential-testing plan

Scope: `lib/Biber/Output/{base,bbl,bblxml,bibtex,biblatexml,dot}.pm`, the placeholder
instantiation in `lib/Biber/DataList.pm::instantiate_entry`, the `.blg` log, biber's
`t/` test suite, and an empirical determinism/performance study of biber 2.22.

All line numbers refer to `vendor/biber` (HEAD ≈ v2.22). All experiments live in
`scratch/output/` (re-runnable):

| Path | Contents |
|---|---|
| `scratch/output/corpus/ex-*.tex/.bcf` | `\nocite{*}` over `biblatex-examples.bib` for authoryear, alphabetic, numeric-comp, biblatex-chicago (authordate), biblatex-apa |
| `scratch/output/corpus/run1, run2` | two independent biber runs of each (byte-identical) |
| `scratch/output/corpus/synth-{10,25,50,100,200}.{bib,bcf}` | synthetic scaling corpus (920 … 18 400 entries) made by `scratch/output/make_synth.py` |
| `scratch/output/tdata-run/{g1,g2}` | installed biber run twice over every `t/tdata/*.bcf` |
| `scratch/output/bugs/` | minimal reproducers for every confirmed bug below (`base.bcf` + per-bug `.bib`) |
| `scratch/output/classify_tests.py` | assertion classifier for `t/*.t` |

---

## 1. Overview

* The output path is: `Biber::prepare` → per section `create_output_section` (builds one
  *list-independent* string per entry, containing `<BDS>…</BDS>` placeholders) →
  `output()` walks sections → datalists → keys, calling `DataList::instantiate_entry`
  to substitute list-specific values (labelalpha, extradate, uniquename, sortinit, …)
  into the cached string, then optionally re-encodes, then `out()` = `print $fh NFC($s)`.
* The `.bbl` format is a small, regular, line-oriented TeX format (§2). It is
  **almost fully deterministic**: across 5 real-world styles, 53 test fixtures and
  two runs each, every `.bbl` was byte-identical — *except* when per-name / per-namelist
  options are present in the data, whose order is Perl hash order (bug B2, confirmed).
* Ordering rules are simple and mostly alphabetical by field name *within groups*;
  groups are emitted in a fixed code order (§2.6). Nothing depends on .bib order except
  the sort fallback (and set member order), which belongs to the sorting area.
* The hard parts for a byte-exact port are not in the writer itself but in values it
  prints: `sortinithash` = `md5_hex(Unicode::Collate::viewSortKey(sortinit))` at
  level 1 with DUCET **13.0.0** (Unicode::Collate 1.31 is bundled), name hashes,
  `Text::Wrap` semantics (only with `--wraplines`), `URI->as_string` escaping,
  and LaTeX re-encoding (`--output-safechars`).
* Several quirks are *observable* and baked into biber's own golden files (e.g. the
  `dates` class in `--output-field-order` never matches, B5). A compatible port must
  reproduce them (optionally behind a "bug-compat" flag).
* biber's own test suite (51 `.t` files, 1 275 planned tests, ≈1 106 assertion
  statements) is mostly white-box. ≈60–70 % of the assertions can be converted
  mechanically into black-box checks against `.bbl` text; the rest are unit tests of
  Perl functions that become table-driven Rust unit tests. The single best oracle is
  simply the installed biber 2.22 binary run on many `.bcf`/`.bib` inputs.
* Performance: ~0.55–0.68 s fixed startup (PAR unpack + perl + module load), ~1.4 s for
  biblatex-examples (≈100 entries), then linear ≈4 ms/entry: 5 100 entries ≈ 20.6 s,
  20 400 entries ≈ 77 s / 742 MB RSS.

---

## 2. `.bbl` format specification

### 2.1 File-level structure, encoding, line endings

Written by `Biber::Output::bbl::output` (bbl.pm:655-782). Exact byte layout:

```
% $ biblatex auxiliary file $
% $ biblatex bbl format version 3.3 $
% Do not modify the above lines!
%
% This is an auxiliary file used by the 'biblatex' package.
% This file may safely be deleted. It will be recreated by
% biber as required.
%
\begingroup
\makeatletter
\@ifundefined{ver@biblatex.sty}
  {\@latex@error
     {Missing 'biblatex' package}
     {The bibliography requires the 'biblatex' package.}
      \aftergroup\endinput}
  {}
\endgroup
<empty line>
[\preamble{%\n<p1>%\n<p2>%\n…<pn>%\n}\n\n]          (only if @preamble present)
for each section number, sorted as STRINGS ("0","1","10","2",…):
\n\refsection{<n>}\n
  for each non-empty datalist (order §2.2):
  ··\datalist[<type>]{<listname>}\n
     [sortdebug: ····% sorting key for '<key>':\n····% <sortkey>\n]
     <entry blocks / or alias? no — see below>
  ··\enddatalist\n
  ··\keyalias{<alias>}{<realkey>}\n      for each cited alias, sorted by alias (string sort)
  ··\missing{<key>}\n                    for each undefined cited key, sorted (string sort)
\endrefsection\n
\endinput\n\n
```

* Header text: bbl.pm:42-61 (heredoc `<<~EOF`, so leading indentation is stripped
  and the final line is empty, i.e. the head ends `\endgroup\n\n`). Version string
  is `$BBL_VERSION = '3.3'` (Constants.pm:53).
* `\refsection` is preceded by `\n`, so there are **two** blank lines between
  `\endgroup` and the first `\refsection`. With a `\preamble` block the layout is
  `\endgroup`, one blank line, `\preamble{%`…`}`, two blank lines, `\refsection{0}`
  (verified on `t/tdata/sections.bcf`).
  Between `\endrefsection` and the next `\refsection{}` there is one blank line.
* Tail is `\endinput\n\n` (bbl.pm:95) — the file ends with an empty line.
* `\preamble`: all `@preamble` strings joined with `"%\n"`, wrapped in
  `\preamble{%\n … %\n}\n\n` (bbl.pm:76-94). Subject to safechars/encoding recode.
* Section loop key: `sort keys %{ENTRIES}` — lexicographic, so refsection 10 is
  emitted before refsection 2. Only sections that have at least one *entry* output
  appear (a section containing only missing keys produces no `\refsection` at all,
  because the loop is over `ENTRIES`, not sections): a document whose only citation
  is an undefined key gets a `.bbl` with **no `\missing{}` line at all** (B22,
  confirmed with `scratch/output/bugs/miss.tex`).
* Encoding: the file handle is opened `>:encoding(<output_encoding>)` (base.pm:57-71,
  default `UTF-8`); every write goes through `out()` = `print $fh NFC($string)`
  (Utils.pm:1575-1578). So output is **NFC**. Line endings are `\n`; on Windows
  Perl's default `:crlf` layer would translate to CRLF (porting note: decide
  explicitly; macOS/Linux output is LF).
* `-` as output file → STDOUT (bbl.pm:664-670).
* **No temp-file + rename.** The output file is opened (and truncated) in
  `bin/biber:415` *before* `prepare` runs; content is written only at the very end.
  Any fatal `biber_error` in between leaves a **0-byte .bbl** (confirmed, B16).
  Malformed `.bcf` explicitly `unlink`s the output (Biber.pm:369-372).

### 2.2 Datalist ordering (bbl.pm:689-760)

1. Take `datalists->get_lists_for_section($secnum)` (insertion = `.bcf` order),
   stable-sort by `sortingtemplatename` (string `cmp`; Perl's mergesort is stable).
2. Drop every list whose `sortingtemplatename` equals the global
   `sortingtemplatename` **and** whose type is `entry`.
3. Append *all* lists matching `section, type=entry, sortingtemplatename=<global>`
   (in insertion order). Rationale (comment): biblatex treats the last list as the
   global one for labelnumber.
4. Skip lists with 0 keys.

Header line: `  \datalist[<type>]{<name>}` where type ∈ {`entry`,`list`} and name is
the full list id, e.g. `nty/global//global/global/global`
(= sortingtemplate/sortingnamekeytemplate/labelprefix/uniquenametemplate/
labelalphanametemplate/namehashtemplate). Entries inside are in the list's sorted
key order. An entry appears once **per list**, re-instantiated per list.

### 2.3 Entry block

Template is built once (bbl.pm:196-645), then placeholders replaced per list
(DataList.pm:1813-2228). Indentation: 4 spaces for `\entry`/`\endentry`, 6 for
contents, 8 for name items and `\list` items, 11 for name parts.

```
    \entry{<key>}{<type>}{<opts>}{<citecount>}
      [\set{<m1>,<m2>,…}]                   set parents only (ENTRYSET placeholder)
      [\inset{<set1>,…}]                    set members
      … names / lists / hashes / labels / fields / dates / xsv / ranges /
      … verbatim / keywords / annotations / warnings (order §2.6)
    \endentry
```

* `<type>` = `$dm->get_outcase($entrytype) // $entrytype` (original case from DM).
* `<opts>` = `filter_entry_options` (Utils.pm:1121-1138): per-entry biblatex options
  **sorted by name**, only those with `backendout="1"` in the `.bcf` optionscope,
  rendered `name=value` with booleans mapped to `true`/`false`
  (`map_boolean(...,'tostring')`); if the value is false-y as a *string* (`''`, `0`)
  just `name` is printed (since `$val ? "=$val" : ''`; booleans are already
  "true"/"false" so this only affects non-boolean options with value 0/'').
  Joined with `,`, no spaces. Example:
  `\entry{Static3}{book}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}`.
* `<citecount>`: `$section->get_citecount($key)`; `-1` → empty string. Entries
  pulled in by `\nocite`/`*`/crossref show `{}`.
* Set parents (`entrytype eq 'set'`, bbl.pm:227-267) get only: `ENTRYSET`,
  (`LABELALPHA`,`EXTRAALPHA` if labelalpha), `SORTINIT`, `SORTINITHASH`,
  `LABELPREFIX`, then raw `\field{label}`, `\field{annotation}`,
  `\field{shorthand}`, `\keyw{…}` (note: these four are **not** escaped, wrapped or
  case-mapped, unlike `_printfield`), then `\endentry`.
* Skipped entirely if the entrytype has `skip_output` in the DM.

### 2.4 Names (bbl.pm:274-329, Name.pm:421-487)

For each name list field in `dmh->{namelists}` (= DM name-list fields, minus
`skip_output`, **sorted by name**):

```
      [\true{more<field>}]                         if "and others"
      [\true{morelabelname}]                       if also the labelname field
      \name{<field>}{<count>}{<nlopts>}{%
        {{<pnopts>}{%
           <np>={<value>},
           <np>i={<initials>},
           [<np>un=<n>,]                           only if uniquename≠false and not a base part
           …}}%
        …
      }
```

* `<count>` = number of names (excluding "others").
* `<nlopts>` only for the labelname list: `ul=<n>` (from `<BDS>UL-<id></BDS>`, removed
  with following comma if undefined) + per-namelist options with `backendout`
  (`name=value`) — **emitted in Perl hash order** (bbl.pm:310; bug B2).
* `<pnopts>`: `un=<n>,uniquepart=<part>` (if uniquename≠false), then per-name
  options (**hash order**, Name.pm:468; bug B2), then `hash=<md5>` (removed with
  comma if no hash). Example:
  `{{un=0,uniquepart=base,hash=87c8f49122411d5d03d1a60a4da2bbdb}{%`.
* Name parts in **DM constant `nameparts` order** (`.bcf`:
  `family,given,prefix,suffix`), only non-empty parts. Value: if the part was
  brace-stripped on input → `{…}` added back (`add_outer`), else `join_name`:
  whitespace → `\bibnamedelimb `, unescaped `~` → `\bibnamedelima `, and any
  `\bibnamedelim[ab]` after a `.` → `\bibnamedelimi` (Utils.pm:1104-1111).
  Initials: `join('\bibinitperiod\bibinitdelim ', inits) . '\bibinitperiod'`, then
  every `\p{Pd}` → `\bibinithyphendelim ` (Name.pm:443-444).
* Lines are joined by `",\n"`; the block is closed by `}}%\n`. When the last
  `UNP-*` placeholder is removed, `…},}}` remains and is fixed by the global
  "dangling comma" regex (DataList.pm:2222) — which also hits user data (bug B1).

### 2.5 Placeholders (`<BDS>…</BDS>`) and instantiation

`instantiate_entry` (DataList.pm:1813-2228) performs a series of global regex
substitutions on the *whole* entry string (user data included):
`ENTRYSET`→`\set{…}`; `UNIQUEPRIMARYAUTHOR|UNIQUEWORK|UNIQUEBARETITLE|UNIQUETITLE|SINGLETITLE`
→`\true{…}`; `EXTRAALPHA|LABELALPHA|EXTRATITLEYEAR|EXTRATITLE|EXTRANAME|EXTRADATE|LABELPREFIX|SORTINIT`
→`\field{…}{v}`; `SORTINITHASH`→`\field{sortinithash}{md5_hex(viewSortKey(sortinit))}`;
`[<nl>]{BIBNAMEHASH,NAMEHASH,FULLHASH,FULLHASHRAW}`→`\strng{…}{v}`; `UL-<id>`, `UNS-<id>`,
`UNP-<id>`, `UNP-<part>-<id>`, `<id>-PERNAMEHASH` inside name blocks. Then:

1. `s/,(?:\n\s+)?\}\}/}}/g` — dangling comma cleanup (**corrupts data**, B1).
2. `s/^\s+<BDS>[^<]+<\/BDS>\n//mg` — any line consisting solely of an unreplaced
   placeholder is deleted (so absent values produce *no line*, never an empty field).

Values are inserted unescaped. `sortinithash` reproduction:
`md5_hex("[" . join(" ", map {sprintf "%04X", primary} …) . " | | |]")`, e.g.
`D` → `[1FEB | | |]` → `6f385f66841fb5e82009dc833c761848` (verified with system
perl's Unicode::Collate; biber bundles Unicode::Collate 1.31 / `allkeys-13.0.0`).
The collator is `Unicode::Collate::Locale->new(locale => <sorting template locale>,
level => 1)` (Biber.pm:1076/1111/1211) — locale tailoring therefore changes the hash.

### 2.6 Order of lines inside a (non-set) entry

Fixed code order; "sorted" = Perl default `sort` (codepoint string order) of DM
field names:

1. `\inset{…}` (if set member).
2. Names: `dmh->{namelists}` sorted (§2.4).
3. Lists: `dmh->{lists}` sorted = DM `fieldtype=list` minus name/verbatim/uri/skipout:
   `[\true{more<f>}]` if last item is `others_string` (case-insensitive; item popped —
   **mutates the entry**), then
   `\list{<f>}{<n>}{%\n` + per item `        {<item>}%\n` + `      }\n`.
4. Hash placeholders: `NAMEHASH, FULLHASH, FULLHASHRAW, BIBNAMEHASH`, then for each
   present namelist (sorted): `<f>BIBNAMEHASH, <f>NAMEHASH, <f>FULLHASH, <f>FULLHASHRAW`.
   Output as `\strng{namehash}{…}` etc.
5. `EXTRANAME` (only if a labelname exists).
6. `LABELALPHA` (if `labelalpha` option).
7. `SORTINIT`, `SORTINITHASH`.
8. If `labeldateparts`: `EXTRADATE`, `\field{extradatescope}{…}` (if set),
   `\field{labeldatesource}{…}` if the field *exists* (empty value printed as `{}` —
   seen 1 162× in the corpus).
9. `LABELPREFIX` unless the entry has a shorthand.
10. `EXTRATITLE` (if labeltitle), `EXTRATITLEYEAR` (if labeltitleyear), `EXTRAALPHA`
    (if labelalpha).
11. `\true{crossrefsource}`, `\true{xrefsource}` if defined.
12. `SINGLETITLE, UNIQUETITLE, UNIQUEBARETITLE, UNIQUEWORK, UNIQUEPRIMARYAUTHOR`
    (→ `\true{…}` or line removed).
13. `\field{labelnamesource}{…}`, `\field{labeltitlesource}{…}`,
    `\field{clonesourcekey}{…}` (raw).
14. Plain fields: `dmh->{fields}` = DM fields of fieldtype `field` and datatype ∈
    {entrykey,key,integer,datepart,literal,code} with format `*`, minus skipout and
    xsv, **sorted**. Printed if `length($val)` (so `0` is printed) or (`nullok` and
    field exists). `crossref`/`xref` skipped unless the target key is in the section
    (except in set entries). Rendered by `_printfield` (§2.7). Date parts appear here
    (`year`, `month`, `endyear`, `urlyear`, …) in alphabetical position.
15. Date meta (per DM date field, sorted, prefix `d` = field minus `date`):
    `\field{<d>dateunspecified}{…}`, `\true{<d>datejulian}`, `\true{<d>enddatejulian}`,
    `\true{<d>datecirca}`, `\true{<d>enddatecirca}`, `\true{<d>dateuncertain}`,
    `\true{<d>enddateuncertain}`, `\true{<d>dateunknown}`, `\true{<d>enddateunknown}`,
    `\field{<d>enddateera}{…}` (if `<d>endyear` exists), `\field{<d>dateera}{…}`
    (only if `<d>year` exists **and** `<d>datesplit` true). (Mutation bug B4.)
16. XSV fields (sorted, skipout removed, `keywords` excluded): `_printfield(f, join(',',…))`.
17. `\true{nocite}` if the entry was nocited.
18. Ranges (sorted): `\field{<f>}{<r1>\bibrangessep <r2>…}` where each range is
    `start\bibrangedash end` (`end` preceded by a space only if non-empty; open range
    `100\bibrangedash`), then `\range{<f>}{<rangelen>}` (`-1` if not computable;
    bug B3).
19. Verbatim fields (`vfields`, sorted, datatypes verbatim+uri): for `uri` datatype
    first `\verb{<f>raw}\n      \verb <raw>\n      \endverb\n`, then the value is
    `URI->new(NFC($v))->as_string` (percent-encodes non-ASCII as UTF-8);
    then `\verb{<f>}\n      \verb <v>\n      \endverb\n`.
20. Verbatim lists (`vlists`): optional `\true{more<f>}`; uri-lists first emit
    `\lverb{<f>raw}{<n>}` + `      \lverb <item>` lines + `      \endlverb`; then
    `\lverb{<f>}{<n>}` + items (URI-encoded for uri) + `\endlverb`.
21. `\keyw{<k1>,<k2>}` (raw, no escaping).
22. Annotations, scopes field → item → part, each sorted by field, name, item, part:
    `\annotation{field}{<f>}{<name>}{}{}{<literal 0|1>}{<value>}`,
    `\annotation{item}{<f>}{<name>}{<item>}{}{<l>}{<v>}`,
    `\annotation{part}{<f>}{<name>}{<item>}{<part>}{<l>}{<v>}`.
23. `\warn{\item <warning>}` per warning (data-validation messages; order of insertion).
24. `    \endentry`.

### 2.7 `_printfield` (bbl.pm:105-151)

* Output field name is `$dm->get_outcase($field)` (original case as declared).
* Returns `''` for null values unless DM `nullok`.
* `crossref`/`xref` → `\strng{…}{…}`; all others `\field{…}{…}`.
* Any field matching `/^(.*)(?!end)year$/` whose `<prefix>year` value
  `looks_like_number` is printed as `abs()` (astronomical year; era in `dateera`).
  The lookahead is a no-op (harmless): `endyear` is abs'd too.
* Escaping: only if the entry's `datatype` ≠ `bibtex` (i.e. biblatexml input) and not
  `strng`: unescaped `#`, `&`, `%` get a backslash. **BibTeX data is never escaped.**
* `--wraplines=N` (`Text::Wrap`, columns N): if `16 + len(name) + len(value) > 2N`
  (lengths in grapheme clusters via Unicode::GCString) → `\field{f}{%\n` +
  `wrap('      ','      ',$value)` + `%\n      }\n`; else if `> N` →
  `wrap('      ','      ', "\field{f}{v}") . "\n"`; else single line. Only
  `_printfield` lines wrap. Note `$Text::Wrap::columns` is a global side effect.

### 2.8 Post-processing of each instantiated entry (bbl.pm:731-752)

* `--output-safechars`: `latex_encode` (LaTeX/Recode.pm:355-424) on the whole entry
  string, including keys, hashes and **verbatim/URL content** (bug B15). Set chosen by
  `--output-safecharsset` (`base`/`full`/`null`, data `recode_data.xml`, 843 maps).
* Else if output encoding ≠ UTF-8: `encode($enc, NFC($s), sub{"\0"})`; if any char is
  unencodable, the *whole entry* is latex-encoded and a warning
  `The entry '<k>' has characters which cannot be encoded in '<enc>'. Recoding problematic characters into macros.` is issued.
* `--sortdebug` prepends `    % sorting key for '<k>':\n    % <sortkey>\n`.

### 2.9 Porting difficulty / risk summary for `.bbl`

| Feature | Difficulty | Correctness risk |
|---|---|---|
| Static layout, ordering, indentation | low | low (well covered by goldens) |
| Placeholder model | low — but must reproduce the two global cleanup regexes incl. B1 side effects | medium |
| sortinithash | medium (needs DUCET 13.0.0 primary weights + CLDR tailorings as in Unicode::Collate::Locale 1.31) | high if UCA version drifts |
| per-name/namelist option order | trivial, but upstream is nondeterministic → choose canonical order and normalise reference | n/a |
| `URI->as_string` | low/medium (URI.pm escaping rules: which chars are left alone) | medium |
| `--wraplines` (Text::Wrap) | medium (exact break/`unexpand` semantics) | medium, rarely used |
| safechars recode | medium (regex alternation order over sorted keys, diacritic stacking) | medium |
| NFC on output | low (unicode-normalization crate; pin Unicode version) | low |

---

## 3. Other output formats

### 3.1 `bibtex` (tool mode `--tool`, or `--output-format=bibtex` from a `.bcf`)

`lib/Biber/Output/bibtex.pm`. Entries collected in a hash `%acc` of
`OUTFIELDNAME → value` then formatted:

* Header: `@<TYPE>{<key>,\n` where TYPE/field names go through `$outmap`:
  `--output-fieldcase` upper (default)/lower/title (`ucfirst`) applied after
  `--output-field-replace` mapping (`f:r,…`, Config.pm:235-240).
* Content sources: `IDS` (joined `,`); names: `name_to_bibtex` (`prefix family, suffix, given`,
  `~`→space, brace-restored) or `--output-xname` → `name_to_xname`
  (`part=value` pairs sorted by part name, joined by `--output-xnamesep`) joined by
  `" <output_namesep> "` (default `and`), with leading `useprefix=…`,
  `sortingnamekeytemplatename=…` namelist pseudo-names, trailing `and others`;
  lists & verbatim lists joined by `" <output_listsep> "`; `OPTIONS` from per-entry
  options (unsorted, raw values); dates rebuilt by `construct_datetime`
  (ISO 8601-2 incl. `X` unspecified, `?`/`~`/`%`, seasons 21-41, times, tz); with
  `--output-legacy-dates` YEAR/MONTH (month macro names via `%MONTHS`); plain
  fields (`if (my $val = …)` — drops `0`, B6); XSV, ranges (`--` and `,`), verbatim,
  KEYWORDS; annotations as `<FIELD><+an>:<name> = {…}`.
* XDATA references are re-emitted (`xdatarefout`) unless `--output-resolve-xdata`.
* Field ordering: `--output-field-order` (default `options,abstract,names,lists,dates`):
  each listed field (exact outmapped name) or class `names|lists|dates` (all fields of
  that class, alphabetical) emitted in order; **remaining fields alphabetical by
  outmapped name** (`sort keys %acc`). The `dates` class is broken (B5): it matches
  `url` (and `event`/`orig` pseudo-names) instead of `date`/`urldate`/…
* `bibfield` (bibtex.pm:529-581): indent `--output-indent` (`N` spaces or `Nt` tabs,
  default 2); `--output-align` pads field names to the longest (GCString length)
  in the entry; value is printed bare if it is a known macro
  (`Text::BibTeX::macro_length`), or replaced by the `@string` name if the whole value
  equals a string's expansion (`%RSTRINGS`), else `{value}`; then `,\n`.
  `--wraplines` wraps with hanging indent of `indent+namewidth+4`.
* Entry ends `}\n\n`. Whole entry recoded by safechars / encoding check — the
  latter uses `=~ /\?/` (B10).
* File: `@STRING{NAME = "value"}` lines (sorted; only used macros unless
  `--output-all-macrodefs`; suppressed by `--output-no-macrodefs`) + `\n`, then entries in
  the order of the section-99999 global sort list, then `@COMMENT{…}` lines (tool mode).
  Output file is opened only at `output()` time (safe for in-place).
* Default filenames: tool `X_bibertool.bib`; non-tool `X_biber.bib`.
* Mutates entries (`del_field` in `construct_datetime`) — output is destructive.

### 3.2 `bblxml` (deprecated; bblxml.pm)

XML::Writer, namespace `https://sourceforge.net/projects/biblatex/bblxml` prefix `bbl`,
always UTF-8 (ignores `--output-encoding`), indent `--output-indent`, plus an
`xml-model` PI and a generated RelaxNG schema `X.rng` (unless `--no-bblxml-schema`).
Each entry is rendered by a separate writer and spliced in with `raw()` after
`instantiate_entry(..., 'bblxml')` (uses `[BDS]…[/BDS]` inside attributes).
All values pass `_bblxml_norm` = `NFC(normalise_string_bblxml())`, which strips LaTeX
macros and simple braces and turns `~` into space (lossy; returns `''` for `"0"`).
Bugs: crashes on any annotation (B12), alias order nondeterministic (B14), annotation
arg errors (B13), list selection differs from bbl (B19). Low priority for a port.

### 3.3 `biblatexml` (biblatexml.pm; tool mode `--output-format=biblatexml`)

XML::Writer with `bltx` prefix; entries with `<ids>`, `<xdata>`, `<crossref>`,
`<options>`, `<names>` (attrs incl. namelist options — hash order, B2-like, confirmed
for per-name attrs), list fields, plain fields, xsv, ranges (`<start>/<end>`), dates
(`<date type=…>` with `<start>/<end>` and ISO strings; same `WinterS` typo B9),
annotations. Fields iterated in sorted DM order.

### 3.4 `dot` (deprecated; dot.pm)

GraphViz output of sections/entries/fields and crossref/xdata/xref/related/set edges,
`--dot-include=section,field,xdata,crossref,xref,related`. Mostly sorted; one
nondeterministic `sort values %hashrefs` (B17). The fixture `t/tdata/full-dot.dot`
reproduces byte-exactly with biber 2.22.

### 3.5 `.blg` log

* Log4perl, file appender `clobber`, UTF-8, layout `[%r] %F{1}:%L> %p - %m%n`
  (Config.pm:311-320): `[<ms since start>] <PerlFile>:<line>> LEVEL - message`,
  e.g. `[236] bibtex.pm:1519> INFO - Found BibTeX data source '…'`; the main script
  appears as `biber-darwin:342` in the packed binary. Screen: `LEVEL - message`
  (SimpleLayout) on STDOUT.
* Name: `--logfile` or `<bcf>.blg` (in `--output-directory` if given). `--nolog`
  disables, `--quiet` (ERROR level) still creates an empty `.blg`.
* First lines: `This is Biber 2.22`, `Logfile is '…'`, `=== <localtime>`; last lines:
  deferred warnings re-logged, `WARNINGS: n`, `ERRORS: n` (display_end, Biber.pm:117-140).
* Non-deterministic content: `[ms]` stamps and the `===` date line. File:line tags
  are Perl-implementation-specific. Tools that parse it (latexmk: `Found BibTeX data
  source`, `WARN`/`ERROR` lines, `WARNINGS:` count) only need message text — a port
  should keep the message strings and the `LEVEL - ` shape but cannot sensibly keep
  `Biber.pm:4487`-style tags (recommend emitting plausible fixed tags or dropping them;
  open question).

---

## 4. Existing test suite analysis

### 4.1 How tests drive biber

Two styles:

1. **In-process white-box** (≈48 of 51 files): `chdir t/tdata`, `Biber->new(noconf=>1)`,
   init Log4perl, `$biber->parse_ctrlfile('x.bcf')`, `set_output_obj(Biber::Output::bbl->new)`,
   then **programmatic option overrides** (`Biber::Config->setoption('sortlocale', …)`,
   `setblxoption(undef,'mincitenames',3)`), `$biber->prepare`, then assertions on
   (a) `$out->get_output_entry($key, $list)` — instantiated `.bbl` text for one entry
   (NFC'd), compared with a literal heredoc via `eq_or_diff`; (b) internal objects:
   `$main->get_keys` (sorted order), `get_sortdata_for_key` (sort key string),
   `get_extradatedata_for_key`, `get_unsummary`, `get_uniquelist`, `get_entryfield`,
   `$bibentries->entry(k)->get_field(f)`; (c) direct calls of utility functions
   (`latex_decode`, `parsename`, `rangelen`, `parse_date`, langtag parsing…).
   Tool-mode tests write to a scalar ref (`$outvar`) and compare full `.bib`/`.bltxml`
   text.
2. **Subprocess golden-file** (3 files, gated by `BIBER_DEV_TESTS`): `full-bbl.t`,
   `full-bibtex.t`, `full-dot.t` run `bin/biber` and `File::Compare` against
   `t/tdata/full-bbl.bbl`, `full-bibtex_biber.bib` (with `--output-align`),
   `full-dot.dot`, plus `like()` on stdout warnings.

**Verified:** the installed biber 2.22 reproduces all three golden files byte-exactly
(`scratch/output/tdata-run/`).

### 4.2 Assertion census (`scratch/output/classify_tests.py`)

51 `.t` files, 1 275 planned tests (`plan tests => …`), 1 106 assertion statements
found statically (loops generate the rest: bcfvalidation 53, dateformats, etc.).

| Category | # | What it checks | Convertible to black-box? |
|---|---|---|---|
| `list_label_data` (uniqueness/labelalpha/extradate/…) | 499 | `get_unsummary`, `get_uniquelist`, `get_extra*data`, `get_labelalphadata`, `get_entryfield` | **Yes, mechanically**: each maps to a substring of the instantiated `.bbl` entry (`un=`, `ul=`, `\field{extradate}`, `\field{labelalpha}`, `\true{singletitle}`…). Needs a small `.bbl` parser + per-assertion translation table. ~90 % |
| `bbl_entry_text` | 153 | full instantiated entry text | **Yes, directly** — expected strings are literal in the `.t` (heredocs/`my $lN = q|…|`); extract with a Perl/regex pass. Need option overrides → CLI flags/`.bcf` edits. ~95 % |
| `unit_function` | 153 | latex_encode/decode, parsename(_x), rangelen, parse_range, langtags, map_boolean, config parsing | Convert to **table-driven Rust unit tests** (inputs/expected are literals). Not golden files. ~85 % |
| `entry_field_internal` | 148 | `$entry->get_field(...)`, labelname/labeldate info, aliases, has_citekey | ~50 % observable in `.bbl` (field values, `\keyalias`, `\missing`, labelnamesource); rest internal (datesplit, raw parsed parts) |
| `sortkey_string` | 53 | `get_sortdata_for_key->[0]` | **Yes**: visible via `--sortdebug` (`% <sortkey>` line). 100 % |
| `sort_order_keys` | 43 | list key order | **Yes**: entry order in `\datalist`. 100 % |
| `tool_output_text` | 19 | full tool-mode `.bib`/`.bltxml`, full-bbl/bibtex/dot | **Yes** (already golden) |
| `process_exit_or_log` | 10 | exit status, warnings on stdout | Yes (message text) |
| other | 28 | misc (datalist attrs, bcf validation) | case-by-case |

Estimate: **≈ 750–800 of ~1 100 assertions** can become black-box golden/predicate
tests over `.bbl`/`.bib` output; ≈130 become Rust unit tables; ≈150 need rewriting or
are implementation-internal. Main friction: in-process option overrides
(`setoption`/`setblxoption`) must be translated to equivalent CLI options or `.bcf`
`<bcf:options>` edits, and some tests mutate config between `prepare` calls.

### 4.3 Fixtures as end-to-end oracle

Running the installed binary over every `t/tdata/*.bcf` (no test-specific overrides):
48 run cleanly, 5 exit 2 by design (`crossrefs`/`xdata` circular-inheritance errors,
`encoding1/3/4` need `--input-encoding`), all 53 byte-identical between two runs.
`remote-files.bcf` fetches a GitHub URL (exclude from offline CI). These outputs
(≈1 650 entries) are an immediate golden corpus even though they are not the exact
configurations the `.t` files assert on.

`testfiles/` contains `test.tex/.bcf/.bib` and two Unicode corpora zips
(`Unicode-bib-files.zip`, `Unicode-tests-preserved-NF.zip`) — useful for encoding/
normalisation differential tests.

---

## 5. Proposed differential / golden testing plan

### 5.1 Oracle

The reference implementation is the **installed biber 2.22 binary** (byte-identical
to its own goldens). Pin it (record `biber --version`, binary sha256, and the PAR
cache's bundled module versions — the cache dir from `biber --cache` contains the full
Perl source of all bundled modules, e.g. `Unicode/Collate.pm` 1.31,
`allkeys.txt` 13.0.0, `Text/BibTeX.pm`). Golden outputs are generated once and
committed (or stored as a content-addressed cache), so CI does not need TeX.

### 5.2 Input sources

1. `t/tdata/*.bcf/.bib/.bltxml` (53 configs) + the converted `.t` expectations (§4.2).
2. biblatex's own 52 example documents:
   `$(kpsewhich -var-value TEXMFDIST)/doc/latex/biblatex/examples/*.tex` (covers refsections,
   refsegments, keyword/category/type filters, labelprefix, related entries, sorting
   schemes, custom lists, dates, annotations, indexing, every standard style).
3. Style matrix: a template doc `\usepackage[style=S,<opts>]{biblatex}` +
   `\addbibresource{B}` + `\nocite{*}` for S in all standard styles (numeric*,
   alphabetic*, authoryear*, authortitle*, verbose*, reading, draft, debug) and
   contributed styles installed in TeX Live (74 `biblatex-*` doc dirs: chicago, apa,
   ieee, mla, gb7714-2015, nature, phys, science, oxref, sbl, ext, iso690, gost, …
   — note some, e.g. apa, ship their own sourcemaps/datamodels and are the most
   demanding). Verified working here: authoryear, alphabetic, numeric-comp,
   chicago-authordate, apa.
4. Option matrix over a few styles: `uniquename` 0..6/minfull, `uniquelist`
   true/false/minyear, `maxnames/minnames/maxbibnames`, `labeldateparts`,
   `labelalpha`, `sorting` (nty, nyt, nyd, ydnt, anyt, none, debug), `sortlocale`
   (en, de, de-1996, sv, fr, ru, zh…), `sortcase`, `sortupper`, `useprefix`,
   `giveninits`, `datamodel` extensions, `\DeclareSourcemap` examples.
5. `.bib` corpora: `biblatex-examples.bib`; the **488 `.bib` files under TeX Live
   `texmf-dist`** (263 `.tex`/95 `.bib` in biblatex-related doc dirs alone); the
   Unicode zips in `testfiles/`; synthetic scaled files (`make_synth.py`); a
   grammar-based fuzzer generating names (prefixes, braces, `and others`, extended
   name format, per-name options), ISO 8601-2 dates (unknown/uncertain/approximate,
   seasons 21–41, eras, times/time zones, open ranges), ranges (roman numerals,
   abbreviated ends), verbatim/URI fields with non-ASCII, crossref/xdata/xref/sets/
   related graphs, `@string` macros.
6. Tool mode: `biber --tool` over all `.bib` files with option matrix
   (`--output-align`, `--output-indent=4|1t`, `--output-fieldcase`,
   `--output-field-order`, `--output-resolve`, `--output-xname`,
   `--output-legacy-dates`, `--output-safechars`, `--output-encoding=latin1|ascii`,
   `--output-format=biblatexml`, `--configfile` sourcemaps).

### 5.3 Generating `.bcf` files automatically

A `.bcf` is written at `\end{document}` on the *first* LaTeX pass, so one
`pdflatex -interaction=nonstopmode -draftmode doc.tex` per configuration suffices
(≈0.3–1 s each; done here for 5 styles). Afterwards the `.bcf` is the real input and
LaTeX is no longer needed:

* swap data sources by editing `<bcf:datasource …>NAME</bcf:datasource>` (used for the
  synthetic corpus);
* vary global options by editing `<bcf:options component="biblatex" type="global">`;
* vary biber options via CLI.

Keep the triple (`.bcf`, data files, argv) + reference outputs (`.bbl`, `.blg`
message list, stdout, exit code) per case. For fuzzing, generate `.bib` only and reuse a
small set of `.bcf` templates.

### 5.4 Normalisation before comparison

`.bbl`: byte comparison after **one** normalisation — sort the comma-separated option
lists inside `\name{f}{n}{<opts>}{%` and inside `{{<opts>}{%` (and keep `un=`,
`uniquepart=` first and `hash=` last, which are deterministic), because upstream
emits them in random hash order (B2). Nothing else needed: no timestamps, no paths,
no hash-order elsewhere observed across 2× (5 styles + 53 fixtures) and 6× (option
tests) runs.

`.blg`/stdout: drop `[ms]` prefixes, the `=== <date>` line, `File.pm:NNN>` tags and
absolute paths; compare the ordered list of `(LEVEL, message)`; treat INFO lines
as advisory, WARN/ERROR as must-match.

`bblxml`: sort `<bbl:keyalias>` siblings (B14). `.dot`: sort edge lines (B17).
`biblatexml`: compare as XML with attribute order ignored.

Exit codes: 0 OK, 2 (`EXIT_ERROR`) on `biber_error`; Perl crashes give other codes
(25, 102…) — a port should map known crash cases to sensible errors and the harness
should mark them "upstream crash, behaviour undefined".

### 5.5 Workflow

1. `make goldens`: run reference biber over the corpus, store outputs + sha256 of
   inputs; check determinism by running twice.
2. Rust harness: for each case run `bibe-rs` with the same argv in a temp dir, apply
   normalisation, diff; on failure auto-minimise the `.bib` (delta-debug over entries,
   then fields) using the reference binary as oracle in the loop.
3. Classify known upstream bugs in an allow-list keyed by bug id so the port can
   choose "compat" or "fixed" mode per bug and the harness knows which diffs are
   expected.
4. Track per-case timing of both implementations (the perf goal).

---

## 6. Performance measurements

Apple M3 (8 cores), macOS, biber 2.22 PAR-packed universal binary (84 MB),
`/usr/bin/time -p|-l`. (Coordinator's independent measurements in `scratch/perf`
agree: ~0.6 s startup, ~1.3 s for biblatex-examples, ~3.5 ms/entry.)

| Run | real s | user s | max RSS | notes |
|---|---|---|---|---|
| `biber --version` | 0.54–0.68 | 0.30 | 108 MB | fixed cost: PAR unpack check + perl + module loading |
| biblatex-examples `\nocite{*}`, authoryear (102 `.bbl` entries, 211 KB) | 1.39–1.48 | 0.94–1.01 | 136 MB | |
| alphabetic | 1.35–1.41 | | | |
| numeric-comp | 1.33–1.38 | | | |
| biblatex-chicago authordate | 1.44–1.50 | | | |
| biblatex-apa (2 datalists → 204 entry blocks, 426 KB) | 1.28–1.54 | | | |
| synthetic 920 entries | 4.68 | | 167 MB | 2.1 MB `.bbl` |
| synthetic 2 300 | 10.31 | | 222 MB | |
| synthetic 4 600 (5 100 blocks) | 20.64 | | 291 MB | 10.7 MB `.bbl` |
| synthetic 9 200 | 41.64 | | 500 MB | |
| synthetic 18 400 | 77.13 | | 742 MB | 42.8 MB `.bbl` |

Scaling is linear at ≈4.1–4.5 ms/entry (synthetic data duplicates every name 50–200×,
which stresses uniqueness bookkeeping somewhat). `biber --cache` →
`/var/folders/…/par-62656e/cache-bcf1e1…` (the PAR extraction cache; already warm in
all measurements). Phase breakdown from `.blg` `[ms]` stamps:

| Phase | 102 entries | 4 600 entries |
|---|---|---|
| start → `.bcf` read | 35–40 ms (+~0.5 s before logging starts) | 35 ms |
| `.bcf` parse | 37 ms | 35 ms |
| locate + btparse + LaTeX decode (to "Found BibTeX data source") | 150 ms (incl. ~90 ms `kpsewhich`) | 2.9 s |
| entry creation + processing (names, dates, inheritance, labels, uniqueness) | 300 ms | 13.8 s |
| sorting | 35 ms | 1.15 s |
| `.bbl` instantiation + write | 21 ms | 1.0 s |

So for typical documents (≤ 300 entries) **~50 % of wall time is fixed startup**; for
large bibliographies entry processing dominates (~67 %), with parsing ~14 %, sorting
~6 %, output ~5 %. Output writing itself is cheap (≈0.2 ms/entry) and is not a
performance concern; a Rust port should still avoid the per-entry chain of ~40 global
regex substitutions over the whole entry string (straight-line formatting with
precomputed list data is equivalent and faster).

Determinism: every `.bbl` in this study (5 styles × 2 runs, 53 fixtures × 2 runs,
synthetic) was byte-identical across runs, except inputs containing per-name /
per-namelist options (B2): 6 runs gave 4 distinct files.

---

## 7. Suspected biber bugs

Reproducers: `scratch/output/bugs/` (`base.bcf` is an authoryear `\nocite{*}` `.bcf`;
`tN.bcf` = `base.bcf` with the datasource renamed to `tN.bib`).

| # | Where | Description | Status |
|---|---|---|---|
| B1 | DataList.pm:2222 | "Dangling comma" cleanup `s/,(?:\n\s+)?\}\}/}}/g` runs over the whole entry incl. user data: `title={Foo {bar,}}` → `\field{title}{Foo {bar}}` (comma silently lost). Any field value containing `,}}` is corrupted. | **CONFIRMED** (`t1.bib`, key `comma`) |
| B2 | Name.pm:468, bbl.pm:310 (also biblatexml.pm:180, Name.pm:229) | Per-name and per-namelist options iterate `keys %CONFIG_SCOPEOPT_BIBLATEX{…}` → random order each run: `{{un=0,uniquepart=base,giveninits=true,useprefix=true,…}` vs `…familyinits=false,terseinits=false,…`; `\name{author}{1}{terseinits=true,useprefix=true,giveninits=true}` vs other orders. Breaks latexmk-style "did .bbl change" checks and reproducible builds. | **CONFIRMED** (`t1.bib` key `pno`: 6 runs → 4 different md5s; `t2.bib`) |
| B3 | Utils.pm:1659 (`next if $n[$i]`) | `rangelen` treats a `0` digit in an abbreviated range end as missing: `pages={105-10}` → `\range{pages}{11}` (105–115) instead of 6 (105–110). `135-38` → 4 is right. | **CONFIRMED** |
| B4 | bbl.pm:457-458 (same pattern bibtex.pm:234-235, Entry.pm:175-176, 696-697, 1117-1118) | `foreach my $d ($dmh->{datefields}->@*) { $d =~ s/date$// }` aliases and **mutates the shared DM helper array**. Idempotent for standard fields, but a date field whose prefix itself ends in `date` (e.g. custom `updatedate` → `update` → `up`) is looked up wrongly from the 2nd entry on: only the first entry gets `\true{updatedatecirca}` / era / unknown flags. | **CONFIRMED** (`t4.bcf` with DM-added `updatedate`) |
| B5 | bibtex.pm:235 + 392-405 | Consequence of B4: the `dates` class of `--output-field-order` compares against the mutated list (`'', 'event', 'orig', 'url'`), so date fields are never placed by class, but **`URL` is** (`URL` printed before `DATE`, out of alphabetical order). biber's own fixture `full-bibtex_biber.bib` encodes this behaviour. | **CONFIRMED** (`tool1.bib`) |
| B6 | bibtex.pm:273-283 (`if (my $val = …)`) | Tool/bibtex output drops any field whose value is `0` (e.g. `volume={0}`, `number={0}`). (`.bbl` path correctly uses `length`.) | **CONFIRMED** |
| B7 | bibtex.pm:236, 711 | Tool/bibtex output drops a date whose year is 0 (`date={0000}`) entirely. | **CONFIRMED** |
| B8 | bibtex.pm:745, 793 (`if (my $sh = …hour)`) | Midnight times are dropped: `date={2020-01-01T00:00:00}` → `DATE = {2020-01-01}`. Similar truthiness tests in biblatexml.pm:338-361. | **CONFIRMED** |
| B9 | bibtex.pm:665, biblatexml.pm:395 | Year-division map key typo `'WinterS'` (Constants uses `winterS`): `date={2020-32}` → `DATE = {2020}` (season lost from DATE). | **CONFIRMED** |
| B10 | bibtex.pm:401, base.pm:313 | Non-UTF-8 `--output-encoding` check uses `encode(...) =~ /\?/`; any literal `?` (titles, URL query strings) triggers a spurious "cannot be encoded … Recoding" warning and full-entry LaTeX recoding. (bbl.pm fixed this with a `\0` substitution char.) | **CONFIRMED** (`--tool --output-encoding=latin1`, `title={Why?}`) |
| B11 | bin/biber:294 vs bibtex.pm:56-66, 129-138 | `--output-fieldcase` is validated case-insensitively (`/i`) but compared with `eq 'upper'` etc.; `--output-fieldcase=Upper` crashes: "Can't use an undefined value as a subroutine reference at …bibtex.pm line 145", exit 25. | **CONFIRMED** |
| B12 | bblxml.pm:561, 576, 594 | Calls undefined `bblxml_norm` (should be `_bblxml_norm`): any entry with an annotation makes `--output-format=bblxml` die ("Undefined subroutine &Biber::Output::bblxml::bblxml_norm"), exit 102, truncated output. | **CONFIRMED** (`t3.bib`) |
| B13 | bblxml.pm:570-572, 586-588 | Item/part annotation loops call `get_annotated_items('item',$key,$f)` without the annotation name (always `default`) and `get_annotation('item',$key,$f,$c)` with the item count in the *name* slot → wrong/missing values. | suspected (masked by B12) |
| B14 | bblxml.pm:705-708, Section.pm:765 | `\keyalias` equivalents in bblxml come from `keys %citekey_alias` (random order); `.bbl` sorts them. | **CONFIRMED** (4 runs, 4 orders) |
| B15 | bbl.pm:731-746 (and bibtex.pm:389-404) | `--output-safechars` (and forced recode for non-UTF-8 encodings) is applied to the whole entry string incl. verbatim fields: `\verb http://example.org/caf\'{e}?a=1` in `urlraw` — verbatim content is no longer the URL. Also applies to keys. | **CONFIRMED** |
| B16 | bin/biber:415, base.pm:57-71 | Output `.bbl` is opened/truncated before processing; any `biber_error` (e.g. missing `.bib`) leaves a 0-byte `.bbl`, destroying the previous good one (no temp-file+rename). Possibly intentional, but undocumented. | **CONFIRMED** (`trunc.bcf`) |
| B17 | dot.pm:266 | `sort values $v->%*` sorts hashrefs by memory address → edge order in `.dot` (no `field` include) can vary run to run. | suspected |
| B18 | Entry.pm:1067 | `next if first {$_ eq $datefield} @removed_fields` compares DM *date field* names (`date`) with *datepart* names (`year`, …) — never true; derived date flags (circa/era/…) of a parent are copied to a child whose own date parts were kept. (Inheritance area.) | suspected |
| B19 | bblxml.pm:683-702 vs bbl.pm:693-707 | bblxml excludes a list from the first pass only if sortingnamekeytemplate=`global` and labelprefix=`''`, but the second pass appends all lists with the global sorting template → a global-sorted list with a labelprefix/other template would be emitted twice. (In `.bbl`, multiple global-sorted lists are all moved to the end; which one biblatex treats as "global" is then whichever is last in `.bcf` order.) | suspected |
| B20 | Utils.pm:683 | `normalise_string_bblxml` returns `''` for `"0"` → bblxml loses `0` values. | suspected |
| B21 | bbl.pm:335-337, 567-569 | `\true{more<list>}` handling `pop`s `others` from the entry's live list array — output has a side effect on entry data (second output pass/list sees a shortened list; harmless today because each entry string is built once). | note (porting hazard) |
| B22 | bbl.pm:681 (`foreach my $secnum (sort keys $data->{ENTRIES}->%*)`) | Sections are enumerated from `ENTRIES`, so a refsection with no found entries emits no `\refsection` block and therefore drops its `\keyalias`/`\missing` lines: `\nocite{nonexistent}` alone → `.bbl` has only header + `\endinput`. Same structure in bblxml.pm. | **CONFIRMED** (`bugs/miss.tex`) |

Docs vs code: the manual (doc/biber.tex:2736-2745) documents the `dates` class of
`--output-field-order` as "All date fields" — contradicted by B5.

---

## 8. Open questions

1. Bug-compat policy: reproduce B1/B3/B4/B5 exactly (needed to pass biber's own
   goldens, e.g. `full-bibtex_biber.bib`) or fix and normalise in the harness? Proposal:
   compat by default, `--fix-upstream-bugs` opt-in, harness aware of both.
2. B2: emit per-name options in a canonical order (e.g. `.bcf` optionscope order or
   sorted). Any order is "compatible" since biblatex parses them as key=value.
3. `.blg` fidelity: what do downstream tools (latexmk, arara, IDEs, Overleaf) parse?
   At least `Found BibTeX data source`, `WARN`, `ERROR`, `WARNINGS:`/`ERRORS:` lines.
   Do we keep `[ms] File:line>` prefixes with synthetic values?
4. UCA/CLDR version: sortinithash and all sorting depend on DUCET 13.0.0 + Unicode::Collate
   1.31 locale tailorings. Need an exact-compatible Rust collator (ICU4X uses newer
   CLDR/UCA — weights differ). Sorting researcher to confirm.
5. Should deprecated formats (`bblxml`, `dot`) be ported at all? They are broken in
   places (B12) and slated for removal upstream.
6. Windows: CRLF vs LF in `.bbl` from the packed Windows binary — needs a check on a
   Windows build if Windows parity matters.
7. `URI->as_string` exact escaping set (URI.pm `uri_escape` reserved/unreserved rules,
   IDN hosts?) must be pinned with tests on exotic URLs.
8. `--wraplines` + `Text::Wrap` (`unexpand` defaults differ between `bbl.pm` and
   `bibtex.pm` because `bibtex.pm` sets the *global* `$Text::Wrap::unexpand = 0` at load
   time only if that module is loaded) — worth goldens if anyone uses `--wraplines`.

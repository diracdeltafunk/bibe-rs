# 04 — Sorting, collation and sort-related labels (biber 2.22)

Status: v2 (complete). All "CONFIRMED" items were reproduced with the installed biber 2.22 binary.
Source: `/Users/ben/bibe-rs/vendor/biber` (HEAD ≈ v2.22). Oracle: installed `biber` 2.22 PAR binary.
Experiments: `/Users/ben/bibe-rs/scratch/sorting/t1..t9`, `perf/` (each has `t.tex`, `t.bib`, `.bcf`, `.bbl`, `.blg`).

---

## 1. Overview

Sorting in biber is a three-stage pipeline run per *datalist* (refcontext list) in
`Biber::process_lists` (`lib/Biber.pm:3085-3180`), *after* filtering:

1. `generate_sortdataschema` (`Biber.pm:3304`) – decides, per sort *set* (`\sort{...}`),
   whether the key type is `str` or `int` and whether it is descending.
2. `generate_sortinfo` → `Internals::_generatesortinfo` (`Internals.pm:1213`) – for every key in
   the list, builds an array of **one plain string/number per sort set** (the "sortobj"),
   a debug/joined "sortstring" (`join(',', ...)`), and the **sortinit** grapheme.
3. `sort_list` (`Biber.pm:4249`) – turns every string element into a Unicode::Collate sort key
   (binary string), every int element into a number, and sorts with
   `Sort::Key::multikeysorter` (stable, lexicographic over the per-set keys).

At output time `DataList.pm:2006-2013` emits `\field{sortinit}{X}` and
`\field{sortinithash}{md5_hex(viewSortKey(X))}` using a *separate* level‑1 collator.

Three different Unicode versions are in play in the shipped binary and all three matter:

| Component | Version in biber 2.22 binary | Used for |
|---|---|---|
| Unicode::Collate 1.31, DUCET (allkeys) | **UCA 13.0.0**, `UCA_Version` 43 | all collation keys, sortinithash |
| perl 5.42 core (`unicore/version`) + Unicode::Normalize 1.32 | **Unicode 16.0.0** | NFD/NFC, regex `\p{..}` classes in `normalise_string*`, nosort regexes |
| Unicode::LineBreak 2019.001 / sombok 2.4.0 (Unicode::GCString 2013.10) | **Unicode 8.0.0** grapheme rules | sortinit (first grapheme), `substring_*`/`pad_*` lengths |

Other bundled modules: Sort::Key 1.33, Lingua::Translit 0.29, Text::Roman (roman numerals),
Unicode::UCD `num()` (perl 5.42).

---

## 2. Exact sort algorithm

### 2.1 Building the per-entry sort object (`_generatesortinfo`, Internals.pm:1213-1272)

```
globals: BIBER_SORT_FINAL = '', BIBER_SUPPRESS_FINAL = 1
for each sortset S in template.spec (ordered by bcf `order`):
    s = _sortset(S)          # first non-false value among S's sortitems, else ''
    if s eq 'BIBERZERO': s = 0, szero = 1
    if BIBER_SORT_FINAL is set:
        push (SUPPRESS ? '' : BIBER_SORT_FINAL); SUPPRESS = 0
    else push s
sortstring = join(',', sortobj)
```

`_sortset` (Internals.pm:1275): iterates sortitems in order; dispatches each; the **first truthy
result** (Perl truthiness: `''` and `'0'` are false!) wins. If the sortset has `final="1"` and a
value is found, the value is stashed in `$BIBER_SORT_FINAL` and `''` is returned.

Consequences of `final` (nyt etc. use `\sort[final]{\field{sortkey}}`):
* the final set's own column is `''` for every entry;
* every later column of an entry that has the final value contains the final string
  (verbatim, also in `int` columns where it is then non-numeric → `2000000000`, see 2.3);
* later columns are still *computed* (wasted work) and discarded.
  Pinned by `t/sorting.t` `$sk1 = 'mm,,AATESTKEY,AATESTKEY,AATESTKEY,AATESTKEY'`.

Real zeros: `_process_sort_attributes` returns the sentinel `'BIBERZERO'` when its input is
exactly `'0'`, so that `_sortset`'s truthiness test accepts it; it is mapped back to numeric `0`.
**But** most dispatchers test `if (my $field = $be->get_field(...))` first, so a field whose value
is `0` (year 0, volume 0, `date={0000}`) is treated as *missing* (see bugs B5).

### 2.2 Per-item dispatch (`_dispatch_sorting`, Internals.pm:1167)

Order of checks:
1. `sortexclusion` for the entry's type contains the item → `''`.
2. `sortexclusion` for `*` contains it → `''` unless `sortinclusion` for the entry's type contains it.
3. `literal="1"` attribute → `_sort_string` (the literal text, through `_process_sort_attributes`).
4. Internal table (Internals.pm:1107): `editoratype|editorbtype|editorctype`, `citeorder`,
   `citecount`, `intciteorder`, `labelalpha`, `labelname`, `labeltitle`, `labelyear|labelmonth|labelday`,
   `presort`, `sortname`, `entrytype`, `entrykey`.
5. Data-model field, by `fieldtype`/`datatype` (Internals.pm:1128-1164):
   * list+name → `_sort_name`; verbatim/uri field → `_sort_verbatim`; field+literal or field+key →
     `_sort_literal`; field+integer or field+datepart → `_sort_integer`; list+literal/key →
     `_sort_list`; list+verbatim/uri → `_sort_list_verbatim`. Anything else (e.g. `field`+`date`,
     `range`, `code`) → falls off the end → `undef` → **"not a sortable field" warning** per entry.
6. Unknown name → `biber_warn("Field '$f' in sorting template is not a sortable field")` per entry, `undef`.

Per-type string pipelines (all return `''` when the field is absent):

| Dispatcher | Pipeline |
|---|---|
| `_sort_literal` (title, sortkey, sorttitle, …) | `normalise_string_sort` → `_process_sort_attributes` → `_translit` |
| `_sort_verbatim` (url, verba…) | `normalise_string_sort` → attrs (no translit) |
| `_sort_list` (publisher, location…) | `_liststring` → attrs → translit |
| `_sort_list_verbatim` | `_liststring(verbatim)` (only `strip_nosort`, **no brace stripping**) → attrs |
| `_sort_name` (author…) | `use<name>` option check → `_namestring` → attrs → translit |
| `_sort_sortname` | only if *any* `use<name>` option is true → `_namestring('sortname')` → attrs → translit |
| `_sort_integer` | roman→int (`Text::Roman::isroman(NFKD(v))` unless `noroman`) → `Unicode::UCD::num(v) // v` → attrs |
| `_sort_editort` | only if `useeditor` → attrs → translit |
| `_sort_citeorder` | see 2.4 |
| `_sort_labelname/labeltitle/labeldate` | redirect to the underlying field's dispatcher (attrs applied there) |
| `_sort_labelalpha` | `sortlabelalpha` entry field (`// ''`) → attrs |
| `_sort_presort` | `presort` option (entry > type > global; default `mm`) → attrs |

`normalise_string_sort` (Utils.pm:659): `strip_nosort` → `s/([^\\])~/$1 /g` →
`s/\\[A-Za-z]+//g` → `s/[{}]+//g` → trim → collapse `\s+` to one space. Note: punctuation is
*kept* (it matters because `variable => 'non-ignorable'`).

`_process_sort_attributes` (Internals.pm:1547): `'0'` → `'BIBERZERO'` (no padding!), falsy →
unchanged; **substring first**: width default 4, side default left (right → offset `-width`),
using `Unicode::GCString->substr` (grapheme clusters, sombok/Unicode 8); **then padding**: width
default 4, side default left, char default `'0'`; pad count = `width - GCString->length` (graphemes).
Transliteration (if any) is applied *after* substring/padding.

### 2.3 Names (`_namestring`, Internals.pm:1589)

* Template chosen: list `sortingnamekeytemplatename` → entry option → name-list option →
  per-name option (per-name override **leaks** to subsequent names, see B9).
* Visibility: `visible_sort` (maxsortnames/minsortnames), or `visible_cite` if the template has
  `visibility="cite"`.
* For each visible name, for each keypart, for each part: namepart (optionally `use=0/1`
  conditional on `use<part>` option; `useprefix` is name/namelist scoped) or literal.
  A namepart is `normalise_string_sort(part)` (or of the joined initials if `inits`), then
  **padded with spaces** to the section-wide maximum length:
  `NFD(sprintf("%-*s", $section->get_np_length($part), NFC($nps)))`.
  * `get_np_length` = max **code-point length of the raw NFD namepart** (before nosort/sort
    normalisation; braces/ties not yet removed) over **every name parsed in the section**
    (`Input/file/bibtex.pm:1847`, `biblatexml.pm:1323`), not just list members.
  * the padding is computed on the NFC form, so e.g. `Ångström` (NFD len 10) is padded to 10
    in NFC (= 8 + 2 spaces) and then re-NFD'd to 12 code points. Observed:
    `"A\x{30A}ngstro\x{308}m  Bob "` (t1).
  * a keypart string is appended only if truthy (`"0"` dropped).
* If not `nosortothers` and the list was truncated: append `"\x{10FFFD}"` (sorts after any
  real letter: implicit primary FBE1).
* Spaces are non-ignorable (primary 0x0209, the lowest non-ignorable), which is what makes the
  fixed-width padding produce "field-wise" comparison.

`_liststring` (Internals.pm:1722): truncation by maxitems/minitems; items joined with `'!'`
(non-ignorable, low primary); trailing whitespace stripped; `"\x{10FFFD}"` appended if truncated.

### 2.4 Cite order (`_sort_citeorder`, Internals.pm:1304)

* `allkeys` section (`\nocite{*}`): `biborder = keyorder_max + index_in_orig_bib_order + 1`;
  explicitly cited keys use their `order` attribute unless the `*` was cited *before* them
  (then biborder). `$ko || $biborder`.
* otherwise `$ko || ''` — keys never cited (crossref/xdata-pulled, related…) get `''` → int
  fallback 2000000000 → last.
* `intciteorder` = order inside one `\cite{a,b}`; `citecount` = number of citations.

### 2.5 Schema and keys (`generate_sortdataschema`, Biber.pm:3304; `sort_list`, Biber.pm:4249)

* Column type = `int` iff the **first** sortitem of the set is one of `citeorder`,
  `intciteorder`, `citecount` or a DM field of datatype `integer`/`datepart`
  (`DataModel.pm:335`); else `str`. Later items in the set are assumed same type (gotcha:
  `\sort{\field{year}\field{title}}` makes the title fallback a non-number → all tie).
* `sort_direction="descending"` → `-int`/`-str`.
* Key extraction (`$extract`, Biber.pm:4376):
  * `int` column: `looks_like_number($v) ? $v : 2000000000`. So missing/non-numeric values sort
    **after** all real numbers ascending (and *first* descending). Sort::Key `int` compares
    `SvIV` (so `1.5`→1, `1e3`→1000, `Inf`→IV_MAX, `nan`→0 — edge cases).
  * `str` column: the eval'd string
    `"$collateobj->change(...)->getSortKey(q{$value})"`; cached in a per-call hash keyed by that
    exact string (`$cache->{$a} ||= eval $a`). Errors from eval are **not checked** → `undef` key (B1).
* Comparison: Sort::Key `multikeysorter`: lexicographic over columns; `str` columns are compared
  with Perl `cmp` on the binary UCA keys (bytewise, keys are big-endian 16-bit weights), `int`
  numerically; `-` negates the comparison.
* **Stability**: the sort is stable (Perl mergesort under Sort::Key). Ties keep the input
  order, which is `$section->get_citekeys` (citation order; bib-file order in `allkeys`
  mode). Verified in t2: five identical entries come out in `\cite` order for both `nyt` and
  `ydnt` (descending does not reverse ties); with `\nocite{*}` they come out in .bib order even
  though `\cite{k4}` preceded it.

### 2.6 Collator configuration per column (Biber.pm:4283-4368)

```
thislocale = biber --sortlocale  ||  locale2bcp47(template.locale || global blx sortlocale)
collopts   = getoption('collate_options')   # default {level=>4, variable=>'non-ignorable',
                                            #          normalization=>'prenormalized'}
if !sortcase: collopts.level = 2            # mutates the global option hash
collopts.upper_before_lower = sortupper     # always overrides -c upper_before_lower
Collator   = Biber::UCollate->new(thislocale, %collopts)
for each sortset:
   sl = locale2bcp47(sortset.locale)
   cobj = (sl defined && sl ne thislocale) ? "Biber::UCollate->new('sl', %collopts)"   # NEW object per uncached key!
                                            : '$Collator'
   if sortset.sortcase defined && != global sortcase: level => (sortcase ? 4 : 2)
   if sortset.sortupper defined && != global sortupper: upper_before_lower => sortupper
   else: ->change(level => collopts.level, upper_before_lower => collopts.upper_before_lower)
```

`Biber::UCollate->new` (UCollate.pm:28): if `collopts{locale}` is absent use `thislocale`
(and drop `table`); construct `Unicode::Collate::Locale->new(locale => L)` **with no other
options**, then `->change(%collopts)` (alternate→variable). `change()` croaks for
`entry, table, ignoreChar, ignoreName, undefChar, undefName, rewrite, suppress, maxlength,
contraction,…` and *silently ignores unknown keys*.

Effective precedence (verified, t3): `-c locale=X` > per-sortset `locale` > `--sortlocale` >
template `locale` > global `sortlocale`. The manual (biber.tex:1923-1927) says `--sortlocale`
beats per-sortset locale — **it does not** (B4).

Effective default UCA options (en-US): level 4 (but see 4.3 — level 4 with non-ignorable has
no quaternary), `variable => non-ignorable`, `normalization => prenormalized` (no NFD inside
U::C; input must already be NFD), `upper_before_lower => 1`, UCA_Version 43, no
backwards, no hangul_terminator, `overrideCJK`/`overrideHangul` false (→ DUCET implicit weights).
Locale files may set `upper_before_lower`, `backwards`, `variable` (th: shifted), `suppress`,
`overrideCJK` — but biber's `change()` then re-forces level, variable, normalization and
upper_before_lower, so e.g. **Thai's `variable=>shifted` and Danish/Maltese/Church Slavonic
`upper_before_lower=>1` are overridden** by biber's values (logged as INFO "Overriding locale…").
`backwards => 2` from `fr_CA`/`cu` is kept (biber doesn't set it).

---

## 3. Feature inventory

(difficulty: L/M/H port effort; risk: correctness risk if done naively)

| Feature | Where | Behaviour / edge cases | Diff. | Risk |
|---|---|---|---|---|
| Sorting templates from bcf | `_parse_sort` Biber.pm:4995 | sorts and items ordered by numeric `order`; attrs `substring_side/width`, `pad_width/side/char`, `literal`; set attrs `final, sort_direction, sortcase, sortupper, locale`. Empty sorts dropped. Template locale = `locale2bcp47(template locale || global sortlocale)` | L | L |
| Template fallback | Biber.pm:3311 | unknown template name → global `sortingtemplatename` | L | L |
| presort | `process_presort` Biber.pm:3066 (entry field → per-entry option), `_sort_presort` | default `mm`; type/entry scoped | L | L |
| final | Internals.pm:1236-1249, 1284 | see 2.1 | M | M |
| literal | `_sort_string` | literal "0" → BIBERZERO → 0 | L | L |
| substring / padding | `_process_sort_attributes` | grapheme based (sombok, Unicode 8); substring then pad; pad not applied to "0" | M | M |
| descending | schema `-` | ties remain in input order | L | L |
| sortcase / sortupper (global & per set) | Config.pm:763 (true/false→1/0), Biber.pm:4295-4364 | level 2 vs 4; upper_before_lower swaps tertiary case weights | M | M |
| --sortlocale / sortlocale / per-set locale / -c locale | see 2.6 | babel names mapped by `%LOCALE_MAP` (Constants.pm:380); unmapped names (e.g. `chinese`, `korean`) passed raw → U::C `_locale` → `default` (no tailoring, INFO only) | M | H |
| --collate-options | bin/biber:43 (Getopt hash `key=value`), Config.pm:172, 563 | merged over defaults. Documented `"k => v"` syntax is broken (B3) | L | M |
| --fastsort | Biber.pm:4287 | no-op, warns "no longer required/supported" | L | L |
| nosort (bcf `\DeclareNosort`, config) | `strip_nosort` Utils.pm:559, compiled in `preprocess_options` Biber.pm:4433 | per field name or `DATAFIELD_SETS` set; field-specific *and* set regexes all applied (`s/$re//gxms` — note **/x**: whitespace in user regexes is ignored!) | M (Perl regex → Rust regex) | M |
| noinit | Utils.pm:538 | affects name *initials* only, not sortinit | – | – |
| sortexclusion / sortinclusion | `_dispatch_sorting` | per entry type | L | L |
| citeorder / intciteorder / citecount | 2.4 | int columns | M | M |
| int fields (volume, sortyear, dateparts) | `_sort_integer` | roman numerals (`isroman(NFKD)`: `C`,`D`,`L`,`I`… volumes become numbers), `Unicode::UCD::num` (non-ASCII digits, `½`→0.5 → IV 0), non-numeric → 2e9 | M | M |
| dates | via `labelyear/month/day` and datepart fields | negative years sort as negative ints (−100 < −50 < 50, t6); `2000~` same as 2000; ranges use start; month/day missing → 2e9 so `2000` sorts *after* `2000-04` and `2000-05-03` | M | M |
| sortname / sortkey / sorttitle / sortyear | DM fields | sortname only if any `use<name>` is on | L | L |
| name keys | `_namestring` 2.3 | padding to section max NFD length; trunc char U+10FFFD | H | H |
| list keys | `_liststring` | `!` separator | L | M |
| translit | `_translit` Internals.pm:1767, Utils.pm:1797-1840 | first matching rule wins; langids compared with `fc`; target `*`, field or set; IAST→Devanagari, Russian→ALA-LC, Russian→BGN/PCGN; output is **NFC and not re-NFD'd** before `prenormalized` collation | M | M |
| sortinit | Internals.pm:1261-1270 | see 3.1 | M | H |
| sortinithash | DataList.pm:2011 | `md5_hex(viewSortKey(sortinit))` level-1 collator, *template* locale | M | H |
| sortdebug | Output/bbl.pm:750 | emits sortstring into .bbl as comment | L | L |
| tool mode | Biber.pm:282-321 | template `tool`, sortlocale `en_US` | L | L |

### 3.1 sortinit / sortinithash (exact)

```
ss = join(',', sortobj)                     # e.g. "mm,,Zebra     Anna,2001,Alpha,0"
if ss or szero:
   pre = presort option for entry
   ss =~ s/\A$pre,+//                       # $pre interpolated AS A REGEX (B2)
   init = GCString(normalise_string(ss))->substr(0,1)
normalise_string: s/([^\\])~/$1 /g; s/\\[A-Za-z]+//g; s/[\p{P}\p{S}\p{C}]+//g; trim; collapse ws
```
* Separators `,` and the U+10FFFD trunc char (Co) and any leading punctuation/symbols are
  removed by `\p{P}\p{S}\p{C}`; so `$100` → sortinit `1`; an empty name column then a title
  → first letter of the title.
* Not upper-cased: `abel, carl` → sortinit `c`; `É` stays `É` (NFD `E\x{301}` grapheme, NFC
  on output). sombok grapheme rules (Unicode 8) decide what "first character" is.
* If the remainder is empty, sortinit is `''` and still output (`\field{sortinit}{}`), hash
  `md5("[| | |]") = 495dc9894017a8b12cafa9c619d10c0c`.
* Hash collator: `Unicode::Collate::Locale->new(locale => template->{locale}, level => 1)`
  created in `parse_ctrlfile` (Biber.pm:1076, 1111, 1211). U::C defaults otherwise:
  **variable = shifted, normalization = NFD**, upper_before_lower irrelevant. It ignores
  `--sortlocale`, `-c locale` and per-sortset locales (B6).
* `viewSortKey` text format at level 1 is `"[" . join(' ', map {sprintf '%04X'} primaries) . " | | |]"`,
  e.g. `A`→`[1FA2 | | |]`, `Æ`→`[1FA2 2007 | | |]`, `ß`→`[21D2 21D2 | | |]`, `中`→`[FB40 CE2D | | |]`,
  sv `Ä`→`[22FC | | |]`. Verified hashes (identical to biber output): `A`/`Å`/`Ä`(en)
  2f401846e2029bad6b3ecc16d50031e2, `Z` 96892c0b0a36bb8557c40c49813d48b3, `É`
  8da8a182d344d5b9047633dfc0cc9131, `T` 9af77f0292593c26bde9a56e688eaee9, `c`
  4d103a86280481745c9c897c925753c0, sv `Ä` db0c88eea8d3c2b513a2fee764f1ad6b.
  Generator: `scratch/sorting/vsk.pl` (system perl U::C 1.29, same DUCET 13.0.0).

---

## 4. Collation details & versions

### 4.1 Bundled versions (from `biber --cache` →
`/var/folders/.../cache-bcf1e1e0.../inc/lib`)

* `Unicode/Collate.pm` **1.31**, `UCA_Version` 43, `Base_Unicode_Version` 13.0.0; `allkeys.txt`
  `@version 13.0.0` (DUCET 13.0.0, 33 096 lines). The XS bundle has DUCET compiled in
  (`__useXS`/`_fetch_simple`); `allkeys.txt` is not read at runtime unless `table` is given.
* `Unicode/Collate/Locale.pm` 1.31 with 87 tailoring files `Locale/*.pl`.
* `Unicode::Collate::CJK::{Big5,GB2312,JISX0208,Korean,Pinyin,Stroke,Zhuyin}` are packed at a
  **wrong path** `Unicode/Collate/CJK;lib/Unicode/Collate/CJK/*.pm` → the binary dies for any
  locale whose .pl needs them (ja, ko, zh, zh_* variants) (B7).
* macOS system perl 5.34's Unicode::Collate 1.29 is behaviourally identical (diff of .pm is
  EBCDIC-only; Locale/*.pl differ only in the `locale_version` line; same DUCET) → usable as a
  local oracle for keys and sortinithash.

### 4.2 Tailoring file format (`Locale/<name>.pl`)

Each is a Perl hashref: `locale_version`, optional `upper_before_lower`, `backwards`,
`variable`/`alternate`, `suppress => [codepoints]`, `overrideCJK => \&Unicode::Collate::CJK::X::weightX`,
and `entry => <<'ENTRY'` — lines in **allkeys.txt syntax with explicit DUCET-13 weights**
(`00E4 ; [.22FC.0020.0002] # …`, contractions as multi-codepoint keys, e.g. `0061 0308`).
They are *not* CLDR rule strings: they're pre-resolved weight tables generated by the module
author for DUCET 13.0.0. This makes them trivially loadable by a custom Rust UCA engine
(parse with the same `parseEntry` grammar). CJK overrides are code (radical/stroke/pinyin
tables in the CJK modules) that compute primary weights for unified ideographs.

Locale selection (`_locale`, Locale.pm:45): lowercase, `[-_ .]`→`_`, alias variant names
(`phone→phonebook`, `trad→traditional`, …); try `lang_Script_REGION_variant` … `lang`; fall back
to `default` (= pure DUCET). Available tailorings include no `en`, `de`, `fr`, `it`, `nl`, `pt`
(they are DUCET); `de__phonebook`, `de_AT_phonebook`, `es__traditional`, `fr_CA`, `sv__reformed`,
etc. map to short file names.

### 4.3 Sort key layout (what must be bit-compatible)

`getSortKey` = XS `mk_SortKey`: level 1 weights, `0000`, level 2, `0000`, level 3, `0000`,
level 4, all as big-endian `n` (16-bit). With `variable => non-ignorable` **no quaternary
weights are generated**, so level 4 behaves as level 3 (plus an extra trailing `LEVEL_SEP`
because `UCA_Version >= 26 && level == 4`; harmless). `upper_before_lower` remaps tertiary
weights (visible in `viewSortKey`: `A`→`0002`, lower→`0008`, even space/hyphen get `0008`).
Level 2 keys stop after secondaries. Implicit weights: `FB40/FB80/FBC0 + (cp >> 15)` /
`(cp & 0x7FFF) | 0x8000` per UCA 13 (`_derivCE_43`), with UCA-13 ideograph and Tangut/Nushu/
Khitan ranges; code points unassigned **in Unicode 13** get FBC0-family implicit weights even
if perl's Unicode 16 NFD knows them.

Because comparison is bytewise on these keys and ties are stable, **any Rust implementation
that produces the same key bytes (or the same total preorder) gives identical output**.

---


## 5. Rust collation strategy (recommendation)

### 5.1 Requirements distilled

To be output-identical we need, per column, the same *total preorder* as
`Unicode::Collate::Locale(1.31)->change(level, variable=>non-ignorable,
normalization=>prenormalized, upper_before_lower)->getSortKey`, on NFD input produced by
perl-5.42 (Unicode 16) normalisation, with DUCET **13.0.0** and the U::C 1.31 tailoring tables;
plus exact `viewSortKey` text at level 1 / `variable=>shifted` / NFD for `sortinithash`
(md5 of the text — any weight difference changes the hash). Also needed for `-c` users:
`backwards`, `variable` (shifted / blanked / shift-trimmed), `level` 1–4, `katakana_before_hiragana`,
`ignore_level2`, `hangul_terminator`, `UCA_Version` (older versions — can be "unsupported, warn").

### 5.2 Options evaluated

**(a) ICU4X `icu_collator`.** CLDR root collation is DUCET *plus* the CLDR root modifications
(UTS #35 Part 5 §3: special U+FFFE/U+FFFF (and U+FFFD) weights, some symbol/currency group
reorderings, etc.), built on a much newer Unicode (15.1/16) than DUCET 13 — so newly assigned
characters get real weights where U::C gives implicit ones, and many existing weights moved
between DUCET 13 and 16. Locale tailorings come from current CLDR, whereas U::C 1.31's tables
are a frozen ~CLDR-36-era conversion (and Biber maps e.g. `de` to plain DUCET; ICU `de` is
also root, but `sv`, `da`, `es`, `vi`, `ja`, `zh` etc. differ in detail). `caseFirst=upper` in ICU
is not the same algorithm as U::C's tertiary-weight swap (differs for compatibility variants,
e.g. circled/fullwidth letters). No way to inject custom weight tables. Sort-key API is
limited/new. Verdict: **not compatible**; acceptable only as an opt-in "modern collation" mode.

**(b) `feruca`.** Pure-Rust UCA (DUCET or CLDR-root tables, current Unicode), options are
essentially `shifting: bool` + `tiebreak`; no level selection, no `upper_before_lower`, no
`backwards`, no tailoring loader beyond a couple of built-in Arabic-script tailorings; tables are
baked in for the current Unicode version. Verdict: **not compatible as-is**; its
contraction / discontiguous-match / implicit-weight code is a reasonable reference to fork.

**(c) Own UCA engine that loads the exact U::C 1.31 data.** Recommended.

### 5.3 Recommendation: (c) — a faithful port of `Unicode::Collate` 1.31

* **Data (build time):** DUCET 13.0.0 `allkeys.txt` (from the PAR cache or the CPAN
  Unicode-Collate-1.31 tarball — identical to system-perl 5.34's copy) → compact table
  (codepoint/contraction → CE list). The 87 `Locale/*.pl` files: each is a Perl hash literal whose
  `entry` heredoc is plain allkeys syntax, plus a handful of scalar options (`upper_before_lower`,
  `backwards`, `variable`, `suppress`, `overrideCJK`). Extract with a tiny Perl script once (or a
  regex parser; the format is regular) into JSON/bincode. The 7 `Unicode::Collate::CJK::*`
  modules are big code-point→weight tables (JIS X 0208 order, Korean, Pinyin, Stroke, Zhuyin,
  Big5, GB2312) plus a small weight-packing function — also extractable as data.
  Licensing: DUCET under the Unicode licence; U::C tables under Perl's Artistic/GPL — check.
* **Algorithm (runtime), mirroring `Collate.pm`/`Collate.xs`:** `splitEnt` (longest contraction
  match incl. discontiguous matching using **canonical combining classes from perl's Unicode 16
  data**, since `getCombinClass` comes from Unicode::Normalize 1.32), `suppress` handling,
  Hangul syllable decomposition/contraction rules, `_derivCE_43` implicit weights with the
  UCA-13 ideograph ranges, `varCE` for the four variable modes, `mk_SortKey` (level separators,
  `backwards` flag, `upper_before_lower` tertiary remap, `katakana_before_hiragana`,
  `ignore_level2`), `identical`, `highestFFFF/minimalFFFE`, `getSortKey`'s trailing
  `LEVEL_SEP` when `UCA_Version>=26 && level==4`, and `visualizeSortKey`/`viewSortKey` text
  format for sortinithash. Estimated ~1.5–2.5 kLOC + data generator. Out-of-range code points
  (> 0x10FFFF) → U+FFFD as in getWt.
* **Normalisation:** keep Biber's own NFD (Unicode 16; `unicode-normalization` crate is
  currently Unicode 16 — pin the version) and pass strings to the collator unnormalised
  ("prenormalized") — do **not** let the collator normalise, so that NFC output from
  transliteration is collated exactly as U::C does.
* **Grapheme clusters (sombok, Unicode 8.0):** `unicode-segmentation` implements current
  UAX #29 (emoji ZWJ, GB9c Indic conjuncts, etc.). For bit-compatibility of sortinit and of
  substring/pad widths, implement legacy-8.0 extended grapheme cluster rules from Unicode 8
  `GraphemeBreakProperty.txt` (small; ~150 LOC) — or accept documented divergence for exotic
  scripts/emoji. Needs a sombok-specific check (see open questions).

### 5.4 Testing equivalence (oracle strategy)

* **Collation keys:** system perl `/usr/bin/perl` (5.34) has Unicode::Collate 1.29 with the
  same DUCET 13.0.0 and the same tailorings (only `locale_version` differs) → a local oracle
  without installing anything. Harness: a Perl script reads lines (hex code points), prints
  `unpack('H*', getSortKey)` and `viewSortKey` for a matrix of
  {locale ∈ default + 87 tailorings} × {level 1,2,3,4} × {variable non-ignorable, shifted} ×
  {upper_before_lower 0,1} × {backwards none, 2}. The Rust side prints the same; diff.
  Caveat: system perl's Unicode::Normalize is Unicode 13 (CCC data) — for strings containing
  marks first assigned after 13.0 the discontiguous-contraction behaviour may differ from the
  biber binary; restrict that subset to end-to-end tests with the real binary.
  Corpora: (1) every code point 0..0x10FFFF as a 1-char string (≈1.1 M lines; the 4-tuple of
  options multiplies — run defaults exhaustively, others on a sample); (2) every
  base-letter × combining-mark pair/triple from allkeys + tailoring contraction prefixes
  (contractions are the classic bug source); (3) every contraction key in every tailoring
  plus a following combining mark (discontiguous); (4) Hangul syllables/jamo sequences;
  (5) random NFD strings mixing scripts, spaces, `!`, U+10FFFD, digits; (6) real bib data
  (names/titles from CTAN/TeX Live bib files). Compare full key bytes, not just ordering.
* **sortinithash:** for each of the above single graphemes, compare md5(viewSortKey) at level 1.
* **End-to-end:** generate `.bcf` files with pdflatex once (cache them), run real `biber`
  and the Rust port, diff `.bbl` (order, `sortinit`, `sortinithash`). Use `--debug` blg lines
  `Sorting object for key ...` to diff the pre-collation sort objects (padding, final, etc.).
  Include: every predefined biblatex template (nty, nyt, nyvt, anyt, anyvt, ynt, ydnt, none,
  debug), per-set locale/sortcase/sortupper/descending, translit, nosort, sortexclusion,
  `\nocite{*}` + explicit cites (tie order), names with prefixes/useprefix/truncation.
* Existing unit tests that pin behaviour: `t/sorting.t` (sortstring formats, padding,
  substring, final, lists, dates, name keys, nosort, useprefix), `sort-uc.t` (sv tailoring,
  descending, per-field upper_before_lower, per-field sortcase), `sort-case.t`,
  `sort-complex.t`, `sort-order.t` (citeorder/allkeys interplay), `sort-names.t`, `translit.t`
  (IAST→Devanagari ordering), `datalists.t` (multiple lists). Port their fixtures
  (`t/tdata/*.bcf/.bib`) as golden tests.

### 5.5 Performance

* Measured (scratch/sorting/perf, 5 000 random entries, 1–4 authors each, `\nocite{*}`):
  whole run ≈25 s; the `sort_list`+context-data phase ≈2.1 s with `nyt`; with a template that
  uses a **per-set `locale=swedish`** it is ≈40 s, because `sort_list` builds the collator
  *string* `Biber::UCollate->new('sv-SE',…)` and `eval`s it for every uncached key —
  constructing a new `Unicode::Collate::Locale` (re-parsing the tailoring heredoc) each time.
* Other costs in Perl: one string-`eval` (Perl compile) per uncached key; `->change()` +
  `checkCollator` before every key; Lingua::Translit object re-created per call
  (`get_transliterator`); sort data for columns after a `final` value computed and discarded;
  `_namestring` per column per entry.
* Rust plan: build each distinct collator once per list; memoise keys by
  `(collator_id, string)`; compute sort objects in parallel (rayon) — they are pure functions of
  entry + options; convert each str column to dense ranks (sort unique keys once), then do a
  single stable `sort_by` on `(rank0, rank1, …)` tuples. Expected well under 50 ms for 5 000
  entries for this phase.

## 6. Test oracle notes (practical)

* `biber --cache` path for this machine:
  `/var/folders/z1/pn7vdxgs7796pkgn52g__pyr0000gn/T/par-62656e/cache-bcf1e1e0a2c546e2ccb004c42af5f8631c12c6fd/inc/lib`
  (Biber sources there are the ones actually executed; error messages cite them).
* `biber --debug` prints `Sorting object for key 'k' -> [...]` (pre-collation values, with
  `\x{..}` escapes) and "Keys before/after sort" — ideal for diffing stage 2 independently of
  collation.
* `scratch/sorting/vsk.pl` reproduces sortinithash values with system perl.
* biblatex macro for transliteration is `\DeclareSortTranslit` (not "…Transliteration").
* Tie order depends on `$section->get_citekeys` (cite order, or .bib order with `\nocite{*}`),
  so end-to-end tests must preserve citation order semantics from the section module.

## 7. Suspected biber bugs

| # | Status | Location | Description / reproducer |
|---|---|---|---|
| B1 | **CONFIRMED** | Biber.pm:4392-4395 | Sort keys are produced by `eval "…->getSortKey(q{$sortfield})"`; values are spliced into Perl source. A trailing backslash / unbalanced `{}` / `\}` makes the eval fail silently → `undef` key (sorts first, "Use of uninitialized value … line 4409"); `\\` or `\{` are silently altered. Repro (t1): `title={The Foo\\Bar}` → `normalise_string_sort` leaves `The Foo\` → entry sorts before everything. Verbatim list fields (`_sort_list_verbatim`, no brace stripping) and literals can contain braces. Port must *not* replicate (or should it? decide: replicate ordering of undef-as-'' only if bit-compat required). |
| B2 | **CONFIRMED** | Internals.pm:1267 | `$ss =~ s/\A$pre$sorting_sep+//` interpolates the presort value as a regex. `presort={a(b}` → fatal "Unmatched ( in regex". Values with `.`/`+` etc. silently mis-strip. |
| B3 | **CONFIRMED** | bin/biber:43 vs doc/biber.tex:1974-2010 | Manual documents `--collate-options="variable => 'shifted'"`, `-c 'locale => "de_DE"'`; Getopt hash syntax is `key=value`, so the documented form creates key `"variable "` = `"> 'shifted'"`, silently ignored by `change()`, and **breaks the eval string for per-set-locale columns** (Bareword syntax errors → all keys undef → list unsorted). |
| B4 | **CONFIRMED** | Biber.pm:4283, 4333-4337 vs biber.tex:1923-1927 | Documented precedence says `--sortlocale` overrides the per-sortset `locale`; in code the per-set locale wins (`--sortlocale=en_US` still sorts the `locale=swedish` set with sv tailoring). Only `-c locale=…` overrides it. |
| B5 | **CONFIRMED** | Internals.pm:1342, 1472, 1487, 1676 (`if (my $x = get_field)`) | Values equal to `"0"` are treated as missing: `year={0}`, `date={0000}` (year 0 = 1 BCE) and `volume={0}` get `''` → int fallback 2000000000 → sort after all dated entries (t6: d12, d13, d17). The BIBERZERO machinery only rescues literals. |
| B6 | **CONFIRMED** | Biber.pm:1076/1111/1211, DataList.pm:2011 | sortinithash collator uses the template/global locale only; ignores `--sortlocale`, `-c locale`, per-set locales. `biber --sortlocale=sv_SE`: Ä sorts after Z but gets A's hash 2f40… (biblatex `sortlocale=swedish` gives db0c…). |
| B7 | **CONFIRMED (binary packaging)** | PAR bundle | `Unicode::Collate::CJK::*` packed under `Unicode/Collate/CJK;lib/…`; `sortlocale=japanese` (→ ja) dies: "Unicode/Collate/Locale/ja.pl can't be found" at Biber.pm:1076. Same for ko, zh, zh__pinyin, etc. Source distribution presumably fine. |
| B8 | **CONFIRMED** | UCollate.pm:64 / `-c` | `-c ignoreChar=…` (and `entry`, `suppress`, `rewrite`, `undefChar`…, all valid U::C options per the manual's "see perldoc Unicode::Collate") crash: "change of ignoreChar via change() is not allowed!", because options are applied via `change()` rather than `new()`. |
| B9 | **CONFIRMED** | Internals.pm:1640-1650 | Per-name `useprefix` (and per-name `sortingnamekeytemplatename`) assignments overwrite the loop variable and **leak to all following names** in the list. Repro (t8): `author={family=Alpha, given=Al, prefix=von, useprefix=true and family=Zulu, given=Zed, prefix=van}` → `"vonAlphaAl vanZulu Zed"` (van treated as useprefix=true); reversing the names gives `"Zulu ZedvanvonAlphaAl "`. |
| B10 | **CONFIRMED** | Biber.pm:3332 / DataModel.pm:337 | Column type uses only the *first* item's name: `labelyear`/`labelmonth`/`labelday` (internal redirects to dateparts) are `str` → collated lexicographically. `\sort{\field{labelyear}}` (authoryear style) orders −50, 1000, 2000, 80, 999 (t9) while `\field{year}` orders numerically. Conversely `\sort{\field{year}\field{title}}` makes the title fallback a non-number (all 2e9 ties). |
| B11 | suspected (reading) | Internals.pm:1548 | `_process_sort_attributes` returns `BIBERZERO` before applying padding/substring, so a `"0"` value (literal / presort / entrykey / labelalpha) is never padded. Also a `final` set whose value is `'BIBERZERO'` copies the sentinel string into later columns. |
| B12 | suspected (reading) | Biber.pm:4296-4300 | `sort_list` mutates the *global* `collate_options` hash (`level=2` when `sortcase` false, `upper_before_lower` always forced from `sortupper`): `-c upper_before_lower=…` is silently ignored, `-c level=3` is overridden by sortcase=false, and per-set `sortcase=true` forces level 4 regardless of `-c level`. |
| B13 | suspected (reading/log) | UCollate.pm:61-70 | Locale-mandated options are overridden without notice beyond INFO: Thai's `variable=>shifted` is replaced by non-ignorable; Danish/Maltese `upper_before_lower=>1` replaced by `sortupper`. Manual says biber "will warn" (it only logs INFO). |
| B14 | suspected (reading) | Biber.pm:4335 | Per-set-locale columns construct a fresh `Unicode::Collate::Locale` for every uncached key (perf: 5 000 entries +38 s, measured). Not a correctness bug. |
| B15 | observation | Constants.pm `%LOCALE_MAP` | babel names `chinese`, `korean` (and others not in the map) are passed raw and fall back to DUCET with only an INFO line. |
| B16 | suspected (reading) | Utils.pm:1797-1824 | `get_transliterator` warns on invalid pairs on *every* call (per entry per field) and builds a new Lingua::Translit object per call. |

Observed outside this area (for the relevant owners):
* Name parsing: `author={abel, carl}` → `prefix={abel carl}`, `given={carl}` (given duplicated into
  prefix; sort string `carlabel carl`) — t1.
* Legacy `year={-50}` is output as `\field{year}{50}` with no era/BCE marker while it sorts as −50 — t1/t6.
* List filtering (Biber.pm:3157-3163): set members are re-added if *any* set survived the
  filter (`first {$be->get_field('entryset')} @sets` never checks membership).

## 8. Open questions

1. Exact sombok 2.4.0 grapheme rules (legacy vs extended clusters; its Unicode 8 tables): need
   an oracle — only the biber binary has GCString. Craft titles with Indic/Hangul/emoji/regional
   indicators and read `sortinit` from `.bbl`.
2. Does the port replicate B1 (undef keys sort first) and B5 (zero = missing) for
   bit-compatibility, or fix them? Recommend a `--compat` flag defaulting to replicate.
3. Collate.xs `mk_SortKey` details (tertiary remap table for `upper_before_lower`, backwards
   handling with level separators) must be read from the CPAN source (not present locally — only
   the compiled bundle); behaviour can be pinned via the differential harness meanwhile.
4. Unicode-16 CCC (biber binary) vs Unicode-13 CCC (system perl oracle) for discontiguous
   contractions: identify affected code points (marks assigned 14.0–16.0) and test those
   end-to-end against the binary only.
5. Should the port fix B7 (support ja/ko/zh tailorings) — almost certainly yes; ensure the CJK
   tables are extracted.
6. `Unicode::UCD::num` and `Text::Roman::isroman` exact acceptance sets (case sensitivity,
   mixed scripts) need small oracles for `_sort_integer`.

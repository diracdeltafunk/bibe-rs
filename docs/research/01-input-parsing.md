# 01 — Data input and parsing (biber 2.22)

Scope: everything that turns data sources into `Biber::Entry` objects. This covers datasource location, encodings, the BibTeX parser (Text::BibTeX/btparse) and how biber drives it, LaTeX decoding, sourcemaps, per-datatype field handlers, names, dates, annotations, XDATA references at input, and biblatexml (summarised).

Source: `/Users/ben/bibe-rs/vendor/biber`. It is identical to the installed biber 2.22 except for the version string. I diffed the vendor tree against the PAR cache (`biber --cache`, `inc/lib/Biber`). The bundled **Text::BibTeX is version 0.91** (Perl 5.42), and its Perl wrapper source is readable in the PAR cache under `inc/lib/Text/BibTeX*.pm`. The C library `libbtparse.dylib` is binary only.

All experiments are in `/Users/ben/bibe-rs/scratch/input/`:
- `t.sh NAME [biber args] < bib` runs biber against a fixed `.bcf` (authoryear, `\nocite{*}`) and prints the `\refsection`.
- `tt.sh NAME [args] < bib` runs `biber --tool` and prints the re-emitted `.bib`.
- `smap/` holds the sourcemap experiments (`sm*.conf` with `--configfile`).
- Run directories are under `runs/`.

"CONFIRMED" below means I observed the behaviour by running biber 2.22.

---

## 1. Overview: the input pipeline

```
Biber::fetch_data (Biber.pm:4573)
  ├─ clrmacros? → Text::BibTeX::delete_all_macros
  ├─ (re)define month macros jan..dec → "1".."12" unless nostdmacros (Biber.pm:4591-4597)
  ├─ glob_data_file per datasource (Utils.pm:127)            [only if glob=true]
  └─ for each datasource (stops early once all cited keys found, unless allkeys) (Biber.pm:4622-4684)
       locate_data_file (Utils.pm:226) → Biber::Input::file::<datatype>::extract_entries
bibtex extract_entries (bibtex.pm:109)
  ├─ empty file → warn, skip;  slurp in encoding (fatal on bad bytes); no '@' anywhere → warn, skip
  ├─ select sourcemaps: all user-level, all style-level, FIRST driver-level (bibtex.pm:138-152)
  ├─ redirect STDERR to temp file (unless --debug/--trace) to capture btparse messages
  ├─ cache_data once per filename per run (bibtex.pm:1504)
  │    preprocess_file (1648): decode → NFD → strip BOM → \R→\n → write tmp "<md5(fn)>_<pid>.utf8"
  │    parse_decode (1705) = PASS 1: btparse-parse tmp file; for every regular entry re-serialise
  │         "@type{key,\n  f = {latex_decode(v)},\n…}"; preamble/comment/string re-emitted; write tmp again
  │    PASS 2: btparse-parse the re-serialised file; preamble/comment/macro bookkeeping; ids aliases;
  │         case-mismatch + duplicate-key checks; parse_ok; cache Text::BibTeX::Entry objects
  ├─ allkeys: create_entry for every cached key (hash order); add citekeys in original .bib order
  │  else: for each wanted key: cached entry → create_entry | citekey alias → create real entry | case-typo warn
  ├─ restore STDERR; translate btparse lines: "error:" → biber_error (FATAL unless --nodieonerror),
  │    "warning:" → biber_warn, "overriding existing definition of macro" ignored (bibtex.pm:304-329)
  └─ push preambles only on first use of the file (counts<2)
create_entry (bibtex.pm:356): clone T::B entry → apply sourcemaps → _create_entry (940):
     for each field in .bib order: 'options' processed first; field must be in data model (else dropped,
     warning only with --validate_datamodel); empty value skipped; dispatch handler by
     (fieldtype, format, datatype) or annotation-marker regex (1994-2004).
```

The **two-pass design** is the central architectural fact. Pass 1 exists only to run `latex_decode` on field values. It parses the file once, expands macros, decodes, and then writes a new BibTeX text that pass 2 parses again. Several bugs come from this round-trip (§6 B1, B2, B20). A Rust port should **parse once into an AST and decode values in memory**. It must then reproduce the observable artefacts of the round-trip deliberately, where we choose compatibility:
- Values are macro-expanded and brace-wrapped before decoding.
- Errors are reported from pass 1.
- A decoded value that contains an unbalanced `{` or `}` corrupts the second parse.

---

## 2. Feature inventory

Ratings: **Difficulty** (porting effort) and **Risk** (chance of silent output divergence) are each L/M/H.

### 2.1 Datasource discovery, remote sources, globbing, encodings

| Feature | Behaviour | Cite | Diff/Risk |
|---|---|---|---|
| Datasource list | Comes from `.bcf` `<bcf:datasource type datatype glob encoding>`, deduplicated per section on (type, datatype, name). In tool mode it comes from `@ARGV` with `input_format`/`input_encoding`. | Biber.pm:881-893, 282-296 | L/L |
| Encoding precedence | `datasource@encoding` from the .bcf **overrides** `--input-encoding`. The CLI value is only a default. CONFIRMED by adding `encoding="ascii"` to a .bcf: `--input-encoding=latin1` was ignored. | Biber.pm:888 | L/M |
| Decoding | `File::Slurper::read_text` with Perl Encode names (`UTF-8`, `latin1`, `cp1252`, …). Decoding is strict: one invalid byte is a fatal `biber_error` ("cannot be read in encoding"). CONFIRMED. `normalise_utf8` maps `utf-?8` (case-insensitive) to `UTF-8`. | bibtex.pm:127, 1672; Utils.pm:1066 | M/M |
| `ascii` | `preprocess_file` intends to read `ascii` as UTF-8, but that code is unreachable (see B14). | bibtex.pm:1667-1670 | L/L |
| BOM / newlines | Strips a leading U+FEFF and converts every `\R` to `\n` before btparse. | bibtex.pm:1677-1681 | L/L |
| Empty file / no `@` | Each gives a warning and the file is ignored. | bibtex.pm:120-133 | L/L |
| `locate_data_file` order | 1. Remote fetch, if the name starts with `http(s)://` or `ftp(s)://`. 2. If `input_directory` is set, `indir/name`; **else if** `output_directory` is set, `outdir/name`. 3. Absolute path. 4. The dir/out-dir candidate. 5. Relative to the cwd. 6. The directory of the control file. 7. `kpsewhich name` (if it is on PATH; strips a trailing `^M`). 8. Fatal "Cannot find". Each existence check also tries NFC and NFD variants of the path. | Utils.pm:226-384, 393-420 | L/L |
| Remote | LWP GET with `User-Agent: Mozilla/5.0` and header `Zotero-Allowed-Request: 1`. Honours `env_proxy`. Saved to a temp `biber_remote_data_source_XXXXX.bib` (always `.bib`, even for biblatexml) and cached per run in `%REMOTE_MAP`. CA bundle: `PERL_LWP_SSL_CA_FILE`/`_PATH`, then Mozilla::CA, then the Linux bundle paths. Options `ssl-nointernalca` and `ssl-noverify-host`. HTTP failure is fatal. Not tested here (no network experiments). | Utils.pm:231-305 | M/L |
| Glob | Only when `glob` is true (per datasource, or global `glob_datasources`), and never for remote sources. `globU` runs `glob(pattern)`, then also NFC and NFD variants of the pattern if they differ, and **concatenates the results with no dedup**. On Windows it uses DosGlob. `_bool_norm` treats any value containing `true` or `1` (case-insensitive) as true. | Utils.pm:112-162, 1986 | L/M |

### 2.2 BibTeX syntax as implemented by btparse (Text::BibTeX 0.91)

What follows was established by experiment, plus the Text::BibTeX POD and the message strings in `libbtparse.dylib`. **This is the largest external behaviour to replicate.**

1. **Top level.** Text outside entries is junk.
   - Non-whitespace junk produces a warning: `N characters of junk seen at toplevel`.
   - **`%` starts a comment that runs to end of line anywhere outside a string**: at top level, indented, after an entry on the same line, and *inside an entry between fields*. These are silently ignored. CONFIRMED (`runs/cmt3`, `runs/cmt4`). This differs from BibTeX 0.99.
2. **Entry.**
   - `@` + type, then `{…}` or `(…)`. A mismatched closer gives a warning: `entry started with "(", but ends with "}"`.
   - The type is matched case-insensitively and **lowercased**. Field names are **lowercased**. The **key is case-preserving**. Macro names are case-insensitive.
3. **Key.** A NAME token. It cannot contain whitespace, `(`, `"`, `'` and probably more. `@misc{my key,…}`, `@misc{k(p),…}` and `@misc{k'x,…}` all produce syntax errors. UTF-8 keys work: Text::BibTeX ≥0.46 passes the bytes through. **The exact NAME character set must be taken from btparse's grammar (bibtex.g / lex_auxiliary.c in the Text-BibTeX CPAN dist).** The dylib strings show the class `abcdefghijklmnopqrstuvwxyz0123456789:+/'.-`, which is probably the legal-key-character table.
4. **Regular entry body.**
   - `key , field = value , … [,]`. A trailing comma is OK.
   - **An entry with no fields, `@misc{k}`, is a syntax error** ("found "}", expected ","". CONFIRMED). BibTeX accepts it.
   - An empty key `@misc{, …}` is a syntax error.
5. **Values.**
   - Simple values are `{…}` with balanced braces (backslash does not escape braces), `"…"` (a `"` inside braces does not terminate it; braces must balance), bare digit strings, or macro names.
   - Simple values are joined with `#`.
   - Unbalanced braces give an error: `unbalanced braces: too many }'s`.
   - **Bare `12--15` is parsed as a macro name** (NAME tokens may contain `-`). It is undefined, so it gives a warning and an empty value. CONFIRMED.
6. **Post-processing ("maximal processing", BTO_FULL).**
   - Outer delimiters are removed.
   - Macros are expanded. An undefined macro gives the warning `undefined macro "x"` and expands to `""`.
   - Pieces are pasted.
   - Whitespace is **collapsed on the final pasted string**: every run of whitespace (including newlines) becomes one space, and leading/trailing space is trimmed. `"A  " # foo # " and " # BAR # {  Tail  }` became `A Foo Bar and baz Tail`. CONFIRMED.
7. **Duplicate field in one entry.** The **last value wins**, with no warning. CONFIRMED (`dupfield`). BibTeX keeps the first and warns.
8. **@string.**
   - Defines one or more macros, comma-separated.
   - The macro table is **global for the process**, so it persists across files and refsections unless `--clrmacros`.
   - Redefinition gives a warning, which biber suppresses.
   - Predefined months: Text::BibTeX defines `jan`→`January`… when loaded, and biber then redefines `jan..dec` → `"1".."12"` before reading any file (Biber.pm:4591) unless `--nostdmacros`.
   - Tool mode records `value → macro name` in `%RSTRINGS` (bibtex.pm:1535-1540).
9. **@preamble.** The value is collected and pushed to the global preamble list once per file per run.
10. **@comment.**
   - Must be delimited. **`@comment free text` is a fatal syntax error.** CONFIRMED. BibTeX accepts it.
   - Contents are kept only in tool mode without `--strip-comments`. JabRef `jabref-meta:` comments get their line breaks restored (Utils.pm:1586).
11. **Error recovery.**
    - On a syntax error btparse prints `file, line N, syntax error: found "x", expected … (skipping to next "@")`. It **keeps the fields parsed before the error**: in pass 1 the partial entry is re-serialised cleanly, so it survives into the output. It then resumes at the next `@` *after the error token*.
    - A value that ends up spanning the next entry (an unclosed brace) swallows that entry.
    - CONFIRMED (`runs/a3`, `runs/a4`): `bad1` kept `title`, and `good3` was lost after `bad2`.
    - Every btparse `error:` line becomes a **fatal** `biber_error` (exit 2) at the end of `extract_entries` unless `--nodieonerror`. Only `--debug` changes this (B15).
12. **Names (split_list / split_name).** See §2.7.
13. **SEGV handler.** `TBSIG` turns a libbtparse crash into a logdie (bibtex.pm:5, 95-98).

### 2.3 Entry caching, keys, duplicates, aliases

- **Per-run cache** `$cache->{data}{$filename}{$key}` is filled once per filename. A later refsection that uses the same file reuses the parsed `Text::BibTeX::Entry` objects. `create_entry` clones before mapping, so mutations do not leak between refsections (bibtex.pm:370-371, 930).
- **Duplicate keys.**
  - Within a section, across all of its datasources (`has_everykey`): warning `Duplicate entry key: 'k' in file 'f', skipping ...` and the **first one wins**.
  - With `--noskipduplicates` there is a warning but no skip, and the cache is then overwritten, so the **last one wins**.
  - CONFIRMED.
- **Case-mismatch keys.**
  - Datasource vs datasource: `Possible typo (case mismatch) between datasource keys`.
  - Cite vs datasource: a similar warning (bibtex.pm:294-296, 1601-1603).
  - Both entries are kept.
- **`ids` aliases.**
  - The field is split with `xsvsep`. Each alias is recorded in `citekey_aliases`.
  - Skipped with a warning when: the alias equals its own key ("BAD RECURSION!"); the alias is a real key; the alias already maps elsewhere (the first mapping wins).
  - If a later real entry has an alias's key, the alias is deleted.
  - Output: `\keyalias{alias}{key}`. CONFIRMED.
  - Note: `ids` is read with `$entry->get('ids')` **before** sourcemaps, so maps cannot create or alter aliases (bibtex.pm:1566).
- **allkeys ordering.** Entries are created in Perl hash order (`each`), but citekeys are added in **original .bib order** (`orig_key_order`). Entries removed by a map are pruned from that order.
- `rawdata` = `print_s` of the mapped entry is stored but never used (dead; do not port).
- **Fields not in the data model are silently dropped at input.** There is a warning only with `--validate_datamodel` (bibtex.pm:975-992).
- **Field order matters.** Handlers run in `.bib` field order. Example: `year` processed after `date` is skipped silently (bibtex.pm:1055-1066), and the warnings differ with order.

### 2.4 Sourcemaps (`\DeclareSourcemap`, `\DeclareStyleSourcemap`, driver maps, biber.conf `<sourcemap>`)

**Field/type aliases.** BibTeX legacy aliases (`address`→`location`, `school`→`institution`, `journal`→`journaltitle`, `@phdthesis`→`thesis`+`type=phdthesis`, `@conference`→`inproceedings`, …) are not hard-coded in biber. They arrive as the **driver-level** sourcemap that biblatex writes into the `.bcf` (`<bcf:maps datatype="bibtex" level="driver">`), and biber.conf carries a similar one for tool mode. They are pinned by `t/bibtex-aliases.t`. So the sourcemap engine must be exact for even basic documents to work.

**Order.**
- user maps (all `\DeclareSourcemap`, plus biber.conf maps, which are user level) → style maps (all) → the driver map (**only the first `level=driver` maps element**) (bibtex.pm:138-152).
- `map_overwrite`: per map, falling back to the `maps` element, default 0 (bibtex.pm:375, 824, 856).

**Map filters.**
- `refsection`: numeric equality.
- `per_type` / `per_nottype`: fold-case equality with the entry type.
- `per_datasource`: case-sensitive equality with the datasource name. For an absolute path (from kpsewhich), only the basename is compared.
- Cite bibtex.pm:381-410.

**`map_foreach`.** Loops the map's steps with `$MAPLOOP` set to each value. The value list is resolved in this order (bibtex.pm:420-432):
1. a datafield set name (`\DeclareDatafieldSet`);
2. else the value of that field in the entry, split on `\s*,\s*`;
3. else the literal attribute, split on `\s*,\s*`.

**Step semantics.** A single step may do several things. They are evaluated in this exact order (bibtex.pm:437-920):
1. `map_entry_null` → drop the entry (`return 0`). With a cited key this gives the warning "cited and found but not created".
2. `map_entry_new` (+ `map_entry_newtype`, required, else warning and skip step; `map_entry_nocite`) → a fresh entry. `$N` (single digit) in the key comes from earlier captures. The new key is removed from the wanted-keys list. It is added to citekeys if `nocite` is set or the section is allkeys.
3. `map_entry_clone` → clone the *current mapped state* of the entry under a new key. Same `$N`, nocite and allkeys rules.
4. `map_entrytarget` → the subsequent operations in this step apply to a new/cloned entry. If unknown, warning and skip step.
5. `map_type_source`/`map_type_target`. On mismatch: `final` → skip the rest of the map (`next MAP`), else skip the step. The type is fold-cased.
6. `map_notfield` → skip if the field exists.
7. `map_entrykey_citedornocited` / `_cited` / `_nocited` / `_allnocited` / `_starnocited` (see B6 for starnocited).
8. `map_field_source` must exist. The pseudo-field `entrykey` always exists. Captured: `$last_field`, `$last_fieldval`.
9. `map_matches`/`map_matchesi`: parallel comma lists of literal strings → replacements (exact or fold-case equality on the whole value). A count mismatch skips the step.
10. `map_match` / `map_matchi` / `map_notmatch` / `map_notmatchi`:
    - **With `map_replace`:** `ireplace` does a global `s///` on the value, and the replacement is evaluated as a Perl double-quoted string (see below). `entrykey` cannot be modified.
    - **Without replace:** `imatch` performs `m//g` in list context. `@imatches` = all captures from all matches (or all whole matches if there are no groups). For `notmatch` it is `(1)`. Before matching, unescaped `$N` (multi-digit) in the *pattern* is replaced by `$imatches[N-1]`, unescaped. No match: `final` → `next MAP`, else skip the step.
11. `map_field_target`:
    - Rename source → target, fold-cased.
    - If the target exists and overwrite is off, the step is skipped (no `final` handling).
    - The value is taken from the **original entry**, not `etarget` (B5).
12. `map_field_set`:
    - `map_null` → delete the field.
    - Else, if the field exists and overwrite is off → `final` ? `next MAP` : skip.
    - Value source is one of `map_origentrytype` | `map_origfieldval` | `map_origfield` | `map_field_value`. `$N` (single digit) is taken from `@imatches`.
    - `map_append` prefixes the old value. `map_appendstrict` sets `''` if the old value is empty.
    - Note `map_origfieldval` skips when the value is falsy (`"0"`).
- `$MAPLOOP`, `$MAPUNIQ` (`Data::Uniqid::suniqid`, random) and `$MAPUNIQVAL` are substituted in every attribute via `maploopreplace`, **but only inside a `map_foreach` map** (Utils.pm:1778, B7). `$MAPUNIQVAL` is a package global that persists across entries and maps (Utils.pm:1775-1786).
- The step hash is **mutated**: `map_match` is overwritten from `notmatch`/`matchi`/`matchesi` (bibtex.pm:731-750). This is harmless today because it is idempotent, but a port should not copy it.

**User regexes: what users can rely on.** These are Perl regexes compiled with `qr/$pattern/` (Unicode semantics; `i` for the `*i` variants; no `x`, `m`, `s`). The following features are CONFIRMED to work via biber.conf (`smap/sm.conf`, `sm2.conf`):
- `(\p{Lu})`, `\w` (Unicode)
- **look-behind** `(?<=x)` and look-ahead
- **backreferences** `(\d)\1`
- named groups `(?<n>…)`, but only usable as `$1…` (see below)
- `\x{263A}`, `\N{U+263B}`
- inline `(?i)`

Biblatex limitation: biblatex does not XML-escape `<`, `>` or `"` when writing `map_match`/`map_replace` into the `.bcf`. Look-behind, named groups and quotes coming from `\DeclareSourcemap` therefore produce a malformed `.bcf`, and biber aborts with "doc.bcf is malformed". CONFIRMED. So in practice look-behind only reaches biber through biber.conf.

**Replacement strings are executed as Perl code.** `ireplace` (Utils.pm:1173-1185) does `s{$re}{ Safe->new->permit_only(':default')->reval(qq{"$replace"}) }egxms`. Consequences, all CONFIRMED:
- `$1`, `$&`, `\u`, `\L…\E` and `\$` (literal) work.
- `@{[ uc $1 ]}` and `${\ scalar reverse 'x'}` **execute** (sandboxed) code.
- Backticks are blocked by Safe.
- `$+{name}` does **not** work.
- On *any* eval error, the match is replaced by the **stringified compiled regex**, e.g. `(?^u:(a)(b)?)` (`(?^ui:` for case-insensitive). Even a stray `"` triggers this.
- In fieldvalue, `$0` resolves to `$imatches[-1]` (the last capture).

### 2.5 Field handlers (bibtex.pm:52-85, 1003-1495)

Dispatch is by `(fieldtype, fieldformat||'default', datatype)` from the data model. Annotation fields are matched first by the regex `marker(?:namedmarker.+)?$` (bibtex.pm:1994-2004). If no handler exists (`undef`), Perl would crash; the handler tables are the ones shown in the code.

| Handler | Applies to | Behaviour | Edge cases | Diff/Risk |
|---|---|---|---|---|
| `_literal` | field/default: code, datepart, entrykey, integer, key, literal | 1. An XDATA ref (`xdata=key-field[-n]`) returns the raw value. 2. `year`/`month` are skipped if already set from a date; a non-integer gives the warning "legacy year/month … not an integer" (uses `Unicode::UCD::num`). 3. `isbn` (below). 4. `month` → `_hack_month`. 5. `langid` → `%LOCALE_MAP_R` (BCP47 → babel name). | `_hack_month`: `^\s*(jan|feb|…|dec).*\s*$/i` → month number from the first 3 grapheme clusters, so `Mayday` → 5. The "not an integer" warning fires even when `_hack_month` then converts `February` → 2. | L/L |
| ISBN | `isbn` | Business::ISBN parse + `is_valid` (checksum). Invalid → warning, value unchanged. `--isbn13`/`--isbn10` convert. `--isbn-normalise` → `as_string` (hyphenated **using the ISBN agency RangeMessage.xml data**). CONFIRMED `0-306-40615-2` → `978-0-306-40615-7`. A multi-ISBN string is invalid as a whole. | Needs the ISBN range table, embedded as a data file. | M/M |
| `_uri` | uri | Raw value plus XDATA ref record. Not LaTeX-decoded (pass 1 skips "verbs"). Escaping happens at **output** (`URI->new(NFC(v))->as_string`, Output/bbl.pm:557), with `urlraw` emitted raw. | URI escaping rules of Perl `URI` must be matched: `é` → `%C3%A9`, space → `%20`, `\`→`%5C`, `{`→`%7B`; existing `%` kept. | M/M |
| `_verbatim` | verbatim | Raw, not decoded. | | L/L |
| `_xsv` | xsv: entrykey, literal, keyword, option | `split(/$xsvsep/, v)`; default `\s*,\s*`; a Perl regex string from config. No brace awareness. Perl `split` drops trailing empty fields. | Granular XDATA never recognised (B10). | L/M |
| `_range` | range (pages…) | Split on `\s*[;,]\s*`; `~`→space; four regex alternatives using `\p{Pd}`; strip one level of braces from start/end. Result `[[start,end|undef],…]`. A failed parse gives the warning "malformed, falling back to literal" and keeps `[orig, undef]`. CONFIRMED: `5-` → `['5','']`; `12~--~15, 17; 20–22`; `M-1--M-4`; `xii--xv`. **`-5` and `-` are rejected** (B3). | Must replicate regex alternation order exactly. | M/M |
| `_list` | list: key, literal, verbatim | `split_list(v, listsep)` (btparse semantics, §2.7). `remove_outer` on each item. "others" is handled at output (`\true{more<field>}`, case-insensitive). CONFIRMED. | | L/L |
| `_urilist` | list uri | As `_list` but **without** `remove_outer`. | | L/L |
| `_name` | list name | §2.7 | | H/H |
| `_datetime` | date fields | §2.9 | | H/M |
| `_annotation` | `<field>+an[:name]` | §2.10 | | M/M |
| `options` | per-entry options | Processed *before* other fields: `split(xsvsep)`, `s/\s+=\s+/=/`, `key[=val]` (bare → 1), meta-option expansion (Utils.pm:1290-1304, 1353-1399). | | M/M |

**XDATA references at input** (Entry.pm:225-265):
- Value form: `^xdata=(\S+)$` (`/xi`; marker, `xnamesep` and `xdatasep` are configurable).
- Then `^([^-]+)-([^-]+)(?:-(\d+))?$`.
- **Consequence: XDATA entry keys or field names containing the separator `-` cannot be referenced** (they produce a "contains no source field" warning).
- Positions are 1-based. A missing position means `*` (all items).

### 2.6 LaTeX → Unicode decoding (Recode.pm) and encoding

- **Data.** `recode_data.xml` has 833 maps. Types:
  - `letters`, `diacritics`, `punctuation`, `symbols`: sets `base,full`, with an extra `symbols` block for `full`.
  - `negatedsymbols`, `superscripts`, `cmdsuperscripts`, `dings`, `greek`: `full` only.
  - `decode_exclude`: `-`, `|`, `textbraceleft/right`, `textunderscore`, `textasciitilde`, `textasciicircum`, `textasciigrave`, `dots`.
  - `encode_exclude`: TeX specials, space, `£ ± ¶`, `g`.
  - `preferred` and `raw` attributes pick the encode direction.
  - Defaults: `decodecharsset=base`, `output_safecharsset=base`. `--recodedata` loads a user XML via kpsewhich.
- **Where decoding applies.** Pass 1 decodes every field except the data-model "verbs" (verbatim/uri fields) (bibtex.pm:1722). Unknown entry metatypes are decoded whole (`print_s`).
- **Algorithm** (`latex_decode`, Recode.pm:200-345), in this order:
  1. `\hbox {x}` → `\hbox{x}`.
  2. `\char"HEX`, `\char'OCT`, `\charDEC` → the character. This uses `/ee`, i.e. Perl eval of `chr(...)`. Bad octal such as `\char'89` evaluates to empty.
  3. `\foo\ ` → `\foo{} `.
  4. `[^{]\\\w[;,.:%]` → add `{}`.
  5. Type passes in order greek, dings, punctuation, symbols, negatedsymbols, superscripts, cmdsuperscripts, letters, diacritics. Within a type, macro names form an alternation sorted by **descending length**, with only `.^|+-)(` escaped.
     - letters: `\\(name)(?:\{\}|\s+|\b)`. All following whitespace is consumed, which is TeX-correct.
     - punctuation/symbols/greek: `\\(name)(?: \{\}|\s+|\b)`. **Note the literal space**: `\textdollar{}` leaves `{}` (B4).
     - diacritics: a complex three-stage procedure.
       - (a) Protect simple `{X}` single-grapheme groups that are *not* the argument of a diacritic macro, using a variable-length-look-behind emulation (recursive named groups + `\k`). They are mapped to `\x0f…\x0e`.
       - (b) `{?\acc\s*{L M*}}?` → letter+combining mark, with the original braces turned into `\x1f`/`\x1e`.
       - (c) `{L M*}` → `\x1f…\x1e`.
       - (d) `\acc` + optional space + letter. This uses a **conditional regex with embedded code** (`(?(?{…})…)`): a symbol accent such as `\'` may be followed directly by any letter, but a letter accent such as `\c` needs a space or a non-ASCII-letter.
  6. Brace elimination: remove `{X}` / `\x1f X \x1e` around a single grapheme unless it is preceded by `\\\pL+(?:\{[^{]+\})*` (another VLB emulation). Then restore the markers to braces.
  7. `ı́` → `í`.
  8. NFD.
- **Pinned in t/utils.t:69-117.** Examples: `{M{\'a}t{\'e}}`→`{Máté}`; `R{\'egis}`→`R{égis}`; `\frac{a}{b}` unchanged; `\textuppercase{\'e}`→`\textuppercase{é}`; `{\v{C}}`→`Č`; `{I}`→`{I}`; `\&{A}` unchanged; `--` unchanged; `\textless\textampersand`→`<&`.
- **Quirks observed** (CONFIRMED, `runs/dec1`):
  - `{\ae}x`→`{æ}x` (braces kept), whereas `{\'e}t\'e`→`été` (braces dropped).
  - `\"\i`→`ı̈` (dotless i + diaeresis, not `ï`).
  - `\'\i`→`í`.
  - `\i and` eats the following space.
  - `\^{}x` and `\textsuperscript{2}` (base set) are unchanged.
- **Encoding** (`latex_encode`, output side; used for `--output-safechars` and the bibtex output format). Types run in order greek, dings, negatedsymbols, superscripts, cmdsuperscripts, diacritics, letters, punctuation, symbols:
  - diacritics: special case for `i`+accent → `\={\i}`; stacks of up to 3 combining marks.
  - `_wrap`: `\text…{}` / raw / `{$\cmd$}`.
- **Port.** These regexes (VLB emulation, recursion, code conditionals) cannot run in Rust regex engines. **Write a hand-coded scanner/state machine** that reproduces the same rewrites, and pin it with a large differential corpus generated from biber (see §5). Difficulty **H**, risk **H**: this is the most likely source of silent divergences.

### 2.7 Names

**Splitting the list** (`_name`, bibtex.pm:1201-1299):
- `Text::BibTeX::split_list(NFC(v), namesep)`. `namesep` defaults to `and` and is case-insensitive.
- A delimiter must be at brace depth 0, surrounded by whitespace, and **not at the start or end**. `A Aa and` → one name whose family is `and`, with no warning. CONFIRMED.
- `A~and~B` is not split.
- An empty piece (`and and`) → warning "Name … is empty (probably consecutive 'and')", and the **whole entry is skipped** (`BIBER_SKIP_ENTRY`, del_citekey). btparse also warns `substring 2, warning: empty substring`. CONFIRMED.

Per name:
1. XDATA ref → placeholder `Name(xdata=…)`.
2. **Per-namelist option.** If the item matches `^(\S+)\s*=\s*(\S+)?$` and the name is a NAMELIST-scope option (e.g. `uniquelist=minyear`), it sets that option and adds no name. CONFIRMED.
3. **Extended format.** Used if the item matches `(?:family|given|prefix|suffix…)\s*=` (the data-model nameparts, case-sensitive, no word boundary) and `--noxname` is off. Then `parsename_x`.
4. Otherwise BibTeX format.
   - Unless the whole item matches `^\{\X+\}$`: more than 2 commas → `biber_error` ("too many commas") and skip the entry. `,,` → error and skip the entry. CONFIRMED.
   - Names that start with `{` and end with `}` bypass these checks (B19).
5. `lc(rawstring) eq others_string` (`others`, case-insensitive; `{others}` also works) → `morenames`, i.e. `\true{moreauthor}`. CONFIRMED.

**`parsename` (BibTeX format)** (bibtex.pm:1784-1858):
1. Pre-sanitise:
   - trim;
   - `\s+|\\\s` → single space;
   - `^\{\{+([^{}]+)\}+\}$` → `{…}`;
   - with `--fixinits`, `(\w)\.(\w)` → `$1. $2`.
2. **btparse `split_name`**, then `format_name` per part with BTJ_MAYTIE token joins. Rules CONFIRMED by experiment (`runs/names`, `runs/nlist`):
   - Tokens are separated by **whitespace only** at depth 0. **`~` and `-` are not separators**: `Charles~de~Gaulle` is one token, and `Jean-Pierre` stays whole. Commas at depth 0 split the parts.
   - 0 commas: "First von Last". The **von part = the first *contiguous* run of lowercase-initial tokens**, excluding the last token. First = the tokens before it; Last = everything after it. So `a b c D e F` → von `a b c`, last `D e F`. **This differs from BibTeX 0.99**, which gives von `a b c D e`, last `F`. With no lowercase tokens, Last = the final token. `aa bb` → von `aa`, last `bb`.
   - 1 comma: "von Last, First". 2 commas: "von Last, Jr, First". More than 2: btparse warns "too many commas in name (removing extras)" (only reachable via B19). A trailing comma gives a warning "comma(s) at end of name (removing)".
   - Case test is **Unicode-aware**: `ąb` and `ébauche` count as lowercase (von); `Émile` and `ŁUKASZ` count as uppercase. Braced tokens `{von}` count as non-lowercase. Special chars `{\relax d}e` are **not** treated as von, whereas BibTeX would treat them as von.
   - Tie insertion (MAYTIE): 2 tokens → `a~b`. 3 or more → a tie after the first token if it is "short", otherwise a space, then spaces, then a tie before the last token. Examples: `Jean Charles~Gabriel`, `J.~C.~G.`, `Ab~Cd~Ef`, `John~{von}`. "Short" is < 3 characters in BibTeX terms; **exact length counting (special chars, UTF-8 bytes vs characters) needs confirming**.
3. For each part:
   - `remove_outer` → `strip` flag (re-braced at output).
   - Spaces inside braces → `_`.
   - `strip_noinit`: user/default regexes `\b\p{Ll}{2}\p{Pd}(?=\S)` and `[\x{2bf}\x{2018}]`, compiled **with `/gxms`**. Then remove `\cmd{…}` macros.
   - Split on `[\h~]+(?![^{]*\})`.
   - `gen_initials` (Utils.pm:1843): for a hyphenated part not wholly braced, the initials are joined with `-`, so `{K}ent-{B}oswell` → `K-B`; a hyphen protected as `{-}` does not split. Otherwise strip leading `{`, take the first **grapheme cluster** (Unicode::GCString), or 2 clusters if the first is `\p{Dia}`.
4. Record max lengths per namepart (`set_np_length`).

**`parsename_x` (extended)** (bibtex.pm:1879-1965):
- Split the item with **Text::CSV** (`sep ,`, `allow_whitespace`, `binary`), so `"family=Doe, Jr"` must quote the whole field.
- Each part is matched with `^(.+)\s*=\s*(.+)$` under `/x`. The first group is greedy, so it splits at the **last** `=`.
- Per-name options (NAME scope, e.g. `useprefix=true`) are applied. Unknown nameparts give the warning "Invalid namepart option".
- `<np>-i` = explicit initials with `{…}` compounds (`_split_initials` on `\b{gcb}`). `id` = hashid.
- Unbraced values: `join_name_parts` (tie rule: 1 part as is; 2 parts → `a~b`; otherwise a tie after the first part if it is < 3 graphemes, spaces, a tie before the last part).
- Braced values are kept, but **their strip flag is not recorded**, so the braces are lost at output (B13).
- Initials split on `[\s~]+`, which differs from `parsename`'s brace-aware split.

**Name objects** (`Entry/Name.pm`, `Names.pm`):
- Fields: nameparts `{string, initial[]}`, strip, per-name options, hashid, `id` = `suniqid` (random), and `index`.
- Index base is inconsistent (S1).
- `rawstring` concatenates the parts in hash order (S2).

### 2.8 Unicode normalisation & internal string form

- Inputs are **NFD** internally:
  - T::B `normalization=>'NFD'` on all reads;
  - `preprocess_file` applies NFD;
  - `latex_decode` returns NFD;
  - the `.bcf` is NFD'd, except datasource filenames (Biber.pm:368, 423-427);
  - cite keys are NFD (Biber.pm:924-991).
- Strings are converted to NFC at every boundary into btparse (`encode('UTF-8', NFC(x))` on get/set/split_list/Name->new) and on output (`out()`, Utils.pm:1575).
- The md5 used for the temp filename is computed on the NFC basename.
- NFKD is used only in range length and roman numerals (Utils.pm:1645-1651).
- **Port.** Keep strings NFD internally to match comparison, hash and length semantics (e.g. `length()` of namepart strings counts NFD code points). Use the `unicode-normalization` crate, but **pin its Unicode version to the one in biber's Perl 5.42 (Unicode 15.1/16?)**; see Open questions. Grapheme clusters come from `unicode-segmentation` (UAX#29); Perl's `\X`/GCString version must match.

### 2.9 Dates (`_datetime` bibtex.pm:1302-1442; Utils.pm:1407-1556; Date/Format.pm)

- `parse_date_range`: `^([^/]+)?(/)?([^/]+)?$`. Anything with two or more `/` gives "Invalid format".
  - Start missing with `/` present → `<type>dateunknown`.
  - End missing → `<type>enddateunknown` and `endyear={}`.
  - `..` → open (an empty date object, year=`''`).
- **Unspecified** (start only, Utils.pm:1438-1479):
  - `199X` → 1990/1999 `yearindecade`
  - `19XX` → `yearincentury`
  - `1999-XX` → `monthinyear`
  - `1999-01-XX` → `dayinmonth`, with leap-year-aware month lengths
  - `1999-XX-XX` → `dayinyear`
  - Any other `X` pattern → **silently an empty year** (B12). `X` in the end date → "Invalid format of end date".
- **Format.pm.**
  - The DateTime::Format::Builder grammar tries, in order: `[-]YYYY-MM-DDThh:mm:ss[.mmm]`, `[-]YYYY-MM-DD`, `[-]YYYY-MM`, `[-]YYYY`, `Y[-]NNNNN…` (5+ digits).
  - Length guards apply to the *pre-processed* input.
  - Times require seconds. Milliseconds are discarded.
  - Timezone is `Z` or `±hh:mm` only. `±hhmm` is not supported. Output `+01\bibtzminsep 00`, `Z`.
  - Pre-processing order, all anchored at the end: strip `?` (uncertain), then `~` (approximate), then `%` (both). Then the TZ. Then the yeardivision `-2[1-9]|-3\d|-4[01]` → `%YEARDIVISIONS` (spring … QD1, etc.; Constants.pm:77-99).
  - **Non-ASCII digits:** any Unicode `\d` run is converted with `Unicode::UCD::num` and remembered, so the output keeps the original script (`٢٠٠٠-٠١` → year `٢٠٠٠`, month `٠١`). CONFIRMED.
  - DateTime validation rejects month 13, Feb 30 and similar ("Invalid format").
- **Julian.** If the biblatex option `julian` is set and both month and day are present, a date before `gregorianstart` is converted with DateTime::Calendar::Julian and flagged `datejulian`.
- **Fields set:**
  - `<t>year/month/day/hour/minute/second/timezone` (datafields);
  - `<t>end…`;
  - `<t>era`/`endera` = `lc secular_era` (`ce`/`bce`);
  - `<t>dateapproximate`, `uncertain`, `unspecified`, `julian`, `datesplit`, `yeardivision`, `endyeardivision`;
  - plus a misspelled legacy `endseaason` (S3).
  - Year values are output as given (e.g. `-0044` → year `44` with era `bce`; the conversion is in the output layer — see Open questions).
- **Overwrite warnings.** "Overwriting field 'year'/'month' with value from field 'date'" when the legacy `year`/`month` differ from the date. These use a numeric compare (`!=`), so `February` vs 3 triggers too.

### 2.10 Annotations (`_annotation` bibtex.pm:1007-1039; Annotation.pm)

- The field name is `<field><annotation_marker>[<named_marker><name>]`, default `+an` and `:`.
- The value is split on `\s*;\s*`. Each item matches `^\s*(\d+)?:?([^=]+)?=(.+)`:
  - `n:part=…` → part scope (`\annotation{part}{field}{name}{n}{part}{literal}{value}`);
  - `n=…` → item scope;
  - `=…` → field scope;
  - `part=…` with no count → part scope with an empty item (CONFIRMED, probably meaningless).
- A value in double quotes is `literal=1`. Because of the `;` split, a literal cannot contain `;` (B11).
- Storage is global, keyed by citekey, so it is shared across sections.

### 2.11 biblatexml (lower depth; `Input/file/biblatexml.pm`, 1592 lines)

- The XML is slurped (encoding parameter ignored; always UTF-8), NFD'd and parsed with XML::LibXML. Namespace `http://biblatex-biber.sourceforge.net/biblatexml`, prefix `bltx`.
- Optional RNG schema generation from the data model and validation (`--validate-bltxml`).
- Entries: `/bltx:entries/bltx:entry[@id][@entrytype]`. Sourcemaps work the same way, applied through XPath (`_changenode`, `_getpath`).
- Field handlers are a parallel table: `_literal`, `_xsv`, `_uri`, `_list`, `_range`, `_datetime`, `_name`, `_annotation`, `_related`. Field names are normalised with `lc` and the namespace prefix stripped.
- Differences from bibtex:
  - Names are structured: `<namepart type=…>` with nested `<namepart initial=…>` parts.
  - Dates are structured: `<date type=… ><start/><end/>` plus ISO strings.
  - The `xdata` attribute is normalised to `xdata=…`.
  - No LaTeX decoding and no btparse.
  - **URIs are percent-escaped at input** on NFD text, and only if they contain no `%` (B17).
  - Ranges come from `<list><item><start/><end/>`.
  - `eprint`'s `type`/`class` attributes are intended to become eprinttype/eprintclass but are dead code (B16).
  - The handler table has no `xsv`/`literal` and no `list`/`uri`/`verbatim` handlers. Such fields would hit an undef handler (unverified).

---

## 3. External-library behaviour we must replicate

| Library | What biber relies on | Rust approach |
|---|---|---|
| **btparse / Text::BibTeX 0.91** | Lexer/parser for `@entries` (§2.2), including `%` comments, junk handling, recovery to next `@`, partial-entry retention, case folding of type/field/macro names, the NAME char set, value post-processing (delimiter strip, macro expand, paste, whitespace collapse on the final string), the global macro table, `split_list` (delimiter rules), `split_name` (tokenisation, commas, Unicode-aware von detection with the "first contiguous lowercase run" rule), `format_name` with BTJ_MAYTIE ties, and the exact warning and error message texts biber surfaces (`BibTeX subsystem: …`). | **Write our own** parser in Rust. Get the btparse C sources from the Text-BibTeX CPAN dist (`btparse/src/*.c`, `bibtex.g`, `names.c`, `format_name.c`, `string_util.c`) and port `names.c`/`format_name.c` almost line for line. Do **not** use `biblatex`/`nom-bibtex` crates: their semantics differ. Difficulty H, risk H. |
| Text::BibTeX Perl layer | `get`/`set`/`fieldlist` with NFD/NFC conversion; `clone`; `print_s` (only for the dead `rawdata` field and BTE_UNKNOWN pass-1 re-emission). | Trivial. |
| **Perl regex engine** | User-supplied patterns: sourcemap `match*` (compiled plain `qr//`, `i` for `*i`); `noinit`/`nosort`/`nolabel`/`nonamestring` (compiled with **`/xms`**); `xsvsep` (split regex). Features in use: look-around, backrefs, `\p{…}` incl. Perl-specific property names (`\p{Dash}`, `\p{Dia}`, `\p{Pd}`, `\p{L}`), `\x{…}`, `\N{U+…}`, `\X`, `\b{gcb}`, inline flags, possibly recursion and possessive quantifiers. | Use **`pcre2`** (PCRE2 10.4x, UTF + UCP mode) for user regexes. It covers look-behind (bounded), backrefs, named groups, `\X`, `\x{}`, `\N{U+}` (UTF mode), `\Q…\E` and `(?x)`; translate `/xms` flags. **fancy-regex** lacks `\X` and `\N{U+}`. Validate property-name coverage (`\p{Dia}`, `\p{Dash}`) at startup and warn on unsupported syntax. Built-in biber regexes should be hand-ported to `regex`/manual code. Difficulty M, risk M. |
| **Perl `Safe` + string eval of replacements** | Replace strings are Perl `qq{"…"}`: `$1..$n`, `${n}`, `$&`, `` $` ``, `$'`, `\u \l \U \L \Q \E`, `\$`, `\@`, escapes; arbitrary sandboxed expressions `@{[ … ]}` / `${\ … }`; the error fallback inserts the stringified `qr` (`(?^u:…)`/`(?^ui:…)`). | Implement a **Perl interpolation mini-language** for the common subset. For `@{[ ]}`, `${\ }` and other code, emit a clear warning (deliberate incompatibility) or emulate the error fallback. Decide policy (Open questions). |
| Text::Balanced `extract_bracketed` | `remove_outer`: outer braces are removed only if the whole string is one balanced `{…}` group; backslash-escaped braces are not counted. | Small custom function. |
| Text::CSV | `split_xsv` for extended names: `,` sep, quotes `"`, `allow_whitespace` trims, binary. | `csv` crate with trim, or a custom splitter. |
| Unicode::GCString / Perl `\X`, `\b{gcb}` | Initials (first grapheme), month substr, `_split_initials`, match_indices. | `unicode-segmentation`. **The Unicode version must match.** |
| Unicode::Normalize | NFD/NFC/NFKD everywhere. | `unicode-normalization` (check its Unicode version). |
| Unicode::UCD `num` | Year/month numeric check; date digit-script mapping. | Table from UCD `Nd` digit values (`unicode` data); `num` returns undef for mixed scripts. |
| DateTime, ::Format::Builder, ::Calendar::Julian, ::TimeZone | Date validation (proleptic Gregorian, years down to −9999 and Y-prefixed large years), `secular_era`, Julian conversion, timezone naming (`UTC` → `Z`). | Write our own small date module (validation + Julian conversion via JDN). Avoid `chrono` year-range limits for the `Y…` years. |
| Business::ISBN (+ RangeMessage.xml), ::ISSN, ::ISMN | ISBN validity (checksum), 10↔13, hyphenation from ISBN agency ranges; ISSN/ISMN validity (datamodel validation). | Write our own and embed RangeMessage.xml (record the version bundled with biber 2.22). |
| URI (Perl) | `URI->new($s)->as_string`: trims whitespace and strips `<…>`, `"…"` and `URL:` wrappers; escapes chars outside the URI char set as UTF-8 `%XX`; keeps `%`. | Port the `URI::Escape` rules; `percent-encoding` with a custom AsciiSet. |
| LWP / IO::Socket::SSL / Mozilla::CA | Remote datasources. | `ureq`/`reqwest` (rustls + webpki-roots) with the same headers and UA. |
| kpsewhich | Datasource and recodedata lookup. | Spawn `kpsewhich` (same as biber). |
| Encode / File::Slurper | Strict decoding by Perl encoding names. | `encoding_rs` plus a Perl-name → WHATWG label alias table. Note encoding_rs is lenient (it replaces bad bytes), so a strict mode must check for errors. |
| Data::Uniqid `suniqid` | `$MAPUNIQ`, internal ids. | Any unique id. Output containing `$MAPUNIQ` is inherently non-deterministic. |
| Text::Roman | `rangelen` roman numerals. | Small custom function. |
| XML::LibXML (+RelaxNG) | biblatexml, `.bcf`, recode data. | `roxmltree`/`quick-xml`. RNG validation via `libxml` bindings or skip (Open questions). |

---

## 4. Rust design suggestions for this area

1. **Single-pass parser with an explicit "pass-1 compatibility" step.** Parse `.bib` → `RawEntry { type_lc, key, fields: IndexMap<name_lc, ExpandedValue>, partial: bool }`, keeping per-field source line numbers. Then apply `latex_decode` to non-verbatim fields. To stay byte-compatible, add a `reserialise_check` that detects decoded values with unbalanced braces (B2), and decide per bug whether to emulate it.
2. **Macro table** as a process-global (per-run) `HashMap<String,String>` with lowercase keys. Seed `jan..dec` → `1..12` unless nostdmacros (or Text::BibTeX's `January..` defaults). Clear it on `--clrmacros` per section.
3. **Name module.** Port btparse `names.c` + `format_name.c` faithfully, with Unicode-aware case detection. Behind a `NameSplitter` trait, so a "BibTeX 0.99" mode could exist later.
4. **Diagnostics.** Model btparse errors as `ParseDiag { severity, file, line, msg }` and format them exactly like `BibTeX subsystem: <file>, line N, <kind>: <msg>`. Prefer the real user path; keep a compat flag if the temp path matters (it does not for tests).
5. **Sourcemap engine** as a separate module operating on `RawEntry`:
   - steps compiled once (regexes cached);
   - a step-evaluation order identical to bibtex.pm:437-920;
   - explicit `Captures` state per map iteration;
   - replacement-string interpreter (§3);
   - `$N` substitution rules (single digit vs multi-digit, `$0` → last);
   - shared by the bibtex and biblatexml drivers. biblatexml uses XPath node paths, so abstract "field get/set/exists/delete".
6. **Field handlers** are a dispatch table keyed on data-model (fieldtype, format, datatype), returning typed values: `Literal(String)`, `Range(Vec<(String, Option<String>)>)`, `Names(NameList)`, `List(Vec<String>)`, `Date(DateParts)`, `XdataRef(...)`.
7. **LaTeX recode.** Generate tables from `recode_data.xml` at build time (`build.rs`) and implement decode as a tokenizer. Keep a slow "regex-faithful" reference implementation only in tests if useful.
8. **Normalisation.** Keep the canonical internal form NFD; NFC at output.
9. **Performance.** The Perl code re-parses each `.bib` twice, runs ~10 large alternation regexes per field, and does a linear `verbs` scan per field. A one-pass parser with an Aho-Corasick or trie for macro names should be the main speed win.
10. **Determinism.** Perl hash-order effects (allkeys creation order, `rawstring`, warning order) do not affect `.bbl` content as far as observed, but warning order in `.blg`/stdout varies run to run. Tests should compare warnings as multisets.

---

## 5. Test oracle notes

- The t/*.t tests drive internal Perl APIs, so they are not directly runnable against Rust. Their **inputs** (`t/tdata/*.bib`, `*.bcf`, `*.conf`, `*.bltxml`) and expected `.bbl` fragments are reusable.
  - `names.t` (1105 lines): many expected name structures and `.bbl` blocks with `namesep=und`, `others_string=andere`.
  - `names_x.t`: extended names.
  - `dateformats.t` (956): date parsing.
  - `encoding.t`: encodings 1-6 .bib/.bcf pairs.
  - `bibtex-aliases.t`, `maps.t`, `annotations.t`.
  - `utils.t:69-117`: latex_decode/encode unit pins.
  - `full-bbl.t` (a full `.bbl`).
  - `remote-files.t` needs network.
- **Primary oracle: run the installed `biber` 2.22** on a fixed `.bcf` (see `scratch/input/base/doc.bcf`) with generated `.bib` files and diff the `.bbl`. `biber --tool` emits the parsed and mapped `.bib` (fields upper-cased, `year`→`date` via tool config), which is useful for parser-level differential tests.
- **Differential fuzzing targets:**
  - btparse grammar: braces, quotes, `#`, `%`, junk, errors;
  - `split_list`/`split_name`/format ties;
  - `latex_decode` on random TeX-ish strings;
  - range parsing;
  - date strings;
  - sourcemap regex/replace.
  - Generate a corpus, run biber once, and store the expected outputs in the repo, so CI does not need TeX.
- Non-determinism to mask: `$MAPUNIQ` values, the temp-file path in btparse messages, and warning order.
- `--debug` changes behaviour (B15), so oracle runs must use the default log level.

---

## 6. Suspected biber bugs

"CONFIRMED" = reproduced with biber 2.22. Reproducers are in `/Users/ben/bibe-rs/scratch/input/runs/<name>` unless stated.

| # | Location | Description | Repro | Status |
|---|---|---|---|---|
| B1 | bibtex.pm:1739-1745 | Pass-1 re-serialisation of `@string` concatenates multiple macros **without commas** (`@STRING{aa = {Alpha}bb = {Beta}}`). Valid BibTeX `@string{aa="Alpha", bb="Beta"}` causes a fatal "syntax error: found "bb"" and no `.bbl`. | `runs/multistring` | CONFIRMED |
| B2 | bibtex.pm:1725 + Recode.pm:216-218 | `\char"7B` / `\char123` (or `\char"7D`) decode to a bare brace, which is written into `{…}` during pass-1 re-serialisation. The result is a fatal syntax error, and **the following entry is swallowed**. | `runs/charbrace`: `title = {a \char"7B b}` | CONFIRMED |
| B3 | bibtex.pm:1152-1198 | Range parser contradicts its own comments: `-5` and `-` give "malformed, falling back to literal". `{12-15}` is not protected by braces: the start is `{12` and the end is `15}`. `12~` keeps a trailing space (start `"12 "`). | `runs/ranges` | CONFIRMED |
| B4 | Recode.pm:244; recode_data.xml symbols | (a) `\textdollar`, `\textpercent`, `\textampersand` (base set) decode to raw `$`, `%`, `&`, which break the `.bbl` in UTF-8 output: pdflatex fails with "File ended while scanning use of \field". This is pinned by utils.t:78, so it is intentional but harmful. (b) punctuation/symbols/greek use `(?: \{\}|\s+|\b)` with a **literal space**, so `\textdollar{}5` → `${}5` and `\pounds{}7` → `£{}7`, while letters (`\ss{}`) are handled correctly. | `runs/dec1`, `runs/dec2` | CONFIRMED |
| B5 | bibtex.pm:838-839 | `map_field_source`→`map_field_target` with `map_entrytarget` reads the value from the **current entry** (`$entry->get`), not the target. The new entry got `note=ORIGTITLE` instead of `NEWTITLE`. | `smap/sm3.conf` | CONFIRMED |
| B6 | bibtex.pm:678-694 | `map_entrykey_starnocited` skips only if `\nocite{*}` is present *and* the key is explicitly cited. Without `\nocite{*}`, the step applies to every entry, including `\cite`d ones. | `runs/star` | CONFIRMED |
| B7 | Utils.pm:1778 | `maploopreplace` returns early when not in a `map_foreach` loop, so `$MAPUNIQ`/`$MAPUNIQVAL` (and `$MAPLOOP`) stay literal in ordinary maps (`addendum = R-Q-$MAPUNIQ`). The docs only mention them alongside foreach, so this is ambiguous; the `my $MAPUNIQVAL` at bibtex.pm:435 is an unused lexical. | `smap/sm.conf` | CONFIRMED (behaviour); intent uncertain |
| B8 | Utils.pm:1173-1185 | Sourcemap replace strings are evaluated as Perl code (Safe `reval`). `@{[ uc $1 ]}` executes. On *any* error the match is replaced with the stringified regex `(?^u:…)` and no warning is given (e.g. a stray `"` or `$+{name}`). | `smap/sm.conf`, `sm2.conf` | CONFIRMED |
| B9 | bibtex.pm:792; Utils.pm:1157-1161 | `$0` resolves to `$imatches[-1]` (the last capture). After `notmatch`, `$1` == `"1"`. Captures are interpolated **unescaped** into the next match pattern, and the `$N` digit rules are inconsistent (single digit in fieldvalue/newkey, multi-digit in match). | `smap/sm2.conf` | CONFIRMED ($0, notmatch); rest by reading |
| B10 | bibtex.pm:1129-1139 | `_xsv` calls `add_xdata_ref($field, $arrayref)`, so the granular-XDATA regex never matches. `keywords = {xdata=xd-keywords}` is output literally as a keyword. | `runs/ann` | CONFIRMED |
| B11 | bibtex.pm:1020-1036 | Annotations are split on `;` before literal-quote handling. `="literal; x"` loses the literal flag, and the fragment without `=` **overwrites** the annotation with undef (empty output). | `runs/ann` | CONFIRMED |
| B12 | Utils.pm:1417-1428, 1438-1479; bibtex.pm:1384-1386 | Unspecified dates that match none of the supported `X` patterns (`199X-05`, `XXXX`) return an empty list, which yields "open-ended" `year={}` **without any warning**. `X` in the end date is reported as invalid. | `runs/dates` d03, d16 | CONFIRMED |
| B13 | bibtex.pm:1913-1930 | `parsename_x` strips outer braces (`family={Barnes Noble}`) but never records the `strip` flag, so the protection is lost in the `.bbl` (`family={Barnes\bibnamedelimb Noble}`). BibTeX-format names keep it. | `runs/xnames` | CONFIRMED (behaviour) |
| B14 | bibtex.pm:127 vs 1667-1670 | `ascii`→UTF-8 leniency in `preprocess_file` is dead code. The earlier strict `slurp_switchr($filename,'ascii')` in `extract_entries` already fails on non-ASCII bytes (fatal). | `runs/enc` (bcf with `encoding="ascii"`) | CONFIRMED |
| B15 | bibtex.pm:165-171, 304-329 | btparse messages are captured only when not `--debug`/`--trace`. With `--debug`, syntax errors are neither counted nor fatal, so biber writes a `.bbl`; normally it exits with an error. The `if/else` at 316-321 has identical branches. | `runs/dbg` | CONFIRMED |
| B16 | biblatexml.pm:922 | `fc(_norm($f) eq "$NS:eprint")`: the parenthesis is misplaced, and `_norm` already stripped the `bltx:` prefix, so it is never true. `<bltx:eprint type="arxiv" class="cs.DL">` produces no eprinttype/eprintclass. | `runs/bltx` | CONFIRMED |
| B17 | biblatexml.pm:968-975 vs Output/bbl.pm:553-557 | biblatexml URIs are percent-escaped at **input** on NFD text (`é` → `e%CC%81`), and `urlraw` is escaped too. BibTeX URIs are escaped at output from NFC (`%C3%A9`) with `urlraw` raw. | `runs/bltx` vs `runs/url` | CONFIRMED |
| B18 | Biber.pm:4622-4625 | `last unless (@remaining_keys or allkeys)` stops reading datasources once all cited keys are found. **@preamble entries (and @string macros) of later files are then dropped**, so `\preamble` in the `.bbl` depends on which keys are cited. | `runs/pre` | CONFIRMED |
| B19 | bibtex.pm:1266 | Comma sanity checks are skipped for any name that starts with `{` and ends with `}`, even if it is not wholly braced (`{Doe}, J, K, {L}`). btparse silently merges the extra commas (only a warning). This contradicts the explicit too-many-commas error path. | `runs/commas` | CONFIRMED |
| B20 | bibtex.pm:1648-1694 | btparse diagnostics name the temp file (`<md5>_<pid>.utf8`) instead of the user's `.bib`. Errors from pass 2 carry line numbers of the re-serialised buffer. "Invalid or undefined BibTeX entry key" also prints the temp path. | any error run | CONFIRMED (cosmetic) |
| B21 | btparse | `@misc{key}` (an entry with no fields) and `@comment text` (no delimiters) are fatal syntax errors; BibTeX 0.99 accepts both. This is a compatibility gap rather than biber logic. | `runs/keys`, `runs/cmt` | CONFIRMED |
| B22 | bibtex.pm:1061-1066 | The legacy month warning "not an integer" fires for `February` even though `_hack_month` then converts it to `2`. Overwrite warnings use numeric `!=` against non-numeric legacy values. | `runs/dates` d18 | CONFIRMED (minor) |
| S1 | Entry/Names.pm:77, 90, 102-108 | `add_name` sets a 1-based index, while `replace_name` and `splice` set 0-based indices. `splice`'s loop `$i<$#list` also skips the last element. `get_index <= $ul` (Biber.pm:3566 etc.) assumes 1-based, so uniquelist/uniquename after granular XDATA name inheritance is likely off by one. `replace_name` also inserts the *shared* Name object from the XDATA entry. | — | Suspected (reading) |
| S2 | Entry/Name.pm:75-76 | `rawstring` joins nameparts in Perl hash order, which is non-deterministic. Currently only used for the "others" check, so it is harmless. | — | Suspected, latent |
| S3 | bibtex.pm:1413 | Legacy field misspelled `endseaason`; the start date sets no legacy `season`. Dead/inconsistent. | — | Reading |
| S4 | Constants.pm:192, 201 | ISSN/ISMN validators use `$_` instead of their argument `$v`. This works only because the caller loops with `$_`. The ISBN DM validator does not check `is_valid` (checksum). | — | Reading, latent |
| S5 | bin/biber:55 | Option spec `input_encoding|input-encodinge=s` is a typo. `--input-encoding` works only via Getopt abbreviation. | `runs/enc` | CONFIRMED works |
| S6 | bibtex.pm:884 | `next unless $last_type` is always true (defaults to the entry type). Dead. | — | Reading |
| S7 | Utils.pm:112-118 | `globU` concatenates the raw, NFC and NFD glob results without dedup. On normalisation-insensitive filesystems (APFS) a non-NFC pattern may list the same file twice, which reads it twice and produces duplicate-key warnings. | — | Suspected, untested |
| S8 | bibtex.pm:1887 | x-name part regex `^(.+)\s*=\s*(.+)$` is greedy, so it splits at the *last* `=`. | — | Reading |
| S9 | bibtex.pm:1244-1249, 1274 | Comments claim consecutive `and`/commas segfault Text::BibTeX. With 0.91, `{Doe},, {L}` parses without a crash, so the guards may be stale. Keep them for compatibility. | `runs/commas` | Observation |

---

## 7. Open questions

1. **btparse exact rules** to lift from source (Text-BibTeX 0.91 dist):
   - the NAME/key character set;
   - the "short token" length used for MAYTIE (characters vs bytes; special chars count as 1?);
   - von detection for `{\cs …}` special chars;
   - whether `Entry->new` ever returns false mid-file. If it does, the `while` loops at bibtex.pm:1521 and 1715 would silently drop all remaining entries; this was not observed.
2. **Unicode versions**: Perl 5.42 bundles Unicode 15.0/15.1? The case tables in btparse are its own. These affect `\p{…}`, NFD, grapheme clusters, and `num`.
3. **Replace-string policy**: support only the interpolation subset (`$n`, `\u\L…`) and warn on code constructs, or embed a tiny expression evaluator? Recommend the subset plus emulation of the `(?^u:…)` error fallback for pure compatibility, behind a flag.
4. **Whether to replicate harmful bugs** (B1, B2, B4a, B18) or fix them. Recommendation: fix B1/B2/B5/B6/B10/B11/B16 behind a `--compat=2.22` switch defaulting to compatible, and document each.
5. **Negative years**: `-0044-03-15` produces year `44` + era `bce` in the `.bbl`. ISO astronomical −44 = 45 BCE, so check the output layer's conversion (owned by the output-analysis doc).
6. biblatexml handler gaps: fields of datatype xsv-literal, list-uri or list-verbatim in a `.bltxml` probably call an undef handler. Test before porting.
7. RNG validation of `.bltxml` and the `.bcf`: needed for parity, or can it be best-effort?
8. Remote sources were not exercised (no network in this study). Confirm redirect handling, the `.bib` suffix for non-bib content, and ftp support in the packaged binary.

# 02 — Configuration, control file (.bcf), data model, CLI and program structure

Scope: `bin/biber`, `Biber::parse_ctrlfile` (lib/Biber.pm:345–1170), `lib/Biber/Config.pm`,
`lib/Biber/Constants.pm`, `lib/Biber/DataModel.pm`, `Section(s).pm`, `DataLists.pm`,
the attribute-level parts of `DataList.pm`, `LangTag*.pm`, `data/schemata/{bcf,config}.rnc`,
`data/biber-tool.conf`, plus the relevant tests.

Research method: source reading of `vendor/biber` (HEAD) plus experiments with the installed
`biber` 2.22 binary (TeX Live 2026, biblatex 3.22, BCF version 3.11) under
`/Users/ben/bibe-rs/scratch/config/`.

**Important oracle fact:** the installed binary is a PAR::Packer executable. Its unpacked cache
(`biber --cache` prints the path) contains `inc/lib/Biber/*.pm`. I diffed them against
`vendor/biber/lib`: they are byte-identical except `$VERSION` ('2.22' vs '2.23') and
`$BETA_VERSION` (0 vs 1) in Config.pm. So the line numbers that appear in `.blg` files
(`Biber.pm:421>` etc.) map exactly onto the vendored source. bin/biber is packed as
`script/biber-darwin` (also identical apart from the name).

---

## 1. Overview

biber's "front end" is: parse the command line (Getopt::Long), build global option state
(defaults → tool-mode conf → user biber.conf → command line), initialise Log4perl, create
the `Biber` object, open the output file early (truncating it), then either fake a control
file (`--tool`) or parse the `.bcf` (XML read via `XML::LibXML::Simple::XMLin` into a loose
hash-of-arrays), then post-process options, build the `Biber::DataModel` from the
(.bcf / conf) data model, and run the per-section pipeline. Everything is stored in
**process-global mutable state**: `$Biber::Config::CONFIG` (options + per-section "state"),
a dozen exported package hashes in `Biber::Constants` (`%CONFIG_OPTSCOPE_BIBLATEX`,
`%DATAFIELD_SETS`, `%CONFIG_META_MARKERS`, ...), and `$Biber::MASTER` (the Biber object,
used by `biber_warn`/`biber_error` and by DataModel/Utils to find the current section).

Key takeaways for the Rust port:

* The `.bcf` should be parsed with a **typed** deserializer, not a generic XMLin clone, but
  several XMLin quirks leak into behaviour (ordering, attribute/element merging, whitespace
  content) — see §3.2.
* Option scoping (GLOBAL / ENTRYTYPE / ENTRY / NAMELIST / NAME) is **data-driven**: the `.bcf`
  (or tool conf) declares which option exists at which scope, with which datatype, and
  whether it is output back to the `.bbl` (`backendout`) or split into sub-options
  (`backendin`). Options not declared in an `<optionscope>` are **silently dropped** by
  `setblxoption`.
* Precedence differs between *biber* options and *biblatex* options (§3.4).
* Sections are processed sequentially and mostly independently, but share global
  state (Config, Text::BibTeX macro table, data source caches, citekey-alias tables in Section,
  keyorder, DOT graph) — parallelism is possible in Rust only after making that state
  per-section (§2.4).
* The `.blg` / stdout log format is parsed by latexmk; the `> LEVEL - message` shape and
  several exact message texts must be preserved (§3.1.4).
* I found ~25 real bugs, many CONFIRMED by running biber (§7). Several documented
  config-file forms crash biber or are silently ignored; `--tool` can overwrite the input
  file in two different ways.

---

## 2. Architecture & data flow

### 2.1 Top-level control flow (bin/biber)

```
GetOptions (bin/biber:33-138)          -> $opts hashref
  --help/--version/--cache/--tool-config short-circuit (exit 0)
  no @ARGV -> pod2usage exit 2           (bin/biber:174)
  legacy alias folding (tool_*, bbl*, outfile, outformat, -u, -U) (bin/biber:179-230)
  output_resolve meta-option split       (bin/biber:233-238)
  validation of output_format/tool/isbn/indent/fieldcase/dot_include/input_format
Biber->new(%$opts)                      (lib/Biber.pm:77)
  Biber::Config->_initopts(\%opts)       (Config.pm:115)   <- config file search, defaults,
                                                              conf file, cmdline, logging init
  File::Temp tempdir; Recode init_sets; $MASTER = $self
  --validate-config (only if -g given)   (Biber.pm:99)
  LangTags parser (Parse::RecDescent compile – unused in production, see §4.9)
log "=== <date>" to logfile only        (bin/biber:342)
compute output filename                 (bin/biber:353-386)
apply --output-directory                (bin/biber:389-392)
require Biber::Output::<format>; set_output_target_file($outfile)  <- OPENS/TRUNCATES file
tool ? tool_mode_setup : parse_ctrlfile($bcf)
set_output_target_file($outfile, 1)     <- re-open with final output_encoding
Biber::Config->postprocess_biber_opts   (sortcase/sortupper true/false -> 1/0)
set_output_target
debug/trace log level bump; trace dumps CONFIG and Biber object
tool ? prepare_tool : prepare
$biberoutput->output
$biber->display_end                     <- flushes queued WARNs, WARNINGS:/ERRORS: counts
exit 0  (display_end exits 2 if errors)
```

### 2.2 Per-section pipeline (`Biber::prepare`, Biber.pm:4474)

```
process_setup  (once)                    Biber.pm:1178
  - bibtex-output: delete all sections != 99999
  - ensure every section has an 'entry' datalist with global sorting template
  - Config->set_dm(DataModel->new(getblxoption('datamodel')))   <- DM built HERE, after conf+bcf
  - _resolve_datafieldsets()             (needs DM)
  - force output_safechars if output_encoding is ascii and input isn't
for section in sections sorted numerically:
  skip if no citekeys and not allkeys
  section->reset_caches; Config->_init (reset per-section state); set_current_section
  preprocess_options        (compile nosort/nonamestring/nolabel/noinit regexes IN PLACE)
  fetch_data                (datasources -> entries; missing keys -> warnings)
  process_citekey_aliases
  instantiate_dynamic       (dynamic sets, related clones)
  resolve_alias_refs; resolve_xdata; cite_setmembers; preprocess_sets
  calculate_interentry; process_interentry (crossref/xref inheritance)
  validate_datamodel        (only if --validate-datamodel)
  postprocess_sets; process_entries_static
  process_lists             (per datalist: name disambiguation, uniqueness, filtering, sorting)
  out->create_output_section
out->create_output_misc
```

Tool mode (`prepare_tool`, Biber.pm:4520) has a reduced pipeline over the single pseudo
section 99999 (note different order: resolve_xdata *after* interentry; no
citekey-alias pruning, no dynamic instantiation, no postprocess_sets/static).

### 2.3 Global state inventory (what a Rust design must own explicitly)

| State | Where | Lifetime / reset |
|---|---|---|
| `$CONFIG->{options}{biber}` | Config.pm | whole run; also mutated mid-run (e.g. `tool`, `mincrossrefs`, `output_safechars`, `sourcemap`, regex compilation of `nosort` etc.) |
| `$CONFIG->{cmdlineoptions}`, `{configfileoptions}` | Config.pm | provenance for `isexplicitoption` |
| `$CONFIG->{options}{biblatex}{GLOBAL|ENTRYTYPE|ENTRY|NAMELIST|NAME}` | Config.pm | ENTRY scope keyed `{citekey}{secnum}{opt}` |
| `$CONFIG->{state}{...}` uniqignore, crossrefkeys, xrefkeys, datafiles, crossref, xdata | Config.pm:66-112 | reset by `_init` each section |
| `$CONFIG->{state}{keyorder,internalkeyorder}` | Config.pm | keyed by section; never reset (set at parse time) |
| `$CONFIG->{state}{graph}` | Config.pm | accumulates across sections (DOT) |
| `$CONFIG->{dm}` | Config.pm:791 | built once in process_setup |
| `$CONFIG->{control_file_location}` | Config.pm:822 | (note `_init` resets `state.control_file_location`, a different key – dead) |
| `%CONFIG_OPTSCOPE_BIBLATEX`, `%CONFIG_SCOPEOPT_BIBLATEX`, `%CONFIG_OPTTYPE_BIBLATEX`, `%CONFIG_BIBLATEX_OPTIONS` | Constants.pm:688-695 | filled from conf + bcf `<optionscope>` |
| `%DATAFIELD_SETS` | Constants.pm | name -> member list; resolved against DM in process_setup |
| `%CONFIG_META_MARKERS` | Config.pm:268 | quotemeta'd annotation markers |
| `%CONFIG_OUTPUT_FIELDREPLACE` | Config.pm:256 | parsed `--output-field-replace` |
| `%RSTRINGS`, `%USEDSTRINGS`, `%REMOTE_MAP` | Constants/Utils | macro & remote-file caches |
| Text::BibTeX macro table | C library global | persists across sections unless `--clrmacros` |
| `$Biber::MASTER->{warnings}` (list), `{errors}` (count) | Utils.pm:463-487 | whole run |
| Dynamic accessors on `Biber::Entry::Names`/`Name` | Biber.pm:526-535, Config.pm:420-429 | `mk_accessors` for every NAMELIST/NAME-scope option name declared in the bcf/conf |
| Section-level caches (`labelcache_*`, `bcfkeycache`) | Section.pm:129 | reset per section |

### 2.4 Are sections independent? (parallelism)

Mostly yes semantically — biblatex treats `\refsection`s independently, and each
`Biber::Section` owns its entries, citekeys, aliases, set relations and datalists. But in
the Perl implementation they share:

* `Biber::Config` per-section state is *reset* (not partitioned) at the start of each section,
  so concurrent sections would clobber each other — must be made per-section.
* Per-entry options are already keyed by `{citekey}{secnum}`.
* Text::BibTeX macros: `@STRING`s defined while reading section 1's .bib are visible when the
  same/other .bib is read for section 2 (unless `--clrmacros`); also the month macros are
  re-defined per section. The order sections are processed in therefore matters for macro
  redefinition warnings (and theoretically for values if two files define the same macro
  differently). A Rust port that parses each data source **once** and caches the parsed
  entries (huge speed win when many refsections share one .bib) must preserve "macro
  definitions accumulate in file read order across sections" semantics, or document the
  divergence. (Data-source parsing details belong to the input agent.)
* Output: `.bbl` sections written in `sort keys` (string!) order of section numbers
  (bbl.pm:680) — "10" before "2". biblatex doesn't care, but byte-exact oracles do.
* Warnings are a single global ordered list; parallel sections would need deterministic
  merging (biber's own warning order is already partly nondeterministic, §6).

Conclusion: a Rust port can parse data sources once, then process sections in parallel with
per-section state, and serialise output/warnings in section order. Big win for documents
with many refsections (e.g. per-chapter bibliographies).

### 2.5 Where caching happens

* `%REMOTE_MAP` — remote URLs downloaded once per run (Utils.pm:242).
* Data source caches in the input modules (cached parsed .bib per file across sections — input
  agent's area).
* `Section::bcfkeycache`, `labelcache_v/l` — per section.
* DataModel `helpers` (pre-sorted field lists) and `sortdataschema` closure built once.
* `preprocess_options` re-compiles regexes every section *in place* (`qr/$re/` of an existing
  qr — harmless, re-wraps `(?^:...)`).

### 2.6 What a clean Rust architecture would change

* Replace global singletons with an explicit `Run` context: `Options` (immutable after
  setup), `DataModel` (immutable), `Templates` (immutable, from bcf), and per-section
  `SectionState`. The few places that mutate options mid-run (`tool`/`mincrossrefs`/
  `minxrefs` for bibtex output, `output_safechars` forcing, `sourcemap` merge, regex
  compilation) all happen before section processing and can be folded into setup.
* Logging: an explicit logger that both writes the `.blg` and stdout, collects warnings
  for deferred emission, and counts errors.
* Typed `.bcf` model with serde/quick-xml; keep a generic "option value" enum
  (`Str`, `Int`, `Bool`, `List(Vec<OptValue{content, attrs}>)`, `Xml(...)`) for biblatex
  options because their set is open-ended and declared at runtime by `<optionscope>`.

---
## 3. Feature inventory

Difficulty: **L** low / **M** medium / **H** high. Risk = correctness risk (chance a port
silently diverges).

### 3.1 Command line (bin/biber)

#### 3.1.1 Option parsing mechanics

* `Getopt::Long` with `:config no_ignore_case` (bin/biber:31). Defaults otherwise apply:
  **auto_abbrev is on** (any unique prefix of a long option name works, e.g.
  `--input-encoding` only works *because* it is a prefix of the misspelt alias
  `input-encodinge`, bin/biber:52), `--opt=value` and `--opt value` both accepted, single-dash
  long options accepted (`-tool` == `--tool`), bundling off. `permute` is on: options may
  follow the file argument. Short single-letter options are just one-letter long names.
  **A compatible Rust CLI must emulate unique-prefix abbreviation** (clap's
  `infer_long_args` is close but also must accept single-dash long names), otherwise
  scripts using abbreviations break. Risk M.
* Unknown or ambiguous option → Getopt prints `Unknown option: e` / `Option out is ambiguous (...)`
  and `pod2usage(-verbose=>0)` prints the SYNOPSIS, all on **STDERR**, exit **2** (confirmed).
  Single-dash long names (`-tool -noconf`) and unique prefixes (`--output-form=bibtex`) work (confirmed).
* `--help` → full POD (verbose 2) exit 0; `--version` → `biber version: 2.22` exit 0;
  `--cache` → PAR cache dir or message, exit 0; `--tool-config` → path of
  `biber-tool.conf` next to Config.pm, exit 0. No positional arg → usage, exit 2.
* Option values: `=s` strings, `+` counter (`quiet|q+`), `:80` optional int with default
  (`wraplines|w:80` – `-w` alone means 80; absent means default 0 from Constants),
  `:s@` (`dot_include`, repeatable, comma-split later), and the hash option
  `collate_options|c=s` stored into a hash (`-c level=3 -c variable=shifted`).

#### 3.1.2 Option list

Commonly used (by humans/editors/latexmk/arara) — **must be exact**:
`file[.bcf]` positional, `--output-directory`, `--input-directory`, `--debug/-D`,
`--trace/-T`, `--validate-datamodel/-V`, `--quiet/-q`, `--onlylog`, `--nolog`,
`--logfile`, `--output-file/-O` (incl. `-`), `--output-format`, `--tool`, `--configfile/-g`,
`--noconf`, `--mincrossrefs/-m`, `--output-safechars`, `--output-encoding/-E`, `-u`, `-U`,
`--input-encoding`, `--sortlocale/-l`, `--isbn-normalise`, `--isbn10/13`,
`--nodieonerror`, `--dieondatamodel`, `--clrmacros`, `--nostdmacros`, `--fixinits`,
`--noskipduplicates`, `--glob-datasources`, `--version/-v`, `--help/-h/-?`,
`--output-resolve*`, `--output-field-order`, `--output-legacy-dates`, `--output-align`,
`--output-indent`, `--output-fieldcase`, `--wraplines/-w`, `--strip-comments`,
`--collate-options/-c`.

Obscure: `--annotation-marker`, `--named-annotation-marker`, `--output-*annotation-marker`,
`--cache`, `--convert-control` (XSLT .bcf→.html), `--decodecharsset`, `--recodedata`,
`--output-safecharsset`, `--dot-include`, `--listsep/--namesep/--others-string` and
`--output-listsep/--output-namesep`, `--xdatamarker/--xdatasep/--xnamesep/--xsvsep` and
`--output-x*`, `--output-xname`, `--noxname`, `--no-bblxml-schema`, `--no-bltxml-schema`,
`--no-default-datamodel`, `--noremove-tmp-dir`, `--show-tmp-dir`, `--sortdebug`,
`--sortcase`, `--sortupper` (take `true|false`, validated in `postprocess_biber_opts`,
Config.pm:762-780 – anything else → `biber_error("Invalid value for option ...")`),
`--ssl-nointernalca`, `--ssl-noverify-host`, `--tool-noremove-missing-dependants`,
`--validate-bblxml`, `--validate-bltxml`, `--validate-config`, `--validate-control`,
`--winunicode/-W`, `--output-all-macrodefs`, `--output-no-macrodefs`,
`--output-macro-fields` (deprecated, prints notice on STDERR), `--output-field-replace`,
`--fastsort/-f` (documented as no-op but actually triggers a `biber_warn` per sorted list,
Biber.pm:4288 — so it *adds* WARN lines), `--input-format` (tool only).

Legacy aliases folded at bin/biber:179-230: `--tool-align`→output_align,
`--tool-fieldcase`, `--tool-indent`, `--tool-resolve`, `--bibencoding`→input_encoding,
`--bblencoding`→output_encoding, `--bblsafechars`, `--bblsafecharsset`, `--outfile`,
`--outformat`, `-u`, `-U`. Note the fold uses `if (my $o = $opts->{x})` so a false value
(e.g. `--tool-indent=0`) is **not** folded and stays as an unknown key `tool_indent`
(harmless but `--tool-indent 0` is silently ignored). `--output-resolve` expands into
the three `output_resolve_*` options.

Options that exist in `$CONFIG_DEFAULT_BIBER` but have no CLI flag: `graph`,
`nouri_encode`, `remove_tmp_dir`, `minxrefs`, `input_format` default, `output_*`
defaults. `minxrefs` can only come from the .bcf or conf.

Validation done in bin/biber (all exit 2 with a `Biber: ...` line on STDERR via `say STDERR`):
unknown `--output-format` (allowed: bbl dot bibtex biblatexml bblxml bibjson);
deprecation notice for bblxml/dot (STDERR, continues); tool mode output format must be
bibtex/biblatexml/bibjson; `--isbn10` + `--isbn13`; `--output-indent` must match `^\d+t?$`;
`--output-fieldcase` upper|lower|title (case-insensitive); `--dot-include` requires
`--output-format=dot` and values in section,field,crossref,xref,xdata,related;
`--input-format` only in tool mode and must be bibtex|biblatexml|'' (case-insensitive).
Non-tool `--output-format=biblatexml|bibjson` fails *later* (bin/biber:377, printed to
**stdout** with `say`, after log init, exit 2, no ERROR in .blg).

Tool-mode input format auto-detection: `.bib` → bibtex, `.bltxml` → biblatexml, else
default `bibtex` (Constants default).

#### 3.1.3 Output filename derivation (bin/biber:353-392)

| mode | format | default outfile |
|---|---|---|
| normal | bbl | `$bcf =~ s/bcf$/bbl/` |
| normal | bblxml | `s/bcf$/bblxml/` |
| normal | dot | `s/bcf$/dot/` |
| normal | bibtex | `s/\..+$/_biber.bib/` — **buggy**, see B5 |
| tool | bibtex | `$ARGV[0] =~ s/\.[^.]+$/_bibertool.bib/` |
| tool | biblatexml | `..._bibertool.bltxml` |
| tool | bibjson | `..._bibertool.json` |

`$bcf` = `$ARGV[0]` with `.bcf` appended unless it already ends in `.bcf` (Config.pm:221).
`--output-file` overrides; then `--output-directory` replaces the directory part
(`splitpath` → `catfile(outdir, basename)`), *even for `-`* (B7). Logfile name:
`--logfile` (with `.blg` stripped then re-added) or `$ARGV[0]` minus `.bcf` + `.blg`
(so tool mode on `refs.bib` logs to `refs.bib.blg`), and also moved into
`--output-directory`. No `@ARGV` (tests) ⇒ `nolog`.

The output file is opened for writing (truncated) **before** the .bcf is parsed
(base.pm:56-73, bin/biber:398) and re-opened after. Consequences pinned by behaviour:
an existing `.bbl` is truncated to 0 bytes even when biber subsequently exits on
"No data sources" (confirmed) or crashes. On malformed .bcf the output file is unlinked.
`-O -`: bbl.pm:664 writes to STDOUT, but base.pm already created an empty file literally
named `-` (B8).

#### 3.1.4 Logging, .blg format, exit codes — compatibility-critical

* Two Log4perl categories: `main` (→ Logfile + Screen) and `screen`/`logfile` helpers.
  Screen appender: **stdout**, `SimpleLayout` → `LEVEL - message`. Logfile: `clobber`,
  UTF-8, `PatternLayout "[%r] %F{1}:%L> %p - %m%n"` → e.g.
  `[112] Biber.pm:132> WARN - ...` where `%r` = ms since start and `%F{1}:%L` = Perl source
  file:line of the logging call (Config.pm:274-313).
* Levels: trace>debug>(default INFO)>`-q` ERROR>`-q -q` FATAL. `--nolog`: no file, and the
  "This is Biber"/"Logfile is" lines are suppressed. `--onlylog`: screen OFF.
  Debug/trace set from the .bcf (`debug` biber option) bump the level after parsing.
  `--debug` also prints `DEBUG mode: all messages are logged to '<blg>'` via screen logger.
* `=== <strftime "%a %b %e, %Y, %H:%M:%S">` goes only to the logfile (bin/biber:342).
* `biber_warn` (Utils.pm:463) does **not** log; it appends to `$MASTER->{warnings}` (and to
  the entry's warnings, which go into the .bbl as `\warn{}`). All WARN lines are emitted
  at the very end by `display_end` (Biber.pm:117), followed by `INFO - WARNINGS: n`.
  `biber_error` logs `ERROR` immediately, increments the count and exits 2 via
  `display_end` unless `--nodieonerror` or the `$nodie` arg is true. At the end
  `INFO - ERRORS: n` and exit 2.
* **latexmk** (`check_biber_log`, latexmk.pl:5306-5440) parses the `.blg` with regexes that
  require the `> ` before the level: `/> WARN /`, `/> (FATAL|ERROR) /`,
  `ERROR - Cannot find file '...'`, `Cannot find '...'`, `Cannot find control file '...'`,
  `ERROR - .*\.bcf is malformed`, `INFO - Data source '...' is a remote BibTeX data source - fetching`,
  `INFO - Found .* '...'`, `INFO - Found '...'`, `INFO - Reading '...'`,
  `INFO - Processing .* file '...'`, `INFO - Config file is '...'`,
  `WARN - Data source .*'...'`, `WARN - I didn't find a database entry for '...'`,
  `INFO - WARNINGS: n`, `INFO - ERRORS: n`. These message texts and the
  `"> LEVEL - "` shape are a de-facto API. The `[%r] File:line` prefix content is not parsed
  (a Rust port can emit e.g. `[12] biber:0> INFO - ...`), but anything between `[` and `> `
  must not contain `> `.
* Exit codes: 0 OK (incl. warnings, incl. "No data sources defined!"); 2 on any error
  (`biber_error`, option validation, pod2usage). Perl `die`/`croak` (internal crashes, bad
  XML config) exit with `$!`-dependent codes (observed 25, 102) and print a Perl message
  on STDERR — not worth emulating exactly, but "non-zero, non-2" is observable.
* arara's biber rule only uses the exit status. biblatex itself never reads the .blg.

#### 3.1.5 Tool mode (`--tool`)

* `Biber->new` loads `biber-tool.conf` (path next to Config.pm, or `configtool` test hook)
  *before* the user conf (Config.pm:139-149). That file supplies optionscopes, a full
  data model, inheritance rules, a `tool` sorting template, sortingnamekey/uniquename/
  namehash templates, datafieldsets, presort and a driver-like sourcemap.
* `tool_mode_setup` (Biber.pm:282): one pseudo section 99999 with `allkeys`, datasources =
  every positional argument (`type=file`, `datatype=input_format`, `encoding=input_encoding`;
  glob is undefined → see input agent), forces `dateapproximate/dateera/dateuncertain=1`,
  `namestrunchandling=0`, `sortlocale=en_US`, one entry datalist
  `tool/global//global/global/global`; forces all conf sourcemaps to `level=user`.
* `prepare_tool` pipeline (§2.2). Output defaults to bibtex.
* Non-tool `--output-format=bibtex` "fakes" tool mode after parsing the .bcf
  (Biber.pm:1127-1167): sets `tool=1`, `mincrossrefs=1`, `minxrefs=1`, creates section 99999
  with union of all sections' citekeys (allkeys if any section is allkeys) and *all*
  sections' datasources (not de-duplicated), adds a datalist using the global sorting
  template; `process_setup` then deletes the real sections. Pipeline is still `prepare`
  (not `prepare_tool`) but code paths check `getoption('tool')`.

### 3.2 Control file (.bcf) parsing — `parse_ctrlfile` (Biber.pm:345-1170)

Ordered behaviour:

1. `locate_data_file($bcf)` (Utils.pm:226) — the .bcf is searched like a data file:
   remote URL (!), absolute, `input_directory` (else `output_directory`), cwd, control-file
   dir (unset yet), `kpsewhich`. Not found → `ERROR - Cannot find 'x.bcf'!` and exit 2.
   **The more specific "Cannot find control file" error at Biber.pm:351 is unreachable**
   unless `--nodieonerror` (B10). latexmk thus treats a missing .bcf as a missing source
   file (return code 4) rather than "missing control file" (6). Keep the existing message.
2. Read as UTF-8; if that fails, read as latin1 and **rewrite the .bcf in place as UTF-8**
   (Biber.pm:358-366). Then NFD the buffer and well-formedness check with libxml; malformed
   → unlink output (unless `-`) and `ERROR - <path> is malformed, last biblatex run probably failed. Deleted <out>`.
3. `--validate-control` → RelaxNG validation against `bcf.rng` (Utils.pm:1193); failure is
   `biber_error`. The current biblatex 3.22 .bcf validates (checked).
4. `--convert-control` → XSLT `bcf.xsl` → `<bcf>.html`.
5. Re-read; NFD every line **except lines matching `<bcf:datasource.+>([^<]+)`** (keep
   file names in OS normalisation form) (Biber.pm:424-428).
6. `XMLin` with `ForceContent=>1`, `NsStrip=>1`, `KeyAttr=>[]` and a ForceArray regex list
   (Biber.pm:432-488).

   XMLin semantics that matter (XML::LibXML::Simple 1.x `collapse`/`_add_kv`):
   * **ForceArray applies to attributes as well as elements** (attributes go through
     `_add_kv`). Hence `$m->{field}[0]` for `<bcf:member field="x"/>`,
     `$ns->{value}[0]` for `<bcf:noinit value=...>`, `$list->{section}[0]` for
     `<bcf:datalist section="0">`, `$tr->{entrytype}[0]`.
   * An attribute and a child element with the same local name merge into one array.
   * Elements with text only → `{content => text}`; with attrs and text → hash with
     `content`; with child elements → text (incl. whitespace) is dropped.
   * Relative order between *different* element names is lost; order within the same name
     is document order. Most consumers sort by `order` attributes; the remaining
     order-sensitive lists (sections, bibdata, datalists, maps, inherit, constraints,
     entryfields, presort, transliteration) use document order.
7. `version` must equal `$BCF_VERSION` (`3.11`, Constants.pm:52) **exactly** (string
   compare), else error mentioning `bltxversion`. biber does *not* check biblatex's version
   otherwise; biblatex checks biber's `.bbl` version (`$BBL_VERSION` 3.3) on its side.
   `controlversion` is stored as a biblatex option (dropped silently since not scoped).
8. `<optionscope type=GLOBAL|ENTRYTYPE|ENTRY|NAMELIST|NAME>` → fills
   `%CONFIG_BIBLATEX_OPTIONS{scope}{opt}{OUTPUT}` (backendout), `{INPUT}` (backendin via
   `process_backendin`: comma list → arrayref, or `k=v` list → hashref),
   `%CONFIG_OPTSCOPE_BIBLATEX{opt}{scope}`, `%CONFIG_SCOPEOPT_BIBLATEX{scope}{opt}`,
   `%CONFIG_OPTTYPE_BIBLATEX{opt}` (warning if a datatype conflicts across scopes; note
   values are accumulated on top of those from biber-tool.conf/biber.conf).
   Then creates per-NAMELIST/NAME accessor methods (dynamic API; in Rust: a
   `HashMap<String, OptValue>` on name lists/names).
9. `<options component="biber" type="global">`: each option set **unless already explicit
   (cmdline or config file)**. singlevalued → string; multivalued → list of contents
   sorted by `order`. biblatex currently sends output_encoding, input_encoding, debug,
   mincrossrefs, minxrefs, sortcase, sortupper.
10. `<options component="biblatex" type="global">` → `setblxoption(undef, key, value)`;
    multivalued values keep their attribute hashes minus `order` (e.g. labeldatespec values
    are `{content, type}`), sorted by order. `type="<entrytype>"` → ENTRYTYPE scope.
    `setblxoption` maps booleans `true/false`→1/0 for options declared boolean
    (`map_boolean`, Utils.pm:1262 — unknown strings become undef) and **drops options not
    declared in the given scope**.
11. `<datafieldset name>` → `%DATAFIELD_SETS{lc name}` (bcf lowercases; config does not):
    members are field names or `{fieldtype, datatype}` resolved later against the DM.
12. `<sourcemap>`: merged with biber.conf maps (B13 for a lossy branch). Precedence:
    doc `\DeclareSourcemap` user maps are *prepended* to config user maps; final list =
    config(user) maps then bcf driver/style maps. (Execution: other agent.)
13. Templates (each re-parsed into normalised Perl structures, sorted by `order`):
    labelalphanametemplate (name → [ {namepart,use,pre,substring_compound,substring_side,substring_width} ]),
    labelalphatemplate (global or per `type`, stored raw), extradatespec
    (list of scopes, each list of fields), inheritance (stored raw XMLin structure),
    noinit / nolabel / nolabelwidthcount (lists of `{value=>re}`; only set if present —
    biber has defaults for noinit/nolabel/nosort), nosort / nonamestring
    (`{name=>field-or-set, value=>re}`), uniquenametemplate (name → [ {namepart,use,disambiguation,base} ];
    error if no template part has `base` anywhere — check is global, B15),
    namehashtemplate (name → [ {namepart,hashscope} ]), sortingnamekeytemplate
    (name → {visibility, template => [[ {type=namepart|literal, value, use?, inits?} ]] }),
    transliteration (`entrytype="*"` → global `translit`, else per entrytype; value = raw
    translit list with `langids/target/from/to`), sortexclusion / sortinclusion (per type
    → set of fields), presort (global or per type), sortingtemplate (via `_parse_sort`,
    Biber.pm:4995: list of passes, each `[ {final,sort_direction,sortcase,sortupper,locale}, {item => {substring_side,substring_width,pad_width,pad_char,pad_side,literal}}...]`,
    template locale = `locale2bcp47(template locale || global sortlocale)`;
    passes with no sortitems are dropped).
14. `datamodel` → `setblxoption('datamodel', [])` then push the raw XMLin DM (array so that
    tool-mode conf + user conf DMs can be merged later).
15. `<bibdata section=N>` / `<bcf:datasource type datatype encoding glob>`: de-duplicated
    per section on (type, datatype, name); encoding defaults to `input_encoding`, glob to
    `glob_datasources`. **If no section has any datasource:** `biber_warn("No data sources defined!")`
    then `exit 0` — the warning is never printed (B9), the (already truncated) .bbl is left
    empty. If *some* sections have sources but a cited section has none → crash (B11).
16. `<section number=N>` (several elements with the same number are merged — biblatex emits
    another `section 0` after each `refsection`): citekeys pre-pass: `\cite` beats
    `\nocite` of the same key (nocite attribute removed). Then for each key:
    `*` → allkeys (+ `allkeys_nocite` if nocite), keyorder recorded; else if not seen:
    `type="set" members="a,b"` → dynamic set (no order recorded, not counted as seen);
    otherwise record cite/nocite, keyorder (`order`), internal key order (`intorder`),
    push, seen++. `<citekeycount>` → `set_citecount`. allkeys ⇒ citekeys cleared.
    Logs `Found N citekeys in bib section S` / `Using all citekeys in bib section S`
    (one line per `<section>` element, so `Found 0 citekeys in bib section 0` lines are
    normal). Keys are NFD-normalised.
17. `<datalist>` (attrs: section, name, type entry|list, sortingtemplatename,
    sortingnamekeytemplatename, labelprefix, uniquenametemplatename,
    labelalphanametemplatename, namehashtemplatename; children `filter`/`filteror`):
    intended de-dup (Biber.pm:1032) is broken (B12). Default type `entry`; default name
    `stn/snktn/prefix/untn/lantn/nhtn`. Each list gets a level-1
    `Unicode::Collate::Locale` for sortinit hashing, locale from its sorting template.
    Then every `<section>` gets an `entry` list with the global `sortingtemplatename`
    and all-global templates if not already present (Biber.pm:1084-1111), and again in
    `process_setup` for any section lacking an entry list.
18. Warn `The file '<bcf>' does not contain any citations!` if no keys anywhere.
19. `normalise_utf8` (utf8/UTF8/utf-8 → `UTF-8` for input/output encodings).
20. bibtex-output pseudo section (see §3.1.5).

Difficulty H overall (lots of small structures), risk M. The XMLin quirks are the main trap.

### 3.3 Config file (biber.conf) — Config.pm

* Discovery (`config_file`, Config.pm:613-662) when `--configfile` is absent **or not an
  existing file** (B3) and `--noconf` absent: `./biber.conf`, `./.biber.conf`,
  `$HOME/.biber.conf`, `$XDG_CONFIG_HOME/biber/biber.conf`, `$HOME/.config/biber/biber.conf`,
  macOS `$HOME/Library/biber/biber.conf`, Windows `$APPDATA/biber/biber.conf`
  (POD/doc say `$APPDATA/biber.conf`, B23), else `kpsewhich biber.conf`. Files in cwd are
  returned as `abs_path`; others as built.
* Parsed with XMLin (different ForceArray list, Config.pm:363-403, no `bcf:` namespace).
  Malformed XML → libxml error on STDERR and Perl die (exit code = errno).
* Processing order inside `_config_file_set` (Config.pm:357-583): optionscope first
  (creates accessors), then datafieldsets, then every other top-level key in **hash order**
  (irrelevant except for keys that interact). Supported complex keys:
  labelalphatemplate, labelalphanametemplate (B16), uniquenametemplate, namehashtemplate,
  sortingnamekeytemplate, transliteration, nosort/noinit/nolabel (expects
  `<x><option .../></x>`, but `noinit`/`nolabel` are ForceArray'd so the documented form
  crashes — B1), collate_options (merged into defaults), sourcemap (driver/style levels
  rejected with `carp`), inheritance, sortexclusion, sortinclusion (B2), presort,
  sortingtemplate, datamodel (pushed onto the datamodel array). Everything else: if the
  value has `content`, it is a simple option (`setconfigfileoption`, which also sets the
  *biblatex* GLOBAL option of the same name if that option is known to be GLOBAL-scoped).
  **Anything else is silently ignored**, including the schema's `noinits`/`nolabels`,
  `nolabelwidthcount`, `nonamestring` (B1).
* `config.rnc` (schema) disagrees with code and doc in several places (B1, B24).
* `--validate-config`: RelaxNG check of the config file *after* it has been used.
* `data/biber-tool.conf`: same format; top-level elements: optionscope×5, datafieldset×2,
  datamodel, inheritance, namehashtemplate, presort, sortingnamekeytemplate, sortingtemplate
  (`tool`), sourcemap, uniquenametemplate, plus ~23 simple options.

### 3.4 Option storage, precedence and scoping

**Biber options** (`$CONFIG->{options}{biber}`, flat):
1. hard-coded `$CONFIG_DEFAULT_BIBER` (Constants.pm:236-311),
2. tool-mode `biber-tool.conf` (tool only),
3. user config file,
4. command line (`setcmdlineoption`), except `collate_options` which is merged key-wise,
5. `.bcf` biber options **only if not explicit** (cmdline or config file),
6. internal mid-run overrides (bibtex output, ascii safechars, `normalise_utf8`, tool setup).

So for biber options: **CLI > biber.conf > .bcf > defaults**. Confirmed by configfile.t
(mincrossrefs 7 from CLI beats conf 5; sortcase 0 from .bcf).

**Biblatex options** (`$CONFIG->{options}{biblatex}`):
1. config-file simple options that happen to be known GLOBAL biblatex options,
2. then `%CONFIG_DEFAULT_BIBLATEX` (Constants.pm:338-353; sortingtemplatename=tool,
   max*/min* = 100, use{author,editor,translator}=1, useprefix=0) **overwrites** them
   (B17, confirmed in tool mode),
3. then every `.bcf` biblatex option overwrites (non-tool mode).

So for biblatex options: **.bcf > hard-coded defaults > biber.conf**, and the CLI cannot set
biblatex options at all (except `--sortlocale`, which is a *biber* option consulted first
in `sort_list`, Biber.pm:4283: biber `sortlocale` > sorting template `locale` > biblatex
global `sortlocale`).

Scope lookup — `getblxoption(secnum, opt, entrytype, citekey)` (Config.pm:999-1021):
ENTRY (`{ENTRY}{citekey}{secnum}{opt}`) if opt is ENTRY-scoped and defined → ENTRYTYPE
(`{ENTRYTYPE}{lc type}{opt}`) → GLOBAL. Returns undef if the option has no GLOBAL scope
and no narrower value. NAMELIST/NAME scope values live on the Names/Name objects (set from
per-namelist/per-name `options` in data) — the namelist→entry→type→global cascade for
those is implemented in Entry/Name code (other agent), using
`%CONFIG_SCOPEOPT_BIBLATEX{NAMELIST|NAME}`.

Per-entry options (`options = {a=b, c}` field) are parsed by `process_entry_options`
(Utils.pm:1290): whitespace around `=` removed, bare option ⇒ `1`, values mapped through
`map_boolean`, meta-options expanded by `expand_option_input` using `backendin`
(array → same value for all sub-options; hash → per-sub-option values; for a false
meta-option only boolean sub-options are set, inverted). `merge_entry_options` for
inheritance/related options merges by option name with override.

Difficulty M, risk M-H (precedence subtleties, boolean mapping, silent drops).

### 3.5 Data model (DataModel.pm)

* Input: array of raw DMs (`[bcf_dm]` normally; `[tool_conf_dm, user_conf_dm]` in tool
  mode; in normal mode a user conf DM is also appended after the .bcf one? — no: the .bcf
  path *resets* `datamodel` to `[]` then pushes the bcf DM, so a biber.conf `<datamodel>`
  is discarded in normal mode). `--no-default-datamodel` ⇒ use `dms[1]` only.
* Merge of user DM into default (DataModel.pm:52-112): constants overwrite by
  case-folded name; constraints appended to `constraints[0]` only (B18); entryfields
  matched on the *first* entrytype only (B18); entrytypes appended; fields overwrite by
  case-folded name; multiscriptfields appended.
* All entrytype and field names are **case-folded** (`fc`); `casemap{foldtoorig}`
  remembers original case for output (`get_outcase`).
* Fatal check: every `datatype="date"` field name must end in `date`
  (`biber_error`, DataModel.pm:126-131).
* Lookups: `fieldsbyname{f} = {fieldtype, datatype, format||'default', nullok?, skipout?}`;
  `fieldsbytype{ft}{dt}{fmt|'*'}`, `fieldsbyfieldtype`, `fieldsbydatatype`,
  `fieldsbyformat`; later definitions override earlier ones (with a removal bug, B19).
  `entrytypesbyname{t} = {skipout?, legal_fields{f}, constraints{mandatory[], conditional[], data[]}}`
  where an `entryfields`/`constraints` block without `<entrytype>` applies to all types.
  Mandatory items are a field name or `['XOR', f...]` / `['OR', f...]`. Conditional:
  `[aq, [afields], cq, [cfields]]`. Data: `{fields, datatype, rangemin, rangemax, pattern}`.
* Helpers (sorted lists) used by output: namelists(all/non-skip), lists, fields
  (field-type with datatypes entrykey/key/integer/datepart/literal/code, not xsv, not skip),
  datefields, dateparts, xsv, ranges, uris, urils, verbs, vfields, vlists, integers.
  `sortdataschema(f)` = `int` for intciteorder/citeorder/citecount/integer+datepart fields,
  else `str`. All `get_fields_of_*` return **sorted** lists (determinism of .bbl output).
* Datatypes seen in biblatex 3.22 DM: code, date, datepart, entrykey, integer, key,
  keyword, literal, name, option, range, uri, verbatim (+ constraint datatypes isbn, issn,
  ismn, pattern, integer). Field formats: `xsv` only.
* `is_field` strips an annotation suffix (`<field><annotation_marker>[<named_marker><name>]`).
* `is_multiscript` is dead and broken (B20).
* Validation (`Biber::validate_datamodel`, Biber.pm:1702) only with `-V`:
  unknown entrytype → warn and **change to misc**; per field legality unless type is
  `xdata`/`set`; mandatory (XOR: extra fields **deleted** after the first found; date field
  ignored if `datesplit`), conditional (`none` consequent violations **delete** fields),
  datatype checks (`check_datatypes` — invalid fields **deleted**; nullok fields with `''`
  skipped; list/xsv fields validated as arrays), data constraints (isbn B21, issn, ismn,
  integer/datepart range with deletion B22, pattern via `imatch`, unanchored). Note the
  schema allows `datatype="date"` constraints but the code ignores them. Each message goes
  to `biber_warn` (or `biber_error` with `--dieondatamodel`, which doesn't actually die,
  B4). Messages format: `Datamodel: <type> entry '<key>' (<datasource>): ...`.
  Note validation **mutates** entries, so `-V` changes `.bbl` output.
* `generate_bltxml_schema` / `generate_bblxml_schema`: emit RelaxNG from the DM (XML::Writer,
  DATA_INDENT 2). Written next to the .bcf (`s/bcf$/rng/`) or, in tool mode, the input file
  with its extension replaced by `.rng` (B6). Low priority (bltxml/bblxml are niche).

Difficulty M, risk M (merge semantics; determinism of sorted helper lists is crucial for .bbl).

### 3.6 Sections, datalists, citekeys

* `Biber::Sections`: hash by number; `get_sections` sorted numerically (`<=>`).
* `Biber::Section` (Section.pm:22-62) holds entries (`Biber::Entries`), citekeys (ordered +
  hash), `orig_order_citekeys`, `undef_citekeys` (written to .bbl as `\missing`),
  cite/nocite sets, `everykey`/`everykey_lc` (bad-case key detection), citekey aliases
  (`IDS` field), dynamic sets (`dkeys`), set parent/child maps, related clones, citecounts
  (`get_citecount` returns `-1` for 0/undef), seenkeys, datasources, labelcaches,
  namepart lengths. `get_static_citekeys` excludes dynamic set keys. `has_citekey` resolves
  aliases. `del_citekey` deletes by the given string (if given an alias it removes nothing
  from the list though `has_citekey` was true).
* Allkeys (`*`): every entry in the section's data sources; `allkeys_nocite` remembers that
  it came from `\nocite{*}`. Keys `\cite`d explicitly retain cite/nocite info for sourcemap
  logic (`map_entrykey_citedonly`/`nocited` etc.).
* Dynamic sets (`\defbibentryset`): instantiated as entries of type `set` with
  `datatype=dynamic`; members' aliases resolved; set keyorder = minimum member order.
* Citekey aliases: entries' `IDS` register aliases; `process_citekey_aliases` prunes aliases
  from the citekey list (so `\cite{alias}` → real entry).
* `Biber::DataLists`: plain list; `get_lists_for_section`, `get_lists_by_attrs` (string
  `eq` on getters), `has_lists_of_type_for_section`, `get_list($name,$section,$type)`.
* `Biber::DataList` attributes: section, name, type (`entry`/`list`), the 5 template names,
  labelprefix, filters (array of `{type,value}` or arrays thereof for `filteror`),
  sortinit collator; `get_attrs` = `stn/snktn/prefix/untn/lantn/nhtn`.
  Filters (`check_list_filter`, Biber.pm:3190): type/nottype/subtype/notsubtype (compare
  lowercased filter value), keyword/notkeyword (`has_keyword`), field/notfield
  (`field_exists`). AND across filters, OR within `filteror`. After filtering, set members
  are re-added (B14). In `process_lists` empty template names fall back to `global`.
* Langtags: `Biber::LangTags` compiles a Parse::RecDescent BCP47 grammar at `Biber->new`
  (startup cost) but nothing in lib/ calls `->langtags->parse` — only t/langtags.t. Grammar
  bug `ALPHA: /[a-zA-z]/` (B25). Language/locale mapping used in production is
  `%LOCALE_MAP` (babel name → BCP47) / `%LOCALE_MAP_R` (reverse, has wrong/duplicate entries,
  B26) via `locale2bcp47`/`bcp472locale`.

### 3.7 Templates & declarations (config side summary)

| biblatex macro | .bcf element | stored as (option, scope) |
|---|---|---|
| `\DeclareSortingTemplate` | `sortingtemplate` | `sortingtemplate` GLOBAL hash name→{locale,spec} |
| `\DeclareSortingNamekeyTemplate` | `sortingnamekeytemplate` | GLOBAL name→{visibility,template} |
| `\DeclareUniquenameTemplate` | `uniquenametemplate` | GLOBAL name→[parts]; ≥1 base part required |
| `\DeclareLabelalphaTemplate` | `labelalphatemplate type=` | GLOBAL or ENTRYTYPE, raw |
| `\DeclareLabelalphaNameTemplate` | `labelalphanametemplate` | GLOBAL name→[parts] |
| `\DeclareNamehashTemplate` | `namehashtemplate` | GLOBAL name→[parts] |
| `\DeclareExtradate` | `extradatespec` | GLOBAL [[fields]] |
| `\DeclareDataInheritance` | `inheritance` | GLOBAL raw (defaults/type_pair/inherit/field, `ignore` → uniqignore) |
| `\DeclareSortTranslit` | `transliteration` | `translit` GLOBAL or ENTRYTYPE |
| `\DeclareNoinit` / `\DeclareNolabel` / `\DeclareNolabelwidthcount` | `noinits`/`nolabels`/`nolabelwidthcount(s)` | biber options (regex lists) |
| `\DeclareNosort` / `\DeclareNonamestring` | `nosorts`/`nonamestrings` | biber options `{name,value}` (name may be a datafieldset) |
| `\DeclarePresort` | `presort [type]` | GLOBAL / ENTRYTYPE |
| `\DeclareSortExclusion` / `Inclusion` | `sortexclusion`/`sortinclusion` | ENTRYTYPE |
| `\DeclareDatafieldSet` | `datafieldset` | `%DATAFIELD_SETS` |
| `\DeclareSourcemap` etc. | `sourcemap/maps[level]` | biber option `sourcemap` |
| `\DeclareDatamodel*` | `datamodel` | GLOBAL `datamodel` (array) |
| refcontext / `\printbiblist` / `\DeclareBiblistFilter` | `datalist` (+filter) | DataLists |

Note `.bcf` parsing uses `nolabelwidthcounts`/`nolabelwidthcount` element names
(Biber.pm:701) — bcf.rnc does not define them at all (schema gap, B24).

### 3.8 Output-encoding / safechars (config side)

* `output_encoding`/`input_encoding` come from the .bcf (`texencoding`/`bibencoding`,
  e.g. `utf8`) unless explicit; normalised to `UTF-8` by `normalise_utf8`. Encoding
  aliases defined in Constants.pm:357-367 (ansinew, applemac, applemacce, next, x-mac-*,
  x-nextstep, x-ascii, lutf8, utf8x). Any `Encode` name is accepted.
* `output_safechars` forced on when output encoding matches `/(?:x-)?ascii/i` and input
  doesn't (Biber.pm:1228-1232) — note the unanchored regex: any encoding *containing*
  "ascii" counts.
* `output_safecharsset`/`decodecharsset` (base|full|null|custom set in recodedata),
  `recodedata` file located via kpsewhich; `Biber::LaTeX::Recode->init_sets` in
  `Biber->new` *before* the .bcf is read (so these can only come from CLI/conf).
* The output file handle is re-opened after .bcf parsing to pick up the final encoding.

---
## 4. Rust design suggestions for this area

1. **CLI**: hand-rolled or clap-based parser with a compatibility layer: accept `-name` and
   `--name`, `_`/`-` spellings as listed in bin/biber (they are separate aliases, not
   normalised!), unique-prefix abbreviation over the *full alias list* (ambiguity error lists
   all candidates), `--opt=v`/`--opt v`, optional-value `-w`, counter `-q`, repeatable
   `-c k=v`, `--dot-include` comma lists, options after positionals. Messages for
   unknown/ambiguous options + usage on STDERR, exit 2. Decide explicitly whether to
   reproduce bugs (B27: `-e` unknown) — recommend *accepting* `-e` (strict superset).
2. **Options model**: `BiberOptions` struct (typed fields for every key in
   `$CONFIG_DEFAULT_BIBER`) + `provenance: {Default, ToolConf, UserConf, Cli, Bcf}` per
   field so `isexplicitoption` = provenance ∈ {UserConf, Cli}. Build it in one pure
   function `resolve(defaults, toolconf, userconf, cli, bcf) -> Options`. Keep
   `collate_options` as an ordered map merged key-wise.
3. **Biblatex options**: `OptionRegistry` built from `<optionscope>` (name → {scopes, datatype,
   backendin, backendout}); `BlxOptions { global: Map, by_type: Map<Type, Map>, by_entry:
   Map<(Section, Key), Map> }` storing `OptValue` enum. Setting an undeclared option must be
   a silent no-op (compat). Boolean mapping exactly like `map_boolean` (`true/false` →
   1/0 case-insensitively, numbers kept, other strings → None). Name-list/name scoped
   options: `SmallVec<(OptId, OptValue)>` on the name objects instead of dynamic accessors.
4. **.bcf model**: typed structs (quick-xml + serde, namespace-stripped). Preserve:
   document order within repeated elements; `order`-attribute sorting (stable sort, numeric
   compare — Perl `<=>` on strings like "1","10"); merging of repeated `<section number>`;
   NFD of everything except datasource names; latin1 fallback + rewrite-as-UTF-8 (side
   effect on the user's .bcf!); exact version string check. Keep raw subtrees where biber
   keeps them raw (inheritance, labelalphatemplate, translit, sourcemap maps) but give them
   typed forms — other modules consume them.
5. **Templates**: intern template names; resolve datalist → template references once
   (biber looks them up by name repeatedly in hot loops).
6. **DataModel**: immutable struct built once; case-fold with Unicode `fc` (use
   `caseless`/`unicase` full folding, not ASCII lowercasing); pre-sorted helper vectors
   (sort by Perl string order = byte order of UTF-8 for these ASCII names). Use interned
   field IDs (u16) and bitsets for `legal_fields` per entry type — this is where the speed
   win is (biber does hash lookups of strings everywhere).
7. **Logging**: `Logger { blg: BufWriter, stdout_level, file_level, deferred_warnings:
   Vec<String>, errors: u32 }`; format `[{ms}] {file}:{line}> {LEVEL} - {msg}` for the file
   (use a fake but stable `biber.rs:0` or the Rust module path), `{LEVEL} - {msg}` on stdout.
   Flush deferred warnings, `WARNINGS:`/`ERRORS:` at end exactly as display_end. NFC
   normalise warnings on output (Biber.pm:131).
8. **Error model**: `biber_error` semantics = log + count + exit(2) unless nodieonerror →
   in Rust return `Result` with a `Fatal` variant that main turns into display_end + exit 2.
   With `--nodieonerror`, continue (but don't reproduce the cascading garbage of B10).
9. **Output file lifecycle**: open+truncate the output early (before .bcf parse) to match
   observable behaviour (empty .bbl on early exit), unlink on malformed .bcf. Consider
   *not* creating the stray `-` file (B8).
10. **Sections**: `Vec<Section>` sorted by number; per-section state struct replacing
    `Config->_init`; data sources parsed once into a shared immutable cache keyed by
    (path, datatype, encoding) then cloned/filtered per section → parallel sections with
    rayon, deterministic merge of warnings and output.
11. **Skip** the Parse::RecDescent LangTags parser at startup (unused) — measurable startup
    saving in Perl; in Rust just don't build it (or use `language-tags` crate if needed).
12. **Remote data sources** & `kpsewhich` lookups: keep as external process call
    (`kpsewhich` is invoked for the .bcf *and* every datasource not found locally, and for
    biber.conf when no other conf exists — 1+ process spawn per run; cache results).

## 5. Test oracle notes

* The installed 2.22 binary == vendored source (except version string), so `.blg`
  `File.pm:line` references are directly mappable when diagnosing differences.
* Normalise `.blg` before comparing: strip `^\[\d+\] \S+:\d+> `, drop the `=== <date>`
  line, drop `This is Biber 2.22`, temp-dir paths (`/var/folders/.../par-...`), and the
  `Config file is` line unless testing config discovery.
* **Warning order is nondeterministic** in biber (Perl hash randomisation): with
  `-V` the relative order of WARN lines changed between identical runs (confirmed on
  `t3`). Compare WARN lines as a multiset; compare counts (`WARNINGS: n`) exactly.
  The `.bbl` itself was byte-stable across runs (md5 identical ×3).
* Run oracle biber with `--noconf` (or a controlled `HOME`/cwd) — otherwise a user's
  `biber.conf` or `kpsewhich biber.conf` changes results.
* `--trace` dumps the full `$CONFIG` and Biber object (Data::Dump) into the .blg: a cheap
  way to observe resolved options/templates/DM for differential testing of the config
  layer (search for "Dump of initial config object"). Note `--trace`/`--debug` also make
  btparse print `warning: overriding existing definition of macro "jan"` lines on STDERR.
* Useful t/*.t that pin config behaviour: `configfile.t` (precedence, sourcemap merge —
  note its `noinit` assertion only passes because the config's `<noinits>` is ignored, B1),
  `options.t` (per-entry/per-type options, `dataonly` meta-option), `bcfvalidation.t`
  (bcf against rng), `tool-config.t` / `tool.t` / `tool-bltxml*.t`, `sections.t`,
  `sections-complex.t`, `datalists.t`, `dm-constraints.t`, `langtags.t`, `set-dynamic.t`.
  t/tdata/*.bcf are older-format fixtures (check their `version`): they make good
  parser regression inputs.
* Generate fresh .bcf files with pdflatex+biblatex 3.22 (BCF 3.11) for every biblatex
  feature (`\DeclareSortingTemplate`, refcontexts, `\printbiblist` filters,
  `\defbibentryset`, `\nocite{*}`, multiple refsections, per-type options via
  `\ExecuteBibliographyOptions[type]`, `\DeclareDatamodel*` in a .dbx) and diff .bbl/.blg.
* Edge cases to include: missing .bcf; malformed .bcf (truncated) — .bbl must be deleted;
  latin1 .bcf (gets rewritten!); bcf version mismatch; no `\addbibresource`; refsection
  with no resources (crash B11); duplicate datalists (B12); `-O -`; `--output-directory`;
  `--input-directory`; bibtex output format; tool mode with every output format.

## 6. Observed behaviours worth pinning (non-bugs)

* `Found 0 citekeys in bib section 0` appears once per extra `<bcf:section number="0">`.
* `.bbl` sections are written in string-sorted section-number order.
* Empty `.bbl` (0 bytes) is left when biber exits early after opening output.
* All WARNs appear at the end of the .blg/stdout, ERRORs inline.
* `-V` mutates data (unknown types → misc, invalid fields removed), changing the .bbl.
* `--sortlocale` (biber option) overrides per-template locales.
* In normal mode a `<datamodel>` in biber.conf is discarded (the .bcf DM replaces it);
  it only matters in tool mode.
* Log goes to **stdout** (not stderr); option-validation messages from bin/biber go to
  STDERR prefixed `Biber: `.

---

## 7. Suspected biber bugs

Status: **CONFIRMED** = reproduced with biber 2.22 in scratch/config; **reading** = from
source only.

| # | Location | Description | Status |
|---|---|---|---|
| B1 | Config.pm:557-561, 363-403; config.rnc:62-90; doc/biber.tex:658-667,1356-1437,1818 | biber.conf handling of noinit/nolabel/nolabelwidthcount/nonamestring is broken three ways: (a) the **documented** form `<noinit><option value=".."/></noinit>` (and same for `nolabel`) **crashes** with "Not a HASH reference at Config.pm line 560" because `noinit`/`nolabel` are in the ForceArray list so `$v` is an array; (b) the **schema** form `<noinits><noinit value/></noinits>` / `<nolabels>` is silently ignored (key not handled); (c) `nolabelwidthcount` and `nonamestring` (documented) are silently ignored (not in the special-case list, no `content`). t/configfile.t "Options 6" only passes because the test conf's `<noinits>` (which contains a deliberately broken regex) is ignored and defaults are returned. Repro: `printf '<config><nolabel><option value="x"/></nolabel></config>' > c.conf; biber -g c.conf doc` | CONFIRMED (all four keys) |
| B2 | Config.pm:363-403, 537-548 | Config file with a single `<sortinclusion>` crashes ("Not an ARRAY reference"): `sortinclusion`/`inclusion` missing from config ForceArray (sortexclusion/exclusion are present). | CONFIRMED |
| B3 | Config.pm:125-127 | `--configfile` pointing to a non-existent file is silently ignored and the default search is used instead; no warning. | CONFIRMED |
| B4 | Biber.pm:1710 + Utils.pm:478 | `--dieondatamodel` is documented to "exit immediately", but `$dmwe->($msg, $be)` passes the entry as `biber_error`'s `$nodie` argument, so it never dies: all violations are logged as ERROR, the .bbl is written, exit 2 at the end. | CONFIRMED |
| B5 | bin/biber:368 | Non-tool `--output-format=bibtex` default name uses `s/\..+$/_biber.bib/` (first dot in the whole path): `biber --output-format=bibtex ./t1` writes `_biber.bib`; `biber --output-format=bibtex sub.dir/t1` writes `sub_biber.bib` in the cwd. | CONFIRMED |
| B6a | Biber.pm:4631-4634 | Tool mode with biblatexml input whose name doesn't end in .bbl/.bblxml/.bib/.bltxml: schema outfile = `dsn =~ s/\.(?:exts)$/.rng/r` leaves the name unchanged, so the generated RNG **overwrites the user's data file** before it is read. Repro: `cp x.bltxml data.xml; biber --tool --input-format=biblatexml data.xml` → data.xml now contains `<grammar ...>`. | CONFIRMED (data loss) |
| B6b | bin/biber:356-364 | Tool mode on an input file without extension: `s/\.[^.]+$/_bibertool.bib/` doesn't match, so the output file **is the input file** (overwritten). Repro: `cp refs.bib refsnoext; biber --tool refsnoext`. | CONFIRMED (data loss) |
| B7 | bin/biber:389-392 | `-O -` combined with `--output-directory D` writes a regular file `D/-` instead of stdout (the stdout check in bbl.pm compares the rewritten path). | CONFIRMED |
| B8 | Output/base.pm:56-73, bin/biber:398 | `-O -` (stdout) still creates an empty file literally named `-` in the cwd (early `IO::File->new('-', '>')`). | CONFIRMED |
| B9 | Biber.pm:894-898 (`biber_warn` at :896) | "No data sources defined!" is `biber_warn`ed and then `exit 0` without `display_end`, so the warning is never logged/printed; the output .bbl has already been truncated to 0 bytes. | CONFIRMED |
| B10 | Biber.pm:348-351, Utils.pm:368 | Missing .bcf: `locate_data_file` already errors with `Cannot find 'x.bcf'!`, so the friendlier `Cannot find control file ...` (which latexmk special-cases → return code 6 "missing control file") is unreachable. With `--nodieonerror` the error cascade continues with the control-file path being the string `1` (return value of biber_error) and ends in a libxml crash. | CONFIRMED |
| B11 | Biber.pm:885-896, 4605-4619 | If any section has data sources but a section with citations has none (e.g. `\cite` in section 0, resources only on `\begin{refsection}[refs.bib]`), biber crashes: "Can't use an undefined value as an ARRAY reference at Biber.pm line 4607" (exit 102, no ERROR in .blg, empty .bbl). | CONFIRMED |
| B12 | Biber.pm:1032-1044 vs DataLists.pm `get_list($name,$section,$type)` | Datalist de-duplication calls `get_list` with named arguments; the positional sub receives name='section', type='name', so it never matches. Duplicate `<bcf:datalist>`s produce duplicate `\datalist` blocks in the .bbl (biblatex currently de-dups before writing the .bcf, so latent). | CONFIRMED (hand-edited .bcf) |
| B13 | Biber.pm:613-640 (else-branch :636) | If biber.conf has user sourcemaps and the .bcf `<sourcemap>` has no driver/style-level maps, the else-branch replaces everything with the .bcf maps, silently dropping all biber.conf maps (the doc maps were just merged into the discarded config list). | CONFIRMED (hand-edited .bcf) |
| B14 | Biber.pm:3155-3162 | After list filtering, `first {$be->get_field('entryset')} @sets` ignores `$_`: if *any* set passes the filter, **every** entry with an `entryset` field (all sets and all set members) is re-added. Repro: two `@set`s, one with `keywords={kw}`, `\DeclareBiblistFilter{kwlist}{\filter[type=keyword,filter=kw]}` → `setB`, `b1`, `b2` appear in the list. | CONFIRMED |
| B15 | Biber.pm:743-760 | The "uniquenametemplate must contain at least one base part" check is global (`$checkbase` across all templates): a template with no `base` passes if another template has one, risking the infinite loop the check guards against. | reading |
| B16 | Config.pm:466-481 | biber.conf `labelalphanametemplate`: `$lants` is re-initialised per template and `setblxoption` called each time, so only the last template survives (the .bcf version accumulates correctly). Impact limited to tool mode. | reading |
| B17 | Config.pm:153-158 | `%CONFIG_DEFAULT_BIBLATEX` is applied *after* the user config file, overwriting biber.conf values for maxbibnames/maxsortnames/minsortnames/useprefix/useauthor/... Matters in tool mode (sorting uses maxsortnames/useprefix). Repro: tool mode conf `<maxbibnames>7</maxbibnames><useprefix>1</useprefix>` → `--trace` dump shows biblatex GLOBAL 100 / 0. | CONFIRMED |
| B18 | DataModel.pm:66-95 | User DM merge: (a) user constraints are appended to `constraints[0]` only, losing their own `<entrytype>` scoping (they inherit whatever types default block 0 lists — in biblatex's DM that's the date/year XOR block for 25 types) and later user constraint blocks are ignored; (b) user entryfields blocks are matched only on their *first* entrytype against default blocks' *first* entrytype, and fields get appended to the default block (thus applying to that block's types, not the user's); a user general-fields block is dropped if the default has none. | reading |
| B19 | DataModel.pm:146-151 | On field redefinition, removal from `fieldsbytype{..}{format}` is keyed on the *new* field's format (`if ($f->{format})`) rather than the previous one's; a field redefined from xsv to plain stays in the xsv list. | reading |
| B20 | DataModel.pm:136-138, 404-407 | `is_multiscript` is broken twice: keys are stringified hash refs (`{content=>..}` under ForceContent) and `my ($self,$field) = shift` leaves `$field` undef. Currently dead code. | reading |
| B21 | Constants.pm:166-182 | The `isbn` data constraint only fails when `Business::ISBN->new` returns undef; invalid checksums pass. Meanwhile another code path warns "ISBN '0-201-13447-1' ... is invalid - run biber with '--validate_datamodel' for details", but -V gives no details for it. `issn`/`ismn` validators read `$_` instead of their argument (works only because the caller loops with `foreach ($fv->@*)`). | CONFIRMED (isbn) / reading ($_) |
| B22 | DataModel.pm:932-950 | Integer/datepart range constraints: `if (my $fmin = $c->{rangemin})` ignores rangemin/rangemax of `0`; `if (my $fv = ...)` skips value `0`; non-numeric values compared numerically. | reading |
| B23 | Config.pm:647-651 vs bin/biber POD & doc | Windows search path is `%APPDATA%\biber\biber.conf` in code but documented as `%APPDATA%\biber.conf`. | reading |
| B24 | data/schemata/config.rnc, bcf.rnc, doc/biber.tex:640-705 | Schema/doc/code drift: config.rnc uses `noinits/nolabels` (code wants `noinit/nolabel`, doc too); no `nolabelwidthcount` in either schema although code reads `nolabelwidthcounts` from the .bcf; config.rnc `output_format` lacks bblxml/bibjson; `wraplines` typed `0|1` though it's a column; bcf.rnc allows `datatype="date"` data constraints which the code ignores, and the code accepts `datepart` which the schema forbids; the doc's sample biber.conf is not well-formed (`<ouput_align>0</output_align>`) and uses `<graph>`, `<tool>false</tool>`, `<sortcase>true</sortcase>` (config.rnc only allows 0/1). | reading |
| B25 | LangTags.pm grammar `ALPHA: /[a-zA-z]/` | Range `A-z` includes `[\]^_\``, so e.g. `en_US` parses as a 5-letter language subtag. Parser is unused outside tests (but compiled at every startup). | reading |
| B26 | Constants.pm `%LOCALE_MAP_R` | `'sr-Latn' => 'latin'` (should be serbian; `la`/`la-Latn` missing), duplicate keys `en-US`×3, `fr-CA`×2 (last wins), `en-UK` invalid region. `bcp472locale` is exported but unused in lib/. | reading |
| B27 | bin/biber:52 | `--input-encoding` alias is misspelt `input-encodinge`; it only works via prefix abbreviation, and the documented short option `-e` doesn't exist ("Unknown option: e", exit 2). | CONFIRMED |
| B28 | Biber.pm:1316-1334 | In `resolve_alias_refs`, `$resolved_keys` is declared outside the loop over XDATA refs, so a second whole-entry XDATA ref would accumulate the first ref's keys. | reading (latent) |
| B29 | Biber.pm:4288-4290, bin/biber:46 | `--fastsort` documented as "does nothing now", but emits `WARN - fastsort option no longer required/supported` once per sorted list, increasing the warning count (latexmk reports "Biber warnings"). | CONFIRMED |
| B30 | bin/biber:375-378 | Non-tool `--output-format=biblatexml|bibjson` passes the early format check, then fails after logging starts with a message on **stdout** (not STDERR, not in .blg as ERROR). Inconsistent with other validation. | CONFIRMED (minor) |
| B31 | Biber.pm:1178-1188 | `process_setup` deletes non-99999 sections inside the loop over `get_sections` (safe only because get_sections returns a copy) — fine; but the bibtex pseudo-section adds all sections' datasources without de-duplication, so a .bib used by N refsections is read N times for section 99999. | reading |

Lower-confidence observations (not listed as bugs): `Config::_init` resets
`state.control_file_location`, a key never used (the real one is top-level); unanchored
`/(?:x-)?ascii/` encoding test; `has_dynamic_sets` always true; `get_citecount` maps 0 to -1
(intentional per comment).

## 8. Open questions

1. Should the Rust port reproduce data-destroying tool-mode bugs B6a/B6b? Recommend no
   (refuse/rename with a warning) — nobody can depend on them.
2. B1/B2/B17/B13: fix (accept both documented and schema forms) or replicate? Accepting the
   documented forms is a strict improvement but changes output for users whose configs are
   currently ignored. Needs a policy decision ("compatible with 2.22 output" vs
   "compatible with 2.22 documentation").
3. Is byte-identical `.blg` required (beyond the latexmk-parsed patterns)? The `File:line`
   tags are Perl-specific; recommend normalised comparison only.
4. Warning order: biber's is partly nondeterministic; should the Rust port pick a
   deterministic order (e.g. data source order) — yes, but tests must use multiset compare.
5. Does any downstream tool depend on the empty-`.bbl` / stray `-` file behaviours? Probably
   not; recommend keeping the early truncate (biblatex sees an empty .bbl after failures,
   which is the current UX) and dropping the stray `-` file.
6. Should `--validate-control`/`--validate-config`/bltxml schema generation be supported in
   v1? They need a RelaxNG validator (no mature Rust crate; could shell out to `xmllint` or
   defer). Rarely used.
7. Remote (http/https/ftp) data sources and remote `.bcf` — keep (needs an HTTP client) or
   drop for v1?
8. `--convert-control` (XSLT) — almost certainly drop or shell out to `xsltproc`.
9. Section parallelism interacts with Text::BibTeX's global macro table semantics — need the
   input agent's findings on whether `@STRING` definitions leak between files/sections in
   observable ways before choosing a shared-parse cache design.

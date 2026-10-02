# 03 — Core entry-processing semantics (biber 2.22)

Scope: everything in `lib/Biber.pm` from `fetch_data` through `process_entries_final` /
`generate_contextdata`, plus the helpers it relies on in `lib/Biber/Entry.pm`,
`lib/Biber/Internals.pm` (labels and hashes), `lib/Biber/DataList.pm` (per-list state),
`lib/Biber/Section.pm`, `lib/Biber/Entry/Names.pm`/`Name.pm` and `lib/Biber/Config.pm` (option
lookup and inheritance state). Collation, sort keys and the `.bbl` writer are covered elsewhere.
They appear here only where they interact with this area.

All line numbers refer to `vendor/biber` (HEAD ≈ v2.22). The experiments are in
`scratch/semantics/<name>/` (`t.tex` + `t.bib`; `run.sh <dir>` runs pdflatex once to produce the
`.bcf` and then runs biber 2.22). The perf experiments are in `scratch/semantics/perf/`.

---

## 1. Overview

For each refsection, biber runs a fixed pipeline (`prepare`, Biber.pm:4474-4512):

```
reset_caches; Config->_init; preprocess_options
fetch_data            -> entries for cited keys + transitive dependents (xref/crossref/xdata/sets/related/aliases)
process_citekey_aliases
instantiate_dynamic   -> dynamic set entries, related-entry clones
resolve_alias_refs    -> xref/crossref/xdata values rewritten alias->real key
resolve_xdata         -> XDATA (whole-entry and granular) copied into entries
cite_setmembers       -> set members promoted to citekeys, set parent inherits 1st member data
preprocess_sets       -> parent<->child set maps, relclones for static set members
calculate_interentry  -> crossref/xref counts, mincrossrefs/minxrefs promotion
process_interentry    -> crossref inheritance (DeclareDataInheritance)
validate_datamodel
postprocess_sets      -> dataonly-style options forced on set members, \inset
process_entries_static-> nocite flag, labelname/labeldate/labeltitle selection, presort
process_lists         -> for each datalist (refcontext): see §2
create_output_section
```

The two most important structural facts for a port:

1. **Entry data is mutated in place.** XDATA, crossref and set inheritance, alias resolution,
   option forcing and label* selection all write directly into the `Biber::Entry` objects. They
   also write process-global state in `Biber::Config` (per-entry options, inheritance records,
   uniqueness-ignore records) and section state. `Biber::Config->_init` resets only part of that
   global state per section (Config.pm:96-106).
2. **Everything that depends on name visibility, templates or sort order is per-datalist.**
   It lives in `DataList->{state}`, keyed by *namelist id* / *name id* / citekey, and is
   recomputed for every list in the section. That includes uniquename/uniquelist, hashes,
   labelalpha and the extra* fields. Names objects are shared by reference between entries in
   several cases (inheritance, XDATA), and that sharing has observable consequences (Bug B10).

---

## 2. Pipeline and data flow (per section, per list)

### 2.1 Per-section, entry-level (list-independent)

| Step | Code | Iterates | Writes |
|---|---|---|---|
| fetch_data | Biber.pm:4574-4713 | datasources × wanted keys | `section->bibentries`, `citekeys` (only for *cited* keys and cited aliases), `undef_citekeys` |
| get_dependents (recursive) | 4720-4885 | keys of current frontier | more bibentries (NOT citekeys), `relkeys` set, removal of missing deps |
| process_citekey_aliases | 1344-1357 | citekeys | deletes alias keys from citekeys |
| instantiate_dynamic | 1368-1425 | `dynamic_set_keys` (**hash order**), then citekeys | dynamic set Entry objects; keyorder of a set = min keyorder of its members; related clones (`relclone`), which are appended to citekeys |
| resolve_alias_refs | 1286-1336 | citekeys | xref/crossref/xdata refs rewritten |
| resolve_xdata | 1433-1455 | **all** bibentries (`values %entries`, hash order) except @xdata | field data copied in |
| cite_setmembers | 1467-1519 | citekeys | members appended to citekeys; `set_inherit_from(first member)` |
| preprocess_sets | 1529-1567 | citekeys | `state.set.pc/cp`; relclones of set members |
| calculate_interentry | 1577-1647 | citekeys | `Config.state.crossrefkeys/xrefkeys` counts; promoted keys appended (**hash order**) with `crossrefsource`/`xrefsource` |
| process_interentry | 1654-1689 | citekeys | `inherit_from` (crossref only; xref never inherits) |
| validate_datamodel | 1702-1762 | citekeys | warnings; unknown entrytype → `misc` |
| postprocess_sets | 1946-1957 → process_sets 2487-2523 | citekeys | per-entry options `skipbib,skiplab,skipbiblist,uniquename=false,uniquelist=false` on members; member `entryset=[parent]` |
| process_entries_static | 1966-1991 | citekeys | `nocite`, labelname info (+ fullhash variant), labeldate fields (`labelyear`, `labelmonth`, …, `labeldatesource`), labeltitle, per-entry presort option |

### 2.2 Per datalist (`process_lists`, Biber.pm:3085-3183)

For each datalist of the section (bcf order, plus a default `entry` list if none exists,
process_setup 1196-1214):

1. `reset_state`; set `keys = section citekeys` (all of them, before filtering).
2. `set_unul_changed(1)`; `process_entries_pre`: `process_namedis` for every citekey → namestrings
   and disambiguation schema per name (only stored if uniquename or uniquelist is not `false`).
3. `uniqueness` (iterative uniquename/uniquelist, §3.6).
4. `process_visible_names` (cite/bib/sort/alpha visibility per namelist id).
5. `process_entries_post`, per citekey in order: fullhash(+raw), labelalpha, extraalpha *counts*,
   extradate/extraname/extratitle/extratitleyear *counts*, work-uniqueness counts, namehash and
   bibnamehash, per-name hashes, uniqueprimaryauthor counts.
6. `process_entries_final`: singletitle, uniquetitle, uniquebaretitle, uniquework,
   uniqueprimaryauthor flags (from the counts).
7. **Filter** (`check_list_filter`) → list keys replaced with the filtered keys.
8. Sort (`generate_sortdataschema`, `generate_sortinfo`, `sort_list`, which belongs to the sorting
   agent).
9. `generate_contextdata`, in **sorted list order**: sorted `entryset`, extraname/extradate/
   extratitle/extratitleyear/extraalpha *numbers*, labelalpha copy, per-namepart uniquename values.

Per-list results live in `DataList->{state}`:
`namelistdata{nlid}{nid}{...}` (namestring(s), schema, un, unall, unparts, hash, ul, vis*),
`fields{citekey}{...}` (labelalpha, sortlabelalpha, fullhash, namehash, labelnamehash,
nametitledateparts, …), counters `uniquenamecount*`, `uniquelistcount`, `seen_*`,
`ladisambiguation`, and `seenname/seentitle/seenbaretitle/seenwork/seenpa`.

### 2.3 Granularity cheat-sheet

* **Per section:** fetched data, aliases, sets, related clones, inheritance (XDATA/crossref),
  mincrossrefs promotion, labelname/labeldate/labeltitle selection, `extradatescope`, nocite flag.
  Section caches `labelcache_v/l` (labelalpha `v`/`l` widths) are *also* per section, not per list
  (Bug B13).
* **Per datalist:** namedis strings (uniquenametemplate can be per list), uniquename/uniquelist,
  visibility, all hashes (namehashtemplate per list; visibility via uniquelist), labelalpha
  (labelalphanametemplate per list), extra* counts and numbers, singletitle and the other
  uniqueness flags, sortinit.
* **Per entry:** options (`Config.options.biblatex.ENTRY{key}{secnum}`), uniq-ignore records,
  inheritance records.

### 2.4 Which loops run over what

Every per-list processing step iterates over **all section citekeys**, not the list's
(filtered) keys. Only filtering, sorting and `generate_contextdata` use the list keys. As a
result, uniqueness, extra* counts and the singletitle counts include entries that the list
later filters out. Extra* *numbers* are assigned only to entries that remain in the list.
So in a filtered list, a lone entry can get `extradate=1` because a filtered-out entry shares its
name and year. That follows from the design ("filter before sorting so no gaps", 3135), but it
is not documented.

---

## 3. Feature inventory

Ratings: **D** = porting difficulty (L/M/H), **R** = risk of not matching output (L/M/H).

### 3.1 Data fetching and dependency resolution — D:M R:M

* `fetch_data` (4574): only static keys (`get_static_citekeys` = citekeys minus dynamic set keys)
  are looked up. Datasources are searched in order; the first source that contains a key wins.
  Keys that are still missing are removed from citekeys, warned about, and added to
  `undef_citekeys` (written to the bbl as `\missing`).
* In `\nocite{*}` (allkeys) mode, every entry of every source is created. `create_entry` runs in
  **`each %hash` order** (bibtex.pm:198). Citekeys are added in file order
  (`orig_key_order`), followed by dynamic set keys (**hash order**, bibtex.pm:229-234).
* Aliases (`ids` field): the T::B cache maps alias→key. When a cited key is an alias, the real
  entry is created and the real key is added to citekeys (bibtex.pm:268-291). The alias is then
  removed from citekeys by `process_citekey_aliases`. `Section::bibentry` and `has_citekey`
  resolve aliases transparently (Section.pm `bibentry`, `has_citekey`).
* `get_dependents` (4720): processes the frontier and gathers new deps from:
  alias→real; dynamic set members that are not yet instantiated; `xdata` refs (whole-entry and
  granular); `xref`; `crossref`; static `entryset` members not already in citekeys; `related`
  keys (recorded in `relkeys`). Deps are de-duplicated with `uniq` (order kept). In allkeys mode
  only the section is checked; otherwise the deps are fetched from the datasources.
  Recursion continues until the frontier is empty. Afterwards, for every key "with deps", each
  missing key is removed (`remove_undef_dependent`, 4894). That deletes `xref`/`crossref`, filters
  the `xdata` list, filters `entryset` and `related` (and drops `relatedtype`/`relatedstring` when
  `related` becomes empty) and filters dynamic set members. The removal happens at the level
  where the key was found missing. `keyswithdeps` is shared by all recursion levels, so the cost
  is O(|keyswithdeps| × |missing|) per level, and the
  `first {$citekey eq $_} @keyswithdeps` membership tests are O(n²) overall.
  * The xdata branch of `remove_undef_dependent` warns only on a match, but it always rewrites
    `xdata` (4945-4955). That is harmless. Granular `xdata=key-field` refs inside other fields are
    not pruned here; `resolve_xdata` warns about them instead ("references XDATA entry … which
    does not exist").
* Dependents are **not** citekeys. They become cited only through: set membership
  (`cite_setmembers`), related clones (`relclone` → `add_citekeys`), mincrossrefs/minxrefs
  promotion, or aliases.

### 3.2 Related entries (`related`, `relatedtype`, `relatedoptions`) — D:M R:M

`Entry::relclone` (Entry.pm:63-151). For each related key (alias-resolved):
* If a clone for that key already exists in this section (`keytorelclone`), it is reused.
* Otherwise the entry is cloned with key `md5_hex(encode_utf8(relkey))`. The key is bit-exact
  and has no NFC step. The clone's options are:
  * when `relatedoptions` is present, those options, plus `skipbib,skipbiblist` if the related key
    is itself cited (via `merge_entry_options`, Utils.pm:1312);
  * otherwise `merge_entry_options([skipbib,skiplab,skipbiblist,uniquename=false,uniquelist=false],
    relentry.options)`, where the entry's own options override.

  The options are set as per-entry options **and** written into the clone's `options` datafield.
  The function then recurses into the clone (cascading related entries).
* After the loop, all clone keys are added to citekeys, and the parent's `related` field is
  rewritten to hold the clone keys.
* `clone()` (Entry.pm:159-207) copies datafields and origfields (shallow: values are shared
  references, including Names objects), the xdatarefs array (shared), selected derived date
  metadata, entrytype, datatype and annotations. It sets `citekey` and `clonesourcekey`. The
  derived-field list contains `yeardivision` twice and omits `endyeardivision` (Entry.pm:179), so
  clones lose `endyeardivision` (Bug B12).
* Call sites: dynamic-set members (in dynamic set **hash order**), then every citekey
  (instantiate_dynamic); then static set members (preprocess_sets). Clone creation order
  determines the order in which clone keys are appended to citekeys.

### 3.3 Sets — D:M R:M

* **Static** `@set{…, entryset={a,b}}`: members are fetched as dependents. `cite_setmembers`
  (1467) does the following for cited set entries with a non-empty entryset: resolves aliases;
  appends members to citekeys (set order); runs `set_inherit_from(first member)` (Entry.pm:674),
  which copies all of the first member's datafields that the set lacks, except
  `annotation`/`shorthand`, plus `*datesplit`; and deletes `crossref` from the set with a warning
  (the legacy style).
* **Dynamic** (`\defbibentryset{S}{a,b}` → bcf citekey `type="set" members="a,b"`):
  `instantiate_dynamic` builds an Entry `{entrytype=set, entryset=[…], citekey=S,
  datatype=dynamic}`. Its keyorder is the minimum keyorder among its members, set only if that
  minimum is truthy (`if ($minorder)`, so an order of 0 is ignored).
* `preprocess_sets` records parent↔child maps. `process_sets` forces
  `skipbib,skiplab,skipbiblist,uniquename=false,uniquelist=false` on members and sets each
  member's `entryset=[parent]` (rendered as `\inset`). Children are iterated with `keys`, so the
  order is random but does not matter. When an entry belongs to several sets, the last-processed
  parent wins (parents are processed in citekey order). A legacy `entryset` on a member is
  deleted with a warning. The block `unless (@entrysetkeys)` at 2517 is dead code.
* `generate_contextdata` with `sortsets`: the set's `entryset` for this list is reordered by the
  members' positions in the sorted list (`first_index`, O(n) per member). If a member were absent
  from the list, `first_index` would return -1 and the slice would pick the **last key of the
  list**. That is unreachable in practice because of Bug B2 (all set members are re-added). It is
  a latent bug.
* Set members get `skiplab` and so take no part in extra*. Their uniquename/uniquelist is
  `false`.

### 3.4 XDATA — D:H R:H

* Refs are recorded at parse time (`add_xdata_ref`, Entry.pm:225-270). There are two kinds:
  * whole-entry `xdata={k1,k2}`;
  * granular `field = {xdata=KEY-FIELD[-N]}` (marker, separators and position are configurable:
    `xdatamarker`, `xnamesep`, `xdatasep`). In name lists the placeholder is an empty Name with
    an `xdata` attribute; in other lists it is the raw string.
* `resolve_xdata` (Biber.pm:1433) visits **every** instantiated entry except @xdata ones. For each
  ref it does the following (Entry.pm:714-921):
  * warn and skip if the target is missing or is not an @xdata entry;
  * record `set_inheritance('xdata', xdref, entry)`; detect cycles with
    `is_inheritance_path('xdata', entry, xdref)` (fatal `biber_error`);
  * **recursively resolve the XDATA entry's own refs** — every time it is referenced (Bug B9);
  * whole-entry ref: copy every datafield except `ids` (`set_datafield`, so it overwrites
    existing fields; later refs win; values are shared by reference);
  * granular ref: types (fieldtype + datatype) must match. Name lists: `*` splices the whole
    source list in place of the placeholder (`Names::splice`); `N` replaces the placeholder with
    the Nth name (`replace_name`). Non-name lists use `splice` or index replacement. Scalars are
    replaced. Annotations are remapped.
* Name objects are **shared** between the XDATA entry and every target, and so are their `id`s.
  `Names::splice` re-indexes **0-based** and skips the last element (`$i<$#…`, Names.pm:106);
  `replace_name` also sets a 0-based index (Names.pm:90). `add_name` uses 1-based indices
  (Names.pm `add_name`). Visibility tests in uniquename (`$n->get_index <= $mincn`/`<= $ul`)
  therefore change (Bug B8, confirmed).

### 3.5 Crossref / xref / data inheritance — D:H R:H

* **Counting** (`calculate_interentry`): for each citekey, `crossref` increments the target count
  if the target exists, and `xref` increments the target count unconditionally (missing xrefs
  were already deleted). Targets with count ≥ `mincrossrefs` (default 2) or ≥ `minxrefs` are
  appended to citekeys **in Perl hash order** (`keys %crossrefkeys`, Config.pm:1254-1270). They
  get `crossrefsource`/`xrefsource=1` unless they are already cited. Only one level cascades,
  because counting happens before promotion: a grandparent referenced only by promoted parents
  is never promoted. Related clones are citekeys, so their crossrefs count too.
* **Inheritance** (`process_interentry` → `Entry::inherit_from`, Entry.pm:931-1125). Only
  citekeys are processed, in order; an entry is skipped if `get_inheritance(crossref, parent,
  child)` already exists. Steps:
  1. `set_inheritance(crossref, parent, child)`; cycle check with `is_inheritance_path`
     (fatal); **cascade**: if the parent has a crossref, run
     `parent->inherit_from(grandparent)` first. This repeats every time any child inherits from
     that parent.
  2. Defaults: `inheritance.defaults` (`inherit_all`, `override_target`, `ignore`), overridden by
     every matching `type_pair` in bcf order (the last match wins; `*` wildcards).
  3. Explicit `\inherit` rules: for each matching type pair, for each field rule
     `{source, target, skip, override_target}`. Rules skip targets in the entry's
     `noinherit` datafield set. A parent field that matched any rule is marked processed, even
     when it was not copied (when skipped, or when the target already exists). The field is
     copied when the target is absent or the rule's `override_target eq 'true'`. The rule's
     `ignore` (or the default `ignore`) is recorded as uniq-ignore (§3.11).
  4. When `inherit_all eq 'true'`: all parent datafields (sorted, `use locale`) not yet
     processed and not in `noinherit`, copied if absent or if `override_target`. There is a
     special case for dateparts when *both* parent and child have `datesplit`: if the child has
     any part of that date, the parent's parts are skipped (or the child's parts are deleted when
     overriding). Derived date metadata (`*dateapproximate`, `*datejulian`, `*era`, …) is copied
     **unconditionally** (Bugs B3, B4).
  5. `*datesplit` copied.
* xref: no data inheritance. It only causes inclusion (minxrefs), plus `\xref` handling in the
  bbl writer.
* Values are shared by reference. Inherited Names objects keep the parent's `nlid`, so per-list
  state keyed by nlid (visibility, ul, un) is shared between parent and child (Bug B10,
  confirmed).
* Cost: `get_inheritance` is a linear scan, and `is_inheritance_path` is an unmemoised DFS over
  a list that grows with duplicates. Both are **O(n²)–O(n³)** in the number of children of one
  parent that has a grandparent (§5).

### 3.6 Uniquename / uniquelist — D:H R:H

**Namestrings** (`process_namedis`, 1771-1940), per name, under the effective uniquenametemplate:
* The template name is chosen in this order: the list's template → per-entry
  `uniquenametemplatename` → per-namelist → per-name. Per-name and per-namelist overrides are
  **not reset** for later names or later namelists in the same entry (Bug B5).
* **base** = concatenation (no separator) of the parts in template order that are marked
  `base` and are non-empty, subject to the `use<part>` check (global/type/entry → namelist →
  name).
* Then, for each non-base part that is non-empty and not `disambiguation=none`, at level
  `np.disambiguation // UNIQUENAME_CONTEXTS{un}`, the code appends to the *last* namestring:
  * `full` → `[last+part, (part,'fullonly')]`;
  * `initorfull` → `[last+inits, (part,'init')]` then `[last+part, (part,'full')]`;
  * `init` → `[last+inits, (part,'init')]`.

  The `if (fc($level) eq fc('full'))` and `if … initorfull` checks are two separate `if`s, not
  `if/elsif`.
* `strip_nonamestring` is applied to the stored strings. Entries are stored only when the
  effective uniquename or uniquelist is not `false`.

**Iteration** (`uniqueness`, 3362-3416):

```
first=1
loop:
  if !changed: break          (changed was preset to 1 in process_lists)
  changed=0; create_uniquename_info; generate_uniquename      (set_uniquename flags changes)
  if !(first || changed): break
  changed=0; first=0; create_uniquelist_info; generate_uniquelist (set_uniquelist flags changes)
```

`set_uniquelist` compares the stored value with the *raw* count before adjusting it
(DataList.pm:571-583). A list whose stored value is decremented or never stored is therefore
flagged "changed" on every uniquelist pass, which costs one more uniquename pass. `unall`
(the uniqueness of every name regardless of visibility) does not depend on `ul`, so `ul` is
stable after pass 1 and `un` after pass 2. In practice the loop runs 2–3 times. The `changed`
flag lives on the DataList object, not in `state`.

**create_uniquename_info** (3475-3647), per citekey with a labelname:
* `un`: global/type/entry → per-namelist → per-name. A per-name `un` persists for later names,
  and `next MAIN if $un eq 'false'` drops the **whole entry** as soon as any name has
  `uniquename=false` (Bug B5b, confirmed).
* A name is *visible* if any of these holds: `un` ∈ {allinit, allfull, minyearinit,
  minyearfull}; `ul` is set and `index ≤ ul`; morenames; `count ≤ maxcitenames`;
  `index ≤ mincitenames`.
* Scope and key:
  * init/full/allinit/allfull: scope `global`, key = join(U+10FFFD, namestrings);
  * mininit/minfull: scope = join of visible basenames (+ "\x{10FFFD}et al" if truncated or
    morenames), key = join of visible full namestrings;
  * minyear*: the same plus `labelyear`.

  Visible names add `uniquenamecount{ns}{scope}{key}++` for each of their namestrings. All names
  add to `uniquenamecount_all` when `uniquelist` is not `false`.
* Counting rule: `get_numofuniquenames(ns, scope)` = the number of **distinct keys**, i.e.
  distinct full names that share this partial string. Identical names never make each other
  ambiguous.

**generate_uniquename** (3653-3770): the same `un`/visibility logic, **except** that the visibility
test omits `minyearinit/minyearfull` (inconsistent with create; Bug B14, suspected). Invisible
names are reset to `['base', baseparts]` without flagging a change. For each visible name the
first schema element whose namestring count is 1 becomes `un`. If none qualifies, `un` falls
back to schema[0] **only if no value is already set**, so a stale value from an earlier
iteration can survive (theoretical; see §4). The same applies to `unall`.

**create_uniquelist_info** (3778-3874), per citekey with a labelname and `ul ≠ false`:
* For each name, push the namestring at the `unall` level (or the basename when there is no
  `unall` or it is base), then `uniquelistcount{prefix}++`. Counting every prefix makes this
  O(L²) per entry for a list of length L.
* `uniquelistcount{final}{full}++`; for minyear, additionally `final{full}++` again (double
  count) and `final{year}{full}++`.
* For minyear only: count the visible prefix + year.
* **Key collision:** prefixes are stored as `{global}{<joined>}` and finals as
  `{global}{final}{…}`. A one-name prefix whose string is exactly `final` collides with that key
  (Bug B1, crashes). The year keys also live inside `final` and show up as pseudo-lists in
  `namelist_differs_*`.

**generate_uniquelist** (3882-3965): grow the prefix until its count is 1 (or the list ends).
With `minyear`, skip (keep the previous `ul`!) when the visible prefix is not shared within the
same year. Then `set_uniquelist`:
* return (store nothing) if the count ≤ 1, `count_names ≤ maxcn`, or `ul ≤ mincn` (unless
  `mincn == count`);
* if `final(prefix) > 1` (the prefix is a complete list that also exists elsewhere):
  `ul = namelist_differs_index + 1`, or nothing;
* else if `count > ul` and no other list at least as long differs at position `ul` → `ul--`.

`namelist_differs_index/nth` scan **all distinct final lists**, splitting each one again. This is
the O(N²) hotspot (§5).

**Context output** (generate_contextdata 4073-4105): per-namepart un values come from the schema
prefix up to the deciding element. Base and `full` entries before it are dropped, `fullonly` is
mapped to `full`, and the values map through `UNIQUENAME_VALUES`. Only the labelname list gets
un/ul.

### 3.7 Visible names — D:L R:M

`process_visible_names` (2892-3013), per namelist (all name fields) of every citekey:
* alpha: `count > maxalphanames ? min(minalphanames, count) : count`. Uniquelist is not used.
* cite: `count > maxcitenames ? (ul // min(mincitenames,count)) : count`. Then `pluralothers`:
  if `count - vis == 1`, `vis = count` (**cite only**).
* bib/sort: like cite but with max/minbibnames and max/minsortnames. They still use `ul`, which
  was computed against the *cite* limits, and `pluralothers` is not applied.
* All max/min options are looked up as entry → type → global. Per-namelist max/min do not exist.
* The visibility is stored per nlid, so shared Names objects (inheritance/XDATA) get
  last-writer-wins visibility (Bug B10).

### 3.8 Hashes (must be bit-exact) — D:M R:M

All four are `md5_hex(encode_utf8(NFC(normalise_string_hash(KEY))))`, where
`normalise_string_hash` (Utils.pm:736-743) does:
1. `s/\\(\p{L}+)\s*/$1:/g`
2. `s/\\([^\p{L}])\s*/ord($1).':'/ge`
3. `s/[\{\}~\.\s]+//g` (`\s` = Unicode White_Space under `use v5.24`)

Perl truthiness applies: `return '' unless $str`, so a KEY of `"0"` hashes as `""`.

| hash | KEY | Code |
|---|---|---|
| namehash / bibnamehash / `<field>namehash` | For the first `visible_cite` (or `visible_bib`) names: `hashid` if present, else for each namepart in the **datamodel `nameparts` constant order**, `get_hash_namepart` (full string or joined initials, per namehashtemplate `hashscope`; empty if the part is not in the template). No `strip_nonamestring`. Then `+` if (`vis < count` or morenames) and not `nohashothers`. | Internals.pm:39-102 |
| fullhash / `<field>fullhash` | All names; same per-part rule but **with** `strip_nonamestring`; `+` only for morenames; `nohashothers` ignored. Labelname variant uses `labelnamefh_info` (skips `short*`). | 104-150 |
| fullhashraw | All names; raw `get_namepart` of every datamodel namepart (ignores the template), stripped; `+` for morenames. | 154-176 |
| per-name `hash` | `hashid` alone (hashed) or the namepart concatenation under the namehashtemplate (per-name template applied *locally* here). | 256-310 |
| `_getnamehash_u` (internal; extraname/extradate/extratitle context) | Visible-cite names under the **uniquenametemplate**: base parts in full; non-base parts in full or as initials depending on the name's current `un->[1]` (`full`/`fullonly` → full, `init` → initials, base → omitted); `+` as for namehash. | 180-253 |

Template resolution in `_getnamehash`/`_getfullhash`/`_getnamehash_u`: per-entry or list template
→ per-namelist → per-name. The per-name override **leaks** to later names (same pattern as Bug B5).

Other bit-exact hashes in this area: related clone key = `md5_hex(encode_utf8(key))` (no
normalisation); extradate context for non-name fields =
`md5_hex(encode_utf8(NFC(normalise_string_hash(value or join('',list)))))`.

### 3.9 labelname / labeldate / labeltitle — D:L R:L

* labelname (2549-2615): the first non-empty field in the type-specific `labelnamespec` that is a
  name list and passes `use<field>` (for `shortX`, the check is `useX`) → `labelname_info`. A
  second pass that skips `short*` → `labelnamefh_info` (used by fullhash).
* labeldate (2623-2763): only when `labeldateparts` (global/type; per-entry is ignored, Bug B6).
  The first spec item with a defined `<x>year` or `<x>endyear` wins (date fields map to
  prefix+parts; non-date fields such as `year` are "pseudodates"). A `string` item gives a
  fallback `labeldatesource`. The function sets `labelyear/month/day/hour/minute/second/timezone`
  and `labeldatesource` (the prefix, e.g. `''` for `date`, or the field name for pseudodates).
  When the end differs, it appends `\bibdatedash <end>` (year always; the other parts only for
  real dates).
* labeltitle (2771-2791): the first non-empty field in `labeltitlespec` → `labeltitle` field.
  Always computed; the `labeltitle` option only gates extratitle.

### 3.10 Labelalpha — D:H R:H

`_genlabel` (Internals.pm:340-357): `labelalphatemplate` per entry type. Label elements are sorted
by `order`. Each element is a disjunction of parts; the first part that produces a non-empty
string wins, and a `final` part stops the whole label. `label`/`shorthand`/`sortkey` are used raw
(`nostrip`); other fields go through `normalise_string_label`, which removes `\macros`, turns
`~` into a space, applies the `nolabel` regexes, and trims and collapses spaces. Literal parts
are taken as-is (escape(unescape)). Every part except literals returns both a label and a sort
label (`unescape_label`).

**Names** (`_label_name`, 492-690):
* `use<field>` gate.
* labelalphanametemplate: list → per-entry → per-namelist → **per-name (leaks to later names,
  Bug B5, confirmed with `useprefix`)**.
* Template parts: `pre` parts (prefix), each truncated to its own `substring_width` (default 1),
  compound-split on `[\s\p{Dash}~]`; main parts go through `_process_label_attributes`.
* The name range is `names=a-b`; `+` means the alpha visibility; capped at the count. The
  default is `1..visible_alpha`.
* `namessep` goes between names. `alphaothers`/`sortalphaothers` are appended when
  `count > end` or morenames, unless `noalphaothers` is set.
* `ifnames=N` or a range is tested against alpha visibility (`maxalphanames`/`minalphanames`).

**Attributes** (`_process_label_attributes`, 696-865):
* **static width:** substring of `substring_width` (default 1) grapheme clusters
  (`Unicode::GCString`), from the left or the right. A namepart-specific width or side overrides
  it. `nolabelwidthcount` regex matches are removed before truncation and reinserted at their
  original indices afterwards. `pad_char`/`pad_side` pad the result, which is then
  `escape_label`led. `substring_compound` applies the substring to each `[\s\p{Dash}]+` part.
* **`v` (variable width per string):** a cache over all section citekeys of the field's strings
  (for names: the normalised concatenation of the template's parts of visible-alpha names →
  name index). For each string it picks the shortest prefix whose count among all strings' prefixes
  of the same length is 1 (or `substring_width_max`). `f` adds a fixed width: per name index, the
  maximum unique width that occurs ≥ `substring_fixed_threshold` times. The fallback uses `||`,
  so a computed global index of 0 is ignored (Bug B15, suspected).
* **`l` (list disambiguation):** `_label_listdisambiguation` over all citekeys' lists. It is an
  iterative greedy algorithm (§3.10.1).
* `uppercase`/`lowercase` (both set → no change).
* The `v`/`l` caches are keyed **only by field name**, per section (Bug B13, suspected). A second
  datalist, or a second label element using the same field with different nameparts or
  templates, reuses the first cache.

#### 3.10.1 `_label_listdisambiguation` (962-1020)

Start with every name at width 1 and key each entry by `join('', substrings)`. Note that there
is **no separator**, so `['Ab']` and `['A','b']` collide. Each round:
* entries with unique keys are final;
* for every ambiguous group: if all lists in the group are identical, set the first entry to
  1-char substrings; otherwise find, for the group's first member, the first position whose
  name is unique among the non-identical members, and widen that position by 1;
* rebuild the keys.

This is greedy and order-dependent (it always widens the first member of a group), and it can
yield identical labels. The ambiguity groups are iterated in hash order, but each group's update
depends only on its own members, so the result is deterministic. There is no termination proof
or guard; the Rust port should add an iteration cap that falls back to the same result.

**extraalpha:** `ladisambiguation{labelalpha}++` for every citekey with labelalpha. Numbers are
assigned in sorted list order (generate_contextdata) for entries whose label count is > 1.

### 3.11 extra* and work uniqueness — D:M R:M (needs exact sort order)

For each citekey that is not `skiplab` (process_entries_post):

| field | tracking key | count rule | number assigned |
|---|---|---|---|
| extradate | `"$ctx,$datestring"`. `ctx` = hash of the first non-empty `extradatecontext` field (a name list via `_getnamehash_u`; list/scalar via md5 of `normalise_string_hash`); `datestring` = concatenation, over `extradatespec` scopes, of the first defined field in each scope. Sets the `extradatescope` entry field. | `seen++` only to 1 if `ctx` is empty (no name or title) | in sorted order, if `seen > 1` |
| extraname | `labelnamehash` = `_getnamehash_u(labelname)` | always | if `seen > 1` |
| extratitle | `"$namehash_u,$labeltitle"` (only if the `labeltitle` option is set, global/type) | beyond 1 only if a title exists | if `seen > 1` |
| extratitleyear | `"$labeltitle,$(labelyear || year || '')"` | beyond 1 only if a title exists | if `seen > 1` |
| extraalpha | labelalpha string | always | if `count > 1` |

Work uniqueness (2180-2260): entries whose `options` **datafield** contains the literal token
`skipbib` are ignored. `skipbib=true`/`dataonly` are not recognised (Bug B11, confirmed). The keys
are:
* singletitle: `fullhash(labelname)`;
* uniquetitle: the labeltitle value;
* uniquebaretitle: labeltitle when there is no labelname;
* uniquework: fullhash . labeltitle;
* uniqueprimaryauthor: the base namestring of the first name (recomputed with `process_namedis`),
  counted by distinct per-name hash.

Uniq-ignore (`ignore=` in inheritance rules) removes inherited labelname/labeltitle fields from
the counts. Flags are set when the count is < 2.

**Interplay with sorting (precise):** every *count* is computed before sorting, over all section
citekeys, and is order-independent. Every *number* (extradate/extraname/extratitle/
extratitleyear/extraalpha) is assigned by walking `dlist->get_keys` *after* filtering and
`sort_list`. Within each tracking key, the i-th entry in sorted order gets value i. Sorting may use
labelalpha/sortlabelalpha, labelname, labeltitle, labelyear, and so on, but it **cannot** use
extra* (Internals.pm:1110-1120 lists the forbidden fields). There is no cycle. Ties in the sort
(identical sort keys) are therefore resolved by the sort's stability relative to the input order,
which is the citekeys order (§4). Each datalist (refcontext) has its own numbering.

### 3.12 Filters, nocite, presort, skips — D:L R:L

* `check_list_filter` (3190-3296): `type/nottype` (entrytype eq lc(value)),
  `subtype/notsubtype`, `keyword/notkeyword` (exact keyword match), `field/notfield`
  (`field_exists`). The list's filters are AND-ed; an array filter is OR-ed. Unknown filter types
  pass. Then the buggy set re-add step runs (Bug B2).
* nocite (2531-2541): `nocite=1` if `\nocite`d, or if allkeys came from `\nocite{*}` and the key
  was not cited explicitly. The bcf pre-pass (Biber.pm:921-943) makes `\cite` win over `\nocite`
  in either order; this pre-pass is O(n²) with an NFD per comparison.
* presort: the entry's `presort` field becomes the per-entry option.
* skips: `skiplab` (per entry via `$secnum`, so it works) suppresses labelalpha, extra* and
  work-uniqueness tracking only through the `options` field check. `skipbib`/`skipbiblist` are
  only passed through to biblatex.

### 3.13 Options lookup — D:L R:H (silent semantics)

`Config::getblxoption(secnum, opt, type, key)` (Config.pm:1012-1034) checks entry
(`{key}{secnum}`) → type → global. When `secnum` is `undef`, per-entry values are never found.
Biber calls it with `undef` for labelalpha, labeldateparts, labeltitle, labeltitleyear,
singletitle, uniquetitle, uniquebaretitle, uniquework and uniqueprimaryauthor, so **per-entry
settings of these ENTRY-scoped options are ignored** (Bug B6, confirmed). A compatible port must
replicate this.

---

## 4. Order dependencies and nondeterminism

**Confirmed nondeterministic output (Perl hash randomisation, different on every run):**
1. **mincrossrefs/minxrefs promotion order** (`keys %crossrefkeys`, Config.pm:1256/1269). The
   promoted parents are appended to citekeys in random order. With `sorting=none` the bbl order,
   and therefore labelnumbers, change between runs (`scratch/semantics/nd1`: 6 runs gave 6
   different md5s).
2. **Dynamic set keys under `\nocite{*}`** (`dynamic_set_keys`, Section.pm, bibtex.pm:229-234).
   With `sorting=none` the order of the sets in the bbl is random (`scratch/semantics/nd2`).

**Hash-order iteration that is (believed) benign:** `create_entry` order under allkeys (matters
only if sourcemaps create or clone entries in a way that collides); `resolve_xdata` over
`values %entries` (cycle detection does not depend on order, but Bug B9 re-applies granular refs
once per reference, regardless of order); `get_set_children`; labelalpha `v` (`keys %indices`;
counts are fully built before use) and `l` (`_check_counts`); `getblxentryoptions` (the bbl
writer sorts); `Name->{rawstring}` (`keys nameparts`, used only for the "others" test).

**Order-dependent but deterministic** (the port must keep the exact citekeys order):
* citekeys order: bcf cite order (dedup; a `\cite` takes the position of an earlier `\nocite`) →
  alias real keys appended at fetch → (allkeys: file order per datasource) → related clones
  (dynamic-set-member clones first, then in citekey order; recursive clones depth-first) → set
  members appended per set → promoted crossref/xref targets (hash order) → alias keys removed.
* Last-writer-wins on shared Names objects: visibility, ul and un are stored per nlid
  (Bug B10). The winner depends on citekeys order.
* An entry in several sets: `\inset` = the last parent processed.
* Per-name option leakage (Bug B5): the result depends on name order.
* `uniqueness` fallback `unless defined(get_uniquename)`: a value set in iteration k can survive
  into k+1 even if it would otherwise be reset to base. This is theoretical, because the full
  namestring always has count 1 (counts are of distinct full keys).
* Sort ties → extra* numbering (§3.11).
* Locale: `Entry::datafields`/`fields` sort with `use locale` (Entry.pm:566-611). Field names are
  ASCII lowercase, so this is safe in practice. Inheritance iterates sorted field names, which
  does not affect results.

**Perl-isms that must be reproduced exactly:**
* Truthiness of `"0"`: `get_namepart` returns `string || ''`, `if (my $p = …)`, and
  `return '' unless $str` in normalisers. A namepart, title or field value of `0` is treated as
  empty.
* `||` versus `//`, `eq` versus `==`.
* `\s`, `\p{Dash}` and `\p{L}` follow Perl's Unicode tables.
* `Unicode::GCString` uses the grapheme-cluster rules of sombok's bundled (old) Unicode version.
* `lc`/`uc`/`fc` use full case mapping.

---

## 5. Performance hotspots and algorithmic notes

Measurements: biber 2.22, `\nocite{*}`, wall time including about 2 s of startup and LaTeX
setup (`scratch/semantics/perf`, generator `gen.py`):

| input | 500 | 1000 | 2000 | 4000 | growth |
|---|---|---|---|---|---|
| base (distinct authors, authoryear) | – | 3.05 s | 5.36 s | 9.69 s | linear |
| `un`: 1 surname "Smith" shared by all, distinct given, uniquename=full | – | 3.15 | 5.28 | 9.87 | linear |
| `ul`: shared 2-name prefix, 5 names, uniquelist=true, maxcitenames=2 | – | 6.06 | 11.48 | 22.08 | linear (×2 base) |
| `alpha`: alphabetic style | – | 3.24 | 5.49 | 11.48 | slightly super-linear |
| **`ul3`: pairs of identical 4-name lists, uniquelist=true** | 6.59 | 13.53 | 33.54 | 103.3 | **quadratic** |
| `cr0`: N children → 1 crossref parent | 2.57 | 4.37 | 8.39 | – | linear (at this size) |
| **`cr`: N children → parent → grandparent** | – | **26.71** | **244.7** | (killed) | **cubic (×9.2 per doubling)** |

(Some runs overlapped with other biber jobs, so absolute times are approximate; the growth rates are clear.)

Uniqueness over many shared surnames is **not** superlinear: counts are hash-based, and
`scalar keys` is O(1). The superlinear paths are these:

1. **`namelist_differs_index` / `namelist_differs_nth`** (DataList.pm:2251-2324). They are called
   from `set_uniquelist` for every entry whose prefix is a complete list that also exists
   elsewhere (`differs_index`, a full scan without early exit) or which has more names than its
   `ul` (`differs_nth`, which exits early on the first match). Each call scans every distinct
   final list and re-`split`s it, giving O(N·D·L) per uniquelist pass, times 1–2 passes.
   Fix: build a trie or prefix-hash over the final lists once per pass. "Longest common prefix
   with any *other* list" and "exists another list ≥ len with the same (n-1)-prefix but a
   different n-th element" can both be answered in O(L) with prefix maps
   `prefix → {count, set of next elements, max length}`.
2. **Crossref cascade bookkeeping** (Config.pm:1119-1165, Entry.pm:941-950). `set_inheritance`
   appends duplicates. `get_inheritance` is a linear `first`. `is_inheritance_path` is a DFS
   without a visited set that runs `grep` over the whole list at every node. With a grandparent,
   every child re-runs `parent->inherit_from(grandparent)`, and the path check walks all earlier
   children: **O(n³)** (26.7 s for 1000 children). Fix: a set of edges plus a cached
   "already inherited" flag per (parent, child), with cycle detection via a visited-set DFS over
   adjacency lists (or a single topological pass). The cascade then runs once per parent; the
   result is identical, because re-inheritance is idempotent given the same inputs.
3. **`_process_label_attributes`** computes `first_index` of the citekey in a freshly copied
   citekeys array for *every* label part with attributes (Internals.pm:703-704): O(N) per call, even though
   the index is used only by the `l` mode. That makes it O(N²·parts) overall. Fix: a key → index map per section.
4. **`Section::del_citekey`** rebuilds two arrays (O(N)) per deletion. `process_citekey_aliases`
   and missing-key handling can become O(N²) when there are many aliases. Fix: an indexed set.
5. **bcf cite/nocite pre-pass** (Biber.pm:921-943): `first {$key eq NFD(...)}` over all earlier
   keys is O(N²) NFD calls for documents with many explicit cites. Fix: a hash of NFD keys.
6. **`get_dependents`**: `first {$x eq $_} @keyswithdeps` gives O(N²) when there are many
   dependents. Fix: a set.
7. **`create_uniquelist_info`**: re-joins every prefix, O(L²) per entry. For long author lists
   (hundreds of authors in physics) this is significant. Fix: incremental hashing or trie node ids.
8. **`process_namedis` is called twice** per entry when `uniqueprimaryauthor` is set. Template
   lookups via `getblxoption` are hash chains on every call; precompute the effective options
   once per entry.
9. **`generate_contextdata` with sortsets**: `first_index` per set member, O(N) each.
10. **XDATA**: every reference re-resolves the XDATA entry recursively. That is wasteful, and also
    wrong for granular refs (B9).

Algorithmic improvements that keep the output identical: the per-list passes are independent
across datalists apart from the shared section caches (Bug B13) and shared Names objects
(Bug B10). If the port reproduces those two quirks explicitly, the lists could be processed in
parallel. Within a list, every counting pass is a map-reduce over entries. Only the extra*
numbering and labelalpha `l`/`v` need ordered sequential passes.

As the coordinator's profiling found, the steady-state cost in normal documents is dominated by
parsing (`_create_entry` field handlers). The Biber.pm passes become superlinear only for the
adversarial shapes above: large groups of identical name lists with uniquelist, and many
children of a crossref parent that has its own crossref. The second is realistic, e.g.
proceedings with hundreds of @inproceedings → @proceedings → @mvproceedings.

---

## 6. Rust design suggestions

* **Arena + ids.** Entries in a `Vec<Entry>` with `EntryId`, and citekeys as `IndexSet<KeyId>`
  (insertion-ordered, O(1) delete). Names live in a `NameListId`/`NameId` arena. To be
  compatible with biber, *preserve sharing semantics*: inheritance and XDATA copy the
  `NameListId` (not a deep clone), so per-list state keyed by `NameListId` shows the same
  last-writer-wins behaviour (B10). Track "logical index" separately, with biber's buggy index
  rules (B8), behind a `compat` flag.
* **Option resolution:** a single `resolve(opt, entry, secnum_given: bool)` function that mirrors
  the `undef`-secnum quirk (B6), so call sites read like the Perl.
* **Determinism:** wherever biber uses hash order and the output depends on it (B7/§4.1-2),
  choose a stable order: first-seen order for promoted crossref/xref targets and bcf order for
  dynamic sets. The test oracle must treat these positions as a multiset (§7).
* **Uniqueness:** keep the exact iterative structure (create/generate × un/ul, the changed flag
  semantics, the "set only if undefined" fallback). Replace `namelist_differs_*` with prefix
  maps. Use interned strings for namestrings (the U+10FFFD joins become `Vec<Sym>` keys) but
  keep the string-concatenation semantics where they leak: the `final` key collision (B1) needs
  an explicit emulation decision, and keys that join without separators (`"$ns,$ys"`, the label
  list join) can collide.
* **Hashes:** implement `normalise_string_hash` on top of `regex` with Unicode classes and check
  it against Perl on a corpus that includes `\v{S}`, `\"a`, `{\'e}`, NBSP and the other Unicode
  spaces, and `0`. MD5 over UTF-8 after NFC (`unicode-normalization`).
* **Graphemes:** `unicode-segmentation` uses current UAX#29 rules, while `Unicode::GCString` is
  older. Labels with emoji or Indic text may differ. Add a compat test set and consider a
  vendored legacy segmentation table.
* **Inheritance:** model `\DeclareDataInheritance` as compiled rules (type-pair matcher →
  defaults + field rules). Copy biber's processing order exactly, including "processed" marking
  on match rather than on copy, the date special case with its bugs (B3/B4), and `*datesplit`
  copying.
* **Labelalpha:** implement `v`/`l` caches keyed exactly as biber does (by field only, per
  section) in compat mode, and add an iteration guard to the list-disambiguation loop.

---

## 7. Test-oracle notes

* `t/*.t` tests call `Biber->prepare` directly with programmatically set options. The `.bcf`
  files in `t/tdata` can be run through the CLI binary, but the options the tests set in Perl
  (e.g. `sortlocale`, `mincrossrefs`) must be replicated in a `biber.conf` or on the command line.
  The test suite cannot run here: no Perl deps (`Text::BibTeX`) are installed.
* Pinned by tests: crossrefs.t (37, incl. mincrossrefs, cascading, the `inherit_all` date special
  case), xdata.t (13, granular refs, loops), related-entries.t (15, clone keys = md5 of the key),
  set-*.t (static/dynamic/legacy), uniqueness.t (227 assertions covering every uniquename mode,
  uniquelist and minyear), uniqueness-nameparts.t (36), labelalpha.t (122 incl. `v`, `l`, `f`,
  padding, ranges, compound, nolabelwidthcount), labelalphaname.t, extradate.t (39),
  extratitle.t, extratitleyear.t, skips.t, truncation.t, sections*.t (82),
  datalists.t (filters), full-bbl.t.
* **Not pinned** (the bugs below are all untested): multiple references to an XDATA entry that
  itself has granular refs; per-name/per-namelist options on any name other than the last;
  per-entry ENTRY-scope options looked up with undef secnum; filters plus sets; crossref date
  metadata; shared Names objects with per-entry max/mincitenames.
* Oracle normalisation: with `sorting=none`, auto-promoted crossref/xref targets and allkeys
  dynamic sets come out in random order. Compare those runs of entries as sets, or run biber with
  `PERL_HASH_SEED=0 PERL_PERTURB_KEYS=0` (check that the PAR binary honours them) and pick one
  canonical order.
* Generating bcf files needs only one `pdflatex -draftmode` run (`scratch/semantics/run.sh`).

---

## 8. Suspected biber bugs

Status legend: **CONFIRMED** = reproduced with biber 2.22 (scenario directory given);
**suspected** = from reading the code only.

| # | Location | Description | Status / repro |
|---|---|---|---|
| B1 | DataList.pm:177, 190 (create_uniquelist_info Biber.pm:3850-3857) | One hash holds both prefix counts `{global}{<joined names>}` and final counts `{global}{final}{…}`. If a labelname list's first (basename) namestring is exactly `final`, `{global}{final}++` turns the hashref into a number and the next line dies: *Can't use string ("1") as a HASH ref*. Similarly, uniquelist=minyear year keys sit inside `final` and are scanned as pseudo-lists by `namelist_differs_*`. | **CONFIRMED** crash: `author={final}`, uniquelist=true (`t1/`) |
| B2 | Biber.pm:3155-3162 | Filtered-list "add set members back" uses `first {$be->get_field('entryset')} @sets`. The block ignores `$_`, so it adds *every* key that has an `entryset` (all sets **and** all set members, which carry `entryset=[parent]` after process_sets) whenever any set passes the filter. Members of passing sets are added only because of that, and failing sets are re-added. | **CONFIRMED** (`filt/`): `\printbiblist{shorthand}` lists S2 (no shorthand) and its members |
| B3 | Entry.pm:1067-1080 | Crossref `inherit_all` copies derived date metadata (`datecirca`, `dateuncertain`, `datejulian`, era, …) even when the child has its own date. `next if first {$_ eq $datefield} @removed_fields` compares date *field* names with date*part* names and never matches. | **CONFIRMED** (`cr1/`): child `date={2000}` gets `\true{datecirca}` from parent `1999~` |
| B4 | Entry.pm:1048-1060 | The date-part guard checks only the main `datesplit` (`$parent->get_field('datesplit') and $self->get_field('datesplit')`) for *every* datepart. `origdate`/`eventdate`/`urldate` parts are mixed: the child `origdate=2005` gets the parent's `origmonth`. | **CONFIRMED** (`cr1/` entry D: origyear 2005 + origmonth 5) |
| B5 | Internals.pm:600-610 (labelalpha), Biber.pm:1816-1837 (namedis), 3537-3545 & 3694-3702 (uniquename), Internals.pm:62-72, 113-123, 205-209 (hashes) | Per-name (and per-namelist) option overrides are assigned to loop-outer variables and never reset. They leak into later names (and later namelists). | **CONFIRMED** for `useprefix` in labelalpha (`leak/`: `vSvJ00` vs `JvS00` for the same names in a different order) |
| B5b | Biber.pm:3545, 3702 | A per-name `uniquename=false` on *any* name triggers `next MAIN`, which drops the whole entry from uniquename counting and generation. Other names lose `un`, and other entries lose disambiguation against it. | **CONFIRMED** (`unleak/`): "Smith, John" gets un=0 although "Smith, Bob" exists |
| B6 | Biber.pm (e.g. 2148, 2203, 2271, 2385, 2448, 3035, 3051, 4020-4052) + Config.pm:1017-1024 | `getblxoption(undef, opt, type, key)` cannot see per-entry options (stored under `{key}{secnum}`). The ENTRY-scoped options labelalpha, labeldateparts, labeltitle, labeltitleyear, singletitle, uniquetitle, uniquebaretitle, uniquework and uniqueprimaryauthor are silently ignored per entry. | **CONFIRMED** (`peropt/`): `options={labelalpha=false}` still gets labelalpha+extraalpha; `labeldateparts=false` still gets extradate |
| B7 | Config.pm:1254-1270; Biber.pm:1617-1646 | Promotion order of mincrossrefs/minxrefs targets is Perl hash order. Output is **nondeterministic** with `sorting=none` (and wherever the input order breaks sort ties). | **CONFIRMED** (`nd1/`, 6 runs → 6 orders) |
| B7b | Section.pm `dynamic_set_keys`; bibtex.pm:229-234 | Dynamic set keys are added to citekeys in hash order under `\nocite{*}`. | **CONFIRMED** (`nd2/`) |
| B8 | Names.pm:88-110 | `replace_name` and `splice` (granular XDATA) set **0-based** indices, while `add_name` uses 1-based ones. `splice`'s loop (`$i<$#…`) also skips the last name, which keeps its index from the XDATA list. Name visibility in uniquename (`index <= mincitenames`) shifts by one. | **CONFIRMED** (`xdidx/` vs `xdidx2/`: same names give un=1 vs un=0) |
| B9 | Entry.pm:765-768 + 818-875 | A referenced XDATA entry's own refs are re-resolved every time it is referenced. Granular name-list refs then splice again at the original position: the placeholder was already replaced, so a real name gets replaced by the whole list again. Names are duplicated once per extra reference. | **CONFIRMED** (`xdtwice/`): 3 entries referencing X give 5 names instead of 3 |
| B10 | Entry.pm:1036, 1098, 798 (shared values); Biber.pm:2892-3013, DataList.pm per-nlid state | Inherited or XDATA Names objects are shared by reference, so the child and parent share an nlid. Visibility, uniquelist and uniquename are per nlid, so the last entry processed wins. A child with `maxcitenames=1` gets the parent's untruncated `editornamehash` (or vice versa, depending on order). | **CONFIRMED** (`sharednl/`: C and D have identical data but different `editornamehash`) |
| B11 | Biber.pm:2194-2198 | Work-uniqueness skip checks for the literal token `skipbib` in the `options` datafield. `skipbib=true` and `dataonly` are not recognised, and set members (whose skipbib is a per-entry option, not a datafield) are never skipped. | **CONFIRMED** (`st_*`): `skipbib` → singletitle set; `skipbib=true`/`dataonly` → not set |
| B12 | Entry.pm:179 | `clone()` lists `yeardivision` twice and omits `endyeardivision`, so related clones lose the end-date season/division. | suspected (clear from code) |
| B13 | Internals.pm:714, 797; Section.pm labelcache | Labelalpha `v`/`l` caches are keyed by field name only, per section. A second datalist (different labelalphanametemplate or visibility) or a second label element on the same field with different nameparts reuses the wrong cache, so lookups can yield `undef` and an empty label part. | suspected |
| B14 | Biber.pm:3565 vs 3704 | Name visibility for uniquename counting includes `minyearinit/minyearfull` (all names visible, no "et al" marker in the scope); generation does not. Scope strings for minyear* therefore always cover the full list, unlike mininit. | suspected (inconsistent) |
| B15 | Internals.pm:719, 785 | `substring_width=vf`: `globalindices || index` ignores a computed global index of 0. Names whose own unique width is larger than 1 then keep it, while a global index ≥ 1 would override them. | suspected |
| B16 | Biber.pm:3857-3862 | `uniquelist=minyear` increments `final{list}` twice per entry. That forces the "identical lists" branch whenever the prefix equals the full list. No differing output was found in several attempts. | suspected, low impact |
| B17 | Entry.pm:176, 697, 1118; bbl.pm:458; bibtex.pm:235 | `foreach my $df ($dmh->{datefields}->@*) { $df =~ s/date$//; … }` mutates the global datamodel helper array in place. It is idempotent for standard fields. A custom date field such as `updatedate` would be stripped twice (`update` → `up`). | suspected (latent) |
| B18 | Biber.pm:3995-4000 | sortsets: a member missing from the list gives `first_index = -1`, and the slice picks the last list key. Unreachable while B2 exists. | suspected (latent) |
| B19 | Section.pm `has_dynamic_sets` | Returns `defined($self->{dkeys})`, which is always true because it is initialised to `{}`. Harmless. | suspected (dead check) |
| B20 | doc/biber.tex:459-461 vs Biber.pm:1617-1646 | The manual says targets are promoted if "CROSSREF'ed or XREF'ed at least mincrossrefs times". The code counts crossrefs and xrefs separately, against `mincrossrefs` and `minxrefs`. Promotion cascades only one level. | doc/code mismatch |
| B21 | Biber.pm:2943-2950 | `pluralothers` adjusts only cite visibility. Bib, sort and alpha visibility (and namehash variants such as bibnamehash) do not get it. | suspected (needs a biblatex-side ruling) |

---

## 9. Open questions

1. Does the PAR-packed biber honour `PERL_HASH_SEED`? If yes, the oracle can pin one order for
   B7/B7b.
2. B10: is sharing intended? biblatex expects per-entry visibility, so a deep copy would be the
   "correct" fix, but it would change hashes. The port needs a policy: emulate the bug, or fix
   it and accept diffs.
3. `_label_listdisambiguation` termination: no proof was found. A pathological input that loops
   forever may exist (the joined keys have no separator).
4. Which biblatex version semantics should `pluralothers` follow for bib/sort visibility (B21)?
5. Should the port reproduce B1 (a crash) or B9 (duplicated names)? We recommend fixing crash
   bugs and keeping a "compat" mode for the output-affecting ones only where the test corpora
   need it.
6. In multi-section documents, `Config->_init` does not reset per-entry options keyed by secnum,
   or keyorder. Per-section correctness relies on the secnum in the key. Not verified for
   entries that appear in several sections with sourcemaps.

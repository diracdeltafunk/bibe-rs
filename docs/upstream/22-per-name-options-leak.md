<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-019 and BB-020 (docs/biber-bugs.md). -->

**Title:** Per-name options (`useprefix`, name templates, `uniquename`) leak to the following names in the list, and a per-name `uniquename=false` drops the whole entry from uniquename

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

**1. Per-name options leak (BB-019).** Wherever biber loops over the names of a list, a per-name option is applied by assigning to the variable that holds the entry/namelist-level value, which is declared outside the loop:

```perl
    foreach my $name ($names->names->@*) {
      # name scope useprefix
      if (defined($name->get_useprefix)) {
        $useprefix = $name->get_useprefix;
      }
```

So the option stays in force for every *following* name in the list. The same pattern is used for

* `useprefix` and `labelalphanametemplatename` in `_label_name` (Internals.pm:582-591),
* `useprefix` and `sortingnamekeytemplatename` in `_namestring` for sorting (Internals.pm:1626-1640),
* `namehashtemplatename` in `_getnamehash`, `_getfullhash` and `_genpnhash` (Internals.pm:58-72, 116-129, 273-…),
* `uniquenametemplatename` in `_getnamehash_u` (Internals.pm:199-208) and `process_namedis` (Biber.pm:1809-1831),
* `uniquename` in `create_uniquename_info` and `generate_uniquename` (Biber.pm:3549-3552, 3706-3709).

Results therefore depend on the order of the names. Example (`style=alphabetic,maxalphanames=3`):

```bibtex
@book{a, author = {family=Smith, given=John, prefix=von, useprefix=true and family=Jones, given=Bob, prefix=van}, title = {T}, year = {2000}}
@book{b, author = {family=Jones, given=Bob, prefix=van and family=Smith, given=John, prefix=von, useprefix=true}, title = {T}, year = {2000}}
```

`b` gets `\field{labelalpha}{JvS00}`, correctly, but `a` gets `vSvJ00`: Jones's `van` is used because Smith's `useprefix=true` leaked to him. It should be `vSJ00`. The same leak changes sort keys (`sorting=nyt` with the same kind of data) and name hashes.

**2. A per-name `uniquename=false` skips the entry (BB-020).** In `create_uniquename_info` and `generate_uniquename`, the per-name value is followed by

```perl
      next MAIN if $un eq 'false';
```

which abandons the whole entry, not just that name. With `style=authoryear,uniquename=init,maxcitenames=3`:

```bibtex
@book{a, author = {Smith, John and Doe, Jane}, title = {T1}, year = {2000}}
@book{b, author = {Smith, Bob and family=Doe, given=Jim, uniquename=false}, title = {T2}, year = {2001}}
```

`b` gets no uniqueness information at all, and `Smith` in both entries gets `un=0`. Without the `uniquename=false`, both Smiths get `un=1` (J. vs. B.), which is what should happen here too; only `b`'s Doe should be left out.

```latex
\documentclass{article}
\usepackage[style=authoryear,uniquename=init,maxcitenames=3]{biblatex}
\addbibresource{test.bib}
\begin{document}
\nocite{*}
\printbibliography
\end{document}
```

Same results on biber 2.22 and current `dev` (1686b9c).

### Suggested fix

In each loop, keep the per-name value in a variable local to the iteration, falling back to the outer one, e.g.

```perl
    foreach my $name ($names->names->@*) {
      my $nuseprefix = $name->get_useprefix // $useprefix;
      my $nlantname  = $name->get_labelalphanametemplatename // $lantname;
      …
```

and in the two uniquename loops, skip only the current name when its `uniquename` is `false` (I tried just replacing `next MAIN` with `next` in both places: `b` then gets uniqueness information, but both Smiths still get `un=0`, so something else in the counting also needs to change). I haven't attached a patch because the change touches about ten loops; I'm happy to prepare one if that helps.

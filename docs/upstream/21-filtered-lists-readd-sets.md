<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-018 and part of BB-137 (docs/biber-bugs.md). -->

**Title:** Filtered lists (e.g. the shorthand list) get every set and set member re-added once any set passes the filter

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

After filtering a list, `process_lists` adds back the members of sets that passed the filter (Biber.pm:3164-3170 on `dev`):

```perl
      my @sets = grep {$section->bibentry($_)->get_field('entrytype') eq 'set'} $flist->@*;
      foreach my $k ($list->get_keys->@*) {
        my $be = $section->bibentry($k);
        if (first {$be->get_field('entryset')} @sets) {
          push $flist->@*, $k;
        }
      }
```

The block passed to `first` doesn't use `$_`, so it is true for every entry that has an `entryset` field (every set, and every member of any set) as soon as `@sets` is non-empty. In the shorthand list below, only `S1` has a shorthand, but `S2`, `c` and `d` are added too:

```bibtex
@set{S1, entryset = {a,b}, shorthand = {SH1}}
@set{S2, entryset = {c,d}}
@book{a, author = {Alpha, A.}, title = {TA}, year = {2000}}
@book{b, author = {Beta, B.},  title = {TB}, year = {2001}}
@book{c, author = {Gamma, C.}, title = {TC}, year = {2002}}
@book{d, author = {Delta, D.}, title = {TD}, year = {2003}}
@book{e, author = {Eps, E.},   title = {TE}, year = {2003}}
```

```latex
\documentclass{article}
\usepackage[style=authoryear]{biblatex}
\addbibresource{test.bib}
\begin{document}
\cite{S1,S2,e}
\printbiblist{shorthand}
\printbibliography
\end{document}
```

biber 2.22 and current `dev` (1686b9c) write `S2, a, b, c, d, S1` into `\datalist[list]{shorthand:…}`; it should be `a, b, S1`. With `first`, a set key `0` would also count as "not found".

### Suggested fix

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -3161,10 +3161,11 @@
       }
 
       # Now add set members of sets which have not been filtered out
-      my @sets = grep {$section->bibentry($_)->get_field('entrytype') eq 'set'} $flist->@*;
+      my %sets = map {$_ => 1} grep {$section->bibentry($_)->get_field('entrytype') eq 'set'} $flist->@*;
       foreach my $k ($list->get_keys->@*) {
         my $be = $section->bibentry($k);
-        if (first {$be->get_field('entryset')} @sets) {
+        next if $be->get_field('entrytype') eq 'set'; # sets are filtered like other entries
+        if (grep {$sets{$_}} ($be->get_field('entryset') // [])->@*) {
           push $flist->@*, $k;
         }
       }
```

With this change the shorthand list contains `a, b, S1`. All tests in `t/` pass on `dev` (1686b9c) with this patch.

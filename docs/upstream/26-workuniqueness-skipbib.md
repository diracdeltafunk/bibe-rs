<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-027 (docs/biber-bugs.md). -->

**Title:** `singletitle`/`uniquetitle`/`uniquework` only ignore entries whose `options` field contains the literal token `skipbib`; `skipbib=true` and `dataonly` still count

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

`process_workuniqueness` leaves entries that are not in the bibliography out of the work-uniqueness counts, but it recognises them by looking for the exact string `skipbib` in the raw `options` field. Equivalent spellings are missed:

```bibtex
@book{a, author={Smith, John}, title={T1}, year={2000}}
@book{b, author={Smith, John}, title={T2}, year={2001}, options={skipbib}}
@book{c, author={Jones, Bob},  title={T3}, year={2000}}
@book{d, author={Jones, Bob},  title={T4}, year={2001}, options={skipbib=true}}
@book{e, author={Brown, Al},   title={T5}, year={2000}}
@book{f, author={Brown, Al},   title={T6}, year={2001}, options={dataonly}}
```

```latex
\documentclass{article}
\usepackage[style=authoryear,singletitle=true]{biblatex}
\addbibresource{test.bib}
\begin{document}
\cite{a,b,c,d,e,f}
\printbibliography
\end{document}
```

biber 2.22 and current `dev` (1686b9c) give `\true{singletitle}` to `a` only. `c` and `e` are in exactly the same situation (one bibliography work by the author, plus one entry that is not printed), but `d` (`skipbib=true`) and `f` (`dataonly`, which implies `skipbib`) are counted, so `c` and `e` lose `singletitle`. The same check guards `uniquetitle`, `uniquebaretitle` and `uniquework`.

### Suggested fix

Use the processed per-entry option instead of the raw field. This also covers set members, which get `skipbib` from `process_sets`:

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -2199,11 +2199,10 @@
   # ignore settings from inheritance data?
   my $ignore = Biber::Config->get_uniq_ignore($citekey);
 
-  # If this is a data clone with skipbib set, don't record uniqueness info
-  if (my $options = $be->get_field('options')) {
-    if (grep {$_ eq 'skipbib'} $options->@*) {
-      return;
-    }
+  # Entries not in the bibliography (skipbib in any form, dataonly, set members)
+  # don't take part in work uniqueness
+  if (Biber::Config->getblxoption($secnum, 'skipbib', undef, $citekey)) {
+    return;
   }
 
   # singletitle
```

With this change `a`, `c` and `e` all get `\true{singletitle}`. All tests in `t/` pass on `dev` (1686b9c) with this patch.

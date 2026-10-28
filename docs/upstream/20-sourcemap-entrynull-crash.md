<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/536 (2026-10-28). Tracks BB-133 (docs/biber-bugs.md). -->

**Title:** Sourcemap: `entrynull` after `entrynew`/`entryclone` with `entrynocite` crashes biber ("Can't call method "get_field" on an undefined value")

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

A map that creates a new entry from the current one and then drops the current one crashes biber:

```latex
\documentclass{article}
\usepackage{biblatex}
\DeclareSourcemap{\maps[datatype=bibtex]{
  \map{
    \step[fieldsource=entrykey, match=\regexp{\Ab\Z}, final]
    \step[entrynew=bnew, entrynewtype=misc, entrynocite]
    \step[entrynull]
  }
}}
\addbibresource{test.bib}
\begin{document}
\cite{b}
\printbibliography
\end{document}
```

```bibtex
@misc{a, title = {T}}
@misc{b, title = {U}}
```

```
Can't call method "get_field" on an undefined value at .../Biber.pm line 4756.
```

(biber 2.22; line 4742 on current `dev`, 012ba95.) The `.bbl` is left empty.

The `entrynew` step adds `bnew` to the section's citekeys straight away (because of `entrynocite`, and likewise in any `\nocite{*}` section), but the entry itself is only created at the end of `create_entry`. The `entrynull` step `return`s from `create_entry` before that, so `bnew` is cited but never created, and `get_dependents` dereferences the missing entry. `entryclone` behaves the same way.

### Suggested fix

Create the entries made by earlier steps before returning:

```diff
--- a/lib/Biber/Input/file/bibtex.pm
+++ b/lib/Biber/Input/file/bibtex.pm
@@ -441,6 +441,11 @@
               if ($logger->is_debug()) { # performance tune
                 $logger->debug("Source mapping (type=$level, key=$key): Ignoring entry completely");
               }
+              # Entries created by earlier steps are still created as their keys
+              # may already have been added to the section
+              while (my ($k, $e) = each %newentries) {
+                _create_entry($k, $e);
+              }
               return 0;         # don't create an entry at all
             }
 
```

With this change the example writes `bnew` to the `.bbl`, and gives the existing warning `Entry with key 'b' in section '0' is cited and found but not created (likely due to sourcemap)`. All tests in `t/` pass on `dev` (012ba95) with this patch.

<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/532 (2026-10-28). Tracks BB-004 (docs/biber-bugs.md). -->

**Title:** `presort` values are used as a regex: `presort = {a(b}` crashes biber, and `|`, `+`, `?` give the wrong `sortinit`

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

To compute `sortinit`, `_generatesortinfo` strips the `presort` value off the front of the sort string (Internals.pm:1267):

```perl
    $ss =~ s/\A$pre$sorting_sep+//;
```

`$pre` is interpolated as a regex, unescaped. With

```bibtex
@book{p1, author = {Bee, A}, title = {T}, presort = {PRESORT}}
@book{p2, author = {Cee, A}, title = {U}}
```

and `sorting=nyt`, the `sortinit` of `p1` should be `B` whatever the `presort` value. biber 2.22 gives:

| `presort` | biber 2.22 |
|---|---|
| `mm`, `\d` | `\field{sortinit}{B}` |
| `a(b` | dies: `Unmatched ( in regex; marked by <-- HERE in m/\Aa( <-- HERE b,+/ at …/Biber/Internals.pm line 1267.` |
| `[` | dies: `Unmatched [ in regex` |
| `a\|b` | `\field{sortinit}{b}` |
| `zz\|a` | `\field{sortinit}{a}` |
| `a+`, `x?` | `\field{sortinit}{a}` / `{x}` (the prefix isn't stripped) |

A wrong `sortinit` puts the entry under the wrong heading with `\bibbysortinit`-style headings and changes `sortinithash`. Current `dev` (012ba95) behaves the same.

### Minimal example

```bibtex
@book{p1, author = {Bee, A}, title = {T}, presort = {a(b}}
@book{p2, author = {Cee, A}, title = {U}}
```

```latex
\documentclass{article}
\usepackage[sorting=nyt]{biblatex}
\addbibresource{test.bib}
\begin{document}
\nocite{*}
\printbibliography
\end{document}
```

### Suggested fix

```diff
--- a/lib/Biber/Internals.pm
+++ b/lib/Biber/Internals.pm
@@ -1264,7 +1264,7 @@
     my $pre = Biber::Config->getblxoption($secnum, 'presort', $be->get_field('entrytype'), $citekey);
 
     # Strip off the prefix
-    $ss =~ s/\A$pre$sorting_sep+//;
+    $ss =~ s/\A\Q$pre\E$sorting_sep+//;
     my $init = Unicode::GCString->new(normalise_string($ss))->substr(0, 1)->as_string;
     $dlist->set_sortinitdata_for_key($citekey, $init);
   }
```

With this change, every row of the table gives `\field{sortinit}{B}`. All tests in `t/` pass on `dev` (012ba95) with this patch.

<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/523 (2026-10-24). Tracks BB-016 (docs/biber-bugs.md). -->

**Title:** Range length wrong when an abbreviated range end contains a 0 (`pages = {315-20}` gives `\range{pages}{11}`)

---

For abbreviated ranges such as `22-4` or `135-38`, `rangelen` fills in the missing leading digits of the end from the start. A `0` digit in the end is false in Perl, though, so it is treated as missing and overwritten:

| `pages` | biber 2.22 | expected |
|---|---|---|
| `105-10` | `\range{pages}{11}` | 6 |
| `315-20` | `\range{pages}{11}` | 6 |
| `1203-08` | `\range{pages}{6}` | 6 (correct only because the overwritten digit is also `0`) |

Abbreviated page ranges ending in `0` (`315-20`, `101-10`) are common in Chicago-style data.

### Minimal example

```bibtex
@article{a, author = {Author, A.}, title = {T}, journaltitle = {J}, date = {2000}, pages = {315-20}}
```

```latex
\documentclass{article}
\usepackage{biblatex}
\addbibresource{test.bib}
\begin{document}
\nocite{*}
\printbibliography
\end{document}
```

The `.bbl` contains `\range{pages}{11}`.

### Suggested fix

```diff
--- a/lib/Biber/Utils.pm
+++ b/lib/Biber/Utils.pm
@@ -1662,7 +1662,7 @@
           my @m = reverse split(//,$m);
           my @n = reverse split(//,$n);
           for (my $i=0;$i<=$#m;$i++) {
-            next if $n[$i];
+            next if defined $n[$i];
             $n[$i] = $m[$i];
           }
           $n = join('', reverse @n);
```

With this change, `105-10` and `315-20` give 6, and `1203-08`, `1995-2001` (7) and `99-100` (2) are unchanged. (biber 2.22 from source; the code is unchanged on `dev`, df6438f.)

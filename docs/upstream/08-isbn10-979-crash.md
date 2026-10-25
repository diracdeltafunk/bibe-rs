<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/527 (2026-10-25). Tracks BB-130 (docs/biber-bugs.md). -->

**Title:** `--isbn10` silently kills biber when an entry has a 979 ISBN (empty `.bbl`, no error message)

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

ISBNs with the 979 prefix (common since about 2020, e.g. in France, Korea and the US) have no ISBN-10 form. `Business::ISBN::as_isbn10` returns `undef` for them, and biber then calls `->isbn` on it:

```perl
    elsif (Biber::Config->getoption('isbn10')) {
      $isbn = $isbn->as_isbn10;
      $value = $isbn->isbn;
```

The `die` happens while STDERR is redirected to the btparse temp file, so nothing is shown. biber 2.22 (TeX Live binary) stops after `INFO - Found BibTeX data source 'test.bib'` with exit code 102, no ERROR, and a 0-byte `.bbl`.

### Minimal example

```bibtex
@book{a, author = {Doe, John}, title = {Alpha}, date = {2020}, isbn = {979-10-90636-07-1}}
@book{b, author = {Roe, Rita}, title = {Beta},  date = {2021}, isbn = {978-0-306-40615-7}}
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

Run `biber --isbn10 test` (the option can also come from `biber.conf`).

### Suggested fix

Keep the ISBN-13 when there is no ISBN-10, and say so:

```diff
--- a/lib/Biber/Input/file/bibtex.pm
+++ b/lib/Biber/Input/file/bibtex.pm
@@ -1089,8 +1089,14 @@
       $value = $isbn->isbn;
     }
     elsif (Biber::Config->getoption('isbn10')) {
-      $isbn = $isbn->as_isbn10;
-      $value = $isbn->isbn;
+      # 979 ISBNs have no ISBN-10 form
+      if (my $isbn10 = $isbn->as_isbn10) {
+        $isbn = $isbn10;
+        $value = $isbn->isbn;
+      }
+      else {
+        biber_warn("ISBN '$value' in entry '$key' has no ISBN-10 form - leaving it as ISBN-13");
+      }
     }
 
     # Normalise if requested
```

With this change biber warns about `a` and keeps `979-10-90636-07-1`, and still converts `b` to `0306406152`. With `--isbn-normalise` as well, `b` becomes `0-306-40615-2`.

A side note: any `die` inside `extract_entries` is invisible in the same way, because STDERR still points at the temp file. The comment at bibtex.pm:155-162 mentions this. Restoring STDERR in a `local $SIG{__DIE__}` handler, or around the per-entry processing, would make such crashes visible.

Tested on biber 2.22 built from source. With this patch and the five others I'm filing alongside it, all 1,275 tests in `t/` pass. The code is unchanged on `dev` (d6e35e7), where the patch applies cleanly.

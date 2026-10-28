<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/533 (2026-10-28). Tracks BB-007, BB-070 and BB-145 (docs/biber-bugs.md). -->

**Title:** A refsection with citations but no data sources crashes biber when another refsection has data sources; with no data sources at all the warning is never shown

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

**1. Crash.** If any refsection has a data source, biber does not stop early, and `fetch_data` then dereferences the undefined data source list of a section that has none (Biber.pm:4595 on `dev`):

```latex
\documentclass{article}
\usepackage{biblatex}
\begin{document}
\cite{knuth}
\begin{refsection}[refs.bib]
\cite{lamport}
\printbibliography
\end{refsection}
\printbibliography
\end{document}
```

(`refs.bib` contains an entry `lamport`.)

```
Can't use an undefined value as an ARRAY reference at .../Biber.pm line 4595.
```

The `.bbl` is empty (exit code 25 for the TeX Live 2.22 binary), so section 1's bibliography is lost too. This is easy to hit by forgetting `\addbibresource` when using `\begin{refsection}[file.bib]` for some sections.

**2. Silent exit.** With no data sources in any section (the same document without the `refsection`), `parse_ctrlfile` does

```perl
  unless (%bibdatasources) {
    biber_warn("No data sources defined!");
    exit EXIT_OK;
  }
```

but `biber_warn` only queues the warning, and the queue is printed by `display_end`, which is never reached. The user gets an empty `.bbl` with exit code 0 and no message at all on the terminal or in the `.blg`.

**3. (Found while reading the same code.)** In `fetch_data`, a data source whose type isn't `file` is pushed onto the list and then *also* passed through `glob_data_file` and pushed again.

Same behaviour on biber 2.22 and current `dev` (012ba95).

### Suggested fix

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -901,6 +901,7 @@
   # Be friendly to latexmk etc.
   unless (%bibdatasources) {
     biber_warn("No data sources defined!");
+    $self->display_end;
     exit EXIT_OK;
   }
 
@@ -4610,10 +4611,11 @@
   }
 
   # Process datasource globs
-  my $ds;
-  foreach my $datasource ($section->get_datasources->@*) {
+  my $ds = [];
+  foreach my $datasource (($section->get_datasources // [])->@*) {
     unless ($datasource->{type} eq 'file') {
       push $ds->@*, $datasource;
+      next;
     }
     foreach my $gds (glob_data_file($datasource->{name}, $datasource->{glob})) {
       push $ds->@*, { type     => $datasource->{type},
```

With this change, example 1 gives `WARN - I didn't find a database entry for 'knuth' (section 0)` and section 1 is written normally; example 2 prints `WARN - No data sources defined!` and `INFO - WARNINGS: 1` and still exits 0. All tests in `t/` pass on `dev` (012ba95) with this patch.

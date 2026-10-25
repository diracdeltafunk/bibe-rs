<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/524 (2026-10-25). Tracks BB-040 (docs/biber-bugs.md). -->

**Title:** `@preamble` in a later `.bib` file is dropped once all cited keys have been found

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

When a document has several data sources, biber stops reading them as soon as every cited key has been found. The `@preamble`s in the files it doesn't read never reach the `.bbl`. Whether the `\preamble` block is complete therefore depends on which entries are cited. BibTeX writes the preambles of every file.

This bites when macros live in a separate file, a common way to share `\noopsort` and similar definitions:

### Minimal example

`a.bib`:

```bibtex
@book{a, author = {Author, Anne}, title = {\noopsort{1}Alpha}, date = {2000}}
```

`b.bib`:

```bibtex
@preamble{ "\providecommand{\noopsort}[1]{}" }
@book{b, author = {Bauer, Bert}, title = {Beta}, date = {2001}}
```

```latex
\documentclass{article}
\usepackage{biblatex}
\addbibresource{a.bib}
\addbibresource{b.bib}
\begin{document}
\cite{a}
\printbibliography
\end{document}
```

`a` is found in `a.bib`, so `b.bib` is never read. The `.bbl` has no `\preamble`, and the next LaTeX run fails with `! Undefined control sequence. \noopsort`. With `\cite{b}` instead, the `.bbl` has the preamble.

### Suggested fix

Drop the early exit, so that every data source is read:

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -4619,8 +4619,6 @@
 
   # Now actually fetch data with expanded list of data sources
   foreach my $datasource ($section->get_datasources->@*) {
-    # shortcut if we have found all the keys now
-    last unless (@remaining_keys or $section->is_allkeys);
     my $type = $datasource->{type};
     my $name = $datasource->{name};
     my $encoding = $datasource->{encoding};
```

`extract_entries` already handles an empty key list (it parses the file, collects the preambles and finds nothing). With this change the example gets its `\preamble` and compiles. The rest of the `.bbl` is unchanged. The cost is parsing the remaining data files when the cited keys come early. If that matters, the loop could keep going only to collect preambles.

Tested on biber 2.22 built from source. With this patch and the five others I'm filing alongside it, all 1,275 tests in `t/` pass. The code is unchanged on `dev` (d6e35e7), where the patch applies cleanly.

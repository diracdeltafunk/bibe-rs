<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/520 (2026-10-24). Tracks BB-149 (docs/biber-bugs.md). -->

**Title:** biber hangs forever when a name list contains an empty name (e.g. `author = {Doe, Jane and ,}`)

---

biber never terminates when a name in the labelname list has no name parts. The `.bbl` stays empty, and nothing is written to the log after the data source is read. A stray comma in a name list is enough, which is easy to get from exported or hand-edited `.bib` files.

### Minimal example

`test.bib`:

```bibtex
@misc{key,
  author = {Doe, Jane and ,},
  title  = {Title},
}
```

`test.tex`:

```latex
\documentclass{article}
\usepackage[style=authoryear]{biblatex}
\addbibresource{test.bib}
\begin{document}
\cite{key}
\printbibliography
\end{document}
```

`pdflatex test && biber test`: biber 2.22 (TeX Live 2026, macOS) is still running when killed after 30 seconds. The last log line is `Found BibTeX data source 'test.bib'`, and `test.bbl` is left empty.

Other inputs that hang the same way:
- `author = {,}`;
- `author = {family=Doe, given="Ja}`;
- a granular XDATA reference to a missing entry, `author = {xdata=missing-author}`.

### Cause

In `Biber::generate_uniquename`, a name with no name parts has empty `namestrings` and `namedisschema`. So it ends up calling

```perl
$dlist->set_uniquename($nlid, $nid, $namedisschema->[0])   # undef
```

and `Biber::DataList::set_uniquename` sets the "changed" flag whenever the current value is undefined:

```perl
if (not defined($currval) or not Compare($currval, $s)) {
  $self->set_unul_changed(1);
}
```

The stored value stays `undef`, so every pass of the `while ('true')` loop in `Biber::uniqueness` sees a change and runs again.

### Suggested fix

`Data::Compare` already treats `undef`/`undef` as equal and `undef`/value as different, so the `defined` test can go:

```diff
--- a/lib/Biber/DataList.pm
+++ b/lib/Biber/DataList.pm
@@ -495,7 +495,7 @@
 
   my $currval = $self->{state}{namelistdata}{$nlid}{$nid}{un};
   # Set modified flag to positive if we changed something
-  if (not defined($currval) or not Compare($currval, $s)) {
+  if (not Compare($currval, $s)) {
     $self->set_unul_changed(1);
   }
   $self->{state}{namelistdata}{$nlid}{$nid}{un} = $s;
```

With this change, all of the inputs above terminate and produce a `.bbl`. I also ran the patched biber (from source, perl 5.42) over 776 test documents: the biblatex example files, the biber test suite's control files, and documents for the standard and contributed styles. The output was byte-identical to unpatched 2.22 in every case.

The code is unchanged on `dev` (df6438f). As an extra safeguard, the loop in `Biber::uniqueness` could also stop with a warning after some large number of passes, since real documents converge in two or three.

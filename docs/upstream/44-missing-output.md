<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-049, BB-093 (docs/biber-bugs.md). -->

**Title:** `.bbl` loses information: no `\refsection` block (and so no `\missing`) for a section without found entries; related-entry clones lose `endyeardivision`

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biber 2.22 and `dev` (1686b9c).

**1. A refsection whose cited keys are all missing gets no block in the `.bbl`.** With

```latex
\documentclass{article}
\usepackage[backend=biber]{biblatex}
\addbibresource{alias.bib}   % contains only @book{x, author={A, B}, title={T}}
\begin{document}
\nocite{nonexistent}
\end{document}
```

biber warns `I didn't find a database entry for 'nonexistent' (section 0)`, but the `.bbl` contains no `\refsection{0}` block at all, so the `\missing{nonexistent}` line that biber would otherwise write for biblatex is lost. The same happens for any refsection in which no cited key is found. `Output::bbl::output` loops over `keys $data->{ENTRIES}`, and a section only gets an `ENTRIES` slot when at least one entry is written; the missing keys and aliases of the section are kept separately in `MISSING_ENTRIES` and `ALIAS_ENTRIES`.

**2. Clones made for `related` entries lose the end date's season.** With

```bibtex
@book{a, author={Doe, John}, title={Main}, date={2020}, related={b}, relatedtype={reprintof}}
@book{b, author={Roe, Jane}, title={Orig}, date={2001-21/2002-23}}
```

and `\cite{a,b}`, the `.bbl` entry for `b` has `\field{yeardivision}{spring}` and `\field{endyeardivision}{autumn}`, but the clone of `b` that biber creates for the related list (key `92eb5ffee6ae2fec3ad71c777531578f`) has only `\field{yeardivision}{spring}`. The list of derived date fields copied in `Entry::clone` contains `'yeardivision'` twice and no `'endyeardivision'`.

### Suggested fix

```diff
--- a/lib/Biber/Entry.pm
+++ b/lib/Biber/Entry.pm
@@ -176,7 +176,7 @@
     my $df = $datefield =~ s/date$//r;
     foreach my $dsf ('dateunspecified', 'datesplit', 'datejulian',
                      'enddatejulian', 'dateapproximate', 'enddateapproximate',
-                     'dateuncertain', 'enddateuncertain', 'yeardivision', 'yeardivision',
+                     'dateuncertain', 'enddateuncertain', 'yeardivision', 'endyeardivision',
                      'era', 'endera') {
       if (my $ds = $self->{derivedfields}{"$df$dsf"}) {
         $new->{derivedfields}{"$df$dsf"} = $ds;
--- a/lib/Biber/Output/bbl.pm
+++ b/lib/Biber/Output/bbl.pm
@@ -718,7 +718,9 @@
 
   out($target, $data->{HEAD});
 
-  foreach my $secnum (sort keys $data->{ENTRIES}->%*) {
+  # A section with no found entries still needs its block for its missing keys and aliases
+  my %secnums = map {$_ => 1} map {keys(($data->{$_} // {})->%*)} qw(ENTRIES ALIAS_ENTRIES MISSING_ENTRIES);
+  foreach my $secnum (sort keys %secnums) {
     if ($logger->is_debug()) {# performance tune
       $logger->debug("Writing entries for section $secnum");
     }
```

With this patch, the first example gives

```latex
\refsection{0}
  \missing{nonexistent}
\endrefsection
```

and the clone in the second example also has `\field{endyeardivision}{autumn}`. All tests in `t/` pass on `dev` (1686b9c) with this patch.

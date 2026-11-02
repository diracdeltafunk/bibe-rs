<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/546 (2026-11-02). Tracks BB-028 (docs/biber-bugs.md). -->

**Title:** Loops over `$dmh->{datefields}` strip `date` from the shared data-model array in place: custom date fields lose their flags, and `--output-field-order` treats `URL` as a date

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Several loops do

```perl
foreach my $d ($dmh->{datefields}->@*) {
  $d =~ s/date$//;
```

The loop variable is an alias, so this rewrites the data-model helper array itself (`date` → `''`, `urldate` → `url`, …), and every later user of `$dmh->{datefields}` sees the stems instead of the field names. The sites are `Output/bbl.pm:458`, `Output/bibtex.pm:235`, `Entry.pm:176`, `697` and `1118` (line numbers on current `dev`, 79a1edc). This has two visible effects.

**1. A date field whose stem itself ends in `date` only works for the first entry.** With a data model that adds `updatedate`:

```latex
% bb028.dbx
\DeclareDatamodelFields[type=field,datatype=date,skipout]{updatedate}
\DeclareDatamodelEntryfields{updatedate}
```

```latex
\documentclass{article}
\usepackage[style=authoryear,datamodel=bb028]{biblatex}
\addbibresource{test.bib}
\begin{document}
\nocite{*}
\printbibliography
\end{document}
```

```bibtex
@misc{u1, author={Aa, A}, title={One},   date={2000}, updatedate={2010~}}
@misc{u2, author={Bb, B}, title={Two},   date={2001}, updatedate={2012~}}
@misc{u3, author={Cc, C}, title={Three}, date={2002}, updatedate={2014~}}
```

biber 2.22 and `dev` write `\true{updatedatecirca}` and `\field{updatedateera}{ce}` for `u1` only. Writing `u1` turns `updatedate` into `update` in the shared array, and writing `u2` turns `update` into `up`, so the flags of `u2` and `u3` are looked up as `updatecirca`, `updateera`, … instead of `updatedatecirca`, `updatedateera`, ….

**2. In tool mode the `dates` class of `--output-field-order` matches the wrong fields.** By the time the fields are ordered, the array holds `''`, `url`, `orig`, … instead of `date`, `urldate`, `origdate`, … So with the default order (`options,abstract,names,lists,dates`), `URL` is placed with the dates and `DATE` is not:

```bibtex
@misc{a, author={Doe, J}, title={T}, date={2000}, url={http://x.org}, abstract={Abs}, annotation={Ann}}
```

`biber --tool --noconf test.bib` writes

```
@MISC{a,
  ABSTRACT = {Abs},
  AUTHOR = {Doe, J},
  URL = {http://x.org},
  ANNOTATION = {Ann},
  DATE = {2000},
  TITLE = {T},
}
```

where `DATE` should come right after `AUTHOR` and `URL` should be with the remaining fields in alphabetical order. The expected outputs in `t/bibtex-output.t` and `t/tdata/full-bibtex_biber.bib` (checked by `t/full-bibtex.t`) currently encode the wrong position of `DATE`.

### Suggested fix

Compute the stem in a new variable everywhere (as `Entry.pm:1068` already does), and correct the two test expectations:

```diff
--- a/lib/Biber/Entry.pm
+++ b/lib/Biber/Entry.pm
@@ -172,8 +172,8 @@
   $new->{xdatarefs} = $self->{xdatarefs};
 
   # clone derived date fields
-  foreach my $df ($dmh->{datefields}->@*) {
-    $df =~ s/date$//;
+  foreach my $datefield ($dmh->{datefields}->@*) {
+    my $df = $datefield =~ s/date$//r;
     foreach my $dsf ('dateunspecified', 'datesplit', 'datejulian',
                      'enddatejulian', 'dateapproximate', 'enddateapproximate',
                      'dateuncertain', 'enddateuncertain', 'yeardivision', 'yeardivision',
@@ -693,8 +693,8 @@
 
   # Datesplit is a special non datafield and needs to be inherited for any
   # validation checks which may occur later
-  foreach my $df ($dmh->{datefields}->@*) {
-    $df =~ s/date$//;
+  foreach my $datefield ($dmh->{datefields}->@*) {
+    my $df = $datefield =~ s/date$//r;
     if (my $ds = $parent->get_field("${df}datesplit")) {
       $self->set_field("${df}datesplit", $ds);
     }
@@ -1122,8 +1122,8 @@
   }
   # Datesplit is a special non datafield and needs to be inherited for any
   # validation checks which may occur later
-  foreach my $df ($dmh->{datefields}->@*) {
-    $df =~ s/date$//;
+  foreach my $datefield ($dmh->{datefields}->@*) {
+    my $df = $datefield =~ s/date$//r;
     if (my $ds = $parent->get_field("${df}datesplit")) {
       $self->set_field("${df}datesplit", $ds);
     }
--- a/lib/Biber/Output/bbl.pm
+++ b/lib/Biber/Output/bbl.pm
@@ -455,8 +455,8 @@
   }
 
   # Date meta-information
-  foreach my $d ($dmh->{datefields}->@*) {
-    $d =~ s/date$//;
+  foreach my $datefield ($dmh->{datefields}->@*) {
+    my $d = $datefield =~ s/date$//r;
 
     # Unspecified granularity
     if (my $unspec = $be->get_field("${d}dateunspecified")) {
--- a/lib/Biber/Output/bibtex.pm
+++ b/lib/Biber/Output/bibtex.pm
@@ -231,8 +231,8 @@
   $acc{$outmap->('options')} = join(',', @entryoptions) if @entryoptions;
 
   # Date fields
-  foreach my $d ($dmh->{datefields}->@*) {
-    $d =~ s/date$//;
+  foreach my $datefield ($dmh->{datefields}->@*) {
+    my $d = $datefield =~ s/date$//r;
     next unless $be->get_field("${d}year");
 
     # Output legacy dates for YEAR/MONTH if requested
--- a/t/bibtex-output.t
+++ b/t/bibtex-output.t
@@ -54,8 +54,8 @@
 
 my $b1 = q|@ARTICLE{murray,
   AUTHOR       = {Hostetler, Michael J. and Wingate, Julia E. and Zhong, Chuan-Jian and Harris, Jay E. and Vachet, Richard W. and Clark, Michael R. and Londono, J. David and Green, Stephen J. and Stokes, Jennifer J. and Wignall, George D. and Glish, Gary L. and Porter, Marc D. and Evans, Neal D. and Murray, Royce W.},
-  ANNOTATION   = {An \texttt{article} entry with \arabic{author} authors. By default, long author and editor lists are automatically truncated. This is configurable},
   DATE         = {1998},
+  ANNOTATION   = {An \texttt{article} entry with \arabic{author} authors. By default, long author and editor lists are automatically truncated. This is configurable},
   INDEXTITLE   = {Alkanethiolate gold cluster molecules},
   JOURNALTITLE = {Langmuir},
   LANGID       = {english},
--- a/t/tdata/full-bibtex_biber.bib
+++ b/t/tdata/full-bibtex_biber.bib
@@ -15,8 +15,8 @@
 @ARTICLE{kastenholz,
   ABSTRACT     = {The computation of ionic solvation free energies from atomistic simulations is a surprisingly difficult problem that has found no satisfactory solution for more than 15 years. The reason is that the charging free energies evaluated from such simulations are affected by very large errors. One of these is related to the choice of a specific convention for summing up the contributions of solvent charges to the electrostatic potential in the ionic cavity, namely, on the basis of point charges within entire solvent molecules (M scheme) or on the basis of individual point charges (P scheme). The use of an inappropriate convention may lead to a charge-independent offset in the calculated potential, which depends on the details of the summation scheme, on the quadrupole-moment trace of the solvent molecule, and on the approximate form used to represent electrostatic interactions in the system. However, whether the M or P scheme (if any) represents the appropriate convention is still a matter of on-going debate. The goal of the present article is to settle this long-standing controversy by carefully analyzing (both analytically and numerically) the properties of the electrostatic potential in molecular liquids (and inside cavities within them).},
   AUTHOR       = {Kastenholz, M. A. and Hünenberger, Philippe H.},
-  ANNOTATION   = {An \texttt{article} entry with an \texttt{eid} and a \texttt{doi} field. Note that the \textsc{doi} is transformed into a clickable link if \texttt{hyperref} support has been enabled},
   DATE         = {2006},
+  ANNOTATION   = {An \texttt{article} entry with an \texttt{eid} and a \texttt{doi} field. Note that the \textsc{doi} is transformed into a clickable link if \texttt{hyperref} support has been enabled},
   DOI          = {10.1063/1.2172593},
   EID          = {124106},
   INDEXTITLE   = {Computation of ionic solvation free energies},
@@ -32,10 +32,10 @@
   EDITOR       = {Westfahl, Gary},
   LOCATION     = {Westport, Conn. and London},
   PUBLISHER    = {Greenwood},
+  DATE         = {2000},
   ANNOTATION   = {This is a \texttt{collection} entry. Note the format of the \texttt{location} field as well as the \texttt{subtitle} and \texttt{booksubtitle} fields},
   BOOKSUBTITLE = {The Frontier Theme in Science Fiction},
   BOOKTITLE    = {Space and Beyond},
-  DATE         = {2000},
   LANGID       = {english},
   LANGIDOPTS   = {variant=american},
   SUBTITLE     = {The Frontier Theme in Science Fiction},
```

With this change all three entries get their `updatedate` flags, the tool output puts `DATE` after `AUTHOR` and `URL` after `TITLE`, and all tests in `t/` pass on `dev` (79a1edc), including the end-to-end tests run with `BIBER_DEV_TESTS=1`, with this patch applied together with the others I'm filing alongside it.

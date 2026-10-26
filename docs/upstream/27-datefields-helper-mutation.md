<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-028 (docs/biber-bugs.md). -->

**Title:** Loops over `$dmh->{datefields}` strip `date` from the shared data-model array in place: custom date fields lose their flags, and `--output-field-order` treats `URL` as a date

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Several loops do

```perl
foreach my $d ($dmh->{datefields}->@*) {
  $d =~ s/date$//;
```

The loop variable is an alias, so this rewrites the data-model helper array itself (`date` → `''`, `urldate` → `url`, …), and every later user of `$dmh->{datefields}` sees the stems instead of the field names. The sites are `Output/bbl.pm:458`, `Output/bibtex.pm:234`, `Entry.pm:175`, `696` and `1125` (line numbers on current `dev`, 1686b9c). This has two visible effects.

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

where `DATE` should come right after `AUTHOR` and `URL` should be with the remaining fields in alphabetical order. The expected output in `t/bibtex-output.t` currently encodes the wrong position of `DATE`.

### Suggested fix

Compute the stem in a new variable everywhere (as `Entry.pm:1076` already does), and correct the test:

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
```

With this change all three entries get their `updatedate` flags, the tool output puts `DATE` after `AUTHOR` and `URL` after `TITLE`, and all tests in `t/` pass on `dev` (1686b9c).

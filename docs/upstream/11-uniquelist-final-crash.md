<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-001 (docs/biber-bugs.md). -->

**Title:** uniquelist crashes ("Can't use string ("1") as a HASH ref") when a name list starts with the family name `final`

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

With `uniquelist` on, biber keeps two kinds of counts in the same hash, `$self->{state}{uniquelistcount}{global}`:

* `add_uniquelistcount` counts partial name lists, keyed by the joined name strings (`{global}{"Smith"}`, `{global}{"Smith\x{10FFFD}Jones"}`, …);
* `add_uniquelistcount_final` counts complete lists under the sub-hash `{global}{final}`.

A partial list consisting of the single name `final` therefore lands on the same key as the sub-hash. Depending on which is written first, biber dies in `add_uniquelistcount_final` (DataList.pm:190) or `add_uniquelistcount` (DataList.pm:177):

```
Can't use string ("1") as a HASH ref while "strict refs" in use at .../Biber/DataList.pm line 190.
```

The `.bbl` is left empty. `final` is not a likely surname, but it does occur in data (e.g. as a corporate or placeholder author), and nothing in the input tells the user why biber died.

The same sharing also affects `uniquelist=minyear`: the per-year counts live in `{global}{final}{$labelyear}`, i.e. in the same hash as the complete lists, so `namelist_differs_index` iterates over the years as if they were one-name lists.

### Minimal example

```bibtex
@book{a, author = {final}, title = {T1}, year = {2000}}
@book{b, author = {final and Smith, John}, title = {T2}, year = {2001}}
```

```latex
\documentclass{article}
\usepackage[style=authoryear,uniquelist=true,uniquename=true,maxcitenames=1]{biblatex}
\addbibresource{test.bib}
\begin{document}
\cite{a,b}
\printbibliography
\end{document}
```

biber 2.22 and current `dev` (1686b9c) both die as above (exit code 25 for the TeX Live binary).

### Suggested fix

Give the three kinds of counts their own hashes:

```diff
--- a/lib/Biber/DataList.pm
+++ b/lib/Biber/DataList.pm
@@ -163,7 +163,7 @@
 
 sub get_uniquelistcount {
   my ($self, $namelist) = @_;
-  return $self->{state}{uniquelistcount}{global}{join("\x{10FFFD}", $namelist->@*)};
+  return $self->{state}{uniquelistcount}{partial}{join("\x{10FFFD}", $namelist->@*)};
 }
 
 =head2 add_uniquelistcount
@@ -174,7 +174,7 @@
 
 sub add_uniquelistcount {
   my ($self, $namelist) = @_;
-  $self->{state}{uniquelistcount}{global}{join("\x{10FFFD}", $namelist->@*)}++;
+  $self->{state}{uniquelistcount}{partial}{join("\x{10FFFD}", $namelist->@*)}++;
   return;
 }
 
@@ -187,9 +187,9 @@
 
 sub add_uniquelistcount_final {
   my ($self, $namelist, $labelyear) = @_;
-  $self->{state}{uniquelistcount}{global}{final}{join("\x{10FFFD}", $namelist->@*)}++;
+  $self->{state}{uniquelistcount}{final}{join("\x{10FFFD}", $namelist->@*)}++;
   if ($labelyear) { # uniquelist=minyear
-    $self->{state}{uniquelistcount}{global}{final}{$labelyear}{join("\x{10FFFD}", $namelist->@*)}++;
+    $self->{state}{uniquelistcount}{finalyear}{$labelyear}{join("\x{10FFFD}", $namelist->@*)}++;
   }
   return;
 }
@@ -229,7 +229,7 @@
 
 sub get_uniquelistcount_final {
   my ($self, $namelist) = @_;
-  my $c = $self->{state}{uniquelistcount}{global}{final}{join("\x{10FFFD}", $namelist->@*)};
+  my $c = $self->{state}{uniquelistcount}{final}{join("\x{10FFFD}", $namelist->@*)};
   return $c // 0;
 }
 
@@ -2249,7 +2249,7 @@
   my $self = shift;
   my @list = shift->@*;
   my $index;
-  foreach my $l_s (keys $self->{state}{uniquelistcount}{global}{final}->%*) {
+  foreach my $l_s (keys $self->{state}{uniquelistcount}{final}->%*) {
     my @l = split("\x{10FFFD}", $l_s);
     next if Compare(\@list, \@l);# Ignore identical lists
     for (my $i=0;$i<=$#list;$i++) {
@@ -2303,9 +2303,9 @@
 
   # uniquelist=minyear should only disambiguate from entries with the
   # same labelyear
-  my $unames = $self->{state}{uniquelistcount}{global}{final};
+  my $unames = $self->{state}{uniquelistcount}{final};
   if ($ul eq 'minyear') {
-    $unames = $self->{state}{uniquelistcount}{global}{final}{$labelyear};
+    $unames = $self->{state}{uniquelistcount}{finalyear}{$labelyear};
   }
 
   foreach my $l_s (keys $unames->%*) {
```

With this change the example runs, and its `.bbl` is the same (apart from names, hashes and `sortinit`) as for the same data with `final` replaced by `Doe`. All tests in `t/` pass on `dev` (1686b9c) with this patch.

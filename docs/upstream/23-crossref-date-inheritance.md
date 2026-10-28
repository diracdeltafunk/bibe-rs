<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-022 and BB-023 (docs/biber-bugs.md). -->

**Title:** Crossref date inheritance: a child with its own date gets the parent's `datecirca`/era/…, and a child `origdate = {2005}` gets the parent's `origmonth`

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Two guards in `Biber::Entry::inherit_from` (Entry.pm:1047-1081 on `dev`) don't work as intended.

**1. Derived date metadata is always inherited (BB-022).** The loop that copies `datecirca`, `dateuncertain`, `dateera`, `datejulian`, `yeardivision`, … from the parent is meant to skip dates the child already has:

```perl
      next if first {$_ eq $datefield} @removed_fields;
```

but `@removed_fields` holds date *part* names (`year`, `month`, …) and `$datefield` is a date *field* name (`date`, `origdate`, …), so this never matches.

**2. The date-part guard only looks at the main date (BB-023).** Date parts are skipped only if `$parent->get_field('datesplit') and $self->get_field('datesplit')`, i.e. only if both entries have a `date`. For `origdate`, `eventdate` and `urldate`, a child that has only some parts of the date gets the remaining parts from the parent.

```bibtex
@mvbook{P, title = {Parent}, date = {1999~}, origdate = {1990-05}, editor = {Ed, Ed}}
@book{C, crossref = {P}, title = {Child},  date = {2000},     author = {Au, Au}}
@book{D, crossref = {P}, title = {Child2}, origdate = {2005}, author = {Bu, Bu}}
```

```latex
\documentclass{article}
\usepackage[style=authoryear]{biblatex}
\addbibresource{test.bib}
\begin{document}
\cite{C,D}
\printbibliography
\end{document}
```

With biber 2.22 and current `dev` (1686b9c):

* `C` has `\field{year}{2000}` and `\true{datecirca}`: its own, exact date is printed as "circa 2000";
* `D` has `\field{origyear}{2005}` and `\field{origmonth}{5}`: the original date becomes May 2005.

### Suggested fix

Check the `…datesplit` flag of the date the part belongs to, and remember which dates were skipped by date type:

```diff
--- a/lib/Biber/Entry.pm
+++ b/lib/Biber/Entry.pm
@@ -1044,16 +1044,17 @@
     # ONLY DO THIS FOR ENTRIES WITH xDATE FIELDS - LEGACY YEAR/MONTH MESS THINGS UP
     # AND WE JUST IGNORE THEM FOR THIS PRE-PROCESSING STEP
     my @filtered_fields;
-    my @removed_fields;
+    my %removed_dates; # date types (eg. '' or 'orig') whose parts we skipped
     foreach my $field (@fields) {
       if (first {$_ eq $field} $dmh->{dateparts}->@*) {
-        if ($parent->get_field('datesplit') and $self->get_field('datesplit')) {
+        my $t = $field =~ s/(?:end)?(?:year|month|day|hour|minute|second|yeardivision|timezone)$//r;
+        if ($parent->get_field("${t}datesplit") and $self->get_field("${t}datesplit")) {
           if ($self->date_fields_exist($field)) {
             if ($override_target eq 'true') {
               $self->delete_date_fields($field); # clear out all date field parts in target
             }
             else {
-              push @removed_fields, $field;
+              $removed_dates{$t} = 1;
               next;
             }
           }
@@ -1068,7 +1069,7 @@
       my $df = $datefield =~ s/date$//r;
       # Ignore derived date special fields from date fields which we have skipped
       # because they already exist in the child.
-      next if first {$_ eq $datefield} @removed_fields;
+      next if $removed_dates{$df};
       foreach my $dsf ('dateunspecified', 'datesplit', 'datejulian',
                        'enddatejulian', 'dateapproximate', 'enddateapproximate',
                        'dateuncertain', 'enddateuncertain', 'yeardivision', 'endyeardivision',
```

With this change, `C` keeps its year without `datecirca`, and `D` has `origyear` 2005 without `origmonth`; nothing else in the two entries changes. All tests in `t/` pass on `dev` (1686b9c) with this patch.

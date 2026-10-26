<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-036, BB-037, BB-039, BB-143 (docs/biber-bugs.md). -->

**Title:** BibTeX field parsing: literal annotations containing `;` are broken up, unsupported `X` dates and `YYYY-20` silently become empty or bare years, and the range parser splits braced values and keeps trailing spaces

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

All four are in `Biber::Input::file::bibtex` and the date helpers, and were checked with biber 2.22 and current `dev` (1686b9c).

**1. Annotations are split on `;` before quoted literals are recognised.**

```bibtex
@misc{a1, title = {T}, title+an:alt = {="literal; x"}}
```

gives `\annotation{field}{title}{alt}{}{}{0}{}`, i.e. an empty, non-literal annotation, instead of `{1}{literal; x}`. The value is split into `="literal` and `x"`; the first is not recognised as a literal, and the second has no `=`, so the regex captures nothing and `set_annotation` is called again with `undef`, overwriting the first.

**2. Unsupported ISO 8601-2 unspecified dates become an empty year without a warning.** `parse_date_unspecified` handles `199X`, `19XX`, `1999-XX`, `1999-01-XX` and `1999-XX-XX`. For any other pattern with an `X` (`199X-05`, `XXXX`, …) none of its branches return, so the sub returns the false value of the last `elsif` test, `$sd` becomes `''`, and the date is treated as an open start:

```bibtex
@misc{e, title={5}, date={199X-05}}
@misc{f, title={6}, date={XXXX}}
```

both give `\field{year}{}` and no message, whereas `date={2000-13}` gives `Invalid format '2000-13' of date field 'date' - ignoring`.

**3. `YYYY-20` is read as a bare year.** The year-division regex in `Biber::Date::Format::_pre` is `([23]\d|4[01])`, i.e. 20–41, but ISO 8601-2 divisions start at 21. `date={2000-20}` gives `year = 2000` with no division and no warning (`2000-21` gives spring, as intended).

**4. The range parser splits braced values and keeps trailing spaces.**

```bibtex
@misc{r6, title={f}, pages = {{12-15}}}
@misc{r8, title={h}, pages = {12~}}
```

`r6` gives `\field{pages}{{12\bibrangedash 15}}`: the braces are meant to protect the value (the code strips `\{…\}` from the start and end afterwards, and the second pattern has `\{[^\}]+\}` alternatives), but the first pattern rejects it because of the hyphen and the second one matches `{12` as the start. `r8` gives `\field{pages}{12 }` and `\range{pages}{-1}`, because `~` is turned into a space and the greedy `(\P{Pd}+)` keeps it; the same happens with trailing spaces in the other patterns.

### Suggested fix

```diff
--- a/lib/Biber/Date/Format.pm
+++ b/lib/Biber/Date/Format.pm
@@ -170,7 +170,7 @@
   }
 
   # ISO8601-2:2016 4.8 (yeardivisions)
-  if ($p{input} =~ s/^(-?\d{4})-([23]\d|4[01])$/$1/) {
+  if ($p{input} =~ s/^(-?\d{4})-(2[1-9]|3\d|4[01])$/$1/) {
     $p{self}{yeardivision} = $Biber::Constants::YEARDIVISIONS{$2};
   }
 
--- a/lib/Biber/Input/file/bibtex.pm
+++ b/lib/Biber/Input/file/bibtex.pm
@@ -1024,8 +1024,15 @@
   }
   $fc =~ s/$ann$//;
 
-  foreach my $a (split(/\s*;\s*/, $value)) {
+  # Split on semicolons outside of quoted literal annotations
+  foreach my $a ($value =~ m/((?:[^;"]|"[^"]*")+)/g) {
+    $a =~ s/\A\s+|\s+\z//g;
+    next unless length($a);
     my ($count, $part, $annotations) = $a =~ /^\s*(\d+)?:?([^=]+)?=(.+)/;
+    unless (defined($annotations)) {
+      biber_warn("Invalid annotation '$a' in field '$field' of entry '$key', ignoring", $bibentry);
+      next;
+    }
     # Is the annotations a literal annotation?
     my $literal = 0;
     if ($annotations =~ m/^\s*"(.+)"\s*$/) {
@@ -1192,10 +1199,10 @@
   foreach my $value (@values) {
     my $ovalue = $value;
     $value =~ s/~/ /g; # Some normalisation for malformed fields
-    $value =~ m/\A\s*(\P{Pd}+)\s*\z/xms ||# Simple value without range
-      $value =~ m/\A\s*(\{[^\}]+\}|[^\p{Pd} ]+)\s*(\p{Pd}+)\s*(\{[^\}]+\}|\P{Pd}*)\s*\z/xms ||
-        $value =~ m/\A\s*(.+)(\p{Pd}{2,})(.+)\s*\z/xms || # M-1--M-4
-          $value =~ m/\A\s*(.+)(\p{Pd}+)(.+)\s*\z/xms;# blah M-1
+    $value =~ m/\A\s*(\{[^\}]+\}|\P{Pd}+?)\s*\z/xms ||# Simple or braced value without range
+      $value =~ m/\A\s*(\{[^\}]+\}|[^\p{Pd} ]+)\s*(\p{Pd}+)\s*(\{[^\}]+\}|\P{Pd}*?)\s*\z/xms ||
+        $value =~ m/\A\s*(.+)(\p{Pd}{2,})(.+?)\s*\z/xms || # M-1--M-4
+          $value =~ m/\A\s*(.+)(\p{Pd}+)(.+?)\s*\z/xms;# blah M-1
         my $start = $1;
     my $end;
     if ($2) {
--- a/lib/Biber/Utils.pm
+++ b/lib/Biber/Utils.pm
@@ -1424,7 +1424,10 @@
 
   my $unspec;
   if ($sd =~ /X/) {# ISO8601-2 4.3 unspecified format
-    ($sd, $sep, $ed, $unspec) = parse_date_unspecified($sd);
+    # Unsupported patterns are left to fail as invalid dates
+    if (my @range = parse_date_unspecified($sd)) {
+      ($sd, $sep, $ed, $unspec) = @range;
+    }
   }
   # Set start date unknown flag
   if ($sep and not $sd) {
@@ -1485,6 +1488,8 @@
   elsif ($d =~ m/^(\d{4})\p{Dash}XX\p{Dash}XX$/) {
     return ("${1}-01-01", '/', "${1}-12-31", 'dayinyear');
   }
+  # Unsupported pattern
+  return;
 }
 
 
```

With this change, `title+an:alt = {="literal; x"}` gives `\annotation{field}{title}{alt}{}{}{1}{literal; x}`, and a stray item without `=` gives a warning instead of erasing the annotation; `199X-05`, `XXXX` and `2000-20` give the usual `Invalid format … - ignoring` warning; and `r6` gives `\field{pages}{12-15}`, `r8` gives `\field{pages}{12}` with `\range{pages}{1}`. Values such as `M-1--M-4`, `12~--~15, 17; 20–22` and `xii--xv` are parsed as before. All tests in `t/` pass on `dev` (1686b9c) with this patch.

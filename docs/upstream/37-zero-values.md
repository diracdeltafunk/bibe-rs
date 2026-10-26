<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-055, BB-057, BB-058, BB-059 (docs/biber-bugs.md). -->

**Title:** A value of `0` is treated as missing: year 0 and `sortkey={0}` sort wrongly, and `month={0}` passes a `rangemin=1` constraint

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Many tests have the form `if (my $x = …)` or `return '' unless $str`, which treat the string `0` like a missing value. `_process_sort_attributes` already has a `BIBERZERO` sentinel "to preserve real zeros", but the values never reach it.

**1. Year 0 sorts after every other year.** With `sorting=nty`:

```bibtex
@book{y0, author={Aa, A}, title={T}, date={0000}}
@book{y1, author={Aa, A}, title={T}, date={0001}}
@book{y2, author={Aa, A}, title={T}, date={2000}}
```

biber 2.22 and `dev` (1686b9c) sort `y1, y2, y0`; it should be `y0, y1, y2` (`0000` is 1 BCE). `_sort_integer` tests `if (my $field = $be->get_field($dmtype))`, so the year is treated as missing. `_sort_literal` and `_sort_verbatim` do the same, so `volume={0}`, `number={0}` etc. are also sorted as missing.

**2. `sortkey={0}` is ignored.** With `sorting=nty` (where `sortkey` is a `final` item):

```bibtex
@book{s0, author={Zz, Z}, title={T}, sortkey={0}}
@book{s1, author={Yy, Y}, title={T}, sortkey={1}}
@book{n,  author={Aa, A}, title={T}}
```

biber sorts `s1, n, s0`; it should be `s0, s1, n`. Here the value is lost earlier, in `strip_nosort` and `normalise_string_sort` (`return '' unless $string`). Once that is fixed, a `final` item whose value is `0` puts the `BIBERZERO` sentinel itself into `$BIBER_SORT_FINAL`, which `_generatesortinfo` then copies into all later columns as the string `BIBERZERO`; the patch maps it back to `0` there.

**3. Range constraints skip the value `0` and bounds of `0`.** With the data-model constraint from `t/tdata/dm-constraints.bcf`

```xml
<bcf:constraint type="data" datatype="integer" rangemin="1" rangemax="12">
  <bcf:field>month</bcf:field>
</bcf:constraint>
```

and `--validate-datamodel`, `month={13}` gives `Invalid value of field 'month' must be '<=12' - ignoring field`, but `month={0}` passes and is written as `\field{month}{0}`. `check_data_constraints` tests `if (my $fv = $be->get_field($f))`, and likewise `if (my $fmin = $c->{rangemin})`, so a constraint with `rangemin="0"` is not checked at all.

**4.** The other string normalisers in `Utils.pm` (`normalise_string_label`, `normalise_string_bblxml`, `normalise_string_hash`, `normalise_string_underscore`, `strip_noinit`, `strip_nonamestring`) have the same `return '' unless $str` test; e.g. `normalise_string_bblxml("0")` returns `''`, so a field whose value is `0` is written empty in bblxml output. The patch changes them all to test `length`, except `normalise_string`, which is used to compute `sortinit`: for an entry whose only sort data is the `\literal{0}` fallback of nty's volume column, the sort string is just `0`, and three tests in `t/basic-misc.t` expect an empty `sortinit` for such entries.

The tool-mode output side of the same pattern (fields with value `0`, year-0 dates and midnight times are dropped from `--tool` output) is covered by the report on tool-mode output.

### Suggested fix

```diff
--- a/lib/Biber/DataModel.pm
+++ b/lib/Biber/DataModel.pm
@@ -922,15 +922,18 @@
     elsif ($c->{datatype} eq 'integer' or
            $c->{datatype} eq 'datepart') {
       foreach my $f ($c->{fields}->@*) {
-        if (my $fv = $be->get_field($f)) {
-          if (my $fmin = $c->{rangemin}) {
+        my $fv = $be->get_field($f);
+        if (defined($fv) and $fv ne '') { # 0 is a value
+          my $fmin = $c->{rangemin};
+          if (defined($fmin) and $fmin ne '') {
             unless ($fv >= $fmin) {
               push @warnings, "Datamodel: $et entry '$key' ($ds): Invalid value of field '$f' must be '>=$fmin' - ignoring field";
               $be->del_field($f);
               next;
             }
           }
-          if (my $fmax = $c->{rangemax}) {
+          my $fmax = $c->{rangemax};
+          if (defined($fmax) and $fmax ne '') {
             unless ($fv <= $fmax) {
               push @warnings, "Datamodel: $et entry '$key' ($ds): Invalid value of field '$f' must be '<=$fmax' - ignoring field";
               $be->del_field($f);
--- a/lib/Biber/Internals.pm
+++ b/lib/Biber/Internals.pm
@@ -1242,6 +1242,7 @@
     # and so have no impact until later sort items where the final item becomes the
     # sorting key for every subsequent sorting item.
     if (my $f = $BIBER_SORT_FINAL) {
+      $f = 0 if $f eq 'BIBERZERO';
       push $sortobj->@*, ($BIBER_SUPPRESS_FINAL ? '' : $f);
       $BIBER_SUPPRESS_FINAL = 0;
     }
@@ -1341,7 +1342,8 @@
   my ($self, $citekey, $secnum, $section, $be, $dlist, $sortelementattributes, $args) = @_;
   my $dmtype = $args->[0]; # get int field type
   my $bee = $be->get_field('entrytype');
-  if (my $field = $be->get_field($dmtype)) {
+  my $field = $be->get_field($dmtype);
+  if (defined($field) and $field ne '') { # 0 is a value (year 0, volume 0)
 
     # Make an attempt to map roman numerals to integers for sorting unless suppressed
     if (isroman(NFKD($field)) and
@@ -1470,7 +1472,8 @@
 sub _sort_literal {
   my ($self, $citekey, $secnum, $section, $be, $dlist, $sortelementattributes, $args) = @_;
   my $literal = $args->[0]; # get actual field
-  if (my $field = $be->get_field($literal)) {
+  my $field = $be->get_field($literal);
+  if (defined($field) and $field ne '') {
     my $string = normalise_string_sort($field, $literal);
     return _translit($literal, $be, _process_sort_attributes($string, $sortelementattributes));
   }
@@ -1485,7 +1488,8 @@
 sub _sort_verbatim {
   my ($self, $citekey, $secnum, $section, $be, $dlist, $sortelementattributes, $args) = @_;
   my $verb = $args->[0]; # get actual field
-  if (my $field = $be->get_field($verb)) {
+  my $field = $be->get_field($verb);
+  if (defined($field) and $field ne '') {
     my $string = normalise_string_sort($field, $verb);
     return _process_sort_attributes($string, $sortelementattributes);
   }
--- a/lib/Biber/Utils.pm
+++ b/lib/Biber/Utils.pm
@@ -537,7 +537,7 @@
 
 sub strip_noinit {
   my $string = shift;
-  return '' unless $string; # Sanitise missing data
+  return '' unless length($string // ''); # Sanitise missing data (0 is data)
   return $string unless my $noinit = Biber::Config->getoption('noinit');
   foreach my $opt ($noinit->@*) {
     my $re = $opt->{value};
@@ -559,7 +559,7 @@
 sub strip_nosort {
   no autovivification;
   my ($string, $fieldname) = @_;
-  return '' unless $string; # Sanitise missing data
+  return '' unless length($string // ''); # Sanitise missing data (0 is data)
   return $string unless my $nosort = Biber::Config->getoption('nosort');
 
   my $restrings;
@@ -599,7 +599,7 @@
 sub strip_nonamestring {
   no autovivification;
   my ($string, $fieldname) = @_;
-  return '' unless $string; # Sanitise missing data
+  return '' unless length($string // ''); # Sanitise missing data (0 is data)
   return $string unless my $nonamestring = Biber::Config->getoption('nonamestring');
 
   my $restrings;
@@ -634,7 +634,7 @@
 
 sub normalise_string_label {
   my $str = shift;
-  return '' unless $str; # Sanitise missing data
+  return '' unless length($str // ''); # Sanitise missing data (0 is data)
   my $nolabels = Biber::Config->getoption('nolabel');
   $str =~ s/\\[A-Za-z]+//g;    # remove latex macros (assuming they have only ASCII letters)
   # Replace ties with spaces or they will be lost
@@ -659,7 +659,7 @@
 sub normalise_string_sort {
   my $str = shift;
   my $fieldname = shift;
-  return '' unless $str; # Sanitise missing data
+  return '' unless length($str // ''); # Sanitise missing data (0 is data)
   # First strip nosort REs
   $str = strip_nosort($str, $fieldname);
   # Then replace ties with spaces or they will be lost
@@ -680,7 +680,7 @@
 
 sub normalise_string_bblxml {
   my $str = shift;
-  return '' unless $str; # Sanitise missing data
+  return '' unless length($str // ''); # Sanitise missing data (0 is data)
   $str =~ s/\\[A-Za-z]+//g; # remove latex macros (assuming they have only ASCII letters)
   $str =~ s/\{([^\{\}]+)\}/$1/g; # remove pointless braces
   $str =~ s/~/ /g; # replace ties with spaces
@@ -735,7 +735,7 @@
 
 sub normalise_string_hash {
   my $str = shift;
-  return '' unless $str; # Sanitise missing data
+  return '' unless length($str // ''); # Sanitise missing data (0 is data)
   $str =~ s/\\(\p{L}+)\s*/$1:/g; # remove tex macros
   $str =~ s/\\([^\p{L}])\s*/ord($1).':'/ge; # remove accent macros like \"a
   $str =~ s/[\{\}~\.\s]+//g; # Remove braces, ties, dots, spaces
@@ -750,7 +750,7 @@
 
 sub normalise_string_underscore {
   my $str = shift;
-  return '' unless $str; # Sanitise missing data
+  return '' unless length($str // ''); # Sanitise missing data (0 is data)
   $str =~ s/([^\\])~/$1 /g; # Foo~Bar -> Foo Bar
   $str = normalise_string($str);
   $str =~ s/\s+/_/g;
@@ -765,7 +765,7 @@
 
 sub escape_label {
   my $str = shift;
-  return '' unless $str; # Sanitise missing data
+  return '' unless length($str // ''); # Sanitise missing data (0 is data)
   $str =~ s/([_\^\$\#\%\&])/\\$1/g;
   $str =~ s/~/{\\textasciitilde}/g;
   $str =~ s/>/{\\textgreater}/g;
@@ -782,7 +782,7 @@
 
 sub unescape_label {
   my $str = shift;
-  return '' unless $str; # Sanitise missing data
+  return '' unless length($str // ''); # Sanitise missing data (0 is data)
   $str =~ s/\\([_\^\$\~\#\%\&])/$1/g;
   $str =~ s/\{\\textasciitilde\}/~/g;
   $str =~ s/\{\\textgreater\}/>/g;
```

With this patch the examples sort `y0, y1, y2` and `s0, s1, n`, and `month={0}` gives `Invalid value of field 'month' must be '>=1' - ignoring field`. All tests in `t/` pass on `dev` (1686b9c) with this patch.

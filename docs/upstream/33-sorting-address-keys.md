<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-042, BB-135, BB-146 (docs/biber-bugs.md). -->

**Title:** Sorting and title uniqueness: `labelyear` sorts as a string, a numeric first sort item makes its fallbacks numeric, verbatim list fields sort by memory address, and name-list labeltitles are compared by memory address

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biber 2.22 and current `dev` (1686b9c).

**1. `labelyear`, `labelmonth` and `labelday` sort as strings.** `sortdataschema` only types `datepart`/`integer` fields (and `citeorder` etc.) as `int`, and the label date fields aren't data-model fields. With

```latex
\DeclareSortingTemplate{ly}{\sort{\field{labelyear}}}
\DeclareSortingTemplate{yr}{\sort{\field{year}}}
```

```bibtex
@book{y1, title={T}, date={0999}}
@book{y2, title={T}, date={1000}}
@book{y3, title={T}, date={-0050}}
@book{y4, title={T}, date={2000}}
@book{y5, title={T}, date={0080}}
```

`sorting=yr` gives `y3 y5 y1 y2 y4` (−50, 80, 999, 1000, 2000), but `sorting=ly` gives `y3 y2 y4 y5 y1` (−50, 1000, 2000, 80, 999).

**2. A sort column's type is taken from its first item only.** In `instantiate_sortingtemplate` (around `Biber.pm:3338`) the column type is `sortdataschema` of the first `\field`, but the other items are fallbacks for entries without the first. With `\sort{\field{year}\field{title}}`, the whole column is numeric, so for entries without a year the title is turned into the "not a number" default and ignored:

```bibtex
@book{n1, title={Zeta}}
@book{n2, title={Alpha}}
@book{n3, title={Mu}}
@book{y1, title={T}, date={1999}}
```

gives `y1 n1 n2 n3` (input order for the undated entries) instead of `y1 n2 n3 n1`.

**3. Verbatim and URI list fields sort by memory address.** In `_dispatch_table_sorting` the test `$dmf->{datatype} eq 'verbatim' or $dmf->{datatype} eq 'uri'` comes before the list case and doesn't check `fieldtype`, so `_sort_list_verbatim` is unreachable and list fields go to `_sort_verbatim`, which stringifies the array reference. With a data model that adds `\DeclareDatamodelFields[type=list,datatype=verbatim]{verblist}` and `\DeclareSortingTemplate{vl}{\sort{\field{verblist}}}`,

```bibtex
@misc{b, title={B}, verblist={bbb}}
@misc{c, title={C}, verblist={ccc}}
@misc{a, title={A}, verblist={aaa}}
```

cited as `\cite{c,a,b}` comes out as `c a b`, and `--debug` shows the sort keys as `ARRAY(0x5627233261f0)` etc. The order depends on where Perl allocated the arrays.

**4. A name list used as `labeltitle` is compared by memory address.** When `labeltitlespec` picks a name list (as chicago-authordate does for some entry types through a per-type spec), `uniquetitle`, `uniquebaretitle`, `uniquework`, `extratitle` and `extratitleyear` build their tracking keys by interpolating the `Biber::Entry::Names` object, i.e. `Biber::Entry::Names=HASH(0x…)`. Two entries with the same editor therefore never match. With `labeltitle=true`, `uniquetitle=true` and `labeltitlespec` = `editor`, `title`:

```bibtex
@misc{a, editor={Doe, John}, date={2000}}
@misc{b, editor={Doe, John}, date={2001}}
@misc{c, editor={Roe, Jane}, date={2001}}
```

all three entries get `\true{uniquetitle}`, and `a` and `b` get no `extratitle`.

### Suggested fix

Type the label date fields as integers; make a column numeric only if all its field items are numeric (numeric literals such as the `0` fallback in `nyt` don't count); restrict `_sort_verbatim` to non-list fields; and compare name-list labeltitles by their full hash:

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -2181,6 +2181,21 @@
   }
 }
 
+=head2 _labeltitle_string
+
+    The labeltitle as a string for comparisons. The labeltitle can be a name list,
+    which is represented by its full hash.
+
+=cut
+
+sub _labeltitle_string {
+  my ($self, $citekey, $dlist) = @_;
+  my $be = $self->sections->get_section($self->get_current_section)->bibentry($citekey);
+  my $lti = $be->get_labeltitle_info or return '';
+  my $lt = $be->get_field($lti) // return '';
+  return ref($lt) eq 'Biber::Entry::Names' ? $self->_getfullhash($citekey, $lt, $dlist) : $lt;
+}
+
 =head2 process_workuniqueness
 
     Track seen work combination for generation of singletitle, uniquetitle, uniquebaretitle and
@@ -2229,7 +2244,7 @@
   # uniquetitle
   # Don't generate information for entries with no labeltitle
   if ($lti and Biber::Config->getblxoption($secnum, 'uniquetitle', $bee, $citekey)) {
-    $identifier = $be->get_field($lti);
+    $identifier = $self->_labeltitle_string($citekey, $dlist);
 
     # Skip due to ignore settings?
     unless (first {fc($lti) eq fc($_)} $ignore->{uniquetitle}->@*) {
@@ -2241,7 +2256,7 @@
   # uniquebaretitle
   # Don't generate information for entries with no labeltitle and with labelname
   if ($lti and not $lni and Biber::Config->getblxoption($secnum, 'uniquebaretitle', $bee, $citekey)) {
-    $identifier = $be->get_field($lti);
+    $identifier = $self->_labeltitle_string($citekey, $dlist);
 
     # Skip due to ignore settings?
     unless (first {fc($lti) eq fc($_)} $ignore->{uniquebaretitle}->@*) {
@@ -2254,7 +2269,7 @@
   # Don't generate information for entries with no labelname and labeltitle
   # Should use fullhash this is not a test of uniqueness of only visible information
   if ($lni and $lti and Biber::Config->getblxoption($secnum, 'uniquework', $bee, $citekey)) {
-    $identifier = $self->_getfullhash($citekey, $be->get_field($lni), $dlist) . $be->get_field($lti);
+    $identifier = $self->_getfullhash($citekey, $be->get_field($lni), $dlist) . $self->_labeltitle_string($citekey, $dlist);
 
     # Skip due to ignore settings?
     unless (first {fc($lni) eq fc($_)} $ignore->{uniquework}->@* and
@@ -2413,8 +2428,7 @@
       $namehash = $self->_getnamehash_u($citekey, $be->get_field($lni), $dlist);
     }
 
-    my $lti = $be->get_labeltitle_info;
-    my $title_string = $be->get_field($lti) // '';
+    my $title_string = $self->_labeltitle_string($citekey, $dlist);
 
     my $nametitle_string = "$namehash,$title_string";
     if ($logger->is_trace()) {# performance tune
@@ -2461,8 +2475,7 @@
       $logger->trace("Creating extratitleyear information for '$citekey'");
     }
 
-    my $lti = $be->get_labeltitle_info;
-    my $title_string = $be->get_field($lti) // '';
+    my $title_string = $self->_labeltitle_string($citekey, $dlist);
 
     # Takes into account the labelyear which can be a range
     my $year_string = $be->get_field('labelyear') || $be->get_field('year') || '';
@@ -3340,7 +3353,17 @@
         }
       }
     }
-    my $spec = $dm->{sortdataschema}->([keys $sort->[1]->%*]->[0]);
+    # The items are fallbacks for each other, so the column is numeric only if
+    # all of its items are numeric fields or numeric literals
+    my $spec = 'int';
+    foreach my $item ($sort->@[1..$sort->$#*]) {
+      my ($name, $attrs) = $item->%*;
+      next if $attrs->{literal} and looks_like_number($name);
+      if ($attrs->{literal} or $dm->{sortdataschema}->($name) ne 'int') {
+        $spec = 'str';
+        last;
+      }
+    }
     push $ds->@*, {spec  => "$direction$spec",
                    $spec => 1}; # Speed shortcut for sortkey extraction sub
 
--- a/lib/Biber/DataModel.pm
+++ b/lib/Biber/DataModel.pm
@@ -334,7 +334,9 @@
   # Mapping of sorting fields to Sort::Key sort data types which are not 'str'
   $self->{sortdataschema} = sub {
     my $f = shift;
-    if (first {$f eq $_} ('intciteorder', 'citeorder', 'citecount', $self->{helpers}{integers}->@*)) {
+    if (first {$f eq $_} ('intciteorder', 'citeorder', 'citecount',
+                          'labelyear', 'labelmonth', 'labelday',
+                          $self->{helpers}{integers}->@*)) {
       return 'int';
     }
     else {
--- a/lib/Biber/Internals.pm
+++ b/lib/Biber/Internals.pm
@@ -1140,7 +1140,8 @@
   if ($dmf->{fieldtype} eq 'list' and $dmf->{datatype} eq 'name') {
     return [\&_sort_name, [$field]];
   }
-  elsif ($dmf->{datatype} eq 'verbatim' or $dmf->{datatype} eq 'uri') {
+  elsif ($dmf->{fieldtype} eq 'field' and
+         ($dmf->{datatype} eq 'verbatim' or $dmf->{datatype} eq 'uri')) {
     return [\&_sort_verbatim, [$field]];
   }
   elsif ($dmf->{fieldtype} eq 'field' and $dmf->{datatype} eq 'literal' ) {
```

With this change, `sorting=ly` gives `y3 y5 y1 y2 y4`, the year/title column gives `y1 n2 n3 n1`, the `verblist` example sorts `a b c`, and in the last example `a` and `b` lose `uniquetitle` and get `extratitle` 1 and 2. All tests in `t/` pass on `dev` (1686b9c) with this patch (`t/sorting.t` checks the schema of `nyt`, whose last column is `\field{volume}\literal{0}` and stays `int`).

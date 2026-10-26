<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-024, BB-025 and BB-035 (docs/biber-bugs.md). -->

**Title:** Granular XDATA: names are duplicated when an XDATA entry has its own references, spliced names get 0-based indices, and references in xsv fields (`keywords`) are not resolved

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Three problems with granular XDATA references (`field = {xdata=entry-field[-position]}`). Same behaviour on biber 2.22 and current `dev` (1686b9c).

**1. An XDATA entry's own references are re-resolved for every referrer (BB-025).** `Biber::Entry::resolve_xdata` recurses into an XDATA entry that has XDATA references of its own each time the entry is referenced, and granular name references splice into the XDATA entry's name list again each time:

```bibtex
@xdata{Y, author = {Alpha, Ann and Beta, Bob}}
@xdata{X, author = {xdata=Y-author and Doe, Jim}, publisher = {Pub}}
@book{A1, xdata = {X}, title = {T1}, year = {2000}}
@book{A2, xdata = {X}, title = {T2}, year = {2001}}
@book{A3, xdata = {X}, title = {T3}, year = {2002}}
```

With `\cite{A1,A2,A3}`, all three entries get `\name{author}{5}`: Alpha, Beta, Beta, Beta, Doe, instead of Alpha, Beta, Doe.

**2. Spliced names get 0-based indices (BB-024).** `Names::replace_name` and `Names::splice` set `$position-1` / `$i` as the name's index, but `add_name` (used for all other names) sets 1-based indices, and visibility (`maxcitenames` etc.) compares against them. `splice` also stops one name short (`$i<$#{…}`). So after a granular reference, one more name than allowed counts as visible for uniquename:

```bibtex
@xdata{X, author = {Smith, John and Brown, Alice and Jones, Jim}}
@book{A, author = {xdata=X-author}, title = {TA}, year = {2000}}
@book{B, author = {Brown, Bill and Doe, Dan and Roe, Ray}, title = {TB}, year = {2001}}
```

With `style=authoryear,uniquename=init,maxcitenames=1` and `\cite{A,B}`, Brown in both entries gets `un=1` (initials), although in `A` Brown is the second name and not visible.

**3. Granular references in xsv fields are output literally (BB-035).** `_xsv` (Input/file/bibtex.pm) passes the whole array to `add_xdata_ref`, which matches a granular reference against a string, so `keywords = {xdata=xd-keywords}` is never recorded. The `.bbl` gets `\keyw{xdata=xd-keywords}`. Also, `resolve_xdata` only treats `list` fields as lists, so an xsv field would be replaced as a whole.

```bibtex
@misc{a, title = {T}, keywords = {xdata=xd-keywords}}
@misc{b, title = {U}, keywords = {first, xdata=xd-keywords-2, last}}
@xdata{xd, keywords = {k1, k2}}
```

```latex
\documentclass{article}
\usepackage[style=authoryear]{biblatex}
\addbibresource{test.bib}
\begin{document}
\nocite{*}
\printbibliography
\end{document}
```

### Suggested fix

Resolve each entry's own XDATA references once, use 1-based indices in `replace_name`/`splice`, record granular references per xsv item, and splice into xsv fields like into lists:

```diff
--- a/lib/Biber/Entry.pm
+++ b/lib/Biber/Entry.pm
@@ -719,6 +719,11 @@
   my $bee = $self->get_field('entrytype');
   my $dm = Biber::Config->get_dm;
 
+  # An XDATA entry which itself has XDATA references is resolved when it is first
+  # referenced. Don't do this again for later referrers, as granular references
+  # splice into its fields.
+  return if $self->{xdata_resolved};
+
   # $xdata =
   # [
   #  { # xdata info for an actual XDATA field (XDATA = {key, key})
@@ -847,7 +852,8 @@
               }
             }
             # Non-name lists
-            elsif ($dm->field_is_fieldtype('list', $reffield)) {
+            elsif ($dm->field_is_fieldtype('list', $reffield) or
+                   ($dm->get_fieldformat($reffield) // '') eq 'xsv') { # xsv fields are lists too
               if ($xdatum->{xdataposition} eq '*') { # insert all positions from XDATA field
                 my $bibentries = $section->bibentries;
                 my $be = $bibentries->entry($xdatum->{xdataentries}[0]);
@@ -915,6 +921,7 @@
       }
     }
   }
+  $self->{xdata_resolved} = 1;
 }
 
 =head2 inherit_from
--- a/lib/Biber/Entry/Names.pm
+++ b/lib/Biber/Entry/Names.pm
@@ -87,7 +87,7 @@
 
 sub replace_name {
   my ($self, $name_obj, $position) = @_;
-  $name_obj->set_index($position-1);
+  $name_obj->set_index($position);
   $self->{namelist}->[$position-1] = $name_obj;
   return;
 }
@@ -102,9 +102,9 @@
 sub splice {
   my ($self, $names, $position) = @_;
   splice($self->{namelist}->@*, $position-1, 1, $names->{namelist}->@*);
-  # now re-index all names in list
-  foreach (my $i=0;$i<$#{$self->{namelist}};$i++) {
-    $self->{namelist}->[$i]->set_index($i);
+  # now re-index all names in list (indices are 1-based, as in add_name())
+  foreach (my $i=0;$i<=$#{$self->{namelist}};$i++) {
+    $self->{namelist}->[$i]->set_index($i+1);
   }
   return;
 }
--- a/lib/Biber/Input/file/bibtex.pm
+++ b/lib/Biber/Input/file/bibtex.pm
@@ -1144,7 +1144,14 @@
   my $value = [ split(/$S/, $entry->get(encode('UTF-8', NFC($field)))) ];
 
   # Record any XDATA
-  $bibentry->add_xdata_ref($field, $value);
+  if ($field eq 'xdata') { # the list of XDATA entries itself
+    $bibentry->add_xdata_ref($field, $value);
+  }
+  else { # granular XDATA references in the items
+    for (my $i = 0; $i <= $value->$#*; $i++) {
+      $bibentry->add_xdata_ref($field, $value->[$i], $i);
+    }
+  }
 
   return $value ;
 }
```

With this change, (1) gives Alpha, Beta, Doe for all three entries; in (2) both Browns get `un=0`; and (3) gives `\keyw{k1,k2}` for `a` and `\keyw{first,k2,last}` for `b`. All tests in `t/` pass on `dev` (1686b9c) with this patch, including `t/xdata.t`.

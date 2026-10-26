<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-060, BB-061, BB-062, BB-112, BB-113, BB-065 (docs/biber-bugs.md). -->

**Title:** The `.bbl` (and errors) change between identical runs: per-name options, promoted crossref targets and dynamic sets in hash order, XDATA cycle errors, `$MAPUNIQ`

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Running biber twice on the same input should give the same `.bbl`: latexmk and similar tools rerun LaTeX when the `.bbl` changes, and reproducible builds compare outputs. In the cases below, biber 2.22 and `dev` (1686b9c) give different output from run to run, because of Perl's hash-order randomisation or the clock.

**1. Per-name options are written in hash order.** With `uniquename=true` and

```bibtex
@book{a, author={useprefix=true, sortingnamekeytemplatename=global, uniquenametemplatename=global, labelalphanametemplatename=global, family=Doe, given=John}, title={A}, date={2000}}
```

eight runs gave eight different `.bbl` files on `dev`, e.g.

```
{{un=0,uniquepart=base,labelalphanametemplatename=global,uniquenametemplatename=global,useprefix=true,sortingnamekeytemplatename=global,hash=…}{%
{{un=0,uniquepart=base,useprefix=true,uniquenametemplatename=global,sortingnamekeytemplatename=global,labelalphanametemplatename=global,hash=…}{%
```

The loops over `keys $CONFIG_SCOPEOPT_BIBLATEX{NAME}` in `Entry/Name.pm` (`name_to_bbl`, `name_to_bblxml`, `name_to_bibjson`) and over `…{NAMELIST}` in `Output/bbl.pm` and `Output/bblxml.pm` are unsorted. (The same loops in the biblatexml and BibTeX writers are covered in the reports on those writers.)

**2. Promoted crossref/xref targets are added in hash order.** With `sorting=none` and `\cite{c1,c2,c3,c4,c5,c6}`:

```bibtex
@inbook{c1, title={C1}, crossref={p1}}
@inbook{c2, title={C2}, crossref={p1}}
@inbook{c3, title={C3}, crossref={p2}}
@inbook{c4, title={C4}, crossref={p2}}
@inbook{c5, title={C5}, crossref={p3}}
@inbook{c6, title={C6}, crossref={p3}}
@book{p1, title={P1}}
@book{p2, title={P2}}
@book{p3, title={P3}}
```

the parents (included because of `mincrossrefs=2`) came out as `p3, p1, p2`, `p1, p3, p2`, `p1, p2, p3`, `p3, p2, p1`, … in six runs, so their labelnumbers change. `Config::get_crossrefkeys` and `get_xrefkeys` return `keys`.

**3. Dynamic sets are added in hash order under `\nocite{*}`.** With `sorting=none`, `\nocite{*}` and `\defbibentryset{setA}{c1,c2}`, `\defbibentryset{setB}{c3,c4}`, `\defbibentryset{setC}{c5,c6}`, the sets came out in a different order in almost every run. `Section::dynamic_set_keys` returns `keys`; the patch returns them in the order of definition.

**4. Circular XDATA errors depend on hash order.** In a copy of `t/tdata`, `biber --noconf xdata.bcf` stops with either `Circular XDATA inheritance between 'loop'<->'loop:3'` or `… between 'lxd1:loop'<->'lxd2:loop'`, after 8 to 19 warnings (8 runs). `resolve_xdata` iterates over `$section->bibentries->entries`, which is `values` of the entries hash. The patch makes `Entries::entries` return the entries in key order, so every caller is deterministic.

**5. `$MAPUNIQ` comes from the clock.** `maploopreplace` uses `Data::Uniqid::suniqid`, so a sourcemap that keys new entries with `$MAPUNIQ` (the pattern of biblatex's `95-customlists.tex`) gives new keys, and therefore a new `.bbl`, on every run:

```xml
<map map_foreach="abstract">
  <map_step map_field_source="abstract" map_final="1"/>
  <map_step map_entry_new="$MAPUNIQ" map_entry_newtype="customa"/>
  <map_step map_field_source="entrykey"/>
  <map_step map_entrytarget="$MAPUNIQVAL" map_field_set="entrykey" map_origfieldval="1"/>
  <map_step map_field_source="abstract"/>
  <map_step map_entrytarget="$MAPUNIQVAL" map_field_set="abstract" map_origfieldval="1"/>
</map>
```

With two entries that have an abstract, three runs gave the keys `6weFaaUxhf, 6yn2Slw6EV`, then `4nuT2xDKQO, 4pDdep5HUS`, then `2l9Rpg6mm7, 2nebml7f7t`. (Outside `map_foreach`, `$MAPUNIQ` is not replaced at all on `dev`; that is a separate report.) The patch uses a counter (`mapuniq1`, `mapuniq2`, …). Even with a counter, which source entry gets which value depended on hash order, because `extract_entries` (BibTeX input, `\nocite{*}`) creates the entries with `each` over the parse cache; the patch creates them in file order (`orig_key_order`, which already exists for `sorting=none`).

**6.** The order of warnings in the `.blg` also varied between runs (e.g. with `-V`), largely for the reasons above. I have not checked every source of warnings after the patch, but the examples above give identical `.blg` files from run to run, apart from the date line and timings (on `dev`, four runs of the XDATA example gave four different logs).

### Suggested fix

```diff
--- a/lib/Biber/Config.pm
+++ b/lib/Biber/Config.pm
@@ -1258,7 +1258,7 @@
 
 sub get_crossrefkeys {
   shift; # class method so don't care about class name
-  return [ keys $CONFIG->{state}{crossrefkeys}->%* ];
+  return [ sort keys $CONFIG->{state}{crossrefkeys}->%* ];
 }
 
 =head1 xrefkeys
@@ -1271,7 +1271,7 @@
 
 sub get_xrefkeys {
   shift; # class method so don't care about class name
-  return [ keys $CONFIG->{state}{xrefkeys}->%* ];
+  return [ sort keys $CONFIG->{state}{xrefkeys}->%* ];
 }
 
 =head2 get_crossrefkey
--- a/lib/Biber/Entries.pm
+++ b/lib/Biber/Entries.pm
@@ -65,7 +65,9 @@
 
 sub entries {
   my $self = shift;
-  return values $self->%*;
+  # In key order, so that processing (and its warnings and errors) doesn't
+  # change between runs
+  return map {$self->{$_}} sort keys $self->%*;
 }
 
 
--- a/lib/Biber/Entry/Name.pm
+++ b/lib/Biber/Entry/Name.pm
@@ -310,7 +310,7 @@
 
 
   # Add per-name options
-  foreach my $no (keys $CONFIG_SCOPEOPT_BIBLATEX{NAME}->%*) {
+  foreach my $no (sort keys $CONFIG_SCOPEOPT_BIBLATEX{NAME}->%*) {
     if (defined($self->${\"get_$no"})) {
       my $nov = $self->${\"get_$no"};
 
@@ -459,7 +459,7 @@
   }
 
   # Add per-name options
-  foreach my $no (keys $CONFIG_SCOPEOPT_BIBLATEX{NAME}->%*) {
+  foreach my $no (sort keys $CONFIG_SCOPEOPT_BIBLATEX{NAME}->%*) {
     if (defined($self->${\"get_$no"})) {
       my $nov = $self->${\"get_$no"};
 
@@ -519,7 +519,7 @@
   }
 
   # Add per-name options
-  foreach my $no (keys $CONFIG_SCOPEOPT_BIBLATEX{NAME}->%*) {
+  foreach my $no (sort keys $CONFIG_SCOPEOPT_BIBLATEX{NAME}->%*) {
     if (defined($self->${\"get_$no"})) {
       my $nov = $self->${\"get_$no"};
 
--- a/lib/Biber/Input/file/bibtex.pm
+++ b/lib/Biber/Input/file/bibtex.pm
@@ -194,8 +194,12 @@
       $logger->debug("All citekeys will be used for section '$secnum'");
     }
 
-    # Loop over all entries, creating objects
-    while (my ($key, $entry) = each $cache->{data}{$filename}->%*) {
+    # Loop over all entries, creating objects. In file order, not hash order, so
+    # that sourcemaps (e.g. $MAPUNIQ keys) give the same result on every run
+    # (A copy, as create_entry failures are removed from orig_key_order below)
+    my @keys = map {"$_"} uniq (($cache->{orig_key_order}{$filename} // [])->@*);
+    foreach my $key (@keys) {
+      my $entry = $cache->{data}{$filename}{$key};
 
       # Record a key->datasource name mapping for error reporting
       $section->set_keytods($key, $filename);
@@ -209,8 +213,8 @@
     # Loop over all aliases, creating data in section object
     # Since this is allkeys, we are guaranteed that the real entry for the alias
     # will be available
-    while (my ($alias, $key) = each $cache->{data}{citekey_aliases}->%*) {
-      $section->set_citekey_alias($alias, $key);
+    foreach my $alias (sort keys $cache->{data}{citekey_aliases}->%*) {
+      $section->set_citekey_alias($alias, $cache->{data}{citekey_aliases}{$alias});
     }
 
     # If allkeys, push all bibdata keys into citekeys (if they are not already there).
--- a/lib/Biber/Output/bbl.pm
+++ b/lib/Biber/Output/bbl.pm
@@ -340,7 +340,7 @@
         }
 
         # Add per-namelist options
-        foreach my $nlo (keys $CONFIG_SCOPEOPT_BIBLATEX{NAMELIST}->%*) {
+        foreach my $nlo (sort keys $CONFIG_SCOPEOPT_BIBLATEX{NAMELIST}->%*) {
           if (defined($nf->${\"get_$nlo"})) {
             my $nlov = $nf->${\"get_$nlo"};
 
--- a/lib/Biber/Output/bblxml.pm
+++ b/lib/Biber/Output/bblxml.pm
@@ -243,7 +243,7 @@
         }
 
         # Add per-namelist options
-        foreach my $nlo (keys $CONFIG_SCOPEOPT_BIBLATEX{NAMELIST}->%*) {
+        foreach my $nlo (sort keys $CONFIG_SCOPEOPT_BIBLATEX{NAMELIST}->%*) {
           if (defined($nf->${\"get_$nlo"})) {
             my $nlov = $nf->${\"get_$nlo"};
 
--- a/lib/Biber/Section.pm
+++ b/lib/Biber/Section.pm
@@ -835,6 +835,8 @@
   my $self = shift;
   my $dkey = shift;
   my @members = @_;
+  # Remember the order of definition, for dynamic_set_keys
+  $self->{dkeysorder}{$dkey} //= scalar(keys $self->{dkeys}->%*);
   $self->{dkeys}{$dkey} = \@members;
   return;
 }
@@ -866,7 +868,9 @@
 
 sub dynamic_set_keys {
   my $self = shift;
-  return [keys $self->{dkeys}->%*];
+  # In order of definition, not hash order, so that the output doesn't change
+  # between runs
+  return [sort {$self->{dkeysorder}{$a} <=> $self->{dkeysorder}{$b}} keys $self->{dkeys}->%*];
 }
 
 =head2 has_dynamic_sets
--- a/lib/Biber/Utils.pm
+++ b/lib/Biber/Utils.pm
@@ -19,7 +19,6 @@
 use Biber::Constants;
 use Biber::LaTeX::Recode;
 use Biber::Entry::Name;
-use Data::Uniqid qw ( suniqid );
 use Regexp::Common qw( balanced );
 use List::AllUtils qw( first );
 use Log::Log4perl qw(:no_extra_logdie_message);
@@ -1784,6 +1783,10 @@
 
 =cut
 
+# $MAPUNIQ values come from a counter, not the clock, so that the output
+# doesn't change between runs
+my $MAPUNIQCOUNT = 0;
+
 sub maploopreplace {
   # $MAPUNIQVAL is lexical here
   no strict 'vars';
@@ -1793,7 +1796,7 @@
   $string =~ s/\$MAPLOOP/$maploop/g if defined($maploop);
   $string =~ s/\$MAPUNIQVAL/$MAPUNIQVAL/g;
   if ($string =~ m/\$MAPUNIQ/) {
-    my $MAPUNIQ = suniqid;
+    my $MAPUNIQ = 'mapuniq' . ++$MAPUNIQCOUNT;
     $string =~ s/\$MAPUNIQ/$MAPUNIQ/g;
     $MAPUNIQVAL = $MAPUNIQ;
   }
```

With this patch, each example gives the same output in repeated runs. All tests in `t/` pass on `dev` (1686b9c) with this patch.

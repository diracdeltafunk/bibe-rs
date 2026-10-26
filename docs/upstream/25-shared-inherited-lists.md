<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-138 and BB-026 (docs/biber-bugs.md). -->

**Title:** Inherited lists and name lists are shared by reference with the parent: `\true{morelocation}` disappears from crossref children, and children's name hashes depend on processing order

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Crossref/XDATA inheritance (`set_datafield($field, $parent->get_field($field))` in `Biber::Entry::inherit_from`) copies the *reference* to the parent's list and `Names` objects. Two things then go wrong. Same behaviour on biber 2.22 and current `dev` (1686b9c).

**1. Writing one entry changes the others (BB-138).** When writing a list field that ends in `others`, the `.bbl` writer pops `others` off the entry's list (Output/bbl.pm:335-337, and 567-569 for verbatim lists):

```perl
      if ( lc($lf->[-1]) eq Biber::Config->getoption('others_string') ) {
        $acc .= "      \\true{more$listfield}\n";
        pop $lf->@*; # remove the last element in the array
      }
```

Since the list is shared, `others` is removed for the parent and every other child too, and only the first entry written gets `\true{more…}`:

```bibtex
@book{parent, title = {P}, location = {A and others}, publisher = {X and Y and others}}
@inbook{child1, crossref = {parent}, title = {C1}, author = {Doe, J}}
@inbook{child2, crossref = {parent}, title = {C2}, author = {Roe, J}}
```

With `\nocite{*}` and `style=authoryear`, `child1` and `child2` have no `\true{morelocation}` or `\true{morepublisher}`, so their "et al." is lost. The URI encoding of verbatim URI lists (line 586) is written back into the shared list in the same way.

**2. Per-list state is keyed by the shared list's id (BB-026).** Visibility, uniquelist and uniquename state is stored per name-list id, so a child that inherits the parent's `Names` object shares that state with the parent, and the last entry processed wins:

```bibtex
@collection{P, editor = {Alpha, Ann and Beta, Bob and Gamma, Gil}, title = {Coll}, year = {2000}}
@incollection{C, crossref = {P}, title = {Chap}, author = {Zed, Zoe}, options = {maxcitenames=1}}
@collection{Q, editor = {Alpha, Ann and Beta, Bob and Gamma, Gil}, title = {Coll2}, year = {2000}}
@incollection{D, title = {Chap2}, author = {Zed, Zoe}, options = {maxcitenames=1}, editor = {Alpha, Ann and Beta, Bob and Gamma, Gil}}
```

With `style=authoryear,maxcitenames=3` and `\cite{C,P,D,Q}`, `C` (inherited editors) and `D` (the same editors given directly) should be identical, but `C` gets `\strng{editornamehash}{df2771f9…}` (the parent's, computed with 3 visible names) and `D` gets `0244e570…` (1 visible name).

### Suggested fix

For (1), the writer should work on a copy:

```diff
--- a/lib/Biber/Output/bbl.pm
+++ b/lib/Biber/Output/bbl.pm
@@ -331,7 +331,8 @@
   # Output list fields
   foreach my $listfield ($dmh->{lists}->@*) {
     # Performance - as little as possible here - loop over DM fields for every entry
-    if (my $lf = $be->get_field($listfield)) {
+    if (my $lfield = $be->get_field($listfield)) {
+      my $lf = [ $lfield->@* ]; # copy: the list may be shared with other entries
       if ( lc($lf->[-1]) eq Biber::Config->getoption('others_string') ) {
         $acc .= "      \\true{more$listfield}\n";
         pop $lf->@*; # remove the last element in the array
@@ -563,7 +564,8 @@
 
   # verbatim lists
   foreach my $vlist ($dmh->{vlists}->@*) {
-    if ( my $vlf = $be->get_field($vlist) ) {
+    if ( my $vlfield = $be->get_field($vlist) ) {
+      my $vlf = [ $vlfield->@* ]; # copy: the list may be shared with other entries
       if ( lc($vlf->[-1]) eq Biber::Config->getoption('others_string') ) {
         $acc .= "      \\true{more$vlist}\n";
         pop $vlf->@*; # remove the last element in the array
```

With this change all three entries in example 1 get `\true{morelocation}` and `\true{morepublisher}`. All tests in `t/` pass on `dev` (1686b9c) with this patch.

For (2), inherited `Names` objects need their own ids (a copy of the list with fresh name-list and name ids), so that per-list state is computed per entry. I haven't attached a patch for this part.

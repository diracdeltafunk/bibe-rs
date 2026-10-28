<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-110 (docs/biber-bugs.md). -->

**Title:** Cascading crossrefs take cubic time in the number of children (`is_inheritance_path` rescans the whole inheritance list)

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with `dev` (1686b9c); 2.22 has the same code.

A common shape of data is many `@inproceedings` that crossref one `@proceedings`, which in turn crossrefs an `@mvproceedings`:

```python
# generates c1000.bib
with open('c1000.bib', 'w') as f:
    f.write('@mvproceedings{gp, title={Grand}, date={2000}, editor={E, E}}\n'
            '@proceedings{p, title={Parent}, crossref={gp}}\n')
    for i in range(1000):
        f.write(f'@inproceedings{{c{i}, author={{A{i}, B}}, title={{T{i}}}, crossref={{p}}}}\n')
```

With `\nocite{*}` and the standard style, biber takes 8.2 s for 500 children and 46.5 s for 1000 children; our earlier measurements with 2000 children took about 245 s. Almost all of the time is spent in `Biber::Config::is_inheritance_path`.

`set_inheritance` pushes every `{s => $source, t => $target}` pair onto a list, and `Entry::inherit_from` records `p → c_i` for each child and then, because the cascade recurses for every child, records `gp → p` again each time. To detect loops, `inherit_from` calls `is_inheritance_path($target, $source)`, which follows edges out of `$target` by `grep`ping the whole list at each step, with no visited set. When `p` inherits from `gp` for the *k*-th time, the search starts at `p`, finds the *k* edges `p → c_i`, and greps the whole list (size about 2*k*) once for each of them. That is O(*k*²) per call and O(*N*³) in total. `get_inheritance` (used in `Biber.pm` to skip crossref inheritance that was already done) is another linear scan.

### Suggested fix

Keep the inheritance relation as an adjacency hash (which also collapses the repeated `gp → p` edges) and search it breadth-first with a visited set:

```diff
--- a/lib/Biber/Config.pm
+++ b/lib/Biber/Config.pm
@@ -69,8 +69,8 @@
 $CONFIG->{state}{citkey_aliases} = {};
 
 # Record of which entries have inherited from other fields. Used for loop detection.
-$CONFIG->{state}{crossref} = [];
-$CONFIG->{state}{xdata} = [];
+$CONFIG->{state}{crossref} = {};
+$CONFIG->{state}{xdata} = {};
 
 # Record of which entries have inherited what from whom, with the fields inherited.
 # Used for generating inheritance trees
@@ -99,8 +99,8 @@
   $CONFIG->{state}{crossrefkeys} = {};
   $CONFIG->{state}{xrefkeys} = {};
   $CONFIG->{state}{datafiles} = [];
-  $CONFIG->{state}{crossref} = [];
-  $CONFIG->{state}{xdata} = [];
+  $CONFIG->{state}{crossref} = {};
+  $CONFIG->{state}{xdata} = {};
 
   return;
 }
@@ -1133,7 +1133,7 @@
 sub set_inheritance {
   shift; # class method so don't care about class name
   my ($type, $source, $target) = @_;
-  push $CONFIG->{state}{$type}->@*, {s => $source, t => $target};
+  $CONFIG->{state}{$type}{$source}{$target} = 1;
   return;
 }
 
@@ -1148,7 +1148,7 @@
 sub get_inheritance {
   shift; # class method so don't care about class name
   my ($type, $source, $target) = @_;
-  return first {$_->{s} eq $source and $_->{t} eq $target} $CONFIG->{state}{$type}->@*;
+  return $CONFIG->{state}{$type}{$source}{$target};
 }
 
 =head2 is_inheritance_path
@@ -1156,24 +1156,24 @@
   Checks for an inheritance path from entry $e1 to $e2
   Can be used for crossrefs and xdata
 
-[
-             {s => 'A',
-              t => 'B'},
-             {s => 'A',
-              t => 'E'},
-             {s => 'B',
-              t => 'C'},
-             {s => 'C',
-              t => 'D'}
-];
+{
+  A => {B => 1, E => 1},
+  B => {C => 1},
+  C => {D => 1}
+}
 
 =cut
 
 sub is_inheritance_path {
   my ($self, $type, $e1, $e2) = @_;
-  foreach my $dps (grep {$_->{s} eq $e1} $CONFIG->{state}{$type}->@*) {
-    return 1 if $dps->{t} eq $e2;
-    return 1 if is_inheritance_path($self, $type, $dps->{t}, $e2);
+  my $edges = $CONFIG->{state}{$type};
+  my %seen = ($e1 => 1);
+  my @todo = ($e1);
+  while (defined(my $s = shift @todo)) {
+    foreach my $t (keys(($edges->{$s} // {})->%*)) {
+      return 1 if $t eq $e2;
+      push @todo, $t unless $seen{$t}++;
+    }
   }
   return 0;
 }
```

With this patch the same files take 3.5 s (500 children) and 6.3 s (1000 children), with byte-identical `.bbl` files. `$CONFIG->{state}{crossref}` and `{xdata}` are only accessed through these three methods. All tests in `t/` pass on `dev` (1686b9c) with this patch.

I saw your answer on #529, so to be clear about why this one seemed worth reporting: the time grows with the cube of the number of children (about 4 minutes for 2,000), and the fix is a plain adjacency hash with a visited set rather than a new data structure.

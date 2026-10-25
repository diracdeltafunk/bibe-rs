<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/529 (2026-10-25). Tracks BB-109 (docs/biber-bugs.md). -->

**Title:** uniquelist processing is quadratic in the number of distinct name lists (`namelist_differs_index`/`_nth` scan every list for every entry)

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

With `uniquelist` on, `set_uniquelist` calls `namelist_differs_index` or `namelist_differs_nth` for most entries. Each call loops over **every** distinct complete name list (`keys …{uniquelistcount}{global}{final}`) and `split`s each one again. A uniquelist pass therefore costs O(entries × distinct lists × list length), so large bibliographies slow down sharply.

### Timings

A synthetic bibliography: pairs of entries sharing a four-name author list, `style=authoryear, uniquelist=true, maxcitenames=1`. biber 2.22 from source, Apple M3, one run each:

| Entries | biber 2.22 | with the fix below | `.bbl` |
|---|---|---|---|
| 1,000 | 9.4 s | 6.0 s | identical |
| 2,000 | 27.0 s | 11.4 s | identical |
| 4,000 | 97.0 s | 23.9 s | identical |

Each doubling multiplies biber 2.22's time by about 3.5, and the fixed version's by about 2.

The generator (Python), to reproduce:

```python
import sys
n = int(sys.argv[1])
with open(f"ul{n}.bib", "w") as f:
    for k in range(n // 2):
        names = " and ".join(f"Name{k}x{j}, Given" for j in range(4))
        for c in "ab":
            f.write(f"@article{{e{k}{c}, author = {{{names}}}, title = {{Title {k}{c}}}, "
                    f"journaltitle = {{J}}, date = {{2000}}}}\n")
with open(f"ul{n}.tex", "w") as f:
    f.write("\\documentclass{article}\n"
            "\\usepackage[style=authoryear, uniquelist=true, maxcitenames=1]{biblatex}\n"
            f"\\addbibresource{{ul{n}.bib}}\n\\begin{{document}}\n\\nocite{{*}}\n"
            "\\printbibliography\n\\end{document}\n")
```

Real documents with long publication lists, such as CVs, group reports and theses, hit the same path.

### Suggested fix

Answer both questions from a prefix tree over the complete lists. The tree is built once per pass and dropped whenever the counts change:

- **`namelist_differs_index`:** each node counts the lists that share its prefix. Walking the list's own path, the answer is the last depth where some *other* list still shares the prefix.
- **`namelist_differs_nth`:** after following the first n−1 names, each node knows which child leads to the longest list, and the longest list among the other children. That answers "is there a list at least as long that differs at n" without iterating over children.

```diff
--- a/lib/Biber/DataList.pm
+++ b/lib/Biber/DataList.pm
@@ -187,6 +187,7 @@
 
 sub add_uniquelistcount_final {
   my ($self, $namelist, $labelyear) = @_;
+  delete $self->{state}{uniquelistcount}{trie};
   $self->{state}{uniquelistcount}{global}{final}{join("\x{10FFFD}", $namelist->@*)}++;
   if ($labelyear) { # uniquelist=minyear
     $self->{state}{uniquelistcount}{global}{final}{$labelyear}{join("\x{10FFFD}", $namelist->@*)}++;
@@ -2252,19 +2253,14 @@
   my $self = shift;
   my @list = shift->@*;
   my $index;
-  foreach my $l_s (keys $self->{state}{uniquelistcount}{global}{final}->%*) {
-    my @l = split("\x{10FFFD}", $l_s);
-    next if Compare(\@list, \@l);# Ignore identical lists
-    for (my $i=0;$i<=$#list;$i++) {
-      if (defined($list[$i]) and defined($l[$i]) and ($list[$i] eq $l[$i])) {
-        if (not defined($index) or $i > $index) {
-          $index = $i;
-        }
-      }
-      else {
-        last;
-      }
-    }
+  my $unames = $self->{state}{uniquelistcount}{global}{final};
+  my $node = $self->_final_list_trie('all', $unames);
+  # The list itself is not "another" list
+  my $self_count = exists($unames->{join("\x{10FFFD}", @list)}) ? 1 : 0;
+  for (my $i=0;$i<=$#list;$i++) {
+    $node = $node->{c}{$list[$i]} or last;
+    last unless $node->{n} > $self_count; # no other list shares this prefix
+    $index = $i;
   }
 
   if (defined($index)) { # one or more similar lists
@@ -2311,17 +2307,65 @@
     $unames = $self->{state}{uniquelistcount}{global}{final}{$labelyear};
   }
 
-  foreach my $l_s (keys $unames->%*) {
-    my @l = split("\x{10FFFD}", $l_s);
-    # If list is shorter than the list we are checking, it's irrelevant
-    next if $#l < $list->$#*;
-    # If list matches at $n, it's irrelevant
-    next if ($list_one[$n-1] eq $l[$n-1]);
-    # If list doesn't match up to $n - 1, it's irrelevant
-    next unless Compare([@list_one[0 .. $n-2]], [@l[0 .. $n-2]]);
-    return 1;
+  return 0 unless $unames;
+  my $node = $self->_final_list_trie($ul eq 'minyear' ? "year:$labelyear" : 'all', $unames);
+  # Follow the first $n - 1 names
+  foreach my $name (@list_one[0 .. $n-2]) {
+    $node = $node->{c}{$name} or return 0;
   }
+  # Is there a list which differs at $n and is at least as long?
+  my ($best, $bestlen, $secondlen) = $node->{top}->@*;
+  return 1 if defined($best) and $best ne $list_one[$n-1] and $bestlen >= $list->$#*;
+  return 1 if defined($secondlen) and $secondlen >= $list->$#*;
   return 0;
+}
+
+=head2 _final_list_trie
+
+    A prefix tree over the complete lists in $unames (keys are lists joined
+    with U+10FFFD). Each node has:
+
+    c   - children by name
+    n   - the number of lists with the prefix ending at this node
+    top - [name, last index] of the child leading to the longest list,
+          and the longest last index among the other children
+
+    Built on first use and cached until the complete-list counts change, so
+    namelist_differs_index/nth no longer scan every list for every entry.
+
+=cut
+
+sub _final_list_trie {
+  my ($self, $cachekey, $unames) = @_;
+  my $cache = $self->{state}{uniquelistcount}{trie} //= {};
+  return $cache->{$cachekey} if exists $cache->{$cachekey};
+  my $root = {n => 0, c => {}, len => {}};
+  foreach my $l_s (keys $unames->%*) {
+    my @l = split("\x{10FFFD}", $l_s);
+    my $node = $root;
+    foreach my $name (@l) {
+      $node->{len}{$name} = $#l if ($node->{len}{$name} // -1) < $#l;
+      $node = $node->{c}{$name} //= {n => 0, c => {}, len => {}};
+      $node->{n}++;
+    }
+  }
+  my @todo = ($root);
+  while (my $node = shift @todo) {
+    my ($best, $bestlen, $secondlen);
+    while (my ($name, $len) = each $node->{len}->%*) {
+      if (not defined($bestlen) or $len > $bestlen) {
+        $secondlen = $bestlen;
+        ($best, $bestlen) = ($name, $len);
+      }
+      elsif (not defined($secondlen) or $len > $secondlen) {
+        $secondlen = $len;
+      }
+    }
+    $node->{top} = [$best, $bestlen, $secondlen];
+    delete $node->{len};
+    push @todo, values $node->{c}->%*;
+  }
+  return $cache->{$cachekey} = $root;
 }
 
 1;
```

The cache lives under `{state}{uniquelistcount}`, so `reset_uniquelistcount` and `reset_state` clear it too. The keys it scans are the same as before, including the `uniquelist=minyear` year sub-hashes inside `{final}`, so the answers are the same.

### Checks

- The `.bbl` is byte-identical before and after on the benchmark files above.
- It is also byte-identical on 60 random documents built from a pool of seven names. These have lists of 1–6 names, some with `and others`, three years, and `uniquelist=true` or `minyear` with assorted `uniquename`/`maxcitenames`. Between them they set 646 `ul=` values.
- Over 776 test documents (biber's `t/tdata` fixtures, biblatex's examples, 129 style/option combinations and real-world documents), the only `.bbl`s that changed were those containing the other bugs I'm filing alongside this.
- With this patch and those five, all 1,275 tests in `t/` pass. The code is unchanged on `dev` (d6e35e7), where the patch applies cleanly.

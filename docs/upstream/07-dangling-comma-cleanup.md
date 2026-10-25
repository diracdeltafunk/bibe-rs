<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/526 (2026-10-25). Tracks BB-015 (docs/biber-bugs.md). -->

**Title:** A comma before `}}` in field data is deleted from the `.bbl` (`title = {Proceedings {Vol. 1, Part 2,}}`)

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

The "Clean up dangling commas" step in `instantiate_entry` (DataList.pm:2227) runs `s|,(?:\n\s+)?\}\}|}}|gxms` over the whole instantiated entry, user data included. Any field whose value has a comma right before two closing braces loses the comma:

| Field in `.bib` | `.bbl` (biber 2.22) |
|---|---|
| `title = {Proceedings {Vol. 1, Part 2,}}` | `\field{title}{Proceedings {Vol. 1, Part 2}}` |
| `note = {See {ibid.,}}` | `\field{note}{See {ibid.}}` |

### Minimal example

```bibtex
@book{a, author = {Doe, John}, title = {Proceedings {Vol. 1, Part 2,}}, date = {2000},
         note = {See {ibid.,}}}
```

```latex
\documentclass{article}
\usepackage{biblatex}
\addbibresource{test.bib}
\begin{document}
\nocite{*}
\printbibliography
\end{document}
```

### Where the dangling commas come from

The cleanup exists for name lists. With uniquename on, each name part line is followed by a placeholder line, joined with `",\n"`. When a placeholder isn't needed, it is removed with `s|\s+<BDS>UNP-$np-$nid</BDS>,?||`. For the last name part this leaves the separator comma in front of the closing braces (`giveni={J\bibinitperiod},}}%`). That is what the cleanup catches.

### Suggested fix

Remove the placeholder together with the comma *before* it. That separator always exists, because the placeholder always follows the `…i={…}` line. Then no comma is left dangling, and the cleanup over the whole entry can go:

```diff
--- a/lib/Biber/DataList.pm
+++ b/lib/Biber/DataList.pm
@@ -1894,7 +1894,7 @@
           $entry_string =~ s|<BDS>UNP-$nid</BDS>|$str|gxms;
           foreach my $np ($n->get_nameparts) {
             if ($self->is_unbasepart($nlid, $nid, $np)) {
-              $entry_string =~ s|\s+<BDS>UNP-$np-$nid</BDS>,?||gxms;
+              $entry_string =~ s|,\s+<BDS>UNP-$np-$nid</BDS>||gxms;
             }
             else {
               $str = "${np}un=" . $self->get_unparts($nlid, $nid, $np);
@@ -1905,7 +1905,7 @@
         else {
           $entry_string =~ s|<BDS>UN[SP]-$nid</BDS>,?||gxms;
           foreach my $np ($n->get_nameparts) {
-            $entry_string =~ s|\s+<BDS>UNP-$np-$nid</BDS>,?||gxms;
+            $entry_string =~ s|,\s+<BDS>UNP-$np-$nid</BDS>||gxms;
           }
         }
       }
@@ -2223,9 +2223,6 @@
     }
   }
 
-  # Clean up dangling commas
-  $entry_string =~ s|,(?:\n\s+)?\}\}|}}|gxms;
-
   # Clean up generic metadata which was not replaced
   $entry_string =~ s|^\s+<BDS>[^<]+</BDS>\n||gxms;
 
```

With this change the two fields keep their commas. Name lists come out byte-identical. I ran biber with this patch (and the five others I'm filing alongside it) over 776 test documents: biber's `t/tdata` fixtures, biblatex's examples, 129 style/option combinations over `biblatex-examples.bib`, and real-world documents. Apart from the reproducers of those bugs, the only `.bbl` that changed was this bug's own reproducer.

Tested on biber 2.22 built from source. With this patch and the five others I'm filing alongside it, all 1,275 tests in `t/` pass. The code is unchanged on `dev` (d6e35e7), where the patch applies cleanly.

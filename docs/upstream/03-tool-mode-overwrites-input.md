<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/522 (2026-10-24). Tracks BB-013 and BB-160 (docs/biber-bugs.md). -->

**Title:** `biber --tool` overwrites an input file that has no extension (and misplaces the output for dotted directories)

---

In tool mode, the output file name is made by replacing the input's extension with `_bibertool.bib` (or `.bltxml`/`.json`) using `s/\.[^.]+$/…/`. That regex goes wrong in two cases:

1. **No extension:** nothing matches, so the output name *is* the input name, and biber overwrites the user's data file with the reformatted output. Anything tool mode doesn't write back is lost. For example, `@preamble`s are not written in tool mode, and there's no backup.
2. **Dotted directory:** with input `my.dir/refs`, the regex matches `.dir/refs`, so the output is `my_bibertool.bib` in the *current* directory.

### Example

```
$ cat refs
@book{k, author = {Doe, Jane}, title = {Book}, date = {2000}}
$ biber --tool refs
INFO - Output to refs              # the input file has been replaced
$ biber --tool my.dir/refs
INFO - Output to my_bibertool.bib  # not my.dir/refs_bibertool.bib
```

(biber 2.22, TeX Live 2026, macOS.)

### Suggested fix

Only replace an extension that is actually in the file-name part, and append the suffix when there is none:

```diff
--- a/bin/biber
+++ b/bin/biber
@@ -349,13 +349,13 @@
 else {
   if (Biber::Config->getoption('tool')) {
     if (Biber::Config->getoption('output_format') eq 'bibtex') { # tool .bib output
-      $outfile = $ARGV[0] =~ s/\.[^.]+$/_bibertool.bib/r;
+      $outfile = $ARGV[0] =~ s/(?:\.[^.\/\\]*)?\z/_bibertool.bib/r;
     }
     elsif (Biber::Config->getoption('output_format') eq 'biblatexml') { # tool .blxtxml output
-      $outfile = $ARGV[0] =~ s/\.[^.]+$/_bibertool.bltxml/r;
+      $outfile = $ARGV[0] =~ s/(?:\.[^.\/\\]*)?\z/_bibertool.bltxml/r;
     }
     elsif (Biber::Config->getoption('output_format') eq 'bibjson') { # bibjson output
-      $outfile = $ARGV[0] =~ s/\.[^.]+$/_bibertool.json/r;
+      $outfile = $ARGV[0] =~ s/(?:\.[^.\/\\]*)?\z/_bibertool.json/r;
     }
   }
   else {
```

With this regex:

| Input | Output |
|---|---|
| `refs` | `refs_bibertool.bib` |
| `refs.bib` | `refs_bibertool.bib` |
| `a.b.bib` | `a.b_bibertool.bib` |
| `my.dir/refs` | `my.dir/refs_bibertool.bib` |
| `C:\my.dir\refs` | `C:\my.dir\refs_bibertool.bib` |

The output names of inputs that already worked are unchanged; I checked against the biber test suite's tool-mode tests and about 110 other tool-mode runs. The code is unchanged on `dev` (df6438f).

A related case: with biblatexml input whose name has no `.bib`/`.bltxml` extension (e.g. `data.xml`), the generated `.rng` schema path (`Biber.pm:4631-4634`, `s/\.(?:bib|bltxml)$/.rng/`) equals the input path, so the data file is overwritten by the schema before it is read. The same approach (append `.rng` when the substitution doesn't match) would fix it.

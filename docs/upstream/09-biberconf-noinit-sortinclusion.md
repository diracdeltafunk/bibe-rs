<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/528 (2026-10-25). Tracks BB-005, BB-006 (docs/biber-bugs.md). -->

**Title:** `biber.conf`: the manual's `<noinit>`/`<nolabel>` examples crash ("Not a HASH reference"), and so does a single `<sortinclusion>`

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Three `biber.conf` options crash biber 2.22 when written in the documented form.

**1. `<noinit>` and `<nolabel>`.** Copying the example from the manual (§ "The noinit option") into `biber.conf`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<config>
  <noinit>
    <!-- strip lowercase prefices like 'al-' when generating initials -->
    <option value="\b\p{Ll}{2}\p{Pd}(?=\S)"/>
    <!-- strip diacritics when generating initials -->
    <option value="[\x{2bf}\x{2018}]"/>
  </noinit>
</config>
```

gives `Not a HASH reference at …/Biber/Config.pm line 560`, exit 102. The `<nolabel>` example does the same. `noinit` and `nolabel` are in the `ForceArray` list passed to `XMLin`, so `$v` is an array, but `_config_file_set` reads `$v->{option}`.

**2. `<sortinclusion>`.** A single `<sortinclusion>` (in `config.rnc`, the same shape as `<sortexclusion>`):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<config>
  <sortinclusion type="book">
    <inclusion>title</inclusion>
  </sortinclusion>
</config>
```

gives `Not an ARRAY reference at …/Biber/Config.pm line 602`, exit 102. `sortexclusion` and `exclusion` are in `ForceArray`, but `sortinclusion` and `inclusion` are not.

Any `.tex`/`.bib` reproduces these, e.g. `biber --configfile=noinit.conf test`.

### Suggested fix

```diff
--- a/lib/Biber/Config.pm
+++ b/lib/Biber/Config.pm
@@ -389,6 +389,8 @@
                                                             qr/\A(?:or)?filter\z/,
                                                             qr/\Asortexclusion\z/,
                                                             qr/\Aexclusion\z/,
+                                                            qr/\Asortinclusion\z/,
+                                                            qr/\Ainclusion\z/,
                                                             qr/\Asortingtemplate\z/,
                                                             qr/\Aconstant\z/,
                                                             qr/\Asort\z/,
@@ -554,11 +556,14 @@
       }
     }
     # mildly complex options - nosort/collate_options
-    elsif (lc($k) eq 'nosort' or
-           lc($k) eq 'noinit' or
-           lc($k) eq 'nolabel' ) {
+    elsif (lc($k) eq 'nosort') {
       Biber::Config->setconfigfileoption($k, $v->{option});
     }
+    # noinit and nolabel are in ForceArray, so $v is an array
+    elsif (lc($k) eq 'noinit' or
+           lc($k) eq 'nolabel' ) {
+      Biber::Config->setconfigfileoption($k, $v->[0]{option});
+    }
     # rather complex options
     elsif (lc($k) eq 'collate_options') {
       my $collopts = Biber::Config->getoption('collate_options');
```

With this change all three configs work, and the options take effect: with `<nolabel><option value=""/></nolabel>`, `O'Toole, Alexander` gets the label `O'T07` instead of `OTo07`.

### Related, for you to decide

- **The schema form is a third spelling.** `config.rnc` describes `<noinits><noinit value="…"/></noinits>` and `<nolabels><nolabel value="…"/></nolabels>`, and `t/tdata/biber-test.conf` and `tool-test.conf` use that form. biber silently ignores it: there is no branch for `noinits`/`nolabels`. That is also why `t/configfile.t` passes, even though the `noinit` regex in `biber-test.conf` has an extra `)`: `\b\p{Ll}{2}\p{Pd}(?=\S))`. Accepting the schema form too would need that regex fixed. Should the manual and the schema agree on one form?
- **The `nolabel` example is no longer the default.** The manual calls `[\p{P}\p{S}\p{C}]+` "the default", but `Constants.pm` now uses `[\p{Pc}\p{Ps}\p{Pe}\p{Pi}\p{Pf}\p{Po}\p{S}\p{C}]+`, which keeps hyphens (`\p{Pd}`).

Tested on biber 2.22 built from source. With this patch and the five others I'm filing alongside it, all 1,275 tests in `t/` pass. The code is unchanged on `dev` (d6e35e7), where the patch applies cleanly.

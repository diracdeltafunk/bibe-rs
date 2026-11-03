<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/560 (2026-11-03). Tracks BB-080, BB-081, BB-131, BB-163 (docs/biber-bugs.md); the biblatexml schema part (BB-164) was dropped after plk/biber#552 and #554. -->

**Title:** Validation: invalid ISBNs and ISMNs pass the data-model checks, and `--validate-control` rejects `map_entry_nocite`

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biber 2.22 and `dev` (a91aae4).

**1. Invalid ISBN checksums pass the `isbn` data constraint.** With

```bibtex
@book{good, author={A, A}, title={G}, isbn={978-3-16-148410-0}, issn={0378-5955}}
@book{bad, author={B, B}, title={B}, isbn={978-3-16-148410-1}, issn={0378-5954}}
@book{badism, author={C, C}, title={C}, ismn={M-2306-7118-8}}
```

and the default data model, `biber --validate-datamodel` prints

```
WARN - ISBN '978-3-16-148410-1' in entry 'bad' is invalid - run biber with '--validate_datamodel' for details.
WARN - Datamodel: book entry 'bad' (t.bib): Invalid ISSN in value of field 'issn'
```

(plus unrelated warnings about the missing dates and about `issn` not being a `@book` field). The first message asks for `--validate_datamodel`, which is already on, and no data-model warning about the ISBN follows. The `isbn` validator in `%DM_DATATYPES` (Constants.pm) only checks that `Business::ISBN->new($v)` returns an object, which it does for a wrong check digit; `is_valid` is never consulted.

**2. Invalid ISMNs are never reported.** The ISMN `M-2306-7118-8` above (the correct check digit is 7) gives no warning at all. `Business::ISMN::is_valid` returns `GOOD_ISMN` (1) for a valid number and negative codes (`-1` bad checksum, `-3` invalid publisher, …) otherwise, so `unless ($ismn and $ismn->is_valid)` never fires.

**3. The ISSN and ISMN validators read `$_` instead of their argument** (`Business::ISSN->new($_)`, `Business::ISMN->new($_)`). They work only because the caller in `check_data_constraints` happens to call them inside a `foreach` over the values; any other caller gets garbage.

**4. `--validate-control` rejects control files from biblatex 3.21 that use `\step[entrynocite]`.** biber implements `map_entry_nocite` on `map_entry_new` and `map_entry_clone` steps (Input/file/bibtex.pm, "Add to the section if explicitly nocited in the map"), and biblatex writes it, but `bcf.rnc`/`bcf.rng` don't allow the attribute. The `.bcf` of any document using biblatex-chicago's `chicago-notes` style contains

```xml
<bcf:map_step map_entry_clone="$1-$MAPLOOP" map_entry_nocite="1"/>
```

and `biber --validate-control` stops with `ERROR - '….bcf' failed to validate against schema '…/bcf.rng'`, leaving an empty `.bbl`. Without `--validate-control` the same file is processed normally.

### Suggested fix

```diff
--- a/data/schemata/bcf.rnc
+++ b/data/schemata/bcf.rnc
@@ -897,10 +897,12 @@ map =
     element bcf:per_nottype { xsd:string {minLength="1"} }*,
     element bcf:map_step {
       (
-        ( attribute map_entry_clone { xsd:string {minLength="1"} }
+        ( attribute map_entry_clone { xsd:string {minLength="1"} },
+          attribute map_entry_nocite { "1" }?
         ) |
         ( attribute map_entry_new { xsd:string {minLength="1"} },
-          attribute map_entry_newtype { xsd:string {minLength="1"} }
+          attribute map_entry_newtype { xsd:string {minLength="1"} },
+          attribute map_entry_nocite { "1" }?
         ) |
         ( attribute map_entrykey_cited { "1" }?,
           attribute map_entrykey_nocited { "1" }?,
--- a/data/schemata/bcf.rng
+++ b/data/schemata/bcf.rng
@@ -2156,11 +2156,18 @@
       <oneOrMore>
         <element name="bcf:map_step">
           <choice>
-            <attribute name="map_entry_clone">
-              <data type="string">
-                <param name="minLength">1</param>
-              </data>
-            </attribute>
+            <group>
+              <attribute name="map_entry_clone">
+                <data type="string">
+                  <param name="minLength">1</param>
+                </data>
+              </attribute>
+              <optional>
+                <attribute name="map_entry_nocite">
+                  <value>1</value>
+                </attribute>
+              </optional>
+            </group>
             <group>
               <attribute name="map_entry_new">
                 <data type="string">
@@ -2172,6 +2179,11 @@
                   <param name="minLength">1</param>
                 </data>
               </attribute>
+              <optional>
+                <attribute name="map_entry_nocite">
+                  <value>1</value>
+                </attribute>
+              </optional>
             </group>
             <group>
               <optional>
--- a/lib/Biber/Constants.pm
+++ b/lib/Biber/Constants.pm
@@ -180,7 +180,7 @@ our %DM_DATATYPES = (
                        }
 
                        my $isbn = Business::ISBN->new($v);
-                       if (not $isbn) {
+                       unless ($isbn and $isbn->is_valid == Business::ISBN::GOOD_ISBN()) {
                          return 0;
                        }
                        return 1;
@@ -189,7 +189,7 @@ our %DM_DATATYPES = (
                        my $v = shift;
                        require Business::ISSN;
 
-                       my $issn = Business::ISSN->new($_);
+                       my $issn = Business::ISSN->new($v);
                        unless ($issn and $issn->is_valid) {
                          return 0;
                        }
@@ -198,8 +198,8 @@ our %DM_DATATYPES = (
                      ismn => sub {
                        my $v = shift;
                        require Business::ISMN;
-                       my $ismn = Business::ISMN->new($_);
-                       unless ($ismn and $ismn->is_valid) {
+                       my $ismn = Business::ISMN->new($v);
+                       unless ($ismn and $ismn->is_valid == Business::ISMN::GOOD_ISMN()) {
                          return 0;
                        }
                        return 1;
```

`Business::ISBN::GOOD_ISBN` and `Business::ISMN::GOOD_ISMN` are called as functions because the modules are loaded at run time with `require`. With the patch, the example in 1 and 2 also gives `Invalid ISBN in value of field 'isbn'` and `Invalid ISMN in value of field 'ismn'`, and the chicago-notes `.bcf` validates. All tests in `t/` pass on `dev` (a91aae4) with this patch, applied together with the others I'm filing alongside it, including the end-to-end tests run with `BIBER_DEV_TESTS=1`.

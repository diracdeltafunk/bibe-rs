<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-080, BB-081, BB-131, BB-163, BB-164 (docs/biber-bugs.md). -->

**Title:** Validation: invalid ISBNs and ISMNs pass the data-model checks, `--validate-control` rejects `map_entry_nocite`, and the generated biblatexml schema rejects valid files

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biber 2.22 and `dev` (1686b9c).

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

(plus the unrelated missing-date warnings). The first message asks for `--validate_datamodel`, which is already on, and no data-model warning about the ISBN follows. The `isbn` validator in `%DM_DATATYPES` (Constants.pm) only checks that `Business::ISBN->new($v)` returns an object, which it does for a wrong check digit; `is_valid` is never consulted.

**2. Invalid ISMNs are never reported.** The ISMN `M-2306-7118-8` above (the correct check digit is 7) gives no warning at all. `Business::ISMN::is_valid` returns `GOOD_ISMN` (1) for a valid number and negative codes (`-1` bad checksum, `-3` invalid publisher, …) otherwise, so `unless ($ismn and $ismn->is_valid)` never fires.

**3. The ISSN and ISMN validators read `$_` instead of their argument** (`Business::ISSN->new($_)`, `Business::ISMN->new($_)`). They work only because the caller in `check_data_constraints` happens to call them inside a `foreach` over the values; any other caller gets garbage.

**4. `--validate-control` rejects control files from biblatex 3.21 that use `\step[entrynocite]`.** biber implements `map_entry_nocite` on `map_entry_new` and `map_entry_clone` steps (Input/file/bibtex.pm, "Add to the section if explicitly nocited in the map"), and biblatex writes it, but `bcf.rnc`/`bcf.rng` don't allow the attribute. The `.bcf` of any document using biblatex-chicago's `chicago-notes` style contains

```xml
<bcf:map_step map_entry_clone="$1-$MAPLOOP" map_entry_nocite="1"/>
```

and `biber --validate-control` stops with `ERROR - '….bcf' failed to validate against schema '…/bcf.rng'` and no `.bbl`. Without `--validate-control` the same file is processed normally.

**5. The generated biblatexml schema rejects biber's own test file.** `biber --validate-bltxml` on `t/tdata/biblatexml.bcf` (which reads `t/tdata/biblatexml.bltxml`) gives `ERROR - 'biblatexml.bltxml' failed to validate against schema 'biblatexml.rng'`, although the file is read without problems otherwise. `xmllint --relaxng` and some bisection show several separate causes in `generate_bltxml_schema`:

- `<bltx:name id="…">` (read as `hashid`) has no `id` attribute in the schema.
- `<bltx:ids><bltx:key>…</bltx:key></bltx:ids>`, which is how the reader expects aliases, is not allowed: `ids` is an `entrykey` field and only gets plain text or `<bltx:list>`.
- `<bltx:annotation field="…" item="…" part="…" literal="…">`, which the reader's `_annotation` handler implements and which may occur many times per entry, is generated as an ordinary literal field (no attributes, at most once). The schema instead defines a `<bltx:mannotation>` element that the reader doesn't know.
- After fixing these, libxml2 (which biber uses through `XML::LibXML::RelaxNG`) still rejects the file, because the schema nests interleaves (the entry's `<interleave>` refers to one define per field type, each of which is again an `<interleave>`). Nested interleaves are valid RELAX NG, but libxml2 mishandles some of them: in the test file, the minimal failing subset is `ids`, `usere`, a `date`, `pagetotal` and `related`, while every pair of these validates. Writing the entry content as one flat interleave (a `define name="entrycontent" combine="interleave"` per field) is equivalent and validates.

With the patch below, `t/tdata/*.bltxml` all validate. Since the biblatexml writer change I proposed in the biblatexml output issue writes `xsv` fields as `<bltx:list>` and omits `type` on `related` items when there is no `relatedtype`, the patch also allows those (and makes the reader ignore a missing `type`), so that the tool's biblatexml output for `biblatex-examples.bib` validates against the schema written alongside it.

### Suggested fix

```diff
--- a/data/schemata/bcf.rnc
+++ b/data/schemata/bcf.rnc
@@ -897,10 +897,12 @@
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
@@ -180,7 +180,7 @@
                        }
 
                        my $isbn = Business::ISBN->new($v);
-                       if (not $isbn) {
+                       unless ($isbn and $isbn->is_valid == Business::ISBN::GOOD_ISBN()) {
                          return 0;
                        }
                        return 1;
@@ -189,7 +189,7 @@
                        my $v = shift;
                        require Business::ISSN;
 
-                       my $issn = Business::ISSN->new($_);
+                       my $issn = Business::ISSN->new($v);
                        unless ($issn and $issn->is_valid) {
                          return 0;
                        }
@@ -198,8 +198,8 @@
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
--- a/lib/Biber/DataModel.pm
+++ b/lib/Biber/DataModel.pm
@@ -1066,22 +1066,10 @@
   }
   $writer->endTag();# choice
   $writer->endTag();# attribute
-  $writer->startTag('interleave');
-
-  foreach my $ft ($dm->fieldtypes->@*) {
-    foreach my $dt ($dm->datatypes->@*) {
-      if ($dm->is_fields_of_type($ft, $dt)) {
-        next if $dt eq 'datepart'; # not legal in input, only output
-        $writer->comment("$dt ${ft}s");
-        $writer->emptyTag('ref', 'name' => "$dt$ft");
-      }
-    }
-  }
-
-  # Annotations
-  $writer->emptyTag('ref', 'name' => "mannotation");
-
-  $writer->endTag();# interleave
+  # Entry content. All fields are given as a single flat interleave (one
+  # 'entrycontent' define per field, combined below) since libxml2, which
+  # validates the data, mishandles nested interleaves
+  $writer->emptyTag('ref', 'name' => 'entrycontent');
   $writer->endTag();# entry element
   $writer->endTag();# oneOrMore
   $writer->endTag();# entries element
@@ -1092,11 +1080,11 @@
       if ($dm->is_fields_of_type($ft, $dt)) {
         next if $dt eq 'datepart'; # not legal in input, only output
         $writer->comment("$dt ${ft}s definition");
-        $writer->startTag('define', 'name' => "$dt$ft");
 
         # Name lists element definition
         # =============================
         if ($ft eq 'list' and $dt eq 'name') {
+          $writer->startTag('define', 'name' => 'entrycontent', 'combine' => 'interleave');
           $writer->startTag('zeroOrMore');# for example, XDATA doesn't need a name
           $writer->startTag('element', 'name' => "$bltx:names");
 
@@ -1165,6 +1153,10 @@
 
           # gender attribute ref
           $writer->emptyTag('ref', 'name' => 'gender');
+          # id attribute (name hash id)
+          $writer->startTag('optional');
+          $writer->emptyTag('attribute', 'name' => 'id');
+          $writer->endTag();    # optional
           # namepart element
           $writer->startTag('oneOrMore');
           $writer->startTag('element', 'name' => "$bltx:namepart");
@@ -1200,13 +1192,14 @@
           $writer->endTag();    # choice
           $writer->endTag();    # names element
           $writer->endTag();    # zeroOrMore
+          $writer->endTag();# define
           # ========================
         }
         elsif ($ft eq 'list') {
           # lists element definition
           # ========================
-          $writer->startTag('interleave');
           foreach my $list ($dm->get_fields_of_type($ft, $dt)->@*) {
+            $writer->startTag('define', 'name' => 'entrycontent', 'combine' => 'interleave');
             $writer->startTag('optional');
             $writer->startTag('element', 'name' => "$bltx:$list");
             $writer->startTag('choice');
@@ -1227,15 +1220,15 @@
             $writer->endTag(); # choice
             $writer->endTag(); # $list element
             $writer->endTag(); # optional
+            $writer->endTag();# define
           }
-          $writer->endTag();# interleave
           # ========================
         }
         elsif ($ft eq 'field' and $dt eq 'uri') {
           # uri field element definition
           # ============================
-          $writer->startTag('interleave');
           foreach my $field ($dm->get_fields_of_type($ft, $dt)->@*) {
+            $writer->startTag('define', 'name' => 'entrycontent', 'combine' => 'interleave');
             $writer->startTag('optional');
             $writer->startTag('element', 'name' => "$bltx:$field");
             $writer->startTag('choice');
@@ -1244,15 +1237,15 @@
             $writer->endTag();   # choice
             $writer->endTag();   # $field element
             $writer->endTag();# optional
+            $writer->endTag();# define
           }
-          $writer->endTag();# interleave
           # ============================
         }
         elsif ($ft eq 'field' and $dt eq 'range') {
           # range field element definition
           # ==============================
-          $writer->startTag('interleave');
           foreach my $field ($dm->get_fields_of_type($ft, $dt)->@*) {
+            $writer->startTag('define', 'name' => 'entrycontent', 'combine' => 'interleave');
             $writer->startTag('optional');
             $writer->startTag('element', 'name' => "$bltx:$field");
 
@@ -1285,15 +1278,15 @@
             $writer->endTag();  # choice
             $writer->endTag();  # $field element
             $writer->endTag();  # optional
+            $writer->endTag();# define
           }
-          $writer->endTag();# interleave
           # ==============================
         }
         elsif ($ft eq 'field' and $dt eq 'entrykey') {
           # entrykey field element definition
           # =================================
-          $writer->startTag('interleave');
           foreach my $field ($dm->get_fields_of_type($ft, $dt)->@*) {
+            $writer->startTag('define', 'name' => 'entrycontent', 'combine' => 'interleave');
             $writer->startTag('optional');
             # related field is special
             if ($field eq 'related') {
@@ -1301,7 +1294,9 @@
               $writer->startTag('element', 'name' => "$bltx:list");
               $writer->startTag('oneOrMore');
               $writer->startTag('element', 'name' => "$bltx:item");
+              $writer->startTag('optional');
               $writer->emptyTag('attribute', 'name' => 'type');
+              $writer->endTag(); # optional
               $writer->emptyTag('attribute', 'name' => 'ids');
               $writer->startTag('optional');
               $writer->emptyTag('attribute', 'name' => 'string');
@@ -1331,19 +1326,28 @@
               $writer->endTag(); # item element
               $writer->endTag(); # oneOrMore
               $writer->endTag(); # list element
+              # ids are given as a sequence of key elements
+              if ($field eq 'ids') {
+                $writer->startTag('oneOrMore');
+                $writer->startTag('element', 'name' => "$bltx:key");
+                $writer->emptyTag('text');# text
+                $writer->endTag(); # key element
+                $writer->endTag(); # oneOrMore
+              }
               $writer->endTag(); # choice
               $writer->endTag(); # choice
               $writer->endTag(); # $field element
             }
             $writer->endTag(); # optional
+            $writer->endTag();# define
           }
-          $writer->endTag();# interleave
         }
         elsif ($ft eq 'field' and $dt eq 'date') {
           # date field element definition
           # Can't strongly type dates as we allow full ISO8601 meta characters
           # =============================
           my @types = map { s/date$//r } $dm->get_fields_of_type($ft, $dt)->@*;
+          $writer->startTag('define', 'name' => 'entrycontent', 'combine' => 'interleave');
           $writer->startTag('zeroOrMore');
           $writer->startTag('element', 'name' => "$bltx:date");
           $writer->startTag('optional');
@@ -1374,26 +1378,37 @@
           $writer->endTag(); # choice
           $writer->endTag(); # $field element
           $writer->endTag(); # zeroOrMore
+          $writer->endTag();# define
           # =============================
         }
         elsif ($ft eq 'field') {
           # field element definition
           # ========================
-          $writer->startTag('interleave');
           foreach my $field ($dm->get_fields_of_type($ft, $dt)->@*) {
+            next if $field eq 'annotation'; # see the annotation definition below
+            $writer->startTag('define', 'name' => 'entrycontent', 'combine' => 'interleave');
             $writer->startTag('optional');
             $writer->startTag('element', 'name' => "$bltx:$field");
             $writer->startTag('choice');
             $writer->emptyTag('ref', 'name' => 'xdata');
             $writer->emptyTag('text');# text
+            # xsv fields can also be given as a list of items
+            if (($dm->get_fieldformat($field) // '') eq 'xsv') {
+              $writer->startTag('element', 'name' => "$bltx:list");
+              $writer->startTag('oneOrMore');
+              $writer->startTag('element', 'name' => "$bltx:item");
+              $writer->emptyTag('text');# text
+              $writer->endTag(); # item element
+              $writer->endTag(); # oneOrMore
+              $writer->endTag(); # list element
+            }
             $writer->endTag(); # choice
             $writer->endTag(); # $field element
             $writer->endTag();# optional
+            $writer->endTag();# define
           }
-          $writer->endTag();# interleave
           # ========================
         }
-        $writer->endTag(); # define
       }
     }
   }
@@ -1428,11 +1443,15 @@
 
   # generic meta annotation element definition
   # ===========================================
+  # This is what the reader's _annotation handler accepts: without 'field' it is
+  # the entry's literal annotation field, with it a field/item/namepart annotation
   $writer->comment('generic annotation element definition');
-  $writer->startTag('define', 'name' => 'mannotation');
+  $writer->startTag('define', 'name' => 'entrycontent', 'combine' => 'interleave');
   $writer->startTag('zeroOrMore');
-  $writer->startTag('element', 'name' => "$bltx:mannotation");
+  $writer->startTag('element', 'name' => "$bltx:annotation");
+  $writer->startTag('optional');
   $writer->emptyTag('attribute', 'name' => 'field');
+  $writer->endTag(); # optional
   $writer->startTag('optional');
   $writer->emptyTag('attribute', 'name' => 'name');
   $writer->endTag(); # optional
@@ -1446,7 +1465,7 @@
   $writer->emptyTag('attribute', 'name' => 'literal');
   $writer->endTag(); # optional
   $writer->emptyTag('text');# text
-  $writer->endTag(); # mannotation element
+  $writer->endTag(); # annotation element
   $writer->endTag(); # zeroOrMore
   $writer->endTag();# define
   # ===========================
--- a/lib/Biber/Input/file/biblatexml.pm
+++ b/lib/Biber/Input/file/biblatexml.pm
@@ -909,7 +909,9 @@
   my $node = $entry->findnodes("./$f")->get_node(1);
   foreach my $item ($node->findnodes("./$NS:list/$NS:item")) {
     $bibentry->set_datafield('related', [ split(/$S/, $item->getAttribute('ids')) ]);
-    $bibentry->set_datafield('relatedtype', $item->getAttribute('type'));
+    if (my $type = $item->getAttribute('type')) {
+      $bibentry->set_datafield('relatedtype', $type);
+    }
     if (my $string = $item->getAttribute('string')) {
       $bibentry->set_datafield('relatedstring', $string);
     }
```

`Business::ISBN::GOOD_ISBN` and `Business::ISMN::GOOD_ISMN` are called as functions because the modules are loaded at run time with `require`. The patch is stacked on the patches of the earlier issues (the DataModel.pm context includes the biblatexml output changes). With it, the example in 1 and 2 also gives `Invalid ISBN in value of field 'isbn'` and `Invalid ISMN in value of field 'ismn'`, and the chicago-notes `.bcf` validates. All tests in `t/` pass on `dev` (1686b9c) with this patch.

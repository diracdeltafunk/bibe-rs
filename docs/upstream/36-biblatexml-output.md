<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-182, BB-183, BB-184, BB-185, BB-186, BB-187, BB-188, BB-114, BB-047 (docs/biber-bugs.md). -->

**Title:** biblatexml output can't be read back: aliases, keywords and related entries are lost, dates are unreadable, entries are unsorted; an unresolved XDATA reference in a range field crashes every writer

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

`biber --tool --output-format=biblatexml` is meant to convert a `.bib` file to biblatexml, but much of the data doesn't survive the trip back. With

```bibtex
@misc{b, title={Bee}, ids={balias}, keywords={k1,k2}, date={-0099-07-11T00:30:00}, author={Doe, John}, author+an={1="corr"; 1:family=fam}}
@misc{a, title={Ay}, date={1999-XX}, options={skipbib=true}, related={b}, pages={1--5}}
@misc{c, title={Cee}, date={2020-32}}
@misc{d, title={Dee}, date={1934-02~}}
```

`biber --tool --noconf --output-format=biblatexml -O out.bltxml test.bib` (2.22 and `dev` at 1686b9c) writes

```xml
  <bltx:entry id="b" entrytype="misc">
    <bltx:ids>
      <bltx:list>
        <bltx:item>balias</bltx:item>
      </bltx:list>
    </bltx:ids>
    <bltx:names type="author">…</bltx:names>
    <bltx:title>Bee</bltx:title>
    <bltx:date>-99-07-11</bltx:date>
    <bltx:annotation field="author" name="default" item="1" literal="1">corr</bltx:annotation>
    <bltx:annotation field="author" name="default" item="1" part="family" literal="0">fam</bltx:annotation>
  </bltx:entry>
  <bltx:entry id="a" entrytype="misc">
    <bltx:options>skipbib=true</bltx:options>
    <bltx:title>Ay</bltx:title>
    <bltx:pages>…</bltx:pages>
    <bltx:date>1999-00</bltx:date>
  </bltx:entry>
  <bltx:entry id="c" entrytype="misc">
    <bltx:title>Cee</bltx:title>
    <bltx:date>2020</bltx:date>
  </bltx:entry>
  <bltx:entry id="d" entrytype="misc">
    <bltx:title>Dee</bltx:title>
    <bltx:date>1934~-02</bltx:date>
  </bltx:entry>
```

and reading it back with `biber --tool --noconf --input-format=biblatexml out.bltxml` dies with `Can't call method "year" on an undefined value at .../Input/file/biblatexml.pm line 1163` (the crash on invalid biblatexml dates is reported separately).

1. **xsv fields are never written:** `keywords`, `related`, `entryset`. The loop is `foreach my $xsvf ($dm->get_fields_of_type('field', 'xsv')->@*)`, but the second argument of `get_fields_of_type` is a *datatype*, and xsv is a *format*, so the list is empty. Also, if the loop ran, it would write `<bltx:keywords>k1,k2</bltx:keywords>`, which the reader (`_split_list`) reads as the single keyword `k1,k2`, and `related` has to be written as `<bltx:related><bltx:list><bltx:item ids="…" type="…" string="…" options="…"/>` to be read by `_related`.
2. **Aliases are written as `<ids><list><item>`, but the reader only reads `<ids><key>`** (`Input/file/biblatexml.pm:156`), so `\cite{balias}` no longer works after a round trip.
3. **Dates are written in forms biber can't read:**
   - years are not padded to four digits (`-99-07-11`, `380`);
   - the approximate/uncertain markers are put after the year (`1934~-02`), but biber reads them only after the whole date;
   - `1999-XX` becomes `1999-00` (`date_monthday('XX')` is `sprintf('%.2d', 'XX')`, with an "isn't numeric" warning);
   - `winterS` is lost because of the `'WinterS' => 32` typo in the year-division map (`2020-32` becomes `2020`, the same typo as in `Output/bibtex.pm`);
   - the end date's approximate/uncertain markers are never written.
4. **Zero values in dates are dropped:** a time whose hour is `0` (`if (my $sh = …)`, so `T00:30:00` is lost), year 0 (`if (my $sf = …)`), and month/day components through `grep {$_}`.
5. **Entries are written in citation order, not sorted.** `create_output_section` iterates `$section->get_citekeys`, while the BibTeX writer uses the section's list, sorted with the `tool` sorting template, as the manual describes. `t/tool-bltxml.t` asserts the sorted order of the list (`get_keys`), but its expected output is in citation order.
6. **The output changes between identical runs.** Per-namelist and per-name option attributes are written in hash order (`keys $CONFIG_SCOPEOPT_BIBLATEX{NAMELIST}`, `…{NAME}`), and the generated `.rng` schema lists entry types, field types and datatypes in hash order (`DataModel::entrytypes`, `fieldtypes`, `datatypes` return `keys`). Four runs on the file above gave four different `.rng` files.
7. **Name parts.** `name_part_to_bltxml` splits a name part on every space or tie, including those inside braces, so `{\relax Ch}ristian` is written as the two parts `{\relax` and `Ch}ristian`. And a simple name part without an initial is written without its `type` attribute.
8. **(No visible effect.)** `name_to_biblatexml` and `name_part_to_bltxml` look for name and name-part annotations with `get_annotation('item', $key, $namefield, $count)`, i.e. with the name's index in the annotation-name slot, so they never find any; the reader doesn't read these attributes either. The annotations survive anyway, because `set_output_entry` also writes them as `<bltx:annotation item="…" part="…">` elements. The patch removes the dead code.

With the patch below, the output for the example is

```xml
  <bltx:entry id="b" entrytype="misc">
    <bltx:ids>
      <bltx:key>balias</bltx:key>
    </bltx:ids>
    <bltx:names type="author">…</bltx:names>
    <bltx:title>Bee</bltx:title>
    <bltx:keywords>
      <bltx:list>
        <bltx:item>k1</bltx:item>
        <bltx:item>k2</bltx:item>
      </bltx:list>
    </bltx:keywords>
    <bltx:date>-0099-07-11T00:30:00</bltx:date>
    <bltx:annotation …>corr</bltx:annotation>
    <bltx:annotation …>fam</bltx:annotation>
  </bltx:entry>
  <bltx:entry id="a" entrytype="misc">
    <bltx:options>skipbib=true</bltx:options>
    <bltx:title>Ay</bltx:title>
    <bltx:related>
      <bltx:list>
        <bltx:item ids="b" />
      </bltx:list>
    </bltx:related>
    <bltx:pages>…</bltx:pages>
    <bltx:date>1999-XX</bltx:date>
  </bltx:entry>
  <bltx:entry id="c" entrytype="misc">
    <bltx:title>Cee</bltx:title>
    <bltx:date>2020-32</bltx:date>
  </bltx:entry>
  <bltx:entry id="d" entrytype="misc">
    <bltx:title>Dee</bltx:title>
    <bltx:date>1934-02~</bltx:date>
  </bltx:entry>
```

and `.bib` → biblatexml → `.bib` gives the same file as `.bib` → `.bib`. (The reader also gets a small `_ids` handler, so that the `IDS` field is kept as for BibTeX input instead of being read from the whitespace text of `<bltx:ids>`.)

Three related problems found on the way:

9. **An unresolved XDATA reference in a range field crashes every writer.** For BibTeX input, `@article{r1, title={R}, pages={xdata=missing-pages}}` gives the expected warning (`references XDATA entry 'missing' which does not exist, not resolving`) and then `Can't use string ("xdata=missing-pages") as an ARRAY ref while "strict refs" in use at .../Output/bbl.pm line 535` (also at `Output/bibtex.pm` line 630 in tool mode). The range field holds the raw reference string, as other fields do, but the writers treat it as a list of ranges; for unresolved references in other fields, the raw string is written. The biblatexml reader stores the reference as a one-item list `[$xdatav]` instead, which `xdatarefcheck` misses, so the biblatexml writer dies at line 330 too. The patch makes the reader store the raw string, as the BibTeX reader does, and makes the `.bbl` and BibTeX writers write it like any other unresolved reference.
10. **Names without braces aren't protected in BibTeX output.** For names that come from biblatexml (or a sourcemap), `name_to_bibtex` has no braces to keep, so `<bltx:namepart type="family">Barnes and Noble</bltx:namepart>` is written as `AUTHOR = {Barnes and Noble and Doe, John}`, i.e. three names. The patch braces a part that contains a top-level `and` or comma, and a family name of several words with no given name or suffix after it, or with a lowercase word that would be read as a prefix.
11. An xsv field whose list became empty (e.g. `related` when the related entries don't exist) is written as `RELATED = {}` by the BibTeX writer. The patch skips empty lists.

### Suggested fix

```diff
--- a/lib/Biber/DataModel.pm
+++ b/lib/Biber/DataModel.pm
@@ -416,7 +416,7 @@
 
 sub fieldtypes {
   my $self = shift;
-  return [ keys $self->{fieldsbyfieldtype}->%* ];
+  return [ sort keys $self->{fieldsbyfieldtype}->%* ];
 }
 
 =head2 datatypes
@@ -427,7 +427,7 @@
 
 sub datatypes {
   my $self = shift;
-  return [ keys $self->{fieldsbydatatype}->%* ];
+  return [ sort keys $self->{fieldsbydatatype}->%* ];
 }
 
 
@@ -462,7 +462,7 @@
 
 sub entrytypes {
   my $self = shift;
-  return [ keys $self->{entrytypesbyname}->%* ];
+  return [ sort keys $self->{entrytypesbyname}->%* ];
 }
 
 
--- a/lib/Biber/Entry/Name.pm
+++ b/lib/Biber/Entry/Name.pm
@@ -226,7 +226,7 @@
 
 
   # Add per-name options
-  foreach my $no (keys $CONFIG_SCOPEOPT_BIBLATEX{NAME}->%*) {
+  foreach my $no (sort keys $CONFIG_SCOPEOPT_BIBLATEX{NAME}->%*) {
     if (defined($self->${\"get_$no"})) {
       my $nov = $self->${\"get_$no"};
 
@@ -241,10 +241,8 @@
     push @attrs, ('gender' => $g);
   }
 
-  # name scope annotation
-  if (my $ann = Biber::Annotation->get_annotation('item', $key, $namefield, $count)) {
-    push @attrs, ('annotation' => $ann);
-  }
+  # (Name and name part annotations are output as <annotation> elements by
+  # Output::biblatexml)
 
   $xml->startTag([$xml_prefix, 'name'], @attrs);
 
@@ -266,14 +264,10 @@
   my $np = $self->get_namepart($npn);
   my $nip = $self->get_namepart_initial($npn);
   if ($np) {
-    my $parts = [split(/[\s~]/, $np)];
+    # Split on spaces and ties outside braces
+    my $parts = [ map {s/\x{0}/ /gr} split(/[\s~]/, $np =~ s/(\{(?:[^{}]++|(?1))*\})/$1 =~ s|\s|\x{0}|gr/ger) ];
     my @attrs;
 
-    # namepart scope annotation
-    if (my $ann = Biber::Annotation->get_annotation('part', $key, $namefield, $count, $npn)) {
-      push @attrs, ('annotation' => $ann);
-    }
-
     # Compound name part
     if ($parts->$#* > 0) {
       $xml->startTag([$xml_prefix, 'namepart'], type => $npn, @attrs);
@@ -294,7 +288,7 @@
         $xml->startTag([$xml_prefix, 'namepart'], type => $npn, initial => $init, @attrs);
       }
       else {
-        $xml->startTag([$xml_prefix, 'namepart']);
+        $xml->startTag([$xml_prefix, 'namepart'], type => $npn, @attrs);
       }
       $xml->characters(NFC($parts->[0]));
       $xml->endTag();           # namepart
@@ -581,6 +575,21 @@
     }
   }
 
+  # Protect parts which would be read differently: a top-level 'and' or comma
+  # anywhere, or, in the family name, several words without a given name or
+  # suffix to follow, or a lowercase word which would be read as a prefix.
+  # (Names from biblatexml or sourcemaps have no braces to keep.)
+  foreach my $np (keys $parts->%*) {
+    next if $self->was_stripped($np);
+    (my $top = $parts->{$np}) =~ s/(\{(?:[^{}]++|(?1))*\})/{}/g;
+    if ($top =~ m/(?:\sand\s|,)/i or
+        ($np eq 'family' and
+         (($top =~ m/\s/ and not $parts->{given} and not $parts->{suffix}) or
+          $top =~ m/(?:^|\s)\p{Ll}\S*\s/))) {
+      $parts->{$np} = "{$parts->{$np}}";
+    }
+  }
+
   if (my $p = $parts->{prefix}) {$namestring .= "$p "};
   if (my $f = $parts->{family}) {$namestring .= $f};
   if (my $s= $parts->{suffix}) {$namestring .= ", $s"};
--- a/lib/Biber/Input/file/biblatexml.pm
+++ b/lib/Biber/Input/file/biblatexml.pm
@@ -39,6 +39,7 @@
 my $dm = Biber::Config->get_dm;
 my $handlers = {
                 'CUSTOM' => {'related' => \&_related,
+                             'ids' => \&_ids,
                              'annotation' => \&_annotation},
                 'field' => {
                             'default' => {
@@ -890,6 +891,16 @@
   return;
 }
 
+# Aliases (registered when the entry is found); keep them in the IDS field as
+# for BibTeX input
+sub _ids {
+  my ($bibentry, $entry, $f, $key) = @_;
+  if (my @ids = map {$_->textContent()} $entry->findnodes("./$f/$NS:key")) {
+    $bibentry->set_datafield('ids', [ @ids ]);
+  }
+  return;
+}
+
 # Related entries
 sub _related {
   my ($bibentry, $entry, $f, $key) = @_;
@@ -1009,7 +1020,7 @@
   if (my $xdatav = $node->getAttribute('xdata')) {
     $xdatav = "$xdmi$xnsi$xdatav"; # normalise to same as bibtex input
     $bibentry->add_xdata_ref(_norm($f), $xdatav);
-    $bibentry->set_datafield(_norm($f), [$xdatav]);
+    $bibentry->set_datafield(_norm($f), $xdatav); # raw value, as for BibTeX input
     return;
   }
 
--- a/lib/Biber/Output/bbl.pm
+++ b/lib/Biber/Output/bbl.pm
@@ -563,6 +563,11 @@
   foreach my $rfield ($dmh->{ranges}->@*) {
     # Performance - as little as possible here - loop over DM fields for every entry
     if ( my $rf = $be->get_field($rfield) ) {
+      # An unresolved XDATA reference is the raw string (as for other fields)
+      unless (ref($rf)) {
+        $acc .= "      \\field{$rfield}{$rf}\n";
+        next;
+      }
       # range fields are an array ref of two-element array refs [range_start, range_end]
       # range_end can be be empty for open-ended range or undef
       my @pr;
--- a/lib/Biber/Output/biblatexml.pm
+++ b/lib/Biber/Output/biblatexml.pm
@@ -113,12 +113,11 @@
 
   # Filter aliases which point to this key and insert them
   if (my @ids = sort grep {$section->get_citekey_alias($_) eq $key} $section->get_citekey_aliases) {
+    # <ids><key> is what Input::file::biblatexml reads
     $xml->startTag([$xml_prefix, 'ids']);
-    $xml->startTag([$xml_prefix, 'list']);
     foreach my $id (@ids) {
-      $xml->dataElement([$xml_prefix, 'item'], NFC($id));
+      $xml->dataElement([$xml_prefix, 'key'], NFC($id));
     }
-    $xml->endTag();# list
     $xml->endTag();# ids
   }
 
@@ -149,7 +148,7 @@
 
   # Per-entry options
   my @entryoptions;
-  foreach my $opt (Biber::Config->getblxentryoptions($secnum, $key)) {
+  foreach my $opt (sort (Biber::Config->getblxentryoptions($secnum, $key))) {
     push @entryoptions, $opt . '=' . Biber::Config->getblxoption($secnum, $opt, undef, $key);
   }
   $xml->dataElement([$xml_prefix, 'options'], NFC(join(',', @entryoptions))) if @entryoptions;
@@ -177,7 +176,7 @@
       }
 
       # Add per-namelist options
-      foreach my $nlo (keys $CONFIG_SCOPEOPT_BIBLATEX{NAMELIST}->%*) {
+      foreach my $nlo (sort keys $CONFIG_SCOPEOPT_BIBLATEX{NAMELIST}->%*) {
         if (defined($nf->${\"get_$nlo"})) {
           my $nlov = $nf->${\"get_$nlo"};
 
@@ -282,17 +281,21 @@
        $be->field_exists($field))) {
       next if $dm->get_fieldformat($field) eq 'xsv';
       next if $field eq 'crossref'; # this is handled above
+      next if $field eq 'relatedtype' or $field eq 'relatedstring'; # output with related
       my @attrs;
 
       $xml->dataElement([$xml_prefix, $field], NFC($val), @attrs);
     }
   }
 
-  # xsv fields
-  foreach my $xsvf ($dm->get_fields_of_type('field', 'xsv')->@*) {
+  # xsv fields (xsv is a format, not a datatype)
+  foreach my $xsvf (sort grep {$dm->get_fieldformat($_) eq 'xsv'} $dm->get_fields_of_fieldtype('field')->@*) {
     if (my $f = $be->get_field($xsvf)) {
+      next unless $f->@*; # e.g. related entries which were not found
       next if $xsvf eq 'ids'; # IDS is special
       next if $xsvf eq 'xdata'; # XDATA is special
+      next if $xsvf eq 'options'; # per-entry options are output above
+      next if $xsvf eq 'relatedoptions'; # output with related below
 
       # XDATA is special
       if (not Biber::Config->getoption('output_resolve_xdata') or
@@ -303,7 +306,34 @@
         }
       }
 
-      $xml->dataElement([$xml_prefix, $xsvf], NFC(join(',',$f->@*)));
+      # Related entries are read from <related><list><item ids="" type="" .../>
+      if ($xsvf eq 'related') {
+        my @rattrs = (ids => NFC(join(',', $f->@*)));
+        if (my $rt = $be->get_field('relatedtype')) {
+          push @rattrs, (type => NFC($rt));
+        }
+        if (my $rs = $be->get_field('relatedstring')) {
+          push @rattrs, (string => NFC($rs));
+        }
+        if (my $ro = $be->get_field('relatedoptions')) {
+          push @rattrs, (options => NFC(join(',', $ro->@*)));
+        }
+        $xml->startTag([$xml_prefix, 'related']);
+        $xml->startTag([$xml_prefix, 'list']);
+        $xml->emptyTag([$xml_prefix, 'item'], @rattrs);
+        $xml->endTag();# list
+        $xml->endTag();# related
+        next;
+      }
+
+      # A plain string is read as one item, so write a list
+      $xml->startTag([$xml_prefix, $xsvf]);
+      $xml->startTag([$xml_prefix, 'list']);
+      foreach my $i ($f->@*) {
+        $xml->dataElement([$xml_prefix, 'item'], NFC($i));
+      }
+      $xml->endTag();# list
+      $xml->endTag();# xsvf
     }
   }
 
@@ -353,33 +383,26 @@
     my $overrided;
 
     my ($d) = $datefield =~ m/^(.*)date$/;
-    if (my $sf = $be->get_field("${d}year") ) { # date exists if there is a year
+    my $sf = $be->get_field("${d}year");
+    if (defined($sf) and $sf ne '') { # date exists if there is a year
 
       push @attrs, ('type', $d) if $d; # ignore for main date
 
       $xml->startTag([$xml_prefix, 'date'], @attrs);
 
-      # Uncertain and approximate dates
-      if ($be->get_field("${d}dateuncertain") and
-          $be->get_field("${d}dateapproximate")) {
-        $sf .= '%';
-      }
-      else {
-        # Uncertain dates
-        if ($be->get_field("${d}dateuncertain")) {
-          $sf .= '?';
-        }
-
-        # Approximate dates
-        if ($be->get_field("${d}dateapproximate")) {
-          $sf .= '~';
-        }
-      }
+      # 'XX' (unspecified) is written as it is, numbers are padded; years need
+      # four digits to be read again
+      my $pad = sub { $_[0] =~ m/^\d+$/ ? sprintf('%.2d', $_[0]) : $_[0] };
+      my $ypad = sub { $_[0] =~ m/^(-?)(\d{1,3})$/ ? $1 . sprintf('%.4d', $2) : $_[0] };
 
-      # Unknown dates
-      if ($be->get_field("${d}dateunknown")) {
-        $sf = 'unknown';
-      }
+      # Uncertain and approximate dates. These go after the whole date
+      # (1934-02~), as biber reads them.
+      my $qualifier = sub {
+        my $e = shift;
+        my $u = $be->get_field("${d}${e}dateuncertain");
+        my $a = $be->get_field("${d}${e}dateapproximate");
+        return $u && $a ? '%' : $u ? '?' : $a ? '~' : '';
+      };
 
       my %yeardivisions = ( 'spring'  => 21,
                             'summer'  => 22,
@@ -392,7 +415,7 @@
                             'springS' => 29,
                             'summerS' => 30,
                             'autumnS' => 31,
-                            'WinterS' => 32,
+                            'winterS' => 32,
                             'Q1'      => 33,
                             'Q2'      => 34,
                             'Q3'      => 35,
@@ -449,68 +472,57 @@
       if (my $s = $be->get_field("${d}endyeardivision")) {
         $overrideem = $yeardivisions{$s};
       }
-      $sf = $overridey || $sf;
 
-      # strip undefs
+      # Unknown dates
+      if ($be->get_field("${d}dateunknown")) {
+        $sf = 'unknown';
+      }
+      else {
+        $sf = $ypad->($overridey // $sf);
+      }
+
+      # strip undefs (but not 0)
       push @start,
-        grep {$_}
-          $sf,
-            date_monthday($overridem || $be->get_field("${d}month")),
-              date_monthday($overrided || $be->get_field("${d}day"));
-      push @end,
-        grep {defined($_)} # because end can be def but empty
-          $be->get_field("${d}endyear"),
-            date_monthday($overrideem || $be->get_field("${d}endmonth")),
-              date_monthday($be->get_field("${d}endday"));
-      # Date range
-      if (@end) {
-        my $start = NFC(join('-', @start));
-        my $end = NFC(join('-', @end));
-
-        # If start hour, there must be minute and second
-        if (my $sh = $be->get_field("${d}hour")) {
-          $start .= NFC('T' . sprintf('%.2d', $sh) . ':' .
-            sprintf('%.2d', $be->get_field("${d}minute")) . ':' .
-              sprintf('%.2d', $be->get_field("${d}second")));
-        }
+        $sf,
+          map {$pad->($_)} grep {defined}
+            $overridem // $be->get_field("${d}month"),
+              $overrided // $be->get_field("${d}day");
+      my $ey = $be->get_field("${d}endyear");
+      if (defined($ey)) { # end can be def but empty
+        push @end,
+          ($ey eq '' ? '' : $ypad->($ey)),
+            map {$pad->($_)} grep {defined}
+              $overrideem // $be->get_field("${d}endmonth"),
+                $be->get_field("${d}endday");
+      }
 
-        # start timezone
-        if (my $stz = $be->get_field("${d}timezone")) {
-          $stz =~ s/\\bibtzminsep\s+/:/;
-          $start .= NFC($stz);
+      # If hour, there must be minute and second
+      my $time = sub {
+        my $e = shift;
+        my $h = $be->get_field("${d}${e}hour");
+        my $t = '';
+        if (defined($h)) {
+          $t = 'T' . sprintf('%.2d', $h) . ':' .
+            sprintf('%.2d', $be->get_field("${d}${e}minute")) . ':' .
+              sprintf('%.2d', $be->get_field("${d}${e}second"));
         }
-
-        # If end hour, there must be minute and second
-        if (my $eh = $be->get_field("${d}endhour")) {
-          $end .= NFC('T' . sprintf('%.2d', $eh) . ':' .
-            sprintf('%.2d', $be->get_field("${d}endminute")) . ':' .
-              sprintf('%.2d', $be->get_field("${d}endsecond")));
+        if (my $tz = $be->get_field("${d}${e}timezone")) {
+          $tz =~ s/\\bibtzminsep\s+/:/;
+          $t .= $tz;
         }
+        return $t;
+      };
 
-        # end timezone
-        if (my $etz = $be->get_field("${d}endtimezone")) {
-          $etz =~ s/\\bibtzminsep\s+/:/;
-          $end .= NFC($etz);
-        }
+      my $start = NFC(join('-', @start) . $qualifier->('') . $time->(''));
 
+      # Date range
+      if (@end) {
+        my $end = NFC(join('-', @end) . $qualifier->('end') . $time->('end'));
         $xml->dataElement([$xml_prefix, 'start'], $start);
         $xml->dataElement([$xml_prefix, 'end'], $end);
       }
       else { # simple date
-        $xml->characters(NFC(join('-', @start)));
-
-        # If start hour, there must be minute and second
-        if (my $sh = $be->get_field("${d}hour")) {
-          $xml->characters(NFC('T' . sprintf('%.2d', $sh) . ':' .
-                               sprintf('%.2d', $be->get_field("${d}minute")) . ':' .
-                               sprintf('%.2d', $be->get_field("${d}second"))));
-        }
-
-        # start timezone
-        if (my $stz = $be->get_field("${d}timezone")) {
-          $stz =~ s/\\bibtzminsep\s+/:/;
-          $xml->characters(NFC($stz));
-        }
+        $xml->characters($start);
       }
       $xml->endTag();           # date
     }
@@ -618,8 +630,9 @@
   my $secnum = $Biber::MASTER->get_current_section;
   my $section = $Biber::MASTER->sections->get_section($secnum);
 
-  # We rely on the order of this array for the order of the output
-  foreach my $k ($section->get_citekeys) {
+  # Output in the order of the section's (tool-sorted) list, as for BibTeX output
+  my $list = $Biber::MASTER->datalists->get_lists_for_section($secnum)->[0];
+  foreach my $k ($list ? $list->get_keys->@* : $section->get_citekeys) {
     # Regular entry
     my $be = $section->bibentry($k) or biber_error("Cannot find entry with key '$k' to output");
     $self->set_output_entry($be, $section, Biber::Config->get_dm);
--- a/lib/Biber/Output/bibtex.pm
+++ b/lib/Biber/Output/bibtex.pm
@@ -298,6 +298,7 @@
     # output with its own special macro below
     next if $field eq 'keywords';
     if (my $f = $be->get_field($field)) {
+      next unless $f->@*; # e.g. related entries which were not found
       my $fl = join(',', $f->@*);
       unless (Biber::Config->getoption('output_resolve_xdata')) {
         my $xd = xdatarefcheck($fl);
@@ -310,7 +311,8 @@
   # Ranges
   foreach my $rfield ($dmh->{ranges}->@*) {
     if ( my $rf = $be->get_field($rfield) ) {
-      my $rfl = construct_range($rf);
+      # An unresolved XDATA reference is the raw string
+      my $rfl = ref($rf) ? construct_range($rf) : $rf;
       unless (Biber::Config->getoption('output_resolve_xdata')) {
         my $xd = xdatarefcheck($rfl);
         $rfl = $xd // $rfl;
--- a/t/tool-bltxml-inout.t
+++ b/t/tool-bltxml-inout.t
@@ -79,10 +79,8 @@
 <bltx:entries xmlns:bltx="http://biblatex-biber.sourceforge.net/biblatexml">
   <bltx:entry id="bltx1" entrytype="book">
     <bltx:ids>
-      <bltx:list>
-        <bltx:item>bltx1a1</bltx:item>
-        <bltx:item>bltx1a2</bltx:item>
-      </bltx:list>
+      <bltx:key>bltx1a1</bltx:key>
+      <bltx:key>bltx1a2</bltx:key>
     </bltx:ids>
     <bltx:options>useprefix=false</bltx:options>
     <bltx:names type="afterword">
@@ -168,7 +166,7 @@
       <bltx:start>1990-05-16</bltx:start>
       <bltx:end>1990-05-17</bltx:end>
     </bltx:date>
-    <bltx:date type="orig">-356</bltx:date>
+    <bltx:date type="orig">-0356</bltx:date>
     <bltx:date type="url">
       <bltx:start>1991~</bltx:start>
       <bltx:end></bltx:end>
```

`t/tool-bltxml.t` also needs its two expected strings updated: aliases as `<bltx:key>`, the `keywords` of entry `i3Š`, and the entries in the order of the sorted list (the order the test already asserts with `get_keys`). That change is mechanical and long, so it is not shown. The patch is on top of the one for the date-field report. All tests in `t/` pass on `dev` (1686b9c) with these changes.

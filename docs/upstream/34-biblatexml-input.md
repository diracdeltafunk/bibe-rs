<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-050, BB-051, BB-170–BB-181 (docs/biber-bugs.md). -->

**Title:** biblatexml input: crashes on invalid dates, fields without a handler, quotes in keys and several map steps; aliases, `relatedoptions`, `eprint` attributes and simple-part initials are ignored

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

All of these are in `Biber::Input::file::biblatexml` and were checked with biber 2.22 and current `dev` (1686b9c). The examples are complete `<bltx:entry>` elements inside a normal `<bltx:entries xmlns:bltx="http://biblatex-biber.sourceforge.net/biblatexml">` file, used with `\addbibresource[datatype=biblatexml]{test.bltxml}` and `\nocite{*}` unless stated otherwise.

### Crashes

**1. Any unparseable date kills biber** instead of giving the intended warning:

```xml
<bltx:entry id="a" entrytype="book"><bltx:title>T</bltx:title><bltx:date>abc</bltx:date></bltx:entry>
```

```
Can't call method "year" on an undefined value at .../Biber/Input/file/biblatexml.pm line 1163.
```

The two date branches test `if (my ($sdate, undef, undef, $unspec) = parse_date_range(…))`, a list assignment, which is always true (it counts the four returned values), so the "Invalid format" `else` branches are unreachable. The same happens for `/2001`, `-380`, ` 2001 ` and an invalid range start.

**2. A data-model field with no biblatexml handler kills biber:** `<bltx:year>2000</bltx:year>` gives `Can't use an undefined value as a subroutine reference at … line 848` (`_get_handler` returns `undef` for date parts).

**3. A cited key containing `'` kills biber.** Keys are interpolated into XPath expressions: `\cite{o'brien}` gives

```
XPath error : Invalid predicate
/bltx:entries/bltx:entry[@id='o'brien']
```

**4. Malformed or unexpected XML kills biber with libxml2's messages** (exit 255, empty `.bbl`) instead of an `ERROR` naming the file: a tag mismatch (`:4: parser error : Opening and ending tag mismatch`); a file whose namespace is bound to another prefix (`XPath error : Undefined namespace prefix`) or is the default namespace (`Can't call method "textContent" on an undefined value at … line 910`); an invalid XPath in a map (`\step[fieldsource={./bltx:title[}]`: `XPath error : Invalid expression`); and a map that creates a node from an XPath step selecting nothing (`fieldset={./bltx:names[@type='author']/bltx:name[2]/@gender}`).

**5. Several map steps kill biber** (each with `\DeclareSourcemap{\maps[datatype=biblatexml]{…}}`):

| step | error |
|---|---|
| `\pertype{book}` | `Can't locate object method "type" via package "XML::LibXML::Element"` (`$entry->type`) |
| `\step[notfield=pubstate, fieldset=pubstate, fieldvalue={x}]` (no `fieldsource`) | `findnodes` on an empty XPath |
| `\step[fieldsource=userb, matches={beta,gamma}, replace={B,C}]` | `Can't locate object method "set" via package "XML::LibXML::Element"` (`$etarget->set`) |
| `\step[fieldset=userc, null]` on an entry without `userc` | `Can't call method "unbindNode" on an undefined value` |
| `\step[fieldsource=note, match=\regexp{DROP}, final]\step[entrynull]` | the key stays cited, then `Can't call method "get_field" on an undefined value at Biber.pm line 4761` (the same crash as the separate report on `entrynull` after `entrynew` for BibTeX input) |
| `\step[…, final]\step[entryclone={cl:}]` | the original is re-created under its own key and biber dies on the clone key, in the same place |

**6. A misplaced parenthesis in the match/replace step** passes `\$cnerror` to `unless` instead of `_changenode`, so `_changenode`'s error path dies instead of warning.

### Wrong or lost data

**7. `eprint` `type`/`class` attributes are ignored.** `fc(_norm($f) eq "$NS:eprint")` has the parenthesis in the wrong place, and `_norm` has already removed the prefix, so `<bltx:eprint type="arxiv" class="cs.DL">1234.5678</bltx:eprint>` gives no `eprinttype`/`eprintclass`.

**8. URIs are escaped differently from BibTeX input.** `<bltx:url>http://x.org/é ü</bltx:url>` gives `url` *and* `urlraw` = `http://x.org/e%CC%81%20u%CC%88`: the URI is escaped at input, from the NFD string (so `é` becomes `e%CC%81`), and `urlraw` gets the escaped value. For the same URL from a `.bib` file, `urlraw` is `http://x.org/é ü` and `url` is `http://x.org/%C3%A9%20%C3%BC` (escaped at output, from NFC).

**9. The `initial` attribute of a simple name part is read from the `<bltx:name>`.** `<bltx:namepart type="given" initial="X">Anders</bltx:namepart>` gives `giveni={A\bibinitperiod}`; the code reads `$node->getAttribute('initial')` where `$node` is the enclosing name.

**10. `relatedoptions` are always empty.** The code checks for an `options` attribute on `<bltx:item>`, but then reads `relatedoptions`: `<bltx:item type="reprint" ids="b" options="useprefix=true"/>` gives the clone `{skipbib=true,skipbiblist=true}`, while the BibTeX equivalent gives `{skipbib=true,skipbiblist=true,useprefix=true}` (and "Use of uninitialized value in split" on stderr).

**11. Citing an alias doesn't work** (cited-keys mode): `<bltx:ids><bltx:key>al</bltx:key></bltx:ids>` and `\cite{al}` give `WARN - I didn't find a database entry for 'al'`. The lookup searches for a `<bltx:id>` element, which doesn't exist; the branch then takes the `@id` of the *first entry in the file* and uses a syntactically invalid XPath (`entry/[@id=…]`) for the entry.

**12. In maps, `matchi`/`notmatchi` without a replacement are case-sensitive and `notmatchi` is not negated:** `imatch` is called without the case-insensitivity flag, and `$negmatch` is only set for `map_notmatch`.

**13. Mapping a field to a field that doesn't exist yet stores serialised XML.** `\step[fieldsource=title, fieldtarget=subtitle]` gives `\field{subtitle}{<bltx:title xmlns:bltx="http://biblatex-biber.sourceforge.net/biblatexml">Report</bltx:title>}`.

**14. Entries created by `entrynew` never get fields.** `map_entry_new` creates `XML::LibXML::Element->new("bltx:entry")`, a local name with a colon and no namespace; XPaths on it fail (`XPath error : Undefined namespace prefix` on stderr), and `exists()` treats the error (−1) as true, so `\step[entrytarget=newkey, fieldset=title, fieldvalue={New}]` does nothing and `newkey` is written without a title.

### Suggested fix

For 1, 2, 3, 6, 7, 9, 10, 11 and 12:

```diff
--- a/lib/Biber/Input/file/biblatexml.pm
+++ b/lib/Biber/Input/file/biblatexml.pm
@@ -220,12 +220,16 @@
     if ($logger->is_debug()) {# performance tune
       $logger->debug('Wanted keys: ' . join(', ', $keys->@*));
     }
+    # Keys are compared in Perl, not interpolated into XPath expressions, as they can
+    # contain quotes
+    my @allentries = $xpc->findnodes("/$NS:entries/$NS:entry");
     foreach my $wanted_key ($keys->@*) {
 
       if ($logger->is_debug()) {# performance tune
         $logger->debug("Looking for key '$wanted_key' in BibLaTeXML file '$filename'");
       }
-      if (my @entries = $xpc->findnodes("/$NS:entries/$NS:entry[\@id='$wanted_key']")) {
+      my $aliasentry;
+      if (my @entries = grep {$_->getAttribute('id') eq $wanted_key} @allentries) {
         # Check to see if there is more than one entry with this key and warn if so
         if ($#entries > 0) {
           biber_warn("Found more than one entry for key '$wanted_key' in '$filename': " .
@@ -250,8 +254,8 @@
         # found a key, remove it from the list of keys we want
         @rkeys = grep {$wanted_key ne $_} @rkeys;
       }
-      elsif ($xpc->findnodes("/$NS:entries/$NS:entry/$NS:id[text()='$wanted_key']")) {
-        my $key = $xpc->findnodes("/$NS:entries/$NS:entry/\@id");
+      elsif ($aliasentry = first {first {$_->textContent() eq $wanted_key} $_->findnodes("./$NS:ids/$NS:key")} @allentries) {
+        my $key = $aliasentry->getAttribute('id');
         if ($logger->is_debug()) {# performance tune
           $logger->debug("Citekey '$wanted_key' is an alias for citekey '$key'");
         }
@@ -260,12 +264,10 @@
         # Make sure there is a real, cited entry for the citekey alias
         # just in case only the alias is cited
         unless ($section->bibentries->entry_exists($key)) {
-          my $entry = $xpc->findnodes("/$NS:entries/$NS:entry/[\@id='$key']");
-
           # Record a key->datasource name mapping for error reporting
           $section->set_keytods($key, $filename);
 
-          create_entry($key, $entry, $filename, $smaps, \@rkeys);
+          create_entry($key, $aliasentry, $filename, $smaps, \@rkeys);
           $section->add_citekeys($key);
         }
 
@@ -626,6 +628,7 @@
             if ($mi = $step->{map_matchi} or $mi = $step->{map_notmatchi}) {
               $step->{map_match} = $mi;
               $caseinsensitive = 1;
+              $negmatch = 1 if $step->{map_notmatchi};
             }
 
             my $caseinsensitives = 0;
@@ -668,7 +671,7 @@
                   $logger->debug("Source mapping (type=$level, key=$etargetkey): Doing match/replace '$m' -> '$r' on field xpath '$xp_fieldsource_s'");
                 }
 
-                unless (_changenode($etarget, $xp_fieldsource_s, ireplace($last_fieldval, $m, $r, $caseinsensitive)), \$cnerror) {
+                unless (_changenode($etarget, $xp_fieldsource_s, ireplace($last_fieldval, $m, $r, $caseinsensitive), \$cnerror)) {
                   biber_warn("Source mapping (type=$level, key=$etargetkey): $cnerror");
                 }
               }
@@ -679,7 +682,7 @@
                 # Be aware that imatch() uses m//g so @imatches can have multiple paren group
                 # captures which might be useful
                 $m =~ s/(?<!\\)\$(\d)/$imatches[$1-1]/ge;
-                unless (@imatches = imatch($last_fieldval, $m, $negmatch)) {
+                unless (@imatches = imatch($last_fieldval, $m, $negmatch, $caseinsensitive)) {
                   # Skip the rest of the map if this step doesn't match and match is final
                   if ($step->{map_final}) {
                     if ($logger->is_debug()) {# performance tune
@@ -844,8 +847,12 @@
 
       # Now run any defined handler
       if ($dm->is_field(_norm($f))) {
-        my $handler = _get_handler($f);
-        $handler->($bibentry, $e, $f, $k);
+        if (my $handler = _get_handler($f)) {
+          $handler->($bibentry, $e, $f, $k);
+        }
+        else {
+          biber_warn("Field '" . _norm($f) . "' in entry '$k' can't be given in biblatexml, ignoring", $bibentry);
+        }
       }
     }
 
@@ -897,7 +904,7 @@
     }
     if (my $string = $item->getAttribute('options')) {
       $bibentry->set_datafield('relatedoptions',
-                               [ split(/$S/, $item->getAttribute('relatedoptions')) ]);
+                               [ split(/$S/, $item->getAttribute('options')) ]);
     }
   }
   return;
@@ -919,7 +926,7 @@
   }
 
   # eprint is special case
-  if (fc(_norm($f) eq "$NS:eprint")) {
+  if (fc(_norm($f)) eq 'eprint') {
     $bibentry->set_datafield('eprinttype', $node->getAttribute('type'));
     if (my $ec = $node->getAttribute('class')) {
       $bibentry->set_datafield('eprintclass', $ec);
@@ -1039,9 +1046,10 @@
 
       # Start of range
       # Using high-level range parsing sub in order to get unspec
-      if (my ($sdate, undef, undef, $unspec) = parse_date_range($bibentry,
+      my ($sdate, undef, undef, $unspec) = parse_date_range($bibentry,
                                                                 $datetype,
-                                                                $start->get_node(1)->textContent())) {
+                                                                $start->get_node(1)->textContent());
+      if ($sdate) { # parse_date_range returns a list, so test the date itself
 
         # Save julian
         $bibentry->set_field($datetype . 'datejulian', 1) if $CONFIG_DATE_PARSERS{start}->julian;
@@ -1140,9 +1148,10 @@
     }
     else { # Simple date
       # Using high-level range parsing sub in order to get unspec
-      if (my ($sdate, undef, undef, $unspec) = parse_date_range($bibentry,
+      my ($sdate, undef, undef, $unspec) = parse_date_range($bibentry,
                                                                 $datetype,
-                                                                $node->textContent())) {
+                                                                $node->textContent());
+      if ($sdate) { # parse_date_range returns a list, so test the date itself
 
         # Save julian
         $bibentry->set_field($datetype . 'datejulian', 1) if $CONFIG_DATE_PARSERS{start}->julian;
@@ -1304,7 +1313,7 @@
         if ($logger->is_debug()) {# performance tune
           $logger->debug("Found namepart '$n': $t");
         }
-        if (my $ni = $node->getAttribute('initial')) {
+        if (my $ni = $npnode->getAttribute('initial')) {
           $namec{"${n}-i"} = [$ni];
         }
         else {
```

With this change, item 1 gives `WARN - book entry 'a' (test.bltxml): Invalid format 'abc' of date field 'bltx:date' - ignoring`, item 2 gives `WARN - Field 'year' in entry 'a' can't be given in biblatexml, ignoring`, `\cite{o'brien}` and `\cite{al}` find their entries (`\keyalias{al}{a}`), and items 7, 9 and 10 give `eprinttype`/`eprintclass`, `giveni={X\bibinitperiod}` and the clone option. All tests in `t/` pass on `dev` (1686b9c) with this patch. Items 4, 5, 8, 13 and 14 need more thought than a one-line change: for 4, wrapping the parse and the map XPath evaluation in `eval` and calling `biber_error` with the file name would at least give a clean error.

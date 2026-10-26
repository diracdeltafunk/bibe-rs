<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-064, BB-089, BB-090, BB-096, BB-097, BB-102, BB-106, BB-139 (docs/biber-bugs.md). -->

**Title:** Small fixes: `langid={sr-Latn}` becomes `latin`, repeated transliteration warnings, and several latent slips

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biber 2.22 and `dev` (1686b9c). The first two items have a visible effect; the others were found by reading the code, and I could not construct an input where they change the output. I've collected them in one issue because each fix is a line or two.

**1. Serbian `langid`s are mapped to the wrong babel languages.** `%LOCALE_MAP_R` (Constants.pm), which turns BCP 47 `langid`s into babel/polyglossia names, has `'sr-Latn' => 'latin'` and `'sr-Cyrl' => 'serbian'`. So

```bibtex
@book{a, author={Petrović, Marko}, title={Naslov}, date={2000}, langid={sr-Latn}}
@book{b, author={Smith, J}, title={T}, date={2000}, langid={sr-Cyrl}}
```

gives `\field{langid}{latin}` and `\field{langid}{serbian}` in the `.bbl`. In babel, `serbian` is the Latin-script language and `serbianc` the Cyrillic one (the forward map `%LOCALE_MAP` already has `'serbian' => 'sr-Latn'` and `'serbianc' => 'sr-Cyrl'`), so these should be `serbian` and `serbianc`. The reverse map also has duplicate keys (`en-US` three times, `fr-CA` twice; only the last one takes effect), and the forward map has `'UKenglish' => 'en-UK'`, which is not a valid BCP 47 tag (`en-GB` is). The patch removes the dead duplicates and uses `en-GB`; `en-UK` stays accepted as input.

**2. Transliteration objects are rebuilt, and warnings repeated, for every string.** `call_transliterator` calls `get_transliterator` for each value, which constructs a new `Lingua::Translit` object and re-checks the pair. With a `<bcf:translit langids="german" target="title" from="german" to="ala-lc"/>` and 21 German entries, biber prints `WARN - Invalid transliteration from/to pair (german/ala-lc)` 21 times. Caching the transliterator (or `undef`) per pair gives one warning; the `.bbl` is unchanged. (In my test with 2000 Russian titles the construction cost itself was small, so this is mainly about the noise.)

**3. A computed global labelalpha index of 0 is ignored.** In `_process_label_attributes` (Internals.pm), `$lcache->{globalindices}{$field_string} || $lcache->{$field_string}{index}` falls back to the per-entry index when the global index computed for `substring_fixed_threshold` is 0, which is a valid index. It should be `//`.

**4. `uniquelist=minyear` counts every complete name list twice.** `create_uniquelist_info` calls `add_uniquelistcount_final($namelist)` and then, for `minyear`, `add_uniquelistcount_final($namelist, $labelyear)`, which increments the same `final` counter again (as well as `finalyear`). `DataList::set_uniquelist` treats a `final` count above 1 as "identical lists", so with `minyear` every list goes down that branch. In 60 random small examples (2–4 entries, 1–4 names, two years) the `ul=` values were the same with and without the fix, so this may be latent, but the count is plainly not what the comment describes.

**5. Per-name options keep a trailing comma when the name has no hash.** When a name has no `hash` in `namelistdata`, `instantiate_entry` (DataList.pm) removes `<BDS>…-PERNAMEHASH</BDS>,?`, but the hash is the *last* option, so the comma before it stays: `{{un=0,uniquepart=base,}{%`. Removing `,?<BDS>…</BDS>` instead fixes it.

**6. `.dot` output order depends on hash order.** In `Output::dot`, the entries-only branch iterates `sort values $v->%*`, which sorts hash references by their addresses. The edges are deduplicated, so the set of edges is the same, but their order (and so the file) can vary between runs.

**7. A typo in a legacy field name, and a grammar typo.** `bibtex.pm` sets `${datetype}endseaason` "for legacy" next to `endyeardivision`; no data model has that field and there is no corresponding start field, so the line is dead and can go. `LangTags.pm`'s grammar has `ALPHA: /[a-zA-z]/`, which also matches `[\]^_` and the backtick.

### Suggested fix

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -3896,12 +3896,8 @@
     # one count. We therefore need to distinguish counts which are of the final, complete
     # list of names. If there is more than one count for these, (meaning that there are
     # two or more identical name lists), we don't expand them at all as there is no point.
-    $dlist->add_uniquelistcount_final($namelist);
-
-    # uniquelist=minyear needs tracking only of namelists in same labelyear
-    if ($ul eq 'minyear') {
-      $dlist->add_uniquelistcount_final($namelist, $labelyear);
-    }
+    # uniquelist=minyear also needs tracking of namelists in the same labelyear
+    $dlist->add_uniquelistcount_final($namelist, $ul eq 'minyear' ? $labelyear : undef);
 
     # Add count for uniquelist=minyear
     unless (Compare($ulminyear_namelist, [])) {
--- a/lib/Biber/Constants.pm
+++ b/lib/Biber/Constants.pm
@@ -486,7 +486,7 @@
                    'turkmen'         => 'tk-TM',
                    'ukrainian'       => 'uk-UA',
                    'urdu'            => 'ur-IN',
-                   'UKenglish'       => 'en-UK',
+                   'UKenglish'       => 'en-GB',
                    'uppersorbian'    => 'hsb-DE',
                    'USenglish'       => 'en-US',
                    'usorbian'        => 'hsb-DE',
@@ -541,8 +541,6 @@
                      'en-UK'      => 'UKenglish',
                      'en-GB'      => 'british',
                      'en-NZ'      => 'newzealand',
-                     'en-US'      => 'USenglish',
-                     'en-US'      => 'american',
                      'en-US'      => 'english',
                      'eo'         => 'esperanto',
                      'eo-001'     => 'esperanto',
@@ -557,7 +555,6 @@
                      'fi'         => 'finnish',
                      'fi-FI'      => 'finnish',
                      'fr'         => 'french',
-                     'fr-CA'      => 'acadian',
                      'fr-CA'      => 'canadien',
                      'fr-FR'      => 'french',
                      'fur'        => 'friulan',
@@ -638,8 +635,8 @@
                      'sq'         => 'albanian',
                      'sq-AL'      => 'albanian',
                      'sr'         => 'serbian',
-                     'sr-Cyrl'    => 'serbian',
-                     'sr-Latn'    => 'latin',
+                     'sr-Cyrl'    => 'serbianc',
+                     'sr-Latn'    => 'serbian',
                      'sv'         => 'swedish',
                      'sv-SE'      => 'swedish',
                      'syc'        => 'syriac',
--- a/lib/Biber/DataList.pm
+++ b/lib/Biber/DataList.pm
@@ -1986,7 +1986,7 @@
           $entry_string =~ s|<BDS>$nid-PERNAMEHASH</BDS>|$str|gxms;
         }
         else {
-          $entry_string =~ s|<BDS>$nid-PERNAMEHASH</BDS>,?||gxms;
+          $entry_string =~ s|,?<BDS>$nid-PERNAMEHASH</BDS>||gxms; # the hash is the last option
         }
       }
     }
--- a/lib/Biber/Input/file/bibtex.pm
+++ b/lib/Biber/Input/file/bibtex.pm
@@ -1447,7 +1447,6 @@
           # Save end yeardivision date information
           if (my $yeardivision = $CONFIG_DATE_PARSERS{end}->yeardivision) {
             $bibentry->set_field($datetype . 'endyeardivision', $yeardivision);
-            $bibentry->set_field($datetype . 'endseaason', $yeardivision); # legacy
           }
 
           # must be an hour if there is a time but could be 00 so use defined()
--- a/lib/Biber/Internals.pm
+++ b/lib/Biber/Internals.pm
@@ -716,7 +716,7 @@
             $logger->debug("Using label disambiguation cache (name) for '$field' in section $secnum");
           }
           # Use the global index override if set (substring_width =~ /f/)
-          $field_string = ${$lcache->{$field_string}{data}}[$lcache->{globalindices}{$field_string} || $lcache->{$field_string}{index}];
+          $field_string = ${$lcache->{$field_string}{data}}[$lcache->{globalindices}{$field_string} // $lcache->{$field_string}{index}];
         }
         else {
           # This contains a mapping of strings to substrings of increasing lengths
@@ -782,7 +782,7 @@
           }
 
           # Use the global index override if set (substring_width =~ /f/)
-          $field_string = ${$lcache->{$field_string}{data}}[$lcache->{globalindices}{$field_string} || $lcache->{$field_string}{index}];
+          $field_string = ${$lcache->{$field_string}{data}}[$lcache->{globalindices}{$field_string} // $lcache->{$field_string}{index}];
           if ($logger->is_trace()) { # performance tune
             $logger->trace("Label disambiguation cache for '$field' " .
                            ($nameparts ? '(' . join(',', $nameparts->@*) . ') ' : '') .
--- a/lib/Biber/LangTags.pm
+++ b/lib/Biber/LangTags.pm
@@ -38,7 +38,7 @@
 
 alphanum: ALPHA|DIGIT
 
-ALPHA: /[a-zA-z]/
+ALPHA: /[a-zA-Z]/
 
 DIGIT: /[0-9]/
 
--- a/lib/Biber/Output/dot.pm
+++ b/lib/Biber/Output/dot.pm
@@ -342,7 +342,7 @@
     else {
       foreach my $f_entry (sort keys $gr->%*) {
         my $v = $gr->{$f_entry};
-        foreach my $w (sort values $v->%*) {
+        foreach my $w (map {$v->{$_}} sort keys $v->%*) {
           foreach my $t_entry (sort keys $w->%*) {
             next unless $state->{$secnum}{"${secnum}/${f_entry}"};
             next unless $state->{$secnum}{"${secnum}/${t_entry}"};
--- a/lib/Biber/Utils.pm
+++ b/lib/Biber/Utils.pm
@@ -1812,8 +1812,16 @@
 
 =cut
 
+my %TRANSLITERATORS; # cache, so that each pair is set up (and warned about) only once
+
 sub get_transliterator {
   my ($target, $from, $to) = map {lc} @_;
+  return $TRANSLITERATORS{"$from\x{10FFFD}$to"} if exists($TRANSLITERATORS{"$from\x{10FFFD}$to"});
+  return $TRANSLITERATORS{"$from\x{10FFFD}$to"} = _get_transliterator($target, $from, $to);
+}
+
+sub _get_transliterator {
+  my ($target, $from, $to) = @_;
   my @valid_from = ('iast', 'russian');
   my @valid_to   = ('devanagari', 'ala-lc', 'bgn/pcgn-standard');
   unless (first {$from eq $_} @valid_from and
```

All tests in `t/` pass on `dev` (1686b9c) with this patch.

<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-030, BB-031, BB-032, BB-033, BB-034, BB-134 (docs/biber-bugs.md). -->

**Title:** Sourcemap (bibtex input) problems: `fieldsource` with `entrytarget` reads the wrong entry, `entrykeystarnocited` matches cited entries, failed `replace` strings silently insert the regex, `foreach` on a missing field loops over its name, and `$MAPUNIQ` only works inside `foreach`

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

All of these are in `Biber::Input::file::bibtex::create_entry` and the helpers in `Utils.pm`, and were checked with biber 2.22 and current `dev` (1686b9c). The examples use `\nocite{*}` unless stated otherwise.

**1. `fieldsource`/`fieldtarget` with `entrytarget` reads the value from the current entry.**

```latex
\DeclareSourcemap{\maps[datatype=bibtex]{\map{
  \step[entrynew=newk, entrynewtype=misc, entrynocite]
  \step[entrytarget=newk, fieldset=title, fieldvalue=NEWTITLE]
  \step[entrytarget=newk, fieldsource=title, fieldtarget=note]
}}}
```

```bibtex
@misc{s1, title = {ORIGTITLE}}
```

The last step checks that `newk` has a `title` and deletes it from `newk`, but the value it writes into `newk`'s `note` is `s1`'s title: the `.bbl` has `\field{note}{ORIGTITLE}` for `newk`, instead of `NEWTITLE`. The value is read with `$entry->get(…)` where everything around it uses `$etarget`.

**2. `entrykeystarnocited` applies to every entry when there is no `\nocite{*}`.** With

```latex
\DeclareSourcemap{\maps[datatype=bibtex]{
  \map{\step[starnocited=true, fieldset=usera, fieldvalue={STAR}]}
}}
...
\cite{a}
```

`a` gets `usera = STAR`, although it was `\cite`d and there is no `\nocite{*}` at all. The test only skips the step when `\nocite{*}` is present *and* the key is cited; it should skip it unless `\nocite{*}` is present and the key is *not* cited.

**3. Captures in later steps (`$1`, `$2`, …, `$0`) don't mean what the manual suggests.** `imatch` matches with `m//g` in list context, so the saved captures are those of *all* matches, one after another; `$N` is then looked up as `$imatches[N-1]`. Consequences, with `location = {London}`:

```latex
\map{
  \step[fieldsource=location, match=\regexp{(o)}]
  \step[fieldset=addendum, fieldvalue={[$1|$2|$3|$0]}]
}
```

gives `addendum = [o|o||o]`: `$2` is the first group of the *second* match, and `$0` is `$imatches[-1]`, the last capture. After a `notmatch` step, `$1` is the string `1` (the dummy list `imatch` returns for a successful non-match). Captures are also interpolated into the patterns of later `match` steps without `\Q…\E`, so a captured `.` or `(` changes the meaning of the next pattern, and the digit rule differs between places (`\$(\d+)` for match patterns, `\$(\d)` for field values and new keys, so `$10` is `$1` followed by `0` in one and capture 10 in the other). I haven't included a patch for this, as it's a question of what the intended semantics are; returning the captures of the first match only, with `$0` as the whole match, would be the least surprising.

**4. A `replace` string that can't be evaluated silently replaces the match with the stringified regex.** `ireplace` evaluates the replacement as a Perl double-quoted string in a `Safe` compartment, and on any error substitutes `$val_match`, i.e. the `qr//` object:

```latex
\map{\step[fieldsource=title, match=\regexp{(?<rest>big)}, replace={[$+{rest}]}]}
\map{\step[fieldsource=publisher, match=\regexp{(a)(b)?}, replace={$1"}]}
```

```bibtex
@misc{s1, title={A big one}, publisher={ab}}
```

gives `\field{title}{A (?^u:(?<rest>big)) one}` and a publisher `(?^u:(a)(b)?)`, with no warning. Named captures always fail (reading `%+` needs `require Tie::Hash::NamedCapture`, which the compartment forbids), and so does any replacement containing a `"`. The evaluation also means that a replacement such as `@{[ uc $1 ]}` runs Perl code (inside the compartment), which the manual doesn't mention.

**5. `foreach` on a field the entry doesn't have loops over the field's name.** If `map_foreach` names a data-model field that the entry lacks (or whose value is `0`), the value falls through to the "explicit CSV" branch:

```latex
\DeclareSourcemap{\maps[datatype=bibtex]{
  \map[overwrite, foreach=keywords]{\step[fieldset=note, fieldvalue={kw=$MAPLOOP}, append]}
}}
```

```bibtex
@misc{a, title={T}, keywords={k1,k2}}
@misc{b, title={U}}
@misc{c, title={V}, keywords={0}}
```

gives `note = kw=k1kw=k2` for `a`, but `note = kw=keywords` for both `b` and `c`.

**6. `$MAPUNIQ` and `$MAPUNIQVAL` are only substituted inside `foreach`.** `maploopreplace` returns at once when there is no loop value, so

```latex
\map{\step[fieldset=userb, fieldvalue={id-$MAPUNIQ}]}
```

writes the literal `id-$MAPUNIQ`. The same early return means that a loop value of `0` is never substituted for `$MAPLOOP`. The manual introduces `$MAPUNIQ` in the paragraph about `map_foreach`, but its purpose (keys for new entries, as in biblatex's `95-customlists.tex`) doesn't depend on a loop.

### Suggested fix

For 1, 2, 4, 5 and 6:

```diff
--- a/lib/Biber/Input/file/bibtex.pm
+++ b/lib/Biber/Input/file/bibtex.pm
@@ -416,15 +416,16 @@
         my @imatches; # For persisting parenthetical matches over several steps
 
         # Set up any mapping foreach loop
-        my @maploop = ('');
+        my @maploop = (undef);
         if (my $foreach = $map->{map_foreach}) {
           if (my $dslist = $DATAFIELD_SETS{fc($foreach)}) { # datafield set list
             @maploop = $dslist->@*;
           }
           # casefold here as the field name does not come from Text::BibTeX so it might not be
           # valid in the case found in the mapping
-          elsif (my $felist = $entry->get(encode('UTF-8', NFC(fc($foreach))))) { # datafield
-            @maploop = split(/\s*,\s*/, $felist);
+          elsif ($dm->is_field(fc($foreach))) { # datafield, no loop if the entry doesn't have it
+            my $felist = $entry->get(encode('UTF-8', NFC(fc($foreach))));
+            @maploop = defined($felist) ? split(/\s*,\s*/, $felist) : ();
           }
           else { # explicit CSV
             @maploop = split(/\s*,\s*/, $foreach);
@@ -681,17 +682,18 @@
 
             # \nocite{*}
             if ($step->{map_entrykey_starnocited}) {
-              if ($section->is_allkeys_nocite and ($section->is_cite($key) or $section->is_nocite($key))) {  # check if NOT nocited
+              # Only entries which are in the section through \nocite{*} alone
+              if (not $section->is_allkeys_nocite or $section->is_cite($key) or $section->is_nocite($key)) {
                 if ($step->{map_final}) {
                   if ($logger->is_debug()) { # performance tune
-                    $logger->debug("Source mapping (type=$level, key=$key): Key is \\nocite{*}'ed but also either \\cite'd or explicitly \\nocited and step has 'final' set, skipping rest of map ...");
+                    $logger->debug("Source mapping (type=$level, key=$key): Key is not only \\nocite{*}'ed and step has 'final' set, skipping rest of map ...");
                   }
                   next MAP;
                 }
                 else {
                   # just ignore this step
                   if ($logger->is_debug()) { # performance tune
-                    $logger->debug("Source mapping (type=$level, key=$key): Key is \\nocite{*}'ed but also either \\cite'd or explicitly \\nocited, skipping step ...");
+                    $logger->debug("Source mapping (type=$level, key=$key): Key is not only \\nocite{*}'ed, skipping step ...");
                   }
                   next;
                 }
@@ -841,7 +843,7 @@
                 # $target and $fieldsource are already casefolded, which is correct as it
                 # does not come from Text::BibTeX's list of fields
                 $etarget->set(encode('UTF-8', NFC($target)),
-                              encode('UTF-8', NFC($entry->get(encode('UTF-8', NFC($fieldsource))))));
+                              encode('UTF-8', NFC($etarget->get(encode('UTF-8', NFC($fieldsource))))));
                 $etarget->delete($fieldsource);
               }
             }
--- a/lib/Biber/Utils.pm
+++ b/lib/Biber/Utils.pm
@@ -1180,8 +1180,15 @@
     my $cpt = Safe->new;
     $cpt->permit_only(qw(:default));
     $value =~ s{$val_match}{
+        my $orig = $&;
         my $r = $cpt->reval(qq{"$val_replace"});
-        $@ ? $val_match : $r;
+        if ($@) {
+          biber_warn("Cannot use sourcemap replacement '$val_replace' for '$orig', leaving it unchanged: $@");
+          $orig;
+        }
+        else {
+          $r;
+        }
       }egxms;
     return $value;
 }
@@ -1777,8 +1784,8 @@
   no strict 'vars';
   my ($string, $maploop) = @_;
   return undef unless defined($string);
-  return $string unless $maploop;
-  $string =~ s/\$MAPLOOP/$maploop/g;
+  # $MAPLOOP only has a value inside map_foreach, $MAPUNIQ can be used anywhere
+  $string =~ s/\$MAPLOOP/$maploop/g if defined($maploop);
   $string =~ s/\$MAPUNIQVAL/$MAPUNIQVAL/g;
   if ($string =~ m/\$MAPUNIQ/) {
     my $MAPUNIQ = suniqid;
```

With this change: `newk` gets `note = NEWTITLE`; the `starnocited` step no longer applies to `\cite{a}` (and still applies to keys that come only from `\nocite{*}`); the two failing replacements leave the values unchanged and give `WARN - Cannot use sourcemap replacement '[$+{rest}]' for 'big', leaving it unchanged: 'require' trapped by operation mask …` and `… Can't find string terminator '"' …`; `b` gets no `note` and `c` gets `kw=0`; and `$MAPUNIQ` gives a unique id outside `foreach` too. All tests in `t/` pass on `dev` (1686b9c) with this patch.

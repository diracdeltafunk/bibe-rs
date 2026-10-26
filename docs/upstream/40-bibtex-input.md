<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-077, BB-078, BB-142, BB-140 (docs/biber-bugs.md). -->

**Title:** BibTeX input: `--input-encoding=ascii` no longer reads UTF-8 leniently, misleading "not an integer" warning for `month={February}`, dates mixing ASCII and other digits lose the ASCII digits

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Three small problems in `Input/file/bibtex.pm` and `Date/Format.pm` (biber 2.22 and `dev` at 1686b9c), and one line that doesn't do what its comment says.

**1. The ascii → UTF-8 leniency is dead code.** `preprocess_file` has

```perl
  if ($benc eq 'ascii') {
    $logger->info("Reading ascii input as UTF-8");
    $benc = 'UTF-8';
  }
```

but `extract_entries` first slurps the file in the given encoding to check that it contains an `@`, and that already fails. With `@book{a, author={Doe, J}, title={Café}, date={2000}}` (UTF-8) and `--input-encoding=ascii` (what biblatex passes with `bibencoding=ascii`), biber stops with

```
ERROR - Data file 'test.bib' cannot be read in encoding 'ascii': ascii "\xC3" does not map to Unicode at .../File/Slurper.pm line 59.
```

The patch makes the first check use UTF-8 for ascii too, so the file is read as `preprocess_file` intends.

**2. `month={February}` gives a misleading warning.** For

```bibtex
@book{c, author={Roe, R}, title={U}, year={2000}, month={February}}
@book{e, author={Roe, R}, title={W}, year={2000}, month={Feb}}
```

biber warns `legacy month field 'February' in entry 'c' is not an integer - this will probably not sort properly.` (and the same for `Feb`), but `_hack_month` then converts both to `2`, and the `.bbl` has `\field{month}{2}`. The patch checks the converted value.

**3. A date that mixes ASCII digits with digits of another script loses the ASCII digits.** `Date::Format::_pre` builds a map from non-ASCII digit runs to ASCII ones (`stoa`), but then replaces *every* digit run through the map: `s/(\d+)/$p{self}{scriptmap}{stoa}{$1}/xge`. ASCII runs aren't in the map, so they are replaced by the empty string. `date={2020-٠٣}` (Arabic-Indic month) becomes `-03` and is rejected (`Invalid format '2020-٠٣' of date field 'date' - ignoring`), while `date={٢٠٢٠-٠٣}` works. The patch keeps runs that aren't in the map.

**4. (No visible effect.)** `parsename` has `$namestr =~ s/\A\s*|\s*\z//xms; # leading and trailing whitespace`, which without `/g` removes only the leading whitespace. I couldn't find a visible effect, because Text::BibTeX trims the name parts anyway, but the patch adds the `/g`.

### Suggested fix

```diff
--- a/lib/Biber/Date/Format.pm
+++ b/lib/Biber/Date/Format.pm
@@ -142,7 +142,8 @@
     }
   }
   if (defined($p{self}{scriptmap})) {
-    $p{input} =~ s/(\d+)/$p{self}{scriptmap}{stoa}{$1}/xge;
+    # ASCII digit runs aren't in the map and are kept as they are
+    $p{input} =~ s/(\d+)/$p{self}{scriptmap}{stoa}{$1} \/\/ $1/xge;
   }
 
   # ISO 8601-2:2016 4.2.1 (uncertain)
--- a/lib/Biber/Input/file/bibtex.pm
+++ b/lib/Biber/Input/file/bibtex.pm
@@ -123,9 +123,11 @@
   }
 
   # Check for files with no macros - they also confuse btparse
+  # ascii input is read as UTF-8 (as in preprocess_file), so check it as UTF-8 here too
   my $tbuf;
-  unless (eval {$tbuf = slurp_switchr($filename, $encoding)->$*}) {
-    biber_error("Data file '$filename' cannot be read in encoding '$encoding': $@");
+  my $cenc = $encoding eq 'ascii' ? 'UTF-8' : $encoding;
+  unless (eval {$tbuf = slurp_switchr($filename, $cenc)->$*}) {
+    biber_error("Data file '$filename' cannot be read in encoding '$cenc': $@");
   }
   unless ($tbuf =~ m/\@/) {
     biber_warn("Data source '$filename' contains no BibTeX entries/macros, ignoring");
@@ -1079,7 +1081,8 @@
   }
   if ($fc eq 'month') {
     return if $bibentry->get_datafield('month');
-    if ($value and not looks_like_number(num($value))) {
+    # Month names and abbreviations are converted below
+    if ($value and not looks_like_number(num(_hack_month($value)))) {
       biber_warn("legacy month field '$value' in entry '$key' is not an integer - this will probably not sort properly.");
     }
   }
@@ -1819,7 +1822,7 @@
   my ($section, $namestr, $fieldname) = @_;
 
   # First sanitise the namestring due to Text::BibTeX::Name limitations on whitespace
-  $namestr =~ s/\A\s*|\s*\z//xms; # leading and trailing whitespace
+  $namestr =~ s/\A\s*|\s*\z//gxms; # leading and trailing whitespace
   # Collapse internal whitespace and escaped spaces like in "Christina A. L.\ Thiele"
   $namestr =~ s/\s+|\\\s/ /g;
   $namestr =~ s/\A\{\{+([^\{\}]+)\}+\}\z/{$1}/xms; # Allow only one enveloping set of braces
```

All tests in `t/` pass on `dev` (1686b9c) with this patch.

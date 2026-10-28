<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-010, BB-045 and BB-120 (docs/biber-bugs.md). -->

**Title:** `--collate-options`: the old `"key => value"` syntax (still in the manual) is silently ignored, `ignoreChar` and other creation-only options crash, and invalid values die without an ERROR

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

**1. The syntax removed in 2.21 is still documented, and silently ignored (BB-045).** Since 2.21, `-c` takes `key=value`, once per option (Changes: *"the 'collate-options' option no longer accepts a single string in perl hash notation"*). The manual (§ "Sorting") still says *"Options are passed using the `--collate-options|-c` option as a single quoted string, each option separated by comma, each key and value separated by `=>`"*, with the examples `--collate-options="backwards => 2"`, `--collate-options="variable => 'shifted'"` and `-c 'locale => "de_DE"'`.

Getopt::Long splits such a value at the first `=`, so `variable => 'shifted'` becomes the key `variable ` with the value `> 'shifted'`. The debug log shows

```
Sorting with Unicode::Collate (normalization => prenormalized, variable  => > 'shifted', level => 3, variable => non-ignorable, …
```

and the option has no effect, without any message. Worse, for sort fields with their own `locale`, the bogus value makes such lists come out **unsorted**.

**2. Creation-only options crash (BB-010).** `Biber::UCollate::new` creates the collator with only the locale and applies every other option with `change()`. Unicode::Collate refuses some options in `change()` (`@Unicode::Collate::ChangeNG`: `ignoreChar`, `ignoreName`, `undefChar`, `undefName`, `entry`, `rewrite`, `suppress`, …):

```
$ biber -c ignoreChar=- test
change of ignoreChar via change() is not allowed! at .../Biber.pm line 4311.
```

**3. Invalid values die without an ERROR (BB-120).** `-c level=5`, `-c level=0` and `-c variable=bogus` make Unicode::Collate croak, and biber dies with the raw Perl message (exit code 255 from source, 25 for the TeX Live binary; with the TeX Live 2.22 binary on macOS, `level=5` even ends in "Out of memory in perl:util:safesysmalloc"). Nothing is written to the `.blg`, and the `.bbl` is empty.

Same behaviour on biber 2.22 and current `dev` (1686b9c).

### Minimal example

```bibtex
@book{a, author = {Zed, A}, title = {T}}
@book{b, author = {Abe, B}, title = {U}}
```

with any document that uses `\nocite{*}`, and `biber -c ignoreChar=- test`, `biber -c level=5 test` or `biber -c "variable => 'shifted'" -D test`.

### Suggested fix

Reject option names that can't be valid (which catches the old syntax), pass creation-only options to `new()`, turn collator errors into biber errors, and update the manual:

```diff
--- a/bin/biber
+++ b/bin/biber
@@ -139,6 +139,15 @@
           ) or pod2usage(-verbose => 0,
                          -exitval => EXIT_ERROR);
 
+# Since biber 2.21, collation options are given as --collate-options key=value, once
+# per option. Reject the old "key => value" syntax instead of silently ignoring it.
+foreach my $k (keys %collateoptions) {
+  unless ($k =~ /\A\w+\z/) {
+    say STDERR "Biber: Invalid collation option '$k=$collateoptions{$k}': use --collate-options key=value, once for each option";
+    exit EXIT_ERROR;
+  }
+}
+
 # verbose > 1 uses external perldoc, this doesn't work with PAR::Packer binaries
 # so use "-noperldoc" to use built-in POD::Text
 if (exists $opts->{'help'}) {
--- a/doc/biber.tex
+++ b/doc/biber.tex
@@ -1921,7 +1921,7 @@
 determined by the following resource chain which is given
 in decreasing precedence order:\\[2ex]
 
-\noindent\opt{--collate-options|-c} (e.g. \opt{-c 'locale => "de\_DE"'}) $\rightarrow$\\
+\noindent\opt{--collate-options|-c} (e.g. \opt{-c locale=de\_DE}) $\rightarrow$\\
 \hspace*{1em}\opt{--sortlocale|-l} $\rightarrow$\\
 \hspace*{2em}\biblatex per-sortset \opt{locale} option $\rightarrow$\\
 \hspace*{3em}\biblatex per-sortscheme \opt{locale} option$\rightarrow$\\
@@ -1972,9 +1972,9 @@
 options, most of which the vast majority of users will never
 need\footnote{For details on the various options, see
   \url{http://search.cpan.org/search?query=Unicode\%3A\%3ACollate&mode=all}}.
-Options are passed using the \opt{--collate-options|-c} option as a
-single quoted string, each option separated by comma, each key and
-value separated by «\verb+=>+». See examples.
+Options are passed using the \opt{--collate-options|-c} option, once
+for each option, with the key and value separated by «\verb+=+». See
+examples.
 
 \textbf{Note:} \biber\ sets the Unicode collation option
 «variable» to «non-ignorable». Effectively, this means that punctuation is
@@ -2003,12 +2003,12 @@
 
 \noindent Case insensitive sorting.
 
-\biberex{biber --sortupper=false --collate-options="backwards => 2"}
+\biberex{biber --sortupper=false --collate-options backwards=2}
 
 \noindent Collate lowercase before upper and collate French accents in
 reverse order at UCA level 2.
 
-\biberex{biber --collate-options="variable => 'shifted'"}
+\biberex{biber --collate-options variable=shifted}
 
 \noindent Use the UCA default setting for variable weight punctuation
 (which is to ignore it for sorting, effectively).
--- a/lib/Biber/UCollate.pm
+++ b/lib/Biber/UCollate.pm
@@ -42,16 +42,19 @@
   # Remove locale from options as we need this to make the object
   my $coll_locale = delete $collopts{locale};
 
-  # Now create the collator object
-  my $Collator = $class->SUPER::new(locale => $coll_locale)
-    or $logger->logcarp("Problem creating Unicode::Collate::Locale object: $@");
-
   # Fix the old "alternate" alias otherwise we have problems as U::C->change() always
   # returns the new "variable" option and we get confused.
   if (my $alt = delete $collopts{alternate}) {
     $collopts{variable} = $alt;
   }
 
+  # Some options can only be set when creating the object
+  my %newopts = map {exists($collopts{$_}) ? ($_ => delete $collopts{$_}) : ()} @Unicode::Collate::ChangeNG;
+
+  # Now create the collator object
+  my $Collator = eval {$class->SUPER::new(locale => $coll_locale, %newopts)}
+    or Biber::Utils::biber_error("Problem creating collation object: " . _collerr($@));
+
   # Show the collation options when debugging
   if ($logger->is_debug()) {# performance tune
     $logger->debug('Collation options: ' . Data::Dump::pp(%collopts));
@@ -60,7 +63,10 @@
   # Tailor the collation object and report differences from defaults for locale
   # Have to do this in ->change method as ->new can croak with conflicting tailoring
   # for locales which enforce certain tailorings
-  my %coll_changed = $Collator->change( %collopts );
+  my %coll_changed = eval {$Collator->change( %collopts )};
+  if ($@) {
+    Biber::Utils::biber_error("Invalid collation options: " . _collerr($@));
+  }
   while (my ($k, $v) = each %coll_changed) {
     # If we are changing something that has no override tailoring in the locale, it
     # is undef in this hash and we don't care about such things
@@ -73,6 +79,13 @@
   return $Collator;
 }
 
+# Error message from Unicode::Collate without the location
+sub _collerr {
+  my $err = shift;
+  $err =~ s/\s+at\s+\S+\s+line\s+\d+\.?\s*\z//;
+  return $err;
+}
+
 1;
 
 __END__
```

With this change:

* `-c "variable => 'shifted'"` gives `Biber: Invalid collation option 'variable => 'shifted'': use --collate-options key=value, once for each option` and exit code 2 (`-c variable=shifted` works as before);
* `-c ignoreChar=-` runs and sorts normally;
* `-c level=5` gives `ERROR - Invalid collation options: Unsupported level 5 (in value for key 'level') higher than 4.` and exit code 2; `-c variable=bogus` gives `ERROR - Invalid collation options: Unicode::Collate unknown variable parameter name: bogus`.

All tests in `t/` pass on `dev` (1686b9c) with this patch.

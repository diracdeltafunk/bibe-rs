<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-008 and BB-159 (docs/biber-bugs.md). -->

**Title:** `--output-fieldcase=Upper` (any non-lowercase spelling) and an invalid `output_fieldcase` in `biber.conf` crash biber

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

`bin/biber` validates `--output-fieldcase` case-insensitively (`/^(?:upper|lower|title)$/i`), but `Output/bibtex.pm` compares it case-sensitively (`eq 'upper'`, etc.) in four places. With any other spelling no casing function is set, and biber dies:

```
$ biber --tool --output-fieldcase=Upper test.bib
Can't use an undefined value as a subroutine reference at .../Biber/Output/bibtex.pm line 145.
```

(exit code 25 for the TeX Live 2.22 binary, 255 from source; no output file.)

The validation also only looks at the command line, so a bad value in `biber.conf` reaches the same line:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<config>
  <output_fieldcase>bogus</output_fieldcase>
</config>
```

`biber --tool --configfile=b.conf test.bib` dies with the same message. Same on biber 2.22 and current `dev` (1686b9c). Both cases apply to tool mode and to `--output-format=bibtex`.

### Minimal example

```bibtex
@book{a, author = {Doe, J}, title = {T}, date = {2000}}
```

```
biber --tool --output-fieldcase=Title test.bib
```

### Suggested fix

Check the option after the config file has been read, and normalise it to lower case:

```diff
--- a/bin/biber
+++ b/bin/biber
@@ -292,10 +292,6 @@
   say STDERR "Biber: Invalid non-numeric argument for 'output_indent' option!";
   exit EXIT_ERROR;
 }
-if (exists($opts->{output_fieldcase}) and $opts->{output_fieldcase} !~ /^(?:upper|lower|title)$/i) {
-  say STDERR "Biber: Invalid argument for 'output_fieldcase' option - must be one of 'upper', 'lower' or 'title'!";
-  exit EXIT_ERROR;
-}
 
 # Check the dot_include option
 if (exists($opts->{dot_include}) and (not exists($opts->{output_format})
@@ -332,6 +328,15 @@
 # Create Biber object, passing command-line options
 my $biber = Biber->new($opts->%*);
 
+# Check output_fieldcase only now, as it can also be set in the config file, and
+# normalise its case as the output code compares it case-sensitively
+my $fieldcase = Biber::Config->getoption('output_fieldcase') // '';
+unless ($fieldcase =~ /^(?:upper|lower|title)$/i) {
+  say STDERR "Biber: Invalid argument for 'output_fieldcase' option - must be one of 'upper', 'lower' or 'title'!";
+  exit EXIT_ERROR;
+}
+Biber::Config->setoption('output_fieldcase', lc($fieldcase));
+
 # get the logger object
 my $logger  = Log::Log4perl::get_logger('main');
 my $screen  = Log::Log4perl::get_logger('screen');
```

With this change, `--output-fieldcase=Title` writes `@Book{a,` / `Author = {Doe, J},`, and the `biber.conf` value `bogus` gives the existing "Invalid argument for 'output_fieldcase' option" message with exit code 2. All tests in `t/` pass on `dev` (1686b9c) with this patch.

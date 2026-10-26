<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-087, BB-088, BB-099, BB-194 (docs/biber-bugs.md). -->

**Title:** Manual and config schema out of date: Windows config path, malformed sample `biber.conf`, options missing from `config.rnc`, `mincrossrefs` description; `--output-format=bibjson` is accepted but the module is gone

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked against `dev` (1686b9c).

**1. Windows config file location.** The manual (section on the config file) lists `$APPDATA/biber.conf`, but `Biber::Config::config_file` looks for `%APPDATA%\biber\biber.conf` (`catfile($ENV{APPDATA}, "biber", $BIBER_CONF_NAME)`).

**2. The sample `biber.conf` in the manual is not well-formed XML.** It contains `<ouput_align>0</output_align>`, so `xmllint` stops with "Opening and ending tag mismatch: ouput_align … and output_align", and biber can't read the file if a user copies it. After fixing that tag, `--validate-config` still rejects it: it has `<graph>0</graph>` (replaced by `--output-format=dot` long ago, and not in the schema), and `<sortcase>true</sortcase>`, `<sortupper>true</sortupper>`, `<tool>false</tool>`, `<trace>false</trace>` where the schema only allows `0`/`1`.

**3. `config.rnc` lacks many options that biber accepts in a config file.** Comparing the keys of `$CONFIG_DEFAULT_BIBER` with the elements of `config.rnc`, these options are missing (besides the obsolete `graph`): `glob_datasources`, `input_format`, `isbn10`, `isbn13`, `isbn_normalise`, `minxrefs`, `named_annotation_marker`, `no_bblxml_schema`, `no_bltxml_schema`, `no_default_datamodel`, `noskipduplicates`, `nouri_encode`, `output_field_order`, `output_named_annotation_marker`, `remove_tmp_dir`, `sortdebug`, `strip_comments`, `tool_noremove_missing_dependants`, `validate_bblxml`, `validate_bltxml`, `winunicode` and `xsvsep`. So does `nolabelwidthcount`, although the manual documents it as a config-file-only option, with this example:

```xml
<config>
  <nolabelwidthcount>
    <option value="OR\s+" />
  </nolabelwidthcount>
</config>
```

`biber --configfile=nl.conf --validate-config` rejects that file with `ERROR - 'nl.conf' failed to validate against schema '…/config.rng'`. The `output_format` choice also lacks `bblxml`.

**4. `mincrossrefs` in the manual.** The description of the processing steps says that entries are promoted if "CROSSREF'ed or XREF'ed at least `mincrosref` times". The option name is misspelt, and the code counts the two separately: crossrefs against `mincrossrefs` and xrefs against `minxrefs` (`Biber.pm`, the two loops after "at least mincrossrefs times are included in the bibliography").

**5. `bibjson` output.** `bin/biber` accepts `--output-format=bibjson` (and lists it in `--help`), but `Biber::Output::bibjson` was removed in "Removed some JSON things" (0a0460ec). `biber --tool --output-format=bibjson file.bib` fails with

```
ERROR - Error loading data source package 'Biber::Output::bibjson': Can't locate Biber/Output/bibjson.pm in @INC …
```

The patch removes `bibjson` from the accepted formats, so this gives the normal "Output format in tool mode must be one of …" message. (`Name.pm` still has `name_to_bibjson`/`name_part_to_bibjson`, which are now unused; I left them alone.)

### Suggested fix

```diff
--- a/bin/biber
+++ b/bin/biber
@@ -252,8 +252,8 @@
 
 # Check the output_format option
 if (my $of = $opts->{output_format}) {
-  unless ($opts->{output_format} =~ /\A(?:bbl|dot|bibtex|biblatexml|bblxml|bibjson)\z/xms) {
-    say STDERR "Biber: Unknown output format '$of', must be one of 'bbl', 'dot', 'bibtex', 'biblatexml', 'bblxml', 'bibjson'";
+  unless ($opts->{output_format} =~ /\A(?:bbl|dot|bibtex|biblatexml|bblxml)\z/xms) {
+    say STDERR "Biber: Unknown output format '$of', must be one of 'bbl', 'dot', 'bibtex', 'biblatexml', 'bblxml'";
     exit EXIT_ERROR;
   }
   if ($opts->{output_format} eq 'bblxml' or $opts->{output_format} eq 'dot') {
@@ -275,8 +275,8 @@
 # Check output-format value
 if (exists($opts->{tool}) and
     exists($opts->{output_format}) and
-    $opts->{output_format} !~ /\A(?:bibtex|biblatexml|bibjson)\z/xms) {
-    say STDERR "Biber: Output format in tool mode must be one of 'bibtex', 'biblatexml' or 'bibjson'";
+    $opts->{output_format} !~ /\A(?:bibtex|biblatexml)\z/xms) {
+    say STDERR "Biber: Output format in tool mode must be one of 'bibtex' or 'biblatexml'";
     exit EXIT_ERROR;
 }
 
@@ -369,9 +369,6 @@
     elsif (Biber::Config->getoption('output_format') eq 'biblatexml') { # tool .blxtxml output
       $outfile = $ARGV[0] =~ s/(?:\.[^.\/\\]*)?\z/_bibertool.bltxml/r;
     }
-    elsif (Biber::Config->getoption('output_format') eq 'bibjson') { # bibjson output
-      $outfile = $ARGV[0] =~ s/(?:\.[^.\/\\]*)?\z/_bibertool.json/r;
-    }
   }
   else {
     if (Biber::Config->getoption('output_format') eq 'dot') { # .dot output
@@ -801,7 +798,7 @@
 set. F<file> can be '-' to output directly to STDOUT. The legacy option
 B<--outfile> is supported as an alias.
 
-=item B<--output-format=dot|bibtex|biblatexml|bbl|bblxml|bibjson>
+=item B<--output-format=dot|bibtex|biblatexml|bbl|bblxml>
 
 Biber output format. Default if not specified is of course, F<bbl>. Use F<dot>
 to output a GraphViz DOT file instead of F<.bbl>. This is a directed graph of
--- a/data/schemata/config.rnc
+++ b/data/schemata/config.rnc
@@ -76,6 +76,13 @@
         empty
       }+
     }? &
+    # nolabelwidthcount specification
+    element nolabelwidthcount {
+      element option {
+        attribute value { xsd:string },
+        empty
+      }+
+    }? &
     # nosort specification
     element nosort     {
       element option {
@@ -101,7 +108,7 @@
     element output_fieldcase     { "upper" | "lower" | "title" }? &
     element output_field_replace { text }? &
     element output_file          { text }? &
-    element output_format        { "bibtex" | "bbl" | "biblatexml" | "dot" }? &
+    element output_format        { "bibtex" | "bbl" | "biblatexml" | "bblxml" | "dot" }? &
     element output_indent        { xsd:string { pattern = "\d+t?" } }? &
     element output_legacy_dates  { "0" | "1" }? &
     element output_listsep       { text }? &
@@ -155,6 +162,29 @@
     element tool                 { "0" | "1" }? &
     element tool_config          { text }? &
     element trace                { "0" | "1" }? &
+    # options without a dedicated place above
+    element glob_datasources     { "0" | "1" }? &
+    element input_format         { text }? &
+    element isbn10               { "0" | "1" }? &
+    element isbn13               { "0" | "1" }? &
+    element isbn_normalise       { "0" | "1" }? &
+    element minxrefs             { xsd:unsignedByte }? &
+    element named_annotation_marker { text }? &
+    element no_bblxml_schema     { "0" | "1" }? &
+    element no_bltxml_schema     { "0" | "1" }? &
+    element no_default_datamodel { "0" | "1" }? &
+    element noskipduplicates     { "0" | "1" }? &
+    element nouri_encode         { "0" | "1" }? &
+    element output_field_order   { text }? &
+    element output_named_annotation_marker { text }? &
+    element remove_tmp_dir       { "0" | "1" }? &
+    element sortdebug            { "0" | "1" }? &
+    element strip_comments       { "0" | "1" }? &
+    element tool_noremove_missing_dependants { "0" | "1" }? &
+    element validate_bblxml      { "0" | "1" }? &
+    element winunicode           { "0" | "1" }? &
+    element xsvsep               { text }? &
+    element validate_bltxml      { "0" | "1" }? &
     element validate_config      { "0" | "1" }? &
     element validate_control     { "0" | "1" }? &
     element validate_datamodel   { "0" | "1" }? &
--- a/doc/biber.tex
+++ b/doc/biber.tex
@@ -457,8 +457,10 @@
   \begin{itemize}
   \item Entries instantiated by being members of a set.
   \item Entries instantiated by being mentioned as a \bibfield{CROSSREF} are
-    promoted to cited status if \bibfield{CROSSREF}'ed or \bibfield{XREF}'ed at
-    least \opt{mincrosref} times.
+    promoted to cited status if \bibfield{CROSSREF}'ed at least
+    \opt{mincrossrefs} times, and entries mentioned as an \bibfield{XREF} if
+    \bibfield{XREF}'ed at least \opt{minxrefs} times. The two are counted
+    separately.
   \item Clones of entries mentioned as a «related» entry of a cited entry.
   \end{itemize}
 \item Some of these auto-cited entries have the «dataonly» option set on
@@ -630,7 +632,7 @@
 \hspace*{1em}\verb+$HOME/.biber.conf+ $\rightarrow$\\
 \hspace*{2em}\verb+$XDG_CONFIG_HOME/biber/biber.conf+ $\rightarrow$\\
 \hspace*{3em}\verb+$HOME/Library/biber/biber.conf+ (Mac OSX only)\\
-\hspace*{3em}\verb+$APPDATA/biber.conf+ (Windows only) $\rightarrow$\\
+\hspace*{3em}\verb+$APPDATA/biber/biber.conf+ (Windows only) $\rightarrow$\\
 \hspace*{4em}the output of «\verb+kpsewhich biber.conf+» (if available on the
 system)\\[2ex]
 
@@ -650,7 +652,6 @@
   <decodecharsset>base</decodecharsset>
   <dieondatamodel>0</dieondatamodel>
   <dieonwarning>0</dieonwarning>
-  <graph>0</graph>
   <input_encoding>UTF-8</input_encoding>
   <listsep>and</listsep>
   <mincrossrefs>0</mincrossrefs>
@@ -676,7 +677,7 @@
   </nosort>
   <onlylog>0</onlylog>
   <others_string>others</others_string>
-  <ouput_align>0</output_align>
+  <output_align>0</output_align>
   <output_encoding>UTF-8</output_encoding>
   <output_fieldcase>upper</output_fieldcase>
   <output_format>bbl</output_format>
@@ -690,10 +691,10 @@
   <output_xdatasep>-</output_xdatasep>
   <output_xnamesep>=</output_xnamesep>
   <quiet>0</quiet>
-  <sortcase>true</sortcase>
-  <sortupper>true</sortupper>
-  <tool>false</tool>
-  <trace>false</trace>
+  <sortcase>1</sortcase>
+  <sortupper>1</sortupper>
+  <tool>0</tool>
+  <trace>0</trace>
   <validate_bltxml>0</validate_bltxml>
   <validate_config>0</validate_config>
   <validate_control>0</validate_control>
```

`config.rng` needs the same additions. I edited it by hand rather than regenerating it, since I didn't have `trang`; the change is mechanical (an `<optional><element name="…">` per option, with `<choice><value>0</value><value>1</value></choice>`, `<text/>` or `<data type="unsignedByte"/>`, plus `<value>bblxml</value>`), and I can attach it. With both files changed, the manual's sample `biber.conf`, the `nolabelwidthcount` example above and `data/biber-tool.conf` all validate with `--validate-config` and `xmllint --relaxng`. All tests in `t/` pass on `dev` (1686b9c) with this patch.

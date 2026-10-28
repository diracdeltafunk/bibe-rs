<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-066, BB-067, BB-068, BB-069, BB-071, BB-072, BB-073, BB-074 (docs/biber-bugs.md). -->

**Title:** Command-line handling: missing `--configfile` ignored, `--dieondatamodel` doesn't die, `-O -` creates a file called `-`, wrong `_biber.bib` name, `-e`/`--input-encoding` don't exist, and more

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

All of these were checked with biber 2.22 and `dev` (1686b9c), with a `test.bcf` for one entry.

1. **A missing `--configfile` is silently ignored.** `biber --configfile=nonexist.conf test.bcf` runs normally, with the default config file search instead (`Config.pm`, `unless (defined($opts->{configfile}) and -f $opts->{configfile}) { $opts->{configfile} = config_file() }`). A typo in the name thus goes unnoticed. The patch makes it an error (exit 2).
2. **`--dieondatamodel` never exits early.** With `--validate-datamodel --dieondatamodel`, every data-model problem is reported as an `ERROR`, but biber carries on to the end. `validate_datamodel` calls `$dmwe->($warning, $be)` with `$dmwe = \&biber_error`, and `biber_error`'s second argument is `$nodie`, so passing the entry object means "don't die".
3. **The default output name for `--output-format=bibtex` is computed from the first dot in the path:** `$bcf =~ s/\..+$/_biber.bib/r`. `biber --output-format=bibtex ./test.bcf` writes `_biber.bib`, and `sub.dir/test` writes `sub_biber.bib`. The patch uses the same pattern as the tool-mode names.
4. **`-O -` (write to standard output) creates a file called `-`.** `Output::base::set_output_target_file` opens the name with `IO::File->new($file, '>…')` before the output methods check for `-` and use `STDOUT`, so `biber -O - test.bcf` writes the `.bbl` to stdout and leaves an empty file `-` in the current directory. With `--output-directory=dir`, the name becomes `dir/-` and the `.bbl` is written to that file instead of stdout.
5. **The specific "Cannot find control file … Did latex run successfully on your .tex file before you ran biber?" message is unreachable.** `parse_ctrlfile` calls `locate_data_file`, which already dies with the generic `Cannot find 'test.bcf'!`. With `--nodieonerror`, biber then continues without a control file and fails with `Use of uninitialized value … at .../Biber.pm line 375.` and `I/O error : Is a directory` from libxml. The patch lets `parse_ctrlfile` give its message, and stops there even with `--nodieonerror`.
6. **The input encoding option is misspelt.** `bin/biber` declares `input_encoding|input-encodinge=s`, so `--input-encoding` only works through Getopt::Long's abbreviation of `--input-encodinge`, and the `-e` short option documented in the manual (`--input-encoding|-e`) gives `Unknown option: e`.
7. **`--fastsort` warns once per sorted list.** It is documented as a no-op kept for compatibility, but `sort_list` emits `fastsort option no longer required/supported, defaulting to UCA` for every list (two lists, two warnings), which inflates the warning count. The patch warns once.
8. **A non-tool `--output-format=biblatexml` (or `bibjson`) is reported on stdout with `say`**, not as an `ERROR` in the log. The patch uses `biber_error`.

### Suggested fix

```diff
--- a/bin/biber
+++ b/bin/biber
@@ -53,7 +53,7 @@
            'glob_datasources|glob-datasources',
            'help|h|?',
            'input_directory|input-directory=s',
-           'input_encoding|input-encodinge=s',
+           'input_encoding|input-encoding|e=s',
            'input_format|input-format=s',
            'isbn10',
            'isbn13',
@@ -378,7 +378,7 @@
       $outfile = $bcf =~ s/bcf$/dot/r;
     }
     elsif (Biber::Config->getoption('output_format') eq 'bibtex') { # bibtex output
-      $outfile = $bcf =~ s/\..+$/_biber.bib/r;
+      $outfile = $bcf =~ s/(?:\.[^.\/\\]*)?\z/_biber.bib/r;
     }
     elsif (Biber::Config->getoption('output_format') eq 'bbl') { # bbl output
       $outfile = $bcf =~ s/bcf$/bbl/r;
@@ -387,14 +387,14 @@
       $outfile = $bcf =~ s/bcf$/bblxml/r;
     }
     else {
-      say "output_format option '" . Biber::Config->getoption('output_format') . "' only makes sense in tool mode (--tool)";
-      exit EXIT_ERROR;
+      biber_error("output_format option '" . Biber::Config->getoption('output_format') . "' only makes sense in tool mode (--tool)");
     }
   }
 }
 
 # Set the .bbl path to the output dir, if specified
-if (my $outdir = Biber::Config->getoption('output_directory')) {
+my $outdir = Biber::Config->getoption('output_directory');
+if ($outdir and $outfile ne '-') { # '-' is STDOUT
   my (undef, undef, $file) = File::Spec->splitpath($outfile);
   $outfile = File::Spec->catfile($outdir, $file)
 }
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -354,10 +354,15 @@
 sub parse_ctrlfile {
   my ($self, $ctrl_file) = @_;
 
-  my $ctrl_file_path = locate_data_file($ctrl_file);
+  my $ctrl_file_path = locate_data_file($ctrl_file, 1);
   Biber::Config->set_ctrlfile_path($ctrl_file_path);
 
-  biber_error("Cannot find control file '$ctrl_file'! - Did latex run successfully on your .tex file before you ran biber?") unless ($ctrl_file_path and check_exists($ctrl_file_path));
+  # Fatal even with --nodieonerror: there is nothing to do without a control file
+  unless ($ctrl_file_path and check_exists($ctrl_file_path)) {
+    biber_error("Cannot find control file '$ctrl_file'! - Did latex run successfully on your .tex file before you ran biber?", 1);
+    $self->display_end;
+    exit EXIT_ERROR;
+  }
 
   # Early check to make sure .bcf is well-formed. If not, this means that the last biblatex run
   # exited prematurely while writing the .bcf. This results is problems for latexmk. So, if the
@@ -1717,7 +1722,8 @@
 
   if (Biber::Config->getoption('validate_datamodel')) {
     $logger->info("Datamodel validation starting");
-    my $dmwe = Biber::Config->getoption('dieondatamodel') ? \&biber_error : \&biber_warn;
+    # (biber_error's second argument is $nodie, so don't pass the entry to it)
+    my $dmwe = Biber::Config->getoption('dieondatamodel') ? sub {biber_error($_[0])} : \&biber_warn;
     foreach my $citekey ($section->get_citekeys) {
       my $be = $section->bibentry($citekey);
       my $bee = $be->get_field('entrytype');
@@ -4319,7 +4325,7 @@
     $logger->debug("Locale for sorting is '$thislocale'");
   }
 
-  if ( Biber::Config->getoption('fastsort') ) {
+  if ( Biber::Config->getoption('fastsort') and not $self->{fastsort_warned}++ ) {
     biber_warn("fastsort option no longer required/supported, defaulting to UCA");
   }
 
--- a/lib/Biber/Config.pm
+++ b/lib/Biber/Config.pm
@@ -121,7 +121,14 @@
   unless (defined($opts->{noconf})) {
     # if a config file was given as cmd-line arg, it overrides all other
     # config file locations
-    unless ( defined($opts->{configfile}) and -f $opts->{configfile} ) {
+    if (defined($opts->{configfile})) {
+      unless (-f $opts->{configfile}) {
+        # The logger isn't set up yet
+        say STDERR "ERROR - Cannot find config file '$opts->{configfile}'";
+        exit EXIT_ERROR;
+      }
+    }
+    else {
       $opts->{configfile} = config_file();
     }
   }
--- a/lib/Biber/Output/base.pm
+++ b/lib/Biber/Output/base.pm
@@ -63,6 +63,9 @@
     $enc_out = ":encoding($enc)";
   }
 
+  # STDOUT: don't create a file called '-' (the output methods open STDOUT)
+  return undef if not ref($file) and $file eq '-';
+
   # This is only for test suite when output is to a scalar ref
   if (ref($file) eq 'SCALAR') {
     return IO::File->new($file, ">$enc_out");
--- a/lib/Biber/Utils.pm
+++ b/lib/Biber/Utils.pm
@@ -223,7 +223,7 @@
 =cut
 
 sub locate_data_file {
-  my $source = shift;
+  my ($source, $noerror) = @_;
   my $sourcepath = $source; # default if nothing else below applies
   my $foundfile;
 
@@ -379,6 +379,7 @@
   }
 
   # Not found
+  return undef if $noerror; # caller reports it
   biber_error("Cannot find '$source'!")
 }
 
```

All tests in `t/` pass on `dev` (1686b9c) with this patch.

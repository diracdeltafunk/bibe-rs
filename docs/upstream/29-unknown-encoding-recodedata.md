<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-165, BB-167 (docs/biber-bugs.md). -->

**Title:** An unknown `--output-encoding` leaves an empty `.bbl` instead of giving an error, and a `--recodedata` file that can't be found dies without naming it

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

**1. Encoding names are never checked.** `--output-encoding=foo` (or any name Encode doesn't know) is passed straight to `binmode($fh, ':encoding(foo)')`. With biber 2.22 and current `dev` (1686b9c) run from source, the log says

```
Cannot find encoding "foo" at .../IO/File.pm line 58.
INFO - Writing 't.bbl' with encoding 'foo'
```

and then the `.bbl` is empty: the bibliography is written to standard output instead, and `dev` exits with status 0 (2.22 with 255). With the TeX Live 2.22 binary, the same happens for real encodings that the binary doesn't pack, e.g. `--output-encoding=gb18030` (Encode::HanExtra isn't included): "Cannot find encoding", then the binary dies with "Unknown encoding …" at `Output/bbl.pm` line 741, exit 25 and an empty `.bbl`. (An unknown `--input-encoding` is caught, but only when the data file is opened, with the misleading `Data file 't.bib' cannot be read in encoding 'bar': Couldn't open t.bib: Invalid argument`.)

**2. A `--recodedata` file that kpsewhich can't find.** With

```
biber --recodedata=nosuch.xml test
```

`kpsewhich` prints nothing, `$mapdata` becomes `undef`, and biber logs `Using user-defined recode data file ''` and then dies with

```
Couldn't open : No such file or directory at .../Biber/Utils.pm line 185.
```

(exit 2), which doesn't say which file it was looking for. When `kpsewhich` isn't installed at all, the error branch calls `biber_error`, which `Biber::LaTeX::Recode` doesn't import:

```
Undefined subroutine &Biber::LaTeX::Recode::biber_error called at .../Biber/LaTeX/Recode.pm line 96.
```

and the message would use the undefined `$err`. Both branches also call the file an "output_safechars data file", although it holds the decoding data too.

### Suggested fix

Check encoding names where they are normalised, and also call that in tool mode (where `normalise_utf8` currently doesn't run); look for the recode file under its own name when kpsewhich doesn't find it, and give a clear error otherwise. `$MASTER` has to be set before `init_sets` so that `biber_error` can be used there:

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -87,12 +87,13 @@
                                         CLEANUP => (Biber::Config->getoption('noremove_tmp_dir') ? 0 : 1));
   $self->{TEMPDIRNAME} = $self->{TEMPDIR}->dirname;
 
+  # biber_error() needs $MASTER
+  $MASTER = $self;
+
   # Initialise recoding schemes
   Biber::LaTeX::Recode->init_sets(Biber::Config->getoption('decodecharsset'),
                                   Biber::Config->getoption('output_safecharsset'));
 
-  $MASTER = $self;
-
   # Validate if asked to.
   # This has to be here, after config file is read and options
   # are parsed. It seems strange to validate the config file after it's been
@@ -288,6 +289,7 @@
 
 sub tool_mode_setup {
   my $self = shift;
+  normalise_utf8();
   my $bib_sections = new Biber::Sections;
   # There are no sections in tool mode so create a pseudo-section
   my $bib_section = new Biber::Section('number' => 99999);
--- a/lib/Biber/LaTeX/Recode.pm
+++ b/lib/Biber/LaTeX/Recode.pm
@@ -85,15 +85,17 @@
     if ( can_run('kpsewhich') ) {
       run3 [ 'kpsewhich', $rdata ], \undef, \$mapdata, \$err, { return_if_system_error => 1};
       if ($? == -1) {
-        biber_error("Error running kpsewhich to look for output_safechars data file: $err");
+        Biber::Utils::biber_error("Error running kpsewhich to look for recode data file: $err");
       }
 
       chomp $mapdata;
       $mapdata =~ s/\cM\z//xms; # kpsewhich in cygwin sometimes returns ^M at the end
       $mapdata = undef unless $mapdata; # sanitise just in case it's an empty string
     }
-    else {
-      biber_error("Can't run kpsewhich to look for output_safechars data file: $err");
+    # Without kpsewhich, or if it didn't find the file, try the name as given
+    $mapdata //= $rdata if -f $rdata;
+    unless (defined($mapdata)) {
+      Biber::Utils::biber_error("Cannot find recode data file '$rdata'");
     }
     $logger->info("Using user-defined recode data file '$mapdata'");
   }
--- a/lib/Biber/Utils.pm
+++ b/lib/Biber/Utils.pm
@@ -1064,13 +1064,15 @@
 =cut
 
 sub normalise_utf8 {
-  if (defined(Biber::Config->getoption('input_encoding')) and
-      Biber::Config->getoption('input_encoding') =~ m/\Autf-?8\z/xmsi) {
-    Biber::Config->setoption('input_encoding', 'UTF-8');
-  }
-  if (defined(Biber::Config->getoption('output_encoding')) and
-      Biber::Config->getoption('output_encoding') =~ m/\Autf-?8\z/xmsi) {
-    Biber::Config->setoption('output_encoding', 'UTF-8');
+  foreach my $opt ('input_encoding', 'output_encoding') {
+    my $enc = Biber::Config->getoption($opt);
+    next unless defined($enc);
+    if ($enc =~ m/\Autf-?8\z/xmsi) {
+      Biber::Config->setoption($opt, 'UTF-8');
+    }
+    elsif (not find_encoding($enc)) {
+      biber_error("Unknown encoding '$enc' for option '$opt'");
+    }
   }
 }
 
```

With this change, `--output-encoding=foo` gives `ERROR - Unknown encoding 'foo' for option 'output_encoding'` (exit 2) before anything is written, `--input-encoding` is checked the same way in normal and tool mode, and `--recodedata=nosuch.xml` gives `ERROR - Cannot find recode data file 'nosuch.xml'` (exit 2) with or without kpsewhich. All tests in `t/` pass on `dev` (1686b9c) with this patch.

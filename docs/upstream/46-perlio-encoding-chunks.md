<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-152, BB-166, BB-168 (docs/biber-bugs.md). -->

**Title:** Non-UTF-8 input and output encodings break on characters that straddle PerlIO's 1024-byte `:encoding` buffer, and ISO-2022-JP input can hang

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biber 2.22 and `dev` (1686b9c), Perl 5.38.2, Encode 3.21.

biber reads data files with `File::Slurper::read_text($filename, $encoding)` and writes the `.bbl` (and tool-mode `.bib`) through handles opened with `:encoding($enc)`. PerlIO's `:encoding` layer converts in chunks of 1024 bytes, and characters that fall across a chunk boundary are not handled correctly. Arguably these are bugs in `PerlIO::encoding`, but biber can avoid the layer completely, since it reads whole files anyway and writes whole entries.

**1. Reading.** A UTF-16 `.bib` whose surrogate pair starts at byte 1022:

```python
pad = '%' + 'x' * 508 + '\n'      # with the BOM, the emoji starts at byte 1022
s = pad + '\U0001F600\n@book{a, author={Doe, J}, title={T}, date={2000}}\n'
open('u16.bib', 'wb').write(s.encode('utf-16'))
```

`biber --input-encoding=UTF-16` stops with

```
ERROR - Data file 'u16.bib' cannot be read in encoding 'UTF-16': UTF-16:Partial character at /usr/share/perl5/File/Slurper.pm line 59.
```

The file is valid UTF-16 (Python and `Encode::decode('UTF-16', …)` read it).

**2. Writing.** With `--output-encoding=latin1` and

```bibtex
@book{a, author={Doe, J}, date={2000}, title={x€€€…€}}   % "x" followed by 400 "€"
```

`€` has no LaTeX macro in biber's recoding data, so the layer writes it with its `\x{20ac}` fallback. But the `.bbl` has only 398 `\x{20ac}`, and in their place `\x{fffd}\x{220ac}`: the encoder got the first byte of a `€` at the end of one chunk, and the rest with the next character at the start of the following one. Without the `x`, all 400 are correct; where the damage happens depends only on where characters fall in the file. The same happens with other encodings: in our regression corpus, the KOI8-R, CP1251, ISO-2022-JP and 7-bit JIS outputs of a test file with CJK text contain `\x{fffd}` and wrong code points, the HZ output has redundant `~}~{` pairs, and the ISO-2022-KR output repeats its `ESC $ ) C` designator at the start of every line (175 times in a 164-line file).

**3. Reading stateful encodings can hang.** With `--input-encoding=iso-2022-jp` (also `iso-2022-jp-1`, `7bit-jis`), a single 8-bit byte in the data file, e.g. `title={T\xe9}`, makes biber loop forever at the end of the file inside `Encode.pm` (I stopped it after 20 s). Decoding the whole file avoids the loop; but `Encode`'s ISO-2022-JP decoder doesn't croak on such a byte even with `FB_CROAK`, it just stops and leaves the rest of the input unconsumed, so the patch checks for that and reports `ERROR - Data file 'j.bib' cannot be read in encoding 'iso-2022-jp': invalid data at byte 34` instead of passing on truncated data.

### Suggested fix

Decode the whole file with `Encode::decode`, and for non-UTF-8 output open the handle raw and encode each string in `out()`. UTF-16 and UTF-32 without a byte order need care, because `encode('UTF-16', …)` adds a BOM to every string: the patch writes the BOM once and then encodes as big-endian, which is what the layer does. The `out()` encoding uses `FB_PERLQQ | WARN_ON_ERR`, which is the layer's default fallback, so unencodable characters give the same `\x{…}` text and the same "does not map" warnings as before.

```diff
--- a/lib/Biber/Output/base.pm
+++ b/lib/Biber/Output/base.pm
@@ -58,21 +58,16 @@
 
   my $file_bytes = encode_CS_system( $file );
   $self->{output_target_file} = $file;
-  my $enc_out;
-  if (my $enc = Biber::Config->getoption('output_encoding')) {
-    $enc_out = ":encoding($enc)";
-  }
-
   # STDOUT: don't create a file called '-' (the output methods open STDOUT)
   return undef if not ref($file) and $file eq '-';
 
   # This is only for test suite when output is to a scalar ref
   if (ref($file) eq 'SCALAR') {
-    return IO::File->new($file, ">$enc_out");
+    return open_output($file, Biber::Config->getoption('output_encoding'));
   }
   # Calls to OS file system use byte strings encoded in the system CS:
   else {
-    return IO::File->new($file_bytes, ">$enc_out");
+    return open_output($file_bytes, Biber::Config->getoption('output_encoding'));
   }
 }
 
--- a/lib/Biber/Output/bbl.pm
+++ b/lib/Biber/Output/bbl.pm
@@ -702,11 +702,7 @@
   }
 
   if (not $target or $target_string eq '-') {
-    my $enc_out;
-    if (Biber::Config->getoption('output_encoding')) {
-      $enc_out = ':encoding(' . Biber::Config->getoption('output_encoding') . ')';
-    }
-    $target = new IO::File ">-$enc_out";
+    $target = open_output('-', Biber::Config->getoption('output_encoding'));
   }
 
   if ($logger->is_debug()) {# performance tune
--- a/lib/Biber/Output/bibtex.pm
+++ b/lib/Biber/Output/bibtex.pm
@@ -437,17 +437,7 @@
 
   # Instantiate output file now that input is read in case we want to do in-place
   # output for tool mode
-  my $enc_out;
-  if (Biber::Config->getoption('output_encoding')) {
-    $enc_out = ':encoding(' . Biber::Config->getoption('output_encoding') . ')';
-  }
-
-  if ($target_string eq '-') {
-    $target = new IO::File ">-$enc_out";
-  }
-  else {
-    $target = IO::File->new($target_string, ">$enc_out");
-  }
+  $target = open_output($target_string, Biber::Config->getoption('output_encoding'));
 
   if ($logger->is_debug()) {# performance tune
     $logger->debug('Preparing final output using class ' . __PACKAGE__ . '...');
--- a/lib/Biber/Utils.pm
+++ b/lib/Biber/Utils.pm
@@ -12,6 +12,7 @@
 use Carp;
 use Encode;
 use File::Find;
+use IO::File;
 use File::Spec;
 use IPC::Cmd qw( can_run );
 use IPC::Run3; # This works with PAR::Packer and Windows. IPC::Run doesn't
@@ -45,7 +46,7 @@
 
 =cut
 
-our @EXPORT = qw{ check_empty check_exists slurp_switchr slurp_switchw
+our @EXPORT = qw{ check_empty check_exists slurp_switchr slurp_switchw open_output
   glob_data_file globU globU1 locate_data_file makenamesid makenameid stringify_hash
   normalise_string normalise_string_hash normalise_string_underscore
   normalise_string_sort normalise_string_label reduce_array remove_outer
@@ -181,7 +182,14 @@
     $fh->close;
   }
   else {
-    $slurp = File::Slurper::read_text($filename, $encoding);
+    # Decode the whole file at once: PerlIO's :encoding layer works in 1024-byte chunks
+    # and fails on (UTF-16) or mangles characters that straddle a chunk boundary
+    my $bytes = File::Slurper::read_binary($filename);
+    my $size = length($bytes);
+    $slurp = decode($encoding, $bytes, Encode::FB_CROAK);
+    # Some decoders (e.g. iso-2022-jp) stop at invalid input instead of croaking,
+    # leaving the rest in $bytes
+    die "invalid data at byte " . ($size - length($bytes)) . "\n" if length($bytes);
   }
   return \$slurp;
 }
@@ -1588,7 +1596,53 @@
 
 sub out {
   my ($fh, $string) = @_;
-  print $fh NFC($string);# Unicode NFC boundary
+  if (my $enc = ${*$fh}{biber_output_encoding}) { # see open_output()
+    print $fh encode($enc, NFC($string), Encode::FB_PERLQQ | Encode::WARN_ON_ERR | Encode::LEAVE_SRC);
+  }
+  else {
+    print $fh NFC($string);# Unicode NFC boundary
+  }
+}
+
+=head2 open_output
+
+  Open an output target ('-' for STDOUT, a scalar ref, or a file name as a byte string)
+  for writing in encoding $enc. Encodings other than UTF-8 are not done with a PerlIO
+  :encoding layer, which mangles characters straddling its 1024-byte buffer: the handle
+  is opened raw and out() encodes each string instead.
+
+=cut
+
+sub open_output {
+  my ($target, $enc) = @_;
+  my $layer = '';
+  my $encode;
+  if ($enc) {
+    if (lc(find_encoding($enc)->name) =~ m/\Autf-?8/) {
+      $layer = ":encoding($enc)";
+    }
+    else {
+      $layer = ':raw';
+      $encode = $enc;
+    }
+  }
+  my $fh;
+  if (not ref($target) and $target eq '-') {
+    $fh = IO::File->new('>-') or return undef;
+    $fh->binmode($layer) if $layer;
+  }
+  else {
+    $fh = IO::File->new($target, ">$layer") or return undef;
+  }
+  if ($encode) {
+    # UTF-16/32 without a byte order would get a BOM from every encode() call
+    if (find_encoding($encode)->name =~ m/\AUTF-(?:16|32)\z/i) {
+      print $fh encode($encode, '');# just the BOM
+      $encode = find_encoding($encode)->name . 'BE';
+    }
+    ${*$fh}{biber_output_encoding} = $encode;
+  }
+  return $fh;
 }
 
 =head2 process_comment
```

With this patch, the UTF-16 file is read, the latin1 `.bbl` has 400 correct `\x{20ac}`, and for the 64 encoding cases of our regression corpus the output is byte-identical except for the damaged places above (ISO-2022-KR now repeats its designator once per entry, which is valid). Stateful encodings are encoded one entry at a time, and Encode returns to ASCII at the end of each string, so the output stays valid. The read change does not touch the Windows code-page branch of `slurp_switchr`. It also fixes `-O -` with a non-UTF-8 `--output-encoding`, which currently writes UTF-8 to standard output: `IO::File->new(">-$enc_out")` is a two-argument open, which opens STDOUT but ignores the layer (`title={café}` comes out as `caf\303\251` instead of `caf\351` with `--output-encoding=latin1`). All tests in `t/` pass on `dev` (1686b9c) with this patch.

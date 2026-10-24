<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/521 (2026-10-24). Tracks BB-048 and BB-169 (docs/biber-bugs.md). -->

**Title:** False "cannot be encoded" warnings: any `?` in tool/bibtex output, and every entry in UTF-16/UTF-32 output

---

The check for characters that the output encoding cannot represent gives false positives in two ways. Each false positive produces a warning and LaTeX-recodes the whole entry:

1. **Tool mode and bibtex output** (`Output/bibtex.pm`) and `Output/base.pm`. The check is `encode($outenc, NFC($s)) =~ /\?/`, so any literal `?` counts as unencodable. That covers a title like "Is it true?" and any URL with a query string. `Output/bbl.pm` was already changed to use `"\0"` for exactly this reason, but the other two writers still use `?`.
2. **`.bbl` output in UTF-16, UTF-32 or UCS-2.** `Output/bbl.pm` uses `encode($outenc, NFC($s), sub {"\0"}) =~ /\0/`, but these encodings always contain NUL bytes. So every entry is reported as unencodable and recoded.

### Examples

`tool.bib`:

```bibtex
@online{q, author = {Doe, Jane}, title = {Is it true?}, url = {https://example.org/page?id=1}, date = {2001}}
@online{p, author = {Doe, Jane}, title = {Plain}, date = {2002}}
```

```
$ biber --tool --output-encoding=latin1 tool.bib
WARN - The entry 'q' has characters which cannot be encoded in 'latin1'. Recoding problematic characters into macros.
```

With any document and `--output-encoding=UTF-16`, every entry warns, `p` included:

```
WARN - The entry 'q' has characters which cannot be encoded in 'UTF-16'. Recoding problematic characters into macros.
WARN - The entry 'p' has characters which cannot be encoded in 'UTF-16'. Recoding problematic characters into macros.
```

(biber 2.22, TeX Live 2026, macOS.)

### Suggested fix

Instead of looking for a substitution character, ask `Encode` to fail on unmappable characters. That works for every encoding and needs no sentinel character:

```diff
--- a/lib/Biber/Output/base.pm
+++ b/lib/Biber/Output/base.pm
@@ -313,7 +313,7 @@
     my $outenc = Biber::Config->getoption('output_encoding');
     if ($outenc ne 'UTF-8') {
       # Can this entry be represented in the output encoding?
-      if (encode($outenc, NFC($out_string)) =~ /\?/) { # Malformed data encoding char
+      if (not eval { encode($outenc, NFC($out_string), Encode::FB_CROAK | Encode::LEAVE_SRC); 1 }) { # Malformed data encoding char
         # So convert to macro
         $out_string = latex_recode_output($out_string);
         biber_warn("The entry '$key' has characters which cannot be encoded in '$outenc'. Recoding problematic characters into macros.");
--- a/lib/Biber/Output/bbl.pm
+++ b/lib/Biber/Output/bbl.pm
@@ -84,7 +84,7 @@
       my $outenc = Biber::Config->getoption('output_encoding');
       if ($outenc ne 'UTF-8') {
         # Can this entry be represented in the output encoding?
-        if (encode($outenc, NFC($pa), sub {"\0"}) =~ /\0/) { # Malformed data encoding char
+        if (not eval { encode($outenc, NFC($pa), Encode::FB_CROAK | Encode::LEAVE_SRC); 1 }) { # Malformed data encoding char
           # So convert to macro
           $pa = latex_recode_output($pa);
         }
@@ -738,7 +738,7 @@
             # Can this entry be represented in the output encoding?
             # We must have an ASCII-safe replacement string for encode which is unlikely to be
             # in the string. Default is "?" which could easily be in URLS so we choose ASCII null
-            if (encode($outenc, NFC($entry_string), sub {"\0"})  =~ /\0/) { # Malformed data encoding char
+            if (not eval { encode($outenc, NFC($entry_string), Encode::FB_CROAK | Encode::LEAVE_SRC); 1 }) { # Malformed data encoding char
               # So convert to macro
               $entry_string = latex_recode_output($entry_string);
               biber_warn("The entry '$k' has characters which cannot be encoded in '$outenc'. Recoding problematic characters into macros.");
--- a/lib/Biber/Output/bibtex.pm
+++ b/lib/Biber/Output/bibtex.pm
@@ -388,7 +388,7 @@
     my $outenc = Biber::Config->getoption('output_encoding');
     if ($outenc ne 'UTF-8') {
       # Can this entry be represented in the output encoding?
-      if (encode($outenc, NFC($acc)) =~ /\?/) { # Malformed data encoding char
+      if (not eval { encode($outenc, NFC($acc), Encode::FB_CROAK | Encode::LEAVE_SRC); 1 }) { # Malformed data encoding char
         # So convert to macro
         $acc = latex_recode_output($acc);
         biber_warn("The entry '$key' has characters which cannot be encoded in '$outenc'. Recoding problematic characters into macros.");
```

With this patch:
- neither example warns;
- a genuinely unencodable character (`author = {Łukasiewicz, Jan}` with `--output-encoding=latin1`) still warns and is written as `\L{}ukasiewicz`;
- the comment about choosing NUL in `bbl.pm` could then go.

I ran the patched biber over 776 test documents (the biblatex examples, the biber test suite's control files, and documents for the standard and contributed styles), including the test suite's latin1, UTF-16 and other encoding tests. Output is unchanged except where these false positives fired. The code is unchanged on `dev` (df6438f).

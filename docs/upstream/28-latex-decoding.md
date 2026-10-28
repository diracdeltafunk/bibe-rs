<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-029, BB-115, BB-117, BB-118, BB-119 (docs/biber-bugs.md). -->

**Title:** LaTeX macro decoding: `\textdollar`/`\textpercent`/`\textampersand` decode to raw TeX specials, `\ding` reads its argument as hex, `--decodecharsset=null` strips protecting braces, and smaller accent/dash problems

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

These are all in `Biber::LaTeX::Recode::latex_decode` and `recode_data.xml`. Each item was checked on biber 2.22 and current `dev` (1686b9c), both by calling `latex_decode` directly and in the `.bbl`.

**1. `\textdollar`, `\textpercent` and `\textampersand` become raw `$`, `%` and `&`.** With

```bibtex
@misc{a, title={Price \textdollar 5 and 10\textpercent{} \textampersand\ more}}
```

and the default settings, the `.bbl` contains

```
      \field{title}{Price $5 and 10%{} &{} more}
```

so the `%` comments out the rest of the line, including the closing brace, and LaTeX stops with a runaway argument; `$` and `&` are just as harmful. `decode_exclude` already lists `textbraceleft`, `textbraceright`, `textunderscore`, `textasciitilde` and `textasciicircum` for this reason, but not these three. (`t/utils.t` pins `\textampersand` → `&`, but I think that test is only about the XML escaping in the data file.)

**2. `\ding{n}` is decoded as if `n` were hexadecimal.** pifont's `\ding` takes a decimal character code, but the `dings` map is keyed by hex codes and the decoder looks the argument up unchanged. With `--decodecharsset=full`:

| input | biber gives | pifont prints |
|---|---|---|
| `\ding{51}` | ✱ (0x51) | ✓ (51 = 0x33) |
| `\ding{33}` | ✓ (0x33) | ✁ (33 = 0x21) |
| `\ding{99}` | *(empty, "Use of uninitialized value" on stderr)* | ❃ |
| `\ding{120}` | `\ding{120}` (three digits never match) | ❘ |

Encoding has the matching error: with `--output-safecharsset=full`, ✓ is written as `\ding{33}`, which prints ✁.

**3. `--decodecharsset=null` still decodes, and strips protecting braces.** The manual says the `null` set "effectively turns off macro to UTF-8 decoding", but `\char` is still decoded, and the brace elimination at the end of `latex_decode` still runs. Without the diacritics pass, nothing marks user braces as protected first, so:

```bibtex
@misc{a, title={Part {I} x}, note={\char65}}
```

gives `\field{title}{Part I x}` and `\field{note}{A}` with `--decodecharsset=null`, compared with `Part {I} x` without it. The case protection is lost, and `\'{e}` turns into `\'e`.

**4. A space before a braced accent argument stops decoding.** `\' {a}` is left as it is (TeX prints á). The `{\X}` protection step only looks for the accent macro *immediately* before the brace, so it protects `{a}` first, and the accent regex then no longer matches.

**5. A letter accent followed by spaces is applied to a space.** `\c  ,` gives a space carrying a combining cedilla, followed by ` ,`. When the `\s*` after `\c` backtracks to an empty match, the "no space" branch accepts any non-ASCII-letter, including the space itself.

**6. `\t{oo}` puts the tie on the brace.** `\t` maps to U+0311 like a one-letter accent, so `\t{oo}` gives `{̑oo}` (the combining mark sits on `{`, and the braces stay). The tie spans two letters: `o͡o` (U+0361 between them).

**7. Punctuation and symbol macros at the end of the string, or before `{}`, are not decoded.** The terminator for the `punctuation`, `symbols` and `greek` types is written `(?: \{\}|\s+|\b)`, with a literal space before `\{\}` (the pattern isn't `/x`). So `\textdollar{}5` is decoded but leaves `{}` behind (`${}5` before item 1's change), and since `\b` needs a word character on one side, a non-letter macro at the end of the value is never decoded: `y\---` stays as it is, while `y\--- z` and `\--x` are decoded. Conversely, `\b` never matches between a letter macro and a following digit, although TeX ends a macro name at the first non-letter: `number = {C\textendash118/07}` (from the `.bib` of biblatex-oxref's documentation) stays as it is, where it should become `C–118/07`. The patch's terminator (`(?<![a-zA-Z])|(?![a-zA-Z])`) handles both cases.

### Suggested fix

```diff
--- a/lib/Biber/LaTeX/Recode.pm
+++ b/lib/Biber/LaTeX/Recode.pm
@@ -212,6 +212,9 @@
     # Special case for \h?box which requires braces around single chars
     $text =~ s/\\(h?box)\s+\{(\X)\}/\\$1\{$2\}/g;
 
+    # The virtual 'null' set turns decoding off
+    return $text if $set_d eq 'null';
+
     # Deal with raw TeX \char macros.
     $text =~ s/\\char"(\p{ASCII_Hex_Digit}+)/"chr(0x$1)"/gee; # hex chars
 
@@ -233,13 +236,14 @@
         $text =~ s/\\textsuperscript\{\\($re)\}/$map->{$1}/ge;
       }
       elsif ($type eq 'dings') {
-        $text =~ s/\\ding\{([2-9AF][0-9A-F])\}/$map->{$1}/ge;
+        # \ding takes a decimal character code; the map is keyed by hex codes
+        $text =~ s/\\ding\{(\d+)\}/$map->{sprintf('%X', $1)} \/\/ "\\ding{$1}"/ge;
       }
       elsif ($type eq 'letters') {
         $text =~ s/\\($re)(?:\{\}|\s+|\b)/$map->{$1}/ge;
       }
       elsif (first {$type eq $_} ('punctuation', 'symbols', 'greek')) {
-        $text =~ s/\\($re)(?: \{\}|\s+|\b)/$map->{$1}/ge;
+        $text =~ s/\\($re)(?:\{\}|\s+|(?<![a-zA-Z])|(?![a-zA-Z]))/$map->{$1}/ge;
       }
       elsif ($type eq 'diacritics') {
 
@@ -255,7 +259,10 @@
         # braces
         # Using the VLB method from the link below, this is equivalent to:
         # $text =~ s/(?<!\\$re)\{(\X)\}/\x{f}$1\x{e}/g;
-        $text =~ s/(?!(?=(?'a'[\s\S]*))(?'b'\\$re(?=\k'a'\z)|(?<=(?=x^|(?&b))[\s\S])))\{(\X)\}/\x{f}$3\x{e}/g;
+        $text =~ s/(?!(?=(?'a'[\s\S]*))(?'b'\\$re\s*(?=\k'a'\z)|(?<=(?=x^|(?&b))[\s\S])))\{(\X)\}/\x{f}$3\x{e}/g;
+
+        # The tie accent spans two letters: \t{oo} -> o U+0361 o
+        $text =~ s/\\t\s*\{(\pL\pM*)(\pL\pM*)\}/$1\x{361}$2/g;
 
         # Rename protecting braces so that they are not broken by RE manipulations
         $text =~ s/(\{?)\\($re)\s*\{(\pL\pM*)\}(\}?)/$bracemap->{$1} . $3 . $map->{$2} . $bracemap->{$4}/ge;
@@ -282,7 +289,7 @@
                        (?(?{$2}) # code block condition (space matched earlier after diacritic?)
                          \pL # yes pattern
                        | # no pattern
-                         [^A-Za-z]
+                         [^A-Za-z\s]
                        ) # close conditional
                      ) # close conditional
                      \pM* # optional marks
@@ -386,7 +393,7 @@
       $text =~ s/($re)/"\\textsuperscript{\\" . $map->{$1} . "}"/ge;
     }
     elsif ($type eq 'dings') {
-      $text =~ s/($re)/'\ding{' . $map->{$1} . '}'/ge;
+      $text =~ s/($re)/'\ding{' . hex($map->{$1}) . '}'/ge;
     }
     elsif ($type eq 'letters') {
       # General macros (excluding special encoding excludes)
--- a/lib/Biber/LaTeX/recode_data.xml
+++ b/lib/Biber/LaTeX/recode_data.xml
@@ -865,6 +865,9 @@
     <char>textasciicircum</char>
     <char>textasciigrave</char>
     <char>dots</char>
+    <char>textdollar</char>
+    <char>textpercent</char>
+    <char>textampersand</char>
   </decode_exclude>
   <encode_exclude>
     <char>"</char>
--- a/t/utils.t
+++ b/t/utils.t
@@ -75,7 +75,7 @@
 # There is a "\x{131}\x{304}" but might look like nothing in current font
 eq_or_diff(NFC(latex_decode('Mu\d{h}ammad ibn M\=us\=a al-Khw\=arizm\={\i} \r{a}')), 'Muḥammad ibn Mūsā al-Khwārizmı̄ å', 'latex decode 1');
 eq_or_diff(latex_decode('\alpha'), '\alpha', 'latex decode 2'); # no greek decoding by default
-eq_or_diff(latex_decode('\textless\textampersand'), '<&', 'latex decode 3'); # checking XML encoding bits
+eq_or_diff(latex_decode('\textless\textampersand'), '<\textampersand', 'latex decode 3'); # checking XML encoding bits; & is special in LaTeX and isn't decoded
 eq_or_diff(latex_encode(NFD('Muḥammad ibn Mūsā al-Khwārizmī')), 'Mu\d{h}ammad ibn M\={u}s\={a} al-Khw\={a}rizm\={\i}', 'latex encode 1');
 eq_or_diff(latex_encode(NFD('α')), 'α', 'latex encode 2'); # no greek encoding by default
 eq_or_diff(NFC(latex_decode("{M{\\'a}t{\\'e}}")), '{Máté}', 'latex decode accent 1 (with redundant explicit brace protection)');
```

With these changes, item 1's example gives `\field{title}{Price \textdollar 5 and 10\textpercent{} \textampersand{} more}`; `\ding{51}`, `\ding{33}`, `\ding{99}` and `\ding{120}` give ✓ ✁ ❃ ❘, and ✓ is encoded as `\ding{51}`; `--decodecharsset=null` leaves `Part {I} x`, `\'{e}` and `\char65` unchanged; `\' {a}` gives á; `\c  ,` is left alone; `\t{oo}` gives o͡o; and `y\---` gives `y—`. The `\hbox` normalisation still runs for the `null` set, which `t/utils.t` tests. All tests in `t/` pass on `dev` (1686b9c) with this patch, with the `t/utils.t` change above.

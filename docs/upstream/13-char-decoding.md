<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/531 (2026-10-28). Tracks BB-003 and BB-116 (docs/biber-bugs.md). -->

**Title:** `\char` decoding: `\char"7D` and `\char0` are fatal, `\char012` is read as octal, and some values silently vanish

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

`latex_decode` (LaTeX/Recode.pm:215-218) converts raw TeX `\char` macros by string-evaluating `chr(…)`:

```perl
    $text =~ s/\\char"(\p{ASCII_Hex_Digit}+)/"chr(0x$1)"/gee; # hex chars
    $text =~ s/\\char'(\d+)/"chr(0$1)"/gee;  # octal chars
    $text =~ s/\\char(\d+)/"chr($1)"/gee;    # decimal chars
```

This goes wrong in several ways. Each line below is a one-entry `.bib` file followed by `@misc{z, title = {ok}}`, run through biber 2.22:

| `title` | biber 2.22 |
|---|---|
| `{[\char"E9]}`, `{[\char233]}` | `[é]` (correct) |
| `{[\char"7D]}` | fatal BibTeX syntax error, exit 2: the decoded `}` unbalances the re-serialised entry in `parse_decode` |
| `{a \char"7B b}` | fatal syntax error, and the following entry is swallowed |
| `{[\char0]}` | biber aborts (exit 134) with no message: the NUL byte reaches btparse |
| `{[\char"110000]}` | biber dies silently (exit 25, empty `.bbl`): `chr` of a non-character |
| `{[\char012]}` | `[` + newline + `]`: `chr(012)` is Perl octal, but TeX reads `\char012` as decimal 12 |
| `{[\char09]}`, `{[\char'89]}` | `[]`: `chr(09)` is a Perl syntax error, so the eval returns undef and the macro vanishes |
| `{[\char99999999999999999999]}` | `[]` (same) |
| `{[\char٣]}` (Arabic-Indic digit) | `[]`: `\d` matches non-ASCII digits, which Perl can't evaluate |

Current `dev` (012ba95) behaves the same.

### Minimal example

```bibtex
@misc{cb1, title = {a \char"7B b}}
@misc{cb2, title = {ok}}
```

```latex
\documentclass{article}
\usepackage[style=authoryear]{biblatex}
\addbibresource{test.bib}
\begin{document}
\nocite{*}
\printbibliography
\end{document}
```

```
ERROR - BibTeX subsystem: /tmp/biber_tmp_…/…_….utf8, line 6, syntax error: found "@misc", expected end of entry ("}" or ")") (skipping to next "@")
WARN - Entry cb1 does not parse correctly
```

### Suggested fix

Parse the number the way TeX does, without `eval`, and leave the macro alone when its character can't sensibly be put into the data: braces and backslash (they would change the structure of the re-serialised entry), control characters (their meaning in TeX depends on the font encoding: `\char12` is "fi" in OT1), and numbers that aren't Unicode characters. LaTeX then typesets these `\char`s itself.

```diff
--- a/lib/Biber/LaTeX/Recode.pm
+++ b/lib/Biber/LaTeX/Recode.pm
@@ -212,10 +212,8 @@
     # Special case for \h?box which requires braces around single chars
     $text =~ s/\\(h?box)\s+\{(\X)\}/\\$1\{$2\}/g;
 
-    # Deal with raw TeX \char macros.
-    $text =~ s/\\char"(\p{ASCII_Hex_Digit}+)/"chr(0x$1)"/gee; # hex chars
-    $text =~ s/\\char'(\d+)/"chr(0$1)"/gee;  # octal chars
-    $text =~ s/\\char(\d+)/"chr($1)"/gee;    # decimal chars
+    # Deal with raw TeX \char macros: \char"7B (hex), \char'173 (octal), \char123 (decimal)
+    $text =~ s/(\\char(?:"([0-9A-Fa-f]+)|'([0-7]+)|([0-9]+)))/_tex_char($1, $2, $3, $4)/ge;
 
     $text =~ s/(\\[a-zA-Z]+)\\(\s+)/$1\{\}$2/g;    # \foo\ bar -> \foo{} bar
     $text =~ s/([^{]\\\w)([;,.:%])/$1\{\}$2/g;     #} Aaaa\o,  -> Aaaa\o{},
@@ -346,6 +344,21 @@
     }
 }
 
+# Convert the number of a TeX \char to a character. Leave the macro alone if the number
+# is not a Unicode character, is a control character (whose meaning in TeX depends on the
+# font), or would be a brace or backslash, which would change the structure of the data.
+sub _tex_char {
+  my ($orig, $hex, $oct, $dec) = @_;
+  my ($digits, $base) = defined($hex) ? ($hex, 16) : defined($oct) ? ($oct, 8) : ($dec, 10);
+  $digits =~ s/^0+(?=.)//;
+  return $orig if length($digits) > 7;
+  my $n = $base == 16 ? hex($digits) : $base == 8 ? oct($digits) : $digits + 0;
+  return $orig if $n > 0x10FFFF or $n < 0x20 or $n == 0x7F or ($n >= 0xD800 and $n <= 0xDFFF);
+  my $c = chr($n);
+  return $orig if $c =~ /[{}\\]/;
+  return $c;
+}
+
 =head2 latex_encode($text, @options)
 
 Converts UTF-8 to LaTeX
```

With this change, `\char"E9`, `\char233` and `\char'351` still give `é`, and every other row of the table above is passed through unchanged (e.g. `\field{title}{[\char"7D]}`), with no errors. All tests in `t/` pass on `dev` (012ba95) with this patch.

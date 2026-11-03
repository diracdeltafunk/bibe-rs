<!-- Draft upstream issue for plk/biblatex (not biber). Posted as https://github.com/plk/biblatex/issues/1497 (2026-11-03). Tracks BB-195 (docs/biber-bugs.md). -->

**Title:** Sourcemap `match`/`replace` values are written to the `.bcf` without XML escaping, so `<`, `"` or `&` in a regex makes the `.bcf` malformed

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biblatex 3.22a and the `dev` branch (c89dfc8, the same code), with biber 2.22 and biber's current `dev`.

`\step` writes `fieldvalue` through `\blx@xmlsanitizeafter`, with the comment that the value "might contain quotes or ampersands", but `match`, `matchi`, `matches`, `matchesi`, `notmatch`, `notmatchi` and `replace` are written to the `.bcf` as they are. With

```latex
\documentclass{article}
\usepackage[backend=biber]{biblatex}
\DeclareSourcemap{
  \maps[datatype=bibtex]{
    \map{\step[fieldsource=title, match=\regexp{(?<=The\s)big}, replace={huge}]}
    \map{\step[fieldsource=title, match={"}, replace={'}]}
    \map{\step[fieldsource=note, match=\regexp{^(.*)$}, replace={<$1>}]}
  }
}
\addbibresource{test.bib}
\begin{document}
\nocite{*}
\printbibliography
\end{document}
```

and `@book{a, author={Doe, J}, title={The big "dog"}, date={2000}, note={n1}}`, the `.bcf` contains

```xml
<bcf:map_step map_field_source="title" map_match="(?<=The\s)big" map_replace="huge"/>
<bcf:map_step map_field_source="title" map_match=""" map_replace="'"/>
<bcf:map_step map_field_source="note" map_match="^(.*)$" map_replace="<$1>"/>
```

which is not well-formed XML. biber then stops with `ERROR - test.bcf is malformed, last biblatex run probably failed. Deleted test.bbl`. Any one of these steps is enough. Lookbehind assertions (`(?<=…)`, `(?<!…)`), named groups (`(?<name>…)`), `"` and `&` are all ordinary in Perl regexes and replacements, so at the moment such maps can only be written in `biber.conf`.

### Suggested fix

Sanitise these values the same way as `fieldvalue`:

```diff
--- a/tex/latex/biblatex/biblatex.sty
+++ b/tex/latex/biblatex/biblatex.sty
@@ -14337,20 +14337,29 @@
   \ifstrequal{#1}{true}
     {\appto\blx@tempf{ map\string_origentrytype="1"}}
     {}}
+% Regular expressions and replacement strings can contain characters which are
+% special in XML attribute values (<, &, ", e.g. in lookbehind assertions), so sanitise them
 \blx@kv@defkey{blx@sourcemap@step}{match}{% characters
-  \appto\blx@tempf{ map\string_match="#1"}}
+  \blx@xmlsanitizeafter{\def\blx@temps}{#1}%
+  \eappto\blx@tempf{ map\string_match="\blx@temps"}}
 \blx@kv@defkey{blx@sourcemap@step}{matchi}{% characters
-  \appto\blx@tempf{ map\string_matchi="#1"}}
+  \blx@xmlsanitizeafter{\def\blx@temps}{#1}%
+  \eappto\blx@tempf{ map\string_matchi="\blx@temps"}}
 \blx@kv@defkey{blx@sourcemap@step}{matches}{% characters
-  \appto\blx@tempf{ map\string_matches="#1"}}
+  \blx@xmlsanitizeafter{\def\blx@temps}{#1}%
+  \eappto\blx@tempf{ map\string_matches="\blx@temps"}}
 \blx@kv@defkey{blx@sourcemap@step}{matchesi}{% characters
-  \appto\blx@tempf{ map\string_matchesi="#1"}}
+  \blx@xmlsanitizeafter{\def\blx@temps}{#1}%
+  \eappto\blx@tempf{ map\string_matchesi="\blx@temps"}}
 \blx@kv@defkey{blx@sourcemap@step}{notmatch}{% characters
-  \appto\blx@tempf{ map\string_notmatch="#1"}}
+  \blx@xmlsanitizeafter{\def\blx@temps}{#1}%
+  \eappto\blx@tempf{ map\string_notmatch="\blx@temps"}}
 \blx@kv@defkey{blx@sourcemap@step}{notmatchi}{% characters
-  \appto\blx@tempf{ map\string_notmatchi="#1"}}
+  \blx@xmlsanitizeafter{\def\blx@temps}{#1}%
+  \eappto\blx@tempf{ map\string_notmatchi="\blx@temps"}}
 \blx@kv@defkey{blx@sourcemap@step}{replace}{% characters
-  \eappto\blx@tempf{ map\string_replace="#1"}}
+  \blx@xmlsanitizeafter{\def\blx@temps}{#1}%
+  \eappto\blx@tempf{ map\string_replace="\blx@temps"}}
 
 % [<specname>]{<spec>}
 \newrobustcmd*{\DeclareLabelalphaNameTemplate}[2][]{%
```

With this change, the steps above are written as `map_match="(?&lt;=The\s)big"`, `map_match="&quot;" map_replace="&apos;"` and `map_replace="&lt;$1&gt;"`. Both biber 2.22 and biber `dev` read the `.bcf`, and the `.bbl` has `\field{title}{The huge 'dog'}` and `\field{note}{<n1>}`.

Values without these characters are written exactly as before. I compared the `.bcf` of a document with ten steps using different kinds of values, among them `\regexp{\A(\d+)\s*-\s*(\d+)\Z}` with `replace=\regexp{$1--$2}`, `\regexp{^\{(.+)\}$}`, `\regexp{\\_}`, `matchesi={a,b}` and plain words. The only other change is for a macro in an unprotected `replace={…}`. `replace={\L$1}` used to be expanded by `\eappto` into the macro's LaTeX definition (`\unhbox \voidb@x \setbox \z@ …`), and is now written as `\L $1`. The manual writes this as `replace=\regexp{\L$1}`, which is unchanged. All 54 tests of `l3build check -ctestconfig-pdftex` pass with the patch, as they do without it.

(Named groups can be matched with this change, but biber can't yet use `$+{name}` in a replacement; that is plk/biber#548.)

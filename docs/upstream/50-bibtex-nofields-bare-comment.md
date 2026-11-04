<!-- Draft upstream issue for plk/biber (the fix is in Text::BibTeX 1.0's parser). Not yet filed. Tracks the 01-B21 note in docs/biber-bugs.md ("Not bugs, but worth mentioning upstream"). -->

**Title:** Text::BibTeX 1.0: an entry with no fields (`@misc{key}`) or a bare `@comment` stops biber, though BibTeX accepts both

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biber's `tb` branch (0e93619) and Text::BibTeX 1.0 (tag `v1.0`, 5a5be4c); biber `dev` with Text::BibTeX 0.91 behaves the same. Since 1.0 isn't released yet, this seemed a good moment to mention it.

BibTeX (0.99e) accepts two inputs that make biber stop. Because biber treats a BibTeX syntax error as fatal, one such entry costs the whole run: exit status 2 and an empty `.bbl`.

**1. An entry with a key but no fields.**

```bibtex
@misc{nofields}
@misc{after, title={After}, author={Zed, Z}}
```

`bibtex` writes `\bibitem`s for both, with only `Warning--to sort, need author or key in nofields`. biber stops with

```
ERROR - BibTeX subsystem: test.bib, line 1, syntax error: found "}", expected ","
```

With a trailing comma, `@misc{nofields,}`, the same entry is accepted.

**2. `@comment` not followed by `{…}` or `(…)`.** BibTeX ignores the word `@comment` itself, and what follows is ordinary text between entries. With

```bibtex
@comment some text here
@misc{after, title={After}, author={Zed, Z}}
```

`bibtex` reads `after` (and `@comment see also @misc{inner, title={Inner}}` even gives `inner` too), while biber stops with `syntax error: found "some", expected one of: start of entry ("{" or "(") or quoted string ({...} or "...")`. A real example is in TeX Live: `doc/latex/jurarsp/rsptest.bib` starts with three bare `@COMMENT` lines, and `biber --tool rsptest.bib` fails at line 2 instead of reading its four entries.

### Suggested fix

In Text::BibTeX's `src/parser.c`: accept the end of the entry right after the key, and skip a bare `@comment` the way the lexer skips other text between entries, without an error or a junk warning.

```diff
--- a/src/parser.c
+++ b/src/parser.c
@@ -19,7 +19,9 @@
                 entry        : '@' NAME body
                 body         : STRING                      (@comment only)
                              | ENTRY_OPEN contents ENTRY_CLOSE
-                contents     : (NAME | NUMBER) ',' fields  (regular entry)
+                             | (empty)  (@comment not followed by a STRING:
+                                         ignored, like BibTeX does)
+                contents     : (NAME | NUMBER) [ ',' fields ]  (regular entry)
                              | fields                      (@string)
                              | value                       (@preamble)
                 fields       : field { ',' fields } | (empty)
@@ -1120,6 +1122,8 @@ contents (bt_parser *p, AST *entry)
          {
             append_child (entry, token_node (p, BTAST_KEY));
             consume (p);
+            if (p->tok == T_ENTRY_CLOSE)  /* no fields, eg. @misc{key}, */
+               return;                    /* which BibTeX accepts */
             if (p->tok != T_COMMA)
             {
                EXPECT1 (p, T_COMMA, NULL);
@@ -1189,7 +1193,33 @@ body (bt_parser *p, AST *entry)
       resync (p, TOKSET (T_AT));
 }
 
-typedef enum { PARSE_EOF, PARSE_ENTRY, PARSE_FAILED } parse_result;
+typedef enum { PARSE_EOF, PARSE_ENTRY, PARSE_FAILED, PARSE_SKIPPED } parse_result;
+
+/* BibTeX ignores the word "@comment" itself, so when it is not followed
+ * by a braced or parenthesised comment, what follows it is ordinary text
+ * between entries: back up to the current token and skip to the next '@'
+ * at top level (without the junk warning; BibTeX doesn't warn either). */
+static void
+skip_bare_comment (bt_parser *p)
+{
+   int c;
+
+   p->pos = p->tok_pos;
+   p->line = p->tok_start_line;
+   while ((c = CUR (p)) >= 0 && c != '@')
+   {
+      if (c == '\n')
+      {
+         p->pos++;
+         p->line++;
+      }
+      else
+         take_char (p, NULL);
+   }
+   initialize_lexer_state (p);
+   bt_sbuf_clear (&p->text);
+   p->have_tok = FALSE;
+}
 
 static parse_result
 parse_one (bt_parser *p, AST **result)
@@ -1223,6 +1253,13 @@ parse_one (bt_parser *p, AST **result)
    entry = token_node (p, BTAST_ENTRY);
    entry->metatype = p->metatype;
    consume (p);
+   if (entry->metatype == BTE_COMMENT && p->tok != T_STRING
+       && p->tok != T_QUOTE)            /* "..." is reported by the lexer */
+   {
+      bt_free_ast (entry);
+      skip_bare_comment (p);
+      return PARSE_SKIPPED;
+   }
    body (p, entry);
    *result = entry;
    return PARSE_ENTRY;
@@ -1295,6 +1332,9 @@ bt_parse_entry (bt_parser *p, bt_errors *errs, btshort options,
          return NULL;
       }
 
+      if (r == PARSE_SKIPPED)           /* a bare @comment: not an error */
+         continue;
+
       /* PARSE_FAILED: no entry was started; try the next one */
       if (status) *status = FALSE;
    }
```

With this change, both examples give the same entries as `bibtex`. `@misc{nofields}` becomes an entry with a key and no fields, as `@misc{nofields,}` already does, and a bare `@comment` gives no entry and no message. Braced and parenthesised comments are unchanged: tool mode still writes them out. Line numbers in later messages stay right: a syntax error on line 4 after a two-line bare comment is reported as line 4. All 700 tests of Text::BibTeX pass with the patch, and so do all 1,308 tests of biber `tb` with the patched Text::BibTeX (with `BIBER_DEV_TESTS=1`).

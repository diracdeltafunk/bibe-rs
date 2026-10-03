# btparse behaviour as biber 2.22 sees it — specification

Status: v1 (2026-10-03). Normative input for `crates/btparse`.

This document specifies the observable behaviour of btparse (the C library in
Text::BibTeX 0.91) for everything biber 2.22 uses. It was written
**clean-room** (PLAN §9, decision #8): from Text::BibTeX's POD, from biber's
own (Artistic-2.0) Perl code, and from black-box experiments run against the
O3 oracle. No btparse source, grammar or C documentation was consulted.

Every rule cites the experiment(s) that establish it:

| Cite | Where |
|---|---|
| `V-nnn`, `L-nnn`, `M-Pn-XX`, `N-nnn`, `R-nnn`, `P-nnn`, `T-nnn`, `C-XX`, `U-1` | `### ID` blocks in `tools/oracle/probes/btparse/cases/*.txt` (parse experiments) |
| `names:n`, `names2:n`, `names3:n`, `split_list:n`, `split_list2:n` | line *n* of the corresponding `cases/*.txt` |
| `enum_commas`, `enum_tabs`, `scan_lower`, `namewarn` | `tools/oracle/probes/btparse/{enum_commas,enum_tabs,scan_lower,probe_namewarn}.pl` |
| `fuzz` | random differential runs (`gen.pl` + `vectors.pl`, §11) |

All experiments are replayed by `cargo test -p btparse --test spec_cases`
against oracle output recorded in `crates/btparse/tests/data/spec_cases.jsonl`
(regenerate: `cases2req.pl | vectors.pl`). Interactive probing:
`tools/oracle/probes/btparse/bt {cases|parse|list D|name|show} < input`.

Terminology: *diag* = one line libbtparse prints on stderr; *LA* = the
parser's one-token lookahead.

---

## 1. How biber drives btparse

(From `lib/Biber.pm`, `lib/Biber/Input/file/bibtex.pm`.)

1. **Macros.** Before reading data sources of each section biber does
   `delete_all_macros` if `--clrmacros`, then (unless `--nostdmacros`)
   `delete_macro(m); add_macro_text(m, n)` for `jan`..`dec` → `"1"`..`"12"`.
   Text::BibTeX itself defines `jan`..`dec` → `January`..`December` when
   loaded (spec probe: `BtProbe::reset_macros('tb')`). After pass 1 of each
   file biber re-adds the month macros with `add_macro_text` (overriding any
   `@string{jan=...}` read in pass 1; the override warnings are filtered).
   The macro table is process-global and accumulates across files and
   sections (§4.4).
2. **Files.** Each `.bib` is decoded, NFD-normalised, BOM-stripped, `\R`→`\n`
   (so btparse never sees `\r`, VT, FF, NEL, LS, PS in biber), written to a temp
   file and read with `Text::BibTeX::File->open($f, {binmode=>'utf-8',
   normalization=>'NFD'})` and `while (my $e = Text::BibTeX::Entry->new($bib))`.
   This happens twice: pass 1 (`parse_decode`) re-serialises every entry with
   LaTeX-decoded values (§9) and pass 2 (`cache_data`) parses that text again.
3. **Accessors used:** `metatype`, `type`, `key`, `fieldlist`,
   `get(encode('UTF-8', NFC($f)))`, `value` (preamble/comment), `parse_ok`
   (pass 2 only: a false value gives `Entry KEY does not parse correctly` and
   the entry is skipped; pass 1 ignores it, so partial entries are re-serialised
   and survive).
4. **Diagnostics.** stderr is redirected to a file for the whole
   `extract_entries`; afterwards each line containing `overriding existing
   definition of macro` is dropped, a line matching `error:` becomes a
   `biber_error("BibTeX subsystem: LINE")` (fatal at the end unless
   `--nodieonerror`), a line matching `warning:` a `biber_warn(...)`. Other
   lines (`invalid token near line ...`, `semantic error; ...`) are silently
   dropped. File names in diags are the temp file's (`<md5>_<pid>.utf8`).
5. **Lists and names.** `split_list(NFC($value), $sep, undef, undef, undef,
   {binmode=>'utf-8', normalization=>'NFD'})` with `namesep`/`listsep`
   (default `and`); names: `Text::BibTeX::Name->new({...NFD}, NFC($name))`
   after biber's own sanitising (trim, `\s+|\\\s` → space, `{{+X}+}` → `{X}`),
   then `format` with four single-part formats (`l`,`f`,`v`,`j`), each with
   `set_options(PART, 0, BTJ_MAYTIE, BTJ_NOTHING)` (§7).
6. **Crashes.** A SIGSEGV in libbtparse is caught by biber (`Caught signal:
   SEGV`, exit 1); SIGABRT and `exit(1)` are not caught.

The crate does no Unicode normalisation; callers pass NFD text to the parser
and NFC text to `split_list`/`split_name`, as biber does. Text::BibTeX returns
all strings in NFD; the crate returns substrings of its input.

---

## 2. Lexical structure

Input is a byte string (UTF-8). All syntactically significant characters are
ASCII; every byte ≥ 0x80 is an ordinary name/junk byte. Lines are counted from
1; the counter advances when a `\n` is consumed (but see §2.7 for dropped
bytes).

### 2.1 Lexer modes

The lexer has four modes. *Whitespace* below means space, `\t`, `\n`, `\r`
(C-0D, C-TAB); VT and FF are *not* whitespace outside strings (C-VT).

| Mode | Entered | Meaning |
|---|---|---|
| Top | at start; after an entry-closing `}`/`)` in InEntry; after a comment body; when the parser "skips to next @" (§3.3) | between entries |
| AfterAt | after `@` in Top | expecting the entry type |
| AfterType | after a NAME in AfterAt | expecting `{`/`(` |
| InEntry | after `{`/`(` in AfterType (non-comment) | entry body |

### 2.2 Tokens outside Top mode

| Input | Token / effect |
|---|---|
| maximal run of *name bytes* | NAME, or NUMBER if all ASCII digits (V-004, V-005: `1999a` is a NAME). In AfterAt a NAME switches to AfterType and fixes the *metatype* (§2.4); a NUMBER does not (N-008) |
| `=` `#` `,` | EQUALS, HASH, COMMA |
| `{` in AfterType | ENTRY_OPEN → InEntry (comment entries: §2.5) |
| `{` in InEntry | a braced STRING (§2.6) |
| `(` in AfterType | ENTRY_OPEN → InEntry |
| `{` or `(` in AfterAt, `(` in InEntry | warning `"X" in strange place -- should get a syntax error`, token ENTRY_OPEN, mode unchanged (M-P1-LB, L-020, L-049, N-008) |
| `}` or `)` in InEntry | ENTRY_CLOSE → Top; if it does not match the opener: warning `entry started with "{", but ends with ")"` (V-022, KEYMID-29) — note `)` closes a `{` entry |
| `}` or `)` in AfterAt/AfterType | warning `"}" in strange place -- should get a syntax error`, token ENTRY_CLOSE, mode unchanged (M-P1-RB, M-P2-RB, M-P12-BS) |
| `"` in InEntry | a quoted STRING (§2.6) |
| `"` in AfterAt / AfterType (non-comment) | warning `start of string seen at weird place`, then a quoted STRING (M-P1-DQ, M-P2-DQ, N-007) |
| `"` in AfterType of a comment | error `comment entries must be delimited by either braces or parentheses`, a token printed as `"`, mode → Top (M-P12-DQ) |
| `%` | comment to end of line, `\n` included (V-028, L-005..L-007); at end of input without `\n`: diag `invalid token near line N (text was '%...')` with the rest of the input (KEYMID-25, V-029, V-030) |
| invalid byte: `'`, `\`, ASCII controls other than `\t\n\r` (incl. VT, FF), DEL | diag `invalid token near line N (text was 'C')` (C is the byte itself, C-01..C-7F); the byte **and the following byte** are discarded (KEYSTART-27: `'ab` → key `b`; L-014: `a',` loses the comma); a discarded `\n` is not counted (E in `c1`, N-019) |

*Name bytes* are all bytes except whitespace, ASCII controls, DEL and
`" # % ' ( ) , = { } \` (namechars scan: KEYMID/KEYSTART/FIELD/TYPE/MACRO-xx).
`@` is a name byte outside Top mode (KEYMID-40, L-019 `@\n@misc` → type
`@misc`, L-023, M-P8-AT).

### 2.3 Top mode

| Input | Effect |
|---|---|
| `@` | if the junk counter is > 0: warning `N characters of junk seen at toplevel` (line = current line, i.e. the `@`'s; N-003) and reset; token AT, mode → AfterAt |
| whitespace (incl. `\t`) | skipped, not junk (T-008, L-009) |
| `%` at the start of a run (not directly after a junk byte) | comment to end of line. At end of input without `\n`: if the rest contains a space or tab → `invalid token` diag (V-030, T-005, T-009); otherwise it is junk, silently (T-010, `fuzz` g1-1820) |
| any other byte, and `%` directly after a junk byte | junk: counter += 1 per **byte** (N-009: `äöü` = 6) (T-007: `2020% x` is 6 junk bytes) |
| end of input | EOF; pending junk is never reported (L-010, FIELDSTART-7D) |

A `%` comment in Top mode hides an `@` on the same line (V-029, T-003).

### 2.4 Entry types

The type is the NAME after `@` (whitespace and `%` comments allowed between,
V-048), lower-cased with **ASCII** case folding (V-025, U-1). Metatype:
`comment`, `preamble`, `string` (ASCII-case-insensitive) → COMMENT, PREAMBLE,
MACRODEF; anything else → REGULAR (L-024, L-022 `@1misc`).

### 2.5 Comment bodies

After `@comment` (any case) in AfterType, `{` or `(` starts the comment body,
lexed directly as one STRING token; mode → Top afterwards (V-036, L-028).
- `{...}`: like a braced string (§2.6).
- `(...)`: ends at the first `)` at parenthesis depth 0, whatever the brace
  depth; if braces are open then: error `unbalanced braces: too many {'s`
  (L-030); a `}` at brace depth 0 gives `unbalanced braces: too many }'s`.
  Parentheses nest (V-036 `x (y) z`).
- End of input inside the body: EOF (M-P12-LB).
- `@comment free text` is a syntax error (V-037, §3.2).
The value is the body without delimiters, post-processed like a field (§4.2).

### 2.6 Strings

Braced string: starts at `{` (depth 1), `{`/`}` change the depth, ends at the
`}` returning to depth 0. `"`, `@`, `%`, `\`, `(`, `)` are ordinary
(V-002, V-013, V-014, V-011: `\{` does not escape).
Quoted string: starts at `"`, depth 0; `{`/`}` change the depth; a `"` at depth
0 ends it (V-002, V-012: `\"` does not escape); a `}` at depth 0 gives error
`unbalanced braces: too many }'s` and lexing continues (M-P3-DQ).
At end of input inside a string, the string is discarded and the token is EOF
(L-033, L-034, V-003). No diagnostic for the unterminated string itself.

**Whitespace inside strings.** The token text keeps all characters except:
- `\r` is deleted (WS-11, WS2-8 in `c1`/`enum_tabs`-style probes).
- The text is divided into *pieces* at `{`, `}`, `"`, `(`, `)` and `\n`
  (each a piece boundary). A piece that starts with `\n` (a *newline piece*)
  extends to the next boundary character **or backslash**; in it `\n`, `\t`,
  VT and FF all become spaces (WS-4..WS-6, WS3-1, WS3-7, `enum_tabs`). A piece
  consisting of exactly one tab becomes a space (`enum_tabs`: `{\t}`, `NOTC`,
  `OCTN`; but `{\t\t}`, `{\t\r}`, `x\t` stay). After a backslash the rest of
  the piece is ordinary (no conversion; `{\n c\\\td}` keeps the tab).
- Tabs elsewhere are kept (WS-1, WS-3, WS-7).

**Runaway warning.** When a string (or comment body) contains a newline after
which the next line matches `[ \t]*[A-Za-z][A-Za-z0-9:+/'.-]*[ \t]*=`, btparse
warns `possible runaway string started at line S` (S = line of the opening
delimiter; diag line = the matching line). At most once per string (R-001..
R-016, T-012..T-016, `fuzz` g1-158, g3-18824).

### 2.7 Line numbers

Diags carry the lexer's current line when they are emitted. For syntax errors
this is the line at the *end* of the offending token (N-001); for `undefined
macro` the line of the macro token (N-002); for junk the line of the `@`
(N-003); `at end of input` uses the last line (N-004). Bytes discarded after an
invalid token are not counted, so a discarded `\n` shifts all later line
numbers by one (N-019).

---

## 3. Grammar and error handling

btparse's parser behaves like an LL(1) recursive-descent parser in the PCCTS
style. One call of `Text::BibTeX::Entry->new` parses one *entry*:

```
entry    : AT NAME body
body     : STRING                       -- comment entries (and see §3.5)
         | ENTRY_OPEN contents ENTRY_CLOSE
contents : <regular>  (NAME|NUMBER) COMMA fields
         | <macrodef> fields
         | <preamble> value
fields   : [ field { COMMA [ field ... ] } ]     -- empty allowed; trailing comma allowed
field    : NAME EQUALS value
value    : simple { HASH simple }
simple   : STRING | NUMBER | NAME           -- NAME = macro reference
```

(V-023 trailing comma; V-038 `@misc{k}` needs the comma; V-039 `@misc{k,}`;
V-024 `,,`; V-043 the key cannot be a string; V-045, N-013, N-014.)

### 3.1 Iteration

- Before the first entry the first token is read. If it is EOF the parser
  reports `syntax error: at end of input, expected "@"` and resynchronises
  (§3.4), and no entry is returned (T-001..T-004; biber never parses files
  without `@`).
- An entry is returned for every `@` that is followed by a NAME. If the
  lookahead after an entry is EOF, or an `@` that is the last byte of the input,
  parsing ends (`fuzz` g2-7118, g3-17941: Text::BibTeX stops at end of stream).
- **Stop**: if the type NAME is missing (`@{`, `@,`, `@123`, …) the parser
  reports `found "X", expected name (...)`, resynchronises to the next AT, and
  Text::BibTeX returns *false*: biber's loop ends and the **rest of the file
  is silently ignored** (M-P1-*, L-020, N-008). The same happens if an entry
  starts with a token other than AT (only reachable after unusual recovery).
- Diagnostics are printed in this order per entry: those emitted while parsing
  (including those caused by reading the lookahead token *after* the closing
  delimiter, e.g. junk warnings for the next entry, L-011), then
  post-processing diags (undefined macros, macro overrides) (M-P7-NM, N-002).

### 3.2 Syntax error messages

`syntax error: found "X", expected E` (or `syntax error: at end of input,
expected E` when LA is EOF), where X is the token text (NAME/NUMBER text; a
STRING with its delimiters, after §2.6 processing, N-001; `{`, `}`, `(`, `)`,
`=`, `#`, `,`, `@`; see §8.3 for `%`) and E is one description or
`one of: A, B or C`, listed in this order:

| token | description |
|---|---|
| NUMBER | `number` |
| NAME | `name (entry type, key, field, or macro name)` |
| ENTRY_OPEN | `start of entry ("{" or "(")` |
| ENTRY_CLOSE | `end of entry ("}" or ")")` |
| STRING | `quoted string ({...} or "...")` |
| COMMA, EQUALS, AT | `","`, `"="`, `"@"` |

Where the errors arise:

| Position | Expected set | Recovery set (§3.4) |
|---|---|---|
| type | NAME | {AT} → Stop |
| body start | ENTRY_OPEN, STRING | {AT} |
| contents start, token not in {NUMBER, NAME, ENTRY_CLOSE, STRING} | NUMBER, NAME, ENTRY_CLOSE, STRING (all metatypes!) | {ENTRY_CLOSE} |
| contents start, token in that set but wrong for the metatype (regular: `}`/STRING; macrodef: NUMBER/STRING; preamble: `}`) | — message is just `found "X"` | {ENTRY_CLOSE} |
| after the key | COMMA | {ENTRY_CLOSE} |
| fields: not NAME/ENTRY_CLOSE | NAME, ENTRY_CLOSE | {ENTRY_CLOSE} |
| field: after NAME | EQUALS | {COMMA, ENTRY_CLOSE} |
| simple value | NUMBER, NAME, STRING | {HASH, COMMA, ENTRY_CLOSE} |
| body end | ENTRY_CLOSE, message suffix ` (skipping to next "@")` | see §3.3 |

(Experiments: the M-P*-* matrix, L-036..L-047, V-024, V-037, V-038, V-040,
V-043, N-012, N-015.)

Loops do not check what follows them: `title = {x} note = {y}` fails only at
the body end (L-038). Field names starting with a digit are NAMEs (`7m`) and
give `syntax error: invalid field name "7m": cannot start with digit` (printed
*before* the next token is read; the field is still recorded and parsing goes
on, parse_ok false; M-P10-DG, M-P5-DG, N-006, `fuzz` g3-10018); a NUMBER in
field position is the generic error (L-045).

### 3.3 Skipping to the next `@`

On the body-end error the lexer is switched to Top mode **at its current
position** (just after the offending token), and recovery then consumes tokens
until AT/EOF. Everything up to the next `@` is junk and is counted (L-001: 14
characters; M-P8-NM vs M-P8-LB). An `@misc{...}` swallowed inside a NAME or
STRING before that point is lost (V-011, L-001).

### 3.4 Resynchronisation

Recovery uses one flag, *consumed*, that is global to the process (initially
true) and persists across entries and files (N-020, `fuzz` g3-16251). To
resynchronise to a set S:
1. if *consumed* is false: consume one token and return (*consumed* stays false);
2. else if LA ∈ S or LA is EOF: set *consumed* := false and return;
3. else consume tokens until LA ∈ S or EOF, set *consumed* := true.

Consequence (N-020): after one error that left *consumed* false, the next
error's recovery skips just one token, which changes later messages.

### 3.5 Fatal and odd cases

- A STRING right after the type of a **regular** (or macrodef) entry
  (`@misc"x"`): btparse prints `semantic error; failed predicate: '
  metatype == BTE_COMMENT '` and `fatal error: bt_postprocess_field: invalid
  AST node (not a field)` and **exits the process** (status 1; N-007). biber
  dies.
- For a **preamble** the same input prints the `semantic error` line and
  continues; the string becomes the preamble value (`fuzz` g1-16).
- `parse_ok` is false after any syntax error or `error:` diag; `invalid token`
  and warnings do not affect it (KEYMID-27, L-013, M-P7-SQ).

---

## 4. Values, post-processing and macros

### 4.1 Field values

A field value is the concatenation of its simple values: string contents
without delimiters, number text, or a macro's expansion (§4.3). Simple values
that failed to parse contribute nothing (L-036 `title = # {x}` → `x`; L-037).
The value is **undef** (`Entry::get` returns undef; biber's pass 1 writes `{}`)
if no simple value was parsed (L-039, M-P6-EQ) or if the value is exactly one
undefined macro (V-005, V-032); `undefd # undefd` and `{} # undefd` give `""`
(L-002).

### 4.2 Whitespace post-processing

For regular-entry fields, preamble and comment values: runs of **spaces**
(only U+0020) become one space and leading/trailing spaces are removed, on the
final concatenated string (V-001, V-006, V-007, WS-8, WS-16). Tabs, VT, FF,
NBSP, U+2003 are untouched (WS-1, WS-3, WS-7, WS-9, WS-14, WS-15).
`@string` values are **not** collapsed (V-008 `"Bar  baz"`, MD-1).

### 4.3 Macro expansion

Macro names are NAMEs, looked up with ASCII case folding (V-008, U-1). An
undefined macro gives `warning: undefined macro "NAME"` with the name *as
written* (U-1: `ÉX`) and expands to nothing.

### 4.4 `@string`

For each definition in order: expand its value (warnings as usual), and if the
value had at least one parsed simple value, define the macro (lower-cased
name) with the expanded text (undef → empty, MD-2), warning `overriding
existing definition of macro "NAME"` if it existed (V-046, V-047, N-005; biber
drops these). A definition whose value failed to parse defines nothing (`fuzz`
g2-1925, g2-1962). After all definitions of the entry, the field values handed
back are **expanded again** with the updated table (with warnings again for
still-undefined macros): `@string{m = "a" # m # "b"}` defines `m="ab"` but
reports `"aabb"` (`fuzz` g1-17, g1-55, g2-1962; macro probes in §11).

The table is global: definitions persist across entries, files and refsections
until `delete_all_macros` (`fuzz` op `files`).

### 4.5 Fields, duplicates, keys

Field names are ASCII-lower-cased (V-025); keys keep their case (V-025) and
may contain any name bytes incl. `@`, non-ASCII, start with digits (V-042,
KEYSTART-*). Duplicate fields are all kept in `fieldlist` order, and `get`
returns the **last** value for every occurrence (V-026; biber's pass 1 thus
writes the last value once per occurrence).

### 4.6 Preamble and comment

`@preamble` value: like a field (V-035, N-017); Perl's `value` returns `""`
for undef (L-026, H in `c1`). Comment value: §2.5, collapsed (V-036, L-028).

---

## 5. `split_list`

`split_list(S, D)` (S in NFC, D = biber's `namesep`/`listsep`):

- A delimiter occurrence is at byte position i with: brace depth 0; i ≥ 1 and
  `S[i-1]` is a space; `S[i .. i+|D|]` equals D after ASCII-lower-casing the
  **string** side only (D itself is not lower-cased: `list AND` never matches;
  `ünd`/`ÜND` do not match case-insensitively) (split_list:1..4, probes in §11);
  and `S[i+|D|]` exists and is a space. Tabs and `~` do not count
  (split_list:19..23).
- The piece before the delimiter ends at i-1 (excluding the space); the next
  starts at i+|D|+1. Pieces are **not trimmed** (split_list:5 `A  and   B` →
  `"A "`, `"  B"`). The trailing space of one delimiter may be the leading
  space of the next.
- Empty input → empty list. Brace depth is tracked on `{`/`}`; an unmatched
  `}` gives `warning: unmatched '}' (ignoring)` (depth stays 0); depth > 0 at the
  end gives `warning: unmatched '{' (ignoring)` (split_list:31..33).
- A piece with stop < start (overlapping spaces, `A and and B`) is empty and
  gives `substring N, warning: empty substring` (N 1-based) (split_list:9).
  With a file/line/description, btparse would print `FILE, line L, DESC N,
  warning: empty DESC`; biber passes none.
- A piece with stop == start (` and B`, `A and `, two delimiters separated by
  exactly two spaces) makes btparse **abort** with `internal error: stop ==
  start for substring N` (N 0-based) (split_list2). Unreachable from biber for
  btparse-produced values (they are space-collapsed, §4.2) but reachable via
  sourcemaps.

## 6. `split_name`

Input: one name, NFC, normally already sanitised by biber (single spaces).

### 6.1 Tokens

Scan bytes with brace depth: at depth 0, space, tab, `\n`, `\r` and `,` end a
token, `,` additionally records a comma at position = number of tokens so far.
Braces belong to tokens; an unmatched `}` at depth 0 warns `warning: unmatched
'}' (ignoring)` and stays in the token; depth > 0 at the end warns
`warning: unmatched '{' (ignoring)`. Each brace warning is printed **twice**
(two passes; `namewarn`). `~` and `-` are not separators (names:76..79).
Special characters (`{\...}`) are ordinary braced text here.

### 6.2 Commas

Let n = number of tokens, c = comma positions.
- more than 2 commas: `warning: too many commas in name (removing extras)`,
  keep the first two (extra commas' tokens go to *first*) (names3:52..53,
  `enum_commas`);
- else if the last comma is at n (trailing): `warning: comma(s) at end of name
  (removing)`, remove all trailing commas (names3:40..45).

### 6.3 Lower-case tokens

A token is *lower-case* iff its first code point is in the btparse lower-case
set: ASCII `a`–`z` plus exactly General_Category=Ll **as of Unicode 6.1**
restricted to NFC-stable characters (`scan_lower`: exhaustive scan of all code
points; the Georgian Mkhedruli letters, Ll only since Unicode 11, are not
lower-case; Unicode ≥ 7 additions are not; Lt/Lm/Lo/Nl/So never). Tokens
starting with `{`, `\`, a digit or punctuation are never lower-case, including
special characters like `{\'e}cole`, `{\relax d}e` (names:29..55). The table is
`crates/btparse/src/lower_table.rs` (from `lower_ranges.txt`). Locale does not
matter.

### 6.4 Parts

- **0 commas** (`First von Last`): if no token is lower-case, last = final
  token, first = the rest. Else von starts at the first lower-case token vs and
  runs while tokens are lower-case (end ve); if ve = n then ve := n-1. first =
  [0,vs), von = [vs,ve), last = [ve,n) (names:14..21, names3:66..76).
- **1 comma** (`von Last, First`) at c1, **2 commas** (`von Last, Jr, First`)
  at c1, c2: if c1 > 0 and token 0 is lower-case, the von run starts at 0 and
  extends while tokens are lower-case **without stopping at the comma** (a
  btparse bug); if it ends exactly at c1 it is shortened by one. von = [0,ve),
  last = [ve,c1) (empty if ve ≥ c1), jr = [c1,c2), first = [c1 or c2, n). Hence
  `a, b` → von `a b`, first `b`, last empty (names2:1..21, `enum_commas`).
  Only a *leading* lower-case run forms von in comma forms (`A b c, D`).

### 6.5 Irregular commas and crashes

A leading comma or two commas with no token between them are *irregular*
(btparse's results are not fully characterised; `enum_commas`). Modelled: with
two commas, c1 = 0 → c1 := c2, then c1 = c2 → c2 := 2 (`Doe, , John` → last
`Doe`, jr `John`; confirmed in biber's `.bbl`). btparse **segfaults** on: one
leading comma with tokens (`, John`, `,A B`), irregular commas combined with
trailing-comma removal (`Doe,,`, `Doe, ,`), and other cases (`enum_commas`
CRASH lines). The crate returns `NameAnomaly::Crash` for the modelled crash
patterns, `IrregularCommas`/`UnbalancedBraces` where results are best effort.

## 7. Formatting (`format` with `BTJ_MAYTIE`)

### 7.1 Joining

One part, tokens t0..tk-1: k=1 → t0; k=2 → `t0~t1`; k≥3 → t0, then `~` if
t0 is *short* else a space, then the middle tokens separated by spaces, then
`~` before the last token (names:3..8, names2:22..60: `A~B C D~E`,
`Abc D E F~G`, `Ab~D E F~G`). Tokens are copied verbatim (braces, `~` kept:
`A~~B~C`). Empty part → empty string. Pre/post texts are empty for biber's
formats.

### 7.2 Short tokens

A token is short if its length < 3, where length counts code points, not
braces; a group `{\...}` at brace depth 0 counts as one character (special
character); other braced text counts its characters (`{\'E}b` = 2, `{\ss}xy` =
3, `{{\'E}}` = 3, `{x\'E}` = 4, `\'Eb` = 4, `{}{}{}` = 0, `李李李` = 3)
(names2:22..60, names3:6..22).

## 8. Diagnostics

### 8.1 Formats

| Kind | Text |
|---|---|
| warning | `FILE, line N, warning: MSG` |
| error | `FILE, line N, error: MSG` |
| syntax error | `FILE, line N, syntax error: MSG` |
| invalid token | `invalid token near line N (text was 'T')` (no file) |
| names/lists | `warning: ...`, `substring N, warning: empty substring`, `internal error: ...` (no file/line) |
| other | `semantic error; failed predicate: '   metatype == BTE_COMMENT '`, `fatal error: bt_postprocess_field: invalid AST node (not a field)` |

### 8.2 Catalogue

Warnings: `N characters of junk seen at toplevel`; `undefined macro "M"`;
`overriding existing definition of macro "M"`; `"C" in strange place --
should get a syntax error`; `entry started with "O", but ends with "C"`;
`start of string seen at weird place`; `possible runaway string started at line
S`. Errors: `unbalanced braces: too many }'s`; `unbalanced braces: too many
{'s`; `comment entries must be delimited by either braces or parentheses`.
Syntax errors: §3.2. Name/list warnings: §5, §6.

### 8.3 `%` in offending tokens (format-string bug)

The `found "X"` text is passed to printf **as part of the format string**.
`%%` prints `%`; `%` + flags/width + `%` prints a padded `%`; `%d` etc. print
an arbitrary number, `%s` an arbitrary string or **crashes** (P-001..P-007,
`fuzz` g2-4200). The crate emulates `%%`, padded `%`, unknown conversions
(the character itself) and prints `0`/`(null)` for integer/string conversions;
garbage-printing conversions cannot match. biber 2.22 dies with SEGV on e.g.
`title {50%s}` (confirmed with O1).

## 9. biber's round trip (crate `compat`)

Pass 1 writes, per entry: regular `@TYPE{KEY,\n` + `  F = {V},\n` per
`fieldlist` occurrence (V = decoded `get(F)`, undef → empty) + `\n}\n\n`;
preamble `@PREAMBLE{"V"}\n`; comment `@COMMENT{V}\n`; macrodef
`@STRING{` + `N = {V}` per definition **without separators** + `}\n` (biber
bug B1). Pass 2 parses that text with the same global state. Decoded values
with unbalanced braces change pass 2 (B2); `compat::braces_round_trip` detects
them. The pipeline should run both passes through the parser (cheap) to
reproduce all artefacts exactly.

## 10. Known gaps

- printf garbage (§8.3) and process crashes (§3.5, §5, §6.5) are reported, not
  reproduced; irregular-comma names and unbalanced-brace names are best effort.
- Lexer state leaking between files after lexical errors (seen once as an
  abort in an un-forked probe run) is not modelled.
- Not characterised: `@string`/regular entries whose body is a STRING (fatal
  anyway), very long tokens, NUL bytes (btparse asserts: WS-18).

## 11. Differential testing

`gen.pl SEED N` generates random requests (mostly-valid `.bib` text with
character mutations, multi-file sequences, names, name lists); `vectors.pl`
runs them through Text::BibTeX exactly as biber does (forked per request,
crashes recorded); `cargo test -p btparse --test fuzz_vectors` compares.

<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-041, BB-108, BB-141, BB-147, BB-038; pre-check for BB-123/124 (docs/biber-bugs.md). -->

**Title:** Name parsing: comma checks count braced commas and skip names that merely start and end with braces; `family = Doe` (spaces around `=`) drops the name; base-less names get the wrong `uniquename` part

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biber 2.22 and current `dev` (1686b9c), with `\nocite{*}`.

**1. The comma checks in `_name` look at the raw string.** They are skipped for any name matching `\A\{\X+\}\z`, i.e. any name that starts with `{` and ends with `}`, even if those are two different brace groups; and when they do run, they count every comma, including commas inside braces. So:

| `author = {…}` | biber 2.22 / `dev` |
|---|---|
| `{Doe, Inc.}, J, Jr` (valid: von Last, Jr, First) | `ERROR - Name "{Doe, Inc.}, J, Jr" has too many commas, skipping entry` |
| `Doe, J, K, L` | too many commas, entry skipped (as intended) |
| `{Doe}, J, K, {L}` | accepted without a message: `family={{Doe}}`, `suffix={J}`, `given={K\bibnamedelima {L}}` |
| `{Doe},, {L}` | accepted (TeX Live binary: `suffix={{L}}`, no given name), or a crash in `Text::BibTeX::Name` |

The "consecutive commas" check exists because empty parts make libbtparse crash, but it only looks for `,,`: `Doe, , John`, `Doe, ,` and `, John` pass the check. With Text::BibTeX 0.91 (TeX Live) `, John` ends with `FATAL - Caught signal: SEGV` and an empty `.bbl`, and `Doe, , John` silently puts `John` in the suffix; with Text::BibTeX 0.89 all three segfault. (The crashes themselves are Text::BibTeX bugs, but biber's check is meant to prevent them.)

**2. In the extended name format, spaces around `=` make a part invalid, and the split is at the last `=`.** The parts are split with `m/^(.+)\s*$xnamesep\s*(.+)$/x`, where the greedy `(.+)` keeps the space before `=` in the part name and runs to the *last* separator:

```bibtex
@misc{x1, title={1}, author = {given=Ann, family={Barnes Noble} and family = Smith, given = John and given=A=B, family=C}}
```

```
WARN - Invalid namepart option 'family ' found in extended name format name 'author' in entry 'x1', ignoring
WARN - Invalid namepart option 'given ' found in extended name format name 'author' in entry 'x1', ignoring
WARN - Invalid namepart option 'given=a' found in extended name format name 'author' in entry 'x1', ignoring
```

The second name is then written to the `.bbl` with no name parts at all (on `dev` such a name is also a trigger for the endless uniqueness loop of #520), and the third loses its given name `A=B`.

**3. Names without a base part get the wrong uniquename schema.** In `create_uniquename_info`/`process_namedis`, the base level is always pushed onto `$namestrings`, but only pushed onto `$namedisschema` when there are base parts, so for a name such as `given=Plato` the two arrays are offset by one. With `uniquename=init`:

```bibtex
@misc{a, title={1}, author = {given=Plato}}
@misc{b, title={2}, author = {given=Paul}}
@misc{c, title={3}, author = {given=Peter}}
@misc{d, title={4}, author = {Doe, Peter}}
@misc{e, title={5}, author = {Doe, Paul}}
```

`d` and `e` get `un=0, uniquepart=base` (the initials are the same, so `init` can't distinguish them), but `a`, `b` and `c`, whose initials are just as identical, get `un=1, uniquepart=given`, i.e. "the given initial makes the name unique".

**4. (Question.) Braces around an extended-format part are dropped from the `.bbl`.** `author = {{Barnes Noble}, Ann}` gives `family={{Barnes Noble}}`, but `author = {given=Ann, family={Barnes Noble}}` gives `family={Barnes\bibnamedelimb Noble}`: the braces are gone from the `.bbl`, so neither the case protection nor the grouping survives. `parsename_x` computes the "stripped" flag and has the comment about passing a `strip` entry to the `Name` object, but doesn't pass it. `t/uniqueness-nameparts.t` expects `given={John}` → `given={John}`, so dropping the braces may be intended; but the manual says "Compound parts may be protected with braces" (`given={Jean Pierre}`), and that protection doesn't reach the `.bbl` either.

### Suggested fix

For 1–3 (only the comma check is changed for 1; the actual splitting is left to Text::BibTeX):

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -1877,7 +1877,8 @@
 
       $namestring .= $base;
       push $namestrings->@*, $base;
-      push $namedisschema->@*, ['base' => $baseparts] if defined($baseparts);
+      # Always add the base level, even if empty, to keep the schema aligned with $namestrings
+      push $namedisschema->@*, ['base' => $baseparts // []];
 
       # ... then add non-base parts by incrementally adding to the last disambiguation level
       foreach my $np (Biber::Config->getblxoption(undef, 'uniquenametemplate')->{$untname}->@*) {
--- a/lib/Biber/Input/file/bibtex.pm
+++ b/lib/Biber/Input/file/bibtex.pm
@@ -1283,17 +1283,19 @@
     else { # Normal bibtex name format
       # Check for malformed names in names which aren't completely escaped
       # Too many commas
-      unless ($name =~ m/\A\{\X+\}\z/xms) { # Ignore these tests for escaped names
-        my @commas = $name =~ m/,/g;
+      # Only commas outside braces separate name parts
+      (my $toplevel = $name) =~ s/(\{(?:[^{}]++|(?1))*\})/{}/g;
+      unless ($toplevel eq '{}') { # Ignore these tests for completely escaped names
+        my @commas = $toplevel =~ m/,/g;
         if ($#commas > 1) {
           biber_error("Name \"$name\" has too many commas, skipping entry '$key'", 1);
           $section->del_citekey($key);
           return 'BIBER_SKIP_ENTRY';
         }
 
-        # Consecutive commas cause Text::BibTeX::Name to segfault
-        if ($name =~ /,,/) {
-          biber_error("Name \"$name\" is malformed (consecutive commas): skipping entry '$key'", 1);
+        # Empty parts before or between commas cause Text::BibTeX::Name to segfault
+        if ($toplevel =~ /\A\s*,|,\s*,/) {
+          biber_error("Name \"$name\" is malformed (empty name part): skipping entry '$key'", 1);
           $section->del_citekey($key);
           return 'BIBER_SKIP_ENTRY';
         }
@@ -1904,7 +1906,8 @@
   my %namec;
   my %pernameopts;
   foreach my $np (split_xsv($namestr)) {# Can have x inside records so use Text::CSV
-    my ($npn, $npv) = $np =~ m/^(.+)\s*$xnamesep\s*(.+)$/x;
+    # Split at the first separator, trimming spaces around it
+    my ($npn, $npv) = $np =~ m/^\s*(.+?)\s*\Q$xnamesep\E\s*(.+?)\s*$/x;
     $npn = lc($npn);
 
     # per-name options
```

With this change: `{Doe, Inc.}, J, Jr` is accepted (`family={{Doe, Inc.}}`, `suffix={J}`, `given={Jr}`); `{Doe}, J, K, {L}` gives the "too many commas" error; `{Doe},, {L}`, `Doe, , John`, `Doe, ,` and `, John` give `ERROR - Name "…" is malformed (empty name part): skipping entry` instead of crashing or misparsing; the second and third names of `x1` parse as `family={Smith}, given={John}` and `family={C}, given={A=B}`; and `a`, `b`, `c` get `un=0, uniquepart=base` like `d` and `e`. All tests in `t/` pass on `dev` (1686b9c) with this patch.

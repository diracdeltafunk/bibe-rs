<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/525 (2026-10-25). Tracks BB-021 (docs/biber-bugs.md). -->

**Title:** Per-entry `singletitle`, `uniquetitle`, `uniquebaretitle`, `uniquework` and `uniqueprimaryauthor` in `options = {…}` are ignored

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

biblatex declares these five options with entry scope in the `.bcf` (`<bcf:optionscope type="ENTRY">`), so they can be set per entry with `options = {…}`. biber ignores the per-entry values: only the global and per-type settings take effect.

The cause: they are looked up with `getblxoption(undef, …)`. Per-entry options are stored per section (`{ENTRY}{$citekey}{$secnum}{$opt}` in `setblxoption`). With an undefined section, `getblxoption` uses its placeholder `"\x{10FFFD}"` for the section, so the per-entry value is never found. The other entry-scope options (`uniquename`, `uniquelist`, `presort`, `noinherit`, `nohashothers`, `nosortothers`, `noroman`) pass `$secnum` and work.

### Minimal example

```bibtex
@book{one, author = {Doe, John},    title = {Only Book},  date = {2000}}
@book{two, author = {Roe, Richard}, title = {Other Book}, date = {2001},
           options = {singletitle=false, uniquetitle=false}}
```

```latex
\documentclass{article}
\usepackage[style=authoryear, singletitle=true, uniquetitle=true]{biblatex}
\addbibresource{test.bib}
\begin{document}
\nocite{*}
\printbibliography
\end{document}
```

Both entries get `\true{singletitle}` and `\true{uniquetitle}`. Entry `two` should get neither.

### Suggested fix

Pass the section number at the ten call sites. Every one of these subs already has `$secnum` in scope.

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -2145,7 +2145,7 @@
   my $bee = $be->get_field('entrytype');
 
   if (my $lni = $be->get_labelname_info) {
-    if (Biber::Config->getblxoption(undef, 'uniqueprimaryauthor', $bee, $citekey)) {
+    if (Biber::Config->getblxoption($secnum, 'uniqueprimaryauthor', $bee, $citekey)) {
       my $nl = $be->get_field($lni);
       if ($logger->is_trace()) {# performance tune
         $logger->trace("Creating uniqueprimaryauthor information for '$citekey'");
@@ -2201,7 +2201,7 @@
   # singletitle
   # Don't generate information for entries with no labelname or labeltitle
   # Use fullhash as this is not a test of uniqueness of only visible information
-  if ($lni and Biber::Config->getblxoption(undef, 'singletitle', $bee, $citekey)) {
+  if ($lni and Biber::Config->getblxoption($secnum, 'singletitle', $bee, $citekey)) {
     $identifier = $self->_getfullhash($citekey, $be->get_field($lni), $dlist);
 
     # Skip due to ignore settings?
@@ -2218,7 +2218,7 @@
 
   # uniquetitle
   # Don't generate information for entries with no labeltitle
-  if ($lti and Biber::Config->getblxoption(undef, 'uniquetitle', $bee, $citekey)) {
+  if ($lti and Biber::Config->getblxoption($secnum, 'uniquetitle', $bee, $citekey)) {
     $identifier = $be->get_field($lti);
 
     # Skip due to ignore settings?
@@ -2230,7 +2230,7 @@
 
   # uniquebaretitle
   # Don't generate information for entries with no labeltitle and with labelname
-  if ($lti and not $lni and Biber::Config->getblxoption(undef, 'uniquebaretitle', $bee, $citekey)) {
+  if ($lti and not $lni and Biber::Config->getblxoption($secnum, 'uniquebaretitle', $bee, $citekey)) {
     $identifier = $be->get_field($lti);
 
     # Skip due to ignore settings?
@@ -2243,7 +2243,7 @@
   # uniquework
   # Don't generate information for entries with no labelname and labeltitle
   # Should use fullhash this is not a test of uniqueness of only visible information
-  if ($lni and $lti and Biber::Config->getblxoption(undef, 'uniquework', $bee, $citekey)) {
+  if ($lni and $lti and Biber::Config->getblxoption($secnum, 'uniquework', $bee, $citekey)) {
     $identifier = $self->_getfullhash($citekey, $be->get_field($lni), $dlist) . $be->get_field($lti);
 
     # Skip due to ignore settings?
@@ -4118,7 +4118,7 @@
   my $be = $bibentries->entry($citekey);
   my $bee = $be->get_field('entrytype');
 
-  if (Biber::Config->getblxoption(undef, 'singletitle', $bee, $citekey)) {
+  if (Biber::Config->getblxoption($secnum, 'singletitle', $bee, $citekey)) {
     my $sn = $dlist->get_entryfield($citekey, 'seenname');
     if (defined($sn) and $dlist->get_seenname($sn) < 2 ) {
       $dlist->set_entryfield($citekey, 'singletitle', 1);
@@ -4142,7 +4142,7 @@
   my $be = $bibentries->entry($citekey);
   my $bee = $be->get_field('entrytype');
 
-  if (Biber::Config->getblxoption(undef, 'uniquetitle', $bee, $citekey)) {
+  if (Biber::Config->getblxoption($secnum, 'uniquetitle', $bee, $citekey)) {
     my $ut = $dlist->get_entryfield($citekey, 'seentitle');
     if (defined($ut) and $dlist->get_seentitle($ut) < 2 ) {
       $dlist->set_entryfield($citekey, 'uniquetitle', 1);
@@ -4166,7 +4166,7 @@
   my $be = $bibentries->entry($citekey);
   my $bee = $be->get_field('entrytype');
 
-  if (Biber::Config->getblxoption(undef, 'uniquebaretitle', $bee, $citekey)) {
+  if (Biber::Config->getblxoption($secnum, 'uniquebaretitle', $bee, $citekey)) {
     my $ubt = $dlist->get_entryfield($citekey, 'seenbaretitle');
     if (defined($ubt) and $dlist->get_seenbaretitle($ubt) < 2 ) {
       $dlist->set_entryfield($citekey, 'uniquebaretitle', 1);
@@ -4190,7 +4190,7 @@
   my $be = $bibentries->entry($citekey);
   my $bee = $be->get_field('entrytype');
 
-  if (Biber::Config->getblxoption(undef, 'uniquework', $bee, $citekey)) {
+  if (Biber::Config->getblxoption($secnum, 'uniquework', $bee, $citekey)) {
     if ($dlist->get_entryfield($citekey, 'seenwork') and
         $dlist->get_seenwork($dlist->get_entryfield($citekey, 'seenwork')) < 2 ) {
       if ($logger->is_trace()) { # performance tune
@@ -4222,7 +4222,7 @@
   my $be = $bibentries->entry($citekey);
   my $bee = $be->get_field('entrytype');
 
-  if (Biber::Config->getblxoption(undef, 'uniqueprimaryauthor', $bee, $citekey)) {
+  if (Biber::Config->getblxoption($secnum, 'uniqueprimaryauthor', $bee, $citekey)) {
     if ($dlist->get_entryfield($citekey, 'seenprimaryauthor') and
         $dlist->get_seenpa($dlist->get_entryfield($citekey, 'seenprimaryauthor')) < 2 ) {
       if ($logger->is_trace()) { # performance tune
```

With this change, entry `two` loses both flags and entry `one` is unchanged.

Tested on biber 2.22 built from source. With this patch and the five others I'm filing alongside it, all 1,275 tests in `t/` pass. The code is unchanged on `dev` (d6e35e7), where the patch applies cleanly.

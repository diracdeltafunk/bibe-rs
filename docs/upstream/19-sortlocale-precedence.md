<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-043 and BB-044 (docs/biber-bugs.md). -->

**Title:** `--sortlocale` doesn't override a per-sortset `locale`, and `sortinithash` ignores `--sortlocale`

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

The manual (§ "Sorting") gives the locale precedence for collating a field as `--collate-options` locale → `--sortlocale` → per-sortset `locale` → per-template `locale` → global `sortlocale`, and the comment in `sort_list` says the same. Two places don't follow it.

**1. A per-sortset `locale` wins over `--sortlocale` (BB-043).** `sort_list` makes a separate collator whenever the sortset's locale differs from `$thislocale`, without checking whether `--sortlocale` was given:

```bibtex
@book{s1, title = {Apple}}
@book{s2, title = {Äpple}}
@book{s3, title = {Zebra}}
```

```latex
\documentclass{article}
\usepackage[sorting=tl]{biblatex}
\DeclareSortingTemplate{tl}{
  \sort[locale=swedish]{\field{title}}
}
\addbibresource{test.bib}
\begin{document}
\nocite{*}
\printbibliography
\end{document}
```

`biber --sortlocale=en_US test` still sorts with Swedish rules: `s1, s3, s2` (Äpple after Zebra). With `en_US` it should be `s1, s2, s3`.

**2. `sortinithash` uses only the template/global locale (BB-044).** The `sortinit` collator (Biber.pm:1084, 1119, 1219) is created from the sorting template's locale only. With the same `.bib`, `sorting=nyt` and `biber --sortlocale=sv_SE test`, the list is sorted as Swedish (`s1, s3, s2`), but `Ä` gets the same `sortinithash` as `A` (`2f401846…`), because the hash is computed with the `.bcf` locale. The hash is what biblatex uses to decide where `\bibinitsep` goes and for initial-letter headings, so these then disagree with the sort order. In the opposite case (`.bcf` locale Swedish, `--sortlocale=en_US`), `Apple` and `Äpple` are adjacent but get different hashes.

Same behaviour on biber 2.22 and current `dev` (1686b9c).

A related, minor documentation point (BB-079): the manual says biber *warns* when it overrides a locale's tailoring with user collation options (e.g. Danish `upper_before_lower`, Thai `variable`), but `Biber::UCollate` only logs an INFO line.

### Suggested fix

For (1), `sort_list` should not make a separate collator for a sortset's `locale` when `--sortlocale` is set, i.e. add `and not Biber::Config->getoption('sortlocale')` to the condition `if (defined($sl) and $sl ne $thislocale)`. For (2), use `--sortlocale` for the `sortinit` collator too:

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -1081,7 +1081,7 @@
     # Potentially, the locale could be different for the first field in the sort spec in which
     # case that might give wrong results but this is highly unlikely as it is only used to
     # determine sortinithash in DataList.pm and that only changes \bibinitsep in biblatex.
-    $datalist->set_sortinit_collator(Unicode::Collate::Locale->new(locale => Biber::Config->getblxoption(undef, 'sortingtemplate')->{$datalist->get_sortingtemplatename}->{locale}, level => 1));
+    $datalist->set_sortinit_collator(Unicode::Collate::Locale->new(locale => Biber::Config->getoption('sortlocale') || Biber::Config->getblxoption(undef, 'sortingtemplate')->{$datalist->get_sortingtemplatename}->{locale}, level => 1));
 
     if ($logger->is_debug()) {# performance tune
       $logger->debug("Adding datalist of type '$ltype' with sortingtemplate '$lstn', sortingnamekeytemplatename '$lsnksn', labelprefix '$lpn', uniquenametemplate '$luntn', labelalphanametemplate '$llantn' and name '$lname' for section $lsection");
@@ -1116,7 +1116,7 @@
       $datalists->add_list($datalist);
       # See comment above
 
-      $datalist->set_sortinit_collator(Unicode::Collate::Locale->new(locale => Biber::Config->getblxoption(undef, 'sortingtemplate')->{$datalist->get_sortingtemplatename}->{locale}, level => 1));
+      $datalist->set_sortinit_collator(Unicode::Collate::Locale->new(locale => Biber::Config->getoption('sortlocale') || Biber::Config->getblxoption(undef, 'sortingtemplate')->{$datalist->get_sortingtemplatename}->{locale}, level => 1));
     }
   }
 
@@ -1216,7 +1216,7 @@
       $datalist->set_section($secnum);
       $self->datalists->add_list($datalist);
       # See comment for same call in .bcf instantiation of datalists
-      $datalist->set_sortinit_collator(Unicode::Collate::Locale->new(locale => Biber::Config->getblxoption(undef, 'sortingtemplate')->{$datalist->get_sortingtemplatename}->{locale}, level => 1));
+      $datalist->set_sortinit_collator(Unicode::Collate::Locale->new(locale => Biber::Config->getoption('sortlocale') || Biber::Config->getblxoption(undef, 'sortingtemplate')->{$datalist->get_sortingtemplatename}->{locale}, level => 1));
     }
   }
 
```

With this change, example 1 sorts `s1, s2, s3`, and in example 2 `Ä` gets its own `sortinithash` (`db0c88ee…`). All tests in `t/` pass on `dev` (1686b9c) with this patch.

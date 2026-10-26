<!-- Draft upstream issue for plk/biber. Not yet filed. Tracks BB-155, BB-156, BB-153, BB-154, BB-056, BB-047, BB-162, BB-046, BB-161, BB-190 (docs/biber-bugs.md). -->

**Title:** Tool mode / `--output-format=bibtex` loses or corrupts data: `@comment`s of all but the last file, `@preamble`s, `XX` dates, literal annotations, zero values, seasons, short years; `--output-safechars` recodes keys and URLs

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

`biber --tool` is meant to re-serialise a `.bib` file, but several kinds of data are lost or written in a form that biber reads back differently. With

```bibtex
% r1.bib
@comment{Comment from r1}
@preamble{"\newcommand{\foo}{x}"}
@misc{a1, author={Doe, J}, title={T}, date={1999-XX}, author+an={1="Lit"}, options={useprefix=true,uniquename=false,skipbib=true}}
@misc{a2, title={Zeros}, date={0000}, volume={0}}
@misc{a3, title={Times}, date={2020-01-01T00:30:00/2020-01-02T00:00:00}}
@misc{a4, title={Seasons}, date={2020-21/2021-32}}
@misc{a5, title={Short year}, year={380}}
@misc{a6, title={Unknown start}, date={/2000}}
@misc{a7, title={Open start}, date={../2000}}
@misc{a8, title={Open end}, date={2000/..}}
```

```bibtex
% r2.bib
@comment{Comment from r2}
@misc{b1, title={B}}
```

`biber --tool --noconf -O - r1.bib r2.bib` (2.22 and `dev` at 1686b9c) prints `Argument "XX" isn't numeric in sprintf at .../Output/bibtex.pm line 730.` and writes

```bibtex
@MISC{a1,
  OPTIONS = {skipbib=true,useprefix=true,uniquename=false},
  AUTHOR = {Doe, J},
  AUTHOR+an:default = {1=Lit},
  DATE = {1999-00},
  TITLE = {T},
}

@MISC{a2,
  TITLE = {Zeros},
}

@MISC{a3,
  DATE = {2020-01-01/2020-01-02},
  MINUTE = {30},
  TITLE = {Times},
}

@MISC{a4,
  DATE = {2020-21/2021},
  ENDYEARDIVISION = {winterS},
  TITLE = {Seasons},
  YEARDIVISION = {spring},
}

@MISC{a5,
  DATE = {380},
  TITLE = {Short year},
}

@MISC{a6,
  ENDYEAR = {2000},
  TITLE = {Unknown start},
}

@MISC{a7,
  ENDYEAR = {2000},
  TITLE = {Open start},
}

@MISC{a8,
  DATE = {2000/},
  TITLE = {Open end},
}

@MISC{b1,
  TITLE = {B},
}

@COMMENT{Comment from r2}
```

1. **Only the last file's `@comment`s are written.** `Input/file/bibtex.pm` (around line 340) does `$Biber::MASTER->{comments} = $cache->{comments}{$filename}` for each file, replacing the comments of the previous ones.
2. **`@preamble`s are never written**, neither in tool mode nor with `--output-format=bibtex`. The preambles are collected (`$Biber::MASTER->{preamble}`), but `Output/bibtex.pm` has no `create_output_misc`, and `prepare_tool` doesn't call it.
3. **Unspecified dates are corrupted.** `construct_datetime` reconstructs `1999-XX` from `dateunspecified`, but then passes the `XX` through `sprintf('%.2d', …)`, giving `1999-00` (also `1999-01-00`, `1999-00-00`). `1999-00` reads back as an invalid date.
4. **Literal annotations lose their quotes.** `author+an={1="Lit"}` is written as `{1=Lit}`, which reads back as a non-literal annotation. `construct_annotation` doesn't check `is_literal_annotation`.
5. **Zero values are dropped** (the "0 is false" pattern): `volume={0}` (`if (my $val = $be->get_field($field))`), year 0 (`date={0000}`, 1 BCE), hour 0 (`T00:30:00` loses its time, and the minute is left over as a `MINUTE` field) and an end year 0 (`…/0000` becomes an open range `…/`). The `next unless $be->get_field("${d}year")` in `set_output_entry` and the `if (my $x = …)` tests in `construct_datetime` all treat `0` as missing.
6. **Seasons.** The year-division map in `construct_datetime` has `'WinterS' => 32`, but the value stored is `winterS` (`%YEARDIVISIONS` in Constants.pm), so the southern winter is lost. In addition, the `yeardivision`/`endyeardivision` fields are never deleted after use, so they are also written as separate fields.
7. **Years with fewer than four digits are written as they are** (`DATE = {380}`), which biber then rejects as an invalid date (`year={380}` is accepted as a legacy field, and `date={0380}` as an ISO date).
8. **Ranges with an unknown or open start are lost, and open ends become unknown.** The start year of `/2000` and `../2000` is stored as `''` (with `dateunknown` set for the first), so the `next unless $be->get_field("${d}year")` test skips the date, and only the end year is left over as an `ENDYEAR` field. For the end, `construct_datetime` writes `2000/` whether or not `enddateunknown` is set, so `2000/..` comes back as `2000/`.
9. **`OPTIONS` is written in hash order** (`getblxentryoptions` returns `keys`), so the output changes between identical runs.

With the patch below, the output is

```bibtex
@PREAMBLE{{\newcommand{\foo}{x}}}

@MISC{a1,
  OPTIONS = {skipbib=true,uniquename=false,useprefix=true},
  AUTHOR = {Doe, J},
  AUTHOR+an:default = {1="Lit"},
  DATE = {1999-XX},
  TITLE = {T},
}

@MISC{a2,
  DATE = {0000},
  TITLE = {Zeros},
  VOLUME = {0},
}

@MISC{a3,
  DATE = {2020-01-01T00:30:00/2020-01-02T00:00:00},
  TITLE = {Times},
}

@MISC{a4,
  DATE = {2020-21/2021-32},
  TITLE = {Seasons},
}

@MISC{a5,
  DATE = {0380},
  TITLE = {Short year},
}

@MISC{a6,
  DATE = {/2000},
  TITLE = {Unknown start},
}

@MISC{a7,
  DATE = {../2000},
  TITLE = {Open start},
}

@MISC{a8,
  DATE = {2000/..},
  TITLE = {Open end},
}

@MISC{b1,
  TITLE = {B},
}

@COMMENT{Comment from r1}
@COMMENT{Comment from r2}
```

and running `biber --tool` on this output reproduces it exactly.

Two related problems in the same code:

10. **`--output-safechars` (and the forced recoding when an entry can't be encoded in `--output-encoding`) is applied to the whole entry string**, including the entry key and verbatim fields. For `@misc{café, title={Café}, url={http://example.org/café}}`, tool mode writes `@MISC{caf\'{e},` and `URL = {http://example.org/caf\'{e}}`, and the `.bbl` gets `\entry{caf\'{e}}`, `\verb http://example.org/caf\'{e}` and `\verb 10.1/\'{e}` for a DOI. The key no longer matches `\cite{café}` (or the unrecoded `\keyalias` lines), and the URL is wrong. The patch recodes field by field in the BibTeX writer, and line by line in the `.bbl` writer, skipping the key lines (`\entry`, `\set`, `\inset`) and `\verb`/`\lverb` blocks. Keys and verbatim data are still recoded if they can't be encoded in the output encoding at all, as before.
11. **Latent:** `Output/bibtex.pm`'s `output` looks up the section-99999 list by a name built from the global `sortingtemplatename`, while `tool_mode_setup` names it `tool/global//global/global/global`. These agree only because `%CONFIG_DEFAULT_BIBLATEX` overwrites the user's config with `sortingtemplatename => 'tool'` (a separate report). If that is fixed, a `biber.conf` setting `sortingtemplatename` makes tool mode die with `Can't call method "get_keys" on an undefined value`. The patch takes the section's list directly.

### Suggested fix

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -4581,6 +4581,7 @@
   $self->process_lists;        # process the output lists (sort and filtering)
   $out->create_output_section; # Generate and push the section output into the
                                # into the output object ready for writing
+  $out->create_output_misc;    # Preambles
   return;
 }
 
--- a/lib/Biber/Input/file/bibtex.pm
+++ b/lib/Biber/Input/file/bibtex.pm
@@ -336,8 +336,9 @@
 
   # Save comments if in tool mode
   if (Biber::Config->getoption('tool')) {
-    if ($cache->{comments}{$filename}) {
-      $Biber::MASTER->{comments} = $cache->{comments}{$filename};
+    # Keep the comments of every data file, but each file only once
+    if ($cache->{counts}{$filename} < 2 and $cache->{comments}{$filename}) {
+      push(($Biber::MASTER->{comments} //= [])->@*, $cache->{comments}{$filename}->@*);
     }
   }
 
--- a/lib/Biber/Output/bbl.pm
+++ b/lib/Biber/Output/bbl.pm
@@ -96,6 +96,39 @@
   return;
 }
 
+=head2 _recode_entry
+
+    Recode an entry string to LaTeX macros, leaving entry keys and
+    verbatim fields alone
+
+=cut
+
+sub _recode_entry {
+  my $string = shift;
+  my $outenc = Biber::Config->getoption('output_encoding');
+  my $inverb = 0;
+  my $acc = '';
+  foreach my $line (split(/(?<=\n)/, $string)) {
+    my $keep = 0;
+    if ($inverb) {
+      $inverb = 0 if $line =~ m/^\s*\\endl?verb\b/;
+      $keep = 1;
+    }
+    elsif ($line =~ m/^\s*\\l?verb\{/) {
+      $inverb = 1;
+    }
+    elsif ($line =~ m/^\s*\\(?:entry|inset|set)\{/) {
+      $keep = 1;
+    }
+    # Keys and verbatim data are only recoded if they can't be output otherwise
+    unless ($keep and eval { encode($outenc, NFC($line), Encode::FB_CROAK | Encode::LEAVE_SRC); 1 }) {
+      $line = latex_recode_output($line);
+    }
+    $acc .= $line;
+  }
+  return $acc;
+}
+
 =head2 _printfield
 
   Add the .bbl for a text field to the output accumulator.
@@ -732,7 +765,7 @@
 
         # If requested to convert UTF-8 to macros ...
         if (Biber::Config->getoption('output_safechars')) {
-          $entry_string = latex_recode_output($entry_string);
+          $entry_string = _recode_entry($entry_string);
         }
         else { # ... or, check for encoding problems and force macros
           my $outenc = Biber::Config->getoption('output_encoding');
@@ -740,7 +773,7 @@
             # Can this entry be represented in the output encoding?
             if (not eval { encode($outenc, NFC($entry_string), Encode::FB_CROAK | Encode::LEAVE_SRC); 1 }) { # Malformed data encoding char
               # So convert to macro
-              $entry_string = latex_recode_output($entry_string);
+              $entry_string = _recode_entry($entry_string);
               biber_warn("The entry '$k' has characters which cannot be encoded in '$outenc'. Recoding problematic characters into macros.");
             }
           }
--- a/lib/Biber/Output/bibtex.pm
+++ b/lib/Biber/Output/bibtex.pm
@@ -141,9 +141,15 @@
     $outmap = sub {my $f = shift; ucfirst($CONFIG_OUTPUT_FIELDREPLACE{$f} // $f)};
   }
 
+  # The key is only recoded if it can't be output otherwise
+  my $outkey = $key;
+  unless (eval { encode(Biber::Config->getoption('output_encoding'), NFC($key), Encode::FB_CROAK | Encode::LEAVE_SRC); 1 }) {
+    $outkey = latex_recode_output($key);
+  }
+
   $acc .= '@';
   $acc .= $outmap->($bee);
-  $acc .=  "\{$key,\n";
+  $acc .=  "\{$outkey,\n";
 
   # hash accumulator so we can gather all the data before formatting so that things like
   # $max_field_len can be calculated
@@ -225,7 +231,7 @@
 
   # Per-entry options
   my @entryoptions;
-  foreach my $opt (Biber::Config->getblxentryoptions($secnum, $key)) {
+  foreach my $opt (sort (Biber::Config->getblxentryoptions($secnum, $key))) {
     push @entryoptions, $opt . '=' . Biber::Config->getblxoption($secnum, $opt, undef, $key);
   }
   $acc{$outmap->('options')} = join(',', @entryoptions) if @entryoptions;
@@ -233,7 +239,7 @@
   # Date fields
   foreach my $datefield ($dmh->{datefields}->@*) {
     my $d = $datefield =~ s/date$//r;
-    next unless $be->get_field("${d}year");
+    next unless defined($be->get_field("${d}year"));
 
     # Output legacy dates for YEAR/MONTH if requested
     if (not $d and Biber::Config->getoption('output_legacy_dates')) {
@@ -275,13 +281,14 @@
 
   # Standard fields
   foreach my $field ($dmh->{fields}->@*) {
-    if (my $val = $be->get_field($field)) {
+    my $val = $be->get_field($field);
+    if (defined($val) and $val ne '') {
       unless (Biber::Config->getoption('output_resolve_xdata')) {
         my $xd = xdatarefcheck($val);
         $val = $xd // $val;
       }
       # Could have been set in dates above (MONTH, YEAR special handling)
-      $acc{$outmap->($field)} = $val unless $acc{$outmap->($field)};
+      $acc{$outmap->($field)} = $val unless defined($acc{$outmap->($field)});
     }
   }
 
@@ -343,6 +350,29 @@
     }
   }
 
+  # Recode to LaTeX macros field by field, so that the entry key and
+  # verbatim/URI fields are left alone
+  my $outenc = Biber::Config->getoption('output_encoding');
+  my $recoded = 0;
+  foreach my $f (keys %acc) {
+    my $bf = lc(strip_annotation($f));
+    my $encodable = eval { encode($outenc, NFC($acc{$f}), Encode::FB_CROAK | Encode::LEAVE_SRC); 1 };
+    # Verbatim data is only recoded if it can't be output otherwise
+    next if ($dm->field_is_datatype('verbatim', $bf) or $dm->field_is_datatype('uri', $bf)) and $encodable;
+    # If requested to convert UTF-8 to macros ...
+    if (Biber::Config->getoption('output_safechars')) {
+      $acc{$f} = latex_recode_output($acc{$f});
+    }
+    # ... or, check for encoding problems and force macros
+    elsif (not $encodable) {
+      $acc{$f} = latex_recode_output($acc{$f});
+      $recoded = 1;
+    }
+  }
+  if ($recoded) {
+    biber_warn("The entry '$key' has characters which cannot be encoded in '$outenc'. Recoding problematic characters into macros.");
+  }
+
   # Determine maximum length of field names
   my $max_field_len;
   if (Biber::Config->getoption('output_align')) {
@@ -368,7 +398,7 @@
       }
       delete @acc{@donefields};
     }
-    elsif (my $value = delete $acc{$outmap->($field)}) {
+    elsif (defined(my $value = delete $acc{$outmap->($field)})) {
       $acc .= bibfield($outmap->($field), $value, $max_field_len);
     }
   }
@@ -380,22 +410,6 @@
 
   $acc .= "}\n\n";
 
-  # If requested to convert UTF-8 to macros ...
-  if (Biber::Config->getoption('output_safechars')) {
-    $acc = latex_recode_output($acc);
-  }
-  else { # ... or, check for encoding problems and force macros
-    my $outenc = Biber::Config->getoption('output_encoding');
-    if ($outenc ne 'UTF-8') {
-      # Can this entry be represented in the output encoding?
-      if (not eval { encode($outenc, NFC($acc), Encode::FB_CROAK | Encode::LEAVE_SRC); 1 }) { # Malformed data encoding char
-        # So convert to macro
-        $acc = latex_recode_output($acc);
-        biber_warn("The entry '$key' has characters which cannot be encoded in '$outenc'. Recoding problematic characters into macros.");
-      }
-    }
-  }
-
   # Create an index by keyname for easy retrieval
   $self->{output_data}{ENTRIES}{$secnum}{index}{$key} = \$acc;
 
@@ -457,15 +471,9 @@
   }
 
   # Bibtex output uses just one special section, always sorted by global sorting spec
-  foreach my $key ($Biber::MASTER->datalists->get_lists_by_attrs(section => 99999,
-                                                                 name => Biber::Config->getblxoption(undef, 'sortingtemplatename') . '/global//global/global/global',
-                                                                 type => 'entry',
-                                                                 sortingtemplatename => Biber::Config->getblxoption(undef, 'sortingtemplatename'),
-                                                                 sortingnamekeytemplatename => 'global',
-                                                                 labelprefix => '',
-                                                                 uniquenametemplatename => 'global',
-                                                                 labelalphanametemplatename => 'global',
-                                                                 namehashtemplatename       => 'global')->[0]->get_keys->@*) {
+  # (The list is made in tool_mode_setup or parse_ctrlfile; don't rely on the
+  # global sortingtemplatename still matching its name.)
+  foreach my $key ($Biber::MASTER->datalists->get_lists_for_section(99999)->[0]->get_keys->@*) {
     out($target, ${$data->{ENTRIES}{99999}{index}{$key}});
   }
 
@@ -483,6 +491,23 @@
   return;
 }
 
+=head2 create_output_misc
+
+    Output the \@preamble data of the data files
+
+=cut
+
+sub create_output_misc {
+  my $self = shift;
+  my $pa = $Biber::MASTER->get_preamble or return;
+  my $fc = Biber::Config->getoption('output_fieldcase');
+  my $name = $fc eq 'upper' ? 'PREAMBLE' : $fc eq 'title' ? 'Preamble' : 'preamble';
+  foreach my $p ($pa->@*) {
+    $self->{output_data}{HEAD} .= "\@${name}{{$p}}\n\n";
+  }
+  return;
+}
+
 =head2 create_output_section
 
     Create the output from the sections data and push it into the
@@ -595,17 +620,23 @@
   my ($key, $field, $name) = @_;
   my @annotations;
 
+  # Literal annotations are written in quotes, as they are read
+  my $q = sub { my ($v, @args) = @_;
+                Biber::Annotation->is_literal_annotation(@args) ? "\"$v\"" : $v };
+
   if (my $fa = Biber::Annotation->get_field_annotation($key, $field, $name)) {
-    push @annotations, "=$fa";
+    push @annotations, '=' . $q->($fa, 'field', $key, $field, $name);
   }
 
   foreach my $item (Biber::Annotation->get_annotated_items('item', $key, $field, $name)) {
-    push @annotations, "$item=" . Biber::Annotation->get_annotation('item', $key, $field, $name, $item);
+    push @annotations, "$item=" . $q->(Biber::Annotation->get_annotation('item', $key, $field, $name, $item),
+                                       'item', $key, $field, $name, $item);
   }
 
   foreach my $item (Biber::Annotation->get_annotated_items('part', $key, $field, $name)) {
     foreach my $part (Biber::Annotation->get_annotated_parts('part', $key, $field, $name, $item)) {
-      push @annotations, "$item:$part=" . Biber::Annotation->get_annotation('part', $key, $field, $name, $item, $part);
+      push @annotations, "$item:$part=" . $q->(Biber::Annotation->get_annotation('part', $key, $field, $name, $item, $part),
+                                               'part', $key, $field, $name, $item, $part);
     }
   }
 
@@ -662,7 +693,7 @@
                         'springS' => 29,
                         'summerS' => 30,
                         'autumnS' => 31,
-                        'WinterS' => 32,
+                        'winterS' => 32,
                         'Q1'      => 33,
                         'Q2'      => 34,
                         'Q3'      => 35,
@@ -715,25 +746,34 @@
   # Seasons derived from ISO 8601 dates
   if (my $s = $be->get_field("${d}yeardivision")) {
     $overridem = $yeardivisions{$s};
+    $be->del_field("${d}yeardivision");
   }
   if (my $s = $be->get_field("${d}endyeardivision")) {
     $overrideem = $yeardivisions{$s};
+    $be->del_field("${d}endyeardivision");
   }
 
+  # 'XX' (unspecified) is written as it is, numbers are padded
+  my $pad = sub { $_[0] =~ m/^\d+$/ ? sprintf('%.2d', $_[0]) : $_[0] };
+  # Years need four digits to be read again (0 is 1 BCE, -99 is 100 BCE)
+  my $ypad = sub { $_[0] =~ m/^(-?)(\d{1,3})$/ ? $1 . sprintf('%.4d', $2) : $_[0] };
+
   # date exists if there is a start year
-  if (my $sy = $overridey || $be->get_field("${d}year") ) {
-    $datestring .= $sy;
+  my $sy = $overridey // $be->get_field("${d}year");
+  if (defined($sy)) {
+    # An empty start year is an open start ('..') unless it is unknown ('')
+    $datestring .= ($sy eq '' and not $be->get_field("${d}dateunknown")) ? '..' : $ypad->($sy);
     $be->del_field("${d}year");
 
     # Start month
-    if (my $sm = $overridem || $be->get_field("${d}month")) {
-      $datestring .= '-' . sprintf('%.2d', $sm);
+    if (defined(my $sm = $overridem // $be->get_field("${d}month"))) {
+      $datestring .= '-' . $pad->($sm);
       $be->del_field("${d}month");
     }
 
     # Start day
-    if (my $sd = $overrided || $be->get_field("${d}day")) {
-      $datestring .= '-' . sprintf('%.2d', $sd);
+    if (defined(my $sd = $overrided // $be->get_field("${d}day"))) {
+      $datestring .= '-' . $pad->($sd);
       $be->del_field("${d}day");
     }
 
@@ -755,7 +795,7 @@
     }
 
     # If start hour, there must be minute and second
-    if (my $sh = $be->get_field("${d}hour")) {
+    if (defined(my $sh = $be->get_field("${d}hour"))) {
       $datestring .= 'T' . sprintf('%.2d', $sh) . ':' .
         sprintf('%.2d', $be->get_field("${d}minute")) . ':' .
           sprintf('%.2d', $be->get_field("${d}second"));
@@ -777,19 +817,20 @@
     }
 
     # End year
-    if (my $ey = $be->get_field("${d}endyear")) {
-      $datestring .= $ey;
+    my $ey = $be->get_field("${d}endyear");
+    if (defined($ey) and $ey ne '') {
+      $datestring .= $ypad->($ey);
       $be->del_field("${d}endyear");
 
       # End month
-      if (my $em = $overrideem || $be->get_field("${d}endmonth")) {
-        $datestring .= '-' . sprintf('%.2d', $em);
+      if (defined(my $em = $overrideem // $be->get_field("${d}endmonth"))) {
+        $datestring .= '-' . $pad->($em);
         $be->del_field("${d}endmonth");
       }
 
       # End day
-      if (my $ed = $be->get_field("${d}endday")) {
-        $datestring .= '-' . sprintf('%.2d', $ed);
+      if (defined(my $ed = $be->get_field("${d}endday"))) {
+        $datestring .= '-' . $pad->($ed);
         $be->del_field("${d}endday");
       }
 
@@ -811,7 +852,7 @@
       }
 
       # If end hour, there must be minute and second
-      if (my $eh = $be->get_field("${d}endhour")) {
+      if (defined(my $eh = $be->get_field("${d}endhour"))) {
         $datestring .= 'T' . sprintf('%.2d', $eh) . ':' .
           sprintf('%.2d', $be->get_field("${d}endminute")) . ':' .
             sprintf('%.2d', $be->get_field("${d}endsecond"));
@@ -827,6 +868,10 @@
         $be->del_field("${d}endtimezone");
       }
     }
+    # Likewise for an empty end year
+    elsif (defined($ey) and not $be->get_field("${d}enddateunknown")) {
+      $datestring .= '..';
+    }
   }
   return $datestring;
 }
```

The patch is on top of the one for the `$dmh->{datefields}` report (whose `my $d = $datefield =~ s/date$//r;` lines appear as context). All tests in `t/` pass on `dev` (1686b9c) with both.

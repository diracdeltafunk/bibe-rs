<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/559 (2026-11-03). Tracks BB-052, BB-053, BB-054, BB-122 (docs/biber-bugs.md); BB-082 and BB-083 were dropped (see there). -->

**Title:** Configuration merging: biber.conf sourcemaps dropped without driver maps, tool-mode defaults overwrite biber.conf, duplicate datalists not removed, undefined sorting templates not reported

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biber 2.22 and `dev` (a91aae4).

**1. User sourcemaps from biber.conf are dropped if the `.bcf` has no driver or style maps.** With a `biber.conf` containing

```xml
<sourcemap>
  <maps datatype="bibtex">
    <map map_overwrite="1">
      <map_step map_field_set="note" map_field_value="from biber.conf"/>
    </map>
  </maps>
</sourcemap>
```

and `@book{a, author={Doe, J}, title={T}, date={2000}, note={orig}}`, the note becomes `from biber.conf` with a normal `.bcf`, but stays `orig` if the `.bcf`'s `<bcf:sourcemap>` has only user-level maps (I tested this by deleting the `level="driver"` block from the `.bcf`). In `parse_ctrlfile`, the document's user maps are merged into the config file's maps, but then the `else` branch ("no driver defaults") replaces the whole option with the `.bcf` maps:

```perl
      else { # no driver defaults, just override the config file user map settings
        Biber::Config->setoption('sourcemap', $bcfxml->{sourcemap}{maps});
      }
```

**2. In tool mode, `%CONFIG_DEFAULT_BIBLATEX` overwrites the user's biber.conf.** `Config::_initopts` reads the tool config and then the user config, and only then applies the hard-coded biblatex defaults (`sortingtemplatename => 'tool'`, `useprefix => 0`, `maxbibnames => 100`, …), so the user can't change any of them in biber.conf. With

```xml
<config>
  <useprefix>1</useprefix>
  <sortingtemplate name="tool">
    <sort order="1"><sortitem order="1">author</sortitem></sort>
  </sortingtemplate>
</config>
```

and `@book{b, author={de Zed, B}}`, `@book{a, author={Mid, A}}`, `biber --tool --configfile=…` sorts `a, b` (by `Zed`), i.e. `useprefix=1` is ignored. The patch applies a default only when the option has no value yet. (This also makes the BibTeX writer's lookup of the section list by the global `sortingtemplatename` fail if a user sets that option; #553 has a patch for that.)

**3. Duplicate `<bcf:datalist>` elements are not removed.** The check in `parse_ctrlfile` calls `$datalists->get_list(section => …, name => …, type => …, …)`, but `DataLists::get_list` takes positional arguments (`$name, $section, $type`), so it never finds a match, and a `.bcf` with the same datalist twice gives two identical `\datalist` blocks in the `.bbl`. (biblatex doesn't write duplicates, as far as I can tell, so this is latent.) The patch uses `get_lists_by_attrs`.

**4. A datalist that names an undefined sorting template silently uses the global one.** With `sortingtemplatename="nosuch"` on a `<bcf:datalist>`, `generate_sortdataschema` falls back to the global sorting template without a message, so a typo in a template name (or a template that wasn't written to the `.bcf`) goes unnoticed. The patch warns: `Datalist 'mylist' uses the sorting template 'nosuch', which is not defined - using the global sorting template instead`.

### Suggested fix

```diff
--- a/lib/Biber.pm
+++ b/lib/Biber.pm
@@ -635,14 +635,11 @@ sub parse_ctrlfile {
         unshift($configmaps->{map}->@*, map {$_->{map}->@*} @docmaps);
       }
 
-      # Merge the driver/style maps with the user maps from the config file
-      if (my @m = grep {$_->{level} eq 'driver' or
-                        $_->{level} eq 'style'} $bcfxml->{sourcemap}{maps}->@* ) {
-        Biber::Config->setoption('sourcemap', [$usms->@*, @m]);
-      }
-      else { # no driver defaults, just override the config file user map settings
-        Biber::Config->setoption('sourcemap', $bcfxml->{sourcemap}{maps});
-      }
+      # Merge the driver/style maps (if any) with the user maps from the config
+      # file, which now include the document's user maps
+      my @m = grep {$_->{level} eq 'driver' or
+                    $_->{level} eq 'style'} $bcfxml->{sourcemap}{maps}->@*;
+      Biber::Config->setoption('sourcemap', [$usms->@*, @m]);
     }
     else { # just write the option as there are no config file settings at all
       Biber::Config->setoption('sourcemap', $bcfxml->{sourcemap}{maps});
@@ -1037,15 +1034,16 @@ SECTION: foreach my $section ($bcfxml->{section}->@*) {
     my $lname = $list->{name};
 
     my $lsection = $list->{section}[0]; # because "section" needs to be a list elsewhere in XML
-    if ($datalists->get_list(section                    => $lsection,
-                             name                       => $lname,
-                             type                       => $ltype,
-                             sortingtemplatename        => $lstn,
-                             sortingnamekeytemplatename => $lsnksn,
-                             labelprefix                => $lpn,
-                             uniquenametemplatename     => $luntn,
-                             labelalphanametemplatename => $llantn,
-                             namehashtemplatename       => $lnhtn)) {
+    # (get_list takes positional arguments; this needs the attribute search)
+    if ($datalists->get_lists_by_attrs(section                    => $lsection,
+                                       name                       => $lname,
+                                       type                       => $ltype,
+                                       sortingtemplatename        => $lstn,
+                                       sortingnamekeytemplatename => $lsnksn,
+                                       labelprefix                => $lpn,
+                                       uniquenametemplatename     => $luntn,
+                                       labelalphanametemplatename => $llantn,
+                                       namehashtemplatename       => $lnhtn)) {
       if ($logger->is_debug()) {# performance tune
         $logger->debug("Section datalist '$lname' of type '$ltype' with sortingtemplate '$lstn', sortingnamekeytemplatename '$lsnksn', labelprefix '$lpn', uniquenametemplate '$luntn', labelalphanametemplate '$llantn' and namehashtemplate '$lnhtn' is repeated for section $lsection - ignoring");
       }
@@ -1060,6 +1058,9 @@ SECTION: foreach my $section ($bcfxml->{section}->@*) {
                                         namehashtemplatename       => $lnhtn,
                                         labelprefix                => $lpn,
                                         name                       => $lname);
+    unless (Biber::Config->getblxoption(undef, 'sortingtemplate')->{$lstn}) {
+      biber_warn("Datalist '$lname' uses the sorting template '$lstn', which is not defined - using the global sorting template instead");
+    }
     $datalist->set_type($ltype || 'entry'); # lists are entry lists by default
     $datalist->set_name($lname || "$lstn/$lsnksn/$lpn/$luntn/$llantn/$lnhtn"); # default to ss+snkss+pn+untn+lantn+lnhtn
     foreach my $filter ($list->{filter}->@*) {
--- a/lib/Biber/Config.pm
+++ b/lib/Biber/Config.pm
@@ -161,7 +161,9 @@ sub _initopts {
   # Set hard-coded biblatex option defaults
   # This has to go after _config_file_set() as this is what defines option scope
   # in tool mode (from the .conf file)
+  # but they are only defaults: don't overwrite values from the config files
   foreach (keys %CONFIG_DEFAULT_BIBLATEX) {
+    next if defined($CONFIG->{options}{biblatex}{GLOBAL}{$_});
     Biber::Config->setblxoption(0, $_, $CONFIG_DEFAULT_BIBLATEX{$_});
   }
 
```

With this patch, the note in item 1 becomes `from biber.conf` with either `.bcf`, item 2 sorts `b` (`de Zed`) before `a`, the duplicated datalist is written once, and item 4 gives the warning above. All tests in `t/` pass on `dev` (a91aae4) with this patch, applied together with the others I'm filing alongside it, including the end-to-end tests run with `BIBER_DEV_TESTS=1`.

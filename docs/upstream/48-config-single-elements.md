<!-- Draft upstream issue for plk/biber. Posted as https://github.com/plk/biber/issues/566 (2026-11-03). Tracks BB-196 (docs/biber-bugs.md). -->

**Title:** biber.conf: a name template with a single `<namepart>`, or a single `<transliteration>`/`<translit>`, crashes biber ("Not an ARRAY reference")

---

🤖 This issue was identified by Claude Opus 5.5, and the text below is entirely written by Claude Opus 5.5

Checked with biber 2.22 and current `dev` (a91aae4). This is the same kind of problem as #464 (`datafieldset`) and #229 (`entryfields`): `_config_file_set` reads the config file with `XML::LibXML::Simple`, and its `ForceArray` list doesn't include `namepart`, `transliteration` or `translit`, so when one of these occurs only once it becomes a hash, and the `->@*` on it dies. (The `.bcf` reader's `ForceArray` list in `Biber.pm` has all three.)

Each of these config files validates against `config.rng`:

```xml
<config>
  <uniquenametemplate name="global">
    <namepart order="1" base="1">family</namepart>
  </uniquenametemplate>
</config>
```

```xml
<config>
  <transliteration entrytype="*">
    <translit target="title" from="russian" to="ala-lc"/>
  </transliteration>
</config>
```

With `--configfile` set to either, biber stops at once with `Not an ARRAY reference at …/Biber/Config.pm line 492.` (line 544 for the second), in normal runs as well as in tool mode, without writing a `.bbl`. A `labelalphanametemplate` (line 475) or `namehashtemplate` (line 506) with a single `<namepart>` does the same.

For transliteration, every biber.conf that uses it crashes: a single `<transliteration>` crashes whatever it contains, and several `<transliteration>` elements crash as soon as one of them has a single `<translit>`. So in tool mode, where biber.conf is where these options go, transliteration can't be used at all.

### Suggested fix

```diff
--- a/lib/Biber/Config.pm
+++ b/lib/Biber/Config.pm
@@ -398,6 +398,9 @@ sub _config_file_set {
                                                             qr/\Asortitem\z/,
                                                             qr/\Auniquenametemplate\z/,
                                                             qr/\Anamehashtemplate\z/,
+                                                            qr/\Anamepart\z/,
+                                                            qr/\Atransliteration\z/,
+                                                            qr/\Atranslit\z/,
                                                             qr/\Apresort\z/,
                                                             qr/\Aoptionscope\z/,
                                                             qr/\Asortingnamekeytemplate\z/,
```

With this change all four files are read and the `.bbl` is written. The transliteration also takes effect. With `@book{a, title={Щука}}`, `@book{b, title={Юла}}`, a tool sorting template on `title` and the `<transliteration>` above, `biber --tool` sorts `b` before `a` (`Iula` before `Shchuka`; without the transliteration, `a` sorts first). All tests in `t/` pass on `dev` (a91aae4) with this patch, on its own and together with the patches of #555–565, including the end-to-end tests run with `BIBER_DEV_TESTS=1`.

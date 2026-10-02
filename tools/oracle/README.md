# Oracle O3: biber 2.22 built from source on a pinned perl

O3 (see `docs/PLAN.md` §6.1) is biber 2.22 run from the pinned source tree
(`vendor/biber`, tag `v2.22`). It runs on a perl 5.42.0 built from source, and its
CPAN dependencies are pinned to the versions bundled in the TeX Live 2026 biber 2.22
PAR binary (`biber` on PATH, sha256 `8a50df30…ca88c`). It is used for:

* function-level differential testing (call `Biber::*` subs directly);
* running biber's own `t/*.t`;
* instrumenting and patching biber, for example to try upstream fixes.

**Status (2026-10-02):** built and working. 1275/1275 tests pass. All 60 parity cases
produce a `.bbl` byte-identical to the PAR binary's.

## Layout

| Path | What |
|---|---|
| `tools/oracle/build-perl-oracle.sh` | Reproducible, idempotent build script (phases `perl cpanm deps biber lock`) |
| `tools/oracle/deps.pinned` | The 123 CPAN dists, exact versions, install order |
| `tools/oracle/versions.lock` | Resulting perl configuration plus the version of every module biber loads |
| `tools/oracle/biber-src` | Wrapper that runs O3 like the `biber` CLI, from any cwd |
| `tools/oracle/run-tests.sh` | Runs biber's `t/*.t`, one file at a time, and reports per-file counts |
| `tools/oracle/parity.pl` | Parity harness: PAR `biber` vs `biber-src` |
| `tools/oracle/list-versions.pl` | Generates `versions.lock` |
| `vendor/perl/` | Install prefix: perl, cpanm, all modules, `lib/libbtparse.dylib` (gitignored) |
| `vendor/perl-build/` | Sources and build dirs (gitignored, ~1.4 GB). It contains `biber-src/` (built copy of `vendor/biber`), `dists/` (downloaded tarballs), `test-results/*.tap` and `parity/` |

Nothing is installed outside these two directories. `vendor/biber` is never written to:
the script copies it with `git archive HEAD` into `vendor/perl-build/biber-src` and runs
`perl Build.PL && ./Build` there. That puts `bcf.xsl`, `recode_data.xml`,
`biber-tool.conf` and the schemata into `blib/lib/Biber/`, where biber looks for them
(paths relative to `$INC{'Biber.pm'}`: `Biber.pm:390-401`, `LaTeX/Recode.pm:102-110`,
`Config.pm:148-154`).

## Build

```sh
tools/oracle/build-perl-oracle.sh          # all phases, about 10-15 min on Apple M-series
tools/oracle/build-perl-oracle.sh deps lock  # run only some phases
```

Requirements:

* Xcode Command Line Tools: clang, make, and the SDK's libxml2/libxslt headers;
* `curl`;
* network access to `www.cpan.org` / `cpan.metacpan.org`;
* optionally, an existing **OpenSSL 3** for `Net::SSLeay`. The script looks at
  `$OPENSSL_PREFIX`, then `/opt/homebrew/opt/openssl@3`, and uses it read-only. Without
  it, the three `[ssl]` dists are skipped and only https remote datasources are lost.
  OpenSSL 4 does not compile with Net::SSLeay 1.94.

What each phase does:

* **perl**: downloads `perl-5.42.0.tar.gz` and checks its sha256. It then runs
  `./Configure -des -Dprefix=vendor/perl -Dinstallstyle=lib/perl5 -Duseshrplib -Duselargefiles -Dusethreads`.
  These are the PAR perl's own flags (it is Homebrew's perl 5.42.0, with
  `archname=darwin-thread-multi-2level`), minus the Homebrew paths. The resulting
  `ccflags` and `optimize` are identical to the PAR perl's.
* **cpanm**: App-cpanminus 1.7048.
* **deps**: for each line of `deps.pinned`, downloads the exact tarball to
  `vendor/perl-build/dists/` and runs `cpanm --notest --no-lwp <tarball>`. Done dists are
  recorded in `vendor/perl/.oracle-installed-dists`. `--no-lwp` is needed: once LWP is
  installed, cpanm would otherwise try to use it for https downloads and fail.
* **biber**: copies the source tree and runs `Build.PL` / `./Build`. It rebuilds when
  the submodule's HEAD changes.
* **lock**: checks that cpanm installed no unpinned dists, then writes `versions.lock`.

Environment isolation: `PERL5LIB`, `PERL_LOCAL_LIB_ROOT`, `PERL_MB_OPT`, `PERL_MM_OPT`
and `PERL5OPT` are unset. `PERL_CPANM_HOME` is set to `vendor/perl-build/cpanm-home`, so
`~/.cpan`, `~/.cpanm` and `~/perl5` are never touched.

How `deps.pinned` was derived (done once, then frozen):

1. Ran `MM->parse_version` over every `.pm` in `$(biber --cache)/inc/lib` (1952 modules).
2. Compared the result with the freshly built perl's core: 1680 modules are non-core,
   and 66 are dual-life core modules in a newer version.
3. Ran `cpanm --installdeps` on biber-src into a throwaway copy of the perl, which gave
   the dependency closure and a valid install order.
4. Pinned each dist to the PAR version of its main module, resolved through
   `fastapi.metacpan.org/v1/download_url/<Module>?version===<v>`.

To change pins, edit `deps.pinned`, `rm -rf vendor/perl` (or delete lines from
`.oracle-installed-dists`), and re-run the script.

## Run

```sh
tools/oracle/biber-src [biber options] file.bcf     # same CLI as biber
```

The wrapper runs `vendor/perl/bin/perl -I…/biber-src/blib/lib …/biber-src/blib/script/biber "$@"`
with `PERL5LIB`/`PERL5OPT` unset. For in-process or function-level use:

```sh
vendor/perl/bin/perl -Ivendor/perl-build/biber-src/blib/lib -MBiber::Utils -e '…'
```

## Versions and deviations from the PAR binary

`versions.lock` lists 362 modules that biber loads (core and site). **All 325 that the
PAR bundles have exactly the PAR's version.** The other 37 are not in the PAR: they are
test and build tools (Module::Build 0.4234, Test::Differences 0.72, Text::Diff,
Algorithm::Diff, File::Which, Capture::Tiny, Perl::OSType), Test2 submodules, and
`XString` / `Params::ValidationCompiler::Exception::*`, which the packer did not bundle.

Key versions:

* perl 5.42.0 (Unicode 16.0.0)
* Unicode::Collate 1.31 (UCA/DUCET 13.0.0)
* Unicode::Normalize 1.32
* Unicode::LineBreak 2019.001 (sombok 2.4.0, Unicode **8.0.0**)
* Text::BibTeX 0.91
* XML::LibXML 2.0210 and XML::LibXSLT 2.003000, using libxml2 2.9.13 and libxslt 1.1.35
* Business::ISBN 3.012 with Business::ISBN::Data 20260218.001
* Lingua::Translit 0.29
* List::Util 1.70
* Mozilla::CA 20250602

The PAR also bundles 6 dual-life modules in versions newer than perl 5.42.0's core;
these were pinned to the PAR versions: Scalar-List-Utils 1.70, File-Temp 0.2312,
Test-Simple 1.302219, CPAN-Meta 2.150013, CPAN-Meta-Requirements 2.145, and
Compress-Raw-*/IO-Compress 2.217/2.221.

Remaining deviations, none of which affect `.bbl` output (see Parity):

| Area | PAR binary | O3 |
|---|---|---|
| perl paths | `/opt/homebrew/...`, built on macOS 26 (osvers 25.2.0) with Apple clang 17 | `vendor/perl`, built on macOS 27 with Apple clang 21. Same Configure flags and identical `ccflags`/`optimize` |
| libxml2 / libxslt | bundles copies of `/usr/lib/libxml2.2.dylib` / `libxslt.1.dylib` from its build host (Apple's 2.9.13 / 1.1.35 fork) | links the host's `/usr/lib` libxml2 2.9.13 / libxslt 1.1.35 via `Alien::Libxml2` 0.20 (install_type `system`) |
| TLS | Net::SSLeay 1.94 against macOS `/usr/lib/libssl` (LibreSSL) | Net::SSLeay 1.94 against Homebrew OpenSSL 3.6.5. The SDK has no LibreSSL headers |
| Extra PAR modules | bundles AnyEvent, Coro, EV, Event, IO-AIO, DBI/DBD-*, Date-Manip, Perl-Tidy, JSON-XS, Log-Dispatch(+FileRotate), PadWalker, Devel-Caller, Class-XSAccessor, Sub-Exporter, … | not installed: packer extras that biber never loads |
| Build-only dists | — | Alien-Build 2.88, Alien-Libxml2 0.20, PkgConfig, FFI-CheckLib, Path-Tiny, File-chdir, Config-AutoConf 0.320, ExtUtils-LibBuilder 0.09, Test-Fatal, Sub-Quote, Clone-PP, HTML-Tagset, HTTP-Cookies, WWW-RobotRules, XML-SAX(+NamespaceSupport). Pinned to CPAN-latest as of 2026-10-02 |
| Module tests | — | dists are installed with `--notest`; biber's suite is the acceptance test |
| Error texts containing Perl file paths | `…/par-…/cache-…/inc/lib/File/Slurper.pm line 59` | `…/vendor/perl/lib/perl5/site_perl/5.42.0/File/Slurper.pm line 59` |

## biber's test suite

Run with `tools/oracle/run-tests.sh`. It uses `LANG=C`, as `Build.PL`'s `ACTION_test` does.

The tests `chdir` into `t/tdata`, so `@INC` must be absolute. The script sets
`PERL5LIB` to the absolute `blib/lib:blib/arch` and puts `vendor/perl/bin` first on
`PATH`: `full-*.t` runs `which('perl') bin/biber`. It also sets `BIBER_DEV_TESTS=1`,
which enables `full-bbl`, `full-bibtex`, `full-dot` and `remote-files`; `remote-files`
needs network access.

**Result: 51 files, 1275 tests, 1275 ok, 0 failures**, all with exit 0. There are no
Perl warnings and no WARN/ERROR lines in the TAP output. TAP is kept in
`vendor/perl-build/test-results/`.

| file | ok/plan | file | ok/plan | file | ok/plan |
|---|---|---|---|---|---|
| annotations | 2/2 | full-dot | 2/2 | skipsg | 3/3 |
| basic-misc | 72/72 | labelalpha | 122/122 | sort-case | 2/2 |
| bcfvalidation | 53/53 | labelalphaname | 7/7 | sort-complex | 9/9 |
| biblatexml | 5/5 | labelname | 4/4 | sort-names | 1/1 |
| bibtex-aliases | 25/25 | langtags | 19/19 | sort-order | 18/18 |
| bibtex-output | 6/6 | maps | 8/8 | sort-uc | 6/6 |
| configfile | 10/10 | names | 75/75 | sorting | 52/52 |
| crossrefs | 37/37 | names_x | 21/21 | tool-bltxml-inout | 1/1 |
| datalists | 15/15 | options | 9/9 | tool-bltxml | 3/3 |
| dateformats | 56/56 | related-entries | 15/15 | tool-config | 12/12 |
| dm-constraints | 16/16 | remote-files | 1/1 | tool | 19/19 |
| encoding | 10/10 | sections-complex | 68/68 | translit | 1/1 |
| extradate | 39/39 | sections | 14/14 | truncation | 12/12 |
| extratitle | 14/14 | set-dynamic | 7/7 | uniqueness-nameparts | 36/36 |
| extratitleyear | 8/8 | set-legacy | 3/3 | uniqueness | 227/227 |
| full-bbl | 5/5 | set-static | 6/6 | utils | 89/89 |
| full-bibtex | 2/2 | skips | 15/15 | xdata | 13/13 |

## Parity with the PAR binary

Run with `/usr/bin/perl tools/oracle/parity.pl [regex]`. Each case runs
`biber --noconf X.bcf` and `tools/oracle/biber-src --noconf X.bcf` in separate copies
under `vendor/perl-build/parity/{par,src}/<case>/`. The harness then compares:

* the `.bbl`, raw and after the docs/research/05 §5.4 normalisation (per-name option
  lists in `\name{..}{n}{<opts>}{%` and `{{<opts>}{%` sorted);
* the exit code;
* the WARN/ERROR lines of the `.blg`, as `same`, `reord` (same set, different order) or
  `differ`.

**Result: 60/60 cases have byte-identical `.bbl` without normalisation**, and every exit
code matches:

* `scratch/perf/small.bcf` (+ `ex.bib`);
* all 53 `t/tdata/*.bcf`. Five of these exit 2 in both binaries by design: `crossrefs`,
  `xdata` (circular inheritance) and `encoding1/3/4` (they need `--input-encoding`).
  `remote-files` fetches over the network.
* `scratch/perf/n10.bcf` (+ `big10.bib`, 920 entries);
* bonus: `scratch/output/corpus/ex-{alphabetic,apa,authoryear,chicago,numeric-comp}.bcf`,
  which is `\nocite{*}` over `biblatex-examples.bib`.

`.blg` differences, none of them caused by O3:

* `full-dot`, `general`, `set-dynamic`, `set-legacy`: the same warnings in a different
  order. This is hash-order nondeterminism (BB-065); it also varies between runs of the
  same binary.
* `encoding1/3/4`: only the Perl source path inside the decode error message differs.
* `xdata`: a different circular-XDATA ERROR, and a different number of WARNs before it.
  This is nondeterministic in **both** binaries; see "New biber issues" below.

Wall times (one run each, M-series, warm cache):

| Case | PAR `biber` | `biber-src` |
|---|---|---|
| typical `t/tdata` case | 0.78–1.02 s | 0.27–0.50 s |
| small.bcf | 1.46 s | 0.67 s |
| n10 (920 entries) | 4.32 s | 3.73 s |
| corpus ex-* (≈100 entries) | 1.30–1.45 s | 0.78–0.93 s |
| total of the 55 core cases | 51.7 s | 22.9 s |

The ~0.5 s constant difference is PAR's start-up (unpacking and loading its archive).
Per-entry throughput is about the same.

## New biber issues seen while doing this

* **Nondeterministic circular-XDATA diagnostics** (candidate BB entry). See
  `Biber.pm:1450`: `resolve_xdata` iterates `$section->bibentries->entries`, which is
  `values %$self` at `Entries.pm:68`, so the order is hash-random. The cycle detected
  first is therefore random, and so is the `biber_error` text from `Entry.pm:913`.
  Because biber exits on the first error, the number of WARNs emitted before it is
  random too.

  Repro: `cd t/tdata && for i in 1 2 3 4; do biber --noconf xdata.bcf | grep ERROR; done`.
  The output alternates between `Circular XDATA inheritance between 'loop'<->'loop:3'`
  and `… 'lxd1:loop'<->'lxd2:loop'`. With `--nodieonerror` the resolution order of loop
  members, and so possibly the output, is also order-dependent (not yet checked).

No test failures; no other new bugs.

## Licences (for the Rust port)

These are facts read from the distribution files; they are not legal advice.

* **Text-BibTeX 0.91** (Perl/XS part): "same terms as Perl itself". META `license: perl_5`,
  which is Artistic-1.0-Perl OR GPL-1.0-or-later. The dist ships no LICENSE/COPYING file.
* **btparse** (the C library inside Text-BibTeX, `btparse/src`): **GNU *Library* General
  Public License, version 2 or (at your option) any later version** (LGPL-2.0-or-later).
  Copyright (c) 1996-99 Gregory P. Ward.
  * The header is in `names.c` (which has `bt_split_name` / `bt_split_list`, the code we
    would port), `format_name.c`, `string_util.c` (no header, but part of the library),
    `util.c`, `macros.c`, `lex_auxiliary.c`, `input.c`, `postprocess.c`, `tex_tree.c`,
    `traversal.c`, `modify.c`, `bibtex.g`, `btparse.h` and others. The docs
    (`btparse/doc/*.pod`, including `bt_split_names.pod`) are under the same LGPL.
  * The PCCTS runtime and generated parser (`btparse/pccts/*`, `sym.c`, `scan.c`,
    `bibtex.c`, `err.c`) are **public domain** (PCCTS notice, credit requested).
  * Implication: a straight translation of `names.c` is a derivative work under
    LGPL-2.0+. That means keeping that crate LGPL (with a relinking/replacement
    possibility for static Rust binaries), or reimplementing from observed behaviour plus
    O3 differential testing without translating the code.
* **Unicode-Collate 1.31**: code "same terms as Perl itself" (perl_5), © SADAHIRO Tomoyuki.
  * `Collate/allkeys.txt` is copied verbatim from Unicode UCA 13.0.0: "Copyright 2020
    Unicode, Inc.", under the Unicode terms of use, i.e. the Unicode data licence
    (permissive, notice required).
  * `Collate/Locale/*.pl` (87 tailorings) are generated by `mklocale` from `data/*.txt`,
    which encode the **CLDR** collation tailorings (Locale.pm cites the CLDR
    `seed/collation` sources and versions). They carry the module's Perl terms, but the
    underlying data is Unicode/CLDR (Unicode License v3).
  * `Collate/CJK/*.pm` (Pinyin/Stroke/Zhuyin from CLDR; Big5/GB2312/JISX0208/Korean
    orderings) are also perl_5.
  * The UCA algorithm itself (UTS #10) is a Unicode specification.
* **Lingua-Translit 0.29**: "same terms as Perl itself" (perl_5). © 2007-2008 Alex and
  Rona Linke, 2009-2022 Lingua-Systems / Netzum Sorglos Software GmbH. The XML
  transliteration tables (`xml/*.xml`, e.g. `iast_devanagari.xml`) are part of the dist
  and carry no separate licence.
* **Business-ISBN-Data 20260218.001**: Artistic-2.0 (LICENSE file, META
  `artistic_2`; © brian d foy). `lib/Business/ISBN/RangeMessage.xml` is the
  International ISBN Agency's range file (`<MessageSource>International ISBN
  Agency</MessageSource>`, from isbn-international.org). It has no copyright or licence
  statement in the file itself, so the agency's site terms apply to the data.
* **biber `lib/Biber/LaTeX/recode_data.xml`**: no per-file header. It is part of biber,
  which is **Artistic-2.0** (`LICENSE`; README "Copyright 2009-2025 François Charette
  and Philip Kime"; `Recode.pm` "© 2009-2012 F. Charette and P. Kime, 2012-2026 Philip
  Kime, Artistic License 2.0"). Note that biber's `Build.PL` metadata says
  `license => 'perl'`, which is inconsistent with its LICENSE file.

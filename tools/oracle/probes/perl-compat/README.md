# perl-compat oracle probes

Perl scripts run with oracle O3 (`vendor/perl/bin/perl`, see `../../README.md`) that
generate the tables and test vectors of `crates/perl-compat`. All are deterministic;
the `#[ignore]`d `oracle_live` tests re-run them and compare with the committed output
(`BIBERS_O3_PERL=… BIBERS_O3_LIB=… cargo test -p perl-compat -- --ignored`).

| Script | Output |
|---|---|
| `gen_unicode_tables.pl [sombok/lib/8.0.0.c]` | `src/tables/*.rs`: case maps, regex classes, `quotemeta`, blocks, `Unicode::UCD::num`, GCB (Unicode 16) and sombok's Unicode 8.0 grapheme table |
| `unicode_digests.pl` | `tests/data/unicode_digests.json`: SHA-256 per behaviour over every code point |
| `scalar_vectors.pl` | `tests/data/scalar_vectors.json`: numbers, `num`, `Text::Roman`, case, graphemes |
| `isbn_vectors.pl` | `tests/data/isbn_vectors.json`: `Business::ISBN`/`ISSN`/`ISMN` |
| `uri_vectors.pl` | `tests/data/uri_vectors.json`: `URI->new->as_string` |
| `regex_vectors.pl` (needs `-I…/biber-src/blib/lib`) | `tests/data/regex_vectors.json`: `Biber::Utils::ireplace`/`imatch`, `split`, `s///g` |

The sombok table comes from the Unicode-LineBreak 2019.001 build directory under
`vendor/perl-build/cpanm-home/work/*/` (pass its `sombok/lib/8.0.0.c` explicitly when
running from a worktree).

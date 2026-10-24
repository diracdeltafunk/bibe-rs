# Security

Please report security problems privately, through [GitHub's private vulnerability reporting](https://github.com/diracdeltafunk/bibe-rs/security/advisories/new). Don't open a public issue.

## Scope

bibers reads `.bcf`, `.bib`, biblatexml and `biber.conf` files that may come from other people, for example a shared bibliography or a document from a co-author. Problems worth reporting include:
- **Crashes, hangs or runaway memory** caused by an input file.
- **Remote data sources:** problems in fetching `http`/`https`/`ftp` data sources, for example TLS verification failures not being reported, or a server response that crashes bibers.
- **Code execution:** anything that makes bibers execute input data as code. bibers never evaluates Perl code; source map replacement strings are interpolated, not run ([docs/compat.md](docs/compat.md), DC-023).
- **Files outside the expected ones:** reading or writing files other than the documented inputs and outputs.

User-supplied regular expressions (source maps, `noinit`, `nosort`, …) run on PCRE2, with backtracking limits from PCRE2's defaults. A pathological regex in your own configuration is not a vulnerability. A regex in a third-party style or data file that makes bibers hang is worth reporting.

Fixes are released as new patch versions. There is only one supported version: the latest release.

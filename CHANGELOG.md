# Changelog

All notable changes to bibers are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [0.1.0] - unreleased

The first release: a preview of bibers, a Rust port of biber 2.22 that runs as a drop-in replacement for it.

### Compatibility

- **Same interface and output.** bibers takes biber 2.22's command line and `biber.conf` and reads the same `.bcf`. It writes the same `.bbl`, `.blg` and other outputs, byte for byte. The exceptions are the deliberate differences listed in [docs/compat.md](docs/compat.md).
- **Requires biblatex 3.22** (control file format 3.11). Like biber 2.22, bibers stops with an error on a `.bcf` from a biblatex that needs a different biber.
- **Feature coverage.** All of biber 2.22's processing is covered:
  - sorting with Unicode::Collate 1.31's tables and every locale tailoring;
  - name, date and range handling;
  - crossref, xref, XDATA and set inheritance;
  - labels, uniqueness and extra* fields;
  - source maps with Perl-compatible regexes;
  - multiple sections and data lists;
  - transliteration;
  - tool mode and bibtex output;
  - biblatexml input and output;
  - RELAX NG validation (`--validate-control`, `--validate-config`, `--validate-bltxml`) and `--convert-control`;
  - remote `http`/`https`/`ftp` data sources;
  - every input and output encoding Perl's Encode supports, apart from the three below;
  - `--recodedata`.
- **Testing.** Verified against biber 2.22 on:
  - a 776-case corpus;
  - every TeX Live `.bib` file under 8 styles;
  - 163 real biblatex documents;
  - about 100,000 random differential tests.

### Differences from biber

- **Fixed: crashes, data loss and nondeterministic output.** Where biber crashes on valid input, loses data or produces output that varies from run to run, bibers gives the intended result or a clear ERROR (exit 2). Examples:
  - endless loops in uniqueness processing;
  - tool mode overwriting an input file that has no extension;
  - hash-order-dependent output.
- **Reproduced by default: biber bugs that silently change output.** Each sits behind a named switch; `--fix-quirks=<name>` or `--fix-quirks=all` turns on the corrected behaviour ([docs/compat.md](docs/compat.md) §2).
- **Not supported:**
  - the deprecated `bblxml` and `dot` output formats, and `bibjson`;
  - the UTF-7, gsm0338 and MIME-* encodings;
  - Perl code inside source map replacement strings;
  - a few Perl-only regex constructs, such as `\N{NAME}`, which give an ERROR naming the construct.
- **Messages:** log messages keep biber's format and texts. The location tags and some low-level error texts differ ([docs/compat.md](docs/compat.md) §5).

### Added

- `--threads N` (default: one per core). Reading, per-entry processing and `.bbl` writing run in parallel. Output and messages are identical for every thread count.
- `--fix-quirks` (see above).

### Performance

Measured on an Apple M3 (8 cores):
- about 120× faster than biber on a 100-entry document (12 ms against 1.5 s);
- about 100× faster on large bibliographies (17,000 entries: 0.7 s against 71 s);
- up to 5,000× faster on cases where biber's processing is quadratic or cubic;
- `bibers --version` takes about 2 ms.

### Distribution

- Static binaries for macOS (universal), Linux (x86_64 and aarch64, musl) and Windows (x86_64), with SHA-256 checksums. The binaries are not code-signed.
- Building from source needs Rust 1.88 or later.

[0.1.0]: https://github.com/diracdeltafunk/bibe-rs/releases/tag/v0.1.0

# Contributing to bibe-rs

Thanks for helping! bibers has one goal: be biber 2.22, only faster. Every change is judged against biber's actual behaviour. "Better" output is a bug unless it is a documented deliberate difference.

## Reporting a difference from biber

The most valuable report is a document where bibers and biber disagree. Please [open an issue](https://github.com/diracdeltafunk/bibe-rs/issues/new/choose) and include:
- the `.bcf` and `.bib` files (or a cut-down example);
- the command line;
- `bibers --version` and `biber --version`;
- what each program produced.

## Building and testing

```sh
git clone --recurse-submodules https://github.com/diracdeltafunk/bibe-rs
cd bibe-rs
cargo test --workspace     # unit tests, corpus well-formedness and the passing.txt ratchet
cargo xtask corpus         # the full differential corpus (tests/corpus), with diffs: -v
```

You need Rust 1.88 or later (the `msrv` CI job checks this) and a C compiler, for PCRE2, `ring` and mimalloc.

**Oracles.** The expected outputs in `tests/corpus` were produced by biber 2.22, so the tests above need no Perl or TeX. Some tasks need biber itself, called an *oracle*. Tests that need an oracle skip when it is missing.
- **O1:** TeX Live's `biber` on `PATH`. Used for new expected outputs (`cargo xtask golden`) and for the comparison scripts in `tools/corpus/`.
- **O3:** biber built from source on a pinned perl (`tools/oracle/build-perl-oracle.sh`, about 15 minutes; see [tools/oracle/README.md](tools/oracle/README.md)). Used for the phase-level differential tests (`crates/bibers/tests/phase_*.rs`) and for trying fixes to biber.

**Corpus cases.** Each case lives in `tests/corpus/<suite>/<case>/`, with a `case.toml`, the inputs and an `expected/` directory (see [tests/corpus/README.md](tests/corpus/README.md)). `tests/corpus/passing.txt` is a ratchet: it only grows. `cargo xtask corpus --update-passing` adds newly passing cases and refuses to drop a failing one.

**Performance work** must not change output. Measure before and after with `tools/bench/run.py`, which also checks every output against biber's (see [docs/M5.md](docs/M5.md)).

## Rules for changes

- **Port faithfully.** Follow biber's code function by function, keep its iteration orders, and cite `Biber.pm:<line>` and the like in comments. Route Perl-isms through `perl-compat` (truthiness, `lc`/`uc`, normalisation, regexes, numbers). See [docs/M2.md](docs/M2.md).
- **Any observable difference from biber needs a `DC-NNN` entry** in [docs/compat.md](docs/compat.md) before it is merged. That covers output, messages, exit codes and file side effects.
- **biber bugs:**
  - Crashes, data loss and nondeterminism are fixed.
  - Bugs that silently change output are reproduced by default, behind a named switch in `bibers_core::quirks` (`--fix-quirks`).
  - Log new biber bugs in [docs/biber-bugs.md](docs/biber-bugs.md) with a minimal reproducer.
- **Clean-room BibTeX parser.** btparse (the C library inside Text::BibTeX) is LGPL-2.0-or-later. The `btparse` crate is written only from the behavioural specification in [docs/spec/btparse.md](docs/spec/btparse.md) and black-box tests against biber. **Do not read, translate or paraphrase btparse's C source** when working on that crate. Describe behaviour in the spec instead.
- **Third-party code.** `ucollate` and `perl-compat` port Perl modules under Perl's terms. Credit any new port or data source in [NOTICE.md](NOTICE.md), and keep each crate's `license` field accurate.
- **Before opening a pull request:**
  - `cargo fmt --all`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo xtask corpus`
  - for performance changes, `tools/bench/run.py`

## Licence

By contributing, you agree that your contributions are licensed under the [Artistic License 2.0](LICENSE). Contributions to the ported parts of `ucollate` and `perl-compat` are also offered under those crates' terms (see [NOTICE.md](NOTICE.md)).

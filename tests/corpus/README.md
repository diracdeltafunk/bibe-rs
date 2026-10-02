# Differential test corpus

Every case runs an implementation (bibers, or an oracle) on fixed inputs and compares its outputs with reference outputs produced by **biber 2.22** (oracle O1, the TeX Live binary; O3, the source-built copy in `tools/oracle/`, gives identical results). See `docs/PLAN.md` §6.

```
tests/corpus/
  passing.txt            # ratchet: cases bibers must pass (checked by `cargo test`)
  shared/                # data files shared by many cases (e.g. biblatex-examples.bib)
  <suite>/<case>/
    case.toml            # the case spec (below)
    *.bcf *.bib …        # inputs, copied into a fresh temp work dir for each run
    expected/            # written by `cargo xtask golden`, never by hand*
      <outputs…>         # raw oracle outputs (e.g. the .bbl)
      messages.txt       # normalised, sorted WARN/ERROR lines from the .blg
      meta.toml          # oracle, oracle version, exit code, determinism flag
```

\* Except for cases whose expectations deliberately differ from biber (see "Deviations" below).

## case.toml

```toml
description = "one line"
source = "where the inputs came from (path in vendor/biber, TeX Live doc, generator…)"
args = ["--noconf", "doc.bcf"]          # argv after the program name; always pass --noconf
                                        # unless the case is about biber.conf (then use --configfile=x.conf)
outputs = ["doc.bbl"]                   # files compared with expected/
shared = ["biblatex-examples.bib"]      # optional: files copied from shared/
log = "doc.blg"                         # optional: defaults to first output with .blg extension
env = { ISBN_RANGE_MESSAGE = "…" }      # optional extra environment (LANG/LC_ALL are fixed by the harness)
compare = "exact"                       # or "unordered-lines" (only for oracle nondeterminism, BB-061/062)
tags = ["suite:styles", "style:apa"]    # free-form; filter with `cargo xtask corpus tag:style:apa`
allow = []                              # DC-NNN ids from docs/compat.md whose documented differences apply
timeout_secs = 300
```

Cases may not contain subdirectories other than `expected/`.

## Normalisation

The same normalisation is applied to both sides before comparing (`crates/testkit/src/normalize.rs`):
- in `.bbl` files, per-name option lists are sorted (biber emits them in hash order, BB-060);
- for `.blg` files, only WARN/ERROR lines are compared, as a sorted multiset, with the work directory replaced by `<WORK>` and btparse temp-file paths replaced by `<TMPBIB>`.

## Commands

```
cargo xtask list [FILTER]                    # list cases
cargo xtask golden [FILTER]                  # (re)generate expected/ with O1, running each case twice
cargo xtask golden --check --oracle o3       # verify O3 reproduces the stored expectations
cargo xtask corpus [-v] [--keep] [FILTER]    # run bibers and compare
cargo xtask corpus --impl o1                 # sanity check: the oracle passes its own expectations
cargo xtask corpus --update-passing          # rewrite the ratchet after a full run
cargo test -p bibers --test corpus           # well-formedness + ratchet (no TeX/biber needed)
```

## Adding cases

1. Make the case directory with its inputs and `case.toml`. If the inputs come from a LaTeX document, keep the `.tex` next to the `.bcf`. One `pdflatex -draftmode` run produces the `.bcf`, and the generator scripts in `tools/corpus/` automate this.
2. `cargo xtask golden <case>`. If it reports NONDETERMINISTIC, find out why before accepting the case.
3. Commit inputs, `case.toml` and `expected/`.

## Deviations

If bibers deliberately differs from biber on a case (a `DC-NNN` entry in `docs/compat.md`), list the id in `allow`. Hand-maintained expectations for such a case are a future harness feature; until then these cases are excluded from the ratchet.

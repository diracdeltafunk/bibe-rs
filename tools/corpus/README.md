# Corpus generators

Scripts that build the differential test corpus in `tests/corpus/` (format: `tests/corpus/README.md`). They write case inputs and `case.toml`; the expectations come from the oracle:

```
python3 tools/corpus/gen_<suite>.py [NAME-REGEX]   # (re)write inputs + case.toml
cargo xtask golden <suite>/                        # (re)generate expected/ with O1
cargo xtask golden --check --oracle o3 <suite>/    # cross-check with the source-built oracle
```

Requirements: Python ≥ 3.12, TeX Live with biblatex (`pdflatex`, `lualatex`, `xelatex`, `kpsewhich`). The scripts are re-runnable. A rerun rewrites each case's inputs and `case.toml` but keeps `expected/`. LaTeX output is reproducible, so an unchanged TeX Live gives byte-identical `.bcf` files. Without a filter, `gen_styles.py` and `gen_bugs.py` also delete case directories they no longer generate. LaTeX runs in temporary directories, so no `.aux`/`.log`/`.pdf`/`.run.xml` reaches the corpus.

| Script | Suite | What it does |
|---|---|---|
| `gen_tdata.py` | `tdata/` | One case per `vendor/biber/t/tdata/*.bcf`, run as `--noconf x.bcf`, with only the data files named in its `<bcf:datasource>`s. Also adds command-line equivalents of `t/encoding.t`'s encoding settings, `general.bcf` + `biber-test.conf` (as in `t/configfile.t`), and `xdata-noloop`. Skipped: `remote-files` (needs the network, DC-032) and `xdata` (its fatal error is nondeterministic, BB-112). |
| `gen_examples.py` | `biblatex-examples/` | One case per biblatex example document (`$TEXMFDIST/doc/latex/biblatex/examples/*.tex`), compiled with pdflatex, or lualatex/xelatex if it loads fontspec/xeCJK. `biblatex-examples.bib` comes from `tests/corpus/shared/`. Skipped: `95-customlists` (`$MAPUNIQ` is time-based, BB-113). |
| `gen_styles.py` | `styles/` | `\nocite{*}` (plus a few `\cite`s) over `biblatex-examples.bib` for the 26 standard styles, 15 contributed styles, and an option matrix that varies one option at a time from authoryear/alphabetic/numeric. `--list` prints the cases. |
| `gen_bugs.py` | `bugs/` | One case per biber bug with an observable effect (`docs/biber-bugs.md`, status V/C). The reproducers are inline in the script. `--list-skipped` lists the bugs without a case and why. |
| `gen_scale.py` | `scale/`, benchmarks | `corpus` writes the committed `scale/mixed-1000` case. `bench [--out DIR] [SHAPE:N…]` writes larger synthetic inputs (default `target/bench-corpus/`, gitignored). Shapes are documented in the script. |
| `gen_tool.py` | `tool/` | Tool mode (`--tool`) and `--output-format=bibtex`: every `--output-*` option alone and in combinations, over `t/tdata/tool.bib` and a synthetic `.bib` (dates, seasons, unspecified dates, annotations, macros, comments) inline in the script; several data files, `--output-file` (incl. `-`), config files with source maps and output options, input/output encodings, biber's fixtures, `biblatex-examples.bib`, and `.bcf`-driven bibtex output. `dc-*` cases have reviewed `expected-bibers/`. |
| `texlive_tool_diff.py` | — | Differential sweep: biber vs bibers in tool mode with five `--output-*` combinations over a list of `.bib` files (e.g. all of TeX Live's). |
| `bench.sh` | — | Times `$BIBER` (default `biber`) over the benchmark inputs. |
| `corpuslib.py` | — | Shared helpers. |

## Tags

- Every case has `suite:<suite>`.
- `styles/` cases have `style:<name>`; matrix cases also have `matrix`, `option:<name>` and `option:<name>=<value>`.
- `bugs/` cases have `bug:BB-NNN`.
- `feature:<x>` marks the feature a case exercises (`biblatexml`, `configfile`, `sourcemap`, `encoding`, `xdata`).
- `deferred:DC-NNN` marks cases that need a feature bibers defers to M4 (docs/compat.md §4). bibers is expected to fail these until the feature lands.
- `perl-die` marks cases where biber dies with a Perl error. The exit code then comes from `errno`: the PAR binary (O1) exits with 25, the source build (O3) with 255.
- `o3-differs` marks cases where O3 legitimately differs from O1: the CJK sort locales (BB-011) and the Perl-die exit codes. `cargo xtask golden --check --oracle o3` reports these cases as mismatches.

## Nondeterminism found while building the corpus

- `bugs/bb-061-…` and `bugs/bb-062-…` (BB-061/062) use `compare = "unordered-lines"`, which is stable.
- BB-112 (xdata cycles) and BB-113 (`$MAPUNIQ`) can't be stabilised by the harness, so they have no case. See `gen_bugs.py` `NO_CASE`.

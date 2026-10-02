# bibe-rs

A fast, behaviour-compatible Rust port of [biber](https://github.com/plk/biber), the biblatex backend. The binary is called **`bibers`**. It aims to produce byte-identical `.bbl` output to biber 2.22, at least 50× faster.

Status: planning / infrastructure (M0). No code yet.

- [docs/PLAN.md](docs/PLAN.md): goals, feature inventory, architecture, correctness strategy, milestones, decisions.
- [docs/compat.md](docs/compat.md): every behavioural difference between bibers and biber.
- [docs/biber-bugs.md](docs/biber-bugs.md): bugs found in biber itself, to be reported upstream.
- [docs/research/](docs/research/): subsystem deep-dives into biber 2.22.
- `vendor/biber`: upstream biber, pinned at `v2.22` (git submodule).

bibers is a modified version of biber (© François Charette and Philip Kime, Artistic License 2.0). It is not affiliated with or endorsed by the biber authors. bibers is distributed under the Artistic License 2.0 (see [LICENSE](LICENSE)); differences from biber are documented in [docs/compat.md](docs/compat.md).

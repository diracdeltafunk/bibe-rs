//! Differential/golden test harness for bibers.
//!
//! A *case* is a directory under `tests/corpus/<suite>/<name>/` containing:
//!
//! - `case.toml`: the [`CaseSpec`];
//! - input files (`.bcf`, `.bib`, `.conf`, …), copied into a fresh work
//!   directory for each run;
//! - `expected/`: the reference outputs produced by an oracle (biber 2.22):
//!   the files listed in `outputs`, `messages.txt` (normalised WARN/ERROR
//!   lines) and `meta.toml` ([`ExpectedMeta`]).
//!
//! The same machinery runs any implementation (bibers, oracle O1 = the
//! installed biber binary, oracle O3 = the source-built biber) and compares it
//! with the expectations after the normalisations documented in
//! `docs/PLAN.md` §6.3.

pub mod case;
pub mod compare;
pub mod normalize;
pub mod phase;
pub mod run;

pub use case::{Case, CaseSpec, CompareMode, ExpectedMeta, discover};
pub use compare::{Outcome, compare, compare_bibers};
pub use run::{RunOutput, Runner};

use std::path::{Path, PathBuf};

/// The repository root (two levels above this crate's manifest).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives in <root>/crates/testkit")
        .to_path_buf()
}

/// The corpus root, `tests/corpus`.
pub fn corpus_root() -> PathBuf {
    repo_root().join("tests").join("corpus")
}

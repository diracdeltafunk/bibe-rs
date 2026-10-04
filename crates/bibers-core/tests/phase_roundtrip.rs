//! The model must hold everything biber's phase dumps contain: for a sample
//! of corpus cases, every phase dump must survive load → dump unchanged.
//!
//! Needs oracle O3; skipped (passes vacuously) when it isn't built. Set
//! `BIBERS_PHASE_CASES` to a comma-separated list of case-id substrings to
//! choose other cases.

use bibers_core::model::dump::{dump_state, json_diff, load_state};
use bibers_testkit::phase::{o3_available, phase_dumps};
use bibers_testkit::{corpus_root, discover};

const DEFAULT_CASES: &[&str] = &[
    "tdata/full-bbl",
    "tdata/names",
    "tdata/crossrefs",
    "tdata/xdata-noloop",
    "tdata/set-dynamic",
    "tdata/related",
    "tdata/uniqueness1",
    "styles/alphabetic",
];

#[test]
fn phase_dumps_round_trip() {
    if !o3_available() {
        eprintln!("skipping: oracle O3 not built");
        return;
    }
    let root = corpus_root();
    let cases = discover(&root).unwrap();
    let wanted: Vec<String> = std::env::var("BIBERS_PHASE_CASES")
        .map(|s| s.split(',').map(str::to_string).collect())
        .unwrap_or_else(|_| DEFAULT_CASES.iter().map(|s| s.to_string()).collect());
    let mut failures = Vec::new();
    let mut checked = 0;
    for w in &wanted {
        let case = cases
            .iter()
            .find(|c| c.id == *w || c.id.contains(w.as_str()))
            .unwrap_or_else(|| panic!("no case matching {w}"));
        let dumps = phase_dumps(case, &root).unwrap();
        if dumps.is_empty() {
            // biber stopped before the first phase (an error case).
            eprintln!("{}: no phase dumps (biber stopped early)", case.id);
            continue;
        }
        for d in &dumps {
            let state = d.state().unwrap();
            let model = load_state(&state).unwrap_or_else(|e| panic!("{}: {e}", d.path.display()));
            let again = dump_state(&model);
            let diffs = json_diff(&state, &again, &[], 10);
            if !diffs.is_empty() {
                failures.push(format!("{}:\n  {}", d.path.display(), diffs.join("\n  ")));
            }
            checked += 1;
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} dumps did not round-trip:\n{}",
        failures.len(),
        failures.join("\n")
    );
    eprintln!("{checked} phase dumps round-tripped");
}

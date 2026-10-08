//! Corpus ratchet: every case listed in `tests/corpus/passing.txt` must pass,
//! and every case in the corpus must be well-formed. Run the full differential
//! corpus with `cargo xtask corpus`.

use bibers_testkit::{Outcome, Runner, corpus_root, discover};

#[test]
fn corpus_is_well_formed() {
    let root = corpus_root();
    let cases = discover(&root).expect("corpus loads");
    for case in &cases {
        case.input_files(&root).expect("inputs resolve");
        let meta = case.load_meta().expect("meta parses");
        assert!(meta.is_some(), "{}: missing expected/meta.toml", case.id);
    }
}

#[test]
fn ratchet_cases_pass() {
    let root = corpus_root();
    let listed = std::fs::read_to_string(root.join("passing.txt")).unwrap_or_default();
    let wanted: Vec<&str> = listed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let cases = discover(&root).expect("corpus loads");
    let runner = Runner::new("bibers", vec![env!("CARGO_BIN_EXE_bibers").to_string()]);
    let mut failures = Vec::new();
    for id in wanted {
        let Some(case) = cases.iter().find(|c| c.id == id) else {
            failures.push(format!("{id}: listed in passing.txt but not in the corpus"));
            continue;
        };
        let run = runner.run(case, &root, false).expect("run");
        match bibers_testkit::compare_bibers(case, &run).expect("compare") {
            Outcome::Pass => {}
            Outcome::Fail(ms) => failures.push(format!(
                "{id}:\n{}",
                ms.iter()
                    .map(|m| format!("  {m}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "ratchet regressions:\n{}",
        failures.join("\n")
    );
}

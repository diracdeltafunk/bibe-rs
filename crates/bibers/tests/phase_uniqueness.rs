//! Phase tests for the uniqueness package: `uniqueness` (the iterative
//! uniquename/uniquelist loop) against biber's phase dumps (oracle O3;
//! skipped when it is not built), plus DC-001 and a performance check.

mod common;

use common::phase::{cases_with_prefix, check_phase};

/// Cases (id prefixes) excluded from a suite, with the reason.
const SKIP: &[(&str, &str)] = &[
    ("ported/tool-", "tool mode is deferred (DC-030)"),
    (
        "ported/bibtex-output-",
        "bibtex output is deferred (DC-030)",
    ),
    ("ported/full-bibtex-", "bibtex output is deferred (DC-030)"),
];

fn run(prefixes: &[&str]) {
    if !bibers_testkit::phase::o3_available() {
        eprintln!("skipping uniqueness: oracle O3 not built");
        return;
    }
    let ids: Vec<String> = cases_with_prefix(prefixes)
        .into_iter()
        .filter(|c| !SKIP.iter().any(|(s, _)| c.starts_with(s)))
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (checked, failures) = check_phase("uniqueness", &refs, &[]);
    eprintln!(
        "uniqueness {prefixes:?}: {}/{checked} occurrences match biber",
        checked - failures.len()
    );
    assert!(
        failures.is_empty(),
        "uniqueness: {} of {checked} occurrences differ from biber:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn tdata() {
    run(&["tdata/"]);
}

#[test]
fn biblatex_examples() {
    run(&["biblatex-examples/"]);
}

#[test]
fn styles() {
    run(&["styles/"]);
}

#[test]
fn ported() {
    run(&["ported/"]);
}

#[test]
fn bugs() {
    run(&["bugs/"]);
}

#[test]
fn scale() {
    run(&["scale/"]);
}

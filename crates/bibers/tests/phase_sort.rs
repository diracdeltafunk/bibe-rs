//! Phase tests for the sort package: list filtering (run by the harness
//! before `generate_sortdataschema`), `generate_sortdataschema`,
//! `generate_sortinfo` (sort data, sortinit) and `sort_list` (key order),
//! against biber's phase dumps (oracle O3; skipped when it is not built).

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

fn cases(prefixes: &[&str]) -> Vec<String> {
    cases_with_prefix(prefixes)
        .into_iter()
        .filter(|c| !SKIP.iter().any(|(s, _)| c.starts_with(s)))
        .collect()
}

fn run(phase: &str, prefixes: &[&str]) {
    if !bibers_testkit::phase::o3_available() {
        eprintln!("skipping {phase}: oracle O3 not built");
        return;
    }
    let ids = cases(prefixes);
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (checked, failures) = check_phase(phase, &refs, &[]);
    eprintln!(
        "{phase} {prefixes:?}: {}/{checked} occurrences match biber",
        checked - failures.len()
    );
    assert!(
        failures.is_empty(),
        "{phase}: {} of {checked} occurrences differ from biber:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

macro_rules! suite {
    ($name:ident, $($p:literal),+) => {
        mod $name {
            #[test]
            fn generate_sortdataschema() {
                super::run("generate_sortdataschema", &[$($p),+]);
            }
            #[test]
            fn generate_sortinfo() {
                super::run("generate_sortinfo", &[$($p),+]);
            }
            #[test]
            fn sort_list() {
                super::run("sort_list", &[$($p),+]);
            }
        }
    };
}

suite!(tdata, "tdata/");
suite!(biblatex_examples, "biblatex-examples/");
suite!(styles, "styles/");
suite!(ported, "ported/");
suite!(bugs, "bugs/");
suite!(scale, "scale/");

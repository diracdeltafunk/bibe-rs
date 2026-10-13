//! Phase tests for the entries package: `process_entries_static`,
//! `process_entries_pre`, `process_visible_names`, `process_entries_post`,
//! `process_entries_final` and `generate_contextdata`, against biber's phase
//! dumps (oracle O3; skipped when it is not built).

mod common;

use common::phase::{cases_with_prefix, check_phase};

/// Cases (id prefixes) excluded from a suite, with the reason.
const SKIP: &[(&str, &str)] = &[
    ("ported/tool-", "tool mode skips these phases"),
    (
        "ported/bibtex-output-",
        "bibtex output (tool option set) skips these phases",
    ),
    (
        "ported/full-bibtex-",
        "bibtex output (tool option set) skips these phases",
    ),
];

// Diff lines mentioning `=HASH(0x…)` are skipped: biber stringifies a
// name list used as `labeltitle` with its memory address (nondeterministic);
// bibers uses a fixed fake address per name list (`entries::perl_str`).

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
    let failures: Vec<String> = failures
        .into_iter()
        .filter_map(|f| {
            let mut lines = f.lines();
            let head = lines.next().unwrap_or("").to_string();
            let rest: Vec<&str> = lines.filter(|l| !l.contains("=HASH(0x")).collect();
            (!rest.is_empty()).then(|| format!("{head}\n{}", rest.join("\n")))
        })
        .collect();
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
            fn process_entries_static() {
                super::run("process_entries_static", &[$($p),+]);
            }
            #[test]
            fn process_entries_pre() {
                super::run("process_entries_pre", &[$($p),+]);
            }
            #[test]
            fn process_visible_names() {
                super::run("process_visible_names", &[$($p),+]);
            }
            #[test]
            fn process_entries_post() {
                super::run("process_entries_post", &[$($p),+]);
            }
            #[test]
            fn process_entries_final() {
                super::run("process_entries_final", &[$($p),+]);
            }
            #[test]
            fn generate_contextdata() {
                super::run("generate_contextdata", &[$($p),+]);
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

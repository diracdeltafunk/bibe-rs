//! Offline differential checks: recompute the Rust keys for every suite and compare
//! their digests with those blessed from the Perl oracle (`tests/differential.rs`).
//! No Perl needed.

mod common;

use std::collections::HashMap;

use common::*;

fn check(group: &str, suites: Vec<Suite>) {
    let path = crate_dir().join(format!("tests/data/digests-{group}.txt"));
    let text = std::fs::read_to_string(&path).unwrap();
    let expected: HashMap<&str, (usize, &str)> = text
        .lines()
        .map(|l| {
            let mut f = l.split('\t');
            let name = f.next().unwrap();
            let n = f.next().unwrap().parse().unwrap();
            (name, (n, f.next().unwrap()))
        })
        .collect();
    assert_eq!(expected.len(), suites.len(), "{group}: suite count");
    let mut bad = Vec::new();
    for s in &suites {
        let (n, d) = expected[s.name.as_str()];
        assert_eq!(n, s.inputs.len(), "{}: input count", s.name);
        let got = digest(&rust_keys(&s.spec.collator(), &s.inputs));
        if got != d {
            bad.push(s.name.clone());
        }
    }
    assert!(
        bad.is_empty(),
        "{} {group} suites differ from the oracle (run the ignored differential tests): {}",
        bad.len(),
        bad.join(" ")
    );
}

#[test]
fn digests_all_code_points() {
    check("allcp", allcp_suites());
}

#[test]
fn digests_locales() {
    check("locales", locale_suites());
}

#[test]
fn digests_options() {
    check("options", option_suites());
}

#[test]
fn locale_names() {
    let text = std::fs::read_to_string(crate_dir().join("tests/data/locale-names.tsv")).unwrap();
    let mut n = 0;
    for line in text.lines() {
        let (name, accepted) = line.split_once('\t').unwrap();
        assert_eq!(ucollate::resolve_locale(Some(name)), accepted, "{name:?}");
        n += 1;
    }
    assert!(n > 8000);
}

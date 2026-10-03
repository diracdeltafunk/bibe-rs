//! Every experiment cited in docs/spec/btparse.md, replayed against the
//! oracle results recorded in tests/data/spec_cases.jsonl.
mod common;

/// Vectors whose behaviour we knowingly do not reproduce (see spec §10).
const ALLOW: &[&str] = &[];

#[test]
fn spec_cases() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/spec_cases.jsonl");
    let (total, bad) = common::run_file(path, 40, ALLOW);
    assert!(
        bad.is_empty(),
        "{} of {total} spec vectors differ: {:?}",
        bad.len(),
        bad
    );
}

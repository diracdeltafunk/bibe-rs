//! Differential test against randomly generated oracle vectors
//! (`tools/oracle/probes/btparse/gen.pl` + `vectors.pl`).
//!
//! `cargo test -p btparse --test fuzz_vectors` checks the committed curated
//! set. For a large local run:
//!
//! ```sh
//! P=tools/oracle/probes/btparse; PERL=vendor/perl/bin/perl
//! $PERL $P/gen.pl 7 100000 > /tmp/req.jsonl
//! $PERL -I$P $P/vectors.pl < /tmp/req.jsonl > /tmp/vec.jsonl
//! BTPARSE_VECTORS=/tmp/vec.jsonl cargo test -p btparse --release --test fuzz_vectors -- --ignored --nocapture
//! ```
mod common;

use common::{Outcome, check, is_name_irregular};
use std::collections::BTreeMap;

/// btparse prints the offending token as part of a printf format (spec section 8.3);
/// conversions such as `%e` or `%c` print stack garbage we cannot
/// reproduce. Such vectors are reported separately.
fn is_printf_garbage(
    v: &serde_json::Value,
    ours: &serde_json::Value,
    theirs: &serde_json::Value,
) -> bool {
    let input = v.to_string();
    if !(v["op"] == "parse" || v["op"] == "files") || !input.contains('%') {
        return false;
    }
    fn strip(x: &serde_json::Value) -> serde_json::Value {
        match x {
            serde_json::Value::Array(a) => serde_json::Value::Array(
                a.iter()
                    .filter(|s| {
                        !s.as_str().is_some_and(|s| {
                            s.contains("syntax error: found") || s.ends_with("syntax error: ")
                        })
                    })
                    .map(strip)
                    .collect(),
            ),
            serde_json::Value::Object(o) => {
                serde_json::Value::Object(o.iter().map(|(k, v)| (k.clone(), strip(v))).collect())
            }
            x => x.clone(),
        }
    }
    if theirs.get("crash").is_some() {
        // e.g. `%s` in the token: printf reads a bogus pointer and crashes
        return ours.to_string().contains("syntax error: found");
    }
    strip(ours) == strip(theirs)
}

fn stats(path: &str, show: usize) -> usize {
    let data = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    // op -> [total, match, crash-match, mismatch, mismatch-irregular]
    let mut st: BTreeMap<String, [usize; 5]> = BTreeMap::new();
    let mut shown = 0;
    let mut hard = 0;
    for line in data.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let op = v["op"].as_str().unwrap().to_string();
        let e = st.entry(op).or_default();
        e[0] += 1;
        match check(&v) {
            Outcome::Match => e[1] += 1,
            Outcome::CrashMatch => e[2] += 1,
            Outcome::Mismatch { ours, theirs } => {
                if is_name_irregular(&v) || is_printf_garbage(&v, &ours, &theirs) {
                    e[4] += 1;
                    continue;
                }
                e[3] += 1;
                hard += 1;
                if shown < show {
                    shown += 1;
                    eprintln!("--- mismatch {}", v["id"]);
                    eprintln!(
                        "input: {}",
                        v.get("text").or(v.get("s")).or(v.get("texts")).unwrap()
                    );
                    eprintln!("ours:   {ours}");
                    eprintln!("theirs: {theirs}");
                }
            }
        }
    }
    for (op, s) in &st {
        eprintln!(
            "{op:6} total {:7}  match {:7}  crash-match {:5}  MISMATCH {:5}  (excluded: irregular names or printf garbage: {})",
            s[0], s[1], s[2], s[3], s[4]
        );
    }
    hard
}

#[test]
fn curated_fuzz_vectors() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/fuzz_vectors.jsonl");
    let bad = stats(path, 20);
    assert_eq!(bad, 0, "{bad} curated fuzz vectors differ");
}

#[test]
#[ignore = "needs a vectors file from the Perl oracle (BTPARSE_VECTORS)"]
fn large_fuzz_run() {
    let path = std::env::var("BTPARSE_VECTORS").expect("set BTPARSE_VECTORS");
    let show = std::env::var("BTPARSE_SHOW")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let bad = stats(&path, show);
    assert_eq!(bad, 0, "{bad} vectors differ");
}

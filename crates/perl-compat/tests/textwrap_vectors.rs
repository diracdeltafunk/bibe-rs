//! Differential vectors from Text::Wrap 2024.001 in oracle O3
//! (`tools/oracle/probes/perl-compat/textwrap_vectors.pl`).

mod common;

use perl_compat::textwrap::wrap;
use serde_json::Value;

#[test]
fn text_wrap() {
    let v = common::read_json("textwrap_vectors.json");
    let mut fails = Vec::new();
    for t in v.as_array().unwrap() {
        let s = |k: &str| t[k].as_str().unwrap_or("").to_string();
        let cols: i64 = s("cols").parse().unwrap();
        let got = wrap(cols, &s("ip"), &s("xp"), &s("in"));
        if got != s("out") {
            fails.push(format!(
                "cols={cols} ip={:?} xp={:?} in={:?}\n  got  {got:?}\n  want {:?}",
                s("ip"),
                s("xp"),
                s("in"),
                s("out")
            ));
        }
    }
    assert!(
        fails.is_empty(),
        "{} failures:\n{}",
        fails.len(),
        fails
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
#[ignore = "needs oracle O3 perl"]
fn oracle_live() {
    let Some(out) = common::run_probe("textwrap_vectors.pl", &[], b"") else {
        return;
    };
    let live: Value = serde_json::from_slice(&out).unwrap();
    assert!(live == common::read_json("textwrap_vectors.json"));
}

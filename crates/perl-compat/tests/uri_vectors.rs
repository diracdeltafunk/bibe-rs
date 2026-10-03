//! Differential vectors from URI 5.34 in oracle O3
//! (`tools/oracle/probes/perl-compat/uri_vectors.pl`).

mod common;

use perl_compat::uri::uri_as_string;
use serde_json::Value;

#[test]
fn uri() {
    let v = common::read_json("uri_vectors.json");
    let mut fails = Vec::new();
    for t in v.as_array().unwrap() {
        let input = t["in"].as_str().unwrap();
        let got = uri_as_string(input).map_err(|e| e.to_string());
        let want = match t["out"].as_str() {
            Some(o) => Ok(o.to_string()),
            None => Err(t["err"].as_str().unwrap_or("").to_string()),
        };
        if got != want {
            fails.push(format!("{input:?}: got {got:?} want {want:?}"));
        }
    }
    assert!(fails.is_empty(), "{} failures:\n{}", fails.len(), fails.join("\n"));
}

#[test]
#[ignore = "needs oracle O3 perl"]
fn oracle_live() {
    let Some(out) = common::run_probe("uri_vectors.pl", &[], b"") else { return };
    let live: Value = serde_json::from_slice(&out).unwrap();
    assert!(live == common::read_json("uri_vectors.json"), "committed uri_vectors.json is stale");
}

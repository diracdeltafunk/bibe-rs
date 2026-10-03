//! Differential vectors from oracle O3 (`scalar_vectors.pl`): truthiness and
//! numbers, `Unicode::UCD::num`, `Text::Roman`, case mapping of strings,
//! `Unicode::GCString` and perl `\X` segmentation.

mod common;

use perl_compat::grapheme::{gcstring, perl};
use perl_compat::roman::{isroman, roman2int};
use perl_compat::truth::{PerlNum, looks_like_number};
use perl_compat::unicode::{nfkd, ucd_num};
use perl_compat::{fc, lc, lcfirst, uc, ucfirst};
use serde_json::Value;

fn vectors() -> Value {
    common::read_json("scalar_vectors.json")
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[test]
fn numbers() {
    let v = vectors();
    let mut fails = Vec::new();
    for t in v["numbers"].as_array().unwrap() {
        let x = s(&t["s"]);
        let n = PerlNum::from_str(x);
        let got = (
            looks_like_number(x),
            n.add(PerlNum::Int(0)).to_string(),
            n.abs().to_string(),
            n.sub(PerlNum::Int(1)).to_string(),
            n.add(PerlNum::Float(0.5)).to_string(),
        );
        let want = (
            t["lln"].as_bool().unwrap(),
            s(&t["num"]).to_string(),
            s(&t["abs"]).to_string(),
            s(&t["sub1"]).to_string(),
            s(&t["add2"]).to_string(),
        );
        if got != want {
            fails.push(format!("{x:?}: got {got:?} want {want:?}"));
        }
    }
    for t in v["numcmp"].as_array().unwrap() {
        let (a, b) = (PerlNum::from_str(s(&t["a"])), PerlNum::from_str(s(&t["b"])));
        let cmp = a.num_cmp(b).map(|o| o as i64);
        let want = t["cmp"].as_i64();
        let sub = a.sub(b).to_string();
        if cmp != want || sub != s(&t["sub"]) {
            fails.push(format!("{:?} <=> {:?}: got {cmp:?}/{sub} want {want:?}/{}", t["a"], t["b"], t["sub"]));
        }
    }
    assert!(fails.is_empty(), "{} failures:\n{}", fails.len(), fails.join("\n"));
}

#[test]
fn unicode_ucd_num() {
    let v = vectors();
    let mut fails = Vec::new();
    for t in v["ucd_num"].as_array().unwrap() {
        let got = ucd_num(s(&t["s"])).map(|n| n.to_string());
        let want = t["num"].as_str().map(str::to_string);
        if got != want {
            fails.push(format!("{:?}: got {got:?} want {want:?}", t["s"]));
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

#[test]
fn text_roman() {
    let v = vectors();
    let mut fails = Vec::new();
    for t in v["roman"].as_array().unwrap() {
        let k = nfkd(s(&t["s"]));
        assert_eq!(k, s(&t["nfkd"]));
        let got = (isroman(&k), roman2int(&k).map(i64::from));
        let want = (t["isroman"].as_bool().unwrap(), t["roman2int"].as_i64());
        if got != want {
            fails.push(format!("{:?}: got {got:?} want {want:?}", t["s"]));
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

#[test]
fn strings() {
    let v = vectors();
    let mut fails = Vec::new();
    for t in v["strings"].as_array().unwrap() {
        let x = s(&t["s"]);
        let lens = |it: &mut dyn Iterator<Item = &str>| -> Vec<u64> { it.map(|c| c.chars().count() as u64).collect() };
        let want_lens = |k: &str| -> Vec<u64> { t[k].as_array().unwrap().iter().map(|n| n.as_u64().unwrap()).collect() };
        if lens(&mut gcstring::clusters(x)) != want_lens("gcstring") {
            fails.push(format!("gcstring {x:?}"));
        }
        if gcstring::length(x) as u64 != t["gclength"].as_u64().unwrap() {
            fails.push(format!("gclength {x:?}"));
        }
        if lens(&mut perl::clusters(x)) != want_lens("perlx") {
            fails.push(format!("perlx {x:?}: got {:?} want {:?}", lens(&mut perl::clusters(x)), want_lens("perlx")));
        }
        let mut subs = Vec::new();
        for o in [-3i64, -1, 0, 1, 2] {
            for l in [None, Some(-2i64), Some(0), Some(1), Some(3)] {
                subs.push(gcstring::substr(x, o, l).to_string());
            }
        }
        let want_subs: Vec<String> = t["substr"].as_array().unwrap().iter().map(|v| s(v).to_string()).collect();
        if subs != want_subs {
            fails.push(format!("substr {x:?}: got {subs:?} want {want_subs:?}"));
        }
        for (k, f) in [("lc", lc as fn(&str) -> String), ("uc", uc), ("fc", fc), ("ucfirst", ucfirst), ("lcfirst", lcfirst)] {
            if f(x) != s(&t[k]) {
                fails.push(format!("{k} {x:?}"));
            }
        }
    }
    assert!(fails.is_empty(), "{} failures:\n{}", fails.len(), fails.join("\n"));
}

#[test]
#[ignore = "needs oracle O3 perl"]
fn oracle_live() {
    let Some(out) = common::run_probe("scalar_vectors.pl", &[], b"") else { return };
    let live: Value = serde_json::from_slice(&out).unwrap();
    assert!(live == vectors(), "committed scalar_vectors.json is stale");
}

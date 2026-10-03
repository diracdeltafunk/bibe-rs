//! Exhaustive per-code-point comparison with perl 5.42 (oracle O3) through
//! SHA-256 digests (see `tools/oracle/probes/perl-compat/unicode_digests.pl`
//! for the serialisation). `tests/data/unicode_digests.json` holds the
//! oracle's digests, so this runs without perl; `oracle_live` regenerates them.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use perl_compat::grapheme::{gcstring, perl};
use perl_compat::unicode::{Prop, nfc, nfd, nfkc, nfkd, ucd_num};
use perl_compat::{fc, lc, uc, ucfirst};
use sha2::{Digest, Sha256};

const PARTNERS: &[u32] = &[
    0x61, 0x301, 0x200D, 0x1F1E6, 0x1100, 0x1161, 0x11A8, 0xAC00, 0xAC01, 0x20, 0xD, 0xA, 0x94D,
    0x915, 0x600, 0x1F600, 0x1F3FB, 0x903, 0xE33, 0x1F1E6, 0xFE0F,
];

fn context(c: char) -> String {
    let mut s = String::new();
    for &p in PARTNERS {
        s.push(c);
        s.push(char::from_u32(p).unwrap());
    }
    s.push(c);
    s
}

fn chars() -> impl Iterator<Item = char> {
    (0..=0x10FFFFu32).filter_map(char::from_u32)
}

fn hexes(s: &str) -> String {
    s.chars()
        .map(|c| format!("{:X}", c as u32))
        .collect::<Vec<_>>()
        .join(" ")
}

fn digest_map(f: impl Fn(&str) -> String) -> String {
    let mut h = Sha256::new();
    let mut buf = [0u8; 4];
    let mut line = String::new();
    for c in chars() {
        let s = c.encode_utf8(&mut buf);
        let m = f(s);
        if m != *s {
            line.clear();
            let _ = writeln!(line, "{:X}:{}", c as u32, hexes(&m));
            h.update(line.as_bytes());
        }
    }
    format!("{:x}", h.finalize())
}

fn digest_prop(p: Prop) -> String {
    let mut h = Sha256::new();
    for c in chars() {
        if p.contains(c) {
            h.update(format!("{:X}\n", c as u32).as_bytes());
        }
    }
    format!("{:x}", h.finalize())
}

fn digest_num() -> String {
    let mut h = Sha256::new();
    let mut buf = [0u8; 4];
    for c in chars() {
        if let Some(v) = ucd_num(c.encode_utf8(&mut buf)) {
            h.update(format!("{:X}:{}\n", c as u32, v).as_bytes());
        }
    }
    format!("{:x}", h.finalize())
}

fn digest_clusters(f: impl Fn(&str) -> Vec<usize>) -> String {
    let mut h = Sha256::new();
    for c in chars() {
        let lens: Vec<String> = f(&context(c)).iter().map(ToString::to_string).collect();
        h.update(format!("{:X}:{}\n", c as u32, lens.join(",")).as_bytes());
    }
    format!("{:x}", h.finalize())
}

fn compute(key: &str) -> Option<String> {
    Some(match key {
        "lc" => digest_map(lc),
        "uc" => digest_map(uc),
        "fc" => digest_map(fc),
        "tc" => digest_map(ucfirst),
        "nfd" => digest_map(|s| nfd(s).into_owned()),
        "nfc" => digest_map(|s| nfc(s).into_owned()),
        "nfkd" => digest_map(|s| nfkd(s).into_owned()),
        "nfkc" => digest_map(|s| nfkc(s).into_owned()),
        "num" => digest_num(),
        "gcstring" => {
            digest_clusters(|s| gcstring::clusters(s).map(|x| x.chars().count()).collect())
        }
        "perlx" => digest_clusters(|s| perl::clusters(s).map(|x| x.chars().count()).collect()),
        k => {
            let syn = k.strip_prefix("prop:")?;
            let p = Prop::ALL.iter().copied().find(|p| p.perl_syntax() == syn)?;
            digest_prop(p)
        }
    })
}

fn check(expected: &BTreeMap<String, String>, keys: &[&str]) {
    let mut bad = Vec::new();
    for k in keys {
        let want = expected
            .get(*k)
            .unwrap_or_else(|| panic!("no digest for {k}"));
        let got = compute(k).unwrap_or_else(|| panic!("unknown key {k}"));
        if &got != want {
            bad.push(k.to_string());
        }
    }
    assert!(bad.is_empty(), "differs from perl 5.42: {bad:?}");
}

fn committed() -> BTreeMap<String, String> {
    serde_json::from_value(common::read_json("unicode_digests.json")).unwrap()
}

#[test]
fn case_mappings() {
    check(&committed(), &["lc", "uc", "fc", "tc"]);
}

#[test]
fn normalization() {
    check(&committed(), &["nfd", "nfc", "nfkd", "nfkc"]);
}

#[test]
fn properties_and_num() {
    let exp = committed();
    let keys: Vec<String> = Prop::ALL
        .iter()
        .map(|p| format!("prop:{}", p.perl_syntax()))
        .collect();
    let mut keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    keys.push("num");
    check(&exp, &keys);
}

#[test]
fn graphemes_gcstring() {
    check(&committed(), &["gcstring"]);
}

#[test]
fn graphemes_perl_x() {
    check(&committed(), &["perlx"]);
}

/// Re-derive the digests from the live oracle and compare with both the
/// committed copy and this implementation.
#[test]
#[ignore = "needs oracle O3 perl"]
fn oracle_live() {
    let Some(out) = common::run_probe("unicode_digests.pl", &[], b"") else {
        return;
    };
    let live: BTreeMap<String, String> = serde_json::from_slice(&out).unwrap();
    assert_eq!(live, committed(), "committed digests are stale");
}

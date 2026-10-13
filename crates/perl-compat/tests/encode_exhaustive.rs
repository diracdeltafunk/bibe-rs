//! Exhaustive comparison of `perl_compat::encoding` with Perl's Encode 3.21
//! (oracle O3), through the digests of
//! `tools/oracle/probes/perl-compat/encode_digests.pl` (committed in
//! `tests/data/encode_digests.json`): for every supported encoding, encoding
//! all of Unicode (with a code-reference CHECK and through a chunked PerlIO
//! layer) and strictly decoding every 1- and 2-byte sequence (and the EUC
//! 3-byte ones); plus `find_encoding` over ~2,000 candidate names.

mod common;

use perl_compat::encoding::{Lookup, find_encoding, lookup};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn all() -> String {
    let mut s = String::new();
    for (i, c) in (0..=0x10FFFFu32).filter_map(char::from_u32).enumerate() {
        if i > 0 {
            s.push('\n');
        }
        s.push(c);
    }
    s
}

fn hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn digests(name: &str, all: &str) -> (String, String, String, String) {
    let e = find_encoding(name).unwrap_or_else(|| panic!("{name}"));
    let cb = |c: u32| format!("<{c:X}>").into_bytes();
    let cbd = hex(Sha256::digest(e.encode_cb(all, &cb)));
    let layer = hex(Sha256::digest(e.encode_layer(all, true)));
    let def = hex(Sha256::digest(e.encode_default(all)));
    let mut seqs: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
    for a in 0..=255u8 {
        seqs.extend((0..=255u8).map(|b| vec![a, b]));
    }
    for a in [0x8Eu8, 0x8F] {
        for b in 0xA1..=0xFEu8 {
            seqs.extend((0xA1..=0xFEu8).map(|c| vec![a, b, c]));
        }
    }
    let mut h = Sha256::new();
    // ENCODE_DUMP=<dir>: write the decode lines (to diff with the probe's
    // `ENCODE_DUMP` output when a digest differs).
    let dump = std::env::var_os("ENCODE_DUMP").map(|d| {
        std::cell::RefCell::new(
            std::fs::File::create(std::path::Path::new(&d).join(format!("{name}.rs.txt"))).unwrap(),
        )
    });
    for s in &seqs {
        let r = match e.decode_layer(s) {
            Ok(t) => t
                .chars()
                .map(|c| format!("{:X}", c as u32))
                .collect::<Vec<_>>()
                .join(" "),
            Err(m) => format!("ERR {m}"),
        };
        let line = format!("{}:{}\n", hex(s), r);
        if let Some(d) = &dump {
            use std::io::Write as _;
            d.borrow_mut().write_all(line.as_bytes()).unwrap();
        }
        h.update(line.as_bytes());
    }
    (cbd, layer, hex(h.finalize()), def)
}

fn load() -> Value {
    let p = common::data("encode_digests.json");
    serde_json::from_slice(&std::fs::read(p).expect("encode_digests.json")).expect("json")
}

#[test]
fn aliases_match_perl() {
    let v = load();
    let mut bad = Vec::new();
    for (name, want) in v["aliases"].as_object().unwrap() {
        let got = match lookup(name) {
            Lookup::Found(e) => Some(e.name()),
            Lookup::Unsupported(n) => Some(n),
            Lookup::Unknown => None,
        };
        if got != want.as_str() {
            bad.push(format!("{name}: perl {want}, ours {got:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} mismatches:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn encodings_match_perl() {
    let v = load();
    let all = all();
    let encs = v["encodings"].as_object().unwrap();
    let filter = std::env::var("ENCODE_ONLY").ok();
    let mut bad = Vec::new();
    for (name, d) in encs {
        if filter
            .as_deref()
            .is_some_and(|f| !f.split(',').any(|x| x == name))
        {
            continue;
        }
        let (cb, layer, dec, def) = digests(name, &all);
        for (k, got) in [("cb", cb), ("layer", layer), ("dec", dec), ("def", def)] {
            // No `dec` for the needs_lines encodings (perl hangs reading
            // malformed input; see encode_digests.pl).
            if d[k].is_null() {
                continue;
            }
            // Lax `utf8` decodes surrogates and values above U+10FFFF, which
            // Rust strings cannot hold (U+FFFD instead; docs/compat.md DC-019).
            if name == "utf8" && k == "dec" {
                continue;
            }
            if d[k].as_str() != Some(&got) {
                bad.push(format!("{name} {k}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} mismatches: {}",
        bad.len(),
        bad.join(", ")
    );
}

/// Re-run the probe for a few encodings with O3 and compare with the
/// committed digests (`cargo test -p perl-compat -- --ignored`).
#[test]
#[ignore]
fn oracle_live() {
    let encs = ["cp1252", "shiftjis", "euc-jp", "UTF-16", "iso-2022-jp"];
    let Some(out) = common::run_probe("encode_digests.pl", &encs, b"") else {
        return;
    };
    let live: Value = serde_json::from_slice(&out).expect("probe json");
    let committed = load();
    for e in encs {
        let name = perl_compat::encoding::find_encoding(e).unwrap().name();
        assert_eq!(
            live["encodings"][name], committed["encodings"][name],
            "{name}: regenerate tests/data/encode_digests.json"
        );
    }
    assert_eq!(live["aliases"], committed["aliases"]);
}

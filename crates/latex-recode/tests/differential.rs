//! Differential tests against biber's own `Biber::LaTeX::Recode` running on
//! oracle O3 (perl 5.42). They need the O3 perl, so they are `#[ignore]`d:
//!
//! ```sh
//! RECODE_N=100000 cargo test -p latex-recode --release --test differential -- --ignored --nocapture
//! ```
//!
//! `regenerate_vectors` rewrites the committed vector file used by
//! `tests/vectors.rs`.

mod common;

use common::*;
use latex_recode::{Normalization, RecodeData, RecodeOptions, Recoder};

fn n_cases(default: usize) -> usize {
    std::env::var("RECODE_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn seed() -> u64 {
    std::env::var("RECODE_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// Returns (cases, mismatches).
fn diff_batch(init: &str, rec: &Recoder, op: &str, inputs: &[String]) -> (usize, Vec<String>) {
    let reqs: Vec<(&str, &str)> = inputs.iter().map(|s| (op, s.as_str())).collect();
    let mut bad = Vec::new();
    for chunk_start in (0..reqs.len()).step_by(20000) {
        let chunk = &reqs[chunk_start..(chunk_start + 20000).min(reqs.len())];
        let got = run_oracle(init, chunk);
        for ((_, input), want) in chunk.iter().zip(got) {
            let ours = match op {
                "d" => rec.decode(input),
                "dn" => rec.decode_with(input, Normalization::None),
                "e" => rec.encode(input),
                _ => unreachable!(),
            };
            match want {
                Ok(w) if w == ours => {}
                Ok(w) => bad.push(format!(
                    "[{init}] {op} \"{}\"\n   perl: \"{}\"\n   rust: \"{}\"",
                    esc(input),
                    esc(&w),
                    esc(&ours)
                )),
                Err(e) => bad.push(format!("[{init}] {op} \"{}\": perl died: {e}", esc(input))),
            }
        }
    }
    (inputs.len(), bad)
}

fn report(name: &str, total: usize, bad: &[String]) {
    eprintln!("{name}: {} cases, {} mismatches", total, bad.len());
    for b in bad.iter().take(40) {
        eprintln!("{b}");
    }
    assert!(bad.is_empty(), "{name}: {} mismatches", bad.len());
}

#[test]
#[ignore = "needs oracle O3"]
fn diff_graphemes() {
    let mut r = Rng::new(seed());
    let k = Keys::builtin();
    let n = n_cases(20000);
    let inputs: Vec<String> = (0..n).map(|_| gen_encode(&mut r, &k)).collect();
    let reqs: Vec<(&str, &str)> = inputs.iter().map(|s| ("x", s.as_str())).collect();
    let got = run_oracle("base base", &reqs);
    let mut bad = Vec::new();
    for (s, w) in inputs.iter().zip(got) {
        let ours: Vec<&str> = latex_recode::grapheme_clusters(s);
        let w = w.unwrap();
        let ours_joined = ours.join("\u{1}");
        if w != ours_joined {
            bad.push(format!(
                "\"{}\": perl {:?} rust {:?}",
                esc(s),
                w,
                ours_joined
            ));
        }
    }
    report("graphemes", n, &bad);
}

fn diff_decode_set(set: &str, n: usize, seed: u64) {
    let mut r = Rng::new(seed);
    let k = Keys::builtin();
    let inputs: Vec<String> = (0..n).map(|_| gen_decode(&mut r, &k)).collect();
    let rec = Recoder::builtin(set, set, RecodeOptions::BIBER);
    let (total, bad) = diff_batch(&format!("{set} {set}"), &rec, "d", &inputs);
    report(&format!("decode/{set}"), total, &bad);
}

#[test]
#[ignore = "needs oracle O3"]
fn diff_decode_base() {
    diff_decode_set("base", n_cases(20000), seed());
}

#[test]
#[ignore = "needs oracle O3"]
fn diff_decode_full() {
    diff_decode_set("full", n_cases(20000), seed() + 1000);
}

#[test]
#[ignore = "needs oracle O3"]
fn diff_decode_null() {
    diff_decode_set("null", n_cases(20000) / 4, seed() + 2000);
}

#[test]
#[ignore = "needs oracle O3"]
fn diff_decode_unnormalised() {
    let mut r = Rng::new(seed() + 3000);
    let k = Keys::builtin();
    let n = n_cases(20000) / 4;
    let inputs: Vec<String> = (0..n).map(|_| gen_decode(&mut r, &k)).collect();
    let rec = Recoder::builtin("full", "full", RecodeOptions::BIBER);
    let (total, bad) = diff_batch("full full", &rec, "dn", &inputs);
    report("decode-nonorm/full", total, &bad);
}

#[test]
#[ignore = "needs oracle O3"]
fn diff_decode_chaos() {
    let k = Keys::builtin();
    for (i, set) in ["base", "full", "null"].into_iter().enumerate() {
        let mut r = Rng::new(seed() + 8000 + i as u64);
        let inputs: Vec<String> = (0..n_cases(20000)).map(|_| gen_chaos(&mut r, &k)).collect();
        let rec = Recoder::builtin(set, set, RecodeOptions::BIBER);
        let (total, bad) = diff_batch(&format!("{set} {set}"), &rec, "d", &inputs);
        report(&format!("decode-chaos/{set}"), total, &bad);
    }
}

#[test]
#[ignore = "needs oracle O3"]
fn diff_encode_chaos() {
    let k = Keys::builtin();
    for (i, set) in ["base", "full"].into_iter().enumerate() {
        let mut r = Rng::new(seed() + 9000 + i as u64);
        let inputs: Vec<String> = (0..n_cases(20000))
            .map(|_| {
                let s = gen_chaos(&mut r, &k) + &gen_encode(&mut r, &k);
                Normalization::Nfd.apply(&s)
            })
            .collect();
        let rec = Recoder::builtin(set, set, RecodeOptions::BIBER);
        let (total, bad) = diff_batch(&format!("{set} {set}"), &rec, "e", &inputs);
        report(&format!("encode-chaos/{set}"), total, &bad);
    }
}

fn diff_encode_set(set: &str, n: usize, seed: u64) {
    let mut r = Rng::new(seed);
    let k = Keys::builtin();
    let inputs: Vec<String> = (0..n).map(|_| gen_encode(&mut r, &k)).collect();
    let rec = Recoder::builtin(set, set, RecodeOptions::BIBER);
    let (total, bad) = diff_batch(&format!("{set} {set}"), &rec, "e", &inputs);
    report(&format!("encode/{set}"), total, &bad);
}

#[test]
#[ignore = "needs oracle O3"]
fn diff_encode_base() {
    diff_encode_set("base", n_cases(20000), seed() + 4000);
}

#[test]
#[ignore = "needs oracle O3"]
fn diff_encode_full() {
    diff_encode_set("full", n_cases(20000), seed() + 5000);
}

/// Round trip: encode random Unicode, then decode the result.
#[test]
#[ignore = "needs oracle O3"]
fn diff_decode_of_encoded() {
    let mut r = Rng::new(seed() + 6000);
    let k = Keys::builtin();
    let n = n_cases(20000) / 2;
    for set in ["base", "full"] {
        let rec = Recoder::builtin(set, set, RecodeOptions::BIBER);
        let inputs: Vec<String> = (0..n)
            .map(|_| rec.encode(&gen_encode(&mut r, &k)))
            .collect();
        let (total, bad) = diff_batch(&format!("{set} {set}"), &rec, "d", &inputs);
        report(&format!("decode-of-encoded/{set}"), total, &bad);
    }
}

/// A custom `--recodedata` file with its own set names, preferred/raw maps,
/// exclusions and overlapping keys.
#[test]
#[ignore = "needs oracle O3"]
fn diff_custom_data() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/custom_recode.xml");
    let data = RecodeData::from_xml(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut r = Rng::new(seed() + 7000);
    let k = Keys::builtin();
    let n = n_cases(20000) / 4;
    for (d, e) in [("mine", "mine"), ("other", "mine"), ("base", "other")] {
        let rec = Recoder::new(&data, d, e, RecodeOptions::BIBER);
        let init = format!("{d} {e} {}", path.display());
        let mut dec: Vec<String> = (0..n).map(|_| gen_decode(&mut r, &k)).collect();
        dec.extend(
            [
                "\\foo{}x",
                "\\foobar",
                "\\foo bar",
                "\\fo{}",
                "\\'{e}",
                "\\~n",
                "\\zz{a}",
                "\\zz a",
                "\\textsuperscript{q}",
                "\\not\\eq",
            ]
            .map(String::from),
        );
        let (t1, b1) = diff_batch(&init, &rec, "d", &dec);
        let enc: Vec<String> = (0..n).map(|_| gen_encode(&mut r, &k)).collect();
        let (t2, b2) = diff_batch(&init, &rec, "e", &enc);
        report(&format!("custom decode {d}"), t1, &b1);
        report(&format!("custom encode {e}"), t2, &b2);
    }
}

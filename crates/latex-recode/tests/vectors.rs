//! Committed differential vectors (`tests/data/vectors.json`, generated from
//! oracle O3 by the ignored `regenerate_vectors` test) and the latex-recode
//! unit vectors extracted from biber's own `t/utils.t`
//! (`tests/ported/utils.json`).

mod common;

use std::path::{Path, PathBuf};

use common::*;
use latex_recode::{RecodeData, RecodeOptions, Recoder};
use serde_json::{Value, json};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn custom_path() -> PathBuf {
    manifest().join("tests/data/custom_recode.xml")
}

fn recoder(data: &str, d: &str, e: &str) -> Recoder {
    match data {
        "builtin" => Recoder::new(RecodeData::builtin(), d, e, RecodeOptions::BIBER),
        "custom" => Recoder::new(
            &RecodeData::from_xml(&std::fs::read_to_string(custom_path()).unwrap()).unwrap(),
            d,
            e,
            RecodeOptions::BIBER,
        ),
        _ => panic!("unknown data {data}"),
    }
}

fn apply(r: &Recoder, op: &str, input: &str) -> String {
    match op {
        "d" => r.decode(input),
        "dn" => r.decode_with(input, latex_recode::Normalization::None),
        "e" => r.encode(input),
        _ => panic!("op {op}"),
    }
}

#[test]
fn committed_vectors() {
    let path = manifest().join("tests/data/vectors.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut n = 0;
    let mut bad = Vec::new();
    for group in v["groups"].as_array().unwrap() {
        let data = group["data"].as_str().unwrap();
        let d = group["decode_set"].as_str().unwrap();
        let e = group["encode_set"].as_str().unwrap();
        let op = group["op"].as_str().unwrap();
        let r = recoder(data, d, e);
        for case in group["cases"].as_array().unwrap() {
            let input = case[0].as_str().unwrap();
            let want = case[1].as_str().unwrap();
            let got = apply(&r, op, input);
            n += 1;
            if got != want {
                bad.push(format!(
                    "[{data} {d} {e}] {op} \"{}\"\n  want \"{}\"\n  got  \"{}\"",
                    esc(input),
                    esc(want),
                    esc(&got)
                ));
            }
        }
    }
    for b in bad.iter().take(30) {
        eprintln!("{b}");
    }
    assert!(bad.is_empty(), "{} of {n} vectors differ", bad.len());
    assert!(n > 10_000, "only {n} vectors");
}

/// Hand-picked edge cases (quirks reproduced from biber), all checked
/// against the oracle when vectors are regenerated.
const EDGE_DECODE: &[&str] = &[
    "\\alpha{} \\textdollar{}5 \\i and",
    "{I} {\\ae}x \\char\"41\\char'101\\char65 \\char012 \\char09 \\char'89",
    "\\c  , \\t{oo} \\' {a}",
    "\\char\"110000 \\char\"D800 \\char\"7FFFFFFF",
    "\\char99999999999999999999 \\char\"FFFFFFFFFFFFFFFFFF \\char\"7FFFFFFFFFFFFFFF \\char\"8000000000000000",
    "\\char9223372036854775807 \\char9223372036854775808 \\char'1000000000000000000000",
    "\\char\"00000000000000000000041 \\char1٣ \\char0 \\char٣ \\char'٣",
    "\\--- \\--x \\---a \\-- \\textendash{} \\textendash {}",
    "\\textdollar \\textpercent \\textampersand{}x \\pounds{}7 \\ss{}x",
    "{\\\"{U}}ber {\\\"{U}}berlegungen zur \\\"{U}berwindung des \\\"{U}bels",
    "\\'\\i {\\'\\i} \\^{\\j} \\u{\\i} \\u\\i {{\\'A}lvarez}, J.~D. \\\"\\i",
    "{\\'{I}} {\\v{C}} {I} \\&{A} \\&\\;{A} a\\-a -- \\textdegree C",
    "{M{\\'a}t{\\'e}} {M\\'{a}t\\'{e}} {M\\'at\\'e} R{\\'egis} \\frac{a}{b} \\textuppercase{\\'e}",
    "\\DH{}and\\dj{}and\\'{c}, H. {\\DH{}and\\dj{}and\\'{c}, H.}",
    "{$\\hbox {N}^3$} \\box  {é} \\hbox{ab} \\mbox {x}",
    "\\foo\\ bar \\i\\ x \\o\\  y a\\o, \\o, {\\o, \\\\o; \\ae.",
    "\\x{a}}{b} \\x{a}b}{c} \\xab{c}{d} x\\y{}{z} {\\'e}t\\'e",
    "\\not\\asymptotic \\textsuperscript{SM} \\textsuperscript{\\textturnrrtail} \\ding{99} \\ding{2A}",
    "\\'\\'{a} \\c\\cx \\tß \\t ß \\t , \\textsubgrave1 \\textsubgravex",
    "a\u{1f}b\u{1e}c\u{f}d\u{e} ı\u{301} {\u{1F1FA}\u{1F1F8}} {\r\n} {}}",
    "\\'{e}} {\\'{e} \\i{} \\j{} \\i x \\l\\L\\o\\O",
    "\\",
    "",
];

const EDGE_ENCODE: &[&str] = &[
    "Muḥammad ibn Mūsā al-Khwārizmī",
    "Ðandđandć, H. {Ðandđandć, H.}",
    "α µ ≄ Þ $ – Åå a̍ ı̆ ® © °C ÷ ı",
    "i\u{301}\u{300} {a}\u{301} a\u{301}\u{300}\u{302}\u{303} }\u{301} \n\u{301}",
    "‐‑‒–— ‘’“” « » ¡¿ ½ ¹²³ ⁰ⁱ⁴ ✁ ☎ ➾ ʱ ≭ ≢",
];

#[test]
fn edge_cases_are_in_vectors() {
    // Sanity: decoding/encoding edge cases never panics in any set.
    for set in ["base", "full", "null"] {
        let r = Recoder::builtin(set, set, RecodeOptions::BIBER);
        let f = Recoder::builtin(set, set, RecodeOptions::FIXED);
        for s in EDGE_DECODE.iter().chain(EDGE_ENCODE) {
            r.decode(s);
            r.encode(s);
            f.decode(s);
        }
    }
}

/// Regenerates `tests/data/vectors.json` from oracle O3.
#[test]
#[ignore = "needs oracle O3; rewrites tests/data/vectors.json"]
fn regenerate_vectors() {
    let k = Keys::builtin();
    let mut r = Rng::new(20261003);
    let mut groups = Vec::new();
    let mut add = |data: &str, d: &str, e: &str, op: &str, inputs: Vec<String>| {
        let mut init = format!("{d} {e}");
        if data == "custom" {
            init.push(' ');
            init.push_str(&custom_path().display().to_string());
        }
        let mut seen = std::collections::HashSet::new();
        let inputs: Vec<String> = inputs
            .into_iter()
            .filter(|s| seen.insert(s.clone()))
            .collect();
        let reqs: Vec<(&str, &str)> = inputs.iter().map(|s| (op, s.as_str())).collect();
        let out = run_oracle(&init, &reqs);
        let cases: Vec<Value> = inputs
            .iter()
            .zip(out)
            .map(|(i, o)| json!([i, o.expect("perl died")]))
            .collect();
        groups.push(
            json!({"data": data, "decode_set": d, "encode_set": e, "op": op, "cases": cases}),
        );
    };
    for set in ["base", "full", "null"] {
        let n = if set == "null" { 500 } else { 1500 };
        let mut inputs: Vec<String> = EDGE_DECODE.iter().map(|s| s.to_string()).collect();
        inputs.extend((0..n).map(|_| gen_decode(&mut r, &k)));
        inputs.extend((0..n / 2).map(|_| gen_chaos(&mut r, &k)));
        add("builtin", set, set, "d", inputs);
    }
    {
        let inputs: Vec<String> = (0..300).map(|_| gen_decode(&mut r, &k)).collect();
        add("builtin", "full", "full", "dn", inputs);
    }
    for set in ["base", "full", "null"] {
        let n = if set == "null" { 100 } else { 1500 };
        let mut inputs: Vec<String> = EDGE_ENCODE.iter().map(|s| s.to_string()).collect();
        inputs.extend((0..n).map(|_| gen_encode(&mut r, &k)));
        inputs.extend((0..n / 3).map(|_| gen_chaos(&mut r, &k) + &gen_encode(&mut r, &k)));
        add("builtin", set, set, "e", inputs);
    }
    for (d, e) in [("mine", "mine"), ("other", "other"), ("base", "other")] {
        let inputs: Vec<String> = (0..300)
            .map(|_| gen_decode(&mut r, &k))
            .chain(
                [
                    "\\foo{}x",
                    "\\foobar",
                    "\\fo{}",
                    "\\zz a",
                    "\\textq",
                    "\\not\\eq",
                ]
                .map(String::from),
            )
            .collect();
        add("custom", d, e, "d", inputs);
        let inputs: Vec<String> = (0..300).map(|_| gen_encode(&mut r, &k)).collect();
        add("custom", d, e, "e", inputs);
    }
    let doc = json!({
        "generated_by": "crates/latex-recode/tests/vectors.rs regenerate_vectors (oracle O3: biber 2.22 Biber::LaTeX::Recode on perl 5.42)",
        "groups": groups,
    });
    let mut text = serde_json::to_string(&doc).unwrap();
    // One case per line keeps diffs readable.
    text = text.replace("],[", "],\n[");
    std::fs::write(manifest().join("tests/data/vectors.json"), text + "\n").unwrap();
}

/// The latex_decode/latex_encode assertions of biber's `t/utils.t`.
#[test]
fn ported_utils_t() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ported/utils.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut n = 0;
    for vec in v["vectors"].as_array().unwrap() {
        if vec["target"] != "latex-recode" {
            continue;
        }
        let line = vec["line"].as_u64().unwrap();
        // init_sets calls in t/utils.t: base/base until line 92, full/full
        // until 138, base/full until 142, then null/full.
        let (d, e) = match line {
            0..=91 => ("base", "base"),
            92..=137 => ("full", "full"),
            138..=141 => ("base", "full"),
            _ => ("null", "full"),
        };
        let r = Recoder::builtin(d, e, RecodeOptions::BIBER);
        let src = vec["source"].as_str().unwrap();
        let arg = vec["call"]["args"][0].as_str().unwrap();
        let expected = vec["expected"].as_str().unwrap();
        let got = match vec["area"].as_str().unwrap() {
            "latex_decode" => r.decode(arg),
            "latex_encode" => {
                // The .t passes NFD(...) for most encode inputs; the recorded
                // argument is what was actually passed.
                r.encode(arg)
            }
            a => panic!("area {a}"),
        };
        // The .t wraps some calls in NFC().
        let got = if src.contains("eq_or_diff(NFC(") {
            latex_recode::Normalization::Nfc.apply(&got)
        } else {
            got
        };
        assert_eq!(got, expected, "utils.t:{line}: {src}");
        n += 1;
    }
    assert_eq!(n, 51);
}

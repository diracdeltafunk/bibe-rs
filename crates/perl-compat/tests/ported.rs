//! Unit vectors from biber's own test suite (`tests/ported/*.json`) that
//! target this crate, plus environment checks.

mod common;

use perl_compat::roman::{isroman, roman2int};
use perl_compat::sort::reduce_array;
use perl_compat::unicode::nfkd;

fn ported(name: &str) -> serde_json::Value {
    let s = std::fs::read_to_string(common::root().join("tests/ported").join(name)).unwrap();
    serde_json::from_str(&s).unwrap()
}

#[test]
fn utils_reduce_array() {
    let v = ported("utils.json");
    let mut n = 0;
    for t in v["vectors"].as_array().unwrap() {
        if t["call"]["sub"] != "Biber::Utils::reduce_array" {
            continue;
        }
        let args = t["call"]["args"].as_array().unwrap();
        let strs = |a: &serde_json::Value| -> Vec<String> {
            a.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect()
        };
        let want = strs(&t["expected"]);
        assert_eq!(reduce_array(&strs(&args[0]), &strs(&args[1])), want);
        n += 1;
    }
    assert!(n > 0);
}

/// The roman-numeral parts of the `rangelen` vectors (`utils.t` 154/156):
/// `XXiv` → 24, `ⅥⅠ` (via NFKD) → 7, `ⅻ` → 12.
#[test]
fn utils_rangelen_roman_parts() {
    assert_eq!(roman2int(&nfkd("XXiv")), Some(24));
    assert_eq!(roman2int(&nfkd("XX")), Some(20));
    assert_eq!(roman2int(&nfkd("\u{2165}\u{2160}")), Some(7));
    assert_eq!(roman2int(&nfkd("\u{217B}")), Some(12));
    assert!(!isroman("10"));
}

/// The regex engine must be the bundled PCRE2 10.46 (Unicode 16.0.0): a
/// system libpcre2 picked up through pkg-config could use other Unicode
/// tables. Build with `PCRE2_SYS_STATIC=1` to force the bundled copy.
#[test]
fn pcre2_version() {
    let (major, minor) = pcre2::version();
    assert!(
        (major, minor) >= (10, 45),
        "PCRE2 {major}.{minor} predates Unicode 16; set PCRE2_SYS_STATIC=1"
    );
}

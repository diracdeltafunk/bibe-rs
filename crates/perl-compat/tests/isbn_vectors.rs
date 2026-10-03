//! Differential vectors from `Business::ISBN`/`ISSN`/`ISMN` in oracle O3
//! (`tools/oracle/probes/perl-compat/isbn_vectors.pl`).

mod common;

use perl_compat::isbn::{Isbn, Ismn, Issn, RangeData};
use serde_json::Value;

fn opt(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

#[test]
fn isbn() {
    let v = common::read_json("isbn_vectors.json");
    let data = RangeData::builtin();
    let mut fails = Vec::new();
    for t in v["isbn"].as_array().unwrap() {
        let input = t["in"].as_str().unwrap();
        let i = Isbn::new_with(&data, input);
        if i.is_some() != t["defined"].as_bool().unwrap() {
            fails.push(format!("{input:?}: defined"));
            continue;
        }
        let Some(i) = i else { continue };
        let mut got = vec![
            Some(i.validity().to_string()),
            Some(i.isbn().to_string()),
            i.as_string(),
            i.group_code().map(str::to_string),
            i.publisher_code().map(str::to_string),
        ];
        let mut want = vec![
            Some(t["valid"].as_i64().unwrap().to_string()),
            opt(&t["isbn"]),
            opt(&t["as_string"]),
            opt(&t["group_code"]),
            opt(&t["publisher_code"]),
        ];
        if i.is_valid() {
            let i13 = i.as_isbn13();
            let i10 = i.as_isbn10();
            got.extend([
                i13.as_ref().map(|x| x.isbn().to_string()),
                i13.as_ref().and_then(Isbn::as_string),
                i10.as_ref().map(|x| x.isbn().to_string()),
                i10.as_ref().and_then(Isbn::as_string),
            ]);
            want.extend([opt(&t["isbn13"]), opt(&t["isbn13_str"]), opt(&t["isbn10"]), opt(&t["isbn10_str"])]);
        }
        if got != want {
            fails.push(format!("{input:?}: got {got:?} want {want:?}"));
        }
    }
    assert!(fails.is_empty(), "{} failures:\n{}", fails.len(), fails.join("\n"));
}

#[test]
fn issn_ismn() {
    let v = common::read_json("isbn_vectors.json");
    let mut fails = Vec::new();
    for t in v["issn"].as_array().unwrap() {
        let input = t["in"].as_str().unwrap();
        let got = Issn::new(input).map(|i| (i.is_valid(), i.as_string()));
        let want = t["defined"]
            .as_bool()
            .unwrap()
            .then(|| (t["valid"].as_bool().unwrap(), opt(&t["as_string"])));
        if got != want {
            fails.push(format!("issn {input:?}: got {got:?} want {want:?}"));
        }
    }
    for t in v["ismn"].as_array().unwrap() {
        let input = t["in"].as_str().unwrap();
        let got = Ismn::new(input).map(|i| (i.is_valid(), i.as_string(), i.country().map(str::to_string)));
        let want = t["defined"]
            .as_bool()
            .unwrap()
            .then(|| (t["valid"].as_i64().unwrap() as i32, opt(&t["as_string"]), opt(&t["country"])));
        if got != want {
            fails.push(format!("ismn {input:?}: got {got:?} want {want:?}"));
        }
    }
    assert!(fails.is_empty(), "{} failures:\n{}", fails.len(), fails.join("\n"));
}

#[test]
fn override_file_parses_like_builtin() {
    // Parsing the vendored file at run time (the ISBN_RANGE_MESSAGE path)
    // gives the same answers as the compiled-in tables.
    let xml = std::fs::read_to_string(common::root().join("data/isbn/RangeMessage.xml")).unwrap();
    let parsed = RangeData::parse(&xml, "file");
    let builtin = RangeData::builtin();
    for s in ["9780306406157", "9791090636071", "0306406152", "9786111234567"] {
        let a = Isbn::new_with(&parsed, s).unwrap();
        let b = Isbn::new_with(&builtin, s).unwrap();
        assert_eq!((a.validity(), a.as_string()), (b.validity(), b.as_string()), "{s}");
    }
}

#[test]
#[ignore = "needs oracle O3 perl"]
fn oracle_live() {
    let Some(out) = common::run_probe("isbn_vectors.pl", &[], b"") else { return };
    let live: Value = serde_json::from_slice(&out).unwrap();
    assert!(live == common::read_json("isbn_vectors.json"), "committed isbn_vectors.json is stale");
}

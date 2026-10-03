//! Differential vectors for user regexes: biber's `ireplace`/`imatch`,
//! `split` and `s/$re//g` run in oracle O3
//! (`tools/oracle/probes/perl-compat/regex_vectors.pl`).
//!
//! Known, documented differences are listed in [`known_gap`]; everything
//! else must match exactly.

mod common;

use perl_compat::regex::{PerlRegex, RegexError, imatch, ireplace};
use serde_json::Value;

/// Cases where bibers deliberately or knowingly differs from perl; returns
/// the category.
fn known_gap(pattern: &str, replace: Option<&str>, ci: bool) -> Option<&'static str> {
    if pattern.contains(r"\N{LATIN") {
        return Some("\\N{NAME} in pattern");
    }
    if pattern.contains(r"\b{") {
        return Some("\\b{…} boundaries");
    }
    if pattern.contains("(?|") {
        return Some("branch reset group count in split");
    }
    if (ci || pattern.contains("(?i)"))
        && (subject_has_multifold(pattern) || MULTIFOLD_SUBJECT.with(|s| *s.borrow()))
    {
        return Some("/i multi-character folds");
    }
    if let Some(r) = replace {
        if r.contains("@{[") || r.contains("${\\") || r.contains(r"\N{LATIN") || r.contains("${$") {
            return Some("DC-023 code in replacement");
        }
        if r == "$<" || r == "$(" {
            return Some("process-dependent variable");
        }
    }
    None
}

thread_local! {
    static MULTIFOLD_SUBJECT: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
}

/// Does the text contain a character whose full case folding has more than
/// one character (or that such a folding produces, like "ss")?
fn subject_has_multifold(s: &str) -> bool {
    s.chars()
        .any(|c| perl_compat::case::fc_char(c).is_some_and(|f| f.chars().count() > 1))
        || s.to_lowercase().contains("ss")
}

fn set_subject(s: &str) {
    MULTIFOLD_SUBJECT.with(|m| *m.borrow_mut() = subject_has_multifold(s));
}

fn s_opt(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn normalise_pid(s: &str, r: &str) -> String {
    if r.contains("$$") {
        let pid = std::process::id().to_string();
        s.replace(&pid, "PID")
    } else {
        s.to_string()
    }
}

#[test]
fn regex_vectors() {
    let v = common::read_json("regex_vectors.json");
    let mut fails = Vec::new();
    let mut gaps: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut total = 0;
    for t in v["ireplace"].as_array().unwrap() {
        total += 1;
        let (val, p, r, ci) = (
            t["v"].as_str().unwrap(),
            t["p"].as_str().unwrap(),
            t["r"].as_str().unwrap(),
            t["ci"].as_bool().unwrap(),
        );
        set_subject(val);
        let got = ireplace(val, p, r, ci).map(|x| normalise_pid(&x.value, r));
        let want = if t["err"].as_bool().unwrap() {
            None
        } else {
            s_opt(&t["out"])
        };
        let ok = match (&got, &want) {
            (Ok(g), Some(w)) => g == w,
            (Err(_), None) => true,
            _ => false,
        };
        if !ok {
            if let Some(g) = known_gap(p, Some(r), ci) {
                *gaps.entry(g).or_default() += 1;
            } else {
                fails.push(format!(
                    "ireplace({val:?}, {p:?}, {r:?}, {ci}): got {got:?} want {want:?}"
                ));
            }
        }
    }
    for t in v["imatch"].as_array().unwrap() {
        total += 1;
        let (val, p, neg, ci) = (
            t["v"].as_str().unwrap(),
            t["p"].as_str().unwrap(),
            t["neg"].as_bool().unwrap(),
            t["ci"].as_bool().unwrap(),
        );
        set_subject(val);
        let got = imatch(val, p, neg, ci);
        let want: Option<Vec<Option<String>>> = if t["err"].as_bool().unwrap() {
            None
        } else {
            Some(t["out"].as_array().unwrap().iter().map(s_opt).collect())
        };
        let ok = match (&got, &want) {
            (Ok(g), Some(w)) => g == w,
            (Err(_), None) => true,
            _ => false,
        };
        if !ok {
            if let Some(g) = known_gap(p, None, ci) {
                *gaps.entry(g).or_default() += 1;
            } else {
                fails.push(format!(
                    "imatch({val:?}, {p:?}, {neg}, {ci}): got {got:?} want {want:?}"
                ));
            }
        }
    }
    for t in v["split"].as_array().unwrap() {
        total += 1;
        let (val, p) = (t["v"].as_str().unwrap(), t["p"].as_str().unwrap());
        let got = PerlRegex::new(p, false).and_then(|re| re.split(val));
        let want: Option<Vec<Option<String>>> = if t["err"].as_bool().unwrap() {
            None
        } else {
            Some(t["out"].as_array().unwrap().iter().map(s_opt).collect())
        };
        let ok = match (&got, &want) {
            (Ok(g), Some(w)) => g == w,
            (Err(_), None) => true,
            _ => false,
        };
        if !ok {
            if let Some(g) = known_gap(p, None, false) {
                *gaps.entry(g).or_default() += 1;
            } else {
                fails.push(format!("split({p:?}, {val:?}): got {got:?} want {want:?}"));
            }
        }
    }
    for t in v["remove"].as_array().unwrap() {
        total += 1;
        let (val, p) = (t["v"].as_str().unwrap(), t["p"].as_str().unwrap());
        let got: Result<String, RegexError> =
            PerlRegex::new(p, false).and_then(|re| re.remove_all(val));
        let ok = if t["err"].as_bool().unwrap() {
            got.is_err()
        } else {
            got.as_deref().ok() == t["out"].as_str()
        };
        if !ok {
            if let Some(g) = known_gap(p, None, false) {
                *gaps.entry(g).or_default() += 1;
            } else {
                fails.push(format!(
                    "remove({p:?}, {val:?}): got {got:?} want {:?}",
                    t["out"]
                ));
            }
        }
    }
    eprintln!(
        "regex vectors: {total} cases, {} failures, known gaps: {gaps:?}",
        fails.len()
    );
    assert!(
        fails.is_empty(),
        "{} failures:\n{}",
        fails.len(),
        fails.join("\n")
    );
}

#[test]
#[ignore = "needs oracle O3 perl"]
fn oracle_live() {
    let Some(out) = common::run_probe("regex_vectors.pl", &[], b"") else {
        return;
    };
    let live: Value = serde_json::from_slice(&out).unwrap();
    assert!(
        live == common::read_json("regex_vectors.json"),
        "committed regex_vectors.json is stale"
    );
}

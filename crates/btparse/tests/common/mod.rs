//! Shared comparison logic for oracle vectors produced by
//! `tools/oracle/probes/btparse/vectors.pl`.

#![allow(dead_code)]

use btparse::{BtState, Item, Metatype, NameAnomaly, NamePart, split_list, split_name};
use serde_json::{Value, json};

pub const FILE: &str = "test.bib";

fn mt(m: Metatype) -> &'static str {
    match m {
        Metatype::Regular => "regular",
        Metatype::Comment => "comment",
        Metatype::Preamble => "preamble",
        Metatype::MacroDef => "macrodef",
    }
}

/// Our result for a parse request, in the oracle's canonical shape.
pub fn ours_parse(text: &str) -> Value {
    let mut st = BtState::biber();
    let mut p = btparse::Parser::new(text, &mut st);
    let mut entries = Vec::new();
    let mut diags = Vec::new();
    let mut fatal = false;
    for item in p.by_ref() {
        match item {
            Item::Entry(e) => {
                diags.extend(e.diags.iter().map(|d| d.render(FILE)));
                let mut o = serde_json::Map::new();
                o.insert("mt".into(), json!(mt(e.metatype)));
                o.insert("type".into(), json!(e.entry_type));
                o.insert("ok".into(), json!(e.parse_ok));
                match e.metatype {
                    Metatype::Regular | Metatype::MacroDef => {
                        if e.metatype == Metatype::Regular {
                            o.insert("key".into(), json!(e.key));
                        }
                        let f: Vec<Value> =
                            e.fieldlist_values().map(|(n, v)| json!([n, v])).collect();
                        o.insert("fields".into(), Value::Array(f));
                    }
                    _ => {
                        o.insert("value".into(), json!(e.value.as_deref().unwrap_or("")));
                    }
                }
                entries.push(Value::Object(o));
            }
            Item::Stop { diags: d, fatal: f } => {
                diags.extend(d.iter().map(|d| d.render(FILE)));
                fatal = f;
            }
        }
    }
    diags.extend(p_trailing(&mut p));
    if fatal {
        return json!({ "crash": 1, "diags": diags });
    }
    json!({ "entries": entries, "diags": diags })
}

fn p_trailing(p: &mut btparse::Parser<'_, '_>) -> Vec<String> {
    p.take_pending().iter().map(|d| d.render(FILE)).collect()
}

pub fn ours_list(s: &str, d: &str) -> Value {
    let r = split_list(s, d);
    let diags: Vec<String> = r.diags.iter().map(|d| d.render("")).collect();
    if r.crash {
        return json!({ "crash": 6, "diags": diags });
    }
    json!({ "items": r.items, "diags": diags })
}

pub fn ours_name(s: &str) -> Value {
    let r = split_name(s);
    let diags: Vec<String> = r.diags.iter().map(|d| d.render("")).collect();
    if r.anomaly == Some(NameAnomaly::Crash) {
        return json!({ "crash": 11, "diags": diags });
    }
    let p = &r.parts;
    json!({
        "first": p.first, "von": p.von, "last": p.last, "jr": p.jr,
        "fmt": {
            "f": p.format(NamePart::First), "v": p.format(NamePart::Von),
            "l": p.format(NamePart::Last), "j": p.format(NamePart::Jr),
        },
        "diags": diags,
    })
}

/// Normalise for comparison: crash signal numbers vary; Text::BibTeX returns
/// NFD strings while this crate returns substrings of its (NFC) input, so all
/// strings are compared in NFD.
fn norm(v: &Value) -> Value {
    if v.get("crash").is_some() {
        return json!({ "crash": true });
    }
    nfd(v)
}

fn nfd(v: &Value) -> Value {
    use unicode_normalization::UnicodeNormalization;
    match v {
        Value::String(s) => Value::String(s.nfd().collect()),
        Value::Array(a) => Value::Array(a.iter().map(nfd).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), nfd(v))).collect()),
        x => x.clone(),
    }
}

/// Outcome of comparing one vector.
pub enum Outcome {
    Match,
    Mismatch {
        ours: Value,
        theirs: Value,
    },
    /// The oracle crashed and so did we (only the crash itself is compared).
    CrashMatch,
}

pub fn check(v: &Value) -> Outcome {
    let op = v["op"].as_str().unwrap();
    let theirs = &v["out"];
    let ours = match op {
        "parse" => ours_parse(v["text"].as_str().unwrap()),
        "list" => ours_list(v["s"].as_str().unwrap(), v["d"].as_str().unwrap()),
        "name" => ours_name(v["s"].as_str().unwrap()),
        _ => panic!("bad op {op}"),
    };
    let (a, b) = (norm(&ours), norm(theirs));
    if a == b {
        if theirs.get("crash").is_some() {
            Outcome::CrashMatch
        } else {
            Outcome::Match
        }
    } else {
        Outcome::Mismatch {
            ours,
            theirs: theirs.clone(),
        }
    }
}

pub fn is_name_irregular(v: &Value) -> bool {
    v["op"] == "name"
        && split_name(v["s"].as_str().unwrap()).anomaly == Some(NameAnomaly::IrregularCommas)
}

/// Run all vectors in a JSON-lines file; returns (total, mismatches) and
/// prints up to `show` mismatches.
pub fn run_file(path: &str, show: usize, allow: &[&str]) -> (usize, Vec<String>) {
    let data = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut total = 0;
    let mut bad = Vec::new();
    for line in data.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        total += 1;
        if let Outcome::Mismatch { ours, theirs } = check(&v) {
            let id = v
                .get("id")
                .map(|i| i.to_string())
                .unwrap_or_else(|| total.to_string());
            if allow.iter().any(|a| id.trim_matches('"') == *a) {
                continue;
            }
            if bad.len() < show {
                eprintln!("--- mismatch {id}");
                eprintln!("input: {}", v.get("text").or(v.get("s")).unwrap());
                eprintln!("ours:   {ours}");
                eprintln!("theirs: {theirs}");
            }
            bad.push(id);
        }
    }
    (total, bad)
}

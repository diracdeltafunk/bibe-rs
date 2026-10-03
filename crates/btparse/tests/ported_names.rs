//! biber's own `parsename` unit tests (t/names.t, extracted into
//! tests/ported/names.json) for the parts that come from btparse: the
//! family/given/prefix/suffix strings are `format` output after biber's
//! pre-sanitising and `remove_outer`.

use btparse::{NamePart, split_name};
use unicode_normalization::UnicodeNormalization;

/// biber's pre-sanitising in `parsename` (bibtex.pm), `fixinits` off.
fn sanitise(s: &str) -> String {
    let s = s.trim_matches(|c: char| c.is_whitespace());
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c.is_whitespace() {
            while it.peek().is_some_and(|c| c.is_whitespace()) {
                it.next();
            }
            out.push(' ');
        } else if c == '\\' && it.peek().is_some_and(|c| c.is_whitespace()) {
            it.next();
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    // s/\A\{\{+([^\{\}]+)\}+\}\z/{$1}/
    let b = out.as_bytes();
    if b.starts_with(b"{{") && b.ends_with(b"}}") {
        let inner = out.trim_start_matches('{').trim_end_matches('}');
        if !inner.is_empty() && !inner.contains(['{', '}']) {
            return format!("{{{inner}}}");
        }
    }
    out
}

/// biber's `remove_outer`: strip one pair of braces enclosing the whole string.
fn remove_outer(s: &str) -> String {
    if !s.starts_with('{') {
        return s.to_string();
    }
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return if i == s.len() - 1 && s.len() > 2 {
                        s[1..s.len() - 1].to_string()
                    } else {
                        s.to_string()
                    };
                }
            }
            _ => {}
        }
    }
    s.to_string()
}

#[test]
fn biber_parsename_vectors() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/ported/names.json");
    let data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let vectors = data
        .as_array()
        .cloned()
        .unwrap_or_else(|| data["vectors"].as_array().cloned().expect("vector list"));
    let mut n = 0;
    for v in vectors {
        if !v["target"].as_str().unwrap_or("").contains("btparse") {
            continue;
        }
        let name = v["call"]["args"][1].as_str().unwrap();
        let input: String = sanitise(name).nfc().collect();
        let r = split_name(&input);
        for (np, part) in [
            ("family", NamePart::Last),
            ("given", NamePart::First),
            ("prefix", NamePart::Von),
            ("suffix", NamePart::Jr),
        ] {
            let f = r.parts.format(part);
            let got = if f.is_empty() {
                None
            } else {
                Some(remove_outer(&f).nfd().collect::<String>())
            };
            let ret = &v["ret"]["value"];
            let want = ret["nameparts"][np]["string"]
                .as_str()
                .map(|s| s.nfd().collect::<String>());
            assert_eq!(got, want, "{name:?} {np} ({})", v["name"]);
            let stripped = !f.is_empty() && remove_outer(&f) != f;
            let want_strip = ret["strip"][np].as_str() == Some("1");
            assert_eq!(stripped, want_strip, "{name:?} {np} strip ({})", v["name"]);
        }
        n += 1;
    }
    assert!(n >= 19, "only {n} vectors found");
}

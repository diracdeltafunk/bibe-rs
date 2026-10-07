//! Sort transliteration (`Biber::Internals::_translit`, Internals.pm:1767;
//! `Biber::Utils::get_transliterator`/`call_transliterator`, Utils.pm:1797).
//!
//! biber supports exactly three `Lingua::Translit` 0.29 tables: `IAST
//! Devanagari`, `ALA-LC RUS` and `BGN/PCGN RUS Standard`. Their rules are
//! extracted verbatim into `translit.json` (from Lingua::Translit::Tables,
//! same terms as Perl) and applied as `Lingua::Translit::translit` does: each
//! rule in order, as a global substitution, with optional look-behind
//! (`after`) / look-ahead (`before`) context.

use crate::bcf::templates::Translit;
use crate::config::{Config, OptValue};
use crate::model::Entry;
use crate::perlish::{fc, lc, split_comma};
use perl_compat::regex::PerlRegex;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug)]
struct Rule {
    from: String,
    to: String,
    /// Compiled context rule (`None` for plain rules).
    re: Option<std::sync::Arc<PerlRegex>>,
}

type Tables = BTreeMap<String, Vec<Rule>>;

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("translit.json")).expect("translit.json");
        let mut out = Tables::new();
        for (name, rules) in raw.as_object().into_iter().flatten() {
            let mut v = Vec::new();
            for r in rules.as_array().into_iter().flatten() {
                let g = |k: &str| r.get(k).and_then(|x| x.as_str()).map(str::to_string);
                let from = g("from").unwrap_or_default();
                let to = g("to").unwrap_or_default();
                let re = match (g("before"), g("after")) {
                    (None, None) => None,
                    (Some(b), None) => Some(format!("{from}(?={b})")),
                    (None, Some(a)) => Some(format!("(?<={a}){from}")),
                    (Some(b), Some(a)) => Some(format!("(?<={a}){from}(?={b})")),
                }
                .map(|p| PerlRegex::cached(&p, false).expect("translit rule regex"));
                v.push(Rule { from, to, re });
            }
            out.insert(name.clone(), v);
        }
        out
    })
}

/// `Lingua::Translit->new($table)->translit($text)`.
fn run_table(table: &str, text: &str) -> String {
    let Some(rules) = tables().get(table) else {
        return text.to_string();
    };
    let mut s = text.to_string();
    for r in rules {
        match &r.re {
            None => {
                if s.contains(&r.from) {
                    s = s.replace(&r.from, &r.to);
                }
            }
            Some(re) => {
                if let Ok(x) = re.replace_all_with(&s, |_| r.to.clone()) {
                    s = x;
                }
            }
        }
    }
    s
}

/// `call_transliterator($target, $from, $to, $text)`. Invalid pairs warn
/// on every call (BB-102) and leave the text unchanged.
fn call_transliterator(from: &str, to: &str, text: &str, warnings: &mut Vec<String>) -> String {
    let (from, to) = (lc(from), lc(to));
    let valid_from = ["iast", "russian"].contains(&from.as_str());
    let valid_to = ["devanagari", "ala-lc", "bgn/pcgn-standard"].contains(&to.as_str());
    if !(valid_from && valid_to) {
        warnings.push(format!(
            "Invalid transliteration from/to pair ({from}/{to})"
        ));
    }
    let table = match (from.as_str(), to.as_str()) {
        ("iast", "devanagari") => "IAST Devanagari",
        ("russian", "ala-lc") => "ALA-LC RUS",
        ("russian", "bgn/pcgn-standard") => "BGN/PCGN RUS Standard",
        _ => return text.to_string(),
    };
    // NFC boundary: the result is *not* re-NFD'd.
    let t = perl_compat::unicode::nfc(text);
    if t.is_empty() {
        return String::new();
    }
    run_table(table, &t)
}

/// `_translit($target, $entry, $string)`.
pub fn translit(
    cfg: &Config,
    target: &str,
    entry: &Entry,
    s: String,
    warnings: &mut Vec<String>,
) -> String {
    let et = entry.entrytype();
    let Some(ov) = cfg.blx.get(None, "translit", Some(et), None) else {
        return s;
    };
    if !ov.truthy() {
        return s;
    }
    let OptValue::Xml(x) = ov else {
        return s;
    };
    for tr in Translit::list_from_xval(x) {
        if let Some(langids) = &tr.langids {
            let Some(langid) = entry
                .get_str("langid")
                .filter(|l| crate::perlish::truthy(Some(l)))
            else {
                continue;
            };
            let fl = fc(langid);
            if !split_comma(langids).iter().any(|l| fc(l) == fl) {
                continue;
            }
        }
        let t = tr.target.as_deref().unwrap_or("");
        if lc(t) == "*"
            || t == target
            || cfg
                .datafield_set(t)
                .is_some_and(|set| set.iter().any(|f| f == target))
        {
            return call_transliterator(
                tr.from.as_deref().unwrap_or(""),
                tr.to.as_deref().unwrap_or(""),
                &s,
                warnings,
            );
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_load() {
        assert_eq!(tables().len(), 3);
        let mut w = Vec::new();
        assert_eq!(
            call_transliterator("russian", "ala-lc", "Москва", &mut w),
            "Moskva"
        );
        assert!(w.is_empty());
        assert_eq!(call_transliterator("iast", "devanagari", "", &mut w), "");
    }
}

//! `Lingua::Translit` 0.29 for the three tables biber supports: `IAST
//! Devanagari`, `ALA-LC RUS` and `BGN/PCGN RUS Standard`.
//!
//! Their rules are extracted verbatim into `data/translit.json` from
//! `Lingua::Translit::Tables` (© Alex Linke, Rona Linke and Lingua-Systems /
//! Netzum Sorglos Software GmbH; same terms as Perl, see NOTICE.md) and
//! applied as `Lingua::Translit::translit` does: each rule in order, as a
//! global substitution, with optional look-behind (`after`) / look-ahead
//! (`before`) context.

use crate::regex::PerlRegex;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

#[derive(Debug)]
struct Rule {
    from: String,
    to: String,
    /// Compiled context rule (`None` for plain rules).
    re: Option<Arc<PerlRegex>>,
}

type Tables = BTreeMap<String, Vec<Rule>>;

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../data/translit.json")).expect("translit.json");
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

/// The names of the tables available.
pub fn table_names() -> impl Iterator<Item = &'static str> {
    tables().keys().map(String::as_str)
}

/// `Lingua::Translit->new($table)->translit($text)`. An unknown table leaves
/// the text unchanged.
pub fn translit(table: &str, text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_load() {
        assert_eq!(table_names().count(), 3);
        assert_eq!(translit("ALA-LC RUS", "Москва"), "Moskva");
    }
}

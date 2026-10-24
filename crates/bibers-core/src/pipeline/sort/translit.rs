//! Sort transliteration (`Biber::Internals::_translit`, Internals.pm:1767;
//! `Biber::Utils::get_transliterator`/`call_transliterator`, Utils.pm:1797).
//!
//! biber supports exactly three `Lingua::Translit` 0.29 tables, provided by
//! [`perl_compat::translit`].

use crate::bcf::templates::Translit;
use crate::config::{Config, OptValue};
use crate::model::Entry;
use crate::perlish::{fc, lc, split_comma};

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
    perl_compat::translit::translit(table, &t)
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
        let mut w = Vec::new();
        assert_eq!(
            call_transliterator("russian", "ala-lc", "Москва", &mut w),
            "Moskva"
        );
        assert!(w.is_empty());
        assert_eq!(call_transliterator("iast", "devanagari", "", &mut w), "");
    }
}

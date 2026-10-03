//! Locale-name resolution, behaving like `Unicode::Collate::Locale` 1.31 (`getlocale`).

/// Accepted locale names and the tailoring file (stem) each one uses. Every name maps to
/// a non-empty stem, except `default` (pure DUCET).
const LOCALES: &[(&str, &str)] = &[
    ("af", "af"),
    ("ar", "ar"),
    ("as", "as"),
    ("az", "az"),
    ("be", "be"),
    ("bn", "bn"),
    ("bs", "hr"),
    ("bs_Cyrl", "sr"),
    ("ca", "ca"),
    ("cs", "cs"),
    ("cu", "cu"),
    ("cy", "cy"),
    ("da", "da"),
    ("de_AT_phonebook", "de_at_ph"),
    ("de__phonebook", "de_phone"),
    ("dsb", "dsb"),
    ("ee", "ee"),
    ("eo", "eo"),
    ("es", "es"),
    ("es__traditional", "es_trad"),
    ("et", "et"),
    ("fa", "fa"),
    ("fi", "fi"),
    ("fi__phonebook", "fi_phone"),
    ("fil", "fil"),
    ("fo", "fo"),
    ("fr_CA", "fr_ca"),
    ("gu", "gu"),
    ("ha", "ha"),
    ("haw", "haw"),
    ("he", "he"),
    ("hi", "hi"),
    ("hr", "hr"),
    ("hu", "hu"),
    ("hy", "hy"),
    ("ig", "ig"),
    ("is", "is"),
    ("ja", "ja"),
    ("kk", "kk"),
    ("kl", "kl"),
    ("kn", "kn"),
    ("ko", "ko"),
    ("kok", "kok"),
    ("lkt", "lkt"),
    ("ln", "ln"),
    ("lt", "lt"),
    ("lv", "lv"),
    ("mk", "mk"),
    ("ml", "ml"),
    ("mr", "mr"),
    ("mt", "mt"),
    ("nb", "nb"),
    ("nn", "nn"),
    ("nso", "nso"),
    ("om", "om"),
    ("or", "or"),
    ("pa", "pa"),
    ("pl", "pl"),
    ("ro", "ro"),
    ("sa", "sa"),
    ("se", "se"),
    ("si", "si"),
    ("si__dictionary", "si_dict"),
    ("sk", "sk"),
    ("sl", "sl"),
    ("sq", "sq"),
    ("sr", "sr"),
    ("sr_Latn", "hr"),
    ("sv", "sv"),
    ("sv__reformed", "sv_refo"),
    ("ta", "ta"),
    ("te", "te"),
    ("th", "th"),
    ("tn", "tn"),
    ("to", "to"),
    ("tr", "tr"),
    ("ug_Cyrl", "ug_cyrl"),
    ("uk", "uk"),
    ("ur", "ur"),
    ("vi", "vi"),
    ("vo", "vo"),
    ("wae", "wae"),
    ("wo", "wo"),
    ("yo", "yo"),
    ("zh", "zh"),
    ("zh__big5han", "zh_big5"),
    ("zh__gb2312han", "zh_gb"),
    ("zh__pinyin", "zh_pin"),
    ("zh__stroke", "zh_strk"),
    ("zh__zhuyin", "zh_zhu"),
];

/// Variant-code aliases.
const TYPE_ALIAS: &[(&str, &str)] = &[
    ("phone", "phonebook"),
    ("phonebk", "phonebook"),
    ("dict", "dictionary"),
    ("reform", "reformed"),
    ("trad", "traditional"),
    ("big5", "big5han"),
    ("gb2312", "gb2312han"),
];

fn lookup(name: &str) -> Option<&'static str> {
    LOCALES.iter().find(|(n, _)| *n == name).map(|(n, _)| *n)
}

/// The tailoring file (`Unicode/Collate/Locale/<stem>.pl`) used for an accepted locale
/// name, e.g. `"de_phone"` for `"de__phonebook"`, `"hr"` for `"bs"`; `None` for `default`.
pub fn file_stem(accepted: &str) -> Option<&'static str> {
    LOCALES
        .iter()
        .find(|(n, _)| *n == accepted)
        .map(|(_, f)| *f)
}

/// All accepted locale names that have a tailoring (i.e. everything except `default`).
pub fn tailored_locales() -> impl Iterator<Item = &'static str> {
    LOCALES.iter().map(|(n, _)| *n)
}

/// Resolve a locale name to the name `Unicode::Collate::Locale` accepts
/// (its `getlocale`): case-insensitive, `-`, space and `.` act as `_`, variant aliases
/// (`phone` → `phonebook`, `trad` → `traditional`, …) are applied, and the fallback order
/// is language+script+region+variant, …, language, then `default`.
///
/// `None`, `""` and `"0"` (false in Perl) give `default`.
pub fn resolve_locale(locale: Option<&str>) -> &'static str {
    let Some(locale) = locale else {
        return "default";
    };
    if locale.is_empty() || locale == "0" {
        return "default";
    }
    let mut loc: String = locale
        .to_lowercase()
        .chars()
        .map(|c| if matches!(c, '-' | ' ' | '.') { '_' } else { c })
        .collect();
    // s/_([0-9a-z]+)\z/.../ : alias the trailing variant
    if let Some(pos) = loc.rfind('_') {
        let tail = &loc[pos + 1..];
        let alias = TYPE_ALIAS.iter().find(|(a, _)| *a == tail);
        if let Some((_, full)) = alias {
            loc.truncate(pos + 1);
            loc.push_str(full);
        }
    }
    if let Some(n) = lookup(&loc) {
        return n;
    }

    // split /_/ (trailing empty fields are dropped)
    let mut code: Vec<&str> = loc.split('_').collect();
    while code.last() == Some(&"") {
        code.pop();
    }
    let mut code = code.into_iter().peekable();
    let lan = code.next().unwrap_or("");
    let mut scr = String::new();
    if code.peek().is_some_and(|c| c.chars().count() == 4) {
        let c = code.next().unwrap_or_default();
        let mut ch = c.chars();
        if let Some(f) = ch.next() {
            scr.extend(f.to_uppercase());
            scr.push_str(ch.as_str());
        }
    }
    let mut reg = String::new();
    if code.peek().is_some_and(|c| c.chars().count() < 4) {
        reg = code.next().unwrap_or_default().to_uppercase();
    }
    let var = code.next().unwrap_or("");

    let mut list = Vec::with_capacity(8);
    if !var.is_empty() {
        list.push(format!("{lan}_{scr}_{reg}_{var}"));
        list.push(format!("{lan}_{scr}__{var}"));
        list.push(format!("{lan}_{reg}_{var}"));
        list.push(format!("{lan}__{var}"));
    }
    list.push(format!("{lan}_{scr}_{reg}"));
    list.push(format!("{lan}_{scr}"));
    list.push(format!("{lan}_{reg}"));
    list.push(lan.to_string());
    list.iter().find_map(|l| lookup(l)).unwrap_or("default")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution() {
        let cases = [
            ("en-US", "default"),
            ("de-DE", "default"),
            ("sv-SE", "sv"),
            ("de_DE_phone", "de__phonebook"),
            ("de-AT-phonebook", "de_AT_phonebook"),
            ("es_ES_trad", "es__traditional"),
            ("FR-ca", "fr_CA"),
            ("sr-Latn-RS", "sr_Latn"),
            ("bs-Cyrl", "bs_Cyrl"),
            ("zh__big5", "zh__big5han"),
            ("zh_TW_big5han", "zh__big5han"),
            ("ja", "ja"),
            ("default", "default"),
            ("0", "default"),
            ("", "default"),
        ];
        for (i, o) in cases {
            assert_eq!(resolve_locale(Some(i)), o, "{i}");
        }
        assert_eq!(resolve_locale(None), "default");
        assert_eq!(tailored_locales().count(), 90);
        let stems: std::collections::BTreeSet<_> = LOCALES.iter().map(|x| x.1).collect();
        assert_eq!(stems.len(), 87);
    }
}

//! The differential suites (shared by the oracle tests and the offline digest tests).

use std::collections::HashMap;
use std::sync::Arc;

use super::*;

const ENTRY: &str = "\
1100 1161 ; [.5000.0020.0002]
1162 11A8 ; [.5001.0020.0002]
1101 1161 11A8 ; [.5002.0020.0002][.5003.0021.0003.1234]
0063 0301 0068 0068 ; [.5004.0020.0002]
0061 0062 0063 0064 0065 ; [.5005.0020.0002]
0078 ; [.0001.0020.0002]
0079 ; [*0001.0020.0002]
0FB2 0F71 ; [.5006.0020.0002]
0045 ; [.0000.0000.0000]
0046 ; [.1FD6.0020.0002.0046][.0000.0111.0002.0001]
";

/// A named differential test: a collator spec applied to an input set.
pub struct Suite {
    pub name: String,
    pub spec: Spec,
    pub inputs: Arc<Vec<Vec<u32>>>,
}

pub fn all_code_points() -> Vec<Vec<u32>> {
    (0..=0x10FFFFu32).map(|c| vec![c]).collect()
}

/// The exhaustive single-code-point suites.
/// (one suite per Unicode plane, so that offline digest failures are localised).
pub fn allcp_suites() -> Vec<Suite> {
    let all = all_code_points();
    let planes: Vec<Arc<Vec<Vec<u32>>>> =
        all.chunks(0x10000).map(|c| Arc::new(c.to_vec())).collect();
    let mut out = Vec::new();
    for spec in [
        biber_sort(Some("en-US"), "4", "1"),
        biber_sortinit(Some("en-US")),
        module_default(None),
    ] {
        for (i, p) in planes.iter().enumerate() {
            out.push(Suite {
                name: format!("allcp:{}:plane{i}", spec.name),
                spec: spec.clone(),
                inputs: p.clone(),
            });
        }
    }
    out
}

/// Per-locale suites: tailoring keys (+ marks, context) and targeted strings under the
/// option sets biber uses.
pub fn locale_suites() -> Vec<Suite> {
    let keys: HashMap<String, Vec<Vec<u32>>> = locale_keys().into_iter().collect();
    let mut out = Vec::new();
    for loc in locale_names() {
        let mut inputs = targeted();
        match loc.and_then(ucollate::file_stem) {
            Some(stem) => inputs.extend(key_strings(&keys[stem])),
            None => inputs.extend(key_strings(&ducet_contractions())),
        }
        let inputs = Arc::new(inputs);
        for spec in [
            biber_sort(loc, "4", "1"),
            biber_sort(loc, "2", "0"),
            biber_sort(loc, "3", "0"),
            biber_sortinit(loc),
            module_default(loc),
        ] {
            out.push(Suite {
                name: format!("locale:{}", spec.name),
                spec,
                inputs: inputs.clone(),
            });
        }
    }
    out
}

/// Option matrix on a few locales over targeted and random strings.
pub fn option_suites() -> Vec<Suite> {
    let keys: HashMap<String, Vec<Vec<u32>>> = locale_keys().into_iter().collect();
    let mut out = Vec::new();
    let locs = [
        None,
        Some("sv"),
        Some("da"),
        Some("fr_CA"),
        Some("cu"),
        Some("ja"),
        Some("ko"),
        Some("th"),
        Some("hu"),
        Some("vi"),
        Some("zh__pinyin"),
        Some("es__traditional"),
    ];
    for (li, loc) in locs.into_iter().enumerate() {
        let mut pool = base_pool();
        if let Some(stem) = loc.and_then(ucollate::file_stem) {
            for k in &keys[stem] {
                pool.extend_from_slice(k);
            }
        }
        let mut inputs = targeted();
        inputs.extend(random_strings(1000 + li as u64, 1500, &pool, 10));
        let l = loc.unwrap_or("undef");
        // biber's path: NFD (Unicode 16) strings collated with normalization=prenormalized
        let nfd: Vec<Vec<u32>> = inputs
            .iter()
            .filter_map(|s| {
                let s: String = s
                    .iter()
                    .map(|&c| char::from_u32(c))
                    .collect::<Option<_>>()?;
                use unicode_normalization::UnicodeNormalization;
                Some(s.nfd().map(u32::from).collect())
            })
            .collect();
        let nfd = Arc::new(nfd);
        for (lv, ubl) in [("4", "1"), ("2", "0")] {
            let spec = biber_sort(loc, lv, ubl);
            out.push(Suite {
                name: format!("nfd:{}", spec.name),
                spec,
                inputs: nfd.clone(),
            });
        }
        let inputs = Arc::new(inputs);
        let mut specs = Vec::new();
        for level in ["1", "2", "3", "4"] {
            for var in ["non-ignorable", "shifted", "blanked", "shift-trimmed"] {
                for ubl in ["0", "1"] {
                    specs.push(
                        Spec::new(format!("{l}/l{level}-{var}-ubl{ubl}"), loc)
                            .opt("level", Val::Str(level))
                            .opt("variable", Val::Str(var))
                            .opt("normalization", Val::Str("prenormalized"))
                            .opt("upper_before_lower", Val::Str(ubl)),
                    );
                }
            }
        }
        let extra: Vec<(&str, &'static str, Val)> = vec![
            ("bw2", "backwards", Val::Str("2")),
            ("bw1234", "backwards", Val::List(vec![1, 2, 3, 4])),
            ("bwundef", "backwards", Val::Undef),
            ("kbh", "katakana_before_hiragana", Val::Str("1")),
            ("il2", "ignore_level2", Val::Str("1")),
            ("ident", "identical", Val::Str("1")),
            ("hterm", "hangul_terminator", Val::Str("16")),
            ("hiFFFF", "highestFFFF", Val::Str("1")),
            ("loFFFE", "minimalFFFE", Val::Str("1")),
            ("nfc", "normalization", Val::Str("NFC")),
            ("nfkd", "normalization", Val::Str("NFKD")),
            ("nfkc", "normalization", Val::Str("KC")),
            ("nfd", "normalization", Val::Str("NFD")),
            ("normundef", "normalization", Val::Undef),
        ];
        for (n, k, v) in extra {
            specs.push(Spec::new(format!("{l}/{n}"), loc).opt(k, v.clone()));
            specs.push(
                Spec::new(format!("{l}/{n}-ni-l3"), loc)
                    .opt("level", Val::Str("3"))
                    .opt("variable", Val::Str("non-ignorable"))
                    .opt(k, v),
            );
        }
        for var in ["shift-trimmed", "blanked"] {
            specs.push(
                Spec::new(format!("{l}/loFFFE-hiFFFF-{var}"), loc)
                    .opt("minimalFFFE", Val::Str("1"))
                    .opt("highestFFFF", Val::Str("1"))
                    .opt("variable", Val::Str(var)),
            );
        }
        // user entries: jamo contractions (Hangul syllable decomposition), a long
        // contraction, a weight-0001 element and 4-weight elements
        for norm in ["prenormalized", "NFD"] {
            specs.push(
                Spec::new(format!("{l}/entry-{norm}"), loc)
                    .new_opt("entry", Val::Str(ENTRY))
                    .opt("normalization", Val::Str(norm))
                    .opt("hangul_terminator", Val::Str("5")),
            );
        }
        for spec in specs {
            out.push(Suite {
                name: format!("opt:{}", spec.name),
                spec,
                inputs: inputs.clone(),
            });
        }
    }
    out
}

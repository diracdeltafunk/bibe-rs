//! Differential tests against the Perl oracle (`Unicode::Collate` 1.31 under the O3
//! perl). Ignored by default: run with
//! `cargo test -p ucollate --release --test differential -- --ignored --nocapture`
//! (set `UCOLLATE_PERL` if the O3 perl is not found automatically).
//! With `UCOLLATE_BLESS=1` the digests in `tests/data/digests-*.txt` are rewritten from
//! the oracle's keys; `tests/digests.rs` checks them offline.

mod common;

use common::*;

fn run_suites(suites: Vec<Suite>, group: &str) {
    let mut bad = 0;
    let mut failing = Vec::new();
    let mut lines = Vec::new();
    for s in &suites {
        let expected = perl_keys(&s.spec, &s.inputs);
        let n = compare(&s.spec, &s.inputs, &expected);
        bad += n;
        if n > 0 {
            failing.push(s.name.clone());
        }
        lines.push(format!("{}\t{}\t{}", s.name, s.inputs.len(), digest(&expected)));
    }
    eprintln!(
        "{group}: {} suites, {} keys, {} failing suites, {bad} mismatches",
        suites.len(),
        suites.iter().map(|s| s.inputs.len()).sum::<usize>(),
        failing.len()
    );
    if !failing.is_empty() {
        eprintln!("failing: {}", failing.join(" "));
    }
    if std::env::var("UCOLLATE_BLESS").is_ok() {
        let dir = crate_dir().join("tests/data");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("digests-{group}.txt")),
            lines.join("\n") + "\n",
        )
        .unwrap();
    }
    assert_eq!(bad, 0);
}

/// Locale names to resolve: every accepted name in several spellings, BCP 47 tags
/// biber produces, and junk.
pub fn locale_inputs() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for l in ucollate::tailored_locales() {
        v.push(l.to_string());
        v.push(l.to_uppercase());
        v.push(l.replace('_', "-"));
        v.push(format!("{l}_XX"));
        v.push(format!("{l}-Latn-US"));
        v.push(format!("{l}.utf8"));
    }
    for lang in ["de", "es", "fi", "si", "sv", "zh", "sr", "bs", "ug", "fr", "en", "xx"] {
        for scr in ["", "Latn", "Cyrl", "Hans"] {
            for reg in ["", "AT", "CA", "DE", "419", "TW"] {
                for var in [
                    "", "phone", "phonebk", "phonebook", "dict", "trad", "reform", "big5",
                    "gb2312", "pinyin", "stroke", "zhuyin", "variant1",
                ] {
                    let parts: Vec<&str> = [lang, scr, reg, var]
                        .into_iter()
                        .filter(|p| !p.is_empty())
                        .collect();
                    v.push(parts.join("_"));
                    v.push(format!("{lang}_{scr}_{reg}_{var}"));
                }
            }
        }
    }
    for s in [
        "", "0", "default", "en-US", "en_US", "de-DE", "sv-SE", "nb-NO", "nn-NO", "pt-BR",
        "zh-CN", "ja-JP", "ko-KR", "el-GR", "ru-RU", "uk-UA", "_", "__", "_de", "de__",
        "de__phone_x", "de _AT. phone", "DE-at-PHONEBOOK", "sr_Latn_RS", "es_ES_traditional",
        "fr_CA_x", "ZH__Big5", "zh-Hant-TW-stroke", "chinese", "korean", "german",
    ] {
        v.push(s.to_string());
    }
    v
}

#[test]
#[ignore]
fn oracle_locale_names() {
    let names = locale_inputs();
    let mut req = String::new();
    for n in &names {
        req.push_str(&format!("L\t{n}\n"));
    }
    req.push_str("L\t~\n");
    let resp = run_oracle(&req);
    assert_eq!(resp.len(), names.len() + 1);
    let mut bad = 0;
    let mut lines = Vec::new();
    for (n, e) in names.iter().zip(&resp) {
        let got = ucollate::resolve_locale(Some(n));
        if got != e {
            eprintln!("{n:?}: perl {e} rust {got}");
            bad += 1;
        }
        lines.push(format!("{n}\t{e}"));
    }
    assert_eq!(resp[names.len()], ucollate::resolve_locale(None));
    eprintln!("{} names, {bad} mismatches", names.len());
    if std::env::var("UCOLLATE_BLESS").is_ok() {
        std::fs::write(
            crate_dir().join("tests/data/locale-names.tsv"),
            lines.join("\n") + "\n",
        )
        .unwrap();
    }
    assert_eq!(bad, 0);
}

#[test]
#[ignore]
fn oracle_option_values() {
    let keys = [
        "level",
        "variable",
        "normalization",
        "upper_before_lower",
        "backwards",
        "katakana_before_hiragana",
        "ignore_level2",
        "identical",
        "hangul_terminator",
        "highestFFFF",
        "minimalFFFE",
        "UCA_Version",
    ];
    let mut bad = 0;
    for loc in locale_names() {
        for spec in [
            Spec::new("fresh", loc),
            biber_sort(loc, "2", "0"),
            biber_sortinit(loc),
        ] {
            let mut req = format!("C\t{}\n", spec.json());
            for k in keys {
                req.push_str(&format!("V\t{k}\n"));
            }
            let resp = run_oracle(&req);
            assert_eq!(resp[0], "ok");
            let c = spec.collator();
            for (k, e) in keys.iter().zip(&resp[1..]) {
                let got = c.option_value(k).unwrap_or_else(|| "~".into());
                if &got != e {
                    eprintln!("{loc:?} {}: {k}: perl {e} rust {got}", spec.name);
                    bad += 1;
                }
            }
        }
    }
    assert_eq!(bad, 0);
}

#[test]
#[ignore]
fn oracle_all_code_points() {
    run_suites(allcp_suites(), "allcp");
}

#[test]
#[ignore]
fn oracle_locales() {
    run_suites(locale_suites(), "locales");
}

#[test]
#[ignore]
fn oracle_options() {
    run_suites(option_suites(), "options");
}

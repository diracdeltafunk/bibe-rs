//! `sortinithash` = md5_hex(viewSortKey(sortinit)) with biber's level-1 collator,
//! checked against values biber 2.22 (oracle O1) wrote into the golden `.bbl` files of
//! the differential corpus (`tests/corpus`), and against hashes listed in
//! docs/research/04-sorting-collation.md.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use md5::{Digest, Md5};
use ucollate::{CollateOptions, Collator};

fn sortinithash(c: &Collator, init: &str) -> String {
    common::hex(&Md5::digest(c.view_sort_key(init).as_bytes()))
}

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "bbl") {
            out.push(p);
        }
    }
}

#[test]
fn research_values() {
    let en = Collator::new(Some("en-US"), &CollateOptions::biber_sortinit()).unwrap();
    let sv = Collator::new(Some("sv-SE"), &CollateOptions::biber_sortinit()).unwrap();
    for (c, s, h) in [
        (&en, "A", "2f401846e2029bad6b3ecc16d50031e2"),
        (&en, "\u{C5}", "2f401846e2029bad6b3ecc16d50031e2"),
        (&en, "\u{C4}", "2f401846e2029bad6b3ecc16d50031e2"),
        (&en, "Z", "96892c0b0a36bb8557c40c49813d48b3"),
        (&en, "\u{C9}", "8da8a182d344d5b9047633dfc0cc9131"),
        (&en, "T", "9af77f0292593c26bde9a56e688eaee9"),
        (&en, "c", "4d103a86280481745c9c897c925753c0"),
        (&en, "", "495dc9894017a8b12cafa9c619d10c0c"),
        (&sv, "\u{C4}", "db0c88eea8d3c2b513a2fee764f1ad6b"),
    ] {
        assert_eq!(sortinithash(c, s), h, "{s:?}");
    }
    assert_eq!(en.view_sort_key("\u{C6}"), "[1FA2 2007 | | |]");
    assert_eq!(en.view_sort_key("\u{DF}"), "[21D2 21D2 | | |]");
    assert_eq!(en.view_sort_key("\u{4E2D}"), "[FB40 CE2D | | |]");
    assert_eq!(sv.view_sort_key("\u{C4}"), "[22FC | | |]");
}

/// Every (sortinit, sortinithash) pair in the corpus goldens must be reproduced by the
/// default (en-US / DUCET) collator or, for documents sorted with a tailoring, by some
/// locale's.
#[test]
fn corpus_goldens() {
    let corpus = common::crate_dir().join("../../tests/corpus");
    let mut files = Vec::new();
    walk(&corpus, &mut files);
    let mut pairs = BTreeSet::new();
    for f in &files {
        // a few goldens use --output-encoding other than UTF-8; skip them
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        let mut init: Option<String> = None;
        for line in text.lines() {
            let line = line.trim();
            if let Some(v) = line.strip_prefix("\\field{sortinit}{") {
                init = v.strip_suffix('}').map(str::to_string);
            } else if let Some(v) = line.strip_prefix("\\field{sortinithash}{") {
                if let (Some(i), Some(h)) = (init.take(), v.strip_suffix('}')) {
                    pairs.insert((i, h.to_string()));
                }
            }
        }
    }
    if files.is_empty() {
        eprintln!("no corpus found; skipping");
        return;
    }
    let default = Collator::new(Some("en-US"), &CollateOptions::biber_sortinit()).unwrap();
    let others: Vec<Collator> = ucollate::tailored_locales()
        .map(|l| Collator::new(Some(l), &CollateOptions::biber_sortinit()).unwrap())
        .collect();
    let mut by_default = 0;
    let mut by_locale = 0;
    for (init, hash) in &pairs {
        if sortinithash(&default, init) == *hash {
            by_default += 1;
        } else if let Some(c) = others.iter().find(|c| sortinithash(c, init) == *hash) {
            eprintln!("{init:?} {hash}: matched by locale {}", c.locale());
            by_locale += 1;
        } else {
            panic!("{init:?} {hash}: no collator reproduces this sortinithash");
        }
    }
    eprintln!(
        "{} bbl files, {} distinct pairs: {by_default} default, {by_locale} tailored",
        files.len(),
        pairs.len()
    );
    assert!(pairs.len() > 20);
}

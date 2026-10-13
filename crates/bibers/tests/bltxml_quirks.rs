//! biblatexml reader quirks with a fixed behaviour not covered by the corpus
//! (whose goldens are biber's): cited-key aliases (BB-174) and fields set
//! on `map_entry_new` entries (BB-175).

use std::path::Path;

use bibers_core::env::Env;

fn run_case(case: &str, extra: &[&str]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(case);
    let d = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(&src).unwrap().flatten() {
        if e.path().is_file() {
            std::fs::copy(e.path(), d.path().join(e.file_name())).unwrap();
        }
    }
    let mut args: Vec<String> = ["-q", "--noconf"].iter().map(|s| s.to_string()).collect();
    args.extend(extra.iter().map(|s| s.to_string()));
    args.push("doc.bcf".into());
    assert_eq!(bibers::run::main(&args, &Env::isolated(d.path())), 0);
    std::fs::read_to_string(d.path().join("doc.bbl")).unwrap()
}

#[test]
fn cited_aliases() {
    let quirky = run_case("bltxml/fields-cited", &[]);
    assert!(quirky.contains("\\missing{f1alias}"), "{quirky}");
    let fixed = run_case(
        "bltxml/fields-cited",
        &["--fix-quirks=bltxml-cited-aliases"],
    );
    assert!(fixed.contains("\\keyalias{f1alias}{f1}"), "{fixed}");
    assert!(!fixed.contains("\\missing{f1alias}"));
}

#[test]
fn smap_new_entry_gets_fields() {
    let quirky = run_case("bltxml/smap-new-entry", &[]);
    let fixed = run_case(
        "bltxml/smap-new-entry",
        &["--fix-quirks=bltxml-smap-new-entry-ns"],
    );
    let entry = |bbl: &str| {
        let s = bbl.find("\\entry{newkey}").expect("newkey entry");
        bbl[s..s + bbl[s..].find("\\endentry").unwrap()].to_string()
    };
    assert!(!entry(&quirky).contains("\\field{title}"));
    assert!(entry(&fixed).contains("\\field{title}{New}"), "{fixed}");
}

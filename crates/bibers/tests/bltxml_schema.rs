//! The bltxml RNG schema (`Biber::DataModel::generate_bltxml_schema`):
//! compared with biber 2.22's output for `tdata/biblatexml.bcf`
//! (`tests/data/bltxml/biblatexml-biber.rng`), modulo the three Perl hash
//! orders biber iterates (entry types, and the field type × data type
//! `<ref>`s and `<define>`s).

use std::path::Path;

use bibers_core::env::Env;

fn corpus(p: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(p)
}

fn run(dir: &Path, args: &[&str]) -> u8 {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    bibers::run::main(&args, &Env::isolated(dir))
}

fn setup() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for f in ["biblatexml.bcf", "biblatexml.bltxml"] {
        std::fs::copy(corpus("tdata/biblatexml").join(f), d.path().join(f)).unwrap();
    }
    d
}

/// Sort the hash-ordered parts of a schema.
fn canon(t: &str) -> String {
    let lines: Vec<&str> = t.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    // entry type values: the lines between `<attribute name="entrytype">`'s
    // `<choice>` and `</choice>`
    while i < lines.len() {
        out.push(lines[i].to_string());
        if lines[i].trim() == r#"<attribute name="entrytype">"# {
            out.push(lines[i + 1].to_string());
            i += 2;
            let mut vals = Vec::new();
            while lines[i].trim() != "</choice>" {
                vals.push(lines[i].to_string());
                i += 1;
            }
            vals.sort();
            out.extend(vals);
            continue;
        }
        if lines[i].trim() == "<interleave>" && lines[i + 1].trim().starts_with("<!--") {
            // (comment, ref) pairs up to the mannotation ref
            i += 1;
            let mut pairs = Vec::new();
            while !lines[i].contains(r#"<ref name="mannotation" />"#) {
                pairs.push(format!("{}\n{}", lines[i], lines[i + 1]));
                i += 2;
            }
            pairs.sort();
            out.extend(pairs);
            continue;
        }
        if lines[i].trim() == "</start>" {
            // (comment, define…/define) blocks up to the xdata definition
            i += 1;
            let mut blocks = Vec::new();
            while !lines[i].contains("xdata attribute definition") {
                let mut b = vec![lines[i], lines[i + 1]];
                i += 2;
                while lines[i] != "  </define>" {
                    b.push(lines[i]);
                    i += 1;
                }
                b.push(lines[i]);
                i += 1;
                blocks.push(b.join("\n"));
            }
            blocks.sort();
            out.extend(blocks);
            continue;
        }
        i += 1;
    }
    out.join("\n")
}

#[test]
fn schema_matches_biber() {
    let d = setup();
    assert_eq!(run(d.path(), &["--noconf", "-q", "biblatexml.bcf"]), 0);
    let ours = std::fs::read_to_string(d.path().join("biblatexml.rng")).unwrap();
    let theirs = include_str!("data/bltxml/biblatexml-biber.rng");
    assert_eq!(ours.lines().count(), theirs.lines().count());
    assert_eq!(canon(&ours), canon(theirs));
    // deterministic
    let d2 = setup();
    assert_eq!(run(d2.path(), &["--noconf", "-q", "biblatexml.bcf"]), 0);
    assert_eq!(
        ours,
        std::fs::read_to_string(d2.path().join("biblatexml.rng")).unwrap()
    );
}

#[test]
fn schema_options() {
    let d = setup();
    assert_eq!(
        run(
            d.path(),
            &["--noconf", "-q", "--no-bltxml-schema", "biblatexml.bcf"]
        ),
        0
    );
    assert!(!d.path().join("biblatexml.rng").exists());

    let d = setup();
    std::fs::create_dir(d.path().join("out")).unwrap();
    assert_eq!(
        run(
            d.path(),
            &["--noconf", "-q", "--output-directory=out", "biblatexml.bcf"]
        ),
        0
    );
    assert!(d.path().join("out/biblatexml.rng").exists());
    assert!(!d.path().join("biblatexml.rng").exists());
}

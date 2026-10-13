//! Tool mode (`--tool`) and bibtex output: file handling and the fixed
//! behaviour of the tool-mode quirks. Output equality with biber is checked
//! by the `tool/` corpus suite.

use std::path::Path;

use bibers::run::tool_output_name;
use bibers_core::env::Env;

fn run(dir: &Path, args: &[&str]) -> u8 {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    bibers::run::main(&args, &Env::isolated(dir))
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

const DATA: &str = r#"@preamble{"\newcommand{\noop}[1]{}"}
@comment{First}
@book{a,
  title = {Why?},
  date = {1999-XX},
  author = {Doe, J.},
  author+an = {1="Lit"},
}
@book{b,
  title = {Winter},
  date = {2020-32},
}
"#;

const DATA2: &str = "@comment{Second}\n@misc{c, title = {C}}\n";

fn setup() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("d.bib"), DATA).unwrap();
    std::fs::write(d.path().join("e.bib"), DATA2).unwrap();
    d
}

#[test]
fn output_names() {
    assert_eq!(
        tool_output_name("x.bib", "_bibertool.bib"),
        "x_bibertool.bib"
    );
    assert_eq!(
        tool_output_name("dir/x.y.bib", "_bibertool.bib"),
        "dir/x.y_bibertool.bib"
    );
    // DC-011 (BB-013): no extension.
    assert_eq!(
        tool_output_name("refs", "_bibertool.bib"),
        "refs_bibertool.bib"
    );
    assert_eq!(
        tool_output_name("my.dir/refs", "_bibertool.bib"),
        "my.dir/refs_bibertool.bib"
    );
    assert_eq!(tool_output_name(".bib", "_bibertool.bib"), "_bibertool.bib");
}

#[test]
fn quirks_reproduced_by_default() {
    let d = setup();
    assert_eq!(run(d.path(), &["--noconf", "--tool", "d.bib", "e.bib"]), 0);
    let out = read(&d.path().join("d_bibertool.bib"));
    assert!(out.contains("DATE = {1999-00}"), "{out}");
    assert!(out.contains("DATE = {2020},"), "{out}");
    assert!(out.contains("{1=Lit}"), "{out}");
    assert!(!out.contains("PREAMBLE"), "{out}");
    assert!(!out.contains("@COMMENT{First}") && out.contains("@COMMENT{Second}"));
}

#[test]
fn quirks_fixed() {
    let d = setup();
    assert_eq!(
        run(
            d.path(),
            &[
                "--noconf",
                "--tool",
                "--fix-quirks=bibtex-unspecified-dates,bibtex-literal-annotations,tool-comments-last-file,tool-drops-preambles,bibtex-winters-typo",
                "d.bib",
                "e.bib"
            ]
        ),
        0
    );
    let out = read(&d.path().join("d_bibertool.bib"));
    assert!(out.contains("DATE = {1999-XX}"), "{out}");
    assert!(out.contains("DATE = {2020-32}"), "{out}");
    assert!(out.contains("{1=\"Lit\"}"), "{out}");
    assert!(
        out.contains("@PREAMBLE{\"\\newcommand{\\noop}[1]{}\"}"),
        "{out}"
    );
    let first = out.find("@COMMENT{First}").expect("first comment");
    let second = out.find("@COMMENT{Second}").expect("second comment");
    assert!(first < second);
}

#[test]
fn question_mark_recoding() {
    let d = setup();
    assert_eq!(
        run(
            d.path(),
            &["--noconf", "--tool", "--output-encoding=latin1", "d.bib"]
        ),
        0
    );
    let blg = read(&d.path().join("d.bib.blg"));
    assert!(blg.contains("The entry 'a' has characters which cannot be encoded in 'latin1'"));
    assert_eq!(
        run(
            d.path(),
            &[
                "--noconf",
                "--tool",
                "--output-encoding=latin1",
                "--fix-quirks=bibtex-encoding-question-mark",
                "d.bib"
            ]
        ),
        0
    );
    let blg = read(&d.path().join("d.bib.blg"));
    assert!(!blg.contains("cannot be encoded"), "{blg}");
}

#[test]
fn stdout_and_no_early_truncation() {
    let d = setup();
    // `-O -` writes to stdout and leaves no file named `-` (DC-026).
    assert_eq!(
        run(
            d.path(),
            &["--noconf", "--tool", "--nolog", "-q", "-O", "-", "d.bib"]
        ),
        0
    );
    assert!(!d.path().join("-").exists());
    // The data writers open their output only when writing: an error before
    // that leaves an existing file alone.
    std::fs::write(d.path().join("missing_bibertool.bib"), "old").unwrap();
    assert_eq!(run(d.path(), &["--noconf", "--tool", "missing.bib"]), 2);
    assert_eq!(read(&d.path().join("missing_bibertool.bib")), "old");
}

#[test]
fn biblatexml_output_still_deferred() {
    let d = setup();
    assert_eq!(
        run(
            d.path(),
            &["--noconf", "--tool", "--output-format=biblatexml", "d.bib"]
        ),
        2
    );
}

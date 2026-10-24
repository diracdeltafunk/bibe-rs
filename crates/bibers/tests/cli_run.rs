//! End-to-end checks of the front end's file side effects and exit codes.

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

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

#[test]
fn malformed_bcf_deletes_output() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("x.bcf"),
        "<?xml version=\"1.0\"?>\n<bcf:controlfile version=\"3.11\"",
    )
    .unwrap();
    std::fs::write(d.path().join("x.bbl"), "old").unwrap();
    assert_eq!(run(d.path(), &["--noconf", "x"]), 2);
    assert!(!d.path().join("x.bbl").exists());
    let blg = read(&d.path().join("x.blg"));
    assert!(
        blg.contains(
            "> ERROR - x.bcf is malformed, last biblatex run probably failed. Deleted x.bbl"
        ),
        "{blg}"
    );
    assert!(blg.contains("> INFO - ERRORS: 1"));
}

#[test]
fn missing_bcf() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(run(d.path(), &["--noconf", "nope"]), 2);
    let blg = read(&d.path().join("nope.blg"));
    assert!(blg.contains("> ERROR - Cannot find 'nope.bcf'!"), "{blg}");
    // The output was truncated/created before the .bcf was looked for.
    assert!(d.path().join("nope.bbl").exists());
}

#[test]
fn no_data_sources_exits_zero_silently() {
    let d = tempfile::tempdir().unwrap();
    std::fs::copy(
        corpus("bugs/bb-070-no-datasources/doc.bcf"),
        d.path().join("doc.bcf"),
    )
    .unwrap();
    assert_eq!(run(d.path(), &["--noconf", "doc.bcf"]), 0);
    assert_eq!(
        std::fs::metadata(d.path().join("doc.bbl")).unwrap().len(),
        0
    );
    let blg = read(&d.path().join("doc.blg"));
    assert!(blg.contains("INFO - Reading 'doc.bcf'"));
    assert!(!blg.contains("WARN"));
}

#[test]
fn stdout_output_with_output_directory() {
    // BB-069: no file literally named '-' is created.
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("out")).unwrap();
    std::fs::copy(
        corpus("tdata/annotations/annotations.bcf"),
        d.path().join("a.bcf"),
    )
    .unwrap();
    // 2 while the M2 pipeline is incomplete (missing entries at output).
    let code = run(
        d.path(),
        &["--noconf", "-O", "-", "--output-directory=out", "a"],
    );
    assert!(code == 0 || code == 2, "exit {code}");
    assert!(!d.path().join("-").exists());
    assert!(!d.path().join("out/-").exists());
    let blg = read(&d.path().join("out/a.blg"));
    assert!(
        blg.contains("INFO - Found 2 citekeys in bib section 0"),
        "{blg}"
    );
    assert!(
        // The data file is not copied, so the run stops when fetch_data
        // looks for it (or, before M2, at the missing .bbl writer).
        blg.contains("ERROR - Cannot find 'annotations.bib'!")
            || blg.contains("not yet implemented"),
        "{blg}"
    );
}

#[test]
fn deferred_features_fail_cleanly() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(run(d.path(), &["--output-format=dot", "x"]), 2);
    assert_eq!(run(d.path(), &["--validate-bblxml", "x"]), 2);
    assert_eq!(run(d.path(), &["--fix-quirks=bogus", "x"]), 2);
    assert!(!d.path().join("x.blg").exists());
}

#[test]
fn uniqueness_terminates_where_biber_loops_forever() {
    // BB-149: a name whose XDATA reference cannot be resolved leaves a name
    // with no parts, and biber's uniquename/uniquelist loop never ends.
    // bibers stops at its pass cap (DC-052) and says so.
    let d = tempfile::tempdir().unwrap();
    let bcf = std::fs::read_to_string(corpus("regress/date-nonascii-no-tz/r.bcf")).unwrap();
    std::fs::write(d.path().join("t.bcf"), bcf.replace(">r.bib<", ">t.bib<")).unwrap();
    std::fs::write(
        d.path().join("t.bib"),
        "@misc{k,author={xdata=xd1-author},title={T}}\n",
    )
    .unwrap();
    assert_eq!(run(d.path(), &["--noconf", "t"]), 0);
    let blg = read(&d.path().join("t.blg"));
    assert!(
        blg.contains("WARN - uniquename/uniquelist processing did not converge"),
        "{blg}"
    );
    assert!(read(&d.path().join("t.bbl")).contains("\\entry{k}{misc}"));
}

/// Copy a corpus case's inputs into a fresh directory.
fn case_dir(case: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(corpus(case)).unwrap().flatten() {
        if e.path().is_file() {
            std::fs::copy(e.path(), d.path().join(e.file_name())).unwrap();
        }
    }
    d
}

/// A data source that exists but can't be read is an ERROR (exit 2), as in
/// biber, not an "empty" data source (biber's `check_empty` is a stat).
#[cfg(unix)]
#[test]
fn unreadable_data_source_is_an_error() {
    use std::os::unix::fs::PermissionsExt;
    let d = case_dir("tdata/general");
    let bib = d.path().join("examples.bib");
    std::fs::set_permissions(&bib, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&bib).is_ok() {
        return; // running as root: permissions don't apply
    }
    let code = run(d.path(), &["--noconf", "general.bcf"]);
    std::fs::set_permissions(&bib, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(code, 2);
    let blg = read(&d.path().join("general.blg"));
    assert!(
        blg.contains("ERROR - Data file 'examples.bib' cannot be read in encoding")
            && blg.contains("Couldn't open examples.bib: Permission denied"),
        "{blg}"
    );
    assert!(!blg.contains("is empty, ignoring"));
}

/// `-O -` into a pipe whose reader has gone is an ERROR (exit 2), not a
/// silent success (DC-069).
#[cfg(unix)]
#[test]
fn stdout_write_failure_is_an_error() {
    let d = case_dir("tdata/general");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_bibers"))
        .args(["--noconf", "-q", "-O", "-", "general.bcf"])
        .current_dir(d.path())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take()); // close the reading end
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(2));
    let blg = read(&d.path().join("general.blg"));
    assert!(
        blg.contains("ERROR - Cannot write output file '-': Broken pipe"),
        "{blg}"
    );
}

/// A user regex option that can't be compiled (here a Perl-only construct,
/// DC-036) is an ERROR naming the option, never a silent fallback that
/// changes the data (DC-071).
#[test]
fn unusable_regex_option_is_an_error() {
    let d = case_dir("tdata/general");
    let code = run(
        d.path(),
        &[
            "--noconf",
            r"--xsvsep=\N{LATIN CAPITAL LETTER A}",
            "general.bcf",
        ],
    );
    assert_eq!(code, 2);
    let blg = read(&d.path().join("general.blg"));
    assert!(
        blg.contains(r"ERROR - Option 'xsvsep' ('\N{LATIN CAPITAL LETTER A}'): unsupported regular expression feature"),
        "{blg}"
    );
}

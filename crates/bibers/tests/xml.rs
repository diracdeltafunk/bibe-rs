//! RELAX NG validation (DC-033) and `--convert-control`.
//!
//! * `bcfvalidation`: biber's `t/bcfvalidation.t` — every `t/tdata` `.bcf`
//!   (copied into `tests/corpus/tdata/`) validates against `bcf.rng` — and
//!   the config-file check of `t/configfile.t`.
//! * `libxml2_verdicts`: the built-in schemas give libxml2's verdict on
//!   every corpus `.bcf` and config file (needs oracle O3; skipped
//!   otherwise).
//! * `convert_control_matches_libxslt`: the `.html` is byte-identical to
//!   libxslt's on every corpus `.bcf` (needs `xsltproc` built on libxslt
//!   1.1.35 / libxml2 2.9.13, biber 2.22's versions; skipped otherwise).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use bibers_core::xmlcheck::{SchemaKind, builtin_schema};
use bibers_testkit::{corpus_root, phase::o3_available, repo_root};

fn files_with_ext(ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![corpus_root()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "expected") {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|e| e == ext) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// One path per distinct file content.
fn unique(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeMap::new();
    for p in paths {
        let bytes = std::fs::read(&p).unwrap();
        seen.entry(bytes).or_insert(p);
    }
    let mut v: Vec<PathBuf> = seen.into_values().collect();
    v.sort();
    v
}

fn read_text(p: &Path) -> String {
    let bytes = std::fs::read(p).unwrap();
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    }
}

#[test]
fn bcfvalidation() {
    let schema = builtin_schema(SchemaKind::Control);
    let mut n = 0;
    for p in files_with_ext("bcf") {
        if !p.starts_with(corpus_root().join("tdata")) {
            continue;
        }
        n += 1;
        if let Err(e) = schema.validate_str(&read_text(&p)) {
            panic!("Validation of {}: {e}", p.display());
        }
    }
    assert!(n >= 50, "expected the t/tdata .bcf files, found {n}");
    let conf = corpus_root().join("tdata/general-configfile/biber-test.conf");
    builtin_schema(SchemaKind::Config)
        .validate_str(&read_text(&conf))
        .expect("Validation of biber-test.conf");
}

#[test]
fn libxml2_verdicts() {
    if !o3_available() {
        eprintln!("skipping: oracle O3 not built");
        return;
    }
    for (kind, schema_file, ext) in [
        (SchemaKind::Control, "bcf.rng", "bcf"),
        (SchemaKind::Config, "config.rng", "conf"),
    ] {
        let files = unique(files_with_ext(ext));
        let list: String = files.iter().map(|p| format!("{}\n", p.display())).collect();
        let mut child = Command::new(repo_root().join("vendor/perl/bin/perl"))
            .arg(repo_root().join("tools/oracle/probes/relaxng/verdicts.pl"))
            .arg(repo_root().join("data/schemata").join(schema_file))
            .arg("-")
            .env_remove("PERL5LIB")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(list.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let verdicts = String::from_utf8(out.stdout).unwrap();
        let mut checked = 0;
        for line in verdicts.lines() {
            let (f, v) = line.rsplit_once('\t').unwrap();
            if v == "error" {
                continue;
            }
            let ours = builtin_schema(kind).validate_str(&read_text(Path::new(f)));
            assert_eq!(
                ours.is_ok(),
                v == "valid",
                "{f}: libxml2 says {v}, relaxng says {ours:?}"
            );
            checked += 1;
        }
        assert!(checked > 10, "{schema_file}: only {checked} files checked");
    }
}

#[test]
fn convert_control_matches_libxslt() {
    let version = Command::new("xsltproc").arg("--version").output();
    let ok = version
        .as_ref()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("libxml 20913, libxslt 10135"));
    if !ok {
        eprintln!("skipping: xsltproc on libxslt 1.1.35 / libxml2 2.9.13 not available");
        return;
    }
    let xsl = repo_root().join("vendor/biber/data/bcf.xsl");
    if !xsl.exists() {
        eprintln!("skipping: vendor/biber not checked out");
        return;
    }
    let mut n = 0;
    for p in unique(files_with_ext("bcf")) {
        let text = read_text(&p);
        let Ok(doc) = roxmltree::Document::parse_with_options(&text, relaxng::parsing_options())
        else {
            continue; // not well-formed: biber stops before converting
        };
        let ours = bibers_core::bcfhtml::render(&doc);
        let theirs = Command::new("xsltproc")
            .arg(&xsl)
            .arg(&p)
            .output()
            .unwrap()
            .stdout;
        assert!(
            ours.as_bytes() == theirs.as_slice(),
            "{}: --convert-control output differs from libxslt",
            p.display()
        );
        n += 1;
    }
    assert!(n > 100);
}

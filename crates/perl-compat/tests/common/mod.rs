//! Shared helpers for the perl-compat integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Workspace root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A committed test-data file.
pub fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// The O3 oracle perl: `$BIBERS_O3_PERL`, else `<root>/vendor/perl/bin/perl`.
/// Returns `None` (and the `#[ignore]`d oracle tests skip) if unavailable.
pub fn oracle_perl() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("BIBERS_O3_PERL") {
        return Some(PathBuf::from(p));
    }
    let p = root().join("vendor/perl/bin/perl");
    p.exists().then_some(p)
}

/// `-I` for biber's modules: `$BIBERS_O3_LIB`, else the in-tree build.
pub fn oracle_lib() -> PathBuf {
    std::env::var_os("BIBERS_O3_LIB")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("vendor/perl-build/biber-src/blib/lib"))
}

/// Run a probe script from `tools/oracle/probes/perl-compat/` with the oracle
/// perl, feeding `stdin`; returns stdout.
pub fn run_probe(script: &str, args: &[&str], stdin: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    let Some(perl) = oracle_perl() else {
        eprintln!("oracle perl not found (set BIBERS_O3_PERL); skipping");
        return None;
    };
    let script = root().join("tools/oracle/probes/perl-compat").join(script);
    let mut child = Command::new(perl)
        .arg(format!("-I{}", oracle_lib().display()))
        .arg(script)
        .args(args)
        .env_remove("PERL5LIB")
        .env_remove("PERL5OPT")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn oracle perl");
    {
        let mut si = child.stdin.take().expect("stdin");
        si.write_all(stdin).expect("write stdin");
    }
    let out = child.wait_with_output().expect("oracle perl");
    assert!(
        out.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(out.stdout)
}

/// Read a committed JSON file.
pub fn read_json(name: &str) -> serde_json::Value {
    let s = std::fs::read_to_string(data(name)).expect("read test data");
    serde_json::from_str(&s).expect("parse test data")
}

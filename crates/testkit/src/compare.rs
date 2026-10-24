use crate::case::{Case, CompareMode, MESSAGES_FILE};
use crate::normalize;
use crate::run::RunOutput;
use anyhow::{Context, Result};
use std::fmt::Write as _;

/// Result of comparing a run with a case's expectations.
#[derive(Debug)]
pub enum Outcome {
    Pass,
    Fail(Vec<Mismatch>),
}

#[derive(Debug)]
pub enum Mismatch {
    TimedOut,
    ExitCode { expected: i32, actual: Option<i32> },
    MissingOutput { name: String },
    UnexpectedOutput { name: String },
    Output { name: String, diff: String },
    Messages { diff: String },
}

impl Outcome {
    pub fn is_pass(&self) -> bool {
        matches!(self, Outcome::Pass)
    }
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mismatch::TimedOut => write!(f, "timed out"),
            Mismatch::ExitCode { expected, actual } => {
                write!(f, "exit code: expected {expected}, got {actual:?}")
            }
            Mismatch::MissingOutput { name } => write!(f, "{name}: not produced"),
            Mismatch::UnexpectedOutput { name } => {
                write!(f, "{name}: produced, but the oracle produced none")
            }
            Mismatch::Output { name, diff } => write!(f, "{name} differs:\n{diff}"),
            Mismatch::Messages { diff } => write!(f, "WARN/ERROR messages differ:\n{diff}"),
        }
    }
}

/// Whether an actual exit code matches the expected one.
///
/// biber exits 0 on success and 2 (`EXIT_ERROR`) on reported errors. Any other
/// code means Perl died, and its value comes from `errno` (e.g. 25 from the
/// PAR binary, 255 from the source build). All such codes therefore count as
/// one "crash" class. A killed process (`None`) never matches.
pub fn exit_codes_match(expected: i32, actual: Option<i32>) -> bool {
    let is_crash = |c: i32| c != 0 && c != 2;
    match actual {
        Some(a) => a == expected || (is_crash(expected) && is_crash(a)),
        None => false,
    }
}

/// Maximum number of diff lines kept per mismatch.
const MAX_DIFF_LINES: usize = 40;

/// Normalise an output file's contents according to its type and the case's
/// compare mode.
pub fn normalize_output(case: &Case, name: &str, bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = if name.ends_with(".bbl") {
        normalize::bbl(&text)
    } else {
        text.into_owned()
    };
    let text = case.apply_substitutions(&text);
    match case.spec.compare {
        CompareMode::Exact => text,
        CompareMode::UnorderedLines => normalize::sorted_lines(&text),
    }
}

/// Compare `run` against the oracle's expectations (`expected/`).
pub fn compare(case: &Case, run: &RunOutput) -> Result<Outcome> {
    compare_in(case, run, &case.expected_dir())
}

/// Compare a bibers run: against `expected-bibers/` where the case has
/// reviewed bibers-specific expectations (documented deliberate differences,
/// docs/compat.md), otherwise against the oracle's `expected/`.
pub fn compare_bibers(case: &Case, run: &RunOutput) -> Result<Outcome> {
    match case.bibers_expected_dir() {
        Some(d) => compare_in(case, run, &d),
        None => compare(case, run),
    }
}

/// Compare `run` against the expectations in directory `exp_dir`.
pub fn compare_in(case: &Case, run: &RunOutput, exp_dir: &std::path::Path) -> Result<Outcome> {
    let meta = Case::load_meta_in(exp_dir)?.with_context(|| {
        format!(
            "{}: no {}/meta.toml (run `cargo xtask golden`)",
            case.id,
            exp_dir.display()
        )
    })?;
    let exp_dir = exp_dir.to_path_buf();
    let mut mismatches = Vec::new();

    if run.timed_out {
        mismatches.push(Mismatch::TimedOut);
    } else if !exit_codes_match(meta.exit_code, run.exit_code) {
        mismatches.push(Mismatch::ExitCode {
            expected: meta.exit_code,
            actual: run.exit_code,
        });
    }

    for name in &case.spec.outputs {
        let expected = std::fs::read(exp_dir.join(name)).ok();
        let actual = run.outputs.get(name).cloned().flatten();
        match (expected, actual) {
            (None, None) => {}
            (Some(_), None) => mismatches.push(Mismatch::MissingOutput { name: name.clone() }),
            (None, Some(_)) => mismatches.push(Mismatch::UnexpectedOutput { name: name.clone() }),
            (Some(e), Some(a)) if e == a => {}
            (Some(e), Some(a))
                if std::str::from_utf8(&e).is_err() || std::str::from_utf8(&a).is_err() =>
            {
                // Encoded (non-UTF-8) output must match byte for byte: a lossy
                // decode would hide differences (BOMs, invalid sequences).
                let at = e
                    .iter()
                    .zip(&a)
                    .position(|(x, y)| x != y)
                    .unwrap_or(e.len().min(a.len()));
                mismatches.push(Mismatch::Output {
                    name: name.clone(),
                    diff: format!(
                        "bytes differ at offset {at} (expected {} bytes, got {}; not UTF-8, compared exactly)\n{}",
                        e.len(),
                        a.len(),
                        diff(&String::from_utf8_lossy(&e), &String::from_utf8_lossy(&a))
                    ),
                });
            }
            (Some(e), Some(a)) => {
                let e = normalize_output(case, name, &e);
                let a = normalize_output(case, name, &a);
                if e != a {
                    mismatches.push(Mismatch::Output {
                        name: name.clone(),
                        diff: diff(&e, &a),
                    });
                }
            }
        }
    }

    let expected_msgs = case.apply_substitutions(
        &std::fs::read_to_string(exp_dir.join(MESSAGES_FILE)).unwrap_or_default(),
    );
    let actual_msgs = case.apply_substitutions(&messages_text(&run.messages));
    if expected_msgs != actual_msgs {
        mismatches.push(Mismatch::Messages {
            diff: diff(&expected_msgs, &actual_msgs),
        });
    }

    Ok(if mismatches.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail(mismatches)
    })
}

/// The on-disk form of a message list (`expected/messages.txt`).
pub fn messages_text(msgs: &[String]) -> String {
    let mut s = String::new();
    for m in msgs {
        s.push_str(m);
        s.push('\n');
    }
    s
}

/// A truncated unified diff (expected → actual).
pub fn diff(expected: &str, actual: &str) -> String {
    let d = similar::TextDiff::from_lines(expected, actual);
    let full = d
        .unified_diff()
        .context_radius(2)
        .header("expected", "actual")
        .to_string();
    let mut out = String::new();
    let total = full.lines().count();
    for line in full.lines().take(MAX_DIFF_LINES) {
        let _ = writeln!(out, "    {line}");
    }
    if total > MAX_DIFF_LINES {
        let _ = writeln!(out, "    … ({} more diff lines)", total - MAX_DIFF_LINES);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Non-UTF-8 output compares byte for byte (a UTF-16 BOM `FE FF` turned
    /// into `FF FF` decodes to the same lossy text).
    #[test]
    fn encoded_outputs_compare_byte_for_byte() {
        let root = crate::corpus_root();
        let case = Case::load(&root, &root.join("io/enc-out-utf-16")).unwrap();
        let exp = case.expected_dir();
        let name = case.spec.outputs[0].clone();
        let bytes = std::fs::read(exp.join(&name)).unwrap();
        assert_eq!(&bytes[..2], [0xFE, 0xFF]);
        let run = |b: Vec<u8>| RunOutput {
            exit_code: Some(0),
            timed_out: false,
            outputs: [(name.clone(), Some(b))].into(),
            messages: Vec::new(),
            blg: String::new(),
            stdout: String::new(),
            stderr: String::new(),
            elapsed: std::time::Duration::ZERO,
            workdir: None,
        };
        let output_mismatch = |o: Outcome| match o {
            Outcome::Pass => false,
            Outcome::Fail(ms) => ms.iter().any(|m| matches!(m, Mismatch::Output { .. })),
        };
        assert!(!output_mismatch(
            compare(&case, &run(bytes.clone())).unwrap()
        ));
        let mut bom = bytes;
        bom[0] = 0xFF;
        assert!(output_mismatch(compare(&case, &run(bom)).unwrap()));
    }

    #[test]
    fn crash_exit_codes_form_one_class() {
        assert!(exit_codes_match(0, Some(0)));
        assert!(exit_codes_match(2, Some(2)));
        assert!(exit_codes_match(25, Some(255)));
        assert!(!exit_codes_match(25, Some(2)));
        assert!(!exit_codes_match(0, Some(2)));
        assert!(!exit_codes_match(2, None));
    }
}

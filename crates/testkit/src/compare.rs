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

/// Compare `run` against the expectations stored in `case`'s `expected/`.
pub fn compare(case: &Case, run: &RunOutput) -> Result<Outcome> {
    let meta = case.load_meta()?.with_context(|| {
        format!(
            "{}: no expected/meta.toml (run `cargo xtask golden`)",
            case.id
        )
    })?;
    let exp_dir = case.expected_dir();
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
    use super::exit_codes_match;

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

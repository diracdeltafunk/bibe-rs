use crate::case::Case;
use crate::normalize;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;

/// A program that can process a case: bibers or one of the oracles.
#[derive(Debug, Clone)]
pub struct Runner {
    /// Short name for reports (`bibers`, `o1`, `o3`).
    pub name: String,
    /// The program and any leading arguments.
    pub argv: Vec<String>,
}

/// The fixed environment every run gets, so the oracle and bibers see the
/// same locale regardless of the developer's shell.
pub const BASE_ENV: &[(&str, &str)] = &[("LANG", "en_US.UTF-8"), ("LC_ALL", "en_US.UTF-8")];

/// The result of running one case.
#[derive(Debug)]
pub struct RunOutput {
    /// `None` if the process was killed by a signal or timed out.
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    /// Output file name → contents (`None` if the file was not produced).
    pub outputs: BTreeMap<String, Option<Vec<u8>>>,
    /// Normalised, sorted WARN/ERROR messages from the case's `.blg`.
    pub messages: Vec<String>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
    /// The work directory, kept only if requested.
    pub workdir: Option<PathBuf>,
}

impl Runner {
    pub fn new(name: impl Into<String>, argv: Vec<String>) -> Runner {
        Runner {
            name: name.into(),
            argv,
        }
    }

    /// Run `case` in a fresh temporary work directory. If `keep` is set, the
    /// work directory is preserved and returned in [`RunOutput::workdir`].
    pub fn run(&self, case: &Case, corpus_root: &Path, keep: bool) -> Result<RunOutput> {
        let tmp = tempfile::Builder::new().prefix("bibers-case-").tempdir()?;
        let work = tmp.path().canonicalize()?;
        for f in case.input_files(corpus_root)? {
            let name = f.file_name().expect("input files have names");
            std::fs::copy(&f, work.join(name))
                .with_context(|| format!("copying {}", f.display()))?;
        }

        let (prog, lead) = self.argv.split_first().context("empty runner argv")?;
        let mut cmd = Command::new(prog);
        cmd.args(lead)
            .args(&case.spec.args)
            .current_dir(&work)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in BASE_ENV {
            cmd.env(k, v);
        }
        cmd.envs(&case.spec.env);

        let start = Instant::now();
        let mut child = cmd.spawn().with_context(|| format!("spawning {prog}"))?;
        // Drain pipes on threads so a chatty child can't block on a full pipe.
        let mut out_pipe = child.stdout.take().expect("piped");
        let mut err_pipe = child.stderr.take().expect("piped");
        let out_t = std::thread::spawn(move || {
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut out_pipe, &mut b).ok();
            b
        });
        let err_t = std::thread::spawn(move || {
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut err_pipe, &mut b).ok();
            b
        });
        let status = child.wait_timeout(Duration::from_secs(case.spec.timeout_secs))?;
        let timed_out = status.is_none();
        let status = match status {
            Some(s) => Some(s),
            None => {
                child.kill().ok();
                child.wait().ok()
            }
        };
        let elapsed = start.elapsed();
        let stdout_bytes = out_t.join().unwrap_or_default();
        let stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();
        let stderr = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned();

        let mut outputs = BTreeMap::new();
        for o in &case.spec.outputs {
            let data = if o == crate::case::STDOUT_OUTPUT {
                Some(stdout_bytes.clone())
            } else {
                std::fs::read(work.join(o)).ok()
            };
            outputs.insert(o.clone(), data);
        }
        let blg = std::fs::read(work.join(case.log_file()))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let messages = normalize::blg_messages(&blg, &work);

        let workdir = if keep { Some(tmp.keep()) } else { None };
        Ok(RunOutput {
            exit_code: if timed_out {
                None
            } else {
                status.and_then(|s| s.code())
            },
            timed_out,
            outputs,
            messages,
            stdout,
            stderr,
            elapsed,
            workdir,
        })
    }
}

//! Phase-level differential testing against biber's internal state.
//!
//! [`phase_dumps`] runs the source-built biber (oracle O3) on a corpus case
//! with `tools/oracle/probes/pipeline/BiberPhaseDump.pm` loaded, which writes
//! biber's full section state after every processing phase. Results are
//! cached under `target/phase-dumps/<case-id>/` (keyed by the case inputs), so
//! repeated test runs are cheap.
//!
//! A pipeline phase is tested by loading the dump *before* it into the Rust
//! model, running the Rust phase, dumping, and diffing against biber's dump
//! *after* it (`bibers_core::model::dump::json_diff`).
//!
//! These tests need the O3 oracle (`tools/oracle/build-perl-oracle.sh`); use
//! [`o3_available`] to skip when it is missing (e.g. in CI).

use crate::case::Case;
use crate::repo_root;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One phase dump file.
#[derive(Debug, Clone)]
pub struct PhaseDump {
    /// Sequence number (order of execution).
    pub seq: u32,
    /// Section number.
    pub section: String,
    /// Phase name (`fetch_data`, …, `sort_list`, `generate_contextdata`).
    pub phase: String,
    /// Data list name for per-list phases (sanitised as in the file name).
    pub list: Option<String>,
    pub path: PathBuf,
}

impl PhaseDump {
    /// Parse the dump's JSON (`{phase, section, list, state}`).
    pub fn load(&self) -> Result<serde_json::Value> {
        let bytes = std::fs::read(&self.path)?;
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", self.path.display()))
    }

    /// The `state` object.
    pub fn state(&self) -> Result<serde_json::Value> {
        let mut v = self.load()?;
        Ok(v.get_mut("state")
            .map(serde_json::Value::take)
            .unwrap_or_default())
    }
}

/// Whether the O3 oracle is built.
pub fn o3_available() -> bool {
    repo_root().join("vendor/perl/bin/perl").exists()
        && repo_root()
            .join("vendor/perl-build/biber-src/blib/lib/Biber.pm")
            .exists()
}

fn inputs_fingerprint(case: &Case, corpus_root: &Path) -> Result<String> {
    // Cheap content fingerprint: sizes + FNV-1a of the bytes of every input,
    // the args, and the dumper module itself.
    let mut h: u64 = 0xcbf29ce484222325;
    let mut feed = |b: &[u8]| {
        for x in b {
            h ^= *x as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    for f in case.input_files(corpus_root)? {
        feed(f.file_name().unwrap().as_encoded_bytes());
        feed(&std::fs::read(&f)?);
    }
    feed(case.spec.args.join("\0").as_bytes());
    feed(&std::fs::read(
        repo_root().join("tools/oracle/probes/pipeline/BiberPhaseDump.pm"),
    )?);
    Ok(format!("{h:016x}"))
}

/// Run (or reuse cached) biber phase dumps for `case`. Returns the dumps in
/// execution order.
pub fn phase_dumps(case: &Case, corpus_root: &Path) -> Result<Vec<PhaseDump>> {
    if !o3_available() {
        bail!("oracle O3 not built (tools/oracle/build-perl-oracle.sh)");
    }
    let fp = inputs_fingerprint(case, corpus_root)?;
    let dir = repo_root()
        .join("target/phase-dumps")
        .join(case.id.replace('/', "__"));
    let stamp = dir.join("FINGERPRINT");
    let fresh =
        dir.join("dumps").is_dir() && std::fs::read_to_string(&stamp).is_ok_and(|s| s == fp);
    if !fresh {
        // Generate into a private directory and rename it into place, so
        // tests running in parallel never see a half-written cache.
        let tmp = dir.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let work = tmp.join("work");
        let dumps = tmp.join("dumps");
        std::fs::create_dir_all(&work)?;
        std::fs::create_dir_all(&dumps)?;
        for f in case.input_files(corpus_root)? {
            std::fs::copy(&f, work.join(f.file_name().unwrap()))?;
        }
        let status = Command::new(repo_root().join("tools/oracle/probes/pipeline/run.sh"))
            .arg(&dumps)
            .args(&case.spec.args)
            .current_dir(&work)
            .envs(crate::run::BASE_ENV.iter().copied())
            .envs(&case.spec.env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running the phase-dump oracle")?;
        // biber may legitimately exit non-zero (error cases); dumps up to the
        // failure are still useful.
        let _ = status;
        std::fs::write(tmp.join("FINGERPRINT"), &fp)?;
        // Replace a stale cache; if another process installed a fresh one
        // meanwhile, keep theirs.
        let now_fresh = std::fs::read_to_string(&stamp).is_ok_and(|s| s == fp);
        if now_fresh {
            std::fs::remove_dir_all(&tmp).ok();
        } else {
            if dir.exists() {
                std::fs::remove_dir_all(&dir).ok();
            }
            if std::fs::rename(&tmp, &dir).is_err() {
                // Lost a race with another process; use its result.
                std::fs::remove_dir_all(&tmp).ok();
            }
        }
    }
    list_dumps(&dir.join("dumps"))
}

/// List the dump files in a directory, in execution order.
pub fn list_dumps(dir: &Path) -> Result<Vec<PhaseDump>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        // NNN-s<sec>-<phase>[-<list>]
        let mut parts = stem.splitn(3, '-');
        let (Some(seq), Some(sec), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let (phase, list) = match rest.split_once('-') {
            Some((p, l)) => (p.to_string(), Some(l.to_string())),
            None => (rest.to_string(), None),
        };
        out.push(PhaseDump {
            seq: seq.parse().unwrap_or(0),
            section: sec.trim_start_matches('s').to_string(),
            phase,
            list,
            path: p,
        });
    }
    out.sort_by_key(|d| d.seq);
    Ok(out)
}

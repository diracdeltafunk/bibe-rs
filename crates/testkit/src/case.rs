use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Contents of a case's `case.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseSpec {
    /// One-line human description.
    #[serde(default)]
    pub description: String,
    /// Where the inputs came from (e.g. `vendor/biber/t/tdata/names.bcf`).
    #[serde(default)]
    pub source: String,
    /// Arguments passed to the program (after the program name). Cases should
    /// normally include `--noconf` so a user's `biber.conf` can't interfere.
    pub args: Vec<String>,
    /// Files copied from `tests/corpus/shared/` into the work directory.
    #[serde(default)]
    pub shared: Vec<String>,
    /// Output files (relative to the work directory) compared with `expected/`.
    /// The name [`STDOUT_OUTPUT`] stands for the program's standard output
    /// (for `--output-file -`).
    pub outputs: Vec<String>,
    /// The `.blg` whose WARN/ERROR messages are compared. Defaults to the
    /// first output with its extension replaced by `.blg`.
    #[serde(default)]
    pub log: Option<String>,
    /// Extra environment variables for the run.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// How outputs are compared.
    #[serde(default)]
    pub compare: CompareMode,
    /// Free-form tags for filtering (e.g. `style:authoryear`, `feature:xdata`).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Ids from `docs/compat.md` (`DC-NNN`) whose documented differences are
    /// accepted for this case. Each must be justified there.
    #[serde(default)]
    pub allow: Vec<String>,
    /// External programs the case needs (e.g. `kpsewhich` for
    /// `--recodedata`, which biber and bibers look up only via kpsewhich).
    /// The ratchet skips the case where one is missing (CI has no TeX).
    #[serde(default)]
    pub requires: Vec<String>,
    /// Timeout in seconds for one run.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Case-specific regex substitutions applied to both sides of every
    /// comparison (outputs and messages), after the standard normalisation.
    /// Only for oracle nondeterminism that can't be fixed otherwise (e.g.
    /// `$MAPUNIQ` values, BB-113); say why in a comment in `case.toml`.
    #[serde(default)]
    pub normalize: Vec<Substitution>,
}

/// A regex substitution (`regex` crate syntax; `$1` etc. in `replace`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Substitution {
    pub pattern: String,
    pub replace: String,
}

fn default_timeout() -> u64 {
    300
}

/// How output files are compared (after normalisation).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompareMode {
    /// Byte-for-byte equality after normalisation.
    #[default]
    Exact,
    /// Equality of the multiset of lines. Used only for cases where the oracle
    /// itself is nondeterministic in ways normalisation can't undo (BB-061/062).
    UnorderedLines,
}

/// Contents of `expected/meta.toml`, written by the golden generator.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExpectedMeta {
    /// Which oracle produced the expectations (`o1` or `o3`).
    pub oracle: String,
    /// The oracle's `--version` line.
    pub oracle_version: String,
    /// Exit code of the oracle run.
    pub exit_code: i32,
    /// Whether two oracle runs gave identical normalised outputs.
    pub deterministic: bool,
}

/// A loaded case.
#[derive(Debug, Clone)]
pub struct Case {
    /// `<suite>/<name>`, relative to the corpus root.
    pub id: String,
    pub dir: PathBuf,
    pub spec: CaseSpec,
}

pub const CASE_FILE: &str = "case.toml";
pub const EXPECTED_DIR: &str = "expected";
/// Reviewed expectations for bibers where it deliberately differs from biber
/// (docs/compat.md); only allowed when `allow` lists the `DC-NNN` ids.
pub const BIBERS_EXPECTED_DIR: &str = "expected-bibers";
pub const MESSAGES_FILE: &str = "messages.txt";
/// The `outputs` name of the program's standard output.
pub const STDOUT_OUTPUT: &str = "@stdout";
pub const META_FILE: &str = "meta.toml";

impl Case {
    pub fn load(corpus_root: &Path, dir: &Path) -> Result<Case> {
        let path = dir.join(CASE_FILE);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let spec: CaseSpec =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        for sub in &spec.normalize {
            regex::Regex::new(&sub.pattern)
                .with_context(|| format!("{}: bad normalize pattern", path.display()))?;
        }
        if dir.join(BIBERS_EXPECTED_DIR).is_dir() && spec.allow.is_empty() {
            bail!(
                "{}: {BIBERS_EXPECTED_DIR}/ requires `allow` to list the docs/compat.md ids it implements",
                path.display()
            );
        }
        if spec.outputs.is_empty() {
            bail!("{}: `outputs` must not be empty", path.display());
        }
        let id = dir
            .strip_prefix(corpus_root)
            .unwrap_or(dir)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        Ok(Case {
            id,
            dir: dir.to_path_buf(),
            spec,
        })
    }

    pub fn expected_dir(&self) -> PathBuf {
        self.dir.join(EXPECTED_DIR)
    }

    /// `expected-bibers/`: reviewed bibers-specific expectations, if any.
    pub fn bibers_expected_dir(&self) -> Option<PathBuf> {
        let d = self.dir.join(BIBERS_EXPECTED_DIR);
        d.is_dir().then_some(d)
    }

    /// The `.blg` file whose messages are compared.
    pub fn log_file(&self) -> String {
        self.spec.log.clone().unwrap_or_else(|| {
            let first = &self.spec.outputs[0];
            match first.rsplit_once('.') {
                Some((stem, _)) => format!("{stem}.blg"),
                None => format!("{first}.blg"),
            }
        })
    }

    /// Input files: everything in the case directory except `case.toml` and
    /// `expected/`, plus the listed shared files.
    pub fn input_files(&self, corpus_root: &Path) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == CASE_FILE || name == EXPECTED_DIR || name == BIBERS_EXPECTED_DIR {
                continue;
            }
            if entry.file_type()?.is_file() {
                files.push(entry.path());
            } else {
                bail!(
                    "{}: subdirectories in a case are not supported ({})",
                    self.id,
                    name.to_string_lossy()
                );
            }
        }
        for s in &self.spec.shared {
            let p = corpus_root.join("shared").join(s);
            if !p.is_file() {
                bail!("{}: shared file {} not found", self.id, p.display());
            }
            files.push(p);
        }
        files.sort();
        Ok(files)
    }

    pub fn load_meta(&self) -> Result<Option<ExpectedMeta>> {
        Self::load_meta_in(&self.expected_dir())
    }

    /// `meta.toml` of an expectations directory.
    pub fn load_meta_in(dir: &Path) -> Result<Option<ExpectedMeta>> {
        let p = dir.join(META_FILE);
        if !p.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&p)?;
        Ok(Some(
            toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))?,
        ))
    }

    /// Apply the case's [`CaseSpec::normalize`] substitutions.
    pub fn apply_substitutions(&self, text: &str) -> String {
        let mut text = text.to_string();
        for sub in &self.spec.normalize {
            let re = regex::Regex::new(&sub.pattern).expect("validated in Case::load");
            text = re.replace_all(&text, sub.replace.as_str()).into_owned();
        }
        text
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.spec.tags.iter().any(|t| t == tag)
    }
}

/// Find all cases under `corpus_root` (directories containing `case.toml`),
/// sorted by id.
pub fn discover(corpus_root: &Path) -> Result<Vec<Case>> {
    let mut out = Vec::new();
    walk(corpus_root, corpus_root, &mut out)?;
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<Case>) -> Result<()> {
    if dir.join(CASE_FILE).is_file() {
        out.push(Case::load(root, dir)?);
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if e.file_type()?.is_dir() && e.file_name() != "shared" {
            walk(root, &e.path(), out)?;
        }
    }
    Ok(())
}

impl Case {
    /// The programs of [`CaseSpec::requires`] that are not on `PATH`.
    pub fn missing_requirements(&self) -> Vec<String> {
        self.spec
            .requires
            .iter()
            .filter(|p| !program_on_path(p))
            .cloned()
            .collect()
    }
}

fn program_on_path(prog: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".bat", ".cmd"]
    } else {
        &[""]
    };
    std::env::split_paths(&path)
        .any(|d| exts.iter().any(|e| d.join(format!("{prog}{e}")).is_file()))
}

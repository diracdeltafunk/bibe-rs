//! Running a fuzz input through a `.bcf` template on the oracle and on
//! bibers, and comparing the results the way the corpus harness does.

use super::bib::FuzzInput;
use anyhow::{Context, Result, bail};
use bibers_testkit::case::{CaseSpec, CompareMode};
use bibers_testkit::compare::{exit_codes_match, normalize_output};
use bibers_testkit::{Case, RunOutput, Runner, corpus_root};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::LazyLock;

/// The name of the generated data file.
pub const BIB_NAME: &str = "fuzz.bib";

/// Per-run timeout (seconds).
const TIMEOUT_SECS: u64 = 40;

/// A `.bcf` from a corpus case, with its data source swapped for
/// [`BIB_NAME`] and its citations replaced by the fuzz input's.
#[derive(Debug, Clone)]
pub struct Template {
    /// The corpus case it was taken from (e.g. `styles/authoryear`).
    pub id: String,
    /// File name of the `.bcf` (e.g. `authoryear.bcf`).
    pub bcf_name: String,
    /// Text before the citation keys of section 0.
    head: String,
    /// Text after them.
    tail: String,
}

impl Template {
    pub fn load(id: &str) -> Result<Template> {
        let root = corpus_root();
        let case = Case::load(&root, &root.join(id)).with_context(|| format!("template {id}"))?;
        let bcf_name = case
            .spec
            .args
            .iter()
            .rev()
            .find(|a| a.ends_with(".bcf"))
            .with_context(|| format!("template {id}: no .bcf in args"))?
            .clone();
        let text = std::fs::read_to_string(case.dir.join(&bcf_name))?;
        let ds = Regex::new(r"(<bcf:datasource [^>]*>)[^<]*(</bcf:datasource>)").unwrap();
        if ds.find_iter(&text).count() != 1 {
            bail!("template {id}: expected exactly one data source");
        }
        let text = ds
            .replace(&text, format!("${{1}}{BIB_NAME}${{2}}").as_str())
            .into_owned();
        let open = "<bcf:section number=\"0\">\n";
        let start = text
            .find(open)
            .with_context(|| format!("template {id}: no section 0"))?
            + open.len();
        let end = start
            + text[start..]
                .find("  </bcf:section>")
                .with_context(|| format!("template {id}: section 0 not closed"))?;
        if text[end..].contains("<bcf:section ") {
            bail!("template {id}: more than one section");
        }
        Ok(Template {
            id: id.to_string(),
            bcf_name,
            head: text[..start].to_string(),
            tail: text[end..].to_string(),
        })
    }

    pub fn bcf(&self, input: &FuzzInput) -> String {
        let mut head = self.head.clone();
        for (k, v) in &input.opts {
            // Every occurrence: biblatex repeats global options in per-type blocks.
            let re = Regex::new(&format!(
                r"(<bcf:key>{}</bcf:key>\s*<bcf:value>)[^<]*(</bcf:value>)",
                regex::escape(k)
            ))
            .expect("valid regex");
            head = re
                .replace_all(&head, format!("${{1}}{}${{2}}", super::bib::xml_escape(v)))
                .into_owned();
        }
        format!("{head}{}{}", input.render_citekeys(), self.tail)
    }

    pub fn bbl_name(&self) -> String {
        format!("{}.bbl", self.bcf_name.trim_end_matches(".bcf"))
    }

    /// The case spec used for fuzz runs and for minimised corpus cases.
    pub fn spec(&self, description: String, source: String) -> CaseSpec {
        CaseSpec {
            description,
            source,
            args: vec!["--noconf".into(), self.bcf_name.clone()],
            shared: vec![],
            outputs: vec![self.bbl_name()],
            log: None,
            env: BTreeMap::new(),
            compare: CompareMode::Exact,
            tags: vec!["suite:fuzz".into(), format!("template:{}", self.id)],
            allow: vec![],
            timeout_secs: TIMEOUT_SECS,
            normalize: vec![],
        }
    }

    /// Write the case inputs into `dir` and return the in-memory case.
    pub fn materialise(&self, input: &FuzzInput, dir: &Path) -> Result<Case> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(BIB_NAME), input.render_bib())?;
        std::fs::write(dir.join(&self.bcf_name), self.bcf(input))?;
        Ok(Case {
            id: format!("fuzz/{}", self.id),
            dir: dir.to_path_buf(),
            spec: self.spec(String::new(), String::new()),
        })
    }
}

/// Outcome of one differential run.
#[derive(Debug)]
pub struct Verdict {
    /// Empty when the outputs agree; otherwise the diff signature.
    pub shapes: BTreeSet<String>,
    /// Human-readable details (diffs).
    pub detail: String,
    pub oracle: RunOutput,
    pub bibers: RunOutput,
}

/// Run `input` through `tpl` on both implementations and compare.
/// When `confirm` is set, a mismatch triggers a second oracle run to rule
/// out nondeterminism in biber itself.
pub fn check(
    input: &FuzzInput,
    tpl: &Template,
    oracle: &Runner,
    bibers: &Runner,
    confirm: bool,
) -> Result<Verdict> {
    let tmp = tempfile::Builder::new().prefix("bibers-fuzz-").tempdir()?;
    let case = tpl.materialise(input, tmp.path())?;
    let root = corpus_root();
    let o = oracle.run(&case, &root, false)?;
    let b = bibers.run(&case, &root, false)?;
    let (mut shapes, mut detail) = compare_runs(&case, &o, &b);
    if confirm && !shapes.is_empty() && !shapes.contains("o3-timeout") {
        let o2 = oracle.run(&case, &root, false)?;
        let (s2, _) = compare_runs(&case, &o, &o2);
        if !s2.is_empty() {
            shapes = BTreeSet::from(["o3-nondeterministic".to_string()]);
            detail.insert_str(0, "oracle runs differ from each other\n");
        }
    }
    Ok(Verdict {
        shapes,
        detail,
        oracle: o,
        bibers: b,
    })
}

/// Messages after the fuzz-specific normalisations of documented
/// deliberate differences:
/// - DC-042: btparse diagnostics name the user's file, not a temp file;
/// - DC-046: the text of undecodable-byte errors differs;
/// - DC-017: which way round biber names an XDATA cycle depends on hash
///   order, so the pair is sorted.
pub fn fuzz_messages(msgs: &[String]) -> Vec<String> {
    static ENC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(cannot be read in encoding '[^']*'): .*").unwrap());
    static CYCLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(ERROR - Circular XDATA inheritance between )'(.*)'<->'(.*)'$").unwrap()
    });
    let mut out: Vec<String> = msgs
        .iter()
        .map(|m| {
            let m = m.replace("<TMPBIB>", BIB_NAME);
            let m = ENC.replace_all(&m, "$1: <ENCERR>").into_owned();
            match CYCLE.captures(&m) {
                Some(c) if c[2] > c[3] => format!("{}'{}'<->'{}'", &c[1], &c[3], &c[2]),
                _ => m,
            }
        })
        .collect();
    out.sort();
    out
}

/// Exit-code class; a process killed by a signal (`None`; timeouts are
/// handled separately) counts as a crash, like a Perl `die`.
fn exit_class(c: Option<i32>) -> String {
    match c {
        None => "crash".into(),
        Some(0) => "0".into(),
        Some(2) => "2".into(),
        Some(_) => "crash".into(),
    }
}

/// Compare two runs of the same case. Returns the diff signature (empty if
/// they agree) and a readable report.
pub fn compare_runs(case: &Case, a: &RunOutput, b: &RunOutput) -> (BTreeSet<String>, String) {
    static PANIC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"panicked at ([^:\n]+:\d+)").unwrap());
    let mut shapes = BTreeSet::new();
    let mut detail = String::new();
    if a.timed_out {
        shapes.insert("o3-timeout".to_string());
        return (shapes, "oracle timed out\n".into());
    }
    if let Some(c) = PANIC.captures(&b.stderr) {
        shapes.insert(format!("panic:{}", c[1].replace('\\', "/")));
        detail.push_str(&b.stderr);
        return (shapes, detail);
    }
    if b.timed_out {
        shapes.insert("bibers-timeout".to_string());
        return (shapes, "bibers timed out\n".into());
    }
    let ea = a.exit_code.unwrap_or(-1);
    if exit_class(a.exit_code) == "crash" && exit_class(b.exit_code) != "crash" {
        shapes.insert(format!("o3-crash:{}", crash_summary(&a.stderr)));
        detail.push_str(&a.stderr);
        return (shapes, detail);
    }
    if !exit_codes_match(ea, b.exit_code) {
        shapes.insert(format!(
            "exit:{}->{}",
            exit_class(a.exit_code),
            exit_class(b.exit_code)
        ));
        detail.push_str(&format!(
            "exit code: oracle {:?}, bibers {:?}\nbibers stderr: {}\n",
            a.exit_code, b.exit_code, b.stderr
        ));
    }
    for name in &case.spec.outputs {
        let oa = a.outputs.get(name).cloned().flatten();
        let ob = b.outputs.get(name).cloned().flatten();
        match (oa, ob) {
            (None, None) => {}
            (Some(_), None) => {
                shapes.insert(format!("missing-output:{name}"));
            }
            (None, Some(_)) => {
                shapes.insert(format!("unexpected-output:{name}"));
            }
            (Some(x), Some(y)) => {
                let x = normalize_output(case, name, &x);
                let y = normalize_output(case, name, &y);
                if x != y {
                    for (sign, line) in changed_lines(&x, &y) {
                        shapes.insert(format!("{sign}{}", bbl_shape(&line)));
                    }
                    detail.push_str(&format!(
                        "{name} differs:\n{}",
                        bibers_testkit::compare::diff(&x, &y)
                    ));
                }
            }
        }
    }
    let ma = fuzz_messages(&a.messages).join("\n");
    let mb = fuzz_messages(&b.messages).join("\n");
    if ma != mb {
        for (sign, line) in changed_lines(&ma, &mb) {
            shapes.insert(format!("msg{sign}{}", msg_shape(&line)));
        }
        detail.push_str(&format!(
            "messages differ:\n{}",
            bibers_testkit::compare::diff(&(ma + "\n"), &(mb + "\n"))
        ));
    }
    (shapes, detail)
}

/// The lines only in `a` (`-`) or only in `b` (`+`).
fn changed_lines(a: &str, b: &str) -> Vec<(char, String)> {
    use similar::{ChangeTag, TextDiff};
    TextDiff::from_lines(a, b)
        .iter_all_changes()
        .filter_map(|c| match c.tag() {
            ChangeTag::Delete => Some(('-', c.value().trim_end().to_string())),
            ChangeTag::Insert => Some(('+', c.value().trim_end().to_string())),
            ChangeTag::Equal => None,
        })
        .collect()
}

/// An abstraction of a `.bbl` line: the macro and, where it names a field,
/// the field name; values are dropped.
pub fn bbl_shape(line: &str) -> String {
    static CMD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\\([A-Za-z]+)(?:\{([^}]*)\})?").unwrap());
    static PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([a-z]+)=\{").unwrap());
    let t = line.trim();
    if let Some(c) = CMD.captures(t) {
        let cmd = &c[1];
        return match cmd {
            "field" | "strng" | "range" | "list" | "name" | "verb" | "true" | "false"
            | "fieldsource" => format!("\\{cmd}{{{}}}", c.get(2).map_or("", |m| m.as_str())),
            _ => format!("\\{cmd}"),
        };
    }
    if let Some(c) = PART.captures(t) {
        return format!("namepart:{}", &c[1]);
    }
    if t.starts_with("{{") {
        return "nameopts".into();
    }
    if t.starts_with('}') {
        return "close".into();
    }
    let mut s: String = t
        .chars()
        .map(|c| if c.is_ascii_digit() { 'N' } else { c })
        .collect();
    s.truncate(s.char_indices().nth(40).map_or(s.len(), |(i, _)| i));
    format!("other:{s}")
}

/// An abstraction of a message: quoted values and numbers dropped.
pub fn msg_shape(line: &str) -> String {
    static Q1: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'[^']*'").unwrap());
    static Q2: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""[^"]*""#).unwrap());
    static NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").unwrap());
    let s = Q1.replace_all(line, "'*'");
    let s = Q2.replace_all(&s, "\"*\"");
    let s = NUM.replace_all(&s, "N");
    let mut s = s.into_owned();
    if let Some((i, _)) = s.char_indices().nth(90) {
        s.truncate(i);
    }
    s
}

/// A short, path-free summary of a Perl die message.
fn crash_summary(stderr: &str) -> String {
    static LOC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\S*/((?:[A-Za-z_]+/)*[A-Za-z_]+\.p[ml]) line \d+").unwrap());
    let line = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    let line = LOC.replace_all(line, "$1");
    msg_shape(&line)
}

/// A short stable id for a signature.
pub fn signature_id(shapes: &BTreeSet<String>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in shapes {
        for b in s.bytes().chain([0u8]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{:08x}", h >> 32)
}

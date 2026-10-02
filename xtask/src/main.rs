//! Developer tasks. Run `cargo xtask help`.

use anyhow::{Context, Result, bail};
use bibers_testkit::case::{EXPECTED_DIR, MESSAGES_FILE, META_FILE};
use bibers_testkit::compare::{messages_text, normalize_output};
use bibers_testkit::{Case, ExpectedMeta, Outcome, RunOutput, Runner, corpus_root, repo_root};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::Mutex;
use std::time::Instant;

const HELP: &str = "\
cargo xtask <command> [options] [FILTER...]

Commands:
  list                 List corpus cases (id, tags).
  golden               (Re)generate expected outputs by running an oracle twice.
      --oracle o1|o3     Oracle to use (default o1: `biber` on PATH, or $BIBERS_O1).
      --check            Don't write; verify the oracle reproduces the stored expectations.
  corpus               Run an implementation over the corpus and compare with expectations.
      --impl bibers|o1|o3  Implementation to run (default bibers).
      --release          Use a release build of bibers.
      --keep             Keep work directories of failing cases.
      -v                 Show diffs for failures.
      --update-passing   Rewrite tests/corpus/passing.txt with the passing case ids (bibers only).
  help                 Show this message.

FILTER: a substring of the case id, or `tag:<tag>`. Several filters are OR-ed.
Options common to golden/corpus:
      -j N               Number of parallel jobs (default: available cores).
";

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

struct Opts {
    filters: Vec<String>,
    oracle: String,
    implementation: String,
    check: bool,
    release: bool,
    keep: bool,
    verbose: bool,
    update_passing: bool,
    jobs: usize,
}

fn parse_opts(args: &[String]) -> Result<Opts> {
    let mut o = Opts {
        filters: vec![],
        oracle: "o1".into(),
        implementation: "bibers".into(),
        check: false,
        release: false,
        keep: false,
        verbose: false,
        update_passing: false,
        jobs: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--oracle" => o.oracle = it.next().context("--oracle needs a value")?.clone(),
            "--impl" => o.implementation = it.next().context("--impl needs a value")?.clone(),
            "--check" => o.check = true,
            "--release" => o.release = true,
            "--keep" => o.keep = true,
            "-v" | "--verbose" => o.verbose = true,
            "--update-passing" => o.update_passing = true,
            "-j" => o.jobs = it.next().context("-j needs a value")?.parse()?,
            s if s.starts_with('-') => bail!("unknown option {s}\n\n{HELP}"),
            s => o.filters.push(s.to_string()),
        }
    }
    Ok(o)
}

fn real_main() -> Result<ExitCode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((cmd, rest)) = args.split_first() else {
        print!("{HELP}");
        return Ok(ExitCode::SUCCESS);
    };
    let opts = parse_opts(rest)?;
    match cmd.as_str() {
        "list" => list(&opts),
        "golden" => golden(&opts),
        "corpus" => corpus(&opts),
        "help" | "-h" | "--help" => {
            print!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
        other => bail!("unknown command {other}\n\n{HELP}"),
    }
}

fn select(opts: &Opts) -> Result<Vec<Case>> {
    let all = bibers_testkit::discover(&corpus_root())?;
    if opts.filters.is_empty() {
        return Ok(all);
    }
    Ok(all
        .into_iter()
        .filter(|c| {
            opts.filters.iter().any(|f| match f.strip_prefix("tag:") {
                Some(tag) => c.has_tag(tag),
                None => c.id.contains(f.as_str()),
            })
        })
        .collect())
}

fn runner(name: &str, release: bool) -> Result<Runner> {
    Ok(match name {
        "o1" => Runner::new(
            "o1",
            vec![std::env::var("BIBERS_O1").unwrap_or_else(|_| "biber".into())],
        ),
        "o3" => {
            let p = repo_root().join("tools/oracle/biber-src");
            if !p.exists() {
                bail!("{} not found; see tools/oracle/README.md", p.display());
            }
            Runner::new("o3", vec![p.to_string_lossy().into_owned()])
        }
        "bibers" => {
            let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
            cmd.args(["build", "--quiet", "-p", "bibers"])
                .current_dir(repo_root());
            if release {
                cmd.arg("--release");
            }
            if !cmd.status()?.success() {
                bail!("building bibers failed");
            }
            let profile = if release { "release" } else { "debug" };
            let bin = repo_root().join("target").join(profile).join("bibers");
            Runner::new("bibers", vec![bin.to_string_lossy().into_owned()])
        }
        other => bail!("unknown implementation/oracle `{other}` (expected bibers, o1 or o3)"),
    })
}

fn version_line(r: &Runner) -> String {
    let (prog, lead) = r.argv.split_first().expect("non-empty argv");
    Command::new(prog)
        .args(lead)
        .arg("--version")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

/// Run `f` over `cases` on `jobs` threads, returning results in case order.
fn par_map<T: Send>(cases: &[Case], jobs: usize, f: impl Fn(&Case) -> T + Sync) -> Vec<T> {
    let next = Mutex::new(0usize);
    let results: Mutex<Vec<Option<T>>> = Mutex::new((0..cases.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| {
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        let i = *n;
                        *n += 1;
                        i
                    };
                    if i >= cases.len() {
                        break;
                    }
                    let r = f(&cases[i]);
                    results.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.expect("every case ran"))
        .collect()
}

fn list(opts: &Opts) -> Result<ExitCode> {
    let cases = select(opts)?;
    for c in &cases {
        let golden = if c.load_meta()?.is_some() {
            ""
        } else {
            "  [no expected]"
        };
        println!("{:<50} {}{}", c.id, c.spec.tags.join(" "), golden);
    }
    eprintln!("{} cases", cases.len());
    Ok(ExitCode::SUCCESS)
}

/// Normalised outputs of a run, for determinism checks.
fn fingerprint(case: &Case, run: &RunOutput) -> (Option<i32>, Vec<Option<String>>, Vec<String>) {
    let outs = run
        .outputs
        .iter()
        .map(|(n, b)| b.as_ref().map(|b| normalize_output(case, n, b)))
        .collect();
    (run.exit_code, outs, run.messages.clone())
}

fn golden(opts: &Opts) -> Result<ExitCode> {
    let oracle = runner(&opts.oracle, false)?;
    let version = version_line(&oracle);
    let cases = select(opts)?;
    let root = corpus_root();
    eprintln!("{} cases, oracle {} ({version})", cases.len(), oracle.name);
    let results = par_map(&cases, opts.jobs, |case| -> Result<String> {
        if opts.check {
            let run = oracle.run(case, &root, false)?;
            return Ok(match bibers_testkit::compare(case, &run)? {
                Outcome::Pass => "ok".into(),
                Outcome::Fail(m) => format!(
                    "MISMATCH\n{}",
                    m.iter()
                        .map(|m| format!("  {m}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            });
        }
        let a = oracle.run(case, &root, false)?;
        let b = oracle.run(case, &root, false)?;
        if a.timed_out {
            bail!("oracle timed out");
        }
        let deterministic = fingerprint(case, &a) == fingerprint(case, &b);
        let exp = case.expected_dir();
        if exp.exists() {
            std::fs::remove_dir_all(&exp)?;
        }
        std::fs::create_dir_all(&exp)?;
        for (name, bytes) in &a.outputs {
            if let Some(bytes) = bytes {
                let p = exp.join(name);
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(p, bytes)?;
            }
        }
        std::fs::write(exp.join(MESSAGES_FILE), messages_text(&a.messages))?;
        let meta = ExpectedMeta {
            oracle: oracle.name.clone(),
            oracle_version: version.clone(),
            exit_code: a.exit_code.unwrap_or(-1),
            deterministic,
        };
        std::fs::write(exp.join(META_FILE), toml::to_string(&meta)?)?;
        Ok(if deterministic {
            format!(
                "ok (exit {}, {:.2}s)",
                meta.exit_code,
                a.elapsed.as_secs_f64()
            )
        } else {
            "NONDETERMINISTIC: two oracle runs differ after normalisation; investigate, \
             then set `compare = \"unordered-lines\"` or extend the normaliser"
                .into()
        })
    });
    let mut failed = 0;
    for (case, r) in cases.iter().zip(results) {
        match r {
            Ok(msg) => {
                if !msg.starts_with("ok") {
                    failed += 1;
                }
                println!("{:<50} {msg}", case.id);
            }
            Err(e) => {
                failed += 1;
                println!("{:<50} ERROR {e:#}", case.id);
            }
        }
    }
    eprintln!(
        "{} cases, {} problems (expectations in <case>/{EXPECTED_DIR}/)",
        cases.len(),
        failed
    );
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn corpus(opts: &Opts) -> Result<ExitCode> {
    let imp = runner(&opts.implementation, opts.release)?;
    let cases = select(opts)?;
    let root = corpus_root();
    let start = Instant::now();
    let results = par_map(&cases, opts.jobs, |case| -> Result<(Outcome, RunOutput)> {
        let run = imp.run(case, &root, opts.keep)?;
        let outcome = bibers_testkit::compare(case, &run)?;
        Ok((outcome, run))
    });

    let mut passing = Vec::new();
    let (mut pass, mut fail, mut err) = (0, 0, 0);
    let mut total_time = 0.0;
    for (case, r) in cases.iter().zip(results) {
        match r {
            Ok((outcome, run)) => {
                total_time += run.elapsed.as_secs_f64();
                match &outcome {
                    Outcome::Pass => {
                        pass += 1;
                        passing.push(case.id.clone());
                        println!("pass  {:<50} {:7.3}s", case.id, run.elapsed.as_secs_f64());
                        if let Some(w) = &run.workdir {
                            std::fs::remove_dir_all(w).ok();
                        }
                    }
                    Outcome::Fail(ms) => {
                        fail += 1;
                        println!("FAIL  {:<50} {:7.3}s", case.id, run.elapsed.as_secs_f64());
                        if let Some(w) = &run.workdir {
                            println!("      work dir: {}", w.display());
                        }
                        if opts.verbose {
                            for m in ms {
                                println!("      {m}");
                            }
                            if !run.stderr.is_empty() {
                                println!("      stderr: {}", run.stderr.trim());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                err += 1;
                println!("ERROR {:<50} {e:#}", case.id);
            }
        }
    }
    eprintln!(
        "{}: {pass} passed, {fail} failed, {err} errors of {} cases \
         (sum of run times {total_time:.1}s, wall {:.1}s)",
        imp.name,
        cases.len(),
        start.elapsed().as_secs_f64()
    );

    if opts.update_passing {
        if imp.name != "bibers" {
            bail!("--update-passing only makes sense with --impl bibers");
        }
        if !opts.filters.is_empty() {
            bail!("--update-passing requires running the whole corpus (no filters)");
        }
        let path = passing_file();
        let mut text = String::from(PASSING_HEADER);
        for id in &passing {
            text.push_str(id);
            text.push('\n');
        }
        std::fs::write(&path, text)?;
        eprintln!("wrote {} ({} cases)", path.display(), passing.len());
    }
    Ok(if fail + err == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

const PASSING_HEADER: &str = "\
# Corpus cases bibers is required to pass (the ratchet). Checked by
# `cargo test -p bibers --test corpus`. Regenerate with
# `cargo xtask corpus --update-passing`; never remove a line to make CI green.
";

fn passing_file() -> PathBuf {
    corpus_root().join("passing.txt")
}

//! `cargo xtask fuzz`: differential fuzzing of bibers against biber (oracle
//! O3), with a delta-debugging minimiser (`cargo xtask fuzz-min`) that turns
//! a failure into a corpus case (docs/PLAN.md §6.2 item 4).

mod bib;
mod exec;
mod generate;
mod minimise;
mod rng;

use anyhow::{Context, Result, bail};
use bib::FuzzInput;
use exec::{BIB_NAME, Template, Verdict, check, signature_id};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Instant;

pub const HELP: &str = "\
  fuzz                 Differential fuzzing: random .bib files × .bcf templates, run on
                       the oracle and on bibers (release), compared as in `corpus`.
      --seed N           Seed (default 1). Input i of seed N is always the same file.
      --count N          Number of generated inputs (default 100).
      --start N          Index of the first input (default 0).
      --templates A,B    Template corpus cases (default: a built-in list; `all-default`;
                         `prefix:realdocs/` = every usable case under a prefix).
      --per-input K      Templates per input, chosen at random (default 4; 0 = all).
      --oracle o3|o1     Oracle (default o3; parallel PAR biber (o1) fails spuriously).
      --examples K       Failing examples kept per signature (default 3).
      --out DIR          Output directory (default target/fuzz/seed-N).
      --print            Only print the generated .bib files.
  fuzz-min DIR         Minimise a failing example saved by `fuzz` (DIR holds input.json)
                       and write it as tests/corpus/fuzz/NAME (then run `cargo xtask golden`).
      --name NAME        Corpus case name (required unless --dry-run).
      --shape SHAPE      Diff shape to preserve (default: the most specific one).
      --dry-run          Print the minimised .bib only.
";

/// Default templates: standard styles, contributed styles with their own
/// sourcemaps/datamodels, and option-matrix variants.
pub const DEFAULT_TEMPLATES: &[&str] = &[
    "styles/authoryear",
    "styles/alphabetic",
    "styles/numeric-comp",
    "styles/authortitle",
    "styles/apa",
    "styles/chicago-authordate",
    "styles/authoryear--uniquename-mininit",
    "styles/authoryear--uniquename-allfull",
    "styles/authoryear--uniquelist-minyear",
    "styles/authoryear--sortlocale-de_DE",
    "styles/alphabetic--maxnames-1",
    "styles/alphabetic--uniquename-full",
    "styles/numeric--sorting-ydnt",
    "styles/authoryear--useprefix-true",
    "styles/authoryear--giveninits-true",
    "styles/authoryear--labeldateparts-false",
];

struct FuzzOpts {
    seed: u64,
    count: u64,
    start: u64,
    templates: Vec<String>,
    per_input: usize,
    oracle: String,
    examples: usize,
    out: Option<PathBuf>,
    print: bool,
    jobs: usize,
    // fuzz-min
    dir: Option<PathBuf>,
    name: Option<String>,
    shape: Option<String>,
    dry_run: bool,
}

fn parse(args: &[String]) -> Result<FuzzOpts> {
    let mut o = FuzzOpts {
        seed: 1,
        count: 100,
        start: 0,
        templates: DEFAULT_TEMPLATES.iter().map(|s| s.to_string()).collect(),
        per_input: 4,
        oracle: "o3".into(),
        examples: 3,
        out: None,
        print: false,
        jobs: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(8),
        dir: None,
        name: None,
        shape: None,
        dry_run: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--seed" => o.seed = val()?.parse()?,
            "--count" => o.count = val()?.parse()?,
            "--start" => o.start = val()?.parse()?,
            "--templates" => {
                let v = val()?;
                if v != "all-default" {
                    o.templates = v.split(',').map(str::to_string).collect();
                }
            }
            "--per-input" => o.per_input = val()?.parse()?,
            "--oracle" => o.oracle = val()?.clone(),
            "--examples" => o.examples = val()?.parse()?,
            "--out" => o.out = Some(PathBuf::from(val()?)),
            "--print" => o.print = true,
            "-j" => o.jobs = val()?.parse()?,
            "--name" => o.name = Some(val()?.clone()),
            "--shape" => o.shape = Some(val()?.clone()),
            "--dry-run" => o.dry_run = true,
            s if s.starts_with('-') => bail!("unknown option {s}"),
            s => o.dir = Some(PathBuf::from(s)),
        }
    }
    Ok(o)
}

/// Templates for input `index`: `k` distinct ones, chosen deterministically.
fn templates_for(o: &FuzzOpts, index: u64, n: usize) -> Vec<usize> {
    let mut all: Vec<usize> = (0..n).collect();
    if o.per_input == 0 || o.per_input >= n {
        return all;
    }
    let mut r = rng::Rng::for_case(o.seed ^ 0x7465_6d70_6c61_7465, index);
    r.shuffle(&mut all);
    all.truncate(o.per_input);
    all.sort_unstable();
    all
}

#[derive(Default)]
struct SigStats {
    shapes: BTreeSet<String>,
    count: usize,
    inputs: BTreeSet<u64>,
    templates: BTreeMap<String, usize>,
    examples: Vec<PathBuf>,
}

pub fn fuzz(args: &[String]) -> Result<ExitCode> {
    let o = parse(args)?;
    if o.print {
        for i in o.start..o.start + o.count {
            let input = generate::generate(o.seed, i);
            println!(
                "%%%% seed {} input {i}: args {:?}, options {:?}, nocite* {}, cites {:?}",
                o.seed,
                input.args,
                input.opts,
                input.nocite_all,
                input
                    .cites
                    .iter()
                    .map(|c| c.key.as_str())
                    .collect::<Vec<_>>()
            );
            print!("{}", input.render_bib());
        }
        return Ok(ExitCode::SUCCESS);
    }
    let mut templates: Vec<Template> = Vec::new();
    for t in &o.templates {
        match t.strip_prefix("prefix:") {
            // Every usable case (one section, one data source) under a prefix.
            Some(prefix) => {
                for c in bibers_testkit::discover(&bibers_testkit::corpus_root())? {
                    if c.id.starts_with(prefix) {
                        match Template::load(&c.id) {
                            Ok(tpl) => templates.push(tpl),
                            Err(e) => eprintln!("skipping template {}: {e:#}", c.id),
                        }
                    }
                }
            }
            None => templates.push(Template::load(t)?),
        }
    }
    if templates.is_empty() {
        bail!("no templates");
    }
    let oracle = crate::runner(&o.oracle, false)?;
    let bibers = crate::runner("bibers", true)?;
    let out = o.out.clone().unwrap_or_else(|| {
        bibers_testkit::repo_root()
            .join("target/fuzz")
            .join(format!("seed-{}", o.seed))
    });
    std::fs::create_dir_all(&out)?;

    let mut tasks: Vec<(u64, usize)> = Vec::new();
    for i in o.start..o.start + o.count {
        for t in templates_for(&o, i, templates.len()) {
            tasks.push((i, t));
        }
    }
    eprintln!(
        "fuzz: seed {}, inputs {}..{}, {} runs over {} templates, oracle {}, {} jobs; output {}",
        o.seed,
        o.start,
        o.start + o.count,
        tasks.len(),
        templates.len(),
        oracle.name,
        o.jobs,
        out.display()
    );

    let start = Instant::now();
    let next = Mutex::new(0usize);
    let stats: Mutex<BTreeMap<String, SigStats>> = Mutex::new(BTreeMap::new());
    let done = Mutex::new((0usize, 0usize, 0usize)); // runs, mismatches, errors
    std::thread::scope(|s| {
        for _ in 0..o.jobs.max(1) {
            s.spawn(|| {
                loop {
                    let k = {
                        let mut n = next.lock().unwrap();
                        let k = *n;
                        *n += 1;
                        k
                    };
                    let Some(&(index, ti)) = tasks.get(k) else {
                        break;
                    };
                    let tpl = &templates[ti];
                    let input = generate::generate(o.seed, index);
                    let res = check(&input, tpl, &oracle, &bibers, true);
                    let mut d = done.lock().unwrap();
                    d.0 += 1;
                    match res {
                        Ok(v) if v.shapes.is_empty() => {}
                        Ok(v) => {
                            d.1 += 1;
                            let id = signature_id(&v.shapes);
                            let mut st = stats.lock().unwrap();
                            let e = st.entry(id.clone()).or_default();
                            e.count += 1;
                            e.inputs.insert(index);
                            *e.templates.entry(tpl.id.clone()).or_default() += 1;
                            if e.shapes.is_empty() {
                                e.shapes = v.shapes.clone();
                            }
                            if e.examples.len() < o.examples {
                                let dir = out.join("fail").join(&id).join(format!(
                                    "{index}-{}",
                                    tpl.id.replace('/', "_")
                                ));
                                match save_example(&dir, &o, index, tpl, &input, &v) {
                                    Ok(()) => e.examples.push(dir),
                                    Err(err) => eprintln!("saving example: {err:#}"),
                                }
                            }
                        }
                        Err(err) => {
                            d.2 += 1;
                            eprintln!("error on input {index} / {}: {err:#}", tpl.id);
                        }
                    }
                    if d.0 % 50 == 0 || d.0 == tasks.len() {
                        let el = start.elapsed().as_secs_f64();
                        eprintln!(
                            "  {}/{} runs, {} mismatches ({} signatures), {} errors, {:.0}s elapsed, ~{:.0}s left",
                            d.0,
                            tasks.len(),
                            d.1,
                            stats.lock().unwrap().len(),
                            d.2,
                            el,
                            el / d.0 as f64 * (tasks.len() - d.0) as f64
                        );
                    }
                }
            });
        }
    });

    let stats = stats.into_inner().unwrap();
    let (runs, mismatches, errors) = done.into_inner().unwrap();
    let mut report = format!(
        "seed {} inputs {}..{}: {runs} runs, {mismatches} mismatches, {errors} errors, \
         {} signatures, {:.0}s\n\n",
        o.seed,
        o.start,
        o.start + o.count,
        stats.len(),
        start.elapsed().as_secs_f64()
    );
    let mut sigs: Vec<(&String, &SigStats)> = stats.iter().collect();
    sigs.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
    for (id, st) in sigs {
        report.push_str(&format!(
            "== {id}: {} runs, {} inputs; templates: {}\n",
            st.count,
            st.inputs.len(),
            st.templates
                .iter()
                .map(|(t, n)| format!("{}×{n}", t.trim_start_matches("styles/")))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        for (i, sh) in st.shapes.iter().enumerate() {
            if i == 12 {
                report.push_str(&format!("   … {} more shapes\n", st.shapes.len() - 12));
                break;
            }
            report.push_str(&format!("   {sh}\n"));
        }
        for ex in &st.examples {
            report.push_str(&format!("   example: {}\n", ex.display()));
        }
    }
    std::fs::write(out.join("report.txt"), &report)?;
    print!("{report}");
    Ok(if mismatches + errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn save_example(
    dir: &Path,
    o: &FuzzOpts,
    index: u64,
    tpl: &Template,
    input: &FuzzInput,
    v: &Verdict,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("input.json"), serde_json::to_string_pretty(input)?)?;
    std::fs::write(dir.join(BIB_NAME), input.render_bib())?;
    std::fs::write(dir.join(&tpl.bcf_name), tpl.bcf(input))?;
    std::fs::write(
        dir.join("meta.txt"),
        format!(
            "seed = {}\nindex = {index}\ntemplate = {}\n",
            o.seed, tpl.id
        ),
    )?;
    std::fs::write(
        dir.join("shapes.txt"),
        v.shapes
            .iter()
            .map(|s| format!("{s}\n"))
            .collect::<String>(),
    )?;
    std::fs::write(dir.join("detail.txt"), &v.detail)?;
    for (who, run) in [("oracle", &v.oracle), ("bibers", &v.bibers)] {
        for (name, bytes) in &run.outputs {
            if let Some(b) = bytes {
                std::fs::write(dir.join(format!("{who}-{name}")), b)?;
            }
        }
        std::fs::write(
            dir.join(format!("{who}-messages.txt")),
            run.messages.join("\n"),
        )?;
        std::fs::write(dir.join(format!("{who}-stderr.txt")), &run.stderr)?;
    }
    Ok(())
}

/// Pick the shape the minimiser preserves: a panic, timeout or exit-code
/// difference first, then `.bbl` lines, then messages.
fn primary_shape(shapes: &BTreeSet<String>) -> Option<String> {
    let rank = |s: &str| -> u8 {
        if s.starts_with("panic:") {
            0
        } else if s == "bibers-timeout" {
            1
        } else if s.starts_with("exit:") || s.starts_with("o3-crash") {
            2
        } else if s.starts_with('-') || s.starts_with('+') {
            if s.contains("other:") || s.contains("close") {
                4
            } else {
                3
            }
        } else if s.starts_with("missing-output") || s.starts_with("unexpected-output") {
            2
        } else {
            5
        }
    };
    shapes.iter().min_by_key(|s| rank(s)).cloned()
}

pub fn fuzz_min(args: &[String]) -> Result<ExitCode> {
    let o = parse(args)?;
    let dir = o
        .dir
        .clone()
        .context("fuzz-min needs the example directory")?;
    let input: FuzzInput = serde_json::from_str(
        &std::fs::read_to_string(dir.join("input.json")).context("reading input.json")?,
    )?;
    let meta = std::fs::read_to_string(dir.join("meta.txt"))?;
    let field = |k: &str| -> Result<String> {
        meta.lines()
            .find_map(|l| l.strip_prefix(&format!("{k} = ")))
            .map(str::to_string)
            .with_context(|| format!("meta.txt: no {k}"))
    };
    let tpl = Template::load(&field("template")?)?;
    let oracle = crate::runner(&o.oracle, false)?;
    let bibers = crate::runner("bibers", true)?;

    let v = check(&input, &tpl, &oracle, &bibers, true)?;
    if v.shapes.is_empty() {
        bail!("the example no longer fails");
    }
    let shape = match &o.shape {
        Some(s) => s.clone(),
        None => primary_shape(&v.shapes).expect("non-empty"),
    };
    if !v.shapes.contains(&shape) {
        bail!(
            "shape {shape:?} not in the current signature: {:?}",
            v.shapes
        );
    }
    eprintln!(
        "minimising {} ({} entries) keeping shape {shape:?}",
        dir.display(),
        input.entry_count()
    );
    let m = minimise::Minimiser::new(&tpl, &oracle, &bibers, shape.clone(), o.jobs);
    let start = Instant::now();
    let min = m.run(input, &|s| eprintln!("  {s}"));
    let bib = min.render_bib();
    eprintln!(
        "done: {} tests, {:.0}s, {} entries, {} bytes",
        m.tests.lock().unwrap(),
        start.elapsed().as_secs_f64(),
        min.entry_count(),
        bib.len()
    );
    println!("{bib}");
    println!(
        "% citations: {}{}",
        min.cites
            .iter()
            .map(|c| if c.nocite {
                format!("\\nocite{{{}}}", c.key)
            } else {
                format!("\\cite{{{}}}", c.key)
            })
            .collect::<Vec<_>>()
            .join(" "),
        if min.nocite_all { " \\nocite{*}" } else { "" }
    );
    if !min.args.is_empty() {
        println!("% command-line options: {:?}", min.args);
    }
    if !min.opts.is_empty() {
        println!("% option overrides: {:?}", min.opts);
    }
    let v = check(&min, &tpl, &oracle, &bibers, true)?;
    println!("{}", v.detail);
    if o.dry_run {
        return Ok(ExitCode::SUCCESS);
    }
    let name = o.name.clone().context("--name is required")?;
    let case_dir = bibers_testkit::corpus_root().join("fuzz").join(&name);
    if case_dir.exists() {
        bail!("{} exists", case_dir.display());
    }
    std::fs::create_dir_all(&case_dir)?;
    std::fs::write(case_dir.join(BIB_NAME), &bib)?;
    std::fs::write(case_dir.join(&tpl.bcf_name), tpl.bcf(&min))?;
    let q = |s: &str| toml::Value::String(s.to_string()).to_string();
    let spec = tpl.spec(&min, String::new(), String::new());
    let text = format!(
        "description = {}\nsource = {}\nargs = [{}]\noutputs = [{}]\n\
         tags = [\"suite:fuzz\", {}]\n",
        q(&format!(
            "fuzz: minimised reproducer for diff shape {shape}"
        )),
        q(&format!(
            "cargo xtask fuzz (seed {}, input {}, template {}), minimised by cargo xtask fuzz-min",
            field("seed")?,
            field("index")?,
            tpl.id
        )),
        spec.args
            .iter()
            .map(|a| q(a))
            .collect::<Vec<_>>()
            .join(", "),
        q(&tpl.bbl_name()),
        q(&format!("template:{}", tpl.id)),
    );
    std::fs::write(case_dir.join("case.toml"), text)?;
    eprintln!(
        "wrote {}; now run `cargo xtask golden fuzz/{name}`",
        case_dir.display()
    );
    Ok(ExitCode::SUCCESS)
}

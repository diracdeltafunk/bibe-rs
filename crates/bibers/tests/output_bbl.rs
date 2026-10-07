//! The `.bbl` writer in isolation from the pipeline: for each corpus case,
//! take biber's final section states (the last `process_lists` dump of each
//! section, oracle O3), render them with `bibers_core::output::bbl::render`
//! and compare with the case's expected `.bbl` (corpus normalisation).
//!
//! Cases listed in `tests/data/output_bbl_passing.txt` must render
//! byte-identically (a ratchet). Skips when O3 isn't built.
//!
//! Environment:
//! - `BIBERS_OUTPUT_CASES=<prefix>[,<prefix>…]` restricts the cases;
//! - `BIBERS_OUTPUT_VERBOSE=1` prints the diff of every failing case;
//! - `BIBERS_OUTPUT_UPDATE=1` rewrites the ratchet file with the passing cases.

mod common;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use bibers_core::model::SectionState;
use bibers_core::model::dump::load_state;
use bibers_core::pipeline::{self, Ctx, Processed};
use bibers_testkit::compare::{diff, normalize_output};
use bibers_testkit::phase::{o3_available, phase_dumps};
use bibers_testkit::{Case, corpus_root, discover};

const RATCHET: &str = "tests/data/output_bbl_passing.txt";

#[derive(Debug)]
enum Outcome {
    Pass,
    Fail(String),
    /// Not applicable or deferred (reason).
    Skip(String),
}

fn deferred(case: &Case) -> Option<&'static str> {
    let args = &case.spec.args;
    if args.iter().any(|a| a == "--tool") {
        return Some("tool mode");
    }
    if args.iter().any(|a| a.starts_with("--output-format")) {
        return Some("non-bbl output format");
    }
    None
}

/// Render biber's final section states for `case` with bibers' writer
/// (setup with the case's arguments, `extra_args` first). Returns the
/// output bytes and the output encoding name.
fn render_final_states(case: &Case, extra_args: &[&str]) -> Result<(Vec<u8>, String), String> {
    let dumps = match phase_dumps(case, &corpus_root()) {
        Ok(d) => d,
        Err(e) => return Err(format!("phase dumps: {e}")),
    };
    // The final state of each section, in processing order.
    let mut last: BTreeMap<String, usize> = BTreeMap::new();
    for (i, d) in dumps.iter().enumerate() {
        if d.phase == "process_lists" {
            last.insert(d.section.clone(), i);
        }
    }
    let mut idx: Vec<usize> = last.into_values().collect();
    idx.sort_unstable();
    let mut states: Vec<SectionState> = Vec::new();
    for i in idx {
        let v = match dumps[i].state() {
            Ok(v) => v,
            Err(e) => return Err(format!("reading dump: {e}")),
        };
        match load_state(&v) {
            Ok(st) => states.push(st),
            Err(e) => return Err(format!("load_state: {e}")),
        }
    }

    let mut setup_for = case.clone();
    for (i, a) in extra_args.iter().enumerate() {
        setup_for.spec.args.insert(i, a.to_string());
    }
    let (_work, env, s) = match common::phase::setup_case(&setup_for) {
        Ok(x) => x,
        Err(code) => return Err(format!("bibers setup exited with {code}")),
    };
    let bibers::run::Setup {
        mut cfg,
        doc,
        mut log,
        ..
    } = s;
    let dm = cfg.dm.take().expect("data model");
    for st in &states {
        pipeline::install_entry_options(&mut cfg, st);
    }
    let mut ctx = Ctx {
        cfg: &mut cfg,
        dm: &dm,
        doc: &doc,
        log: &mut log,
        env: &env,
        run: Default::default(),
    };
    let processed = Processed { sections: states };
    let text = match bibers_core::output::bbl::render(&mut ctx, &processed) {
        Ok(Some(t)) => t,
        Ok(None) => return Err("render returned None".into()),
        Err(_) => return Err("render: fatal error".into()),
    };
    let bytes = match bibers_core::output::bbl::encode_output(&mut ctx, &text) {
        Ok(b) => b,
        Err(_) => return Err("encode_output: fatal error".into()),
    };
    let enc_name = bibers_core::output::bbl::output_encoding_name(&ctx);
    Ok((bytes, enc_name))
}

fn run_case(case: &Case) -> Outcome {
    if let Some(r) = deferred(case) {
        return Outcome::Skip(r.into());
    }
    let Some(bbl) = case
        .spec
        .outputs
        .iter()
        .find(|o| o.ends_with(".bbl") || *o == "-")
    else {
        return Outcome::Skip("no .bbl output".into());
    };
    let Ok(expected) = std::fs::read(case.expected_dir().join(bbl)) else {
        return Outcome::Skip("no expected .bbl".into());
    };
    if expected.is_empty() {
        // biber stopped before writing any output.
        return Outcome::Skip("empty expected .bbl".into());
    }
    let (bytes, enc_name) = match render_final_states(case, &[]) {
        Ok(x) => x,
        Err(e) => return Outcome::Fail(e),
    };
    // Compare as text, decoded from the output encoding.
    let enc = perl_compat::encoding::find_encoding(&enc_name).expect("known encoding");
    let expected = enc.decode(&expected).into_bytes();
    let actual = enc.decode(&bytes).into_bytes();
    // `-` is the .bbl written to stdout.
    let name = if bbl == "-" {
        "stdout.bbl"
    } else {
        bbl.as_str()
    };
    let exp = normalize_output(case, name, &expected);
    let act = normalize_output(case, name, &actual);
    if exp == act {
        Outcome::Pass
    } else {
        Outcome::Fail(diff(&exp, &act))
    }
}

#[test]
fn bbl_writer_matches_biber() {
    if !o3_available() {
        eprintln!("skipping: oracle O3 not built");
        return;
    }
    let filter: Vec<String> = std::env::var("BIBERS_OUTPUT_CASES")
        .ok()
        .map(|s| s.split(',').map(str::to_string).collect())
        .unwrap_or_default();
    let cases: Vec<Case> = discover(&corpus_root())
        .expect("corpus loads")
        .into_iter()
        .filter(|c| filter.is_empty() || filter.iter().any(|p| c.id.starts_with(p.as_str())))
        .collect();
    let results: Mutex<Vec<(String, Outcome)>> = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(case) = cases.get(i) else { break };
                    let r = std::panic::catch_unwind(|| run_case(case))
                        .unwrap_or_else(|_| Outcome::Fail("panicked".into()));
                    results.lock().unwrap().push((case.id.clone(), r));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.0.cmp(&b.0));

    let verbose = std::env::var_os("BIBERS_OUTPUT_VERBOSE").is_some();
    let mut per_suite: BTreeMap<String, [usize; 3]> = BTreeMap::new();
    let mut passing = Vec::new();
    for (id, r) in &results {
        let suite = id.split('/').next().unwrap_or("").to_string();
        let c = per_suite.entry(suite).or_default();
        match r {
            Outcome::Pass => {
                c[0] += 1;
                passing.push(id.clone());
            }
            Outcome::Fail(d) => {
                c[1] += 1;
                if verbose {
                    eprintln!("FAIL {id}\n{d}");
                } else {
                    eprintln!("FAIL {id}");
                }
            }
            Outcome::Skip(why) => {
                c[2] += 1;
                if verbose {
                    eprintln!("SKIP {id}: {why}");
                }
            }
        }
    }
    for (suite, [p, f, s]) in &per_suite {
        eprintln!("{suite}: {p} pass, {f} fail, {s} skipped");
    }

    let ratchet_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(RATCHET);
    if std::env::var_os("BIBERS_OUTPUT_UPDATE").is_some() {
        let mut s = passing.join("\n");
        s.push('\n');
        std::fs::write(&ratchet_path, s).expect("write ratchet");
        return;
    }
    let ratchet = std::fs::read_to_string(&ratchet_path).unwrap_or_default();
    let status: BTreeMap<&str, &Outcome> = results.iter().map(|(i, r)| (i.as_str(), r)).collect();
    let regressions: Vec<&str> = ratchet
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|id| {
            matches!(
                status.get(id),
                Some(Outcome::Fail(_)) | Some(Outcome::Skip(_))
            )
        })
        .collect();
    assert!(
        regressions.is_empty(),
        "cases in {RATCHET} no longer render identically: {regressions:?}"
    );
}

/// The output-related quirk switches change what they should
/// (`--fix-quirks`, docs/compat.md §2).
#[test]
fn quirk_switches() {
    if !o3_available() {
        eprintln!("skipping: oracle O3 not built");
        return;
    }
    let text = |id: &str, extra: &[&str]| {
        let (b, _) = render_final_states(&common::phase::find_case(id), extra).expect("renders");
        String::from_utf8(b).unwrap()
    };
    // BB-015
    let on = text("bugs/bb-015-bbl-comma-cleanup", &[]);
    let off = text(
        "bugs/bb-015-bbl-comma-cleanup",
        &["--fix-quirks=bbl-comma-cleanup"],
    );
    assert!(on.contains("\\field{title}{Foo {bar}}"));
    assert!(off.contains("\\field{title}{Foo {bar,}}"));
    assert!(!off.contains(",}}%"), "structural commas are still cleaned");
    // BB-016
    let on = text("bugs/bb-016-rangelen-zero-digit", &[]);
    let off = text(
        "bugs/bb-016-rangelen-zero-digit",
        &["--fix-quirks=rangelen-zero-digit"],
    );
    assert_ne!(on, off);
    // BB-028: with the quirk fixed every entry gets its updatedate flags.
    let n = |s: &str| s.matches("\\true{updatedatecirca}").count();
    let on = text("bugs/bb-028-datefield-helper-mutation", &[]);
    let off = text(
        "bugs/bb-028-datefield-helper-mutation",
        &["--fix-quirks=datefield-helper-mutation"],
    );
    assert_eq!(n(&on), 1);
    assert!(n(&off) > 1);
}

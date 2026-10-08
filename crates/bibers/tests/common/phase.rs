//! Phase-level differential testing against biber's internal state.
//!
//! For a corpus case and a pipeline phase (named after the Perl method, e.g.
//! `fetch_data`, `uniqueness`, `sort_list`), [`check_phase`]:
//! 1. gets biber's state dumps for the case (oracle O3 with
//!    `BiberPhaseDump.pm`, cached under `target/phase-dumps/`);
//! 2. for every occurrence of the phase (every section, every data list):
//!    loads biber's state from just *before* the phase into the Rust model,
//!    runs the Rust phase, dumps the result, and diffs it against biber's
//!    state just *after* the phase.
//!
//! The configuration (options, data model, templates) comes from bibers' own
//! `.bcf` parsing (`bibers::run::setup`), which is verified identical to
//! biber's separately.
//!
//! Usage in a test file:
//! ```ignore
//! mod common;
//! #[test]
//! fn fetch_data_tdata() {
//!     common::phase::assert_phase("fetch_data", &["tdata/names", "tdata/dateformats"]);
//! }
//! ```
//! Tests skip (pass) when oracle O3 is not built, as in CI.

use bibers::run::{SetupResult, setup};
use bibers_core::env::Env;
use bibers_core::model::SectionState;
use bibers_core::model::dump::{dump_state, json_diff, load_state};
use bibers_core::pipeline::{self, Ctx, lists};
use bibers_testkit::phase::{PhaseDump, o3_available, phase_dumps};
use bibers_testkit::{Case, corpus_root, discover};

/// Paths ignored in every comparison: values that are nondeterministic in
/// biber itself.
pub const DEFAULT_IGNORE: &[&str] = &[
    // Name::rawstring joins name parts in Perl hash order (S2).
    "section.bibentries.*.datafields.*.namelist.*.rawstring",
    "section.bibentries.*.derivedfields.*.namelist.*.rawstring",
];

/// Maximum diff lines reported per dump.
const LIMIT: usize = 25;

/// A case of the main corpus.
pub fn find_case(id: &str) -> Case {
    find_case_at(&corpus_root(), id)
}

/// Run `bibers::run::setup` for `case` (of the corpus rooted at `root`) in a
/// fresh temp work dir holding its inputs (quietly). Returns the work dir
/// (keep it alive), its `Env` and the setup, or the setup's exit code.
pub fn setup_case_at(
    root: &std::path::Path,
    case: &Case,
) -> Result<(tempfile::TempDir, Env, bibers::run::Setup), u8> {
    let work = tempfile::tempdir().expect("tempdir");
    for f in case.input_files(root).expect("inputs") {
        std::fs::copy(&f, work.path().join(f.file_name().unwrap())).expect("copy input");
    }
    let env = Env::isolated(work.path());
    let mut args = case.spec.args.clone();
    args.splice(0..0, ["-q".to_string(), "-q".to_string()]);
    match setup(&args, &env) {
        SetupResult::Ready(s) => Ok((work, env, *s)),
        SetupResult::Exit(code) => Err(code),
    }
}

/// [`setup_case_at`] on the main corpus.
pub fn setup_case(case: &Case) -> Result<(tempfile::TempDir, Env, bibers::run::Setup), u8> {
    setup_case_at(&corpus_root(), case)
}

/// A case of the corpus rooted at `root`.
pub fn find_case_at(root: &std::path::Path, id: &str) -> Case {
    let cases = discover(root).expect("corpus loads");
    cases
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no corpus case {id}"))
}

/// `[^A-Za-z0-9_.-]+` → `_`, as the Perl dumper names list dump files.
fn sanitize(name: &str) -> String {
    let mut out = String::new();
    let mut in_run = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    out
}

/// Which dump holds the state a phase starts from.
fn before_dump<'a>(dumps: &'a [PhaseDump], target: &PhaseDump) -> Option<&'a PhaseDump> {
    let same_section = |d: &&PhaseDump| d.section == target.section && d.seq < target.seq;
    match target.phase.as_str() {
        "init" => None,
        // process_lists starts from the state after process_entries_static.
        "process_lists" => dumps
            .iter()
            .filter(same_section)
            .rfind(|d| d.phase == "process_entries_static"),
        _ => dumps.iter().rfind(same_section),
    }
}

/// Run one phase occurrence and diff with biber. Returns diff lines.
fn check_one(
    root: &std::path::Path,
    case: &Case,
    dumps: &[PhaseDump],
    target: &PhaseDump,
    ignore: &[&str],
    norm: Option<Normalise>,
) -> Vec<String> {
    let work = tempfile::tempdir().expect("tempdir");
    for f in case.input_files(root).expect("inputs") {
        std::fs::copy(&f, work.path().join(f.file_name().unwrap())).expect("copy input");
    }
    let env = Env::isolated(work.path());
    let mut args = case.spec.args.clone();
    args.splice(0..0, ["-q".to_string(), "-q".to_string()]);
    let s = match setup(&args, &env) {
        SetupResult::Ready(s) => *s,
        SetupResult::Exit(code) => return vec![format!("bibers setup exited with {code}")],
    };
    let bibers::run::Setup {
        mut cfg,
        doc,
        mut log,
        ..
    } = s;
    let dm = cfg.dm.take().expect("data model");
    let mut ctx = Ctx {
        cfg: &mut cfg,
        dm: &dm,
        doc: &doc,
        log: &mut log,
        env: &env,
        run: Default::default(),
    };

    let expected = target.state().expect("load expected dump");
    let mut st: SectionState = if target.phase == "init" {
        match pipeline::fetch::init_section(&mut ctx, &target.section) {
            Ok(mut st) => {
                // Carrying the (global) preamble across sections is
                // `prepare`'s job, not `init_section`'s.
                st.preamble = load_state(&expected).ok().and_then(|e| e.preamble);
                st
            }
            Err(_) => return vec!["init_section failed".into()],
        }
    } else {
        let Some(before) = before_dump(dumps, target) else {
            return vec![format!("no dump before {}", target.path.display())];
        };
        // Run-level state (the btparse macro table, the data file cache,
        // annotations) persists across sections: rebuild it by replaying the
        // earlier sections' fetch_data.
        if target.phase == "fetch_data" {
            for d in dumps
                .iter()
                .filter(|d| d.phase == "fetch_data" && d.seq < target.seq)
            {
                if let Some(b) = before_dump(dumps, d)
                    && let Ok(mut s) = load_state(&b.state().expect("load before dump"))
                {
                    pipeline::install_entry_options(ctx.cfg, &s);
                    let _ = pipeline::fetch::fetch_data(&mut ctx, &mut s);
                }
            }
        }
        let mut st = load_state(&before.state().expect("load before dump")).expect("load_state");
        pipeline::install_entry_options(ctx.cfg, &st);
        pipeline::install_annotations(&mut ctx, &st);
        let res = if let Some(f) = pipeline::section_phase(&target.phase) {
            f(&mut ctx, &mut st)
        } else if let Some(f) = pipeline::list_phase(&target.phase) {
            let want = target.list.clone().unwrap_or_default();
            // Lists can share a name (BB-054): the n-th dump of this phase
            // for the name belongs to the n-th such list.
            let nth = dumps
                .iter()
                .filter(|d| {
                    d.section == target.section
                        && d.phase == target.phase
                        && d.list == target.list
                        && d.seq < target.seq
                })
                .count();
            let Some(i) = st
                .datalists
                .iter()
                .enumerate()
                .filter(|(_, d)| sanitize(&d.name) == want)
                .map(|(i, _)| i)
                .nth(nth)
            else {
                return vec![format!("no data list {want} in state")];
            };
            // Reproduce what process_lists does around the phase.
            if target.phase == "process_entries_pre" {
                lists::init_list(&mut st, i);
            }
            let pre = if target.phase == "generate_sortdataschema" {
                lists::filter_list(&mut ctx, &mut st, i)
            } else {
                Ok(())
            };
            pre.and_then(|_| f(&mut ctx, &mut st, i))
        } else {
            panic!("unknown phase {}", target.phase);
        };
        if res.is_err() {
            return vec![format!("phase {} returned a fatal error", target.phase)];
        }
        st
    };
    pipeline::capture_entry_options(ctx.cfg, &mut st);
    let mut actual = dump_state(&st);
    let mut expected = expected;
    if let Some(n) = norm {
        n(&mut expected);
        n(&mut actual);
    }
    json_diff(&expected, &actual, ignore, LIMIT)
}

/// Check `phase` on each case; returns `(occurrences checked, failures)`,
/// where each failure is a dump file name followed by its diff lines.
pub fn check_phase(phase: &str, case_ids: &[&str], extra_ignore: &[&str]) -> (usize, Vec<String>) {
    check_phase_at(&corpus_root(), phase, case_ids, extra_ignore)
}

/// [`check_phase`] on cases of the corpus rooted at `root`.
pub fn check_phase_at(
    root: &std::path::Path,
    phase: &str,
    case_ids: &[&str],
    extra_ignore: &[&str],
) -> (usize, Vec<String>) {
    check_phase_norm_at(root, phase, case_ids, extra_ignore, None)
}

/// A normalisation applied to both biber's and bibers' state (the dump's
/// `state` object) before comparing, for values whose order is
/// nondeterministic in biber.
pub type Normalise = fn(&mut serde_json::Value);

/// [`check_phase_at`] with a normalisation.
pub fn check_phase_norm_at(
    root: &std::path::Path,
    phase: &str,
    case_ids: &[&str],
    extra_ignore: &[&str],
    norm: Option<Normalise>,
) -> (usize, Vec<String>) {
    let mut ignore: Vec<&str> = DEFAULT_IGNORE.to_vec();
    ignore.extend_from_slice(extra_ignore);
    let mut checked = 0;
    let mut failures = Vec::new();
    for id in case_ids {
        let case = find_case_at(root, id);
        let dumps =
            phase_dumps(&case, root).unwrap_or_else(|e| panic!("phase dumps for {id}: {e:#}"));
        for d in dumps.iter().filter(|d| d.phase == phase) {
            checked += 1;
            let diffs = check_one(root, &case, &dumps, d, &ignore, norm);
            if !diffs.is_empty() {
                failures.push(format!(
                    "{} ({}):\n  {}",
                    id,
                    d.path.file_name().unwrap().to_string_lossy(),
                    diffs.join("\n  ")
                ));
            }
        }
    }
    (checked, failures)
}

/// Assert that `phase` matches biber on every listed case (skips when the
/// O3 oracle is not built).
pub fn assert_phase(phase: &str, case_ids: &[&str]) {
    assert_phase_ignoring(phase, case_ids, &[]);
}

/// [`assert_phase`] with additional ignored paths.
pub fn assert_phase_ignoring(phase: &str, case_ids: &[&str], extra_ignore: &[&str]) {
    assert_phase_normalised(phase, case_ids, extra_ignore, None);
}

/// [`assert_phase_ignoring`] with a normalisation of both states.
pub fn assert_phase_normalised(
    phase: &str,
    case_ids: &[&str],
    extra_ignore: &[&str],
    norm: Option<Normalise>,
) {
    if !o3_available() {
        eprintln!("skipping {phase}: oracle O3 not built");
        return;
    }
    let (checked, failures) =
        check_phase_norm_at(&corpus_root(), phase, case_ids, extra_ignore, norm);
    assert!(checked > 0, "{phase}: no occurrences in the given cases");
    assert!(
        failures.is_empty(),
        "{phase}: {} of {checked} occurrences differ from biber:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// All corpus case ids whose id starts with one of `prefixes`.
pub fn cases_with_prefix(prefixes: &[&str]) -> Vec<String> {
    discover(&corpus_root())
        .expect("corpus loads")
        .into_iter()
        .map(|c| c.id)
        .filter(|id| prefixes.iter().any(|p| id.starts_with(p)))
        .collect()
}

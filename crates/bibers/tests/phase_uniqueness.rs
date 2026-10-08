//! Phase tests for the uniqueness package: `uniqueness` (the iterative
//! uniquename/uniquelist loop) against biber's phase dumps (oracle O3;
//! skipped when it is not built), plus DC-001 and a performance check.

mod common;

use common::phase::{cases_with_prefix, check_phase};

/// Cases (id prefixes) excluded from a suite, with the reason.
const SKIP: &[(&str, &str)] = &[
    ("ported/tool-", "tool mode is deferred (DC-030)"),
    (
        "ported/bibtex-output-",
        "bibtex output is deferred (DC-030)",
    ),
    ("ported/full-bibtex-", "bibtex output is deferred (DC-030)"),
];

fn run(prefixes: &[&str]) {
    if !bibers_testkit::phase::o3_available() {
        eprintln!("skipping uniqueness: oracle O3 not built");
        return;
    }
    let ids: Vec<String> = cases_with_prefix(prefixes)
        .into_iter()
        .filter(|c| !SKIP.iter().any(|(s, _)| c.starts_with(s)))
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (checked, failures) = check_phase("uniqueness", &refs, &[]);
    eprintln!(
        "uniqueness {prefixes:?}: {}/{checked} occurrences match biber",
        checked - failures.len()
    );
    assert!(
        failures.is_empty(),
        "uniqueness: {} of {checked} occurrences differ from biber:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn tdata() {
    run(&["tdata/"]);
}

#[test]
fn biblatex_examples() {
    run(&["biblatex-examples/"]);
}

#[test]
fn styles() {
    run(&["styles/"]);
}

#[test]
fn ported() {
    run(&["ported/"]);
}

#[test]
fn bugs() {
    run(&["bugs/"]);
}

#[test]
fn scale() {
    run(&["scale/"]);
}

/// DC-001 (BB-001): biber crashes in `uniqueness` when a labelname's
/// namestring is `final` (it collides with the `final` sub-hash of
/// `uniquelistcount`). bibers processes it normally: the result is what
/// biber gives for any other name (checked with O3 on the same data with
/// `final` renamed to `finale`).
#[test]
fn dc001_final_name_does_not_collide() {
    use bibers_core::model::dump::{dump_state, load_state};
    use bibers_core::pipeline::{self, Ctx};
    if !bibers_testkit::phase::o3_available() {
        eprintln!("skipping: oracle O3 not built");
        return;
    }
    let case = common::phase::find_case("bugs/bb-001-uniquelist-final-crash");
    let dumps = bibers_testkit::phase::phase_dumps(&case, &bibers_testkit::corpus_root())
        .expect("phase dumps");
    // biber dies inside uniqueness: the last dump is process_entries_pre.
    assert!(!dumps.iter().any(|d| d.phase == "uniqueness"));
    let pre = dumps
        .iter()
        .rfind(|d| d.phase == "process_entries_pre")
        .expect("process_entries_pre dump");
    let (_work, env, s) = common::phase::setup_case(&case).expect("setup");
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
    let mut st = load_state(&pre.state().unwrap()).unwrap();
    pipeline::install_entry_options(ctx.cfg, &st);
    pipeline::uniqueness::uniqueness(&mut ctx, &mut st, 0).expect("uniqueness");
    let out = dump_state(&st);
    let state = &out["datalists"][0]["state"];
    let nld = state["namelistdata"].as_object().unwrap();
    // `a` = {final}: no uniquelist; `b` = {final and Smith, John}: ul=2.
    let uls: Vec<Option<&str>> = nld.values().map(|l| l["ul"].as_str()).collect();
    assert_eq!(uls, vec![None, Some("2")], "{nld:#?}");
    for l in nld.values() {
        for (k, n) in l.as_object().unwrap() {
            if k != "ul" {
                assert_eq!(n["un"], serde_json::json!(["base", ["family"]]));
            }
        }
    }
    let ulc = &state["uniquelistcount"]["global"]["final"];
    assert_eq!(ulc["final"], "1");
    assert_eq!(ulc["final\u{10FFFD}Smith"], "1");
    assert_eq!(out["datalists"][0]["unulchanged"], "0");
}

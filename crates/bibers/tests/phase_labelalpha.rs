//! Phase tests for the labelalpha package, in isolation from the rest of
//! `process_entries_post` (owned by the entries package).
//!
//! For every `process_entries_post` occurrence in biber's phase dumps: load
//! the state before it, run `process_labelalpha` and `process_extraalpha` for
//! every section citekey in biber's order (Biber.pm:2056-2066), and compare
//! only the label paths with biber's state after it: the list's
//! `fields.*.{labelalpha,sortlabelalpha}` and `ladisambiguation`, and the
//! section's `labelcache_v`/`labelcache_l`. Skipped when oracle O3 is not
//! built.

mod common;

use bibers_core::model::dump::{dump_state, json_diff, load_state};
use bibers_core::pipeline::{self, Ctx, labelalpha};
use bibers_core::quirks::Quirk;
use bibers_testkit::phase::{PhaseDump, o3_available, phase_dumps};
use common::phase::{cases_with_prefix, find_case, setup_case};
use serde_json::{Map, Value as J};

/// Cases (id prefixes) excluded, with the reason.
const SKIP: &[(&str, &str)] = &[
    ("ported/tool-", "tool mode is deferred (DC-030)"),
    (
        "ported/bibtex-output-",
        "bibtex output is deferred (DC-030)",
    ),
    ("ported/full-bibtex-", "bibtex output is deferred (DC-030)"),
];

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

/// The labelalpha-related parts of a dumped state for the list named
/// `list` (the `nth` list of that name).
fn extract(state: &J, list: &str, nth: usize) -> J {
    let mut out = Map::new();
    let sec = &state["section"];
    for k in ["labelcache_v", "labelcache_l"] {
        out.insert(k.into(), sec.get(k).cloned().unwrap_or(J::Null));
    }
    let dl = state["datalists"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["name"].as_str().map(sanitize).as_deref() == Some(list))
        .nth(nth);
    if let Some(dl) = dl {
        let st = &dl["state"];
        let mut fields = Map::new();
        if let Some(fs) = st.get("fields").and_then(J::as_object) {
            for (key, f) in fs {
                let mut m = Map::new();
                for k in ["labelalpha", "sortlabelalpha"] {
                    if let Some(v) = f.get(k) {
                        m.insert(k.into(), v.clone());
                    }
                }
                if !m.is_empty() {
                    fields.insert(key.clone(), J::Object(m));
                }
            }
        }
        out.insert("fields".into(), J::Object(fields));
        out.insert(
            "ladisambiguation".into(),
            st.get("ladisambiguation").cloned().unwrap_or(J::Null),
        );
    }
    J::Object(out)
}

/// Run the label functions for the `process_entries_post` occurrence
/// `target` (with the quirks `fix` fixed). Returns the expected and actual
/// label paths.
fn run_one(
    id: &str,
    dumps: &[PhaseDump],
    target: &PhaseDump,
    fix: &[Quirk],
) -> Result<(J, J), String> {
    let case = find_case(id);
    let (_work, env, s) =
        setup_case(&case).map_err(|code| format!("bibers setup exited with {code}"))?;
    let bibers::run::Setup {
        mut cfg,
        doc,
        mut log,
        ..
    } = s;
    let dm = cfg.dm.take().expect("data model");
    for q in fix {
        cfg.quirks.fix(*q);
    }
    let mut ctx = Ctx {
        cfg: &mut cfg,
        dm: &dm,
        doc: &doc,
        log: &mut log,
        env: &env,
        run: Default::default(),
    };
    let Some(before) = dumps
        .iter()
        .rfind(|d| d.section == target.section && d.seq < target.seq)
    else {
        return Err("no dump before".into());
    };
    let mut st = load_state(&before.state().expect("before dump")).expect("load_state");
    pipeline::install_entry_options(ctx.cfg, &st);
    let want = target.list.clone().unwrap_or_default();
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
        return Err(format!("no data list {want} in state"));
    };
    let keys = st.section.citekeys.clone();
    for k in &keys {
        if labelalpha::process_labelalpha(&mut ctx, &mut st, i, k).is_err()
            || labelalpha::process_extraalpha(&mut ctx, &mut st, i, k).is_err()
        {
            return Err("fatal error".into());
        }
    }
    let expected = extract(&target.state().expect("expected dump"), &want, nth);
    let actual = extract(&dump_state(&st), &want, nth);
    Ok((expected, actual))
}

fn check_one(id: &str, dumps: &[PhaseDump], target: &PhaseDump) -> Vec<String> {
    match run_one(id, dumps, target, &[]) {
        Ok((e, a)) => json_diff(&e, &a, &[], 25),
        Err(e) => vec![e],
    }
}

fn run(prefixes: &[&str]) {
    if !o3_available() {
        eprintln!("skipping labelalpha: oracle O3 not built");
        return;
    }
    let ids: Vec<String> = cases_with_prefix(prefixes)
        .into_iter()
        .filter(|c| !SKIP.iter().any(|(s, _)| c.starts_with(s)))
        .collect();
    let mut checked = 0;
    let mut with_labels = 0;
    let mut failures = Vec::new();
    for id in &ids {
        let case = find_case(id);
        let root = bibers_testkit::corpus_root();
        let dumps =
            phase_dumps(&case, &root).unwrap_or_else(|e| panic!("phase dumps for {id}: {e:#}"));
        for d in dumps.iter().filter(|d| d.phase == "process_entries_post") {
            checked += 1;
            if d.state()
                .ok()
                .is_some_and(|s| s.to_string().contains("\"labelalpha\""))
            {
                with_labels += 1;
            }
            let diffs = check_one(id, &dumps, d);
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
    eprintln!(
        "labelalpha {prefixes:?}: {}/{checked} occurrences match biber ({with_labels} with labels)",
        checked - failures.len()
    );
    assert!(
        failures.is_empty(),
        "labelalpha: {} of {checked} occurrences differ from biber:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn tdata() {
    run(&["tdata/"]);
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
fn biblatex_examples() {
    run(&["biblatex-examples/"]);
}

#[test]
fn scale() {
    run(&["scale/"]);
}

/// Labels of the `process_entries_post` occurrence for data list `list` of
/// `id`, computed with the quirks `fix` fixed: citekey → labelalpha.
fn fixed_labels(id: &str, list: &str, fix: &[Quirk]) -> Option<Vec<(String, String)>> {
    if !o3_available() {
        return None;
    }
    let case = find_case(id);
    let dumps = phase_dumps(&case, &bibers_testkit::corpus_root()).expect("phase dumps");
    let d = dumps
        .iter()
        .find(|d| d.phase == "process_entries_post" && d.list.as_deref() == Some(list))
        .expect("occurrence");
    let (_, actual) = run_one(id, &dumps, d, fix).expect("run");
    Some(
        actual["fields"]
            .as_object()
            .expect("fields")
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v["labelalpha"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect(),
    )
}

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

/// BB-094 fixed: the second list builds its own `v` cache from given names.
#[test]
fn bb094_fixed() {
    let id = "bugs/bb-094-labelcache-field-key";
    let Some(l2) = fixed_labels(id, "L2", &[Quirk::LabelcacheFieldKey]) else {
        return;
    };
    assert_eq!(l2, pairs(&[("a", "Jo00"), ("b", "Jan01"), ("c", "Jac02")]));
    // reproduced (default): the family-name cache is reused, name parts are lost
    let l2 = fixed_labels(id, "L2", &[]).unwrap();
    assert_eq!(l2, pairs(&[("a", "00"), ("b", "01"), ("c", "02")]));
}

/// BB-096 fixed: a fixed width index of 0 applies (`C` for Cat and Cow).
#[test]
fn bb096_fixed() {
    let id = "bugs/bb-096-label-global-index-zero";
    let Some(l) = fixed_labels(id, "L1", &[Quirk::LabelGlobalIndexZero]) else {
        return;
    };
    assert_eq!(
        l,
        pairs(&[
            ("a", "A00"),
            ("b", "B00"),
            ("c", "C00"),
            ("d", "D00"),
            ("e", "C00")
        ])
    );
}

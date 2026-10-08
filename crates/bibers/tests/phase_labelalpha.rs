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

fn check_one(id: &str, dumps: &[PhaseDump], target: &PhaseDump) -> Vec<String> {
    let case = find_case(id);
    let (_work, env, s) = match setup_case(&case) {
        Ok(x) => x,
        Err(code) => return vec![format!("bibers setup exited with {code}")],
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
    let Some(before) = dumps
        .iter()
        .rfind(|d| d.section == target.section && d.seq < target.seq)
    else {
        return vec!["no dump before".into()];
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
        return vec![format!("no data list {want} in state")];
    };
    let keys = st.section.citekeys.clone();
    for k in &keys {
        if labelalpha::process_labelalpha(&mut ctx, &mut st, i, k).is_err()
            || labelalpha::process_extraalpha(&mut ctx, &mut st, i, k).is_err()
        {
            return vec!["fatal error".into()];
        }
    }
    let expected = extract(&target.state().expect("expected dump"), &want, nth);
    let actual = extract(&dump_state(&st), &want, nth);
    json_diff(&expected, &actual, &[], 25)
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

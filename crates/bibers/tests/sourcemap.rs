//! Differential tests of the source map engine
//! (`bibers_core::pipeline::sourcemap`) against biber.
//!
//! `tools/oracle/probes/sourcemap/SourcemapDump.pm` runs the source-built
//! biber (oracle O3) on a corpus case and records every bibtex
//! `create_entry` call: the Text::BibTeX entry before mapping, the section
//! state the map engine reads, and afterwards the mapped entry, the entries
//! created by maps, the section changes, the remaining wanted keys, and the
//! warnings and debug messages of the engine. For each record this test
//! rebuilds the inputs, runs `apply_maps_in` with bibers' own configuration
//! for the case (`bibers::run::setup`), and compares everything.
//!
//! Probe output is cached under `target/sourcemap-probe/<case>/`. The tests
//! skip (pass) when O3 is not built (CI).

use std::collections::BTreeSet;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use bibers::run::{SetupResult, setup};
use bibers_core::env::Env;
use bibers_core::log::{LogSettings, Logger};
use bibers_core::model::{Section, SectionState};
use bibers_core::pipeline::Ctx;
use bibers_core::pipeline::sourcemap::{MapRunState, RawEntry, apply_maps_in};
use bibers_testkit::phase::o3_available;
use bibers_testkit::{Case, corpus_root, discover, repo_root};
use serde_json::Value as J;

/// Run (or reuse) the probe for a case; returns the JSON records.
fn probe(case: &Case) -> Result<Vec<J>, String> {
    let root = corpus_root();
    let inputs = case.input_files(&root).map_err(|e| e.to_string())?;
    let mut h: u64 = 0xcbf29ce484222325;
    let mut feed = |b: &[u8]| {
        for x in b {
            h ^= *x as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    for f in &inputs {
        feed(f.file_name().unwrap().as_encoded_bytes());
        feed(&std::fs::read(f).map_err(|e| e.to_string())?);
    }
    feed(case.spec.args.join("\0").as_bytes());
    let probe_dir = repo_root().join("tools/oracle/probes/sourcemap");
    feed(&std::fs::read(probe_dir.join("SourcemapDump.pm")).map_err(|e| e.to_string())?);
    let fp = format!("{h:016x}");

    let dir = repo_root()
        .join("target/sourcemap-probe")
        .join(case.id.replace('/', "__"));
    let out = dir.join("out.jsonl");
    let stamp = dir.join("FINGERPRINT");
    if std::fs::read_to_string(&stamp).ok().as_deref() != Some(fp.as_str()) {
        let _ = std::fs::remove_dir_all(&dir);
        let work = dir.join("work");
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        for f in &inputs {
            std::fs::copy(f, work.join(f.file_name().unwrap())).map_err(|e| e.to_string())?;
        }
        let _ = Command::new(probe_dir.join("run.sh"))
            .arg(&out)
            .args(&case.spec.args)
            .current_dir(&work)
            .envs(bibers_testkit::run::BASE_ENV.iter().copied())
            .envs(&case.spec.env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&work);
        std::fs::write(&stamp, &fp).map_err(|e| e.to_string())?;
    }
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    text.lines()
        .map(|l| serde_json::from_str(l).map_err(|e| e.to_string()))
        .collect()
}

fn strs(v: &J) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|x| x.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn raw_entry(v: &J) -> RawEntry {
    RawEntry {
        entrytype: v["type"].as_str().unwrap_or("").to_string(),
        key: v["key"].as_str().unwrap_or("").to_string(),
        fields: v["fields"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|f| {
                        (
                            f[0].as_str().unwrap_or("").to_string(),
                            f[1].as_str().unwrap_or("").to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// (type, fields) of an entry, for comparison.
fn shape(e: &RawEntry) -> String {
    format!("@{}{:?}", e.entrytype, e.fields)
}

/// Records whose differences are documented (case id, citekey, reason):
/// they are reported but do not fail the test.
const ALLOWED: &[(&str, &str, &str)] = &[
    // Perl code in replacement strings is not executed.
    ("bugs/bb-033-smap-replace-perl-eval", "s1", "DC-023"),
];

/// Our DC-023 warning (biber executes the code silently).
fn is_dc023_warning(w: &str) -> bool {
    w.contains("contains Perl code")
}

/// Compare one case. Returns (records checked, failure descriptions).
fn check_case(case: &Case, records: &[J]) -> (usize, Vec<String>) {
    let mut fails = Vec::new();
    let work = tempfile::tempdir().expect("tempdir");
    for f in case.input_files(&corpus_root()).expect("inputs") {
        std::fs::copy(&f, work.path().join(f.file_name().unwrap())).expect("copy input");
    }
    let env = Env::isolated(work.path());
    let mut args = case.spec.args.clone();
    args.splice(0..0, ["-q".to_string(), "-q".to_string()]);
    let s = match setup(&args, &env) {
        SetupResult::Ready(s) => *s,
        SetupResult::Exit(code) => {
            // e.g. tool mode / bibtex output, not yet supported (DC-030)
            eprintln!(
                "{}: skipped ({} records): bibers setup exited with {code}",
                case.id,
                records.len()
            );
            return (0, fails);
        }
    };
    let bibers::run::Setup { mut cfg, doc, .. } = s;
    let dm = cfg.dm.take().expect("data model");
    let mut checked = 0;
    for (i, r) in records.iter().enumerate() {
        if let Some(d) = r.get("die") {
            fails.push(format!("record {i}: biber died: {d}"));
            continue;
        }
        let key = r["key"].as_str().unwrap_or("").to_string();
        let b = &r["before"];
        let a = &r["after"];
        let mut log = Logger::memory(LogSettings::new(false, true, 0, false, true));
        cfg.sourcemap_run = MapRunState {
            uniq_counter: b["uniq_counter"].as_str().map_or_else(
                || b["uniq_counter"].as_u64().unwrap_or(0),
                |s| s.parse().unwrap_or(0),
            ),
            mapuniqval: b["mapuniqval"].as_str().map(str::to_string),
        };
        let citekeys = strs(&b["citekeys"]);
        let mut st = SectionState {
            section: Section {
                number: r["section"].as_str().unwrap_or("0").to_string(),
                citekeys_h: citekeys.iter().cloned().collect(),
                orig_order_citekeys: citekeys.clone(),
                citekeys,
                allkeys: b["allkeys"].as_i64() == Some(1),
                allkeys_nocite: b["allkeys_nocite"].as_i64() == Some(1),
                cite_citekeys: strs(&b["cite"]).into_iter().collect(),
                nocite_citekeys: strs(&b["nocite"]).into_iter().collect(),
                ..Section::default()
            },
            ..SectionState::default()
        };
        let mut entry = raw_entry(&b["entry"]);
        entry.key = key.clone();
        let mut ctx = Ctx {
            cfg: &mut cfg,
            dm: &dm,
            doc: &doc,
            log: &mut log,
            env: &env,
        };
        let ds = r["datasource"].as_str().unwrap_or("");
        let out = match apply_maps_in(&mut ctx, &mut st, "bibtex", ds, &mut entry) {
            Ok(o) => o,
            Err(_) => {
                fails.push(format!("record {i} ({key}): fatal error"));
                continue;
            }
        };
        checked += 1;
        let mut diffs = Vec::new();
        let want_skip = a["skip"].as_i64() == Some(1);
        if out.skip != want_skip {
            diffs.push(format!("skip: got {} want {want_skip}", out.skip));
        }
        if !want_skip {
            let want = shape(&raw_entry(&a["entry"]));
            let got = shape(&entry);
            if want != got {
                diffs.push(format!("entry:\n      got  {got}\n      want {want}"));
            }
            let mut got_new: Vec<(String, String)> = out
                .new_entries
                .iter()
                .map(|e| (e.key.clone(), shape(e)))
                .collect();
            got_new.sort();
            let want_new: Vec<(String, String)> = a["new"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| {
                    (
                        n["citekey"].as_str().unwrap_or("").to_string(),
                        shape(&raw_entry(&n["entry"])),
                    )
                })
                .collect();
            if got_new != want_new {
                diffs.push(format!(
                    "new entries:\n      got  {got_new:?}\n      want {want_new:?}"
                ));
            }
        }
        if st.section.citekeys != strs(&a["citekeys"]) {
            diffs.push(format!(
                "citekeys: got {:?} want {:?}",
                st.section.citekeys,
                strs(&a["citekeys"])
            ));
        }
        let nocite: Vec<String> = st.section.nocite_citekeys.iter().cloned().collect();
        if nocite != strs(&a["nocite"]) {
            diffs.push(format!(
                "nocite: got {nocite:?} want {:?}",
                strs(&a["nocite"])
            ));
        }
        let created: BTreeSet<&String> = out.created_keys.iter().collect();
        let rkeys: Vec<String> = strs(&b["rkeys"])
            .into_iter()
            .filter(|k| !created.contains(k))
            .collect();
        if rkeys != strs(&a["rkeys"]) {
            diffs.push(format!("rkeys: got {rkeys:?} want {:?}", strs(&a["rkeys"])));
        }
        let warnings: Vec<String> = log
            .warnings()
            .iter()
            .filter(|w| !is_dc023_warning(w))
            .cloned()
            .collect();
        if warnings != strs(&a["warnings"]) {
            diffs.push(format!(
                "warnings:\n      got  {warnings:?}\n      want {:?}",
                strs(&a["warnings"])
            ));
        }
        let (_, file) = log.memory_lines();
        let debug: Vec<String> = file
            .iter()
            .filter_map(|l| l.split_once("> DEBUG - ").map(|(_, m)| m.to_string()))
            .collect();
        let want_debug = strs(&a["debug"]);
        if debug != want_debug {
            let n = debug
                .iter()
                .zip(&want_debug)
                .take_while(|(x, y)| x == y)
                .count();
            diffs.push(format!(
                "debug (first difference at message {n}):\n      got  {:?}\n      want {:?}",
                debug.get(n),
                want_debug.get(n)
            ));
        }
        if !diffs.is_empty() {
            if let Some((_, _, why)) = ALLOWED.iter().find(|(c, k, _)| *c == case.id && *k == key) {
                eprintln!(
                    "{}: record {i} ({key}) differs as documented ({why})",
                    case.id
                );
                continue;
            }
            fails.push(format!("record {i} ({key}):\n    {}", diffs.join("\n    ")));
        }
    }
    (checked, fails)
}

/// Probe and check every case whose id starts with one of `prefixes`.
fn check_suites(prefixes: &[&str]) {
    check_cases(&corpus_root(), prefixes);
}

/// Probe-only fixtures (no golden outputs), laid out like corpus cases.
fn fixture_root() -> std::path::PathBuf {
    repo_root().join("crates/bibers/tests/data/sourcemap")
}

fn check_cases(root: &std::path::Path, prefixes: &[&str]) {
    if !o3_available() {
        eprintln!("skipping: oracle O3 not built");
        return;
    }
    let cases: Vec<Case> = discover(root)
        .expect("corpus")
        .into_iter()
        .filter(|c| prefixes.iter().any(|p| c.id.starts_with(p)))
        .collect();
    let queue = Mutex::new(cases.iter());
    let results = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let Some(case) = queue.lock().unwrap().next() else {
                        break;
                    };
                    let res = match probe(case) {
                        Ok(recs) => check_case(case, &recs),
                        Err(e) => (0, vec![format!("probe failed: {e}")]),
                    };
                    results.lock().unwrap().push((case.id.clone(), res));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.0.cmp(&b.0));
    let total: usize = results.iter().map(|r| r.1.0).sum();
    let failing: Vec<String> = results
        .iter()
        .filter(|r| !r.1.1.is_empty())
        .map(|(id, (_, f))| {
            let shown: Vec<&String> = f.iter().take(3).collect();
            format!(
                "{id}: {} failures\n  {}",
                f.len(),
                shown
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            )
        })
        .collect();
    eprintln!(
        "sourcemap: {} cases, {total} create_entry calls checked, {} cases failing",
        results.len(),
        failing.len()
    );
    assert!(failing.is_empty(), "{}", failing.join("\n"));
}

#[test]
fn sourcemap_fixtures() {
    check_cases(&fixture_root(), &[""]);
}

#[test]
fn sourcemap_tdata_ported() {
    check_suites(&["tdata/", "ported/"]);
}

#[test]
fn sourcemap_bugs() {
    check_suites(&["bugs/"]);
}

#[test]
fn sourcemap_biblatex_examples() {
    check_suites(&["biblatex-examples/"]);
}

#[test]
fn sourcemap_styles() {
    check_suites(&["styles/"]);
}

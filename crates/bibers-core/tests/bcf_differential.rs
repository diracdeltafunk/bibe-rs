//! Differential test of the configuration front end against biber 2.22.
//!
//! `tests/data/bcf/cases.json` holds, for every corpus case with a `.bcf`,
//! biber's post-parse state (options per scope, option registry, datafield
//! sets, sections/citekeys/datalists, data model, warnings, log lines) as
//! dumped by `tools/oracle/probes/bibers-core/probe.pl` running oracle O3.
//! Large parts are stored once in `pool/`. Regenerate with
//! `python3 tools/oracle/probes/bibers-core/gen_vectors.py config`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bibers_core::bcf::{self, ParseArgs, Parsed};
use bibers_core::config::{self, CliOptions, CliValue};
use bibers_core::env::Env;
use bibers_core::log::{LogSettings, Logger};
use bibers_core::quirks::Quirks;
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/bcf")
}

fn unpool(v: &Value, pool: &Path) -> Value {
    match v {
        Value::Object(m) if m.len() == 1 && m.contains_key("$pool") => {
            let h = m["$pool"].as_str().unwrap();
            let s = std::fs::read_to_string(pool.join(format!("{h}.json"))).unwrap();
            serde_json::from_str(&s).unwrap()
        }
        other => other.clone(),
    }
}

fn diff(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    if out.len() > 8 {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => diff(&format!("{path}.{k}"), p, q, out),
                    (p, q) => out.push(format!(
                        "{path}.{k}: expected {} got {}",
                        short(p),
                        short(q)
                    )),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                diff(&format!("{path}[{i}]"), p, q, out);
            }
        }
        _ if a == b => {}
        _ => out.push(format!(
            "{path}: expected {} got {}",
            short(Some(a)),
            short(Some(b))
        )),
    }
}

fn short(v: Option<&Value>) -> String {
    match v {
        None => "<missing>".into(),
        Some(v) => {
            let s = v.to_string();
            if s.len() > 200 {
                format!("{}…", &s[..200])
            } else {
                s
            }
        }
    }
}

fn cli_opts(v: &Value) -> CliOptions {
    let mut o = CliOptions::new();
    if let Value::Object(m) = v {
        for (k, v) in m {
            let cv = match v {
                Value::String(s) => CliValue::Str(s.clone()),
                Value::Null => CliValue::Undef,
                Value::Object(h) => CliValue::Map(
                    h.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                        .collect::<BTreeMap<_, _>>(),
                ),
                other => panic!("unexpected option value {other}"),
            };
            o.insert(k.clone(), cv);
        }
    }
    o
}

/// Run the front end for one case; returns (exit, log lines, state).
fn run_case(
    dir: &Path,
    opts: &Value,
    rest: &[String],
) -> (Option<String>, Vec<String>, Option<Value>) {
    let env = Env::isolated(dir);
    let mut opts = cli_opts(opts);
    let init = config::initopts(&mut opts, rest, &env, Quirks::default(), None).expect("initopts");
    let b = &init.config.biber;
    let mut log = Logger::memory(LogSettings::new(
        b.trace(),
        b.debug(),
        b.num("quiet") as i64,
        true,
        b.is_true("onlylog"),
    ));
    init.log_startup(&mut log, "This is bibers");
    let mut cfg = init.config;
    log.set_nodieonerror(cfg.biber.nodieonerror());
    let bcf_path = cfg.biber.get_str("bcf").unwrap().to_string();
    let res = bcf::parse_ctrlfile(
        &mut cfg,
        &mut log,
        &env,
        &ParseArgs {
            ctrl_file: &bcf_path,
            output_target: None,
        },
    );
    let lines = |log: &Logger| -> Vec<String> {
        log.memory_lines()
            .0
            .into_iter()
            .map(|l| l.strip_prefix("S:").unwrap_or(&l).to_string())
            .collect()
    };
    match res {
        Err(_) => (
            Some("2".into()),
            lines(&log),
            Some(bcf::dump_state(&cfg, None, log.warnings())),
        ),
        Ok(Parsed::NoDataSources) => (Some("0".into()), lines(&log), None),
        Ok(Parsed::Document(mut doc)) => {
            for e in cfg.postprocess_biber_opts() {
                let _ = log.biber_error(bibers_core::loc!(), &e);
            }
            let r = bcf::process_setup(&mut cfg, &mut log, &mut doc);
            let exit = r.err().map(|_| "2".to_string());
            let warnings = log.warnings().to_vec();
            (
                exit,
                lines(&log),
                Some(bcf::dump_state(&cfg, Some(&doc), &warnings)),
            )
        }
    }
}

#[test]
fn bcf_state_matches_biber() {
    let data = data_dir();
    let pool = data.join("pool");
    let cases: BTreeMap<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(data.join("cases.json")).unwrap()).unwrap();
    let corpus = root().join("tests/corpus");
    let mut failures = Vec::new();
    let mut compared = 0;
    for (id, case) in &cases {
        if case.get("die").is_some() {
            // biber crashes (DC-005); bibers deliberately differs.
            continue;
        }
        if case["opts"].is_null() {
            continue; // CLI error before Biber->new: covered by the CLI tests
        }
        let rest: Vec<String> = case["rest"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let (exit, log, state) = run_case(&corpus.join(id), &case["opts"], &rest);
        let mut msgs = Vec::new();
        let want_exit = case["exit"].as_str().map(str::to_string);
        if exit != want_exit {
            msgs.push(format!("exit: expected {want_exit:?} got {exit:?}"));
        }
        let want_log: Vec<String> = case["log"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        if log != want_log {
            msgs.push(format!("log: expected {want_log:?} got {log:?}"));
        }
        if let (Some(Value::Object(want)), Some(got)) = (case.get("state"), &state) {
            let mut want_v = serde_json::Map::new();
            for (k, v) in want {
                let v = if k == "dm" {
                    match v {
                        Value::Object(m) => Value::Object(
                            m.iter()
                                .map(|(k, v)| (k.clone(), unpool(v, &pool)))
                                .collect(),
                        ),
                        other => other.clone(),
                    }
                } else {
                    unpool(v, &pool)
                };
                want_v.insert(k.clone(), v);
            }
            diff("", &Value::Object(want_v), got, &mut msgs);
        }
        compared += 1;
        if !msgs.is_empty() {
            failures.push(format!("{id}:\n  {}", msgs.join("\n  ")));
        }
    }
    assert!(compared > 400, "only {compared} cases compared");
    assert!(
        failures.is_empty(),
        "{} of {compared} cases differ from biber:\n{}",
        failures.len(),
        failures
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

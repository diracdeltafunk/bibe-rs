//! Differential test of the command-line prelude against biber's own
//! Getopt::Long call and option post-processing (bin/biber:31-329), run by
//! `tools/oracle/probes/bibers-core/probe.pl getopt` under oracle O3.

use bibers::cli::{Prelude, prelude};
use bibers::getopt::Val;
use serde_json::Value;

fn val_json(v: &Val) -> Value {
    match v {
        Val::Scalar(s) => s.clone().into(),
        Val::Array(a) => Value::Array(a.iter().map(|s| s.clone().into()).collect()),
        Val::Hash(h) => Value::Object(
            h.iter()
                .map(|(k, v)| (k.clone(), v.clone().into()))
                .collect(),
        ),
    }
}

#[test]
fn prelude_matches_biber() {
    let data = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/getopt.jsonl"),
    )
    .unwrap();
    let mut failures = Vec::new();
    let mut n = 0;
    for line in data.lines().filter(|l| !l.trim().is_empty()) {
        let case: Value = serde_json::from_str(line).unwrap();
        let argv: Vec<String> = case["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        n += 1;
        let got = prelude(&argv, bibers::run::special);
        let want_exit = case["exit"].as_str();
        let want_stdout = case["stdout"].as_str().unwrap_or("");
        let want_stderr = case["stderr"].as_str().unwrap_or("");
        let mut msgs = Vec::new();
        match &got {
            Prelude::Exit {
                code,
                stdout,
                stderr,
            } => {
                let is_version = argv
                    .iter()
                    .any(|a| a == "--version" || a == "-v" || a == "--vers");
                let is_toolconfig = argv.iter().any(|a| a == "--tool-config");
                if is_version {
                    // DC-040: our own version line.
                    if *code != 0 || !stdout.starts_with("bibers version: ") {
                        msgs.push(format!("version: {code} {stdout:?}"));
                    }
                } else if is_toolconfig {
                    // DC-030: tool mode deferred.
                    if *code != 2 {
                        msgs.push("tool-config should fail".into());
                    }
                } else {
                    if Some(code.to_string().as_str()) != want_exit {
                        msgs.push(format!("exit: expected {want_exit:?} got {code}"));
                    }
                    if want_stdout.starts_with("md5:") {
                        if stdout != bibers::cli::HELP {
                            msgs.push("stdout: expected the help text".into());
                        }
                    } else if stdout != want_stdout {
                        msgs.push(format!("stdout: expected {want_stdout:?} got {stdout:?}"));
                    }
                    if stderr != want_stderr {
                        msgs.push(format!("stderr: expected {want_stderr:?} got {stderr:?}"));
                    }
                }
            }
            Prelude::Run {
                opts,
                argv: rest,
                stderr,
            } => {
                if !case["exit"].is_null() {
                    msgs.push(format!("expected exit {want_exit:?}, got Run"));
                } else {
                    let got_opts =
                        Value::Object(opts.iter().map(|(k, v)| (k.clone(), val_json(v))).collect());
                    if got_opts != case["opts"] {
                        msgs.push(format!("opts: expected {} got {got_opts}", case["opts"]));
                    }
                    let want_rest: Vec<String> = case["rest"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap().to_string())
                        .collect();
                    if *rest != want_rest {
                        msgs.push(format!("rest: expected {want_rest:?} got {rest:?}"));
                    }
                    if stderr != want_stderr {
                        msgs.push(format!("stderr: expected {want_stderr:?} got {stderr:?}"));
                    }
                }
            }
        }
        if !msgs.is_empty() {
            failures.push(format!("{argv:?}:\n  {}", msgs.join("\n  ")));
        }
    }
    assert!(n > 100);
    assert!(
        failures.is_empty(),
        "{} of {n} argv cases differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

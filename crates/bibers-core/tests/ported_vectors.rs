//! Unit vectors from biber's own tests (`tests/ported/{configfile,
//! tool-config,options}.json`) for the configuration front end.
//!
//! `tests/data/conf/` holds verbatim copies of biber's `data/biber-tool.conf`
//! and `t/tdata/tool-testconfig.conf` (Artistic-2.0).

use std::path::{Path, PathBuf};

use bibers_core::bcf::{self, ParseArgs, Parsed};
use bibers_core::config::{self, CliOptions, CliValue, OptValue, biber_value_json};
use bibers_core::datamodel::DataModel;
use bibers_core::env::Env;
use bibers_core::log::{LogSettings, Logger};
use bibers_core::quirks::Quirks;
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn vectors(name: &str) -> Vec<Value> {
    let s = std::fs::read_to_string(root().join("tests/ported").join(name)).unwrap();
    let v: Value = serde_json::from_str(&s).unwrap();
    v["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| !v["target"].as_str().unwrap().contains("RNG"))
        .cloned()
        .collect()
}

/// `{}` and `null` sorting-pass options are the same thing (biber's sort
/// code autovivifies the undef).
fn norm(v: &Value) -> Value {
    match v {
        Value::Null => Value::Object(Default::default()),
        Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), norm(v))).collect()),
        o => o.clone(),
    }
}

fn s(v: &str) -> CliValue {
    CliValue::Str(v.to_string())
}

#[test]
fn configfile_t() {
    // configfile.t: Biber->new(configfile => 'biber-test.conf', mincrossrefs => 7);
    // parse_ctrlfile('general.bcf')
    let dir = tempfile::tempdir().unwrap();
    let corpus = root().join("tests/corpus");
    std::fs::copy(
        corpus.join("ported/basic-misc-1/biber-test.conf"),
        dir.path().join("biber-test.conf"),
    )
    .unwrap();
    std::fs::copy(
        corpus.join("tdata/general/general.bcf"),
        dir.path().join("general.bcf"),
    )
    .unwrap();
    let env = Env::isolated(dir.path());
    let mut opts = CliOptions::new();
    opts.insert("configfile".into(), s("biber-test.conf"));
    opts.insert("mincrossrefs".into(), s("7"));
    let init = config::initopts(&mut opts, &[], &env, Quirks::default(), None).unwrap();
    let mut cfg = init.config;
    let mut log = Logger::memory(LogSettings::new(false, false, 0, true, false));
    let r = bcf::parse_ctrlfile(
        &mut cfg,
        &mut log,
        &env,
        &ParseArgs {
            ctrl_file: "general.bcf",
            output_target: None,
        },
    )
    .unwrap();
    assert!(matches!(r, Parsed::Document(_)));
    for v in vectors("configfile.json") {
        let opt = v["call"]["args"][1].as_str().unwrap();
        let got = cfg
            .biber
            .get(opt)
            .map(biber_value_json)
            .unwrap_or(Value::Null);
        let want = if v["ret"].is_null() {
            &v["expected"]
        } else {
            &v["ret"]
        };
        assert_eq!(&got, want, "configfile.t test {}: {opt}", v["tests"]);
    }
}

#[test]
fn tool_config_t() {
    // tool-config.t: tool mode with biber-tool.conf and tool-testconfig.conf.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/conf");
    let env = Env::isolated(&dir);
    let mut opts = CliOptions::new();
    opts.insert("tool".into(), s("1"));
    opts.insert("noconf".into(), s("1"));
    opts.insert("configfile".into(), s("tool-testconfig.conf"));
    let init = config::initopts(
        &mut opts,
        &["x.bib".to_string()],
        &env,
        Quirks::default(),
        Some("biber-tool.conf"),
    )
    .unwrap();
    let cfg = init.config;
    let dms = match cfg.blx.global("datamodel") {
        Some(OptValue::Datamodel(v)) => v.clone(),
        _ => panic!("no datamodel"),
    };
    let (dm, errs) = DataModel::new(&dms, false);
    assert!(errs.is_empty());
    for v in vectors("tool-config.json") {
        let args = v["call"]["args"].as_array().unwrap();
        let sub = v["call"]["sub"].as_str().unwrap();
        let want = if v["ret"].is_null() {
            &v["expected"]
        } else {
            &v["ret"]
        };
        let got = match sub {
            "Biber::Config::getoption" => cfg
                .biber
                .get(args[1].as_str().unwrap())
                .map(biber_value_json)
                .unwrap_or(Value::Null),
            "Biber::Config::getblxoption" => cfg
                .blx
                .global(args[2].as_str().unwrap())
                .map(OptValue::to_json)
                .unwrap_or(Value::Null),
            "Biber::DataModel::get_fields_of_type" => Value::Array(
                dm.get_fields_of_type(
                    args[1].as_str().unwrap(),
                    &[args[2].as_str().unwrap()],
                    None,
                )
                .into_iter()
                .map(Value::from)
                .collect(),
            ),
            "Biber::DataModel::is_field_for_entrytype" => Value::from(
                if dm.is_field_for_entrytype(args[1].as_str().unwrap(), args[2].as_str().unwrap()) {
                    "1"
                } else {
                    "0"
                },
            ),
            other => panic!("unhandled {other}"),
        };
        assert_eq!(norm(&got), norm(want), "tool-config.t test {}", v["tests"]);
    }
}

#[test]
fn options_t() {
    let dir = root().join("tests/corpus/ported/options-1");
    let bcf = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.ends_with(".bcf"))
        .unwrap();
    let env = Env::isolated(&dir);
    let mut opts = CliOptions::new();
    opts.insert("noconf".into(), s("1"));
    let init = config::initopts(
        &mut opts,
        std::slice::from_ref(&bcf),
        &env,
        Quirks::default(),
        None,
    )
    .unwrap();
    let mut cfg = init.config;
    let mut log = Logger::memory(LogSettings::new(false, false, 0, true, false));
    bcf::parse_ctrlfile(
        &mut cfg,
        &mut log,
        &env,
        &ParseArgs {
            ctrl_file: &bcf,
            output_target: None,
        },
    )
    .unwrap();
    for v in vectors("options.json") {
        let args = v["call"]["args"].as_array().unwrap();
        let got = match v["call"]["sub"].as_str().unwrap() {
            "Biber::Config::getoption" => cfg
                .biber
                .get(args[1].as_str().unwrap())
                .map(biber_value_json)
                .unwrap_or(Value::Null),
            "Biber::Config::getblxoption" => cfg
                .blx
                .get(
                    None,
                    args[2].as_str().unwrap(),
                    args.get(3).and_then(Value::as_str),
                    None,
                )
                .map(OptValue::to_json)
                .unwrap_or(Value::Null),
            _ => continue, // entry-level (M2)
        };
        assert_eq!(got, v["expected"], "options.t test {}", v["tests"]);
    }
}

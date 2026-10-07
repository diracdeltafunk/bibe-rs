//! `fetch_data`: reading data sources and creating entries
//! (`pipeline::fetch`) matches biber's state.

mod common;

use std::path::{Path, PathBuf};

/// Corpus cases whose `fetch_data` deliberately differs from biber, with
/// the documented reason.
const EXCLUDED: &[(&str, &str)] = &[
    // biblatexml data sources are not supported yet (DC-031)
    ("tdata/biblatexml", "DC-031"),
    ("tdata/datalists", "DC-031"),
    ("tdata/xdata-noloop", "DC-031"),
    ("bugs/bb-050-bltxml-eprint", "DC-031"),
    ("bugs/bb-051-bltxml-uri-escaping", "DC-031"),
    ("ported/biblatexml-1", "DC-031"),
    ("ported/datalists-1", "DC-031"),
    // tool mode / bibtex output (DC-030)
    ("ported/bibtex-output-1", "DC-030"),
    ("ported/full-bibtex-1", "DC-030"),
    // $MAPUNIQ keys are random in biber (DC-018)
    ("tdata/general-configfile", "DC-018"),
    ("ported/basic-misc-2", "DC-018"),
    ("ported/basic-misc-3", "DC-018"),
    ("ported/basic-misc-4", "DC-018"),
    // Perl code in sourcemap replacements is not executed (DC-023)
    ("bugs/bb-033-smap-replace-perl-eval", "DC-023"),
    // dynamic set order under \nocite{*} is Perl hash order (DC-015)
    ("bugs/bb-062-dynamic-sets-nocite-order", "DC-015"),
];

/// Annotations are global module state in biber (not in the phase dumps):
/// check the store against the `\annotation` lines of biber's `.bbl`.
#[test]
fn annotations_match_bbl() {
    use bibers::run::{SetupResult, setup};
    use bibers_core::env::Env;
    use bibers_core::pipeline::{Ctx, fetch};
    let root = bibers_testkit::corpus_root();
    for id in [
        "biblatex-examples/97-annotations",
        "bugs/bb-036-annotation-literal-split",
    ] {
        let case = bibers_testkit::discover(&root)
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap();
        let work = tempfile::tempdir().unwrap();
        for f in case.input_files(&root).unwrap() {
            std::fs::copy(&f, work.path().join(f.file_name().unwrap())).unwrap();
        }
        let env = Env::isolated(work.path());
        let mut args = case.spec.args.clone();
        args.splice(0..0, ["-q".to_string(), "-q".to_string()]);
        let SetupResult::Ready(s) = setup(&args, &env) else {
            panic!("setup failed for {id}")
        };
        let bibers::run::Setup {
            mut cfg,
            doc,
            mut log,
            ..
        } = *s;
        let dm = cfg.dm.take().unwrap();
        let mut ctx = Ctx {
            cfg: &mut cfg,
            dm: &dm,
            doc: &doc,
            log: &mut log,
            env: &env,
            run: Default::default(),
        };
        let secs: Vec<String> = doc
            .sorted_sections()
            .iter()
            .map(|s| s.number.clone())
            .collect();
        for sec in secs {
            let mut st = fetch::init_section(&mut ctx, &sec).unwrap();
            fetch::fetch_data(&mut ctx, &mut st).unwrap();
        }
        let a = &ctx.run.annotations;
        let lit = |l: bool| if l { "1" } else { "0" };
        let mut got = std::collections::BTreeSet::new();
        for m in a.field.values() {
            for (f, names) in m {
                for (n, x) in names {
                    let v = x.value.clone().unwrap_or_default();
                    got.insert(format!(
                        "\\annotation{{field}}{{{f}}}{{{n}}}{{}}{{}}{{{}}}{{{v}}}",
                        lit(x.literal)
                    ));
                }
            }
        }
        for m in a.item.values() {
            for (f, names) in m {
                for (n, counts) in names {
                    for (c, x) in counts {
                        let v = x.value.clone().unwrap_or_default();
                        got.insert(format!(
                            "\\annotation{{item}}{{{f}}}{{{n}}}{{{c}}}{{}}{{{}}}{{{v}}}",
                            lit(x.literal)
                        ));
                    }
                }
            }
        }
        for m in a.part.values() {
            for (f, names) in m {
                for (n, counts) in names {
                    for (c, parts) in counts {
                        for (p, x) in parts {
                            let v = x.value.clone().unwrap_or_default();
                            got.insert(format!(
                                "\\annotation{{part}}{{{f}}}{{{n}}}{{{c}}}{{{p}}}{{{}}}{{{v}}}",
                                lit(x.literal)
                            ));
                        }
                    }
                }
            }
        }
        let mut want = std::collections::BTreeSet::new();
        for e in std::fs::read_dir(case.expected_dir()).unwrap().flatten() {
            if e.path().extension().is_some_and(|x| x == "bbl") {
                for l in std::fs::read_to_string(e.path()).unwrap().lines() {
                    let l = l.trim();
                    if l.starts_with("\\annotation{") {
                        want.insert(l.to_string());
                    }
                }
            }
        }
        assert_eq!(got, want, "{id}");
    }
}

#[test]
fn fetch_data_matches_biber() {
    let cases = common::phase::cases_with_prefix(&[
        "tdata/",
        "biblatex-examples/",
        "bugs/",
        "ported/",
        "styles/",
    ]);
    let ids: Vec<&str> = cases
        .iter()
        .map(String::as_str)
        .filter(|id| !EXCLUDED.iter().any(|(x, _)| x == id))
        .collect();
    common::phase::assert_phase("fetch_data", &ids);
}

/// Remove every `<open…>…</close>` block from `s`.
fn strip_blocks(s: &str, open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(open) {
        let after = &rest[i..];
        // the tag must end right after its name (or have attributes)
        let c = after[open.len()..].chars().next();
        if !matches!(c, Some('>' | ' ' | '\n' | '\t')) {
            out.push_str(&rest[..i + open.len()]);
            rest = &rest[i + open.len()..];
            continue;
        }
        let Some(j) = after.find(close) else { break };
        out.push_str(&rest[..i]);
        rest = &after[j + close.len()..];
    }
    out.push_str(rest);
    out
}

/// A copy of corpus case `id` with all source maps removed (from the `.bcf`
/// and any biber.conf), under `target/nomaps-corpus/nomaps-<suite>/<case>`.
/// Returns the corpus root and the new case id.
fn nomaps_variant(id: &str) -> (PathBuf, String) {
    let corpus = bibers_testkit::corpus_root();
    let root = bibers_testkit::repo_root().join("target/nomaps-corpus");
    let (suite, name) = id.split_once('/').expect("suite/case");
    let newid = format!("nomaps-{suite}/{name}");
    let dest = root.join(&newid);
    std::fs::create_dir_all(&dest).expect("mkdir");
    copy_dir(&corpus.join("shared"), &root.join("shared"));
    for e in std::fs::read_dir(corpus.join(id)).expect("case dir") {
        let p = e.expect("entry").path();
        if p.is_dir() {
            continue;
        }
        let fname = p.file_name().unwrap();
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
        if ext == "bcf" || ext == "conf" {
            let s = std::fs::read_to_string(&p).expect("read");
            let s = strip_blocks(&s, "<bcf:sourcemap", "</bcf:sourcemap>");
            let s = strip_blocks(&s, "<sourcemap", "</sourcemap>");
            write_if_changed(&dest.join(fname), s.as_bytes());
        } else {
            write_if_changed(&dest.join(fname), &std::fs::read(&p).expect("read"));
        }
    }
    (root, newid)
}

fn write_if_changed(p: &Path, b: &[u8]) {
    if std::fs::read(p).ok().as_deref() != Some(b) {
        std::fs::write(p, b).expect("write");
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    if let Ok(rd) = std::fs::read_dir(from) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() {
                write_if_changed(
                    &to.join(p.file_name().unwrap()),
                    &std::fs::read(&p).unwrap(),
                );
            }
        }
    }
}

/// Print a per-suite summary of `fetch_data` over the corpus (run with
/// `cargo test --test phase_fetch -- --ignored --nocapture`; set
/// `FETCH_CASES=tdata/,bugs/` to restrict). Failing cases are re-checked on
/// a copy with all source maps removed, to separate map-dependent
/// differences (the map engine is a separate package) from others.
#[test]
#[ignore]
fn fetch_data_report() {
    if !bibers_testkit::phase::o3_available() {
        return;
    }
    let filter = std::env::var("FETCH_CASES").unwrap_or_default();
    let prefixes: Vec<&str> = if filter.is_empty() {
        vec![
            "tdata/",
            "biblatex-examples/",
            "bugs/",
            "ported/",
            "styles/",
        ]
    } else {
        filter.split(',').collect()
    };
    let cases = common::phase::cases_with_prefix(&prefixes);
    // suite → (occurrences, matching, failing only with maps)
    let mut per_suite: std::collections::BTreeMap<String, (usize, usize, usize)> =
        Default::default();
    for id in &cases {
        let suite = id.split('/').next().unwrap_or("").to_string();
        let (n, f) = common::phase::check_phase("fetch_data", &[id.as_str()], &[]);
        let e = per_suite.entry(suite).or_default();
        e.0 += n;
        e.1 += n.saturating_sub(f.len());
        if f.is_empty() {
            continue;
        }
        let (root, nid) = nomaps_variant(id);
        let (nn, nf) = common::phase::check_phase_at(&root, "fetch_data", &[nid.as_str()], &[]);
        if nn > 0 && nf.is_empty() {
            e.2 += f.len();
            for x in f {
                println!("MAPS {x}");
            }
            continue;
        }
        for x in f {
            println!("FAIL {x}");
        }
        for x in nf {
            println!("NOMAPS-FAIL {x}");
        }
    }
    for (s, (n, ok, maps)) in per_suite {
        println!("SUITE {s}: {ok}/{n} match; {maps} differ only through source maps");
    }
}

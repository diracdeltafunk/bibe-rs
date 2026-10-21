//! serial == parallel (DC-068): every corpus case gives byte-identical
//! results with `--threads 1` and with 8 threads, with every parallel loop
//! forced on (`BIBERS_PAR_MIN_ITEMS=1`; the corpus inputs are smaller than
//! the default threshold), and repeated parallel runs are identical.
//!
//! The bench workloads are checked the same way by
//! `tools/bench/run.py --check-threads`.

use bibers_testkit::{Case, RunOutput, Runner, corpus_root, discover};

/// What must not depend on the thread count.
fn fingerprint(run: &RunOutput) -> String {
    let blg: Vec<&str> = run
        .blg
        .lines()
        // The elapsed-time column and the date line.
        .map(|l| match l.strip_prefix('[') {
            Some(r) => r.split_once("] ").map_or(l, |(_, rest)| rest),
            None => l,
        })
        .filter(|l| !l.contains(" - === "))
        .collect();
    format!(
        "exit: {:?}\noutputs: {:?}\nstdout:\n{}\nstderr:\n{}\nblg:\n{}",
        run.exit_code,
        run.outputs,
        run.stdout,
        run.stderr,
        blg.join("\n")
    )
}

fn check(case: &Case, root: &std::path::Path, runners: &[Runner]) -> Option<String> {
    let runs: Vec<String> = runners
        .iter()
        .map(|r| fingerprint(&r.run(case, root, false).expect("run")))
        .collect();
    let first = &runs[0];
    runs[1..].iter().position(|r| r != first).map(|i| {
        let d = similar::TextDiff::from_lines(first.as_str(), runs[i + 1].as_str());
        format!(
            "{} (run {} vs --threads 1):\n{}",
            case.id,
            i + 1,
            d.unified_diff().context_radius(2)
        )
    })
}

#[test]
fn corpus_identical_at_1_and_8_threads() {
    let root = corpus_root();
    let cases: Vec<Case> = discover(&root)
        .expect("corpus loads")
        .into_iter()
        .filter(|c| c.missing_requirements().is_empty())
        .collect();
    let bin = env!("CARGO_BIN_EXE_bibers").to_string();
    let serial = Runner::new("bibers-1", vec![bin.clone(), "--threads=1".into()]);
    let parallel = Runner::new("bibers-8", vec![bin, "--threads=8".into()])
        .with_env("BIBERS_PAR_MIN_ITEMS", "1");
    let runners = [serial, parallel.clone(), parallel];
    let jobs = std::thread::available_parallelism().map_or(4, usize::from);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failures = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(case) = cases.get(i) else { break };
                    if let Some(f) = check(case, &root, &runners) {
                        failures.lock().unwrap().push(f);
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    assert!(
        failures.is_empty(),
        "{} of {} cases depend on the thread count:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

//! Startup microbenchmark: runs bibers' setup (command line, options,
//! `.bcf`, data model) `N` times in one process, for timing and profiling
//! the start of a run without the processing pipeline.
//!
//!     cargo build --profile profiling -p bibers --example startup
//!     target/profiling/examples/startup DIR N [ARGS...]
//!
//! `DIR` is the working directory (holding the `.bcf`); `ARGS` default to
//! `-q --noconf --nolog doc.bcf`. Add `--pipeline` before `DIR` to time
//! whole runs (setup and processing) instead.

use std::time::Instant;

use bibers::run::{SetupResult, process, setup};
use bibers_core::env::Env;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let pipeline = args.first().is_some_and(|a| a == "--pipeline");
    if pipeline {
        args.remove(0);
    }
    let (dir, n) = match args.as_slice() {
        [dir, n, ..] => (dir.clone(), n.parse::<u32>().expect("N")),
        _ => {
            eprintln!("usage: startup [--pipeline] DIR N [ARGS...]");
            std::process::exit(2);
        }
    };
    let mut run_args: Vec<String> = args[2..].to_vec();
    if run_args.is_empty() {
        run_args = ["-q", "--noconf", "--nolog", "doc.bcf"]
            .map(String::from)
            .to_vec();
    }
    let mut env = Env::from_process();
    env.cwd = std::path::PathBuf::from(dir);
    let t0 = Instant::now();
    for _ in 0..n {
        match setup(&run_args, &env) {
            SetupResult::Ready(s) => {
                if pipeline {
                    process(*s, &env);
                }
            }
            SetupResult::Exit(code) => {
                eprintln!("setup exited with {code}");
                std::process::exit(1);
            }
        }
    }
    let per = t0.elapsed() / n;
    println!("{} per run ({n} runs)", humanize(per));
}

fn humanize(d: std::time::Duration) -> String {
    format!("{:.3} ms", d.as_secs_f64() * 1000.0)
}

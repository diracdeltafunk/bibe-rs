//! Throughput benchmark: decodes (and encodes the result of) every line of
//! the given `.bib` files, approximating biber's per-field `latex_decode`.
//!
//! ```sh
//! cargo run -p latex-recode --release --example recode_bench -- tests/corpus/shared/biblatex-examples.bib
//! ```

use std::time::Instant;

use latex_recode::{Normalization, RecodeOptions, Recoder};

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    let mut lines = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).expect("read file");
        let text = Normalization::Nfd.apply(&text);
        lines.extend(text.lines().map(str::to_owned));
    }
    // Adversarial shapes (should all be linear).
    let r = Recoder::builtin("full", "full", RecodeOptions::BIBER);
    for (name, s) in [
        ("macro+groups", format!("\\x{}", "{a}".repeat(100_000))),
        ("brace runs", "{".repeat(100_000) + &"}".repeat(100_000)),
        ("backslashes", "\\'".repeat(100_000)),
        ("accents", "\\'{\\i}\\c c\\t{oo}".repeat(20_000)),
        ("marks", format!("{{a{}}}", "\u{301}".repeat(100_000))),
    ] {
        let t = Instant::now();
        let n = r.decode(&s).len() + r.encode(&s).len();
        println!(
            "{name}: {} bytes in {:.1} ms [{n}]",
            s.len(),
            t.elapsed().as_secs_f64() * 1e3
        );
    }
    let bytes: usize = lines.iter().map(|l| l.len() + 1).sum();
    for set in ["base", "full"] {
        let r = Recoder::builtin(set, set, RecodeOptions::BIBER);
        let reps = (50_000_000 / bytes.max(1)).max(1);
        let t = Instant::now();
        let mut sink = 0usize;
        for _ in 0..reps {
            for l in &lines {
                sink += r.decode(l).len();
            }
        }
        let dt = t.elapsed().as_secs_f64();
        let decoded: Vec<String> = lines.iter().map(|l| r.decode(l)).collect();
        let t2 = Instant::now();
        for _ in 0..reps {
            for l in &decoded {
                sink += r.encode(l).len();
            }
        }
        let dt2 = t2.elapsed().as_secs_f64();
        let mb = (bytes * reps) as f64 / 1e6;
        println!(
            "{set}: {} lines x{reps}: decode {:.1} MB/s ({:.0} ns/line), encode {:.1} MB/s ({:.0} ns/line) [{sink}]",
            lines.len(),
            mb / dt,
            dt * 1e9 / (lines.len() * reps) as f64,
            mb / dt2,
            dt2 * 1e9 / (lines.len() * reps) as f64,
        );
    }
}

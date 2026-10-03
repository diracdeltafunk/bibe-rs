//! Rough throughput numbers: `cargo run -p btparse --release --example bench [file.bib]`
//! (default: biblatex-examples.bib via kpsewhich), replicated to ~50 MB.

use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        let o = std::process::Command::new("kpsewhich")
            .arg("biblatex-examples.bib")
            .output()
            .expect("kpsewhich");
        String::from_utf8(o.stdout).unwrap().trim().to_string()
    });
    let one = std::fs::read_to_string(&path).expect("read bib");
    let reps = (50_000_000 / one.len().max(1)).max(1);
    let big = one.repeat(reps);
    let mb = big.len() as f64 / 1e6;

    let mut st = btparse::BtState::biber();
    let t = Instant::now();
    let parsed = st.parse(&big);
    let dt = t.elapsed().as_secs_f64();
    let nfields: usize = parsed.entries.iter().map(|e| e.fields.len()).sum();
    println!(
        "parse: {mb:.1} MB, {} entries, {nfields} fields in {dt:.3} s = {:.0} MB/s",
        parsed.entries.len(),
        mb / dt
    );

    // names: every author/editor value, split and each name split + formatted
    let lists: Vec<&str> = parsed
        .entries
        .iter()
        .flat_map(|e| e.fields.iter())
        .filter(|f| f.name == "author" || f.name == "editor")
        .filter_map(|f| f.value.as_deref())
        .collect();
    let t = Instant::now();
    let mut names = 0usize;
    let mut bytes = 0usize;
    for l in &lists {
        for n in btparse::split_list(l, "and").items {
            let r = btparse::split_name(n);
            for p in [
                btparse::NamePart::First,
                btparse::NamePart::Von,
                btparse::NamePart::Last,
                btparse::NamePart::Jr,
            ] {
                bytes += r.parts.format(p).len();
            }
            names += 1;
        }
    }
    let dt = t.elapsed().as_secs_f64();
    println!(
        "names: {} lists, {names} names split+formatted in {dt:.3} s = {:.2} M names/s ({bytes} bytes out)",
        lists.len(),
        names as f64 / dt / 1e6
    );
}

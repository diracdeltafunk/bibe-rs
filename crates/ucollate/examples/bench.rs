//! Rough timing of collator construction and sort-key generation.
//!
//! `cargo run -p ucollate --release --example bench`

use std::hint::black_box;
use std::time::Instant;

use ucollate::{CollateOptions, Collator};

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    println!("{label:<48} {:>10.3} ms", t.elapsed().as_secs_f64() * 1e3);
    r
}

fn per_key(label: &str, c: &Collator, strings: &[String]) {
    let rounds = 20;
    let mut out = Vec::with_capacity(256);
    let t = Instant::now();
    let mut bytes = 0;
    for _ in 0..rounds {
        for s in strings {
            out.clear();
            c.sort_key_into(black_box(s), &mut out);
            bytes += out.len();
        }
    }
    let n = (rounds * strings.len()) as f64;
    let avg_chars =
        strings.iter().map(|s| s.chars().count()).sum::<usize>() as f64 / strings.len() as f64;
    println!(
        "{label:<48} {:>10.1} ns/key ({avg_chars:.0} chars avg, {} B key avg)",
        t.elapsed().as_secs_f64() * 1e9 / n,
        bytes / (rounds * strings.len())
    );
}

fn main() {
    let biber = CollateOptions::biber_sort(4, true);
    let d = time("first Collator::new(en-US) (no data load)", || {
        Collator::new(Some("en-US"), &biber).unwrap()
    });
    time("first Collator::new(sv) (parse tailoring)", || {
        Collator::new(Some("sv"), &biber).unwrap()
    });
    time("second Collator::new(sv) (cached)", || {
        Collator::new(Some("sv"), &biber).unwrap()
    });
    time("first Collator::new(ja) (tailoring + JIS table)", || {
        Collator::new(Some("ja"), &biber).unwrap()
    });
    time("first Collator::new(zh__pinyin)", || {
        Collator::new(Some("zh__pinyin"), &biber).unwrap()
    });
    time("first Collator::new(ko)", || {
        Collator::new(Some("ko"), &biber).unwrap()
    });

    // typical biber sort strings (padded name keys, years, titles; NFD)
    let names = [
        "Doe",
        "Smith",
        "Ångström",
        "Müller",
        "van Beethoven",
        "Østergård",
        "Zola",
        "Ïbsen",
        "Ševčík",
        "Nguyễn",
    ];
    let titles = [
        "The Art of Computer Programming",
        "A Treatise on Electricity and Magnetism",
        "Über die Elektrodynamik bewegter Körper",
        "On the Origin of Species",
        "Ἰλιάς",
        "Война и мир",
    ];
    use unicode_normalization::UnicodeNormalization;
    let mut strings = Vec::new();
    for (i, n) in names.iter().enumerate() {
        for (j, t) in titles.iter().enumerate() {
            let s = format!("mm,,{n:<16}John  ,{},{t}", 1900 + i * 7 + j);
            strings.push(s.nfd().collect::<String>());
        }
    }
    let short: Vec<String> = names.iter().map(|n| n.nfd().collect()).collect();
    let sv = Collator::new(Some("sv"), &biber).unwrap();
    let init = Collator::new(Some("en-US"), &CollateOptions::biber_sortinit()).unwrap();
    per_key("biber sort key, en-US, full sort string", &d, &strings);
    per_key("biber sort key, en-US, short name", &d, &short);
    per_key("biber sort key, sv, full sort string", &sv, &strings);
    per_key("sortinithash collator (NFD, level 1), short", &init, &short);
    let inits: Vec<String> = ["A", "B", "É", "Z", "Ø"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    per_key("sortinithash collator, single grapheme", &init, &inits);
}

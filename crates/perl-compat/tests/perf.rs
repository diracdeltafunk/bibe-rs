//! Rough throughput numbers (`cargo test --release -p perl-compat --test perf
//! -- --ignored --nocapture`).

use std::time::Instant;

use perl_compat::grapheme::{gcstring, perl};
use perl_compat::regex::{PerlRegex, ireplace};
use perl_compat::unicode::{nfc, nfd};
use perl_compat::{fc, lc};

const N: usize = 200_000;

fn bench(name: &str, mut f: impl FnMut(usize) -> usize) {
    let t = Instant::now();
    let mut acc = 0;
    for i in 0..N {
        acc += f(i);
    }
    let ns = t.elapsed().as_nanos() as f64 / N as f64;
    eprintln!("{name:32} {ns:8.0} ns/op ({acc})");
}

#[test]
#[ignore = "benchmark"]
fn throughput() {
    let names = [
        "Müller, Hans-Jürgen",
        "Ludwig van Beethoven",
        "Šimon Čapek",
        "Doe, John",
        "Ἀριστοτέλης",
    ];
    bench("lc", |i| lc(names[i % 5]).len());
    bench("fc", |i| fc(names[i % 5]).len());
    bench("nfd", |i| nfd(names[i % 5]).len());
    bench("nfc(nfd)", |i| nfc(&nfd(names[i % 5])).len());
    bench("gcstring::length", |i| gcstring::length(names[i % 5]));
    bench("gcstring::substr(0,1)", |i| {
        gcstring::substr(names[i % 5], 0, Some(1)).len()
    });
    bench("perl::clusters", |i| perl::clusters(names[i % 5]).count());
    let re = PerlRegex::new(r"\s*,\s*", false).unwrap();
    bench("split xsvsep", |i| re.split(names[i % 5]).unwrap().len());
    let re = PerlRegex::new(r"\p{Dash}", false).unwrap();
    bench("remove_all \\p{Dash}", |i| {
        re.remove_all(names[i % 5]).unwrap().len()
    });
    bench("ireplace (compile+run)", |i| {
        ireplace(names[i % 5], "(\\w+)", "\\u$1", false)
            .unwrap()
            .value
            .len()
    });
}

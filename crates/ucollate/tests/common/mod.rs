//! Shared helpers for the differential tests: collator specs, deterministic input sets,
//! the Perl oracle driver and digests.
#![allow(dead_code)]

mod suites;
#[allow(unused_imports)]
pub use suites::*;

use std::fmt::Write as _;
use std::path::PathBuf;

use md5::{Digest, Md5};
use ucollate::{CollateOptions, Collator};

/// An option value as passed to Perl.
#[derive(Clone, Debug)]
pub enum Val {
    Str(&'static str),
    Undef,
    List(Vec<u8>),
}

/// A collator configuration: `Unicode::Collate::Locale->new(locale => L, %new)->change(%change)`.
#[derive(Clone, Debug)]
pub struct Spec {
    pub name: String,
    pub locale: Option<String>,
    pub new: Vec<(&'static str, Val)>,
    pub change: Vec<(&'static str, Val)>,
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => write!(o, "\\u{:04x}", c as u32).unwrap(),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn json_opts(v: &[(&'static str, Val)]) -> String {
    let parts: Vec<String> = v
        .iter()
        .map(|(k, v)| {
            let v = match v {
                Val::Str(s) => json_str(s),
                Val::Undef => "null".into(),
                Val::List(l) => format!(
                    "[{}]",
                    l.iter()
                        .map(|x| x.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            };
            format!("{}:{}", json_str(k), v)
        })
        .collect();
    format!("{{{}}}", parts.join(","))
}

impl Spec {
    pub fn new(name: impl Into<String>, locale: Option<&str>) -> Spec {
        Spec {
            name: name.into(),
            locale: locale.map(str::to_string),
            new: vec![],
            change: vec![],
        }
    }
    pub fn new_opt(mut self, k: &'static str, v: Val) -> Spec {
        self.new.push((k, v));
        self
    }
    pub fn opt(mut self, k: &'static str, v: Val) -> Spec {
        self.change.push((k, v));
        self
    }
    pub fn json(&self) -> String {
        format!(
            "{{\"locale\":{},\"new\":{},\"change\":{}}}",
            self.locale.as_deref().map_or("null".into(), json_str),
            json_opts(&self.new),
            json_opts(&self.change)
        )
    }
    pub fn options(&self) -> CollateOptions {
        let mut o = CollateOptions::default();
        for (k, v) in self.new.iter().chain(self.change.iter()) {
            match v {
                Val::Str(s) if *k == "entry" => o.entry = Some(s.to_string()),
                Val::Str(s) => o.set_perl(k, Some(s)).unwrap(),
                Val::Undef => o.set_perl(k, None).unwrap(),
                Val::List(l) => {
                    assert_eq!(*k, "backwards");
                    o.backwards = Some(l.clone());
                }
            }
        }
        o
    }
    pub fn collator(&self) -> Collator {
        Collator::new(self.locale.as_deref(), &self.options()).unwrap()
    }
}

/// biber's sort collator: level, upper_before_lower.
pub fn biber_sort(locale: Option<&str>, level: &'static str, ubl: &'static str) -> Spec {
    Spec::new(
        format!("{}/sort-l{level}-ubl{ubl}", locale.unwrap_or("undef")),
        locale,
    )
    .opt("level", Val::Str(level))
    .opt("variable", Val::Str("non-ignorable"))
    .opt("normalization", Val::Str("prenormalized"))
    .opt("upper_before_lower", Val::Str(ubl))
}

/// biber's sortinithash collator.
pub fn biber_sortinit(locale: Option<&str>) -> Spec {
    Spec::new(format!("{}/sortinit", locale.unwrap_or("undef")), locale)
        .new_opt("level", Val::Str("1"))
}

/// Module defaults (level 4, shifted, NFD).
pub fn module_default(locale: Option<&str>) -> Spec {
    Spec::new(format!("{}/default", locale.unwrap_or("undef")), locale)
}

// ---------------------------------------------------------------- inputs

pub fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn parse_keys(text: &str) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with('#') || line.starts_with('@') || line.trim().is_empty() {
            continue;
        }
        let k = line.split(';').next().unwrap();
        out.push(
            k.split_whitespace()
                .map(|h| u32::from_str_radix(h, 16).unwrap())
                .collect(),
        );
    }
    out
}

/// Tailoring keys of each locale file, from data/locales.txt: (stem, keys).
pub fn locale_keys() -> Vec<(String, Vec<Vec<u32>>)> {
    let text = std::fs::read_to_string(crate_dir().join("data/locales.txt")).unwrap();
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(stem) = line.strip_prefix("@locale ") {
            out.push((stem.trim().to_string(), String::new()));
        } else if let Some(last) = out.last_mut() {
            last.1.push_str(line);
            last.1.push('\n');
        }
    }
    out.into_iter().map(|(s, t)| (s, parse_keys(&t))).collect()
}

/// DUCET contraction keys.
pub fn ducet_contractions() -> Vec<Vec<u32>> {
    let text = std::fs::read_to_string(crate_dir().join("data/allkeys-13.0.0.txt")).unwrap();
    let mut v: Vec<Vec<u32>> = parse_keys(
        &text
            .lines()
            .map(|l| l.split('#').next().unwrap())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .into_iter()
    .filter(|k| k.len() > 1)
    .collect();
    v.sort();
    v
}

/// One representative accepted locale name per tailoring file stem, plus `None`.
pub fn locale_names() -> Vec<Option<&'static str>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut v = vec![None];
    for l in ucollate::tailored_locales() {
        let c = Collator::new(Some(l), &CollateOptions::default()).unwrap();
        // bs/sr_Latn share hr, bs_Cyrl shares sr
        let stem = match l {
            "bs" | "sr_Latn" => "hr",
            "bs_Cyrl" => "sr",
            x => x,
        };
        assert_eq!(c.locale(), l);
        if seen.insert(stem) {
            v.push(Some(l));
        }
    }
    v
}

/// Strings exercising a set of contraction/tailoring keys: the key alone, with combining
/// marks appended or inserted (discontiguous matching), and in context.
pub fn key_strings(keys: &[Vec<u32>]) -> Vec<Vec<u32>> {
    let marks: [&[u32]; 8] = [
        &[0x0301],
        &[0x0323],
        &[0x0323, 0x0301],
        &[0x0301, 0x0323],
        &[0x0308],
        &[0x0327],
        &[0x031B, 0x0301],
        &[0x0F71],
    ];
    let mut out = Vec::new();
    for k in keys {
        out.push(k.clone());
        for m in marks {
            let mut s = k.clone();
            s.extend_from_slice(m);
            out.push(s);
        }
        if k.len() >= 2 {
            for m in [&[0x0301u32][..], &[0x0323], &[0x0300], &[0x0334], &[0x05B0]] {
                let mut s = vec![k[0]];
                s.extend_from_slice(m);
                s.extend_from_slice(&k[1..]);
                out.push(s);
            }
            let mut s = k[..k.len() - 1].to_vec();
            s.push(0x0301);
            out.push(s);
        }
        let mut s = vec![0x61];
        s.extend_from_slice(k);
        out.push(s);
        let mut s = k.clone();
        s.extend_from_slice(&[0x20, 0x41]);
        out.push(s);
        let mut s = k.clone();
        s.push(0x2D);
        s.extend_from_slice(k);
        out.push(s);
    }
    out
}

/// Hand-picked strings.
pub fn targeted() -> Vec<Vec<u32>> {
    let s = |t: &str| t.chars().map(u32::from).collect::<Vec<u32>>();
    let mut v: Vec<Vec<u32>> = vec![
        vec![],
        s("a"),
        s("A"),
        s("Perl"),
        s("perl"),
        s("PERL"),
        s("Zebra     Anna"),
        s("A\u{30A}ngstro\u{308}m  Bob "),
        s("mm,,AATESTKEY,AATESTKEY"),
        s("The Foo\\Bar"),
        s("Doe!Smith!Jones\u{10FFFD}"),
        s("Doe John\u{10FFFD}"),
        s("\u{10FFFD}"),
        s("\u{FFFE}"),
        s("\u{FFFF}"),
        s("\u{FFFD}"),
        s("a\u{FFFE}b"),
        s("ab\u{FFFF}"),
        s("$100"),
        s("-50"),
        s("2000"),
        s("½"),
        s("ß"),
        s("Æsir"),
        s("œuvre"),
        s("ĳ"),
        s("ﬁ"),
        s("Ⅻ"),
        s("①"),
        s("Ａ"),
        s("ｱ"),
        s("ア"),
        s("あ"),
        s("ァ"),
        s("ヴ"),
        s("ゔ"),
        s("ー"),
        s("中文"),
        s("日本語"),
        s("한국어"),
        s("\u{1100}\u{1161}\u{11A8}"),
        s("\u{1100}\u{1161}\u{1100}\u{1161}"),
        s("\u{1100}\u{1100}\u{1161}\u{11A8}\u{11A8}"),
        s("\u{1161}\u{11A8}"),
        s("\u{AC00}\u{11A8}"),
        s("\u{AC01}\u{1161}"),
        s("\u{1100}"),
        s("\u{1161}"),
        s("\u{11A8}"),
        s("\u{AC00}a"),
        s("가각"),
        s("\u{3131}\u{314F}"),
        s("Ελληνικά"),
        s("Русский"),
        s("Й"),
        s("И\u{306}"),
        s("и\u{306}"),
        s("ё"),
        s("е\u{308}"),
        s("עברית"),
        s("العربية"),
        s("हिन्दी"),
        s("क\u{93C}"),
        s("ไทย"),
        s("เก"),
        s("\u{0E40}\u{0E01}"),
        s("\u{0FB2}\u{0F71}\u{0F80}"),
        s("\u{0FB2}\u{0F80}\u{0F71}"),
        s("\u{0FB2}\u{0344}\u{0F71}\u{0F80}"),
        s("\u{0FB2}\u{0F71}\u{0F80}\u{0344}"),
        s("\u{0FB3}\u{0F71}\u{0F72}"),
        s("\u{0F71}\u{0F72}\u{0F74}"),
        s("l\u{B7}"),
        s("L\u{387}"),
        s("l\u{301}\u{B7}"),
        s("l\u{323}\u{B7}"),
        s("\u{17000}\u{18AFF}\u{18D00}\u{18D8F}\u{18D90}"),
        s("\u{1B170}\u{1B2FF}\u{18B00}\u{18CFF}"),
        s("\u{4E00}\u{9FFC}\u{9FFD}\u{9FFF}\u{3400}\u{4DBF}\u{4DC0}"),
        s(
            "\u{20000}\u{2A6DD}\u{2A6DE}\u{2A700}\u{2B734}\u{2B735}\u{2B740}\u{2B81D}\u{2B820}\u{2CEA1}\u{2CEB0}\u{2EBE0}\u{30000}\u{3134A}\u{3134B}",
        ),
        s("\u{F900}\u{FA0E}\u{FA10}\u{FA11}\u{2F800}"),
        s("\u{E000}\u{F8FF}\u{F0000}\u{10FFFF}"),
        s("\u{378}\u{E0001}\u{E0100}"),
        s("😀👍🏽👨\u{200D}👩"),
        s("a\u{0}b"),
        s("a\u{AD}b"),
        s("a\u{200B}b\u{200D}c"),
        s("\t\n "),
        s("a b"),
        s("a-b"),
        s("a_b"),
        s("ab"),
        s("a\u{301}\u{301}"),
        s("e\u{301}\u{323}"),
        s("e\u{323}\u{301}"),
        s("o\u{31B}\u{301}"),
        s("o\u{301}\u{31B}"),
        s("u\u{308}\u{304}"),
        s("Å"),
        s("A\u{30A}"),
        s("Ä"),
        s("ä"),
        s("aa"),
        s("Aa"),
        s("AA"),
        s("ch"),
        s("Ch"),
        s("CH"),
        s("c\u{301}h"),
        s("c\u{30C}"),
        s("ll"),
        s("ñ"),
        s("n\u{303}"),
        s("dzs"),
        s("ddzs"),
        s("Ddzs"),
        s("ng"),
        s("w"),
        s("v"),
        s("þ"),
        s("ð"),
        s("ı"),
        s("İ"),
        s("i\u{307}"),
        s("ơ"),
        s("ư\u{301}"),
        s("ǆ"),
        s("Ǆ"),
        s("ǅ"),
        s("ŉ"),
    ];
    // every combining class boundary near a contraction starter
    for m in [
        0x0300u32, 0x0316, 0x0334, 0x0345, 0x05B0, 0x0E38, 0x0F71, 0x1DCE, 0x302A,
    ] {
        v.push(vec![0x6C, m, 0xB7]);
        v.push(vec![0x0FB2, m, 0x0F80]);
        v.push(vec![0x0418, m, 0x0306]);
        v.push(vec![0x63, m, 0x68]);
    }
    // surrogates and out-of-range values (Perl strings may hold them)
    v.push(vec![0xD800]);
    v.push(vec![0x61, 0xDFFF, 0x62]);
    v.push(vec![0x110000]);
    v.push(vec![0x61, 0x7FFFFFFF]);
    v
}

/// Deterministic xorshift PRNG.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A pool of interesting code points for random strings.
pub fn base_pool() -> Vec<u32> {
    let mut p: Vec<u32> = Vec::new();
    p.extend(0x20..0x7F);
    p.extend([
        0x61, 0x65, 0x6F, 0x63, 0x68, 0x6C, 0x6E, 0x41, 0x45, 0x4C, 0x43,
    ]);
    p.extend(0xA0..0x180);
    p.extend((0x300..0x370).step_by(3));
    p.extend([
        0x0301, 0x0323, 0x0308, 0x030A, 0x0327, 0x0306, 0x031B, 0x0334, 0x0345,
    ]);
    p.extend([
        0x1AB0, 0x1DC0, 0x1DCE, 0x20D0, 0x302A, 0x3099, 0x309A, 0x093C, 0x05B0,
    ]);
    p.extend([
        0x0F71, 0x0F72, 0x0F74, 0x0F80, 0x0F81, 0x0FB2, 0x0FB3, 0x0F40,
    ]);
    p.extend((0x400..0x460).step_by(2));
    p.extend([
        0x0418, 0x0438, 0x0306, 0x0415, 0x0435, 0x0413, 0x0433, 0x0301,
    ]);
    p.extend([
        0x0E01, 0x0E40, 0x0E44, 0x0E38, 0x0E48, 0x0627, 0x0644, 0x0915, 0x093F,
    ]);
    p.extend([
        0x1100, 0x1101, 0x1161, 0x1162, 0x11A8, 0x11AB, 0xAC00, 0xAC01, 0xD7A3,
    ]);
    p.extend([
        0x3042, 0x30A2, 0x30A1, 0x309D, 0x30FC, 0xFF71, 0xFF21, 0x3131,
    ]);
    p.extend([
        0x4E00, 0x4E8C, 0x9FA5, 0x9FFC, 0x9FFD, 0x3400, 0x20000, 0x2A6DE, 0xF900,
    ]);
    p.extend([
        0x17000, 0x18800, 0x18D00, 0x1B170, 0x18B00, 0x1F600, 0x10FFFD, 0x378,
    ]);
    p.extend([
        0xFFFD, 0xFFFE, 0xFFFF, 0x0, 0xAD, 0x200B, 0x200D, 0x2D, 0x21, 0x20, 0x20,
    ]);
    p
}

pub fn random_strings(seed: u64, n: usize, pool: &[u32], maxlen: usize) -> Vec<Vec<u32>> {
    let mut r = Rng::new(seed);
    (0..n)
        .map(|_| {
            let len = 1 + r.below(maxlen);
            (0..len).map(|_| pool[r.below(pool.len())]).collect()
        })
        .collect()
}

// ---------------------------------------------------------------- oracle

/// The oracle perl (`UCOLLATE_PERL`, default: the O3 perl of the main checkout).
pub fn oracle_perl() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("UCOLLATE_PERL") {
        return Some(PathBuf::from(p));
    }
    let mut d = crate_dir();
    for _ in 0..6 {
        let p = d.join("vendor/perl/bin/perl");
        if p.exists() {
            return Some(p);
        }
        if !d.pop() {
            break;
        }
    }
    let p = PathBuf::from("/Users/ben/bibe-rs/vendor/perl/bin/perl");
    p.exists().then_some(p)
}

pub fn oracle_script() -> PathBuf {
    let mut d = crate_dir();
    d.pop();
    d.pop();
    d.join("tools/oracle/probes/ucollate/oracle.pl")
}

/// Run a batch of oracle requests, returning one response line per request.
pub fn run_oracle(requests: &str) -> Vec<String> {
    let perl = oracle_perl().expect("no oracle perl (set UCOLLATE_PERL)");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let req = dir.join(format!("ucollate-req-{}-{seq}.txt", std::process::id()));
    std::fs::write(&req, requests).unwrap();
    let out = std::process::Command::new(perl)
        .arg(oracle_script())
        .stdin(std::fs::File::open(&req).unwrap())
        .env_remove("PERL5LIB")
        .env_remove("PERL5OPT")
        .output()
        .expect("run oracle");
    std::fs::remove_file(&req).ok();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

pub fn hex_cps(s: &[u32]) -> String {
    s.iter()
        .map(|c| format!("{c:04X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        write!(s, "{x:02x}").unwrap();
    }
    s
}

/// Perl's sort keys (hex) for each input under `spec`.
pub fn perl_keys(spec: &Spec, inputs: &[Vec<u32>]) -> Vec<String> {
    let mut req = format!("C\t{}\n", spec.json());
    for s in inputs {
        req.push_str("K\t");
        req.push_str(&hex_cps(s));
        req.push('\n');
    }
    let mut resp = run_oracle(&req);
    assert_eq!(resp[0], "ok", "{}: {}", spec.name, resp[0]);
    resp.remove(0);
    assert_eq!(resp.len(), inputs.len());
    resp.into_iter()
        .map(|l| l.split('\t').next().unwrap().to_string())
        .collect()
}

pub fn rust_keys(c: &Collator, inputs: &[Vec<u32>]) -> Vec<String> {
    inputs
        .iter()
        .map(|s| hex(&c.sort_key_from_code_points(s)))
        .collect()
}

/// Compare Rust against Perl; returns the number of mismatches (printing the first few).
pub fn compare(spec: &Spec, inputs: &[Vec<u32>], expected: &[String]) -> usize {
    let c = spec.collator();
    let got = rust_keys(&c, inputs);
    let mut bad = 0;
    for ((s, e), g) in inputs.iter().zip(expected).zip(&got) {
        if e != g {
            if bad < 15 {
                eprintln!(
                    "MISMATCH {} [{}]\n  perl {}\n  rust {}",
                    spec.name,
                    hex_cps(s),
                    view_hex(e),
                    view_hex(g)
                );
            }
            bad += 1;
        }
    }
    if bad > 0 {
        eprintln!("{}: {bad}/{} mismatches", spec.name, inputs.len());
    }
    bad
}

pub fn view_hex(h: &str) -> String {
    let bytes: Vec<u8> = (0..h.len() / 2)
        .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    ucollate::view_key(&bytes)
}

/// md5 over the hex keys, one per line.
pub fn digest(keys: &[String]) -> String {
    let mut h = Md5::new();
    for k in keys {
        h.update(k.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

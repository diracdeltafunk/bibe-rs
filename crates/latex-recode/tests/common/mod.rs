//! Shared helpers for the differential tests: a deterministic RNG, generators
//! of TeX-ish (decode) and Unicode (encode) strings, and a driver for the O3
//! perl probe (`tools/oracle/probes/latex-recode/recode_probe.pl`).
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use latex_recode::RecodeData;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
}

/// Keys of the built-in data.
pub struct Keys {
    /// LaTeX-side names per type (all sets).
    pub from: Vec<(String, Vec<String>)>,
    /// Unicode-side strings (all types and sets).
    pub to: Vec<String>,
    /// Combining marks used as diacritics.
    pub marks: Vec<String>,
}

impl Keys {
    pub fn builtin() -> Keys {
        let raw = RecodeData::builtin().raw();
        let mut from: Vec<(String, Vec<String>)> = Vec::new();
        let mut to = Vec::new();
        let mut marks = Vec::new();
        for b in &raw.blocks {
            let entry = match from.iter_mut().position(|(t, _)| *t == b.ty) {
                Some(i) => &mut from[i].1,
                None => {
                    from.push((b.ty.clone(), Vec::new()));
                    &mut from.last_mut().unwrap().1
                }
            };
            for m in &b.maps {
                entry.push(m.from.clone());
                to.push(m.to.clone());
                if b.ty == "diacritics" {
                    marks.push(m.to.clone());
                }
            }
        }
        for c in raw.decode_exclude.iter().chain(&raw.encode_exclude) {
            from[0].1.push(c.clone());
        }
        Keys { from, to, marks }
    }

    pub fn of(&self, ty: &str) -> &[String] {
        &self.from.iter().find(|(t, _)| t == ty).unwrap().1
    }
}

const LETTERS: &[&str] = &[
    "a", "e", "i", "j", "o", "u", "A", "E", "I", "O", "C", "S", "c", "s", "z", "g", "n", "x", "y",
    "ı", "ȷ", "ß", "æ", "é", "e\u{301}", "ö", "Ł", "α", "Ж", "ŋ", "ǅ", "ﬁ", "\u{915}", "가",
];
const ODD: &[&str] = &[
    " ",
    " ",
    "  ",
    "\t",
    "\n",
    "\r\n",
    "\u{A0}",
    "\u{2028}",
    "\u{3000}",
    "\u{85}",
    "\x0b",
    "{",
    "}",
    "{}",
    "\\",
    "\\\\",
    "$",
    "%",
    "&",
    "#",
    "_",
    "^",
    "~",
    ",",
    ";",
    ".",
    ":",
    "-",
    "--",
    "---",
    "'",
    "\"",
    "`",
    "=",
    "1",
    "0",
    "9",
    "٣",
    "²",
    "\u{301}",
    "\u{308}",
    "\u{327}",
    "\u{338}",
    "\u{200D}",
    "\u{1F468}",
    "\u{1F1FA}",
    "\u{1F1F8}",
    "\x1f",
    "\x1e",
    "\x0f",
    "\x0e",
    "ı\u{301}",
    "\u{94D}",
    "\u{1100}",
    "\u{1161}",
    "\u{11A8}",
    "(",
    ")",
    "[",
    "]",
    "/",
    "*",
    "+",
    "<",
    ">",
    "\u{2013}",
    "\u{FFFD}",
    "\u{E000}",
    "\u{10FFFF}",
    "x",
    "ab",
    "word",
    "Text",
    "\u{903}",
];
const TERMS: &[&str] = &[
    "", "", "", "{}", " {}", "  {}", " ", "  ", "\t", "\n", "x", "X", "1", "_", ",", ".", ";",
    "\\", "{", "}", "{x}", "{\\i}", "-", "é", "\u{301}", "ab", " x", "{}{}", "\u{A0}", "$",
];

fn letter(r: &mut Rng) -> String {
    let mut s = r.pick(LETTERS).to_string();
    if r.chance(15) {
        s.push_str(r.pick(&["\u{301}", "\u{300}", "\u{308}", "\u{327}", "\u{303}\u{304}"]));
    }
    s
}

fn hex_digits(r: &mut Rng, min: usize, max: usize) -> String {
    let n = min + r.below(max - min + 1);
    (0..n)
        .map(|_| *r.pick(&['0', '1', '4', '7', '9', 'a', 'B', 'c', 'D', 'e', 'F', 'f']))
        .collect()
}

fn digits(r: &mut Rng, min: usize, max: usize) -> String {
    let n = min + r.below(max - min + 1);
    (0..n)
        .map(|_| *r.pick(&['0', '0', '1', '2', '3', '5', '7', '7', '8', '9', '٣', '６']))
        .collect()
}

fn arg(r: &mut Rng) -> String {
    match r.below(12) {
        0 => letter(r),
        1 => format!("{{{}}}", letter(r)),
        2 => format!(" {}", letter(r)),
        3 => format!("{{{}{}}}", letter(r), letter(r)),
        4 => "\\i".into(),
        5 => "{\\i}".into(),
        6 => "\\j".into(),
        7 => format!("{{{}}}}}", letter(r)),
        8 => r.pick(ODD).to_string(),
        9 => format!("  {{{}}}", letter(r)),
        10 => format!(
            "{{\\{} {}}}",
            r.pick(&["'", "\"", "c", "v", "i"]),
            letter(r)
        ),
        _ => String::new(),
    }
}

/// One TeX-ish fragment.
fn fragment(r: &mut Rng, k: &Keys) -> String {
    match r.below(26) {
        0..=3 => {
            // known macro of a random type + terminator
            let (ty, names) = r.pick(&k.from);
            let name = r.pick(names);
            let t = match ty.as_str() {
                "diacritics" => arg(r),
                _ => r.pick(TERMS).to_string(),
            };
            let pre = if r.chance(10) { "{" } else { "" };
            format!("{pre}\\{name}{t}")
        }
        4..=5 => {
            let name = r.pick(k.of("diacritics"));
            let ws = *r.pick(&["", "", " ", "  ", "\t"]);
            let open = if r.chance(25) { "{" } else { "" };
            let close = if r.chance(25) { "}" } else { "" };
            format!("{open}\\{name}{ws}{}{close}", arg(r))
        }
        6 => match r.below(4) {
            0 => format!("\\char\"{}", hex_digits(r, 1, 5)),
            1 => format!("\\char'{}", digits(r, 1, 4)),
            2 => format!("\\char{}", digits(r, 1, 4)),
            _ => format!(
                "\\char{}{}",
                r.pick(&["\"", "'", "", " "]),
                hex_digits(r, 0, 24)
            ),
        },
        7 => format!(
            "\\{}{}{{{}}}",
            r.pick(&["hbox", "box", "hbo", "mbox"]),
            r.pick(&["", " ", "  ", "\n"]),
            if r.chance(70) {
                letter(r)
            } else {
                r.pick(ODD).to_string()
            }
        ),
        8 => format!(
            "\\{}\\{}{}",
            r.pick(&["foo", "o", "ae", "i", "textbf", "L"]),
            r.pick(&[" ", "  ", "\n", "", "x"]),
            letter(r)
        ),
        9 => format!(
            "{}\\{}{}",
            r.pick(&["a", "{", " ", "\\", "", "é"]),
            letter(r),
            r.pick(&[",", ";", ".", ":", "%", "x", ""])
        ),
        10 => format!("\\not\\{}{}", r.pick(k.of("negatedsymbols")), r.pick(TERMS)),
        11 => format!(
            "\\textsuperscript{{{}{}}}",
            if r.chance(50) { "\\" } else { "" },
            if r.chance(50) {
                r.pick(k.of("superscripts")).to_string()
            } else {
                r.pick(k.of("cmdsuperscripts")).to_string()
            }
        ),
        12 => format!(
            "\\ding{{{}{}}}",
            r.pick(&['2', '3', '9', 'A', 'F', 'G', '1']),
            r.pick(&['0', '1', '5', 'A', 'E', 'F', 'g'])
        ),
        13..=14 => format!(
            "{{{}}}",
            if r.chance(60) {
                letter(r)
            } else {
                r.pick(ODD).to_string()
            }
        ),
        15 => format!(
            "\\{}{{{}}}{}",
            r.pick(&["frac", "textuppercase", "emph", "x", "ab"]),
            letter(r),
            if r.chance(50) {
                format!("{{{}}}", letter(r))
            } else {
                String::new()
            }
        ),
        16 => format!("${}$", fragment_simple(r, k)),
        17..=19 => r.pick(ODD).to_string(),
        20..=21 => letter(r),
        22 => format!(
            "\\{}",
            r.pick(&["foo", "bar", "&", "\\", "-", "|", "dots", "textbraceleft"])
        ),
        23 => format!("{{{}}}", fragment_simple(r, k)),
        24 => format!("\x1f{}\x1e", letter(r)),
        _ => "\\".to_string(),
    }
}

fn fragment_simple(r: &mut Rng, k: &Keys) -> String {
    let (_, names) = r.pick(&k.from);
    format!("\\{}{}", r.pick(names), r.pick(TERMS))
}

/// A random decode input.
pub fn gen_decode(r: &mut Rng, k: &Keys) -> String {
    let n = 1 + r.below(7);
    let mut s = String::new();
    for _ in 0..n {
        s.push_str(&fragment(r, k));
    }
    if r.chance(10) && !s.is_empty() {
        // delete a random char
        let idx: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
        let i = *r.pick(&idx);
        let c = s[i..].chars().next().unwrap();
        s.replace_range(i..i + c.len_utf8(), "");
    }
    s
}

/// A random string over a small, TeX-heavy alphabet (structural chaos).
pub fn gen_chaos(r: &mut Rng, k: &Keys) -> String {
    const TOK: &[&str] = &[
        "\\",
        "\\",
        "\\",
        "{",
        "{",
        "{",
        "}",
        "}",
        "}",
        " ",
        " ",
        "\t",
        "a",
        "e",
        "i",
        "c",
        "t",
        "u",
        "v",
        "o",
        "x",
        "S",
        "'",
        "\"",
        "^",
        "~",
        "=",
        ".",
        "`",
        ",",
        ";",
        "%",
        "1",
        "0",
        "\u{301}",
        "\u{327}",
        "ı",
        "é",
        "\x1f",
        "\x1e",
        "\x0f",
        "\x0e",
        "char",
        "not",
        "hbox",
        "box",
        "ding",
        "textsuperscript",
        "-",
        "--",
        "ae",
        "ss",
        "frac",
        "\u{A0}",
        "\n",
        "_",
        "$",
    ];
    let n = 1 + r.below(30);
    let mut s = String::new();
    for _ in 0..n {
        if r.chance(12) {
            let (_, names) = r.pick(&k.from);
            s.push_str(r.pick(names));
        } else {
            s.push_str(r.pick(TOK));
        }
    }
    s
}

/// A random encode input.
pub fn gen_encode(r: &mut Rng, k: &Keys) -> String {
    let n = 1 + r.below(8);
    let mut s = String::new();
    for _ in 0..n {
        match r.below(10) {
            0..=2 => s.push_str(r.pick(&k.to)),
            3..=4 => {
                if r.chance(20) {
                    s.push('{');
                }
                s.push_str(&letter(r));
                if r.chance(20) {
                    s.push('}');
                }
                for _ in 0..r.below(4) {
                    s.push_str(r.pick(&k.marks));
                }
            }
            5 => {
                s.push('i');
                s.push_str(r.pick(&k.marks));
            }
            6 => s.push_str(r.pick(ODD)),
            7 => s.push_str(r.pick(&k.marks)),
            8 => s.push_str(r.pick(&["Muḥammad", "Ðandđandć, H.", "°C", "Åå", "ı̆", "–", "÷", "ﬀ"])),
            _ => s.push_str(&letter(r)),
        }
    }
    if r.chance(30) {
        s = latex_recode::Normalization::Nfd.apply(&s);
    } else if r.chance(20) {
        s = latex_recode::Normalization::Nfc.apply(&s);
    }
    s
}

/// The O3 perl, if available.
pub fn oracle_paths() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let root = std::env::var("BIBERS_ROOT").unwrap_or_else(|_| "/Users/ben/bibe-rs".into());
    let root = Path::new(&root);
    let perl = root.join("vendor/perl/bin/perl");
    let lib = root.join("vendor/perl-build/biber-src/blib/lib");
    let probe = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/oracle/probes/latex-recode/recode_probe.pl");
    (perl.exists() && lib.exists()).then_some((perl, lib, probe))
}

/// Decode Perl's internal (lax) UTF-8: code points Rust cannot hold become
/// U+FFFD, as the crate documents.
pub fn lax_utf8(b: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let (len, init) = match c {
            0x00..=0x7F => (1, c as u64),
            0xC0..=0xDF => (2, (c & 0x1F) as u64),
            0xE0..=0xEF => (3, (c & 0x0F) as u64),
            0xF0..=0xF7 => (4, (c & 0x07) as u64),
            0xF8..=0xFB => (5, (c & 0x03) as u64),
            0xFC..=0xFD => (6, (c & 0x01) as u64),
            0xFE => (7, 0),
            _ => (13, 0),
        };
        let mut v = init;
        for j in 1..len {
            v = (v << 6) | (b.get(i + j).copied().unwrap_or(0x80) & 0x3F) as u64;
        }
        out.push(
            u32::try_from(v)
                .ok()
                .and_then(char::from_u32)
                .unwrap_or('\u{FFFD}'),
        );
        i += len;
    }
    out
}

/// Run a batch of probe requests. `init` is e.g. `"base base"`;
/// `reqs` are `(op, input)`. Returns `Ok(output)` or `Err(exception text)`.
pub fn run_oracle(init: &str, reqs: &[(&str, &str)]) -> Vec<Result<String, String>> {
    let (perl, lib, probe) = oracle_paths().expect("O3 perl not available");
    let mut child = Command::new(perl)
        .arg(format!("-I{}", lib.display()))
        .arg(probe)
        .env_remove("PERL5LIB")
        .env_remove("PERL5OPT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn perl");
    let mut stdin = child.stdin.take().unwrap();
    let mut input = format!("init {init}\n");
    for (op, s) in reqs {
        input.push_str(op);
        input.push(' ');
        for b in s.as_bytes() {
            input.push_str(&format!("{b:02x}"));
        }
        input.push('\n');
    }
    let writer = std::thread::spawn(move || {
        stdin.write_all(input.as_bytes()).unwrap();
    });
    let out = BufReader::new(child.stdout.take().unwrap());
    let mut lines = out.lines();
    assert_eq!(lines.next().unwrap().unwrap(), "ok");
    let mut res = Vec::with_capacity(reqs.len());
    for _ in reqs {
        let l = lines.next().expect("probe output").unwrap();
        let unhex = |h: &str| -> Vec<u8> {
            (0..h.len() / 2)
                .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
                .collect()
        };
        if let Some(e) = l.strip_prefix('!') {
            res.push(Err(lax_utf8(&unhex(e))));
        } else {
            res.push(Ok(lax_utf8(&unhex(&l))));
        }
    }
    writer.join().unwrap();
    child.wait().unwrap();
    res
}

pub fn esc(s: &str) -> String {
    s.escape_debug().to_string()
}

#[allow(unused)]
pub fn read_lines(p: &Path) -> Vec<String> {
    let f = std::fs::File::open(p).unwrap();
    BufReader::new(f).lines().map(|l| l.unwrap()).collect()
}

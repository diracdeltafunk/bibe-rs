//! `latex_decode`: a pass-by-pass reimplementation of `Recode.pm:201-347`.
//!
//! Each `s///g` of the Perl code is one function below, written as a scanner
//! that finds exactly the matches Perl's leftmost/backtracking regex engine
//! finds, in the same order, against the same (pre-substitution) subject. The
//! comment above each function gives the Perl substitution and the reasoning
//! for equivalence where it is not obvious.

use memchr::{memchr, memchr2, memrchr};

use crate::RecodeOptions;
use crate::tables::{DecodeTables, KeyMap, Ty};
use crate::unicode::{grapheme_end, is_digit, is_letter, is_mark, is_space, is_word};

/// Accumulates the output of one `s///g`: unchanged runs of the subject are
/// copied lazily.
struct Rw<'a> {
    src: &'a str,
    out: &'a mut String,
    last: usize,
    changed: bool,
}

impl<'a> Rw<'a> {
    fn new(src: &'a str, out: &'a mut String) -> Self {
        out.clear();
        Rw {
            src,
            out,
            last: 0,
            changed: false,
        }
    }

    /// Replace `src[start..end]`; returns the output to append the
    /// replacement to.
    #[inline]
    fn replace(&mut self, start: usize, end: usize) -> &mut String {
        self.out.push_str(&self.src[self.last..start]);
        self.last = end;
        self.changed = true;
        self.out
    }

    fn finish(self) -> bool {
        if self.changed {
            self.out.push_str(&self.src[self.last..]);
        }
        self.changed
    }
}

#[inline]
fn char_at(s: &str, i: usize) -> Option<char> {
    s.get(i..).and_then(|r| r.chars().next())
}

#[inline]
fn char_before(s: &str, i: usize) -> Option<char> {
    s[..i].chars().next_back()
}

/// End of the maximal run of Perl `\s` starting at `i`.
#[inline]
fn skip_space(s: &str, mut i: usize) -> usize {
    let b = s.as_bytes();
    while i < b.len() {
        let c = b[i];
        if c < 0x80 {
            if matches!(c, b'\t' | b'\n' | 0x0B | 0x0C | b'\r' | b' ') {
                i += 1;
                continue;
            }
            break;
        }
        let ch = char_at(s, i).unwrap();
        if is_space(ch) {
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    i
}

/// End of the maximal run of `\pM` starting at `i`.
#[inline]
fn skip_marks(s: &str, mut i: usize) -> usize {
    while let Some(c) = char_at(s, i) {
        if is_mark(c) {
            i += c.len_utf8();
        } else {
            break;
        }
    }
    i
}

/// Perl `\b` at byte offset `i`.
#[inline]
fn word_boundary(s: &str, i: usize) -> bool {
    let before = char_before(s, i).is_some_and(is_word);
    let after = char_at(s, i).is_some_and(is_word);
    before != after
}

/// `s/\\(h?box)\s+\{(\X)\}/\\$1\{$2\}/g`
fn hbox(src: &str, out: &mut String) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr(b'\\', &b[pos..]) {
        let i = pos + k;
        pos = i + 1;
        let rest = &b[i + 1..];
        // `h?` is greedy; if "hbox" fails later, "box" cannot match at an `h`.
        let name_len = if rest.starts_with(b"hbox") {
            4
        } else if rest.starts_with(b"box") {
            3
        } else {
            continue;
        };
        let ws = i + 1 + name_len;
        let j = skip_space(src, ws);
        if j == ws || b.get(j) != Some(&b'{') || j + 1 >= b.len() {
            continue;
        }
        // `\X` is a single atomic cluster.
        let ce = grapheme_end(src, j + 1);
        if b.get(ce) != Some(&b'}') {
            continue;
        }
        let o = rw.replace(i, ce + 1);
        o.push('\\');
        o.push_str(&src[i + 1..ws]);
        o.push('{');
        o.push_str(&src[j + 1..ce]);
        o.push('}');
        pos = ce + 1;
    }
    rw.finish()
}

const IV_MAX: u128 = i64::MAX as u128;

/// Perl `chr()` of an integer code point, as stored in a Rust string.
///
/// Perl dies for values above `IV_MAX` (the `/ee` eval then yields undef,
/// i.e. the empty string). Smaller values that are not Unicode scalar values
/// (surrogates, > U+10FFFF) are representable in Perl but not in Rust; they
/// become U+FFFD (a documented deviation).
fn push_chr(o: &mut String, v: u128) {
    if v <= IV_MAX {
        o.push(
            u32::try_from(v)
                .ok()
                .and_then(char::from_u32)
                .unwrap_or('\u{FFFD}'),
        );
    }
}

/// Parse `digits` (ASCII) in `radix`, saturating above `IV_MAX`.
fn parse_radix(digits: &[u8], radix: u32) -> Option<u128> {
    let mut v: u128 = 0;
    for &d in digits {
        let x = (d as char).to_digit(radix)?;
        v = (v * radix as u128 + x as u128).min(IV_MAX + 1);
    }
    Some(v)
}

/// The value Perl's `eval "chr(0$digits)"` / `eval "chr($digits)"` yields.
/// `digits` is a non-empty `\d+` match (possibly non-ASCII digits).
fn eval_chr(digits: &str, force_octal: bool) -> Option<u128> {
    if !digits.is_ascii() {
        // Non-ASCII digits are a syntax error in Perl source.
        return None;
    }
    let d = digits.as_bytes();
    if force_octal || (d[0] == b'0' && d.len() > 1) {
        // "Illegal octal digit" is fatal.
        parse_radix(d, 8)
    } else {
        parse_radix(d, 10)
    }
}

/// End of the maximal run of Perl `\d` starting at `i`.
fn skip_digits(s: &str, mut i: usize) -> usize {
    while let Some(c) = char_at(s, i) {
        if is_digit(c) {
            i += c.len_utf8();
        } else {
            break;
        }
    }
    i
}

/// `s/\\char"(\p{ASCII_Hex_Digit}+)/"chr(0x$1)"/gee`
/// `s/\\char'(\d+)/"chr(0$1)"/gee`
/// `s/\\char(\d+)/"chr($1)"/gee`
#[derive(Clone, Copy, PartialEq, Eq)]
enum CharForm {
    Hex,
    Octal,
    Decimal,
}

fn char_macro(src: &str, out: &mut String, form: CharForm) -> bool {
    let b = src.as_bytes();
    let prefix: &[u8] = match form {
        CharForm::Hex => b"char\"",
        CharForm::Octal => b"char'",
        CharForm::Decimal => b"char",
    };
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr(b'\\', &b[pos..]) {
        let i = pos + k;
        pos = i + 1;
        if !b[i + 1..].starts_with(prefix) {
            continue;
        }
        let j = i + 1 + prefix.len();
        let (end, val) = match form {
            CharForm::Hex => {
                let e = j + b[j..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
                (e, parse_radix(&b[j..e], 16))
            }
            CharForm::Octal | CharForm::Decimal => {
                let e = skip_digits(src, j);
                (e, eval_chr(&src[j..e], form == CharForm::Octal))
            }
        };
        if end == j {
            continue;
        }
        let o = rw.replace(i, end);
        if let Some(v) = val {
            push_chr(o, v);
        }
        pos = end;
    }
    rw.finish()
}

/// `s/(\\[a-zA-Z]+)\\(\s+)/$1\{\}$2/g`
///
/// `[a-zA-Z]+` is followed by `\`, so only the maximal run can match.
fn control_space(src: &str, out: &mut String) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr(b'\\', &b[pos..]) {
        let i = pos + k;
        pos = i + 1;
        let l = b[i + 1..].iter().take_while(|c| c.is_ascii_alphabetic()).count();
        let bs = i + 1 + l;
        if l == 0 || b.get(bs) != Some(&b'\\') {
            continue;
        }
        let m = skip_space(src, bs + 1);
        if m == bs + 1 {
            continue;
        }
        let o = rw.replace(i, m);
        o.push_str(&src[i..bs]);
        o.push_str("{}");
        o.push_str(&src[bs + 1..m]);
        pos = m;
    }
    rw.finish()
}

/// `s/([^{]\\\w)([;,.:%])/$1\{\}$2/g`
///
/// A match starts at the character *before* a backslash, so after a match
/// ending at `e` the next backslash must be at least one character past `e`.
fn punct_brace(src: &str, out: &mut String) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    // Search for a backslash from here.
    let mut from = match char_at(src, 0) {
        Some(c) => c.len_utf8(),
        None => return false,
    };
    while from < b.len() {
        let Some(k) = memchr(b'\\', &b[from..]) else {
            break;
        };
        let i = from + k;
        from = i + 1;
        let prev = char_before(src, i).unwrap();
        if prev == '{' {
            continue;
        }
        let Some(w) = char_at(src, i + 1) else {
            continue;
        };
        if !is_word(w) {
            continue;
        }
        let pi = i + 1 + w.len_utf8();
        let Some(&p) = b.get(pi) else {
            continue;
        };
        if !matches!(p, b';' | b',' | b'.' | b':' | b'%') {
            continue;
        }
        let h = i - prev.len_utf8();
        let o = rw.replace(h, pi + 1);
        o.push_str(&src[h..pi]);
        o.push_str("{}");
        o.push(p as char);
        let e = pi + 1;
        from = e + char_at(src, e).map_or(1, char::len_utf8);
    }
    rw.finish()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Trail {
    /// `(?: \{\}|\s+|\b)` (punctuation, symbols, greek; BB-029b)
    SpaceBraces,
    /// `(?:\{\}|\s+|\b)` (letters)
    Braces,
}

/// `s/\\($re)(?: \{\}|\s+|\b)/$map->{$1}/ge` and the letters variant.
///
/// `$re` is the keys sorted by descending length; the engine tries each key
/// that is a prefix of the text in that order until the trailing group
/// matches. The trailing group ends the regex, so its first matching
/// alternative is taken (with `\s+` maximal).
fn macros(src: &str, out: &mut String, km: &KeyMap, trail: Trail, cands: &mut Vec<(usize, u32)>) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    let empty = km.trie.has_empty();
    while let Some(k) = memchr(b'\\', &b[pos..]) {
        let i = pos + k;
        pos = i + 1;
        if !empty && !b.get(i + 1).is_some_and(|&c| km.trie.starts_with_byte(c)) {
            continue;
        }
        km.trie.prefixes(b, i + 1, cands);
        for &(e, v) in cands.iter().rev() {
            let braces: &[u8] = match trail {
                Trail::SpaceBraces => b" {}",
                Trail::Braces => b"{}",
            };
            let end = if b[e..].starts_with(braces) {
                e + braces.len()
            } else {
                let ws = skip_space(src, e);
                if ws > e {
                    ws
                } else if word_boundary(src, e) {
                    e
                } else {
                    continue;
                }
            };
            rw.replace(i, end).push_str(&km.values[v as usize]);
            pos = end;
            break;
        }
    }
    rw.finish()
}

/// Longest key that is a prefix at `j` and is followed by `close`.
fn longest_key_then(b: &[u8], j: usize, km: &KeyMap, close: &[u8], cands: &mut Vec<(usize, u32)>) -> Option<(usize, u32)> {
    km.trie.prefixes(b, j, cands);
    cands.iter().rev().find(|&&(e, _)| b[e..].starts_with(close)).copied()
}

/// `s/\\not\\($re)/$map->{$1}/ge`,
/// `s/\\textsuperscript\{($re)\}/$map->{$1}/ge`,
/// `s/\\textsuperscript\{\\($re)\}/$map->{$1}/ge`.
fn prefixed(src: &str, out: &mut String, km: &KeyMap, prefix: &[u8], close: &[u8], cands: &mut Vec<(usize, u32)>) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr(b'\\', &b[pos..]) {
        let i = pos + k;
        pos = i + 1;
        if !b[i + 1..].starts_with(prefix) {
            continue;
        }
        let j = i + 1 + prefix.len();
        if let Some((e, v)) = longest_key_then(b, j, km, close, cands) {
            let end = e + close.len();
            rw.replace(i, end).push_str(&km.values[v as usize]);
            pos = end;
        }
    }
    rw.finish()
}

/// `s/\\ding\{([2-9AF][0-9A-F])\}/$map->{$1}/ge` (unknown codes vanish).
fn dings(src: &str, out: &mut String, km: &KeyMap) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr(b'\\', &b[pos..]) {
        let i = pos + k;
        pos = i + 1;
        let r = &b[i + 1..];
        if r.len() >= 8
            && r.starts_with(b"ding{")
            && matches!(r[5], b'2'..=b'9' | b'A' | b'F')
            && matches!(r[6], b'0'..=b'9' | b'A'..=b'F')
            && r[7] == b'}'
        {
            let o = rw.replace(i, i + 9);
            if let Some(v) = km.trie.get(&r[5..7]) {
                o.push_str(&km.values[v as usize]);
            }
            pos = i + 9;
        }
    }
    rw.finish()
}

/// Is `src[..p]` ending in `\` + one of the diacritic keys?
/// (Emulated look-behind `(?<!\\$re)` of the first diacritics substitution.)
#[inline]
fn ends_with_diacritic_macro(b: &[u8], p: usize, km: &KeyMap) -> bool {
    km.key_lens.iter().any(|&l| {
        p > l && b[p - l - 1] == b'\\' && km.trie.get(&b[p - l..p]).is_some()
    })
}

/// D1: `s/(?<!\\$re)\{(\X)\}/\x{f}$1\x{e}/g` (written with a variable-length
/// look-behind emulation in `Recode.pm:260`).
fn protect_single(src: &str, out: &mut String, km: &KeyMap) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr(b'{', &b[pos..]) {
        let p = pos + k;
        pos = p + 1;
        if p + 1 >= b.len() {
            break;
        }
        let ce = grapheme_end(src, p + 1);
        if b.get(ce) != Some(&b'}') || ends_with_diacritic_macro(b, p, km) {
            continue;
        }
        let o = rw.replace(p, ce + 1);
        o.push('\x0f');
        o.push_str(&src[p + 1..ce]);
        o.push('\x0e');
        pos = ce + 1;
    }
    rw.finish()
}

/// D2: `s/(\{?)\\($re)\s*\{(\pL\pM*)\}(\}?)/$bracemap->{$1} . $3 . $map->{$2} . $bracemap->{$4}/ge`
fn accent_braced(src: &str, out: &mut String, km: &KeyMap, cands: &mut Vec<(usize, u32)>) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr(b'\\', &b[pos..]) {
        let i = pos + k;
        pos = i + 1;
        km.trie.prefixes(b, i + 1, cands);
        let mut found = None;
        for &(e, v) in cands.iter().rev() {
            let j = skip_space(src, e);
            if b.get(j) != Some(&b'{') {
                continue;
            }
            let Some(l) = char_at(src, j + 1) else {
                continue;
            };
            if !is_letter(l) {
                continue;
            }
            let ls = j + 1;
            let m = skip_marks(src, ls + l.len_utf8());
            if b.get(m) != Some(&b'}') {
                continue;
            }
            found = Some((v, ls, m));
            break;
        }
        let Some((v, ls, m)) = found else {
            continue;
        };
        // A match starting at a preceding `{` (not consumed by the previous
        // match) is further left, and its remainder is exactly this one.
        let opened = i > rw.last && b[i - 1] == b'{';
        let start = if opened { i - 1 } else { i };
        let mut end = m + 1;
        let closed = b.get(end) == Some(&b'}');
        if closed {
            end += 1;
        }
        let o = rw.replace(start, end);
        if opened {
            o.push('\x1f');
        }
        o.push_str(&src[ls..m]);
        o.push_str(&km.values[v as usize]);
        if closed {
            o.push('\x1e');
        }
        pos = end;
    }
    rw.finish()
}

/// D3: `s/(\{)(\pL\pM*)(\})/\x{1f}$2\x{1e}/g`
fn letter_braced(src: &str, out: &mut String) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr(b'{', &b[pos..]) {
        let p = pos + k;
        pos = p + 1;
        let Some(l) = char_at(src, p + 1) else {
            break;
        };
        if !is_letter(l) {
            continue;
        }
        let m = skip_marks(src, p + 1 + l.len_utf8());
        if b.get(m) != Some(&b'}') {
            continue;
        }
        let o = rw.replace(p, m + 1);
        o.push('\x1f');
        o.push_str(&src[p + 1..m]);
        o.push('\x1e');
        pos = m + 1;
    }
    rw.finish()
}

/// D4 (`Recode.pm:277-292`):
///
/// ```text
/// s/\\($re)(\s*)((?(?{$1 !~ m:[A-Za-z]$:})\pL|(?(?{$2})\pL|[^A-Za-z]))\pM*)/$3 . $map->{$1}/gxe
/// ```
///
/// For each key (longest first) the engine backtracks `\s*` from maximal to
/// empty, re-evaluating the code conditions:
/// - key not ending in `[A-Za-z]`: `\pL` must follow; whitespace is never
///   `\pL`, so only the maximal whitespace run can succeed;
/// - key ending in a letter, whitespace present: `\pL` after the maximal run,
///   else (with `$2` now empty) `[^A-Za-z]` matches the first whitespace char;
/// - key ending in a letter, no whitespace: `[^A-Za-z]` (any char).
fn accent_bare(src: &str, out: &mut String, km: &KeyMap, cands: &mut Vec<(usize, u32)>) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr(b'\\', &b[pos..]) {
        let i = pos + k;
        pos = i + 1;
        km.trie.prefixes(b, i + 1, cands);
        let mut found = None;
        for &(e, v) in cands.iter().rev() {
            let ws = skip_space(src, e);
            let x = if !km.ends_ascii_letter[v as usize] {
                match char_at(src, ws) {
                    Some(c) if is_letter(c) => ws,
                    _ => continue,
                }
            } else if ws > e {
                match char_at(src, ws) {
                    Some(c) if is_letter(c) => ws,
                    _ => e,
                }
            } else {
                match char_at(src, e) {
                    Some(c) if !c.is_ascii_alphabetic() => e,
                    _ => continue,
                }
            };
            found = Some((v, x));
            break;
        }
        let Some((v, x)) = found else {
            continue;
        };
        let c = char_at(src, x).unwrap();
        let m = skip_marks(src, x + c.len_utf8());
        let o = rw.replace(i, m);
        o.push_str(&src[x..m]);
        o.push_str(&km.values[v as usize]);
        pos = m;
    }
    rw.finish()
}

/// Does some suffix of `src[..p]` match `\\\pL+(?:\{[^{]+\})*`?
///
/// Parsed backwards, the match is unique: a trailing `}` must close a group
/// whose `{` is the last `{` before it (groups contain no `{`) and whose
/// interior is non-empty; otherwise the suffix must be a maximal run of
/// letters preceded by `\`.
fn ends_with_macro_args(src: &str, p: usize) -> bool {
    let b = src.as_bytes();
    let mut pos = p;
    loop {
        if pos == 0 {
            return false;
        }
        if b[pos - 1] == b'}' {
            let Some(s) = memrchr(b'{', &b[..pos - 1]) else {
                return false;
            };
            if s + 2 >= pos {
                return false;
            }
            pos = s;
            continue;
        }
        let mut k = pos;
        while let Some(c) = char_before(src, k) {
            if is_letter(c) {
                k -= c.len_utf8();
            } else {
                break;
            }
        }
        return k < pos && k > 0 && b[k - 1] == b'\\';
    }
}

/// Brace elimination (`Recode.pm:321`):
/// `s/(?<!\\\pL+(?:\{[^{]+\})*)[{\x{1f}](\X)[}\x{1e}]/$1/g`
/// (the look-behind is emulated in Perl with recursion; see
/// [`ends_with_macro_args`]).
fn eliminate_braces(src: &str, out: &mut String) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr2(b'{', 0x1f, &b[pos..]) {
        let p = pos + k;
        pos = p + 1;
        if p + 1 >= b.len() {
            break;
        }
        let ce = grapheme_end(src, p + 1);
        if !matches!(b.get(ce), Some(b'}' | 0x1e)) || ends_with_macro_args(src, p) {
            continue;
        }
        rw.replace(p, ce + 1).push_str(&src[p + 1..ce]);
        pos = ce + 1;
    }
    rw.finish()
}

/// Restore brace markers (`\x1f \x0f` → `{`, `\x1e \x0e` → `}`) and apply
/// `s/ı́/í/g` (U+0131 U+0301 → U+00ED).
fn finalize(src: &str, out: &mut String) -> bool {
    let needs = src.bytes().any(|c| matches!(c, 0x0e | 0x0f | 0x1e | 0x1f))
        || src.contains("\u{131}\u{301}");
    if !needs {
        return false;
    }
    out.clear();
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\x1f' | '\x0f' => out.push('{'),
            '\x1e' | '\x0e' => out.push('}'),
            '\u{131}' if it.peek() == Some(&'\u{301}') => {
                it.next();
                out.push('\u{ED}');
            }
            _ => out.push(c),
        }
    }
    true
}

/// Could any decode pass change `s`?
#[inline]
pub(crate) fn needs_work(s: &str) -> bool {
    s.bytes()
        .any(|c| matches!(c, b'\\' | b'{' | 0x0e | 0x0f | 0x1e | 0x1f))
        || s.contains("\u{131}\u{301}")
}

/// `latex_decode` without the final normalisation.
pub(crate) fn decode_raw(t: &DecodeTables, opts: &RecodeOptions, input: &str) -> String {
    if !needs_work(input) {
        return input.to_owned();
    }
    let mut a = input.to_owned();
    let mut b = String::with_capacity(input.len() + 16);
    let mut cands = Vec::with_capacity(16);

    macro_rules! run {
        ($f:expr) => {
            if $f(a.as_str(), &mut b) {
                std::mem::swap(&mut a, &mut b);
            }
        };
    }
    let has_bs = |s: &str| memchr(b'\\', s.as_bytes()).is_some();

    if has_bs(&a) {
        run!(hbox);
        run!(|s, o| char_macro(s, o, CharForm::Hex));
        run!(|s, o| char_macro(s, o, CharForm::Octal));
        run!(|s, o| char_macro(s, o, CharForm::Decimal));
        run!(control_space);
        run!(punct_brace);
    }
    let space_braces = if opts.fix_space_braces {
        Trail::Braces
    } else {
        Trail::SpaceBraces
    };
    for ty in [
        Ty::Greek,
        Ty::Dings,
        Ty::Punctuation,
        Ty::Symbols,
        Ty::NegatedSymbols,
        Ty::Superscripts,
        Ty::CmdSuperscripts,
        Ty::Letters,
        Ty::Diacritics,
    ] {
        let Some(km) = t.get(ty) else { continue };
        if ty == Ty::Diacritics {
            run!(|s, o| protect_single(s, o, km));
            if has_bs(&a) {
                run!(|s, o| accent_braced(s, o, km, &mut cands));
            }
            run!(letter_braced);
            if has_bs(&a) {
                run!(|s, o| accent_bare(s, o, km, &mut cands));
            }
            continue;
        }
        if !has_bs(&a) {
            continue;
        }
        match ty {
            Ty::NegatedSymbols => run!(|s, o| prefixed(s, o, km, b"not\\", b"", &mut cands)),
            Ty::Superscripts => {
                run!(|s, o| prefixed(s, o, km, b"textsuperscript{", b"}", &mut cands))
            }
            Ty::CmdSuperscripts => {
                run!(|s, o| prefixed(s, o, km, b"textsuperscript{\\", b"}", &mut cands))
            }
            Ty::Dings => run!(|s, o| dings(s, o, km)),
            Ty::Letters => run!(|s, o| macros(s, o, km, Trail::Braces, &mut cands)),
            _ => run!(|s, o| macros(s, o, km, space_braces, &mut cands)),
        }
    }
    run!(eliminate_braces);
    run!(finalize);
    a
}

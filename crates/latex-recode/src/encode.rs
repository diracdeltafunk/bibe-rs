//! `latex_encode`: a pass-by-pass reimplementation of `Recode.pm:355-424`.
//!
//! The encode alternations are the keys sorted with Perl's `sort` (code point
//! order) with no trailing context, so at a given position the first
//! alternative that matches is the *shortest* key that is a prefix there.

use crate::tables::{EncodeTables, KeyMap, Ty};
use crate::unicode::{is_letter, is_mark};

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

#[derive(Clone, Copy)]
enum Fmt {
    /// `_wrap` (greek, punctuation, symbols)
    Wrap,
    /// `\ding{…}`
    Ding,
    /// `{$\not\…$}`
    Not,
    /// `\textsuperscript{…}`
    Sup,
    /// `\textsuperscript{\…}`
    CmdSup,
    /// letters: raw, or `\…{}`
    Letter,
}

fn format(o: &mut String, fmt: Fmt, key: &str, v: &str, t: &EncodeTables) {
    match fmt {
        Fmt::Wrap => {
            if v.starts_with("text") || v.starts_with("guil") {
                o.push('\\');
                o.push_str(v);
                o.push_str("{}");
            } else if t.raw.contains(key) {
                o.push_str(v);
            } else {
                o.push_str("{$\\");
                o.push_str(v);
                o.push_str("$}");
            }
        }
        Fmt::Ding => {
            o.push_str("\\ding{");
            o.push_str(v);
            o.push('}');
        }
        Fmt::Not => {
            o.push_str("{$\\not\\");
            o.push_str(v);
            o.push_str("$}");
        }
        Fmt::Sup => {
            o.push_str("\\textsuperscript{");
            o.push_str(v);
            o.push('}');
        }
        Fmt::CmdSup => {
            o.push_str("\\textsuperscript{\\");
            o.push_str(v);
            o.push('}');
        }
        Fmt::Letter => {
            if t.raw.contains(key) {
                o.push_str(v);
            } else {
                o.push('\\');
                o.push_str(v);
                o.push_str("{}");
            }
        }
    }
}

/// `s/($re)/…/ge` with no context.
fn simple(src: &str, out: &mut String, km: &KeyMap, fmt: Fmt, t: &EncodeTables) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut i = 0;
    // A key's first byte is never a UTF-8 continuation byte, so testing
    // every byte offset is the same as testing every char boundary.
    while i < b.len() {
        if km.trie.starts_with_byte(b[i])
            && let Some((e, v)) = km.trie.shortest_prefix(b, i)
        {
            let v = v as usize;
            format(rw.replace(i, e), fmt, &km.keys[v], &km.values[v], t);
            i = e;
            continue;
        }
        i += 1;
    }
    rw.finish()
}

/// E1: `s/i($re)/"\\" . $map->{$1} . '{\i}'/ge`
fn dotless_i(src: &str, out: &mut String, km: &KeyMap) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr::memchr(b'i', &b[pos..]) {
        let i = pos + k;
        pos = i + 1;
        if let Some((e, v)) = km.trie.shortest_prefix(b, i + 1) {
            let o = rw.replace(i, e);
            o.push('\\');
            o.push_str(&km.values[v as usize]);
            o.push_str("{\\i}");
            pos = e;
        }
    }
    rw.finish()
}

/// E2: `s/\{(\pL\pM*)\}($re)/"\\" . $map->{$2} . "{$1}"/ge`
fn braced_letter(src: &str, out: &mut String, km: &KeyMap) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut pos = 0;
    while let Some(k) = memchr::memchr(b'{', &b[pos..]) {
        let p = pos + k;
        pos = p + 1;
        let Some(l) = char_at(src, p + 1) else {
            break;
        };
        if !is_letter(l) {
            continue;
        }
        let mut m = p + 1 + l.len_utf8();
        while let Some(c) = char_at(src, m).filter(|&c| is_mark(c)) {
            m += c.len_utf8();
        }
        if b.get(m) != Some(&b'}') {
            continue;
        }
        if let Some((e, v)) = km.trie.shortest_prefix(b, m + 1) {
            let o = rw.replace(p, e);
            o.push('\\');
            o.push_str(&km.values[v as usize]);
            o.push('{');
            o.push_str(&src[p + 1..m]);
            o.push('}');
            pos = e;
        }
    }
    rw.finish()
}

/// E3: `s/(\pL\pM*)($re)/"\\" . $map->{$2} . "{$1}"/ge`
///
/// `\pM*` backtracks from maximal: the key is tried after all marks, then
/// after one fewer, and so on.
fn letter_marks(src: &str, out: &mut String, km: &KeyMap, ends: &mut Vec<usize>) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut i = 0;
    while let Some(c) = char_at(src, i) {
        if !is_letter(c) {
            i += c.len_utf8();
            continue;
        }
        // Possible ends of `\pL\pM*`, shortest first.
        ends.clear();
        let mut m = i + c.len_utf8();
        ends.push(m);
        while let Some(mc) = char_at(src, m).filter(|&mc| is_mark(mc)) {
            m += mc.len_utf8();
            ends.push(m);
        }
        let hit = ends
            .iter()
            .rev()
            .find_map(|&j| km.trie.shortest_prefix(b, j).map(|kv| (j, kv)));
        match hit {
            Some((j, (e, v))) => {
                let o = rw.replace(i, e);
                o.push('\\');
                o.push_str(&km.values[v as usize]);
                o.push('{');
                o.push_str(&src[i..j]);
                o.push('}');
                i = e;
            }
            None => i += c.len_utf8(),
        }
    }
    rw.finish()
}

/// `n` consecutive keys starting at `pos`, trying alternatives in sorted
/// (increasing length) order with backtracking. Fills `keys` with value ids.
fn key_chain(b: &[u8], pos: usize, n: usize, km: &KeyMap, keys: &mut Vec<u32>) -> Option<usize> {
    if n == 0 {
        return Some(pos);
    }
    let mut cands = Vec::new();
    km.trie.prefixes(b, pos, &mut cands);
    for (e, v) in cands {
        keys.push(v);
        if let Some(end) = key_chain(b, e, n - 1, km, keys) {
            return Some(end);
        }
        keys.pop();
    }
    None
}

/// E4–E6: `s/(\PM)($re){n}/\m_n{…\m_1{c}…}/gex` for n = 3, 2, 1.
fn stacked(src: &str, out: &mut String, km: &KeyMap, n: usize) -> bool {
    let b = src.as_bytes();
    let mut rw = Rw::new(src, out);
    let mut i = 0;
    let mut keys = Vec::with_capacity(n);
    while let Some(c) = char_at(src, i) {
        let after = i + c.len_utf8();
        if !is_mark(c) && after < b.len() && km.trie.starts_with_byte(b[after]) {
            keys.clear();
            if let Some(e) = key_chain(b, after, n, km, &mut keys) {
                let o = rw.replace(i, e);
                for &v in keys.iter().rev() {
                    o.push('\\');
                    o.push_str(&km.values[v as usize]);
                    o.push('{');
                }
                o.push_str(&src[i..after]);
                for _ in 0..n {
                    o.push('}');
                }
                i = e;
                continue;
            }
        }
        i = after;
    }
    rw.finish()
}

pub(crate) fn encode(t: &EncodeTables, input: &str) -> String {
    if t.null {
        return input.to_owned();
    }
    let mut a = input.to_owned();
    let mut b = String::with_capacity(input.len() + 16);
    let mut ends = Vec::new();
    macro_rules! run {
        ($f:expr) => {
            if $f(a.as_str(), &mut b) {
                std::mem::swap(&mut a, &mut b);
            }
        };
    }
    for ty in [
        Ty::Greek,
        Ty::Dings,
        Ty::NegatedSymbols,
        Ty::Superscripts,
        Ty::CmdSuperscripts,
        Ty::Diacritics,
        Ty::Letters,
        Ty::Punctuation,
        Ty::Symbols,
    ] {
        let Some(km) = t.get(ty) else { continue };
        let fmt = match ty {
            Ty::NegatedSymbols => Fmt::Not,
            Ty::Superscripts => Fmt::Sup,
            Ty::CmdSuperscripts => Fmt::CmdSup,
            Ty::Dings => Fmt::Ding,
            Ty::Letters => Fmt::Letter,
            Ty::Diacritics => {
                run!(|s, o| dotless_i(s, o, km));
                run!(|s, o| braced_letter(s, o, km));
                run!(|s, o| letter_marks(s, o, km, &mut ends));
                run!(|s, o| stacked(s, o, km, 3));
                run!(|s, o| stacked(s, o, km, 2));
                run!(|s, o| stacked(s, o, km, 1));
                continue;
            }
            _ => Fmt::Wrap,
        };
        run!(|s, o| simple(s, o, km, fmt, t));
    }
    a
}

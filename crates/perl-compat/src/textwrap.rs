//! `Text::Wrap::wrap` 2024.001 (perl 5.42 core, with `Text::Tabs`
//! `expand`/`unexpand`), at its default settings (`$huge = 'wrap'`,
//! `$unexpand = 1`, `$tabstop = 8`, `$separator = "\n"`). biber uses it for
//! `--wraplines` (bbl.pm:135-146).
//!
//! Text is measured in "units" of a non-mark character followed by its
//! combining marks (`\PM\pM*`), as `Text::Wrap` does.
//!
//! ```
//! use perl_compat::textwrap::wrap;
//! assert_eq!(wrap(20, "  ", "  ", "aaa bbb ccc ddd eee fff"), "  aaa bbb ccc ddd\n  eee fff");
//! ```

use crate::unicode::Prop;

fn is_mark(c: char) -> bool {
    Prop::M.contains(c)
}

/// `$brkspc`: whitespace other than U+00A0 and U+202F.
fn is_brkspc(c: char) -> bool {
    Prop::PerlSpace.contains(c) && c != '\u{a0}' && c != '\u{202f}'
}

/// Number of `\PM` characters plus one if the string starts with a mark
/// (`_xlen`, and the offset computation of `expand`).
fn xlen(s: &[char]) -> usize {
    usize::from(s.first().is_some_and(|c| is_mark(*c))) + s.iter().filter(|c| !is_mark(**c)).count()
}

/// `Text::Tabs::expand` (tabstop 8) for one string.
pub fn expand(s: &str) -> String {
    let mut out = String::new();
    // split(/^/m, $_, -1): lines keeping their "\n"
    for line in s.split_inclusive('\n') {
        let mut offs: Option<usize> = None;
        for part in line.split('\t') {
            if let Some(o) = offs {
                let pad = 8 - o % 8;
                out.extend(std::iter::repeat_n(' ', pad));
            }
            out.push_str(part);
            // the length of the segment, not the column (same modulo 8)
            offs = Some(xlen(&part.chars().collect::<Vec<_>>()));
        }
    }
    out
}

/// `Text::Tabs::unexpand` (tabstop 8) for one string.
pub fn unexpand(s: &str) -> String {
    let lines: Vec<String> = s
        .split('\n')
        .map(|line| {
            let line: Vec<char> = expand(line).chars().collect();
            // split(/((?:\PM\pM*|^\pM+){8})/, $line, -1): chunks of 8 units
            let mut chunks: Vec<String> = Vec::new();
            let mut i = 0;
            loop {
                let start = i;
                let mut units = 0;
                let mut j = i;
                while units < 8 && j < line.len() {
                    if j == 0 && is_mark(line[0]) {
                        while j < line.len() && is_mark(line[j]) {
                            j += 1;
                        }
                    } else if is_mark(line[j]) {
                        break;
                    } else {
                        j += 1;
                        while j < line.len() && is_mark(line[j]) {
                            j += 1;
                        }
                    }
                    units += 1;
                }
                if units == 8 {
                    chunks.push(line[start..j].iter().collect());
                    i = j;
                } else {
                    break;
                }
            }
            let lastbit: String = line[i..].iter().collect();
            let mut out = String::new();
            for c in chunks {
                // s/  +$/\t/
                let trimmed = c.trim_end_matches(' ');
                if c.len() - trimmed.len() >= 2 {
                    out.push_str(trimmed);
                    out.push('\t');
                } else {
                    out.push_str(&c);
                }
            }
            if lastbit == "        " {
                out.push('\t');
            } else {
                out.push_str(&lastbit);
            }
            out
        })
        .collect();
    lines.join("\n")
}

/// Length of a `$break` match at `i` (`(?>\n|\r\n|$brkspc\pM*)`).
fn brk(t: &[char], i: usize) -> Option<usize> {
    let c = *t.get(i)?;
    if c == '\n' {
        return Some(1);
    }
    if c == '\r' && t.get(i + 1) == Some(&'\n') {
        return Some(2);
    }
    if is_brkspc(c) {
        let mut j = i + 1;
        while j < t.len() && is_mark(t[j]) {
            j += 1;
        }
        return Some(j - i);
    }
    None
}

/// One unit at `i`: `(?>(?!\n)\PM\pM*|(?<![^\n])\pM+)`.
fn unit(t: &[char], i: usize) -> Option<usize> {
    let c = *t.get(i)?;
    if c != '\n' && !is_mark(c) {
        let mut j = i + 1;
        while j < t.len() && is_mark(t[j]) {
            j += 1;
        }
        return Some(j);
    }
    if is_mark(c) && (i == 0 || t[i - 1] == '\n') {
        let mut j = i;
        while j < t.len() && is_mark(t[j]) {
            j += 1;
        }
        return Some(j);
    }
    None
}

/// `\Z` at `i`: end of string or before a final newline.
fn at_big_z(t: &[char], i: usize) -> bool {
    i == t.len() || (i + 1 == t.len() && t[i] == '\n')
}

/// `Text::Wrap::wrap($ip, $xp, $text)` with `$columns = columns`.
pub fn wrap(columns: i64, ip: &str, xp: &str, text: &str) -> String {
    let mut columns = columns;
    let t: Vec<char> = expand(text).chars().collect();
    let xlen_s = |s: &str| xlen(&expand(s).chars().collect::<Vec<_>>()) as i64;
    let mut nll = columns - xlen_s(xp) - 1;
    if nll <= 0 && !xp.is_empty() {
        columns = xlen_s(xp) + 2;
        nll = 1;
    }
    let mut ll = (columns - xlen_s(ip) - 1).max(0);
    let mut r = String::new();
    let mut nl = String::new();
    let mut remainder = String::new();
    let mut lead = ip.to_string();
    let mut pos = 0usize;
    loop {
        // $t !~ /\G(?:$break)*\Z/gc (on success pos moves to the match end)
        {
            let mut j = pos;
            let mut ends = vec![j];
            while let Some(n) = brk(&t, j) {
                j += n;
                ends.push(j);
            }
            if let Some(&e) = ends.iter().rev().find(|e| at_big_z(&t, **e)) {
                pos = e;
                break;
            }
        }
        // \G((?>unit){0,$ll})($break|\n+|\z)
        let mut bounds = vec![pos];
        let mut j = pos;
        while (bounds.len() as i64 - 1) < ll {
            match unit(&t, j) {
                Some(n) => {
                    j = n;
                    bounds.push(j);
                }
                None => break,
            }
        }
        let mut matched = None;
        for &e in bounds.iter().rev() {
            if let Some(n) = brk(&t, e) {
                matched = Some((e, e + n));
                break;
            }
            if t.get(e) == Some(&'\n') {
                let mut k = e;
                while k < t.len() && t[k] == '\n' {
                    k += 1;
                }
                matched = Some((e, k));
                break;
            }
            if e == t.len() {
                matched = Some((e, e));
                break;
            }
        }
        if let Some((e, k)) = matched {
            let s1: String = t[pos..e].iter().collect();
            r.push_str(&unexpand(&format!("{nl}{lead}{s1}")));
            remainder = t[e..k].iter().collect();
            pos = k;
        } else if (bounds.len() as i64 - 1) == ll {
            // $huge eq 'wrap': exactly $ll units
            let e = *bounds.last().unwrap_or(&pos);
            let s1: String = t[pos..e].iter().collect();
            r.push_str(&unexpand(&format!("{nl}{lead}{s1}")));
            remainder = "\n".into();
            pos = e;
        } else {
            // Cannot happen with $columns >= 2.
            break;
        }
        lead = xp.to_string();
        ll = nll;
        nl = "\n".into();
    }
    r.push_str(&remainder);
    if pos != t.len() {
        r.push_str(&lead);
        r.extend(&t[pos..]);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(expand("a\tb"), "a       b");
        assert_eq!(unexpand("ab      cd"), "ab\tcd");
        assert_eq!(wrap(10, "", "", "aaaaaaaaaaaaaaa"), "aaaaaaaaa\naaaaaa");
    }
}

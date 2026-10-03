//! `Text::Roman` 3.5 `isroman` / `roman2int`, as used by biber.
//!
//! biber call sites: `Utils.pm:1652-1653` (`rangelen`, after `NFKD` so that
//! Unicode roman numerals such as `Ⅻ` become ASCII) and `Internals.pm:1346`
//! (`_sort_integer`, `isroman(NFKD($field))` unless `noroman`).
//!
//! Faithful quirks:
//! * the input is upper-cased with perl's full Unicode `uc` first, so `ı`
//!   (dotless i) counts as `I`;
//! * the character class is written `[@RSN]` under `/x`, i.e.
//!   `[I V X L C D M]`, and `/x` does not apply inside brackets, so **spaces
//!   are accepted** by `isroman` (`isroman("X I")` and even `isroman(" ")` are
//!   true), while `roman2int` then fails on them (returns `undef`);
//! * `$` matches before a trailing newline, so `isroman("XIV\n")` is true but
//!   `roman2int("XIV\n")` is `undef`;
//! * `roman2int` just adds the values of greedy tokens (`IV IX XL XC CD CM`
//!   before single letters), so non-canonical numerals that pass `isroman`
//!   (`IIX` = 10, `IM` = 1001) get a value.

use crate::case::uc;

const RSN: [char; 7] = ['I', 'V', 'X', 'L', 'C', 'D', 'M'];

fn value_of(c: char) -> u32 {
    match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        'C' => 100,
        'D' => 500,
        'M' => 1000,
        _ => 0,
    }
}

/// `$` semantics: the string with one trailing `\n` removed (for matching).
fn strip_final_newline(s: &str) -> &str {
    s.strip_suffix('\n').unwrap_or(s)
}

/// `isroman` on an already upper-cased string.
fn isroman_uc(s: &str) -> bool {
    let body = strip_final_newline(s);
    // /^[I V X L C D M]+$/x
    if body.is_empty() || !body.chars().all(|c| c == ' ' || RSN.contains(&c)) {
        return false;
    }
    let chars: Vec<char> = s.chars().collect();
    // /([IXCM])\1{3,}|([VLD])\2+/  (searched over the whole string)
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut run = 1;
        while i + run < chars.len() && chars[i + run] == c {
            run += 1;
        }
        if (matches!(c, 'I' | 'X' | 'C' | 'M') && run >= 4) || (matches!(c, 'V' | 'L' | 'D') && run >= 2) {
            return false;
        }
        i += run;
    }
    // IXI|XCX|CMC and, for each adjacent pair (a,b) in RSN, "aba" and "bab".
    let mut bad: Vec<[char; 3]> = vec![['I', 'X', 'I'], ['X', 'C', 'X'], ['C', 'M', 'C']];
    for k in 1..RSN.len() {
        let (a, b) = (RSN[k - 1], RSN[k]);
        bad.push([a, b, a]);
        bad.push([b, a, b]);
    }
    !chars.windows(3).any(|w| bad.iter().any(|p| p[..] == w[..]))
}

/// `Text::Roman::isroman`.
pub fn isroman(s: &str) -> bool {
    isroman_uc(&uc(s))
}

/// `Text::Roman::roman2int`; `None` is `undef`.
pub fn roman2int(s: &str) -> Option<u32> {
    let s = uc(s);
    if !isroman_uc(&s) {
        return None;
    }
    let mut r: &str = &s;
    let mut ret: u32 = 0;
    let mut prev: u32 = 0;
    const RCN: [&str; 6] = ["IV", "IX", "XL", "XC", "CD", "CM"];
    // `while ($r)`: perl truthiness of the remaining string
    while crate::truth::perl_truthy(r) {
        let mut matched = false;
        for tok in RCN {
            if let Some(rest) = r.strip_prefix(tok) {
                let mut cs = tok.chars();
                let (a, b) = (cs.next().unwrap_or('I'), cs.next().unwrap_or('I'));
                ret += value_of(b) - value_of(a);
                r = rest;
                matched = true;
                break;
            }
        }
        if !matched {
            for c in RSN {
                if let Some(rest) = r.strip_prefix(c) {
                    ret += value_of(c);
                    r = rest;
                    matched = true;
                    break;
                }
            }
        }
        let _ = matched;
        if ret <= prev {
            return None;
        }
        prev = ret;
    }
    Some(ret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roman() {
        assert_eq!(roman2int("XXiv"), Some(24));
        assert_eq!(roman2int("MCMXCIV"), Some(1994));
        assert_eq!(roman2int("IIX"), Some(10));
        assert!(isroman("X I"));
        assert!(isroman(" "));
        assert_eq!(roman2int("X I"), None);
        assert!(isroman("XIV\n"));
        assert_eq!(roman2int("XIV\n"), None);
        assert!(!isroman("IIII"));
        assert!(!isroman("VV"));
        assert!(!isroman("IVI"));
        assert!(!isroman(""));
        assert!(isroman("ı"));
    }
}

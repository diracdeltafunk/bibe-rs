//! Character classes and grapheme clusters with the exact semantics of the
//! regex constructs biber's `Recode.pm` uses, under perl 5.42 (Unicode 16.0.0).
//!
//! The tables in `unicode_tables.rs` are dumped from that perl by
//! `tools/oracle/probes/latex-recode/gen_unicode_tables.pl`.

use crate::unicode_tables as t;

#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Gcb {
    Other,
    CR,
    LF,
    Control,
    Extend,
    ZWJ,
    Regional_Indicator,
    Prepend,
    SpacingMark,
    L,
    V,
    T,
    LV,
    LVT,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InCb {
    None,
    Consonant,
    Extend,
    Linker,
}

#[inline]
fn in_ranges(c: char, r: &[(u32, u32)]) -> bool {
    let c = c as u32;
    r.binary_search_by(|&(lo, hi)| {
        if hi < c {
            std::cmp::Ordering::Less
        } else if lo > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

#[inline]
fn in_ranges_val<V: Copy>(c: char, r: &[(u32, u32, V)]) -> Option<V> {
    let c = c as u32;
    r.binary_search_by(|&(lo, hi, _)| {
        if hi < c {
            std::cmp::Ordering::Less
        } else if lo > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .ok()
    .map(|i| r[i].2)
}

/// Perl `\s` (Unicode rules: `XPosixSpace`).
#[inline]
pub(crate) fn is_space(c: char) -> bool {
    if c.is_ascii() {
        matches!(c, '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' ')
    } else {
        in_ranges(c, t::SPACE)
    }
}

/// Perl `\w` (Unicode rules: `XPosixWord`).
#[inline]
pub(crate) fn is_word(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || c == '_'
    } else {
        in_ranges(c, t::WORD)
    }
}

/// Perl `\d` (Unicode rules: `\p{Nd}`).
#[inline]
pub(crate) fn is_digit(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_digit()
    } else {
        in_ranges(c, t::DIGIT)
    }
}

/// Perl `\pL`.
#[inline]
pub(crate) fn is_letter(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphabetic()
    } else {
        in_ranges(c, t::LETTER)
    }
}

/// Perl `\pM`.
#[inline]
pub(crate) fn is_mark(c: char) -> bool {
    if (c as u32) < 0x300 {
        false
    } else {
        in_ranges(c, t::MARK)
    }
}

#[inline]
fn gcb(c: char) -> Gcb {
    if (c as u32) < 0x7F {
        return match c {
            '\r' => Gcb::CR,
            '\n' => Gcb::LF,
            '\0'..='\x1F' => Gcb::Control,
            _ => Gcb::Other,
        };
    }
    in_ranges_val(c, t::GCB).unwrap_or(Gcb::Other)
}

#[inline]
fn incb(c: char) -> InCb {
    if (c as u32) < 0x300 {
        return InCb::None;
    }
    in_ranges_val(c, t::INCB).unwrap_or(InCb::None)
}

#[inline]
fn is_ext_pict(c: char) -> bool {
    (c as u32) >= 0xA9 && in_ranges(c, t::EXT_PICT)
}

/// End (byte offset) of the extended grapheme cluster that Perl's `\X` matches
/// when it starts at byte offset `start` of `s` (`start < s.len()`).
///
/// Perl's `\X` may consult characters before `start` for the RI, emoji and
/// Indic-conjunct rules; every call site in `Recode.pm` starts `\X` right after
/// a `{` or `\x1f`, which stops all of those look-backs, so starting fresh is
/// equivalent.
pub(crate) fn grapheme_end(s: &str, start: usize) -> usize {
    let mut it = s[start..].char_indices();
    let Some((_, c0)) = it.next() else {
        return start;
    };
    let mut prev = gcb(c0);
    let mut ri_run: u32 = u32::from(prev == Gcb::Regional_Indicator);
    // GB11: 0 = none, 1 = ExtPict Extend*, 2 = ExtPict Extend* ZWJ
    let mut ep: u8 = u8::from(is_ext_pict(c0));
    // GB9c: 0 = none, 1 = Consonant [Extend|Linker]* (no Linker), 2 = ... with a Linker
    let mut cb: u8 = u8::from(incb(c0) == InCb::Consonant);
    for (off, c) in it {
        let cur = gcb(c);
        let join = if prev == Gcb::CR && cur == Gcb::LF {
            true
        } else if matches!(prev, Gcb::Control | Gcb::CR | Gcb::LF)
            || matches!(cur, Gcb::Control | Gcb::CR | Gcb::LF)
        {
            false
        } else if (prev == Gcb::L && matches!(cur, Gcb::L | Gcb::V | Gcb::LV | Gcb::LVT))
            || (matches!(prev, Gcb::LV | Gcb::V) && matches!(cur, Gcb::V | Gcb::T))
            || (matches!(prev, Gcb::LVT | Gcb::T) && cur == Gcb::T)
            || matches!(cur, Gcb::Extend | Gcb::ZWJ | Gcb::SpacingMark)
            || prev == Gcb::Prepend
        {
            true
        } else {
            let c_incb = incb(c);
            (cb == 2 && c_incb == InCb::Consonant)
                || (prev == Gcb::ZWJ && ep == 2 && is_ext_pict(c))
                || (prev == Gcb::Regional_Indicator
                    && cur == Gcb::Regional_Indicator
                    && ri_run % 2 == 1)
        };
        if !join {
            return start + off;
        }
        // update states with c
        ri_run = if cur == Gcb::Regional_Indicator {
            ri_run + 1
        } else {
            0
        };
        ep = if is_ext_pict(c) {
            1
        } else if ep == 1 && cur == Gcb::Extend {
            1
        } else if ep == 1 && cur == Gcb::ZWJ {
            2
        } else {
            0
        };
        cb = match incb(c) {
            InCb::Consonant => 1,
            InCb::Linker if cb != 0 => 2,
            InCb::Extend if cb != 0 => cb,
            _ => 0,
        };
        prev = cur;
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clusters(s: &str) -> Vec<&str> {
        let mut v = Vec::new();
        let mut i = 0;
        while i < s.len() {
            let e = grapheme_end(s, i);
            v.push(&s[i..e]);
            i = e;
        }
        v
    }

    #[test]
    fn basic_clusters() {
        assert_eq!(clusters("a\u{301}b"), vec!["a\u{301}", "b"]);
        assert_eq!(clusters("\r\n\n"), vec!["\r\n", "\n"]);
        assert_eq!(
            clusters("\u{1F1FA}\u{1F1F8}\u{1F1EB}"),
            vec!["\u{1F1FA}\u{1F1F8}", "\u{1F1EB}"]
        );
        assert_eq!(
            clusters("\u{1F468}\u{200D}\u{1F469}"),
            vec!["\u{1F468}\u{200D}\u{1F469}"]
        );
        assert_eq!(
            clusters("\u{915}\u{94D}\u{937}"),
            vec!["\u{915}\u{94D}\u{937}"]
        );
        assert_eq!(
            clusters("\u{1100}\u{1161}\u{11A8}"),
            vec!["\u{1100}\u{1161}\u{11A8}"]
        );
    }

    #[test]
    fn classes() {
        assert!(is_space('\u{85}') && is_space('\u{A0}') && is_space('\x0B'));
        assert!(!is_space('\u{200B}'));
        assert!(is_word('\u{301}') && is_word('_') && is_word('\u{661}'));
        assert!(is_digit('\u{661}') && !is_digit('\u{B2}'));
        assert!(is_letter('\u{131}') && !is_letter('\u{301}'));
        assert!(is_mark('\u{301}') && is_mark('\u{903}'));
    }
}

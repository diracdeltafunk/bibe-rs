//! Grapheme clusters, in the two flavours biber mixes.
//!
//! # `Unicode::GCString` (sombok 2.4.0, Unicode 8.0.0) — [`gcstring`]
//!
//! biber uses `Unicode::GCString->new($s)` (Unicode::LineBreak 2019.001,
//! bundling sombok 2.4.0 with **Unicode 8.0.0** property data) for:
//! * sorting `sortinit` — first grapheme of the normalised sort string
//!   (`Internals.pm:1268`);
//! * labelalpha substrings and their padding/width bookkeeping
//!   (`Internals.pm:648-653, 747-758, 866-891, 1003, 1030`);
//! * sort-string `substring_width` / padding (`Internals.pm:1563, 1575`);
//! * initials: first grapheme of a name part, or first two after a
//!   hyphen-like start (`Utils.pm:1858-1861`), `~` vs space after short first
//!   names (`Utils.pm:1886`);
//! * `match_indices` (`Utils.pm:1705`, which passes *character* offsets from
//!   `@-`/`@+` to the grapheme `substr`, a biber quirk the caller reproduces
//!   by calling [`gcstring::substr`] with those numbers);
//! * month names (`bibtex.pm:1987`, `substr(0,3)`);
//! * `.bbl` line-wrapping heuristics (`Output/bbl.pm:137-140`) and tool-mode
//!   field alignment (`Output/bibtex.pm:349, 541, 580`).
//!
//! `Unicode::GCString->new($str)` without a `Unicode::LineBreak` object uses a
//! default sombok object whose options are only `COMPLEX_BREAKING`; in
//! particular `LegacyCM` and `ViramaAsJoiner` are **off** there (they are on
//! only in objects made through `Unicode::LineBreak->new`). The resulting rules
//! (sombok `gcstring.c` `_gcinfo`) differ from any version of UAX #29:
//! * a character of line-break class SP (U+0020) is always a cluster of its
//!   own, even when followed by a combining mark (`" \x{301}"` is 2 clusters);
//! * CR LF is one cluster; Control/CR/LF otherwise stand alone;
//! * Hangul L/V/T/LV/LVT sequences join (GB6-8);
//! * any run of Regional Indicators is one cluster (Unicode 8 GB8a, no
//!   pairing);
//! * Extend, SpacingMark and sombok's own `Virama` class always extend;
//! * there is no ZWJ/emoji rule (ZWJ is Extend in Unicode 8, so `👨‍👩` is two
//!   clusters: `👨\x{200D}` and `👩`), no GB9c, and the Prepend rule is
//!   inert because sombok's 8.0.0 table has no Prepend characters.
//!
//! The property table is sombok's own (extracted from its `lib/8.0.0.c`), so
//! characters assigned after Unicode 8.0 get sombok's defaults (mostly
//! `Other`, i.e. they start a new cluster; e.g. U+1AB0..1AFF combining marks
//! added later still extend because they were assigned by 8.0).
//!
//! # perl's `\X` and `\b{gcb}` (Unicode 16.0.0) — [`perl`]
//!
//! perl 5.42's regex engine implements extended grapheme clusters per UAX #29
//! for Unicode 16.0.0 (GB9c Indic conjuncts, GB11 emoji ZWJ sequences, GB12/13
//! regional-indicator pairs). biber uses it in its own regexes: `split
//! /\b{gcb}/` when splitting names into characters for initials
//! (`bibtex.pm:2013`), and `\X` in `\A\{\X+\}\z`-style tests for protected
//! names (`bibtex.pm:1266`, `Utils.pm:829, 842`), `\Ashort(\X+)\z`
//! (`Biber.pm:2563`, `Internals.pm:529`), `(\X*)year` (`Biber.pm:2705`) and
//! the LaTeX recoder (`Recode.pm:213-321`).

/// Grapheme_Cluster_Break values in sombok's Unicode 8.0.0 table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Gcb8 {
    Cr,
    Lf,
    Control,
    Extend,
    Prepend,
    SpacingMark,
    L,
    V,
    T,
    Lv,
    Lvt,
    Other,
    Virama,
    OtherLetter,
    RegionalIndicator,
}

/// Grapheme_Cluster_Break values, Unicode 16.0.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Gcb16 {
    Cr,
    Lf,
    Control,
    Extend,
    Zwj,
    RegionalIndicator,
    Prepend,
    SpacingMark,
    L,
    V,
    T,
    Lv,
    Lvt,
    Other,
}

fn lookup3<T: Copy>(t: &[(u32, u32, T)], cp: u32, default: T) -> T {
    match t.binary_search_by(|&(lo, hi, _)| {
        if hi < cp {
            std::cmp::Ordering::Less
        } else if lo > cp {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(i) => t[i].2,
        Err(_) => default,
    }
}

/// `Unicode::GCString` (sombok, Unicode 8.0.0) grapheme clusters.
pub mod gcstring {
    use super::{Gcb8, lookup3};
    use crate::tables::gcb8;
    use crate::unicode::in_ranges;

    /// sombok's grapheme-cluster-break class of `c`.
    #[inline]
    pub fn gcb(c: char) -> Gcb8 {
        let cp = c as u32;
        if (0x20..0x7F).contains(&cp) {
            return Gcb8::Other;
        }
        lookup3(gcb8::GCB, cp, Gcb8::Other)
    }

    #[inline]
    fn is_lb_sp(c: char) -> bool {
        c == ' ' || (!c.is_ascii() && in_ranges(gcb8::LB_SP, c as u32))
    }

    #[inline]
    fn is_extender(g: Gcb8) -> bool {
        matches!(g, Gcb8::Extend | Gcb8::SpacingMark | Gcb8::Virama)
    }

    /// Byte length of the cluster starting at the beginning of `s`
    /// (port of sombok `_gcinfo` with default options). `s` must be non-empty.
    pub fn cluster_len(s: &str) -> usize {
        let mut it = s.char_indices();
        let Some((_, c0)) = it.next() else { return 0 };
        let mut end = c0.len_utf8();
        let mut g = gcb(c0);
        match g {
            Gcb8::Lf | Gcb8::Control => return end,
            Gcb8::Cr => {
                if let Some((_, c1)) = it.next()
                    && gcb(c1) == Gcb8::Lf
                {
                    end += c1.len_utf8();
                }
                return end;
            }
            _ => {}
        }
        // LegacyCM is off: a SP-class base never takes extenders.
        if is_lb_sp(c0) {
            return end;
        }
        for (_, c) in it {
            let ng = gcb(c);
            let join = if matches!(ng, Gcb8::Control | Gcb8::Cr | Gcb8::Lf) {
                false
            } else if (g == Gcb8::L && matches!(ng, Gcb8::L | Gcb8::V | Gcb8::Lv | Gcb8::Lvt))
                || (matches!(g, Gcb8::Lv | Gcb8::V) && matches!(ng, Gcb8::V | Gcb8::T))
                || (matches!(g, Gcb8::Lvt | Gcb8::T) && ng == Gcb8::T)
                || (g == Gcb8::RegionalIndicator && ng == Gcb8::RegionalIndicator)
            {
                true
            } else {
                // GB9/9a (+ sombok's Virama); GB9b; the Virama-as-joiner rule
                // is disabled in the default object.
                is_extender(ng) || g == Gcb8::Prepend
            };
            if !join {
                break;
            }
            end += c.len_utf8();
            g = ng;
        }
        end
    }

    /// Iterator over the clusters of a string.
    #[derive(Debug, Clone)]
    pub struct Clusters<'a> {
        rest: &'a str,
    }

    impl<'a> Iterator for Clusters<'a> {
        type Item = &'a str;
        fn next(&mut self) -> Option<&'a str> {
            if self.rest.is_empty() {
                return None;
            }
            let n = cluster_len(self.rest);
            let (a, b) = self.rest.split_at(n);
            self.rest = b;
            Some(a)
        }
    }

    /// The clusters of `s`.
    pub fn clusters(s: &str) -> Clusters<'_> {
        Clusters { rest: s }
    }

    /// `Unicode::GCString->new($s)->length`.
    pub fn length(s: &str) -> usize {
        if s.is_ascii() && !s.contains('\r') {
            return s.len();
        }
        clusters(s).count()
    }

    /// `Unicode::GCString->new($s)->substr($offset, $length)->as_string`
    /// (`length = None` means "to the end", the XS default `gclen`).
    /// Out-of-range requests give `""`, exactly as sombok's
    /// `gcstring_substr`.
    pub fn substr(s: &str, offset: i64, length: Option<i64>) -> &str {
        if offset >= 0 && length.is_some_and(|l| l >= 0) {
            // Fast path: no need to know the total length.
            let len = length.unwrap_or(0);
            let mut start = None;
            let mut pos = 0usize;
            let mut idx = 0i64;
            let mut rest = s;
            while !rest.is_empty() {
                if idx == offset {
                    start = Some(pos);
                }
                if idx == offset + len {
                    break;
                }
                let n = cluster_len(rest);
                pos += n;
                rest = &rest[n..];
                idx += 1;
            }
            if idx == offset && start.is_none() {
                start = Some(pos);
            }
            return match start {
                Some(st) => &s[st..pos],
                None => "",
            };
        }
        let starts: Vec<usize> = {
            let mut v = Vec::new();
            let mut pos = 0;
            for c in clusters(s) {
                v.push(pos);
                pos += c.len();
            }
            v
        };
        let gclen = starts.len() as i64;
        let mut offset = offset;
        let mut length = length.unwrap_or(gclen);
        if offset < 0 {
            offset += gclen;
        }
        if offset < 0 {
            length += offset;
            offset = 0;
        }
        if length < 0 {
            length += gclen - offset;
        }
        if length < 0 || gclen < offset {
            return "";
        }
        if gclen == offset {
            return "";
        } else if gclen <= offset + length {
            length = gclen - offset;
        }
        let b = starts[offset as usize];
        let e = if ((offset + length) as usize) < starts.len() {
            starts[(offset + length) as usize]
        } else {
            s.len()
        };
        &s[b..e]
    }

    /// `Unicode::GCString->new($s)->substr(0, 1)->as_string`: the first
    /// cluster (`""` for an empty string).
    pub fn first(s: &str) -> &str {
        if s.is_empty() {
            s
        } else {
            &s[..cluster_len(s)]
        }
    }
}

/// perl 5.42 `\X` / `\b{gcb}` (UAX #29, Unicode 16.0.0).
pub mod perl {
    use super::{Gcb16, lookup3};
    use crate::tables::gcb16;
    use crate::unicode::in_ranges;

    /// Grapheme_Cluster_Break of `c` (Unicode 16.0.0).
    #[inline]
    pub fn gcb(c: char) -> Gcb16 {
        let cp = c as u32;
        if (0x20..0x7F).contains(&cp) {
            return Gcb16::Other;
        }
        lookup3(gcb16::GCB, cp, Gcb16::Other)
    }

    #[inline]
    fn ext_pict(c: char) -> bool {
        !c.is_ascii() && in_ranges(gcb16::EXT_PICT, c as u32)
    }

    /// Kind of the last character for the GB9c emulation.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum IncbKind {
        Other,
        Consonant,
        LinkerOrExtend,
    }

    /// perl's GB9c check (`regexec.c` `isGCB`, case
    /// `GCB_InCB_Consonant_then_InCB_Extend_or_InCB_Linker_v_InCB_Consonant`)
    /// scans *backwards from the character before the boundary* over
    /// `InCB=Linker|Extend` characters, counting linkers, and requires a
    /// consonant at the end. When the character before the boundary is itself
    /// a consonant, the scan starts at the character before *that*, so perl
    /// also keeps `C1 Linker C2 × C3` together (standard UAX #29 breaks before
    /// C3). This state machine reproduces perl's behaviour.
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Incb {
        kind: IncbKind,
        /// The current Linker/Extend run is preceded by a consonant.
        run_cons: bool,
        run_linkers: u32,
        /// When `kind == Consonant`: perl joins a following consonant.
        cons_join: bool,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Pict {
        None,
        Seen,
        SeenZwj,
    }

    #[derive(Clone, Copy)]
    struct State {
        prev: Gcb16,
        incb: Incb,
        pict: Pict,
        ri: u32,
    }

    impl State {
        fn new(c: char) -> State {
            let mut s = State {
                prev: Gcb16::Other,
                incb: Incb {
                    kind: IncbKind::Other,
                    run_cons: false,
                    run_linkers: 0,
                    cons_join: false,
                },
                pict: Pict::None,
                ri: 0,
            };
            s.push(c, gcb(c));
            s
        }

        fn push(&mut self, c: char, g: Gcb16) {
            let cp = c as u32;
            let (cons, link, ext) = if c.is_ascii() {
                (false, false, false)
            } else {
                (
                    in_ranges(gcb16::INCB_CONSONANT, cp),
                    in_ranges(gcb16::INCB_LINKER, cp),
                    in_ranges(gcb16::INCB_EXTEND, cp),
                )
            };
            let st = &mut self.incb;
            if cons {
                st.cons_join =
                    st.kind == IncbKind::LinkerOrExtend && st.run_cons && st.run_linkers > 0;
                st.kind = IncbKind::Consonant;
                st.run_cons = true;
                st.run_linkers = 0;
            } else if link || ext {
                if st.kind == IncbKind::Other {
                    st.run_cons = false;
                    st.run_linkers = 0;
                }
                st.run_linkers += u32::from(link);
                st.kind = IncbKind::LinkerOrExtend;
            } else {
                st.kind = IncbKind::Other;
                st.run_cons = false;
                st.run_linkers = 0;
            }
            self.pict = if ext_pict(c) {
                Pict::Seen
            } else {
                match (self.pict, g) {
                    (Pict::Seen, Gcb16::Extend) => Pict::Seen,
                    (Pict::Seen, Gcb16::Zwj) => Pict::SeenZwj,
                    _ => Pict::None,
                }
            };
            self.ri = if g == Gcb16::RegionalIndicator {
                self.ri + 1
            } else {
                0
            };
            self.prev = g;
        }

        /// Is there a boundary between the state's last char and `c`?
        fn breaks_before(&self, c: char, g: Gcb16) -> bool {
            use Gcb16::*;
            let p = self.prev;
            if p == Cr && g == Lf {
                return false;
            }
            if matches!(p, Control | Cr | Lf) || matches!(g, Control | Cr | Lf) {
                return true;
            }
            if p == L && matches!(g, L | V | Lv | Lvt) {
                return false;
            }
            if matches!(p, Lv | V) && matches!(g, V | T) {
                return false;
            }
            if matches!(p, Lvt | T) && g == T {
                return false;
            }
            if matches!(g, Extend | Zwj | SpacingMark) || p == Prepend {
                return false;
            }
            let incb_join = match self.incb.kind {
                IncbKind::LinkerOrExtend => self.incb.run_cons && self.incb.run_linkers > 0,
                IncbKind::Consonant => self.incb.cons_join,
                IncbKind::Other => false,
            };
            if incb_join && !c.is_ascii() && in_ranges(gcb16::INCB_CONSONANT, c as u32) {
                return false;
            }
            if p == Zwj && self.pict == Pict::SeenZwj && ext_pict(c) {
                return false;
            }
            if p == RegionalIndicator && g == RegionalIndicator && self.ri % 2 == 1 {
                return false;
            }
            true
        }
    }

    /// Byte length of the `\X` match at the start of `s` (`s` non-empty).
    pub fn cluster_len(s: &str) -> usize {
        let mut it = s.chars();
        let Some(c0) = it.next() else { return 0 };
        let mut st = State::new(c0);
        let mut end = c0.len_utf8();
        for c in it {
            let g = gcb(c);
            if st.breaks_before(c, g) {
                break;
            }
            st.push(c, g);
            end += c.len_utf8();
        }
        end
    }

    /// Iterator over `\X` clusters.
    #[derive(Debug, Clone)]
    pub struct Clusters<'a> {
        rest: &'a str,
    }

    impl<'a> Iterator for Clusters<'a> {
        type Item = &'a str;
        fn next(&mut self) -> Option<&'a str> {
            if self.rest.is_empty() {
                return None;
            }
            let n = cluster_len(self.rest);
            let (a, b) = self.rest.split_at(n);
            self.rest = b;
            Some(a)
        }
    }

    /// The `\X` clusters of `s`; equal to `split /\b{gcb}/, $s` (which
    /// returns an empty list for an empty string).
    pub fn clusters(s: &str) -> Clusters<'_> {
        Clusters { rest: s }
    }

    /// Number of `\X` clusters.
    pub fn length(s: &str) -> usize {
        clusters(s).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sombok() {
        assert_eq!(gcstring::length("e\u{301}a"), 2);
        assert_eq!(gcstring::length(" \u{301}"), 2);
        assert_eq!(gcstring::length("\r\n"), 1);
        assert_eq!(gcstring::length("🇩🇪🇫🇷"), 1);
        assert_eq!(gcstring::length("👨\u{200d}👩"), 2);
        assert_eq!(gcstring::substr("abcdef", 1, Some(2)), "bc");
        assert_eq!(gcstring::substr("abcdef", -2, None), "ef");
        assert_eq!(gcstring::substr("abc", 5, Some(1)), "");
        assert_eq!(gcstring::substr("abc", 1, Some(10)), "bc");
        assert_eq!(gcstring::substr("abc", 0, Some(-1)), "ab");
        assert_eq!(gcstring::first("e\u{301}x"), "e\u{301}");
        assert_eq!(gcstring::first(""), "");
    }

    #[test]
    fn perl_x() {
        assert_eq!(perl::length("🇩🇪🇫🇷"), 2);
        assert_eq!(perl::length("👨\u{200d}👩"), 1);
        assert_eq!(perl::length(" \u{301}"), 1);
        assert_eq!(perl::length("\r\n"), 1);
        assert_eq!(perl::length("क्\u{200d}ष"), 1);
        let v: Vec<&str> = perl::clusters("ab\u{308}c").collect();
        assert_eq!(v, ["a", "b\u{308}", "c"]);
    }
}

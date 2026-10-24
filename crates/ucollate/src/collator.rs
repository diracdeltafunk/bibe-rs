//! The collator: string → collation elements → sort key.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::sync::Arc;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

use crate::Error;
use crate::cjk::{CjkTable, Override};
use crate::locale::{file_stem, resolve_locale};
use crate::options::{CollateOptions, Normalization, Variable};
use crate::tables::{self, Ce};
use crate::tailoring::{LocaleData, Tailoring, locale_data};

/// Marks a code point consumed by a discontiguous contraction.
const REMOVED: u32 = u32::MAX;
/// Code points above U+10FFFF are collated as U+FFFD (they can only be passed in via
/// [`Collator::sort_key_from_code_points`]).
const OUT_OF_RANGE: u32 = 0x110000;

/// A collator equivalent to a `Unicode::Collate::Locale` 1.31 object
/// (`Unicode::Collate::Locale->new(locale => …)->change(…)`), UCA 43 / DUCET 13.0.0.
#[derive(Clone, Debug)]
pub struct Collator {
    locale: &'static str,
    data: Option<Arc<LocaleData>>,
    tailoring: Option<Arc<Tailoring>>,
    cjk: Option<&'static CjkTable>,
    suppress: Vec<u32>,
    level: u8,
    variable: Variable,
    normalization: Normalization,
    /// Bit `n - 1` set: level `n` is backwards.
    backwards: u8,
    backwards_list: Vec<u8>,
    upper_before_lower: Option<bool>,
    katakana_before_hiragana: bool,
    ignore_level2: bool,
    hangul_terminator: u16,
    identical: bool,
    highest_ffff: bool,
    minimal_fffe: bool,
}

#[derive(Default)]
struct Scratch {
    cps: Vec<u32>,
    orig: Vec<u32>,
    key: Vec<u32>,
    ces: Vec<Ce>,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

#[inline]
fn ccc(cp: u32) -> u8 {
    char::from_u32(cp).map_or(0, canonical_combining_class)
}

#[inline]
fn is_hangul_syllable(cp: u32) -> bool {
    (0xAC00..=0xD7A3).contains(&cp)
}

/// Hangul syllable type (conjoining jamo behaviour) as a small enum.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hst {
    None,
    L,
    V,
    T,
    Lv,
    Lvt,
}

fn hst(cp: u32) -> Hst {
    match cp {
        0x1100..=0x115F | 0xA960..=0xA97C => Hst::L,
        0x1160..=0x11A7 | 0xD7B0..=0xD7C6 => Hst::V,
        0x11A8..=0x11FF | 0xD7CB..=0xD7FB => Hst::T,
        0xAC00..=0xD7A3 => {
            if (cp - 0xAC00).is_multiple_of(28) {
                Hst::Lv
            } else {
                Hst::Lvt
            }
        }
        _ => Hst::None,
    }
}

impl Collator {
    /// Create a collator for `locale` (resolved like `Unicode::Collate::Locale`; see
    /// [`crate::resolve_locale`]), then apply `options` like `->change(%options)`.
    pub fn new(locale: Option<&str>, options: &CollateOptions) -> Result<Collator, Error> {
        let accepted = resolve_locale(locale);
        let data = match file_stem(accepted) {
            Some(stem) => Some(locale_data(stem)?),
            None => None,
        };
        let mut c = Collator {
            locale: accepted,
            tailoring: data.as_ref().map(|d| d.tailoring.clone()),
            cjk: data.as_ref().and_then(|d| d.override_cjk),
            suppress: data
                .as_ref()
                .map(|d| d.suppress.clone())
                .unwrap_or_default(),
            level: 4,
            variable: data
                .as_ref()
                .and_then(|d| d.variable)
                .unwrap_or(Variable::Shifted),
            normalization: Normalization::Nfd,
            backwards: 0,
            backwards_list: Vec::new(),
            upper_before_lower: data.as_ref().and_then(|d| d.upper_before_lower),
            katakana_before_hiragana: false,
            ignore_level2: false,
            hangul_terminator: 0,
            identical: false,
            highest_ffff: false,
            minimal_fffe: false,
            data,
        };
        if let Some(b) = c.data.as_ref().and_then(|d| d.backwards) {
            c.set_backwards(&[b]);
        }
        if let Some(e) = &options.entry {
            let mut t = c.tailoring.as_deref().cloned().unwrap_or_default();
            t.add_entries(e)?;
            c.tailoring = Some(Arc::new(t));
        }
        if let Some(s) = &options.suppress {
            c.suppress = s.clone();
        }
        c.suppress.sort_unstable();
        if c.tailoring.as_ref().is_some_and(|t| t.is_empty()) {
            c.tailoring = None;
        }
        c.change(options)?;
        Ok(c)
    }

    fn set_backwards(&mut self, levels: &[u8]) {
        self.backwards = 0;
        for &l in levels {
            self.backwards |= 1 << (l - 1);
        }
        self.backwards_list = levels.to_vec();
    }

    /// Apply options like `Unicode::Collate::change()`. `entry` and `suppress` are
    /// constructor-only and rejected here.
    pub fn change(&mut self, o: &CollateOptions) -> Result<(), Error> {
        if let Some(l) = o.level {
            self.level = crate::options::check_level(l as i64, "level")?;
        }
        if let Some(v) = o.uca_version
            && v != 43
        {
            return Err(Error::Unsupported(format!(
                "UCA_Version {v} is not supported (only 43)"
            )));
        }
        if let Some(b) = &o.backwards {
            for &l in b {
                crate::options::check_level(l as i64, "backwards")?;
            }
            self.set_backwards(b);
        }
        if let Some(v) = o.variable {
            self.variable = v;
        }
        if let Some(n) = o.normalization {
            self.normalization = n;
        }
        if let Some(v) = o.upper_before_lower {
            self.upper_before_lower = Some(v);
        }
        if let Some(v) = o.katakana_before_hiragana {
            self.katakana_before_hiragana = v;
        }
        if let Some(v) = o.ignore_level2 {
            self.ignore_level2 = v;
        }
        if let Some(v) = o.hangul_terminator {
            self.hangul_terminator = v;
        }
        if let Some(v) = o.identical {
            self.identical = v;
        }
        if let Some(v) = o.highest_ffff {
            self.highest_ffff = v;
        }
        if let Some(v) = o.minimal_fffe {
            self.minimal_fffe = v;
        }
        Ok(())
    }

    /// The locale name actually used (`getlocale`), e.g. `"sv"`, `"fr_CA"` or `"default"`.
    pub fn locale(&self) -> &'static str {
        self.locale
    }

    /// The version of the tailoring file (`locale_version`), `None` for `default`.
    pub fn locale_version(&self) -> Option<&str> {
        self.data.as_ref().and_then(|d| d.locale_version.as_deref())
    }

    /// The DUCET version (`version()`): `"13.0.0"`.
    pub fn version(&self) -> &'static str {
        "13.0.0"
    }

    /// The UCA revision emulated (`UCA_Version`): 43.
    pub fn uca_version(&self) -> u32 {
        43
    }

    /// The current value of a `Unicode::Collate` option as Perl would stringify what
    /// `change()` returns for it (`None` for `undef` and for unknown keys).
    ///
    /// Biber uses this to log "Overriding locale '…' defaults '…'" messages: on a fresh
    /// collator it reports the locale's/module's defaults, e.g. `level` → `"4"`,
    /// `variable` → `"shifted"`, `normalization` → `"NFD"`, and `upper_before_lower` →
    /// `"1"` for `da` but `None` for `en`.
    pub fn option_value(&self, key: &str) -> Option<String> {
        let b = |v: bool| if v { Some("1".to_string()) } else { None };
        match key {
            "level" => Some(self.level.to_string()),
            "variable" | "alternate" => Some(self.variable.as_str().to_string()),
            "normalization" => self.normalization.as_str().map(str::to_string),
            "upper_before_lower" => self
                .upper_before_lower
                .map(|v| if v { "1" } else { "0" }.to_string()),
            "backwards" => match self.backwards_list.as_slice() {
                [] => None,
                [l] => Some(l.to_string()),
                _ => Some("ARRAY".to_string()),
            },
            "katakana_before_hiragana" => b(self.katakana_before_hiragana),
            "ignore_level2" => b(self.ignore_level2),
            "identical" => b(self.identical),
            "highestFFFF" => b(self.highest_ffff),
            "minimalFFFE" => b(self.minimal_fffe),
            "hangul_terminator" => {
                (self.hangul_terminator != 0).then(|| self.hangul_terminator.to_string())
            }
            "UCA_Version" => Some("43".to_string()),
            "long_contraction" | "overrideHangul" => Some(String::new()),
            "overrideCJK" => Some(if self.cjk.is_some() {
                "CODE".to_string()
            } else {
                String::new()
            }),
            _ => None,
        }
    }

    /// Maximum level.
    pub fn level(&self) -> u8 {
        self.level
    }

    /// Variable weighting.
    pub fn variable(&self) -> Variable {
        self.variable
    }

    /// Normalisation.
    pub fn normalization(&self) -> Normalization {
        self.normalization
    }

    // ------------------------------------------------------------------ lookups

    #[inline]
    fn single(&self, cp: u32) -> Option<&[Ce]> {
        if let Some(t) = &self.tailoring
            && let Some(c) = t.single(cp)
        {
            return Some(c);
        }
        tables::ducet_single(cp)
    }

    #[inline]
    fn suppressed(&self, cp: u32) -> bool {
        !self.suppress.is_empty() && self.suppress.binary_search(&cp).is_ok()
    }

    /// The mapping of `cp` (if it has one of its own) and the length of the longest
    /// contraction starting with it, in one lookup.
    #[inline]
    fn info(&self, cp: u32) -> (Option<&[Ce]>, usize) {
        let raw = tables::ducet_raw(cp);
        let mut single = tables::raw_single(raw);
        let mut ml = tables::raw_maxlen(raw);
        if ml > 0 && self.suppressed(cp) {
            ml = 0;
        }
        if let Some(t) = &self.tailoring
            && let Some(st) = t.start(cp)
        {
            if let Some(s) = t.single_of(st) {
                single = Some(s);
            }
            ml = ml.max(st.maxlen);
        }
        (single, ml)
    }

    #[inline]
    fn maxlen(&self, cp: u32) -> usize {
        self.info(cp).1
    }

    #[inline]
    fn contraction(&self, cps: &[u32]) -> Option<&[Ce]> {
        if let Some(t) = &self.tailoring
            && let Some(c) = t.contraction(cps)
        {
            return Some(c);
        }
        if self.suppressed(cps[0]) {
            return None;
        }
        tables::ducet_contraction(cps)
    }

    /// Collation elements of a single code point that is not part of a contraction.
    fn push_single(&self, cp: u32, out: &mut Vec<Ce>) {
        if self.highest_ffff && cp == 0xFFFF {
            out.push(Ce::new(0xFFFE, 0x20, 0x05, 0xFFFF, false));
            return;
        }
        if self.minimal_fffe && cp == 0xFFFE {
            out.push(Ce::new(0x0001, 0x20, 0x05, 0xFFFE, false));
            return;
        }
        let cp = if cp >= OUT_OF_RANGE { 0xFFFD } else { cp };
        if let Some(c) = self.single(cp) {
            out.extend_from_slice(c);
        } else if is_hangul_syllable(cp) {
            self.push_hangul(cp, out);
        } else if let Some(t) = self.cjk.filter(|_| tables::cjk_unified(cp).is_some()) {
            match t.get(cp) {
                Some(Override::Primary(p)) => out.push(Ce::new(p, 0x20, 0x02, cp as u16, false)),
                Some(Override::Elements(e)) => out.extend_from_slice(e),
                None => tables::push_implicit(cp, out),
            }
        } else {
            tables::push_implicit(cp, out);
        }
    }

    /// A precomposed Hangul syllable without an entry of its own: decompose into jamo,
    /// using jamo contractions where defined.
    fn push_hangul(&self, cp: u32, out: &mut Vec<Ce>) {
        let s = cp - 0xAC00;
        let l = 0x1100 + s / 588;
        let v = 0x1161 + (s % 588) / 28;
        let t = s % 28;
        let jamo = |cps: &[u32], out: &mut Vec<Ce>| {
            if cps.len() == 1 {
                self.push_single(cps[0], out);
            } else if let Some(c) = self.contraction(cps) {
                out.extend_from_slice(c);
            }
        };
        if t == 0 {
            if self.contraction(&[l, v]).is_some() {
                jamo(&[l, v], out);
            } else {
                jamo(&[l], out);
                jamo(&[v], out);
            }
            return;
        }
        let t = 0x11A7 + t;
        if self.maxlen(l) > 1 {
            if self.contraction(&[l, v, t]).is_some() {
                return jamo(&[l, v, t], out);
            }
            if self.contraction(&[l, v]).is_some() {
                jamo(&[l, v], out);
                return jamo(&[t], out);
            }
        }
        jamo(&[l], out);
        if self.maxlen(v) > 1 && self.contraction(&[v, t]).is_some() {
            return jamo(&[v, t], out);
        }
        jamo(&[v], out);
        jamo(&[t], out);
    }

    // ------------------------------------------------------------------ algorithm

    fn normalize_into(&self, input: &str, cps: &mut Vec<u32>) {
        cps.clear();
        match self.normalization {
            Normalization::Prenormalized | Normalization::None => {
                cps.extend(input.chars().map(u32::from));
            }
            Normalization::Nfd => cps.extend(input.nfd().map(u32::from)),
            Normalization::Nfc => cps.extend(input.nfc().map(u32::from)),
            Normalization::Nfkd => cps.extend(input.nfkd().map(u32::from)),
            Normalization::Nfkc => cps.extend(input.nfkc().map(u32::from)),
        }
    }

    fn normalize_cps_into(&self, input: &[u32], cps: &mut Vec<u32>) {
        cps.clear();
        if matches!(
            self.normalization,
            Normalization::Prenormalized | Normalization::None
        ) {
            cps.extend_from_slice(input);
            return;
        }
        // Normalise runs of valid scalar values; surrogates and out-of-range values are
        // starters without decompositions and pass through unchanged.
        let mut run = String::new();
        for &cp in input {
            match char::from_u32(cp) {
                Some(c) => run.push(c),
                None => {
                    self.flush_run(&run, cps);
                    run.clear();
                    cps.push(cp);
                }
            }
        }
        self.flush_run(&run, cps);
    }

    fn flush_run(&self, run: &str, cps: &mut Vec<u32>) {
        let mut tmp = Vec::new();
        self.normalize_into(run, &mut tmp);
        cps.extend_from_slice(&tmp);
    }

    /// Code points → collation elements (UTS #10 S2: contractions incl. discontiguous
    /// ones, derived elements, Hangul terminators).
    fn collation_elements(&self, cps: &mut [u32], key: &mut Vec<u32>, out: &mut Vec<Ce>) {
        let n = cps.len();
        let discontiguous = self.normalization != Normalization::None;
        let mut i = 0;
        while i < n {
            let cp = cps[i];
            if cp == REMOVED {
                i += 1;
                continue;
            }
            let (single, ml) = self.info(cp);
            let mut end = i;
            let mut found: Option<&[Ce]> = None;
            if ml > 1 {
                key.clear();
                key.push(cp);
                let mut best = 1;
                let mut p = i + 1;
                while key.len() < ml && p < n {
                    if cps[p] != REMOVED {
                        key.push(cps[p]);
                        if let Some(c) = self.contraction(key) {
                            found = Some(c);
                            end = p;
                            best = key.len();
                        }
                    }
                    p += 1;
                }
                key.truncate(best);
                if discontiguous {
                    let mut pre_cc = 0u8;
                    let mut p = end + 1;
                    while p < n {
                        let c = cps[p];
                        if c != REMOVED {
                            let cc = ccc(c);
                            if cc == 0 {
                                break;
                            }
                            let mut hit = None;
                            if pre_cc != cc {
                                key.push(c);
                                hit = self.contraction(key);
                                if hit.is_none() {
                                    key.pop();
                                }
                            }
                            match hit {
                                Some(h) => {
                                    found = Some(h);
                                    cps[p] = REMOVED;
                                }
                                None => pre_cc = cc,
                            }
                        }
                        p += 1;
                    }
                }
            }
            match (found, single) {
                (Some(c), _) => out.extend_from_slice(c),
                // U+FFFE/U+FFFF may be overridden by minimalFFFE/highestFFFF
                (None, Some(c)) if cp != 0xFFFE && cp != 0xFFFF => out.extend_from_slice(c),
                _ => self.push_single(cp, out),
            }
            if self.hangul_terminator != 0 {
                let last = if found.is_some() { cps[end] } else { cp };
                let next = cps[end + 1..].iter().copied().find(|&c| c != REMOVED);
                if self.ends_syllable(last, next) {
                    out.push(Ce::new(self.hangul_terminator, 0, 0, 0, false));
                }
            }
            i = end + 1;
        }
    }

    fn ends_syllable(&self, cur: u32, next: Option<u32>) -> bool {
        let next = next.map_or(Hst::None, hst);
        match hst(cur) {
            Hst::L => !matches!(next, Hst::L | Hst::V | Hst::Lv | Hst::Lvt),
            Hst::V | Hst::Lv => !matches!(next, Hst::V | Hst::T),
            Hst::T | Hst::Lvt => next != Hst::T,
            _ => false,
        }
    }

    /// Apply `ignore_level2` and variable weighting in place (UTS #10 §4).
    fn weigh(&self, ces: &mut [Ce]) {
        if self.ignore_level2 {
            for ce in ces.iter_mut() {
                if ce.p == 0 && ce.s != 0 {
                    ce.s = 0;
                    ce.t = 0;
                }
            }
        }
        let mut after_variable = false;
        match self.variable {
            Variable::NonIgnorable => {}
            Variable::Blanked => {
                for ce in ces.iter_mut() {
                    if ce.var {
                        after_variable = true;
                        *ce = Ce::new(0, 0, 0, ce.q, false);
                    } else if ce.p == 0 {
                        if after_variable {
                            *ce = Ce::default();
                        }
                    } else {
                        after_variable = false;
                    }
                }
            }
            Variable::Shifted | Variable::ShiftTrimmed => {
                let ffff = if self.variable == Variable::Shifted {
                    0xFFFF
                } else {
                    0
                };
                for ce in ces.iter_mut() {
                    if ce.var {
                        after_variable = true;
                        *ce = Ce::new(0, 0, 0, ce.p, false);
                    } else if ce.p == 0 && after_variable {
                        *ce = Ce::default();
                    } else {
                        if ce.p != 0 {
                            after_variable = false;
                        }
                        ce.q = if ce.p == 0x0001 {
                            // minimalFFFE's [.0001.0020.0005]: under `shifted` level 4
                            // gets the minimal weight too (trimmed by shift-trimmed)
                            ffff & 0x0001
                        } else if ce.s != 0 || ce.t != 0 {
                            ffff
                        } else {
                            0
                        };
                    }
                }
            }
        }
    }

    #[inline]
    fn tertiary(&self, t: u16) -> u16 {
        let mut t = t;
        if self.upper_before_lower == Some(true) {
            t = match t {
                0x02..=0x06 => t + 6,
                0x08..=0x0C => t - 6,
                0x1C => 0x1D,
                0x1D => 0x1C,
                _ => t,
            };
        }
        if self.katakana_before_hiragana {
            t = match t {
                0x0D | 0x0E => t + 5,
                0x0F..=0x13 => t - 2,
                _ => t,
            };
        }
        t
    }

    #[inline]
    fn write_level(&self, ces: &[Ce], level: u8, w: impl Fn(&Ce) -> u16, out: &mut Vec<u8>) {
        let push = |out: &mut Vec<u8>, ce: &Ce| {
            let x = w(ce);
            if x != 0 {
                out.extend_from_slice(&x.to_be_bytes());
            }
        };
        if self.backwards & (1 << (level - 1)) != 0 {
            ces.iter().rev().for_each(|ce| push(out, ce));
        } else {
            ces.iter().for_each(|ce| push(out, ce));
        }
    }

    fn write_key(&self, ces: &[Ce], ident: Option<&[u32]>, out: &mut Vec<u8>) {
        out.reserve(ces.len() * 8 + 8);
        self.write_level(ces, 1, |ce| ce.p, out);
        out.extend_from_slice(&[0, 0]);
        if self.level >= 2 {
            self.write_level(ces, 2, |ce| ce.s, out);
        }
        out.extend_from_slice(&[0, 0]);
        if self.level >= 3 {
            if self.upper_before_lower == Some(true) || self.katakana_before_hiragana {
                self.write_level(ces, 3, |ce| self.tertiary(ce.t), out);
            } else {
                self.write_level(ces, 3, |ce| ce.t, out);
            }
        }
        out.extend_from_slice(&[0, 0]);
        if self.level >= 4 {
            self.write_level(ces, 4, |ce| ce.q, out);
        }
        if self.level == 4 || self.identical {
            out.extend_from_slice(&[0, 0]);
        }
        if let Some(cps) = ident {
            for &cp in cps {
                out.extend_from_slice(&cp.to_be_bytes());
            }
        }
    }

    fn key_from_scratch(&self, sc: &mut Scratch, out: &mut Vec<u8>) {
        let Scratch {
            cps,
            orig,
            key,
            ces,
        } = sc;
        if self.identical {
            orig.clear();
            orig.extend_from_slice(cps);
        }
        for cp in cps.iter_mut() {
            if *cp > OUT_OF_RANGE {
                *cp = OUT_OF_RANGE;
            }
        }
        ces.clear();
        self.collation_elements(cps, key, ces);
        self.weigh(ces);
        self.write_key(ces, self.identical.then_some(&orig[..]), out);
    }

    /// Append the sort key of `s` (`getSortKey`) to `out`.
    pub fn sort_key_into(&self, s: &str, out: &mut Vec<u8>) {
        SCRATCH.with(|sc| {
            let mut sc = sc.borrow_mut();
            let mut cps = std::mem::take(&mut sc.cps);
            self.normalize_into(s, &mut cps);
            sc.cps = cps;
            self.key_from_scratch(&mut sc, out);
        });
    }

    /// The sort key of `s`, byte-identical to `Unicode::Collate::getSortKey`.
    /// Compare sort keys bytewise.
    pub fn sort_key(&self, s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        self.sort_key_into(s, &mut out);
        out
    }

    /// The sort key of a sequence of code points, which (unlike `&str`) may contain
    /// surrogates or values above U+10FFFF, as Perl strings can.
    pub fn sort_key_from_code_points(&self, cps: &[u32]) -> Vec<u8> {
        let mut out = Vec::new();
        SCRATCH.with(|sc| {
            let mut sc = sc.borrow_mut();
            let mut buf = std::mem::take(&mut sc.cps);
            self.normalize_cps_into(cps, &mut buf);
            sc.cps = buf;
            self.key_from_scratch(&mut sc, &mut out);
        });
        out
    }

    /// Compare two strings (`cmp`).
    pub fn cmp(&self, a: &str, b: &str) -> Ordering {
        self.sort_key(a).cmp(&self.sort_key(b))
    }

    /// The textual form of the sort key of `s`, identical to `viewSortKey`, e.g.
    /// `"[1FA2 | 0020 | 0002 | FFFF |]"`.
    pub fn view_sort_key(&self, s: &str) -> String {
        view_key(&self.sort_key(s))
    }
}

/// Render a sort key like `Unicode::Collate::viewSortKey` (UCA_Version ≥ 9): 16-bit
/// weights in hex, the first four level separators as `|`.
pub fn view_key(key: &[u8]) -> String {
    let mut s = String::with_capacity(key.len() * 5 / 2 + 2);
    s.push('[');
    let mut seps = 0;
    for (i, w) in key.chunks(2).enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let w = u16::from_be_bytes([w[0], *w.get(1).unwrap_or(&0)]);
        if w == 0 && seps < 4 {
            seps += 1;
            s.push('|');
        } else {
            use std::fmt::Write;
            let _ = write!(s, "{w:04X}");
        }
    }
    s.push(']');
    s
}

//! User-supplied perl regexes, evaluated with PCRE2 (10.46, Unicode 16.0,
//! UTF + UCP mode) behind a perl-syntax translation layer
//! ([`translate`](translate::translate)), with perl's matching-loop semantics
//! re-implemented on top.
//!
//! biber call sites:
//! * sourcemap `map_match`/`map_matchi`/`map_notmatch`/`map_notmatchi` with
//!   `map_replace`: [`ireplace`] (`Utils.pm:1173-1185`, from
//!   `bibtex.pm:784` and `biblatexml.pm:671`);
//! * the same without replace: [`imatch`] (`Utils.pm:1148-1162`, from
//!   `bibtex.pm:793`, `biblatexml.pm:682` and datamodel pattern constraints
//!   `DataModel.pm:948`), with [`expand_imatches`] for the `$N` substitution
//!   into later patterns/keys/values (`bibtex.pm:452, 493, 529, 792, 912`);
//! * `nosort`, `nonamestring`, `nolabel`, `noinit` (compiled once with
//!   `qr/$re/` in `Biber.pm:4433-4462`; applied with `s/$re//gxms`, where the
//!   outer `/xms` do not affect an interpolated `qr` object):
//!   [`PerlRegex::remove_all`];
//! * `xsvsep` (`split(/$S/, $value)`, `bibtex.pm:969-971, 1130-1133,
//!   1567-1569`, `biblatexml.pm:889`): [`PerlRegex::split`].
//!
//! Perl semantics reproduced: `qr` stringification `(?^u:…)`/`(?^ui:…)`;
//! `m//g` in list context (all groups of all matches, `undef` for groups
//! that did not participate, or the whole matches if there are no groups);
//! `s///g` and `m//g` iteration where a match may not be empty at the
//! position where the previous *empty* match ended; `split` with limit 0
//! (a match must end after the current position; captured groups are
//! returned; trailing empty fields removed; `^` means `^` with `/m`).
//!
//! Known differences (documented in the final report): perl's `/i` uses full
//! case folding (`/strasse/i` matches `straße`), PCRE2 simple folding;
//! `\N{NAME}` and `\b{…}` are unsupported; PCRE2's property tables are its
//! own Unicode 16.0 tables.

mod interp;
mod translate;

pub use interp::{InterpError, MatchCtx, Template, quotemeta};
pub use translate::translate;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use pcre2::bytes::{CaptureLocations, Regex, RegexBuilder};

/// A regex error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegexError {
    /// The pattern does not compile (perl would die).
    Compile(String),
    /// Valid perl that bibers does not support.
    Unsupported(String),
    /// PCRE2 failed while matching (e.g. backtracking limit).
    Match(String),
}

impl std::fmt::Display for RegexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegexError::Compile(m) => write!(f, "invalid regular expression: {m}"),
            RegexError::Unsupported(m) => write!(f, "unsupported regular expression feature: {m}"),
            RegexError::Match(m) => write!(f, "regular expression match failed: {m}"),
        }
    }
}

impl std::error::Error for RegexError {}

/// A compiled user regex: perl `qr/$pattern/` (or `qr/$pattern/i`).
#[derive(Debug)]
pub struct PerlRegex {
    src: String,
    ci: bool,
    /// The translated pattern.
    translated: String,
    re: Regex,
    /// The same pattern, but a match may not end at the start offset
    /// (perl's `minend`/`NOTEMPTY_ATSTART` rule); compiled on first use.
    re_noempty: OnceLock<Result<Regex, RegexError>>,
    /// `split /^/` is `split /^/m`.
    re_split: Option<Regex>,
}

fn build(pat: &str, ci: bool, multiline: bool) -> Result<Regex, RegexError> {
    RegexBuilder::new()
        .utf(true)
        .ucp(true)
        .caseless(ci)
        .multi_line(multiline)
        .jit_if_available(true)
        .build(pat)
        .map_err(|e| RegexError::Compile(e.to_string()))
}

/// A match: byte spans of group 0..=n.
pub type Groups = Vec<Option<(usize, usize)>>;

impl PerlRegex {
    /// `qr/$pattern/` (`ci = false`) or `qr/$pattern/i`.
    pub fn new(pattern: &str, ci: bool) -> Result<PerlRegex, RegexError> {
        let t = translate(pattern, ci)?;
        let re = build(&t, ci, false)?;
        let re_split = if pattern == "^" {
            Some(build("(?:^)(?!\\G)", ci, true)?)
        } else {
            None
        };
        Ok(PerlRegex {
            src: pattern.to_string(),
            ci,
            translated: t,
            re,
            re_noempty: OnceLock::new(),
            re_split,
        })
    }

    /// A shared compiled copy of `qr/$pattern/` from a process-wide cache
    /// (biber recompiles user regexes for every entry; compiling PCRE2
    /// patterns with perl's large `\w` class is comparatively expensive).
    pub fn cached(pattern: &str, ci: bool) -> Result<Arc<PerlRegex>, RegexError> {
        type Cache = Mutex<HashMap<(String, bool), Arc<PerlRegex>>>;
        static CACHE: OnceLock<Cache> = OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        let key = (pattern.to_string(), ci);
        if let Some(r) = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
        {
            return Ok(Arc::clone(r));
        }
        let re = Arc::new(PerlRegex::new(pattern, ci)?);
        let mut g = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if g.len() > 4096 {
            g.clear();
        }
        g.insert(key, Arc::clone(&re));
        Ok(re)
    }

    fn noempty(&self) -> Result<&Regex, RegexError> {
        self.re_noempty
            .get_or_init(|| build(&format!("(?:{})(?!\\G)", self.translated), self.ci, false))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The pattern as given.
    pub fn as_str(&self) -> &str {
        &self.src
    }

    /// Perl's stringification of the `qr` object: `(?^u:PAT)` or
    /// `(?^ui:PAT)` (biber inserts this when a replacement fails).
    pub fn stringify(&self) -> String {
        format!("(?^u{}:{})", if self.ci { "i" } else { "" }, self.src)
    }

    /// Number of capture groups.
    pub fn captures_len(&self) -> usize {
        self.re.captures_len() - 1
    }

    fn find_at(&self, re: &Regex, s: &str, pos: usize) -> Result<Option<Groups>, RegexError> {
        let mut locs: CaptureLocations = re.capture_locations();
        match re.captures_read_at(&mut locs, s.as_bytes(), pos) {
            Ok(Some(_)) => Ok(Some((0..locs.len()).map(|i| locs.get(i)).collect())),
            Ok(None) => Ok(None),
            Err(e) => Err(RegexError::Match(e.to_string())),
        }
    }

    /// `$s =~ /$re/`.
    pub fn is_match(&self, s: &str) -> Result<bool, RegexError> {
        self.re
            .is_match(s.as_bytes())
            .map_err(|e| RegexError::Match(e.to_string()))
    }

    /// All matches of `m//g` / `s///g`, with perl's empty-match rule.
    pub fn matches(&self, s: &str) -> Result<Vec<Groups>, RegexError> {
        let mut out = Vec::new();
        let mut pos = 0;
        let mut prev_empty = false;
        while pos <= s.len() {
            let re = if prev_empty {
                self.noempty()?
            } else {
                &self.re
            };
            let Some(g) = self.find_at(re, s, pos)? else {
                break;
            };
            let (st, en) = g[0].unwrap_or((pos, pos));
            prev_empty = st == en;
            pos = en;
            out.push(g);
        }
        Ok(out)
    }

    /// `$s =~ m/$re/g` in list context.
    pub fn match_all(&self, s: &str) -> Result<Vec<Option<String>>, RegexError> {
        let n = self.captures_len();
        let mut out = Vec::new();
        for g in self.matches(s)? {
            if n == 0 {
                out.push(g[0].map(|(a, b)| s[a..b].to_string()));
            } else {
                out.extend(g[1..].iter().map(|x| x.map(|(a, b)| s[a..b].to_string())));
            }
        }
        Ok(out)
    }

    /// `$s =~ s/$re//g` (the `nosort`/`noinit`/`nolabel`/`nonamestring`
    /// stripping).
    pub fn remove_all(&self, s: &str) -> Result<String, RegexError> {
        let ms = self.matches(s)?;
        if ms.is_empty() {
            return Ok(s.to_string());
        }
        let mut out = String::with_capacity(s.len());
        let mut last = 0;
        for g in ms {
            let (a, b) = g[0].unwrap_or((last, last));
            out.push_str(&s[last..a]);
            last = b;
        }
        out.push_str(&s[last..]);
        Ok(out)
    }

    /// `$s =~ s/$re/REPL/g` where the replacement for each match is computed
    /// by `f`.
    pub fn replace_all_with(
        &self,
        s: &str,
        mut f: impl FnMut(&Groups) -> String,
    ) -> Result<String, RegexError> {
        let ms = self.matches(s)?;
        if ms.is_empty() {
            return Ok(s.to_string());
        }
        let mut out = String::with_capacity(s.len());
        let mut last = 0;
        for g in &ms {
            let (a, b) = g[0].unwrap_or((last, last));
            out.push_str(&s[last..a]);
            out.push_str(&f(g));
            last = b;
        }
        out.push_str(&s[last..]);
        Ok(out)
    }

    /// `split(/$re/, $s)` (limit 0): fields and captured groups (`None` =
    /// `undef`), trailing empty/undef fields removed.
    pub fn split(&self, s: &str) -> Result<Vec<Option<String>>, RegexError> {
        let re = match &self.re_split {
            Some(r) => r,
            None => self.noempty()?,
        };
        let mut out: Vec<Option<String>> = Vec::new();
        let mut pos = 0;
        while pos < s.len() {
            let Some(g) = self.find_at(re, s, pos)? else {
                break;
            };
            let (a, b) = g[0].unwrap_or((pos, pos));
            out.push(Some(s[pos..a].to_string()));
            out.extend(g[1..].iter().map(|x| x.map(|(p, q)| s[p..q].to_string())));
            pos = b;
        }
        if pos < s.len() {
            out.push(Some(s[pos..].to_string()));
        }
        while out
            .last()
            .is_some_and(|f| f.as_deref().is_none_or(str::is_empty))
        {
            out.pop();
        }
        Ok(out)
    }
}

/// Result of [`ireplace`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replaced {
    /// The new value.
    pub value: String,
    /// Set when the replacement contained perl code that bibers does not
    /// execute (DC-023); the caller should warn.
    pub code_warning: Option<String>,
}

/// `Biber::Utils::ireplace($value, $val_match, $val_replace, $ci)`.
///
/// An empty pattern returns the value unchanged. If the replacement string
/// fails to evaluate (syntax error, or code — see [`InterpError`]), every
/// match is replaced by the stringified regex, as biber does.
pub fn ireplace(
    value: &str,
    pattern: &str,
    replace: &str,
    ci: bool,
) -> Result<Replaced, RegexError> {
    if pattern.is_empty() {
        return Ok(Replaced {
            value: value.to_string(),
            code_warning: None,
        });
    }
    let re = PerlRegex::cached(pattern, ci)?;
    let tpl = Template::parse(replace);
    let code_warning = match &tpl {
        Err(InterpError::Code(m)) => Some(m.clone()),
        _ => None,
    };
    let fallback = re.stringify();
    let value = re.replace_all_with(value, |g| match &tpl {
        Ok(t) => t.eval(&MatchCtx {
            subject: value,
            groups: g,
        }),
        Err(_) => fallback.clone(),
    })?;
    Ok(Replaced {
        value,
        code_warning,
    })
}

/// `Biber::Utils::imatch($value, $val_match, $negmatch, $ci)` in list
/// context (`@imatches = imatch(…)`).
///
/// * A perl-false pattern (`""` or `"0"`) returns `(0)`: one element `"0"`,
///   which biber treats as a successful match with `$1 = "0"`.
/// * `negmatch`: `()` if the pattern matches, else `(1)` (so `$1` is `"1"`
///   afterwards, BB-032).
/// * Otherwise the list of `m//g` (see [`PerlRegex::match_all`]).
pub fn imatch(
    value: &str,
    pattern: &str,
    negmatch: bool,
    ci: bool,
) -> Result<Vec<Option<String>>, RegexError> {
    if !crate::truth::perl_truthy(pattern) {
        return Ok(vec![Some("0".to_string())]);
    }
    let re = PerlRegex::cached(pattern, ci)?;
    if negmatch {
        return Ok(if re.is_match(value)? {
            vec![]
        } else {
            vec![Some("1".to_string())]
        });
    }
    re.match_all(value)
}

/// Whether `$N` digits are a single digit (`(?<!\\)\$(\d)`, used for new
/// entry keys, clone keys, entry targets and field values) or a run
/// (`(?<!\\)\$(\d+)`, used for later match patterns).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureDigits {
    /// `\$(\d)`
    Single,
    /// `\$(\d+)`
    Multi,
}

/// biber's `s/(?<!\\)\$(\d+)/$imatches[$1-1]/ge` (BB-032): replace every `$N`
/// not preceded by a backslash with element `N-1` of the previous
/// `imatch` result, unescaped. `$0` is element -1, i.e. the *last* capture;
/// out-of-range or `undef` elements give `""`. `\d` is perl's Unicode digit
/// class; non-ASCII digits numify to 0 (so they also select the last
/// element).
pub fn expand_imatches(
    template: &str,
    imatches: &[Option<String>],
    digits: CaptureDigits,
) -> String {
    let mut out = String::with_capacity(template.len());
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$'
            && (i == 0 || chars[i - 1] != '\\')
            && chars.get(i + 1).is_some_and(|&d| is_perl_digit(d))
        {
            let k = match digits {
                CaptureDigits::Single => 1,
                CaptureDigits::Multi => chars[i + 1..]
                    .iter()
                    .take_while(|&&d| is_perl_digit(d))
                    .count(),
            };
            let num: String = chars[i + 1..i + 1 + k].iter().collect();
            let n = crate::truth::PerlNum::parse(&num).minus(crate::truth::PerlNum::Int(1));
            let idx = match n {
                crate::truth::PerlNum::Int(x) => x,
                _ => i64::MAX,
            };
            let len = imatches.len() as i64;
            let real = if idx < 0 { len + idx } else { idx };
            if (0..len).contains(&real) {
                if let Some(v) = &imatches[real as usize] {
                    out.push_str(v);
                }
            }
            i += 1 + k;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

use crate::unicode::is_perl_digit;

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(v: &str, m: &str, r: &str) -> String {
        ireplace(v, m, r, false).unwrap().value
    }

    #[test]
    fn replace() {
        assert_eq!(rep("abc", "(b)", "[$1]"), "a[b]c");
        assert_eq!(rep("abc", "(b)", "$&-$`-$'"), "ab-a-cc");
        assert_eq!(rep("abc", "(b)", "a\"b"), "a(?^u:(b))c");
        assert_eq!(rep("abc", "(b)", "a@b.c"), "aa.cc");
        assert_eq!(rep("aaa", "a*", "X"), "XX");
        assert_eq!(rep("abc", "x*", "-"), "-a-b-c-");
        assert_eq!(rep("abc", "(b)", "\\L$1\\E\\u$1"), "abBc");
        assert_eq!(rep("aBc", "(B)", "\\u\\LABC"), "aAbcc");
        assert_eq!(rep("aBc", "(B)", "\\Uab\\Lcd"), "aABcdc");
        assert_eq!(rep("aBc", "(B)", "\\Qa.b\\E."), "aa\\.b.c");
        assert_eq!(rep("aBc", "(B)", "@-"), "a11c");
        assert_eq!(rep("aUc", "\\Uc", "X"), "aX");
        assert_eq!(ireplace("abc", "B", "X", true).unwrap().value, "aXc");
        let r = ireplace("abc", "(b)", "@{[ uc $1 ]}", false).unwrap();
        assert_eq!(r.value, "a(?^u:(b))c");
        assert!(r.code_warning.is_some());
    }

    #[test]
    fn matching_and_split() {
        assert_eq!(
            imatch("a1b2", "(\\d)", false, false).unwrap(),
            [Some("1".into()), Some("2".into())]
        );
        assert_eq!(
            imatch("ab", "(a)|(b)", false, false).unwrap(),
            [Some("a".into()), None, None, Some("b".into())]
        );
        assert_eq!(imatch("x", "y", true, false).unwrap(), [Some("1".into())]);
        assert_eq!(imatch("x", "0", false, false).unwrap(), [Some("0".into())]);
        let re = PerlRegex::new(r"\s*,\s*", false).unwrap();
        let v: Vec<_> = re.split(" a , b,,c,,").unwrap();
        assert_eq!(
            v,
            [
                Some(" a".into()),
                Some("b".into()),
                Some("".into()),
                Some("c".into())
            ]
        );
        let re = PerlRegex::new("x*", false).unwrap();
        assert_eq!(
            re.split("axxbc").unwrap(),
            [Some("a".into()), Some("b".into()), Some("c".into())]
        );
        let re = PerlRegex::new("^", false).unwrap();
        assert_eq!(
            re.split("a\nb").unwrap(),
            [Some("a\n".into()), Some("b".into())]
        );
        assert_eq!(PerlRegex::new("x", true).unwrap().stringify(), "(?^ui:x)");
    }

    #[test]
    fn imatches_expansion() {
        let m = vec![Some("a".to_string()), None, Some("c".to_string())];
        assert_eq!(
            expand_imatches("$1-$2-$3-$0-\\$1-$4", &m, CaptureDigits::Multi),
            "a--c-c-\\$1-"
        );
        assert_eq!(expand_imatches("$13", &m, CaptureDigits::Single), "a3");
    }
}

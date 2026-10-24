//! Perl's `glob`, as biber's `globU` calls it for `--glob-datasources` and
//! `\addbibresource[glob]`: File::Glob's `csh_glob` with its default flags
//! `GLOB_CSH` (brace expansion, `NOMAGIC`, backslash quoting, tilde
//! expansion, case-insensitive sort; case-insensitive matching on Windows).
//! Ported from perl 5.42's `ext/File-Glob` (`Glob.xs` `csh_glob`,
//! `bsd_glob.c`), under the same terms as Perl (see NOTICE.md).
//!
//! Patterns and file names are handled as bytes (biber globs UTF-8 encoded
//! names), so `?` matches one byte, as in perl.
//!
//! Difference: `~user` is left unexpanded (biber looks the user's home up in
//! the password database; docs/compat.md DC-070). `~` and `~/…` use `$HOME`.

use crate::env::Env;

/// A pattern byte that a backslash protected (`M_PROTECT`): it is matched
/// literally and is never a brace, bracket, wildcard or tilde.
const PROTECT: u16 = 0x100;

const fn b(c: u8) -> u16 {
    c as u16
}

/// `glob($pattern)` relative to the current directory of `env`.
pub fn glob(env: &Env, pattern: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in csh_words(pattern) {
        out.extend(bsd_glob(env, &w));
    }
    out
}

/// Perl's `isSPACE` on an ASCII byte.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// `csh_glob`'s tokenizer: split on whitespace; `'…'` and `"…"` group (the
/// quotes are removed, wildcards inside stay active); a backslash keeps the
/// next character in the word, and is removed before a quote. With an
/// unmatched quote, the whole pattern (trimmed) becomes one more word.
fn csh_words(pat: &str) -> Vec<Vec<u8>> {
    let s = pat.as_bytes();
    let n = s.len();
    let mut words: Vec<Vec<u8>> = Vec::new();
    let mut word: Option<Vec<u8>> = None;
    let mut piece: Option<usize> = None;
    let mut i = 0;
    while i < n {
        match s[i] {
            q @ (b'\'' | b'"') => {
                let w = word.get_or_insert_with(Vec::new);
                if let Some(p) = piece {
                    w.extend_from_slice(&s[p..i]);
                }
                let mut p = i + 1;
                let mut found = false;
                loop {
                    i += 1;
                    if i >= n {
                        break;
                    }
                    if s[i] == b'\\' {
                        i += 1;
                        // a backslash escaping the quote is obliterated
                        if i < n && s[i] == q {
                            w.extend_from_slice(&s[p..i - 1]);
                            p = i;
                        }
                    } else if s[i] == q {
                        w.extend_from_slice(&s[p..i]);
                        found = true;
                        break;
                    }
                }
                piece = None;
                if !found {
                    let t = pat.trim_matches(|c: char| c.is_ascii() && is_space(c as u8));
                    words.push(t.as_bytes().to_vec());
                    return words;
                }
            }
            b'\\' => {
                let p = *piece.get_or_insert(i);
                i += 1;
                if i < n && (s[i] == b'"' || s[i] == b'\'') {
                    word.get_or_insert_with(Vec::new)
                        .extend_from_slice(&s[p..i - 1]);
                    piece = Some(i);
                }
            }
            c if is_space(c) => {
                if let Some(p) = piece.take() {
                    word.get_or_insert_with(Vec::new)
                        .extend_from_slice(&s[p..i]);
                }
                if let Some(w) = word.take() {
                    words.push(w);
                }
            }
            _ => {
                piece.get_or_insert(i);
            }
        }
        i += 1;
    }
    if piece.is_some() || word.is_some() {
        let mut w = word.unwrap_or_default();
        if let Some(p) = piece {
            w.extend_from_slice(&s[p..n]);
        }
        words.push(w);
    }
    words
}

/// `bsd_glob` on one word: backslash quoting (`GLOB_QUOTE`), then brace
/// expansion and matching.
fn bsd_glob(env: &Env, word: &[u8]) -> Vec<String> {
    let mut pat: Vec<u16> = Vec::with_capacity(word.len());
    let mut i = 0;
    while i < word.len() {
        let c = word[i];
        i += 1;
        if c != b'\\' {
            pat.push(b(c));
            continue;
        }
        // On Windows a backslash only quotes `[]-{}~\` (it is a separator).
        let quotes = |n: u8| !cfg!(windows) || b"[]-{}~\\".contains(&n);
        match word.get(i) {
            Some(&n) if quotes(n) => {
                pat.push(b(n) | PROTECT);
                i += 1;
            }
            _ => pat.push(b(b'\\') | PROTECT),
        }
    }
    let mut out = Vec::new();
    globexp1(env, &pat, &mut out);
    out
}

/// `globexp1`: expand the first unprotected `{`, or match.
fn globexp1(env: &Env, pat: &[u16], out: &mut Vec<String>) {
    // a lone `{}` is literal, like csh
    if pat == [b(b'{'), b(b'}')] {
        return glob0(env, pat, out);
    }
    match pat.iter().position(|&c| c == b(b'{')) {
        Some(p) => globexp2(env, pat, p, out),
        None => glob0(env, pat, out),
    }
}

/// `globexp2`: expand the `{…}` starting at `open`. `[…]` groups are skipped
/// when looking for commas and the closing brace. With no balanced closing
/// brace, only the part before `{` is globbed (as in `bsd_glob.c`).
fn globexp2(env: &Env, pat: &[u16], open: usize, out: &mut Vec<String>) {
    let len = pat.len();
    // the `]` closing a `[` at `at`, if any
    let close_bracket = |at: usize| (at + 1..len).find(|&j| pat[j] == b(b']'));
    let mut depth = 0;
    let mut pe = open + 1;
    while pe < len {
        let c = pat[pe];
        if c == b(b'[') {
            if let Some(j) = close_bracket(pe) {
                pe = j;
            }
        } else if c == b(b'{') {
            depth += 1;
        } else if c == b(b'}') {
            if depth == 0 {
                break;
            }
            depth -= 1;
        }
        pe += 1;
    }
    if depth != 0 || pe >= len {
        return glob0(env, &pat[..open], out);
    }
    let mut depth = 0;
    let mut pl = open + 1;
    let mut pm = open + 1;
    while pm <= pe {
        let c = pat[pm];
        if c == b(b'[') {
            if let Some(j) = close_bracket(pm) {
                pm = j;
            }
        } else if c == b(b'{') {
            depth += 1;
        } else if c == b(b'}') && depth > 0 {
            depth -= 1;
        } else if (c == b(b'}') || c == b(b',')) && depth == 0 {
            let mut np = pat[..open].to_vec();
            np.extend_from_slice(&pat[pl..pm]);
            np.extend_from_slice(&pat[pe + 1..]);
            globexp1(env, &np, out);
            pl = pm + 1;
        }
        pm += 1;
    }
}

/// `globtilde`: an unprotected leading `~` or `~/` becomes `$HOME`
/// (`%USERPROFILE%` on Windows when `$HOME` is unset). `~user` is left as is.
fn globtilde(env: &Env, pat: &[u16]) -> Vec<u16> {
    if pat.first() != Some(&b(b'~')) {
        return pat.to_vec();
    }
    let end = pat.iter().position(|&c| c == b(b'/')).unwrap_or(pat.len());
    if end != 1 {
        return pat.to_vec();
    }
    let home = env.home.clone().or_else(|| {
        if cfg!(windows) {
            std::env::var("USERPROFILE").ok()
        } else {
            None
        }
    });
    let Some(home) = home else {
        return pat.to_vec();
    };
    let mut q: Vec<u16> = home.bytes().map(b).collect();
    q.extend_from_slice(&pat[end..]);
    q
}

/// A compiled pattern element (`M_ALL`, `M_ONE`, `M_SET`, a literal byte).
#[derive(Debug, PartialEq)]
enum Tok {
    Lit(u8),
    All,
    One,
    /// Negated?, and inclusive byte ranges.
    Set(bool, Vec<(u8, u8)>),
}

impl Tok {
    fn is_sep(&self) -> bool {
        matches!(self, Tok::Lit(b'/')) || (cfg!(windows) && matches!(self, Tok::Lit(b'\\')))
    }
}

/// `glob0`: tilde expansion, compilation, matching; `NOMAGIC` (a pattern
/// without wildcards is returned as is when nothing matches); each call's
/// matches are sorted case-insensitively (`ci_compare`).
fn glob0(env: &Env, pat: &[u16], out: &mut Vec<String>) {
    let qpat = globtilde(env, pat);
    let len = qpat.len();
    let lit = |c: u16| c as u8; // `CHAR(c)`: the byte, protection dropped
    let mut toks: Vec<Tok> = Vec::new();
    let mut magic = false;
    let mut i = 0;
    while i < len {
        let c = qpat[i];
        i += 1;
        if c == b(b'[') {
            let mut j = i;
            let neg = qpat.get(j) == Some(&b(b'!'));
            if neg {
                j += 1;
            }
            if j >= len || !qpat[j + 1..].contains(&b(b']')) {
                toks.push(Tok::Lit(b'['));
                continue;
            }
            let mut items = Vec::new();
            let mut c = qpat[j];
            j += 1;
            loop {
                if qpat.get(j) == Some(&b(b'-')) && qpat.get(j + 1).is_some_and(|&h| h != b(b']')) {
                    items.push((lit(c), lit(qpat[j + 1])));
                    j += 2;
                } else {
                    items.push((lit(c), lit(c)));
                }
                c = qpat[j];
                j += 1;
                if c == b(b']') {
                    break;
                }
            }
            i = j;
            magic = true;
            toks.push(Tok::Set(neg, items));
        } else if c == b(b'?') {
            magic = true;
            toks.push(Tok::One);
        } else if c == b(b'*') {
            magic = true;
            if toks.last() != Some(&Tok::All) {
                toks.push(Tok::All);
            }
        } else {
            toks.push(Tok::Lit(lit(c)));
        }
    }
    let mut found: Vec<Vec<u8>> = Vec::new();
    if !toks.is_empty() {
        glob2(env, Vec::new(), &toks, &mut found);
    }
    if found.is_empty() {
        if !magic {
            out.push(
                String::from_utf8_lossy(&qpat.iter().map(|&c| lit(c)).collect::<Vec<_>>())
                    .into_owned(),
            );
        }
        return;
    }
    found.sort_by(|a, b| {
        a.iter()
            .map(u8::to_ascii_lowercase)
            .cmp(b.iter().map(u8::to_ascii_lowercase))
            .then_with(|| a.cmp(b))
    });
    out.extend(found.into_iter().filter_map(|p| String::from_utf8(p).ok()));
}

fn path_str(path: &[u8]) -> Option<&str> {
    std::str::from_utf8(path).ok()
}

/// `glob2`: copy segments without wildcards; expand the first one with
/// wildcards (`glob3`); at the end, keep the path if it exists (`lstat`).
fn glob2(env: &Env, mut path: Vec<u8>, mut toks: &[Tok], found: &mut Vec<Vec<u8>>) {
    loop {
        if toks.is_empty() {
            if path_str(&path).is_some_and(|p| std::fs::symlink_metadata(env.resolve(p)).is_ok()) {
                found.push(path);
            }
            return;
        }
        let end = toks.iter().position(Tok::is_sep).unwrap_or(toks.len());
        let seg = &toks[..end];
        if seg.iter().any(|t| !matches!(t, Tok::Lit(_))) {
            return glob3(env, &path, seg, &toks[end..], found);
        }
        path.extend(seg.iter().map(|t| match t {
            Tok::Lit(c) => *c,
            _ => unreachable!(),
        }));
        toks = &toks[end..];
        while let Some(Tok::Lit(c)) = toks.first().filter(|t| t.is_sep()) {
            path.push(*c);
            toks = &toks[1..];
        }
    }
}

/// `glob3`: read the directory `path` and recurse on matching names. A
/// leading `.` must be matched literally.
fn glob3(env: &Env, path: &[u8], seg: &[Tok], rest: &[Tok], found: &mut Vec<Vec<u8>>) {
    let Some(dir) = path_str(path) else {
        return;
    };
    let Ok(rd) = std::fs::read_dir(env.resolve(if dir.is_empty() { "." } else { dir })) else {
        return;
    };
    let nocase = cfg!(windows);
    for e in rd.flatten() {
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        let name = name.as_bytes();
        if name.first() == Some(&b'.') && seg.first() != Some(&Tok::Lit(b'.')) {
            continue;
        }
        if matches(name, seg, nocase) {
            let mut p = path.to_vec();
            p.extend_from_slice(name);
            glob2(env, p, rest, found);
        }
    }
}

/// `match` (`bsd_glob.c`): `*` matches leftmost-shortest with backtracking.
fn matches(name: &[u8], pat: &[Tok], nocase: bool) -> bool {
    let fold = |c: u8| if nocase { c.to_ascii_lowercase() } else { c };
    let (mut n, mut p) = (0, 0);
    let mut retry: Option<(usize, usize)> = None; // (pattern at `*`, name)
    'redo: loop {
        while p < pat.len() {
            match &pat[p] {
                Tok::All => {
                    if p + 1 == pat.len() {
                        return true;
                    }
                    if n >= name.len() {
                        return false;
                    }
                    retry = Some((p, n + 1));
                    p += 1;
                }
                Tok::One => {
                    if n >= name.len() {
                        return false;
                    }
                    n += 1;
                    p += 1;
                }
                Tok::Set(neg, items) => {
                    let Some(&k) = name.get(n) else {
                        return false;
                    };
                    n += 1;
                    let ok = items
                        .iter()
                        .any(|&(lo, hi)| fold(lo) <= fold(k) && fold(k) <= fold(hi));
                    if ok == *neg {
                        match retry {
                            Some((rp, rn)) => {
                                (p, n) = (rp, rn);
                                continue 'redo;
                            }
                            None => return false,
                        }
                    }
                    p += 1;
                }
                Tok::Lit(c) => {
                    if name.get(n).map(|&k| fold(k)) != Some(fold(*c)) {
                        match retry {
                            Some((rp, rn)) => {
                                (p, n) = (rp, rn);
                                continue 'redo;
                            }
                            None => return false,
                        }
                    }
                    n += 1;
                    p += 1;
                }
            }
        }
        if n >= name.len() {
            return true;
        }
        match retry {
            Some((rp, rn)) => (p, n) = (rp, rn),
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(p: &str) -> Vec<String> {
        csh_words(p)
            .into_iter()
            .map(|w| String::from_utf8(w).unwrap())
            .collect()
    }

    #[test]
    fn tokenizer() {
        assert_eq!(words("a.bib b.bib"), ["a.bib", "b.bib"]);
        assert_eq!(words("  a.bib  "), ["a.bib"]);
        assert_eq!(words(r"ref\ space*.bib"), [r"ref\ space*.bib"]);
        assert_eq!(words(r#""ref space*.bib""#), ["ref space*.bib"]);
        assert_eq!(words(r#"a"b c"d"#), ["ab cd"]);
        assert_eq!(words(r#""a.bib"b.bib"#), ["a.bibb.bib"]);
        assert_eq!(words(r#"ref" "space1.bib"#), ["ref space1.bib"]);
        assert_eq!(words(r#""a.bib"#), [r#""a.bib"#]);
        assert_eq!(words(r"ref\\ space1.bib"), [r"ref\\", "space1.bib"]);
    }

    /// Vectors recorded with perl 5.42's `glob` (File::Glob 1.42).
    #[test]
    fn matches_perl() {
        let d = tempfile::tempdir().unwrap();
        let cwd = d.path().join("cwd");
        let home = d.path().join("home");
        std::fs::create_dir_all(home.join("sub")).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        for f in [
            "ref space1.bib",
            "ref space2.bib",
            "a.bib",
            "b.bib",
            "x*y.bib",
            "q'uote.bib",
            ".hidden.bib",
        ] {
            std::fs::write(cwd.join(f), "").unwrap();
        }
        for f in ["h1.bib", "h2.bib", "sub/s.bib"] {
            std::fs::write(home.join(f), "").unwrap();
        }
        let mut env = Env::isolated(&cwd);
        env.home = Some(home.to_str().unwrap().to_string());
        let h = home.to_str().unwrap();
        let g = |p: &str| glob(&env, p);
        assert_eq!(g(r"ref\ space*.bib"), ["ref space1.bib", "ref space2.bib"]);
        assert_eq!(
            g(r#""ref space*.bib""#),
            ["ref space1.bib", "ref space2.bib"]
        );
        assert_eq!(g("'ref space1.bib'"), ["ref space1.bib"]);
        assert_eq!(g("a.bib b.bib"), ["a.bib", "b.bib"]);
        assert_eq!(g("~/*.bib"), [format!("{h}/h1.bib"), format!("{h}/h2.bib")]);
        assert_eq!(g("~/nonexist.bib"), [format!("{h}/nonexist.bib")]);
        assert_eq!(g("~"), [h.to_string()]);
        assert_eq!(g("~/sub/*.bib"), [format!("{h}/sub/s.bib")]);
        assert_eq!(g(r"x\*y.bib"), ["x*y.bib"]);
        assert_eq!(g(r"x\*y*"), ["x*y.bib"]);
        assert_eq!(g("{a,b}.bib"), ["a.bib", "b.bib"]);
        assert!(g("nomatch*.bib").is_empty());
        assert_eq!(g("missing.bib"), ["missing.bib"]);
        assert_eq!(g("[ab].bib"), ["a.bib", "b.bib"]);
        assert_eq!(g("[!a].bib"), ["b.bib"]);
        assert_eq!(g("[^a].bib"), ["a.bib"], "`^` is not a negation");
        assert_eq!(g(r"a.bib\ "), ["a.bib "]);
        assert_eq!(g(r#""x*y.bib""#), ["x*y.bib"]);
        assert_eq!(g("*.bib").len(), 6, "hidden files need a literal dot");
        assert_eq!(g(".*.bib"), [".hidden.bib"]);
        assert_eq!(g("a{b.bib"), ["a"], "unbalanced brace globs the prefix");
        assert_eq!(g("{}"), ["{}"]);
        assert_eq!(g("~nosuchuser/x"), ["~nosuchuser/x"]);
    }
}

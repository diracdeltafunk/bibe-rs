//! Name parsing: `parsename` and `parsename_x` (bibtex.pm:1784-1965) and the
//! helpers they use from `Biber::Utils` (`remove_outer`, `has_outer`,
//! `strip_noinit`, `gen_initials`, `join_name_parts`, `split_xsv`) and
//! `_split_initials` (bibtex.pm:2008).

use crate::config::blx::{Registry, Scope, expand_option_input};
use crate::model::{Name, NamePart};
use crate::perlish::{lc, nfc_string, nfd_string};
use perl_compat::grapheme::{gcstring, perl};
use perl_compat::regex::PerlRegex;
use perl_compat::unicode::{Prop, is_perl_space};
use serde_json::Value as J;
use std::borrow::Cow;
use std::sync::Arc;

/// Settings name parsing needs.
pub struct NameCfg<'a> {
    /// Compiled `noinit` regexes.
    pub noinit: &'a [Arc<PerlRegex>],
    /// `--fixinits`.
    pub fixinits: bool,
    /// The data model's `nameparts` constant.
    pub nameparts: &'a [String],
    /// `xnamesep`.
    pub xnamesep: &'a str,
    /// The biblatex option registry (per-name options).
    pub registry: &'a Registry,
}

/// `remove_outer` (Utils.pm:823): `(stripped?, string)`.
pub fn remove_outer(s: &str) -> (bool, String) {
    // extract_bracketed($str, '{}') must return the whole string.
    let b = s.as_bytes();
    if b.first() != Some(&b'{') {
        return (false, s.to_string());
    }
    let mut depth = 0usize;
    let mut i = 0;
    let mut end = None;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    match end {
        Some(e) if e == b.len() - 1 && b.len() > 2 => (true, s[1..b.len() - 1].to_string()),
        _ => (false, s.to_string()),
    }
}

/// `has_outer` (Utils.pm:839).
pub fn has_outer(s: &str) -> bool {
    // return 0 if $str =~ m/}\s*{/
    let c: Vec<char> = s.chars().collect();
    for i in 0..c.len() {
        if c[i] == '}' {
            let mut j = i + 1;
            while j < c.len() && is_perl_space(c[j]) {
                j += 1;
            }
            if j < c.len() && c[j] == '{' {
                return false;
            }
        }
    }
    s.starts_with('{') && s.ends_with('}') && s.len() > 2
}

/// `strip_noinit` (Utils.pm:538).
pub fn strip_noinit(s: &str, noinit: &[Arc<PerlRegex>]) -> String {
    if !perl_compat::perl_truthy(s) {
        return String::new();
    }
    let mut s = Cow::Borrowed(s);
    for re in noinit {
        let stripped = match re.remove_all_cow(&s) {
            Ok(Cow::Owned(r)) => Some(r),
            _ => None,
        };
        if let Some(r) = stripped {
            s = Cow::Owned(r);
        }
    }
    // s{\\[A-Za-z]+\s*(\{([^\}]*)?\})?}{defined($2)?$2:q{}}eg
    if !s.contains('\\') {
        let out = s.into_owned();
        return if out == "{}" || out == "{}\n" {
            String::new()
        } else {
            out
        };
    }
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < c.len() {
        if c[i] == '\\' && i + 1 < c.len() && c[i + 1].is_ascii_alphabetic() {
            let mut j = i + 1;
            while j < c.len() && c[j].is_ascii_alphabetic() {
                j += 1;
            }
            while j < c.len() && is_perl_space(c[j]) {
                j += 1;
            }
            if j < c.len()
                && c[j] == '{'
                && let Some(k) = c[j + 1..].iter().position(|&x| x == '}')
            {
                out.extend(&c[j + 1..j + 1 + k]);
                i = j + 1 + k + 1;
            } else {
                i = j;
            }
            continue;
        }
        out.push(c[i]);
        i += 1;
    }
    if out == "{}" || out == "{}\n" {
        out.clear();
    }
    out
}

/// Perl `split(/\p{Dash}/, $s)` (trailing empty fields removed).
fn split_dash(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s
        .split(|c| Prop::Dash.contains(c))
        .map(str::to_string)
        .collect();
    while v.last().is_some_and(String::is_empty) {
        v.pop();
    }
    v
}

/// `gen_initials` (Utils.pm:1843).
pub fn gen_initials(strings: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for s in strings {
        let wholly_braced = s.chars().nth(2).is_some()
            && s.starts_with('{')
            && s.ends_with('}')
            && !s.contains('\n');
        // (three consecutive characters: not `{`, a dash, not `}`)
        let hyphenated = s
            .chars()
            .zip(s.chars().skip(1))
            .zip(s.chars().skip(2))
            .any(|((a, b), c)| a != '{' && Prop::Dash.contains(b) && c != '}');
        if !wholly_braced && hyphenated {
            out.push(gen_initials(&split_dash(s)).join("-"));
        } else {
            let t = s.trim_start_matches('{');
            let chr = gcstring::substr(t, 0, Some(1));
            if chr.chars().next().is_some_and(|x| Prop::Dia.contains(x)) {
                out.push(gcstring::substr(t, 0, Some(2)).to_string());
            } else {
                out.push(chr.to_string());
            }
        }
    }
    out
}

/// `join_name_parts` (Utils.pm:1875).
pub fn join_name_parts(parts: &[String]) -> String {
    match parts.len() {
        1 => parts[0].clone(),
        2 => format!("{}~{}", parts[0], parts[1]),
        0 => "~~".into(),
        n => {
            let mut s = parts[0].clone();
            s.push(if gcstring::length(&parts[0]) < 3 {
                '~'
            } else {
                ' '
            });
            s.push_str(&parts[1..n - 1].join(" "));
            s.push('~');
            s.push_str(&parts[n - 1]);
            s
        }
    }
}

/// `_split_initials` (bibtex.pm:2008): `"ab{cd}e"` → `[a, b, cd, e]`.
fn split_initials(npv: &str) -> Vec<Option<String>> {
    let mut out = Vec::new();
    let mut ci = false;
    let mut acc: Option<String> = None;
    for c in perl::clusters(npv) {
        if c == "{" {
            ci = true;
        } else if c == "}" {
            ci = false;
            out.push(acc.take());
            acc = Some(String::new());
        } else if ci {
            acc.get_or_insert_with(String::new).push_str(c);
        } else {
            out.push(Some(c.to_string()));
        }
    }
    out
}

/// `split_xsv` (Utils.pm:1893): `Text::CSV` with `sep_char ','`,
/// `binary`, `allow_whitespace`. On a parse error `fields()` returns
/// `undef`, which biber iterates over as one undefined field: `None`.
pub fn split_xsv(s: &str) -> Vec<Option<String>> {
    split_xsv_ok(s).map_or_else(|| vec![None], |v| v.into_iter().map(Some).collect())
}

/// The fields of a successful `Text::CSV` parse, `None` on a parse error.
fn split_xsv_ok(s: &str) -> Option<Vec<String>> {
    let ws = |c: char| c == ' ' || c == '\t';
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        while i < c.len() && ws(c[i]) {
            i += 1;
        }
        let mut field = String::new();
        if i < c.len() && c[i] == '"' {
            i += 1;
            loop {
                if i >= c.len() {
                    return None;
                }
                if c[i] == '"' {
                    if i + 1 < c.len() && c[i + 1] == '"' {
                        field.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                field.push(c[i]);
                i += 1;
            }
            while i < c.len() && ws(c[i]) {
                i += 1;
            }
            if i < c.len() && c[i] != ',' {
                return None;
            }
        } else {
            while i < c.len() && c[i] != ',' {
                if c[i] == '"' {
                    return None;
                }
                field.push(c[i]);
                i += 1;
            }
            while field.ends_with(ws) {
                field.pop();
            }
        }
        out.push(field);
        if i >= c.len() {
            break;
        }
        i += 1; // the comma
    }
    Some(out)
}

/// `split(/[\h~]+(?![^{]*\})/, $part)`.
fn split_initial_parts(s: &str) -> Vec<String> {
    let c: Vec<char> = s.chars().collect();
    let is_sep = |x: char| x == '~' || Prop::PerlHSpace.contains(x);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    while i < c.len() {
        if is_sep(c[i]) {
            let mut j = i;
            while j < c.len() && is_sep(c[j]) {
                j += 1;
            }
            // (?![^{]*\}): reject if a '}' follows before any '{'
            let mut k = j;
            while k < c.len() && c[k] != '{' && c[k] != '}' {
                k += 1;
            }
            let inside = k < c.len() && c[k] == '}';
            if inside {
                cur.extend(&c[i..j]);
            } else {
                out.push(std::mem::take(&mut cur));
            }
            i = j;
            continue;
        }
        cur.push(c[i]);
        i += 1;
    }
    out.push(cur);
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// Perl `split(/\s+/, $s)` / `split(/[\s~]+/, $s)`.
fn split_on(s: &str, sep: impl Fn(char) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_sep = false;
    let mut any = false;
    for ch in s.chars() {
        if sep(ch) {
            if !in_sep {
                out.push(std::mem::take(&mut cur));
                in_sep = true;
            }
        } else {
            in_sep = false;
            cur.push(ch);
        }
        any = true;
    }
    if any {
        out.push(cur);
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// Result of parsing one name.
pub struct ParsedName {
    pub name: Name,
    /// Name-part lengths to record (`set_np_length`).
    pub lengths: Vec<(String, usize)>,
    /// btparse messages (rendered as libbtparse prints them).
    pub btlines: Vec<String>,
    /// btparse would have crashed on this name.
    pub crash: bool,
    /// biber warnings (no entry attached).
    pub warnings: Vec<String>,
}

fn rawstring(name: &Name) -> String {
    ["family", "given", "prefix", "suffix"]
        .iter()
        .filter_map(|p| name.nameparts.get(*p).and_then(|x| x.string.clone()))
        .chain(
            name.nameparts
                .iter()
                .filter(|(k, _)| !["family", "given", "prefix", "suffix"].contains(&k.as_str()))
                .filter_map(|(_, v)| v.string.clone()),
        )
        .collect()
}

/// `parsename` (bibtex.pm:1784).
pub fn parsename(cfg: &NameCfg, namestr: &str) -> ParsedName {
    // s/\A\s*|\s*\z//xms (no /g: only the leading whitespace goes)
    let mut s = namestr.trim_start_matches(is_perl_space).to_string();
    // s/\s+|\\\s/ /g
    {
        let mut o = String::with_capacity(s.len());
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            if is_perl_space(c) {
                while it.next_if(|&n| is_perl_space(n)).is_some() {}
                o.push(' ');
            } else if c == '\\' && it.next_if(|&n| is_perl_space(n)).is_some() {
                o.push(' ');
            } else {
                o.push(c);
            }
        }
        s = o;
    }
    // s/\A\{\{+([^\{\}]+)\}+\}\z/{$1}/
    {
        let a = s.len() - s.trim_start_matches('{').len();
        let b = s.len() - s.trim_end_matches('}').len();
        if a >= 2 && b >= 2 && a + b < s.len() {
            let inner = &s[a..s.len() - b];
            if !inner.contains(['{', '}']) {
                s = format!("{{{inner}}}");
            }
        }
    }
    if cfg.fixinits {
        // s/(\w)\.(\w)/$1. $2/g
        let c: Vec<char> = s.chars().collect();
        let w = perl_compat::unicode::is_perl_word;
        let mut o = String::new();
        let mut i = 0;
        while i < c.len() {
            if i + 2 < c.len() && w(c[i]) && c[i + 1] == '.' && w(c[i + 2]) {
                o.push(c[i]);
                o.push_str(". ");
                o.push(c[i + 2]);
                i += 3;
            } else {
                o.push(c[i]);
                i += 1;
            }
        }
        s = o;
    }
    let nfcs = nfc_string(s);
    let sp = btparse::split_name(&nfcs);
    let btlines: Vec<String> = sp.diags.iter().map(|d| d.render("")).collect();
    let crash = sp.anomaly == Some(btparse::NameAnomaly::Crash);
    use btparse::NamePart as P;
    let parts = [
        ("prefix", nfd_string(sp.parts.format(P::Von))),
        ("family", nfd_string(sp.parts.format(P::Last))),
        ("given", nfd_string(sp.parts.format(P::First))),
        ("suffix", nfd_string(sp.parts.format(P::Jr))),
    ];
    let mut name = Name::default();
    let mut lengths = Vec::new();
    for (np, v) in parts {
        let mut part = NamePart::default();
        let mut strip = None;
        if perl_compat::perl_truthy(&v) {
            let (flag, stripped) = remove_outer(&v);
            let mut p = v.clone();
            if flag {
                p = replace_ws_runs(&v, "_");
            }
            let p = strip_noinit(&p, cfg.noinit);
            let inits = gen_initials(&split_initial_parts(&p));
            part.string = Some(stripped);
            part.initial = Some(inits);
            strip = Some(if flag { "1" } else { "0" }.to_string());
        }
        if let Some(st) = part
            .string
            .as_deref()
            .filter(|x| perl_compat::perl_truthy(x))
        {
            lengths.push((np.to_string(), st.chars().count()));
        }
        if let Some(i) = &part.initial {
            lengths.push((format!("{np}-i"), i.concat().chars().count()));
        }
        name.nameparts.insert(np.to_string(), part);
        name.strip.insert(np.to_string(), strip);
    }
    // Name->new keeps only data-model name parts
    name.nameparts
        .retain(|k, _| cfg.nameparts.iter().any(|n| n == k));
    name.rawstring = Some(rawstring(&name));
    ParsedName {
        name,
        lengths,
        btlines,
        crash,
        warnings: Vec::new(),
    }
}

/// `s/\s+/$r/g`.
fn replace_ws_runs(s: &str, r: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.chars() {
        if is_perl_space(c) {
            if !in_ws {
                o.push_str(r);
                in_ws = true;
            }
        } else {
            in_ws = false;
            o.push(c);
        }
    }
    o
}

/// `m/^(.+)\s*$xnamesep\s*(.+)$/x` on one extended-name part.
fn xname_kv<'s>(s: &'s str, sep: &str) -> Option<(&'s str, String)> {
    if sep.is_empty() {
        return None;
    }
    let s = s.strip_suffix('\n').unwrap_or(s);
    if s.contains('\n') {
        return None;
    }
    let mut search_end = s.len();
    while let Some(p) = s[..search_end].rfind(sep) {
        let rest = &s[p + sep.len()..];
        if p > 0 && !rest.is_empty() {
            let trimmed = rest.trim_start_matches(is_perl_space);
            let v = if trimmed.is_empty() {
                // \s* gives back one character to (.+)
                let last = rest.chars().last().unwrap();
                last.to_string()
            } else {
                trimmed.to_string()
            };
            return Some((&s[..p], v));
        }
        if p == 0 {
            break;
        }
        search_end = p;
    }
    None
}

/// `parsename_x` (bibtex.pm:1879).
pub fn parsename_x(cfg: &NameCfg, namestr: &str, fieldname: &str, key: &str) -> ParsedName {
    use std::collections::BTreeMap;
    let mut namec: BTreeMap<String, String> = BTreeMap::new();
    let mut inits: BTreeMap<String, Vec<Option<String>>> = BTreeMap::new();
    let mut hashid: Option<String> = None;
    let mut pernameopts: Vec<(String, Option<String>)> = Vec::new();
    let mut warnings = Vec::new();
    for np in split_xsv(namestr) {
        let (npn, npv) = match np.as_deref().and_then(|np| xname_kv(np, cfg.xnamesep)) {
            Some((n, v)) => (lc(n), Some(v)),
            None => (String::new(), None),
        };
        if cfg.registry.has_scope(&npn, Scope::Name) {
            let input = cfg
                .registry
                .decl(&npn, Scope::Name)
                .and_then(|d| d.input.as_ref());
            for (o, v) in expand_option_input(cfg.registry, &npn, npv.as_deref(), input) {
                pernameopts.push((o, v));
            }
            continue;
        }
        let base = npn.strip_suffix("-i").unwrap_or(&npn);
        if !(cfg.nameparts.iter().any(|n| n == base) || npn == "id") {
            warnings.push(format!(
                "Invalid namepart option '{npn}' found in extended name format name '{fieldname}' in entry '{key}', ignoring"
            ));
            continue;
        }
        let npv = npv.unwrap_or_default();
        if npn.ends_with("-i") {
            inits.insert(npn.clone(), split_initials(&npv));
        } else if npn == "id" {
            hashid = Some(npv);
        } else if has_outer(&npv) {
            namec.insert(npn, npv);
        } else {
            namec.insert(npn, join_name_parts(&split_on(&npv, is_perl_space)));
        }
    }
    let mut name = Name::default();
    let mut lengths = Vec::new();
    for np in cfg.nameparts {
        let mut part = NamePart::default();
        if let Some(v) = namec.get(np) {
            let (s, stripped) = remove_outer(v);
            let p = if s {
                replace_ws_runs(&stripped, "_")
            } else {
                stripped.clone()
            };
            let p = strip_noinit(&p, cfg.noinit);
            let ini: Vec<Option<String>> = match inits.get(&format!("{np}-i")) {
                Some(i) => i.clone(),
                None => gen_initials(&split_on(&p, |c| c == '~' || is_perl_space(c)))
                    .into_iter()
                    .map(Some)
                    .collect(),
            };
            part.string = Some(stripped);
            // undef initials (a stray `}`) are kept as empty strings
            part.initial = Some(ini.into_iter().map(Option::unwrap_or_default).collect());
        }
        if let Some(st) = part
            .string
            .as_deref()
            .filter(|x| perl_compat::perl_truthy(x))
        {
            lengths.push((np.clone(), st.chars().count()));
        }
        if let Some(i) = &part.initial {
            lengths.push((format!("{np}-i"), i.concat().chars().count()));
        }
        name.nameparts.insert(np.clone(), part);
    }
    if let Some(h) = hashid.filter(|h| perl_compat::perl_truthy(h)) {
        name.slots.insert("hashid", J::from(h));
    }
    let name_opts = cfg.registry.options_in_scope(Scope::Name);
    for (o, v) in pernameopts {
        if name_opts.contains(&o) || o == "gender" || o == "useprefix" {
            name.slots.insert(o, v.map_or(J::Null, J::from));
        }
    }
    name.rawstring = Some(rawstring(&name));
    ParsedName {
        name,
        lengths,
        btlines: Vec::new(),
        crash: false,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(remove_outer("{a b}"), (true, "a b".into()));
        assert_eq!(remove_outer("{a}{b}"), (false, "{a}{b}".into()));
        assert_eq!(remove_outer("{}"), (false, "{}".into()));
        assert!(has_outer("{Barnes Noble}"));
        assert!(!has_outer("{a} {b}"));
        assert_eq!(
            gen_initials(&["{K}ent-{B}oswell".into(), "Hans{-}Peter".into()]),
            vec!["K-B", "H"]
        );
        assert_eq!(
            join_name_parts(&["Jo".into(), "B".into(), "C".into()]),
            "Jo~B~C"
        );
        assert_eq!(
            join_name_parts(&["John".into(), "B".into(), "C".into()]),
            "John B~C"
        );
        assert_eq!(
            split_xsv(r#"family=Doe, "given=J, Jr""#),
            vec![Some("family=Doe".into()), Some("given=J, Jr".into())]
        );
        // A parse error: Text::CSV's fields() returns one undef.
        assert_eq!(split_xsv(r#"given={{"}}"#), vec![None]);
        assert_eq!(split_initial_parts("A~{B C} D"), vec!["A", "{B C}", "D"]);
        assert_eq!(
            xname_kv("family = Doe", "="),
            Some(("family ", "Doe".to_string()))
        );
        assert_eq!(strip_noinit("\\foo {bar}x", &[]), "barx");
    }
}

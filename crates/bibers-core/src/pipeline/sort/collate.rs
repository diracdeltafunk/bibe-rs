//! `Biber::sort_list` (Biber.pm:4249): collation keys and the stable
//! multi-key sort.
//!
//! biber builds, per sort column, a Perl *code string* such as
//! `$Collator->change(level => 4 ,upper_before_lower => 1)->getSortKey(q{…})`
//! and `eval`s it for every uncached (column, value) pair. We reproduce the
//! observable effects of that without evaluating anything:
//! * the main collator is one stateful object: `change()` only runs when a
//!   key is computed (not on cache hits), and a column with a per-set
//!   `sortcase` *or* `sortupper` override changes only that option, so the
//!   other one keeps whatever an earlier column left (state machine below);
//! * columns with a per-set `locale` get a fresh collator per evaluation
//!   (`Biber::UCollate->new(…)`), which is why `--sortlocale` cannot override
//!   them (BB-043) and why a quote in a `--collate-options` value breaks the
//!   eval and leaves such columns unsorted (BB-045);
//! * `sortcase`/`sortupper` override `-c level`/`-c upper_before_lower`
//!   (BB-101);
//! * the value is spliced into `q{…}`: unbalanced braces or a trailing
//!   backslash make the eval fail and give an `undef` key, which sorts
//!   first; `\\`, `\{`, `\}` are unescaped (BB-017, `sortkey-eval`).
//!
//! Each distinct collator configuration is built once and each (column
//! code, value) key is computed once, as biber's cache does.

use super::{Ctx, list_sorting_template};
use crate::biber_error;
use crate::info;
use crate::log::Fatal;
use crate::model::SectionState;
use crate::perlish::{looks_like_number, numify, truthy};
use crate::quirks::Quirk;
use serde_json::Value as J;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use ucollate::{CollateOptions, Collator};

/// A sort key cell.
#[derive(Clone, Debug)]
enum Cell {
    Int(i64),
    /// A collation key; `None` is an undef key (compares as `""`).
    Str(Option<Rc<[u8]>>),
}

/// How a str column obtains its collator.
#[derive(Clone, Debug)]
enum CObj {
    /// `$Collator`, the list's main (stateful) collator.
    Main,
    /// `Biber::UCollate->new('<locale>', %collopts)`.
    New(String),
    /// A code string that does not compile (BB-045).
    Broken,
}

/// The `->change(...)` part of a column's code string.
#[derive(Clone, Debug)]
struct Change {
    level: Option<String>,
    ubl: Option<String>,
}

#[derive(Clone, Debug)]
struct StrCol {
    cobj: CObj,
    change: Change,
    /// The code string without the value (the cache key prefix).
    code: String,
}

/// Perl's `q{…}` parsing of `value` followed by biber's closing brace:
/// `None` if the code would not compile (BB-017).
fn perl_q_brace(value: &str) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    let mut depth = 1usize;
    let mut it = value.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.peek() {
                Some(&n @ ('\\' | '{' | '}')) => {
                    out.push(n);
                    it.next();
                }
                // A trailing backslash escapes the closing brace.
                None => return None,
                Some(_) => out.push('\\'),
            },
            '{' => {
                depth += 1;
                out.push(c);
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    // The rest of the value would be Perl code.
                    return None;
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    (depth == 1).then_some(out)
}

/// The value as a Perl `int` sort key: `looks_like_number ? SvIV : 2e9`.
fn int_key(v: &str) -> i64 {
    if !looks_like_number(v) {
        return 2_000_000_000;
    }
    let f = numify(v);
    if f.is_nan() {
        0
    } else if f >= i64::MAX as f64 {
        i64::MAX
    } else if f <= i64::MIN as f64 {
        i64::MIN
    } else {
        f.trunc() as i64
    }
}

/// Collator construction shared by the main and per-locale collators.
struct Collators {
    /// `collate_options` (after biber's `level`/`upper_before_lower`
    /// mutation), without `locale`/`table`, `alternate` renamed.
    opts: BTreeMap<String, String>,
    /// `-c locale=…`, which beats every other locale setting.
    c_locale: Option<String>,
    base: CollateOptions,
    built: HashMap<(Option<String>, u8, bool), Rc<Collator>>,
}

impl Collators {
    /// The locale `Biber::UCollate->new($thislocale, …)` uses.
    fn locale_for(&self, thislocale: Option<&str>) -> Option<String> {
        match (&self.c_locale, thislocale) {
            (Some(c), _) if truthy(Some(c)) => Some(c.clone()),
            (_, Some(t)) if truthy(Some(t)) => Some(t.to_string()),
            (c, _) => c.clone(),
        }
    }

    fn get(&mut self, locale: Option<&str>, level: u8, ubl: bool) -> Result<Rc<Collator>, String> {
        let k = (locale.map(str::to_string), level, ubl);
        if let Some(c) = self.built.get(&k) {
            return Ok(Rc::clone(c));
        }
        let mut o = self.base.clone();
        o.level = Some(level);
        o.upper_before_lower = Some(ubl);
        let c = Rc::new(Collator::new(locale, &o).map_err(|e| e.to_string())?);
        self.built.insert(k, Rc::clone(&c));
        Ok(c)
    }

    /// biber's INFO lines when `Biber::UCollate->new` overrides options the
    /// locale tailoring sets (UCollate.pm:61-70), in sorted key order.
    fn override_infos(&self, locale: Option<&str>) -> Vec<String> {
        let Ok(fresh) = Collator::new(locale, &CollateOptions::default()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (k, v) in &self.opts {
            if let Some(old) = fresh.option_value(k)
                && &old != v
            {
                out.push(format!(
                    "Overriding locale '{}' defaults '{k} = {old}' with '{k} = {v}'",
                    locale.unwrap_or("")
                ));
            }
        }
        out
    }
}

/// `Biber::sort_list` (Biber.pm:4249), for the data list `st.datalists[list]`.
pub fn sort_list(ctx: &mut Ctx, st: &mut SectionState, list: usize) -> Result<(), Fatal> {
    let dl = &st.datalists[list];
    let keys: Vec<String> = dl.keys.clone().unwrap_or_default();
    let lstn = dl.sortingtemplatename.clone();
    let ltype = dl.type_.clone();
    let lname = dl.name.clone();
    let tmpl = list_sorting_template(ctx.cfg, dl);
    let quirks = ctx.cfg.quirks;

    // Locale: biber --sortlocale > template locale > global sortlocale.
    let tlocale = tmpl
        .and_then(|t| t.locale.as_deref())
        .filter(|l| truthy(Some(l)))
        .map(str::to_string)
        .or_else(|| {
            ctx.cfg
                .blx
                .global("sortlocale")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });
    let llocale = crate::config::locale::locale2bcp47(tlocale.as_deref()).map(str::to_string);
    let thislocale = ctx
        .cfg
        .biber
        .get_str("sortlocale")
        .filter(|l| truthy(Some(l)))
        .map(str::to_string)
        .or(llocale);

    if ctx.cfg.biber.is_true("fastsort") {
        ctx.log
            .biber_warn("fastsort option no longer required/supported, defaulting to UCA");
    }

    // `collate_options`, with biber's (global, BB-101) mutation.
    let sortcase = ctx.cfg.biber.get_str("sortcase").unwrap_or("").to_string();
    let sortupper = ctx.cfg.biber.get_str("sortupper").unwrap_or("").to_string();
    let mut collopts = ctx.cfg.biber.collate_options().cloned().unwrap_or_default();
    if !truthy(Some(&sortcase)) {
        collopts.insert("level".into(), "2".into());
    }
    collopts.insert("upper_before_lower".into(), sortupper.clone());

    // Options as `Biber::UCollate->new` applies them.
    let c_locale = collopts.get("locale").cloned();
    let mut opts = collopts.clone();
    opts.remove("locale");
    if truthy(thislocale.as_deref())
        && !truthy(c_locale.as_deref())
        && let Some(t) = opts.remove("table")
    {
        info!(
            ctx.log,
            "Ignoring collation table '{t}' as locale is set ({})",
            thislocale.as_deref().unwrap_or("")
        );
    }
    if let Some(alt) = opts.remove("alternate")
        && truthy(Some(&alt))
    {
        opts.insert("variable".into(), alt);
    }
    let mut base = CollateOptions::default();
    for (k, v) in &opts {
        let r = match k.as_str() {
            // Constructor-only options are applied at construction (DC-008).
            "entry" => {
                base.entry = Some(v.clone());
                Ok(())
            }
            _ => base.set_perl(k, Some(v)),
        };
        if let Err(e) = r {
            return biber_error!(ctx.log, "Invalid collation option '{k}': {e}");
        }
    }
    let level0 = opts
        .get("level")
        .map(String::as_str)
        .unwrap_or("4")
        .to_string();
    let ubl0 = sortupper.clone();
    let mut colls = Collators {
        opts,
        c_locale,
        base,
        built: HashMap::new(),
    };
    let main_locale = colls.locale_for(thislocale.as_deref());

    let perl_level = |s: &str| -> u8 { (numify(s).trunc() as i64).clamp(1, 4) as u8 };
    let mut main_state = (perl_level(&level0), truthy(Some(&ubl0)));
    let main = match colls.get(main_locale.as_deref(), main_state.0, main_state.1) {
        Ok(c) => c,
        Err(e) => return biber_error!(ctx.log, "{e}"),
    };
    for i in colls.override_infos(main_locale.as_deref()) {
        info!(ctx.log, "{i}");
    }
    info!(
        ctx.log,
        "Sorting list '{lname}' of type '{ltype}' with template '{lstn}' and locale '{}'",
        thislocale.as_deref().unwrap_or("")
    );
    if main.locale() == "default" {
        info!(
            ctx.log,
            "No sort tailoring available for locale '{}'",
            thislocale.as_deref().unwrap_or("")
        );
    }

    // The list's sort data schema.
    let schema: Vec<(bool, bool)> = match dl.slots.get("sortdataschema") {
        Some(J::Array(a)) => a
            .iter()
            .map(|c| {
                let int = c.get("int").is_some();
                let desc = c
                    .get("spec")
                    .and_then(J::as_str)
                    .is_some_and(|s| s.starts_with('-'));
                (int, desc)
            })
            .collect(),
        _ => Vec::new(),
    };

    // Per-column code strings (Biber.pm:4322-4366).
    let gsortcase = numify(&sortcase);
    let gsortupper = numify(&sortupper);
    let breaks_quote = |s: &str| s.contains('\'') || s.contains('\\');
    let new_args_broken = collopts
        .iter()
        .any(|(k, v)| breaks_quote(k) || breaks_quote(v));
    let mut cols: Vec<StrCol> = Vec::new();
    if let Some(t) = tmpl {
        for pass in &t.spec {
            let sl = crate::config::locale::locale2bcp47(pass.opts.locale.as_deref());
            let cobj = match sl {
                Some(sl) if Some(sl) != thislocale.as_deref() => {
                    if breaks_quote(sl) || new_args_broken {
                        CObj::Broken
                    } else {
                        CObj::New(sl.to_string())
                    }
                }
                _ => CObj::Main,
            };
            let mut change = Change {
                level: None,
                ubl: None,
            };
            if let Some(sc) = pass.opts.sortcase.as_deref()
                && numify(sc) != gsortcase
            {
                change.level = Some(if truthy(Some(sc)) { "4" } else { "2" }.into());
            }
            if let Some(su) = pass.opts.sortupper.as_deref()
                && numify(su) != gsortupper
            {
                change.ubl = Some(if truthy(Some(su)) { "1" } else { "0" }.into());
            }
            let fc = if change.level.is_none() && change.ubl.is_none() {
                change.level = Some(level0.clone());
                change.ubl = Some(ubl0.clone());
                format!("->change(level => {level0} ,upper_before_lower => {ubl0})")
            } else {
                let mut parts = Vec::new();
                if let Some(l) = &change.level {
                    parts.push(format!("level => {l}"));
                }
                if let Some(u) = &change.ubl {
                    parts.push(format!("upper_before_lower => {u}"));
                }
                format!("->change({})", parts.join(","))
            };
            let prefix = match &cobj {
                CObj::Main => "$Collator".to_string(),
                CObj::New(l) => format!("new({l})"),
                CObj::Broken => "broken".to_string(),
            };
            cols.push(StrCol {
                cobj,
                change,
                code: prefix + &fc,
            });
        }
    }

    // Key extraction, in biber's order (keys in list order, columns in
    // order), with the per-call cache (`$cache->{$code} ||= eval $code`).
    let sortdata = dl.sortdata.as_ref();
    let mut cache: HashMap<(usize, String), Rc<[u8]>> = HashMap::new();
    let mut code_ids: HashMap<String, usize> = HashMap::new();
    let col_code: Vec<usize> = cols
        .iter()
        .map(|c| {
            let n = code_ids.len();
            *code_ids.entry(c.code.clone()).or_insert(n)
        })
        .collect();
    let mut infos: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<Cell>> = Vec::with_capacity(keys.len());
    for k in &keys {
        let obj: Vec<&str> = sortdata
            .and_then(|m| m.get(k))
            .map(|d| d.obj.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();
        let mut row = Vec::with_capacity(obj.len());
        for (i, v) in obj.iter().enumerate() {
            if schema.get(i).is_some_and(|c| c.0) {
                row.push(Cell::Int(int_key(v)));
                continue;
            }
            let Some(col) = cols.get(i) else {
                row.push(Cell::Str(None));
                continue;
            };
            let ck = (col_code[i], v.to_string());
            if let Some(hit) = cache.get(&ck) {
                row.push(Cell::Str(Some(Rc::clone(hit))));
                continue;
            }
            let value = if quirks.on(Quirk::SortkeyEval) {
                perl_q_brace(v)
            } else {
                Some(v.to_string())
            };
            let coll = match (&col.cobj, value.as_ref()) {
                (CObj::Broken, _) | (_, None) => None,
                (CObj::Main, Some(_)) => {
                    if let Some(l) = &col.change.level {
                        main_state.0 = perl_level(l);
                    }
                    if let Some(u) = &col.change.ubl {
                        main_state.1 = truthy(Some(u));
                    }
                    Some(colls.get(main_locale.as_deref(), main_state.0, main_state.1))
                }
                (CObj::New(sl), Some(_)) => {
                    let loc = colls.locale_for(Some(sl));
                    infos.extend(colls.override_infos(loc.as_deref()));
                    let level = col
                        .change
                        .level
                        .as_deref()
                        .map_or(perl_level(&level0), perl_level);
                    let ubl = truthy(Some(col.change.ubl.as_deref().unwrap_or(&ubl0)));
                    Some(colls.get(loc.as_deref(), level, ubl))
                }
            };
            let key: Option<Rc<[u8]>> = match coll {
                None => None,
                Some(Err(e)) => return biber_error!(ctx.log, "{e}"),
                Some(Ok(c)) => {
                    let kb: Rc<[u8]> = c.sort_key(value.as_deref().unwrap_or("")).into();
                    if !kb.is_empty() {
                        cache.insert(ck, Rc::clone(&kb));
                    }
                    Some(kb)
                }
            };
            row.push(Cell::Str(key));
        }
        rows.push(row);
    }
    for i in infos {
        info!(ctx.log, "{i}");
    }

    // Sort::Key multikeysorter: stable, lexicographic over the columns.
    let desc: Vec<bool> = (0..rows.iter().map(Vec::len).max().unwrap_or(0))
        .map(|i| schema.get(i).is_some_and(|c| c.1))
        .collect();
    let mut idx: Vec<usize> = (0..keys.len()).collect();
    idx.sort_by(|&a, &b| {
        for (i, (x, y)) in rows[a].iter().zip(rows[b].iter()).enumerate() {
            let o = match (x, y) {
                (Cell::Int(p), Cell::Int(q)) => p.cmp(q),
                (Cell::Str(p), Cell::Str(q)) => {
                    let p: &[u8] = p.as_deref().unwrap_or(&[]);
                    let q: &[u8] = q.as_deref().unwrap_or(&[]);
                    p.cmp(q)
                }
                _ => Ordering::Equal,
            };
            let o = if desc[i] { o.reverse() } else { o };
            if o != Ordering::Equal {
                return o;
            }
        }
        rows[a].len().cmp(&rows[b].len())
    });
    let sorted: Vec<String> = idx.into_iter().map(|i| keys[i].clone()).collect();
    st.datalists[list].keys = Some(sorted);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_brace() {
        assert_eq!(perl_q_brace("abc").as_deref(), Some("abc"));
        assert_eq!(perl_q_brace("a{b}c").as_deref(), Some("a{b}c"));
        assert_eq!(perl_q_brace("a\\\\b").as_deref(), Some("a\\b"));
        assert_eq!(perl_q_brace("a\\{b").as_deref(), Some("a{b"));
        assert_eq!(perl_q_brace("a\\xb").as_deref(), Some("a\\xb"));
        assert_eq!(perl_q_brace("The Foo\\"), None);
        assert_eq!(perl_q_brace("a}b"), None);
        assert_eq!(perl_q_brace("a{b"), None);
    }

    #[test]
    fn int_keys() {
        assert_eq!(int_key("1995"), 1995);
        assert_eq!(int_key("-50"), -50);
        assert_eq!(int_key("1.5"), 1);
        assert_eq!(int_key(""), 2_000_000_000);
        assert_eq!(int_key("mm"), 2_000_000_000);
    }
}

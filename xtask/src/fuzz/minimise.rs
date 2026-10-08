//! Delta debugging with the oracle in the loop: shrink a failing input while
//! the discrepancy (a chosen diff shape) persists.

use super::bib::{Entry, FuzzInput, Item, Val};
use super::exec::{Template, check};
use bibers_testkit::Runner;
use std::collections::HashMap;
use std::sync::Mutex;

pub struct Minimiser<'a> {
    pub tpl: &'a Template,
    pub oracle: &'a Runner,
    pub bibers: &'a Runner,
    /// The shape that must stay in the signature.
    pub shape: String,
    pub jobs: usize,
    cache: Mutex<HashMap<FuzzInput, bool>>,
    pub tests: Mutex<usize>,
}

impl<'a> Minimiser<'a> {
    pub fn new(
        tpl: &'a Template,
        oracle: &'a Runner,
        bibers: &'a Runner,
        shape: String,
        jobs: usize,
    ) -> Self {
        Minimiser {
            tpl,
            oracle,
            bibers,
            shape,
            jobs,
            cache: Mutex::new(HashMap::new()),
            tests: Mutex::new(0),
        }
    }

    /// Whether `input` still shows the discrepancy.
    pub fn interesting(&self, input: &FuzzInput) -> bool {
        if let Some(&r) = self.cache.lock().unwrap().get(input) {
            return r;
        }
        if input.render_bib().contains('\u{0}') {
            return false;
        }
        *self.tests.lock().unwrap() += 1;
        let r = match check(input, self.tpl, self.oracle, self.bibers, false) {
            Ok(v) => v.shapes.contains(&self.shape),
            Err(_) => false,
        };
        self.cache.lock().unwrap().insert(input.clone(), r);
        r
    }

    /// The first interesting candidate (in order), testing up to `jobs` at a time.
    fn first_interesting(&self, cands: Vec<FuzzInput>) -> Option<FuzzInput> {
        for batch in cands.chunks(self.jobs.max(1)) {
            let results: Vec<bool> = std::thread::scope(|s| {
                let hs: Vec<_> = batch
                    .iter()
                    .map(|c| s.spawn(move || self.interesting(c)))
                    .collect();
                hs.into_iter().map(|h| h.join().unwrap_or(false)).collect()
            });
            if let Some(i) = results.iter().position(|&r| r) {
                return Some(batch[i].clone());
            }
        }
        None
    }

    /// ddmin over a list of `T`, where `build` turns a sublist into an input.
    fn ddmin<T: Clone>(&self, items: Vec<T>, build: &dyn Fn(&[T]) -> FuzzInput) -> Vec<T> {
        let mut items = items;
        let mut n = 2usize;
        while !items.is_empty() {
            if items.len() == 1 {
                if self.interesting(&build(&[])) {
                    items.clear();
                }
                break;
            }
            let n_eff = n.min(items.len());
            let len = items.len();
            let mut cands = Vec::new();
            let mut subsets = Vec::new();
            for k in 0..n_eff {
                let s = len * k / n_eff;
                let e = len * (k + 1) / n_eff;
                let mut sub: Vec<T> = items[..s].to_vec();
                sub.extend_from_slice(&items[e..]);
                cands.push(build(&sub));
                subsets.push(sub);
            }
            if let Some(c) = self.first_interesting(cands.clone()) {
                let i = cands.iter().position(|x| *x == c).expect("candidate");
                items = subsets.swap_remove(i);
                n = (n_eff - 1).max(2);
                continue;
            }
            if n_eff >= items.len() {
                break;
            }
            n = (n_eff * 2).min(items.len());
        }
        items
    }

    pub fn run(&self, input: FuzzInput, log: &dyn Fn(&str)) -> FuzzInput {
        let mut cur = input;
        for round in 1..=4 {
            let before = cur.clone();
            // 1. Top-level items.
            let base = cur.clone();
            let items = self.ddmin(cur.items.clone(), &|xs| FuzzInput {
                items: xs.to_vec(),
                ..base.clone()
            });
            cur.items = items;
            log(&format!(
                "round {round}: {} items ({} tests)",
                cur.items.len(),
                self.tests.lock().unwrap()
            ));
            // 2. Citations.
            let base = cur.clone();
            let cites = self.ddmin(cur.cites.clone(), &|xs| FuzzInput {
                cites: xs.to_vec(),
                ..base.clone()
            });
            cur.cites = cites;
            let base = cur.clone();
            let opts = self.ddmin(cur.opts.clone(), &|xs| FuzzInput {
                opts: xs.to_vec(),
                ..base.clone()
            });
            cur.opts = opts;
            if cur.nocite_all {
                let c = FuzzInput {
                    nocite_all: false,
                    ..cur.clone()
                };
                if self.interesting(&c) {
                    cur = c;
                }
            }
            // 3. Fields of each entry.
            for i in 0..cur.items.len() {
                let Item::Entry(e) = &cur.items[i] else {
                    continue;
                };
                let base = cur.clone();
                let fields = self.ddmin(e.fields.clone(), &|xs| {
                    let mut c = base.clone();
                    if let Item::Entry(e) = &mut c.items[i] {
                        e.fields = xs.to_vec();
                    }
                    c
                });
                if let Item::Entry(e) = &mut cur.items[i] {
                    e.fields = fields;
                }
            }
            log(&format!(
                "round {round}: fields done ({} tests)",
                self.tests.lock().unwrap()
            ));
            // 4. Values.
            for i in 0..cur.items.len() {
                let nf = match &cur.items[i] {
                    Item::Entry(e) => e.fields.len(),
                    _ => 0,
                };
                for f in 0..nf {
                    cur = self.simplify_value(cur, i, f);
                }
                cur = self.simplify_entry(cur, i);
            }
            log(&format!(
                "round {round}: values done ({} tests)",
                self.tests.lock().unwrap()
            ));
            if cur == before {
                break;
            }
        }
        cur
    }

    /// Try a lowercase, parenthesis-free entry type and unparenthesised form.
    fn simplify_entry(&self, cur: FuzzInput, i: usize) -> FuzzInput {
        let Item::Entry(e) = &cur.items[i] else {
            return cur;
        };
        let mut cands = Vec::new();
        if e.parens {
            cands.push(with_entry(&cur, i, |e| e.parens = false));
        }
        if e.ty != e.ty.to_lowercase() {
            let ty = e.ty.to_lowercase();
            cands.push(with_entry(&cur, i, |e| e.ty = ty.clone()));
        }
        match self.first_interesting(cands) {
            Some(c) => c,
            None => cur,
        }
    }

    fn simplify_value(&self, mut cur: FuzzInput, i: usize, f: usize) -> FuzzInput {
        let get = |c: &FuzzInput| -> Option<Val> {
            match &c.items[i] {
                Item::Entry(e) => e.fields.get(f).map(|x| x.value.clone()),
                _ => None,
            }
        };
        let set = |c: &FuzzInput, v: Val| -> FuzzInput {
            let mut c = c.clone();
            if let Item::Entry(e) = &mut c.items[i] {
                e.fields[f].value = v;
            }
            c
        };
        let Some(v) = get(&cur) else {
            return cur;
        };
        // Concatenations: try each part alone, and braced plain text.
        if let Val::Concat(parts) = &v {
            let cands: Vec<FuzzInput> = parts.iter().map(|p| set(&cur, p.clone())).collect();
            if let Some(c) = self.first_interesting(cands) {
                cur = c;
            }
        }
        let Some(v) = get(&cur) else {
            return cur;
        };
        let (text, wrap): (String, fn(String) -> Val) = match &v {
            Val::Braced(s) => (s.clone(), Val::Braced),
            Val::Quoted(s) => {
                let c = set(&cur, Val::Braced(s.clone()));
                if self.interesting(&c) {
                    cur = c;
                    (s.clone(), Val::Braced)
                } else {
                    (s.clone(), Val::Quoted)
                }
            }
            _ => return cur,
        };
        // Token-level reductions: " and "-separated, comma-separated, words, chars.
        let mut text = text;
        for sep in [" and ", ",", " "] {
            if !text.contains(sep) {
                continue;
            }
            let toks: Vec<String> = text.split(sep).map(str::to_string).collect();
            let base = cur.clone();
            let kept = self.ddmin(toks, &|xs| set(&base, wrap(xs.join(sep))));
            let new = kept.join(sep);
            if new != text {
                text = new;
                cur = set(&cur, wrap(text.clone()));
            }
        }
        if text.chars().count() <= 60 {
            let chars: Vec<char> = text.chars().collect();
            let base = cur.clone();
            let kept = self.ddmin(chars, &|xs| {
                let s: String = xs.iter().collect();
                if balanced(&s) {
                    set(&base, wrap(s))
                } else {
                    // Never interesting: an unbalanced value is a syntax error.
                    set(&base, Val::Bare("\u{0}unbalanced".into()))
                }
            });
            let s: String = kept.iter().collect();
            if s != text && balanced(&s) {
                cur = set(&cur, wrap(s));
            }
        }
        cur
    }
}

fn with_entry(cur: &FuzzInput, i: usize, f: impl Fn(&mut Entry)) -> FuzzInput {
    let mut c = cur.clone();
    if let Item::Entry(e) = &mut c.items[i] {
        f(e);
    }
    c
}

fn balanced(s: &str) -> bool {
    let mut d = 0i32;
    for c in s.chars() {
        match c {
            '{' => d += 1,
            '}' => {
                d -= 1;
                if d < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    d == 0
}

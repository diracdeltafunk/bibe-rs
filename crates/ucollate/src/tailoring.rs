//! Tailorings: collation elements added via `entry` (allkeys syntax), and the
//! `Unicode::Collate::Locale` tailoring files compiled into `data/locales.txt`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use rustc_hash::FxHashMap;

use crate::Error;
use crate::cjk::CjkTable;
use crate::options::Variable;
use crate::tables::Ce;

/// Collation elements defined on top of the DUCET (a locale's and/or the user's `entry`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Tailoring {
    singles: FxHashMap<u32, (u32, u32)>,
    contractions: FxHashMap<Box<[u32]>, (u32, u32)>,
    maxlen: FxHashMap<u32, usize>,
    ces: Vec<Ce>,
}

impl Tailoring {
    #[inline]
    fn slice(&self, r: (u32, u32)) -> &[Ce] {
        &self.ces[r.0 as usize..(r.0 + r.1) as usize]
    }

    #[inline]
    pub(crate) fn single(&self, cp: u32) -> Option<&[Ce]> {
        self.singles.get(&cp).map(|&r| self.slice(r))
    }

    #[inline]
    pub(crate) fn contraction(&self, cps: &[u32]) -> Option<&[Ce]> {
        self.contractions.get(cps).map(|&r| self.slice(r))
    }

    #[inline]
    pub(crate) fn maxlen(&self, cp: u32) -> usize {
        self.maxlen.get(&cp).copied().unwrap_or(0)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.singles.is_empty() && self.contractions.is_empty()
    }

    /// All keys (single code points and contractions) defined here.
    pub(crate) fn keys(&self) -> impl Iterator<Item = Vec<u32>> + '_ {
        self.singles
            .keys()
            .map(|&c| vec![c])
            .chain(self.contractions.keys().map(|k| k.to_vec()))
    }

    /// Add entries in allkeys syntax (`CP [CP…] ; [.XXXX.XXXX.XXXX(.XXXX)]…`); later
    /// definitions replace earlier ones. Comments (`#…`), blank lines and `@` lines are
    /// ignored.
    pub(crate) fn add_entries(&mut self, text: &str) -> Result<(), Error> {
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with('@') {
                continue;
            }
            self.add_line(line)
                .map_err(|e| Error::InvalidEntry(format!("{e}: {raw}")))?;
        }
        Ok(())
    }

    fn add_line(&mut self, line: &str) -> Result<(), &'static str> {
        let (k, v) = line.split_once(';').ok_or("missing ';'")?;
        let cps = k
            .split_whitespace()
            .map(|h| u32::from_str_radix(h, 16).map_err(|_| "bad code point"))
            .collect::<Result<Vec<u32>, _>>()?;
        if cps.is_empty() {
            return Err("no code point");
        }
        let start = self.ces.len() as u32;
        for elt in v.split(']') {
            let elt = elt.trim();
            if elt.is_empty() {
                continue;
            }
            let elt = elt.strip_prefix('[').ok_or("collation element must start with '['")?;
            let var = match elt.as_bytes().first() {
                Some(b'*') => true,
                Some(b'.') => false,
                _ => return Err("collation element must start with '[.' or '[*'"),
            };
            let mut w = [0u16; 4];
            let mut n = 0;
            for h in elt[1..].split('.') {
                if n == 4 {
                    return Err("too many weights");
                }
                w[n] = u16::from_str_radix(h.trim(), 16).map_err(|_| "bad weight")?;
                n += 1;
            }
            if n < 3 {
                return Err("too few weights");
            }
            self.ces.push(Ce::new(w[0], w[1], w[2], w[3], var));
        }
        let r = (start, self.ces.len() as u32 - start);
        if cps.len() == 1 {
            self.singles.insert(cps[0], r);
        } else {
            let ml = self.maxlen.entry(cps[0]).or_insert(1);
            *ml = (*ml).max(cps.len());
            self.contractions.insert(cps.into_boxed_slice(), r);
        }
        Ok(())
    }
}

/// A `Unicode::Collate::Locale` tailoring file (`Locale/<stem>.pl`).
#[derive(Debug)]
pub(crate) struct LocaleData {
    pub locale_version: Option<String>,
    pub upper_before_lower: Option<bool>,
    pub backwards: Option<u8>,
    pub variable: Option<Variable>,
    pub suppress: Vec<u32>,
    pub override_cjk: Option<&'static CjkTable>,
    pub tailoring: Arc<Tailoring>,
}

static LOCALES_TXT: &str = include_str!("../data/locales.txt");

fn parse_locale(stem: &str) -> Result<LocaleData, Error> {
    let header = format!("@locale {stem}\n");
    let start = LOCALES_TXT
        .find(&header)
        .or_else(|| LOCALES_TXT.find(&format!("@locale {stem}\r\n")))
        .ok_or_else(|| Error::InvalidOption(format!("no tailoring data for {stem}")))?;
    let body = &LOCALES_TXT[start..];
    let body = &body[body.find('\n').map_or(body.len(), |i| i + 1)..];
    let end = body.find("@locale ").unwrap_or(body.len());
    let body = &body[..end];

    let mut d = LocaleData {
        locale_version: None,
        upper_before_lower: None,
        backwards: None,
        variable: None,
        suppress: Vec::new(),
        override_cjk: None,
        tailoring: Arc::new(Tailoring::default()),
    };
    let mut t = Tailoring::default();
    let mut entries = String::new();
    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(opt) = line.strip_prefix('@') {
            let (k, v) = opt.split_once(' ').unwrap_or((opt, ""));
            match k {
                "locale_version" => d.locale_version = Some(v.to_string()),
                "upper_before_lower" => d.upper_before_lower = Some(v != "0" && !v.is_empty()),
                "backwards" => d.backwards = v.parse().ok(),
                "variable" | "alternate" => d.variable = Some(Variable::parse(v)?),
                "suppress" => {
                    d.suppress = v
                        .split_whitespace()
                        .filter_map(|h| u32::from_str_radix(h, 16).ok())
                        .collect();
                }
                "overrideCJK" => d.override_cjk = Some(crate::cjk::table(v)?),
                _ => {
                    return Err(Error::InvalidOption(format!(
                        "unknown option {k} in tailoring {stem}"
                    )));
                }
            }
        } else {
            entries.push_str(line);
            entries.push('\n');
        }
    }
    t.add_entries(&entries)?;
    d.tailoring = Arc::new(t);
    Ok(d)
}

/// The (cached) tailoring data for a locale file stem.
pub(crate) fn locale_data(stem: &'static str) -> Result<Arc<LocaleData>, Error> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Arc<LocaleData>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(d) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(stem) {
        return Ok(d.clone());
    }
    let d = Arc::new(parse_locale(stem)?);
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(stem, d.clone());
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries() {
        let mut t = Tailoring::default();
        t.add_entries("0063 0068 ; [.0E6A.0020.0002.0063] # ch\n00E6 ; [.0E33.0020.0002][*0E8B.0020.0002]\n")
            .unwrap();
        assert_eq!(t.maxlen(0x63), 2);
        assert_eq!(
            t.contraction(&[0x63, 0x68]),
            Some(&[Ce::new(0xE6A, 0x20, 2, 0x63, false)][..])
        );
        assert_eq!(t.single(0xE6).unwrap().len(), 2);
        assert!(t.single(0xE6).unwrap()[1].var);
        assert!(t.add_entries("0061 ; [.1.2]").is_err());
    }

    #[test]
    fn all_locales_parse() {
        for stem in crate::locale::tailored_locales().filter_map(crate::locale::file_stem) {
            let d = locale_data(stem).unwrap();
            assert_eq!(d.locale_version.as_deref(), Some("1.31"));
            assert!(stem == "fr_ca" || !d.tailoring.is_empty(), "{stem}");
        }
        let da = locale_data("da").unwrap();
        assert_eq!(da.upper_before_lower, Some(true));
        let cu = locale_data("cu").unwrap();
        assert_eq!(cu.backwards, Some(2));
        assert_eq!(cu.suppress, vec![0x418, 0x438]);
        let th = locale_data("th").unwrap();
        assert_eq!(th.variable, Some(Variable::Shifted));
    }
}

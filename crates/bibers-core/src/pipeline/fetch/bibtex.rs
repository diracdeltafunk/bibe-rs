//! `Biber::Input::file::bibtex`: reading BibTeX data sources
//! (`extract_entries`, `cache_data`, `preprocess_file`, `parse_decode`),
//! creating entries (`create_entry`, `_create_entry`) and the field handlers.

use super::dates::{self, DateParser, JulianOpts, Parsed};
use super::names::{self, NameCfg, remove_outer};
use super::{SecExt, encoding};
use crate::config::blx::{OptValue, Scope, expand_option_input};
use crate::log::{Fatal, Logger};
use crate::model::{AnnotationScope, Entry, Name, NameArena, SectionState, Value};
use crate::par;
use crate::perlish::{fc, lc, nfc, nfd, nfd_string, truthy};
use crate::pipeline::Ctx;
use crate::pipeline::sourcemap::{self, RawEntry};
use crate::{biber_error, debug, info};
use perl_compat::case::fc_eq;
use perl_compat::regex::PerlRegex;
use perl_compat::unicode::{Prop, is_perl_digit, is_perl_space};
use serde_json::{Map, Value as J};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// One cached data file (`$cache->{data}{$filename}`).
#[derive(Clone, Debug, Default)]
pub struct FileData {
    /// Entries by key (pass-2 `Text::BibTeX::Entry` objects).
    pub entries: HashMap<String, Arc<btparse::Entry>>,
    /// Distinct keys in first-cached order (bibers iterates this where biber
    /// iterates the hash: one of biber's possible orders).
    pub keys: Vec<String>,
}

/// `Biber::Input::file::bibtex`'s run-level `$cache`.
#[derive(Clone, Debug, Default)]
pub struct BibCache {
    /// `{data}{$filename}`; present when Perl's hash ref would be true.
    pub data: HashMap<String, FileData>,
    /// `{data}{GLOBALDS}`: all cached entries of all files.
    pub globalds: HashMap<String, Arc<btparse::Entry>>,
    /// `{data}{citekey_aliases}`: alias → key.
    pub citekey_aliases: BTreeMap<String, String>,
    /// `{counts}{$filename}`.
    pub counts: HashMap<String, u32>,
    /// `{preamble}{$filename}`.
    pub preamble: HashMap<String, Vec<String>>,
    /// `{orig_key_order}{$filename}`.
    pub orig_key_order: HashMap<String, Vec<String>>,
    /// `{comments}{$filename}`: `@comment` values (tool mode only, unless
    /// `--strip-comments`), after `process_comment`.
    pub comments: HashMap<String, Vec<String>>,
    /// `$Biber::MASTER->{comments}`: the comments of the last data source
    /// read that has any (bibtex.pm:338-342); written by the bibtex output.
    pub master_comments: Option<Vec<String>>,
    /// `%RSTRINGS` (Constants.pm:100): `@string` value → macro name, over
    /// every data file read (bibtex.pm:1535-1539). Used by the bibtex
    /// output to write macros back.
    pub rstrings: HashMap<String, String>,
}

impl BibCache {
    /// Free the cached entries (once no data source will be read again);
    /// what the output writers use (`rstrings`, `master_comments`) stays.
    pub fn release_entries(&mut self) {
        self.data = HashMap::new();
        self.globalds = HashMap::new();
    }
}

/// Option values used while reading data, looked up once per `fetch_data`.
pub struct FetchOpts {
    pub xsvsep: Option<Arc<PerlRegex>>,
    pub namesep: String,
    pub listsep: String,
    pub xnamesep: String,
    pub xdatamarker: String,
    pub xdatasep: String,
    /// `$xdatamarker$xnamesep`.
    pub xdata_prefix: String,
    pub others_string: String,
    pub annotation_marker: String,
    pub named_annotation_marker: String,
    pub noinit: Vec<Arc<PerlRegex>>,
    pub fixinits: bool,
    pub noxname: bool,
    pub isbn13: bool,
    pub isbn10: bool,
    pub isbn_normalise: bool,
    pub validate_datamodel: bool,
    pub nostdmacros: bool,
    pub noskipduplicates: bool,
    pub julian: JulianOpts,
    pub nameparts: Vec<String>,
    pub verbs: Vec<String>,
    pub recoder: Arc<latex_recode::Recoder>,
    pub quiet_btparse: bool,
}

impl FetchOpts {
    pub fn new(ctx: &Ctx) -> FetchOpts {
        let b = &ctx.cfg.biber;
        let s = |o: &str| b.get_str(o).unwrap_or("").to_string();
        let xsv = s("xsvsep");
        let xsvsep = if xsv == r"\s*,\s*" {
            None
        } else {
            PerlRegex::cached(&xsv, false).ok()
        };
        let noinit = b
            .patterns("noinit")
            .iter()
            .filter_map(|p| p.value.as_deref())
            .filter_map(|v| PerlRegex::cached(v, false).ok())
            .collect();
        let julian = ctx.cfg.blx.global("julian").is_some_and(OptValue::truthy);
        let gregorianstart = ctx
            .cfg
            .blx
            .global("gregorianstart")
            .and_then(OptValue::as_str)
            .and_then(parse_gs);
        let recoder = ctx.cfg.recoder(latex_recode::RecodeOptions::default());
        FetchOpts {
            xsvsep,
            namesep: s("namesep"),
            listsep: s("listsep"),
            xnamesep: s("xnamesep"),
            xdatamarker: s("xdatamarker"),
            xdatasep: s("xdatasep"),
            xdata_prefix: s("xdatamarker") + &s("xnamesep"),
            others_string: s("others_string"),
            annotation_marker: s("annotation_marker"),
            named_annotation_marker: s("named_annotation_marker"),
            noinit,
            fixinits: b.is_true("fixinits"),
            noxname: b.is_true("noxname"),
            isbn13: b.is_true("isbn13"),
            isbn10: b.is_true("isbn10"),
            isbn_normalise: b.is_true("isbn_normalise"),
            validate_datamodel: b.is_true("validate_datamodel"),
            nostdmacros: b.is_true("nostdmacros"),
            noskipduplicates: b.is_true("noskipduplicates"),
            julian: JulianOpts {
                enabled: julian,
                gregorianstart,
            },
            nameparts: ctx.dm.get_constant_value("nameparts"),
            verbs: ctx.dm.helpers.verbs.iter().map(|v| fc(v)).collect(),
            recoder,
            quiet_btparse: !(ctx.log.is_debug() || ctx.log.is_trace()),
        }
    }

    /// `split(/$xsvsep/, $s)`.
    pub fn split_xsv(&self, s: &str) -> Vec<String> {
        match &self.xsvsep {
            None => perl_compat::regex::split_default_xsvsep(s),
            Some(re) => re
                .split(s)
                .map(|v| v.into_iter().map(Option::unwrap_or_default).collect())
                .unwrap_or_else(|_| vec![s.to_string()]),
        }
    }
}

fn parse_gs(s: &str) -> Option<(i64, u32, u32)> {
    let c: Vec<char> = s.chars().collect();
    if c.len() != 10
        || !Prop::Dash.contains(c[4])
        || !Prop::Dash.contains(c[7])
        || ![0, 1, 2, 3, 5, 6, 8, 9]
            .iter()
            .all(|&i| c[i].is_ascii_digit())
    {
        return None;
    }
    let n = |r: std::ops::Range<usize>| c[r].iter().collect::<String>().parse().ok();
    Some((n(0..4)?, n(5..7)? as u32, n(8..10)? as u32))
}

/// Per-`extract_entries` state: the captured libbtparse stderr.
struct Extract<'a> {
    filename: &'a str,
    btlines: Vec<String>,
}

/// Noncharacters (U+FDD0..U+FDEF, U+xFFFE, U+xFFFF) → U+FFFD.
fn replace_noncharacters(s: String) -> String {
    // Their UTF-8 forms start with 0xEF (BMP) or 0xF0..0xF4 (other planes).
    if !s.bytes().any(|b| b >= 0xEF) {
        return s;
    }
    s.chars()
        .map(|c| {
            let v = c as u32;
            if (v & 0xFFFE) == 0xFFFE || (0xFDD0..=0xFDEF).contains(&v) {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect()
}

/// `\R` → `\n`.
fn normalise_newlines(s: &str) -> Cow<'_, str> {
    // Every `\R` other than `\n` contains one of these bytes (U+0085 is
    // C2 85, U+2028/U+2029 are E2 80 A8/A9).
    if !s
        .bytes()
        .any(|b| matches!(b, b'\r' | 0x0B | 0x0C | 0xC2 | 0xE2))
    {
        return Cow::Borrowed(s);
    }
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\r' => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
                o.push('\n');
            }
            '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}' => o.push('\n'),
            c => o.push(c),
        }
    }
    Cow::Owned(o)
}

/// The `RawEntry` a cached Text::BibTeX entry exposes to source maps.
fn to_raw(e: &btparse::Entry) -> RawEntry {
    RawEntry {
        entrytype: e.entry_type.clone(),
        key: e.key.clone().unwrap_or_default(),
        fields: e
            .fieldlist_values()
            .map(|(n, v)| (n.to_string(), v.unwrap_or("").to_string()))
            .collect(),
    }
}

/// `$entry->get($field)` on a mapped entry: the last occurrence.
fn raw_get<'e>(e: &'e RawEntry, f: &str) -> Option<&'e str> {
    e.get(f)
}

/// `Text::BibTeX::Entry::print_s` for a regular entry.
fn print_s(e: &RawEntry, key: &str) -> String {
    let len: usize = e.fields.iter().map(|(f, v)| f.len() + v.len() + 8).sum();
    let mut o = String::with_capacity(e.entrytype.len() + key.len() + len + 8);
    for s in ["@", &e.entrytype, "{", key, ",\n"] {
        o.push_str(s);
    }
    for (f, _) in &e.fields {
        let v = raw_get(e, f).unwrap_or("");
        for s in ["  ", f, " = {", v, "},\n"] {
            o.push_str(s);
        }
    }
    o.push_str("}\n\n");
    nfd_string(o)
}

/// `extract_entries` (bibtex.pm:109). Returns the keys not found.
pub fn extract_entries(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    filename: &str,
    encoding: &str,
    keys: &[String],
) -> Result<Vec<String>, Fatal> {
    let secnum = st.section.number.clone();
    let mut rkeys: Vec<String> = keys.to_vec();
    let path = ctx.env.resolve(filename);
    // `check_empty` is `-s $filename`, a stat: a file that exists but can't be
    // read is not "empty", its read fails below.
    if std::fs::metadata(&path).map_or(true, |m| m.len() == 0) {
        ctx.log
            .biber_warn(format!("Data source '{filename}' is empty, ignoring"));
        return Ok(rkeys);
    }
    let utf16_chunks = ctx.cfg.quirks.on(crate::quirks::Quirk::Utf16ChunkSurrogate);
    let text = match std::fs::read(&path)
        .map_err(|e| {
            format!(
                "Couldn't open {filename}: {}",
                crate::perlish::errno_text(&e)
            )
        })
        .and_then(|bytes| encoding::decode(&bytes, encoding, utf16_chunks))
    {
        Ok(t) => Some(t),
        Err(e) => {
            biber_error!(
                ctx.log,
                "Data file '{filename}' cannot be read in encoding '{encoding}': {e}"
            )?;
            None
        }
    };
    if !text.as_deref().is_some_and(|t| t.contains('@')) {
        ctx.log.biber_warn(format!(
            "Data source '{filename}' contains no BibTeX entries/macros, ignoring"
        ));
        return Ok(rkeys);
    }
    let text = text.unwrap_or_default();
    let mut x = Extract {
        filename,
        btlines: Vec::new(),
    };

    *ctx.run
        .bibcache
        .counts
        .entry(filename.to_string())
        .or_default() += 1;
    if !ctx.run.bibcache.data.contains_key(filename) {
        debug!(
            ctx.log,
            "Caching data for BibTeX format file '{filename}' for section {secnum}"
        );
        crate::par::timed("cache_data", || {
            cache_data(ctx, st, opts, &mut x, filename, encoding, &text)
        })?;
    }
    drop(text);

    if st.section.allkeys {
        let fkeys = ctx
            .run
            .bibcache
            .data
            .entry(filename.to_string())
            .or_default()
            .keys
            .clone();
        let mut stage = Stage::default();
        let mut premapped = None;
        for (i, key) in fkeys.iter().enumerate() {
            // (in windows, so that only a window's mapped copies exist)
            if i % STAGE_WINDOW == 0 {
                let end = fkeys.len().min(i + STAGE_WINDOW);
                premapped = premap(ctx, st, filename, &fkeys[i..end]);
            }
            st.section.set_keytods(key, filename);
            let entry = ctx.run.bibcache.data[filename].entries[key].clone();
            let pre = premapped.as_mut().and_then(|p| p[i % STAGE_WINDOW].take());
            let created = create_entry(
                ctx, st, opts, &mut x, &mut stage, pre, key, &entry, filename, &mut rkeys,
            )?;
            if !created {
                prune_orig_order(ctx, filename, key);
            }
        }
        stage.flush(ctx, st, opts, &mut x)?;
        let aliases: Vec<(String, String)> = ctx
            .run
            .bibcache
            .citekey_aliases
            .iter()
            .map(|(a, k)| (a.clone(), k.clone()))
            .collect();
        for (a, k) in aliases {
            st.section.citekey_alias.insert(a, k);
        }
        let order = ctx
            .run
            .bibcache
            .orig_key_order
            .entry(filename.to_string())
            .or_default()
            .clone();
        st.section.add_citekeys(&order);
        // has_dynamic_sets is always true; dynamic_set_keys is in Perl hash
        // order (DC-015: we use .bcf declaration order).
        let mut dkeys = st.section.dynamic_set_keys();
        if let Some(bs) = ctx.doc.section(&secnum) {
            let pos = |k: &String| {
                bs.citekeys
                    .iter()
                    .position(|c| c == k)
                    .unwrap_or(usize::MAX)
            };
            dkeys.sort_by_key(pos);
        }
        st.section.add_citekeys(&dkeys);
    } else {
        if !keys.is_empty() {
            ctx.run
                .bibcache
                .data
                .entry(filename.to_string())
                .or_default();
        }
        // (keys found: removed from `rkeys` once, after the loop)
        let mut found: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut stage = Stage::default();
        for wanted in keys {
            st.section.set_keytods(wanted, filename);
            let cached = ctx
                .run
                .bibcache
                .data
                .get(filename)
                .and_then(|d| d.entries.get(wanted))
                .cloned();
            if let Some(entry) = cached {
                if !st.section.entries.contains_key(wanted)
                    && !stage.has(wanted)
                    && !create_entry(
                        ctx, st, opts, &mut x, &mut stage, None, wanted, &entry, filename,
                        &mut rkeys,
                    )?
                {
                    st.section.del_citekey(wanted);
                    prune_orig_order(ctx, filename, wanted);
                    ctx.log.biber_warn(format!(
                        "Entry with key '{wanted}' in section '{secnum}' is cited and found but not created (likely due to sourcemap)"
                    ));
                }
                found.insert(wanted.as_str());
            } else if let Some(rk) = ctx.run.bibcache.citekey_aliases.get(wanted).cloned() {
                st.section.citekey_alias.insert(wanted.clone(), rk.clone());
                if !st.section.entries.contains_key(&rk)
                    && !stage.has(&rk)
                    && let Some(entry) = ctx.run.bibcache.globalds.get(&rk).cloned()
                {
                    let created = create_entry(
                        ctx, st, opts, &mut x, &mut stage, None, &rk, &entry, filename, &mut rkeys,
                    )?;
                    if !created {
                        st.section.del_citekey(wanted);
                        prune_orig_order(ctx, filename, &rk);
                        ctx.log.biber_warn(format!(
                            "Entry with key '{rk}' in section '{secnum}' is cited and found but not created (likely due to sourcemap)"
                        ));
                    }
                    if st.section.citekeys_h.contains(wanted) {
                        st.section.add_citekeys(std::slice::from_ref(&rk));
                    }
                }
                found.insert(wanted.as_str());
            } else if let Some(okey) = st.section.has_badcasekey(wanted) {
                stage.flush(ctx, st, opts, &mut x)?;
                ctx.log.biber_warn(format!(
                    "Possible typo (case mismatch) between citation and datasource keys: '{wanted}' and '{okey}' in file '{filename}'"
                ));
            }
        }
        stage.flush(ctx, st, opts, &mut x)?;
        if !found.is_empty() {
            rkeys.retain(|k| !found.contains(k.as_str()));
        }
    }

    // The captured libbtparse messages.
    if opts.quiet_btparse {
        for line in std::mem::take(&mut x.btlines) {
            if line.contains("overriding existing definition of macro") {
                continue;
            }
            if line.contains("error:") {
                biber_error!(ctx.log, "BibTeX subsystem: {line}")?;
            } else if line.contains("warning:") {
                ctx.log.biber_warn(format!("BibTeX subsystem: {line}"));
            }
        }
    } else {
        for line in std::mem::take(&mut x.btlines) {
            eprintln!("{line}");
        }
    }

    let cache = &ctx.run.bibcache;
    if cache.counts.get(filename).copied().unwrap_or(0) < 2
        && let Some(p) = cache.preamble.get(filename).filter(|p| !p.is_empty())
    {
        st.preamble
            .get_or_insert_with(Vec::new)
            .extend(p.iter().cloned());
    }

    // Save comments if in tool mode (bibtex.pm:338-342): the last data
    // source with comments wins (quirk `tool-comments-last-file`); with the
    // quirk fixed, the comments of every data source are kept, in order.
    if ctx.cfg.biber.tool()
        && let Some(c) = ctx.run.bibcache.comments.get(filename).cloned()
    {
        let mc = &mut ctx.run.bibcache.master_comments;
        if ctx
            .cfg
            .quirks
            .on(crate::quirks::Quirk::ToolCommentsLastFile)
        {
            *mc = Some(c);
        } else if ctx.run.bibcache.counts.get(filename).copied().unwrap_or(0) < 2 {
            mc.get_or_insert_with(Vec::new).extend(c);
        }
    }
    Ok(rkeys)
}

fn prune_orig_order(ctx: &mut Ctx, filename: &str, key: &str) {
    ctx.run
        .bibcache
        .orig_key_order
        .entry(filename.to_string())
        .or_default()
        .retain(|k| k != key);
}

/// `cache_data` (bibtex.pm:1515), including `preprocess_file` and
/// `parse_decode`: biber's two parses, run in memory.
fn cache_data(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    x: &mut Extract,
    filename: &str,
    encoding: &str,
    text: &str,
) -> Result<(), Fatal> {
    ctx.run
        .bibcache
        .preamble
        .insert(filename.to_string(), Vec::new());
    if encoding == "ascii" {
        info!(ctx.log, "Reading ascii input as UTF-8");
    }
    let nfd_text = perl_compat::unicode::nfd(text);
    let buf = nfd_text.strip_prefix('\u{feff}').unwrap_or(&nfd_text);
    let buf = normalise_newlines(buf);

    // Pass 1 (parse_decode)
    info!(ctx.log, "LaTeX decoding ...");
    let p1 = parse_pass1(&mut ctx.run.bt, &buf);
    push_diags(x, &p1);
    let recoder = &opts.recoder;
    let verbs = &opts.verbs;
    fn decode<'v>(
        verbs: &[String],
        recoder: &latex_recode::Recoder,
        f: &str,
        v: &'v str,
    ) -> Cow<'v, str> {
        // (`verbs` are case-folded already)
        if verbs.iter().any(|vb| fc_eq(vb, f)) {
            Cow::Borrowed(v)
        } else {
            let d = recoder.decode_cow(v);
            // DC-003: a decoded value with unbalanced braces (e.g. `\char"7B`
            // → `{`) breaks biber's pass 2 (fatal error, the next entry is
            // swallowed: BB-003). Leave such a value undecoded instead: the
            // macro then reaches the .bbl, where LaTeX typesets it.
            if !btparse::compat::braces_round_trip(&d) && btparse::compat::braces_round_trip(v) {
                Cow::Borrowed(v)
            } else {
                d
            }
        }
    }
    // The pass-1 text, one piece per entry, decoded in parallel. biber
    // writes the decoded buffer to its temp file through PerlIO's
    // :utf8_strict layer (slurp_switchw), which turns noncharacters (e.g.
    // from `\char"FDD0`) into U+FFFD.
    let pieces: Vec<String> = par::map(&p1.entries, |e| {
        let mut out = String::new();
        btparse::compat::reserialise_entry(e, &mut out, |f, v| decode(verbs, recoder, f, v));
        replace_noncharacters(out)
    });
    let kinds: Vec<btparse::Metatype> = p1.entries.iter().map(|e| e.metatype).collect();
    // Pass 1's entries and the input text are done with: free them before
    // pass 2 builds the cache (a large file is three copies otherwise).
    drop(p1);
    drop(buf);
    drop(nfd_text);
    if !opts.nostdmacros {
        for (k, v) in btparse::BIBER_MONTHS {
            ctx.run.bt.macros.define(k, v);
        }
    }

    // Pass 2 (cache_data proper)
    let p2 = parse_pieces(&mut ctx.run.bt, &kinds, &pieces);
    drop(pieces);
    push_diags(x, &p2);
    info!(ctx.log, "Found BibTeX data source '{filename}'");

    for entry in p2.entries {
        match entry.metatype {
            btparse::Metatype::Preamble => {
                ctx.run
                    .bibcache
                    .preamble
                    .entry(filename.to_string())
                    .or_default()
                    .push(nfd(entry.value.as_deref().unwrap_or("")));
                continue;
            }
            // Save comments for output in tool mode unless comment stripping
            // is requested (bibtex.pm:1527-1533).
            btparse::Metatype::Comment => {
                if ctx.cfg.biber.tool() && !ctx.cfg.biber.is_true("strip_comments") {
                    let c = process_comment(entry.value.as_deref().unwrap_or(""));
                    ctx.run
                        .bibcache
                        .comments
                        .entry(filename.to_string())
                        .or_default()
                        .push(c);
                }
                continue;
            }
            // Record macros so that tool mode can output them
            // (bibtex.pm:1535-1539).
            btparse::Metatype::MacroDef => {
                for (f, v) in entry.fieldlist_values() {
                    // `$entry->get($f)` with the NFD name from `fieldlist`
                    // finds nothing when the name has decomposable
                    // characters (Text::BibTeX keys fields by NFC bytes).
                    let v = if nfc(f) == f { v.unwrap_or("") } else { "" };
                    ctx.run
                        .bibcache
                        .rstrings
                        .insert(v.to_string(), f.to_string());
                }
                continue;
            }
            btparse::Metatype::Regular => {}
        }
        let key = match entry.key.as_deref() {
            Some(k) if perl_compat::perl_truthy(k) => k.to_string(),
            _ => {
                ctx.log.biber_warn(format!(
                    "Invalid or undefined BibTeX entry key in file '{filename}', skipping ..."
                ));
                continue;
            }
        };
        let cache = &mut ctx.run.bibcache;
        if cache.citekey_aliases.remove(&key).is_some() {
            ctx.log.biber_warn(format!(
                "Citekey alias '{key}' is also a real entry key, skipping ..."
            ));
        }
        if let Some(ids) = entry.get("ids").filter(|v| perl_compat::perl_truthy(v)) {
            for id in opts.split_xsv(ids) {
                if id == key {
                    ctx.log.biber_warn(format!(
                        "BAD RECURSION! Entry alias '{id}' is identical to the entry key, skipping ..."
                    ));
                    continue;
                }
                if st.section.has_everykey(&id) {
                    ctx.log.biber_warn(format!(
                        "Entry alias '{id}' is also a real entry key, skipping ..."
                    ));
                    continue;
                }
                let cache = &mut ctx.run.bibcache;
                if let Some(other) = cache.citekey_aliases.get(&id) {
                    if *other != key {
                        let other = other.clone();
                        ctx.log.biber_warn(format!(
                            "Entry alias '{id}' already has an alias '{other}', skipping ..."
                        ));
                    }
                } else {
                    cache.citekey_aliases.insert(id, key.clone());
                }
            }
        }
        if let Some(okey) = st.section.has_badcasekey(&key) {
            ctx.log.biber_warn(format!(
                "Possible typo (case mismatch) between datasource keys: '{key}' and '{okey}' in file '{filename}'"
            ));
        }
        if st.section.has_everykey(&key) && !opts.noskipduplicates {
            ctx.log.biber_warn(format!(
                "Duplicate entry key: '{key}' in file '{filename}', skipping ..."
            ));
            continue;
        }
        if st.section.has_everykey(&key) {
            ctx.log
                .biber_warn(format!("Duplicate entry key: '{key}' in file '{filename}'"));
        }
        st.section.add_everykey(&key);
        if !entry.parse_ok {
            ctx.log
                .biber_warn(format!("Entry {key} does not parse correctly"));
            continue;
        }
        let cache = &mut ctx.run.bibcache;
        let entry = Arc::new(entry);
        cache.globalds.insert(key.clone(), Arc::clone(&entry));
        let fd = cache.data.entry(filename.to_string()).or_default();
        if !fd.entries.contains_key(&key) {
            fd.keys.push(key.clone());
        }
        fd.entries.insert(key.clone(), entry);
        cache
            .orig_key_order
            .entry(filename.to_string())
            .or_default()
            .push(key);
    }
    Ok(())
}

/// `BtState::parse` of the data file's text `buf` (pass 1), in parallel
/// when it splits cleanly into entries.
///
/// The text is cut before every line that starts with `@` (after blanks).
/// `@string`, `@comment` and `@preamble` pieces are parsed in order with
/// (a copy of) the real state; each regular entry's piece is parsed on its
/// own, in parallel, with a copy of the macros defined before it. When
/// every piece parses as exactly one entry of the expected type without
/// any diagnostic ([`parse_piece`]), this is what the parse of the whole
/// text gives (entries do not change the parser state; `@string`s only add
/// macros); otherwise the whole text is parsed.
fn parse_pass1(bt: &mut btparse::BtState, buf: &str) -> btparse::ParsedFile {
    use btparse::Metatype;
    // Piece starts: the starts of lines whose first non-blank is `@`.
    let mut starts = Vec::new();
    let b = buf.as_bytes();
    let mut line = 0;
    while line < b.len() {
        let mut i = line;
        while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        if i < b.len() && b[i] == b'@' {
            starts.push(line);
        }
        match memchr::memchr(b'\n', &b[line..]) {
            Some(n) => line += n + 1,
            None => break,
        }
    }
    if starts.is_empty() || !par::is_parallel(starts.len()) {
        return bt.parse(buf);
    }
    // (text before the first entry goes with it)
    starts[0] = 0;
    starts.push(b.len());
    let pieces: Vec<&str> = starts.windows(2).map(|w| &buf[w[0]..w[1]]).collect();
    let last = pieces.len() - 1;
    let kind = |p: &str| {
        let t = p.trim_start_matches(|c: char| c != '@')[1..].trim_start();
        let name: String = t
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.eq_ignore_ascii_case("string") {
            Metatype::MacroDef
        } else if name.eq_ignore_ascii_case("comment") {
            Metatype::Comment
        } else if name.eq_ignore_ascii_case("preamble") {
            Metatype::Preamble
        } else {
            Metatype::Regular
        }
    };
    let kinds: Vec<Metatype> = pieces.iter().map(|p| kind(p)).collect();
    // The other pieces in order, with a copy of the state; the macros each
    // regular piece sees.
    let mut state = bt.clone();
    let mut snapshots: Vec<btparse::MacroTable> = Vec::new();
    let mut changed = true;
    let mut parsed: Vec<Option<btparse::Entry>> = Vec::with_capacity(pieces.len());
    let mut snapshot_of = vec![0usize; pieces.len()];
    for (i, p) in pieces.iter().enumerate() {
        if kinds[i] == Metatype::Regular {
            if changed {
                if snapshots.len() == 64 {
                    // (@strings and entries alternate: not worth it)
                    return bt.parse(buf);
                }
                snapshots.push(state.macros.clone());
                changed = false;
            }
            snapshot_of[i] = snapshots.len() - 1;
            parsed.push(None);
        } else {
            let Some(e) = parse_piece(&mut state, p, i < last, kinds[i]) else {
                return bt.parse(buf);
            };
            changed |= kinds[i] == Metatype::MacroDef;
            parsed.push(Some(e));
        }
    }
    // The regular pieces, in parallel.
    let regular: Vec<usize> = (0..pieces.len())
        .filter(|&i| kinds[i] == Metatype::Regular)
        .collect();
    let done: Vec<Vec<(usize, Option<btparse::Entry>)>> = par::map_slices(&regular, |slice| {
        let mut st: Option<(usize, btparse::BtState)> = None;
        slice
            .iter()
            .map(|&i| {
                let g = snapshot_of[i];
                if st.as_ref().is_none_or(|(sg, _)| *sg != g) {
                    st = Some((g, btparse::BtState::new(snapshots[g].clone())));
                }
                let (_, s) = st.as_mut().expect("set");
                (i, parse_piece(s, pieces[i], i < last, Metatype::Regular))
            })
            .collect()
    });
    for (i, e) in done.into_iter().flatten() {
        let Some(e) = e else {
            return bt.parse(buf);
        };
        parsed[i] = Some(e);
    }
    let mut entries = Vec::with_capacity(pieces.len());
    let mut line = 1u32;
    for (i, e) in parsed.into_iter().enumerate() {
        let mut e = e.expect("every piece parsed");
        e.line += line - 1;
        entries.push(e);
        line += pieces[i].bytes().filter(|&b| b == b'\n').count() as u32;
    }
    *bt = state;
    btparse::ParsedFile {
        entries,
        stop: None,
        trailing_diags: Vec::new(),
    }
}

/// Parse one piece of [`parse_pass1`]: its entry, if it is exactly one
/// entry of type `kind` without any diagnostic. A piece `followed` by
/// another is parsed with an `@comment{}` after it, so that junk after its
/// entry is reported as it would be before the next piece's `@`.
fn parse_piece(
    st: &mut btparse::BtState,
    piece: &str,
    followed: bool,
    kind: btparse::Metatype,
) -> Option<btparse::Entry> {
    let pf = if followed {
        st.parse(&format!("{piece}@comment{{}}"))
    } else {
        st.parse(piece)
    };
    let n = 1 + usize::from(followed);
    let clean = pf.stop.is_none()
        && pf.trailing_diags.is_empty()
        && pf.entries.len() == n
        && pf.entries.iter().all(|e| e.diags.is_empty())
        && pf.entries[0].parse_ok
        && pf.entries[0].metatype == kind
        && (!followed || pf.entries[1].metatype == btparse::Metatype::Comment);
    clean.then(|| pf.entries.into_iter().next().expect("one entry"))
}

/// `BtState::parse` of the concatenation of `pieces`, the pass-1 texts of
/// the entries of types `p1` (one each).
///
/// Pass 2 reads text bibers wrote itself, so it normally parses cleanly,
/// one entry per piece. A regular entry's piece has only braced values (no
/// macros) and, if it parses cleanly on its own, parses the same way within
/// the whole text, where the parser is back at the top level after each
/// entry: so the regular pieces are parsed in parallel, each on its own,
/// and the others (`@string`s, which define macros, `@preamble`s and
/// `@comment`s) in order with the real state. If any piece does not parse
/// as exactly one clean entry of its type, the whole text is parsed
/// instead, from the state before.
fn parse_pieces(
    bt: &mut btparse::BtState,
    p1: &[btparse::Metatype],
    pieces: &[String],
) -> btparse::ParsedFile {
    if !par::is_parallel(pieces.len()) {
        return bt.parse(&pieces.concat());
    }
    let regular = |i: usize| p1[i] == btparse::Metatype::Regular;
    let idx: Vec<usize> = (0..pieces.len()).collect();
    let mut parsed: Vec<Option<btparse::ParsedFile>> = par::map(&idx, |&i| {
        regular(i).then(|| btparse::BtState::new(btparse::MacroTable::default()).parse(&pieces[i]))
    });
    let saved = bt.clone();
    let mut entries = Vec::with_capacity(pieces.len());
    let mut line = 1u32;
    for (i, piece) in pieces.iter().enumerate() {
        let pf = parsed[i].take().unwrap_or_else(|| bt.parse(piece));
        // (An `@string` parsed in order redefines pass 1's macros: btparse
        // warns, and the caller drops the warning.)
        let harmless = |d: &btparse::Diag| {
            !regular(i)
                && d.kind == btparse::DiagKind::Warning
                && d.message
                    .starts_with("overriding existing definition of macro")
        };
        let clean = pf.stop.is_none()
            && pf.trailing_diags.is_empty()
            && pf.entries.len() == 1
            && pf.entries[0].parse_ok
            && pf.entries[0].diags.iter().all(harmless)
            && pf.entries[0].metatype == p1[i];
        if !clean {
            *bt = saved;
            return bt.parse(&pieces.concat());
        }
        let mut e = pf.entries.into_iter().next().expect("one entry");
        e.line += line - 1;
        for d in &mut e.diags {
            d.line += line - 1;
        }
        entries.push(e);
        line += piece.bytes().filter(|&b| b == b'\n').count() as u32;
    }
    btparse::ParsedFile {
        entries,
        stop: None,
        trailing_diags: Vec::new(),
    }
}

/// `process_comment` (Utils.pm:1586): re-instate the line breaks of
/// structured JabRef comments.
fn process_comment(comment: &str) -> String {
    if !comment.contains("jabref-meta:") {
        return comment.to_string();
    }
    // s/([:;])\s(\d)/$1\n$2/xmsg
    let c: Vec<char> = comment.chars().collect();
    let mut out = String::with_capacity(comment.len() + 8);
    let mut i = 0;
    while i < c.len() {
        if (c[i] == ':' || c[i] == ';')
            && i + 2 < c.len()
            && is_perl_space(c[i + 1])
            && is_perl_digit(c[i + 2])
        {
            out.push(c[i]);
            out.push('\n');
            out.push(c[i + 2]);
            i += 3;
            continue;
        }
        out.push(c[i]);
        i += 1;
    }
    // s/\z/\n/xms
    out.push('\n');
    out
}

fn push_diags(x: &mut Extract, p: &btparse::ParsedFile) {
    // DC-042: messages name the user's data file.
    for d in p.all_diags() {
        x.btlines.push(d.render(x.filename));
    }
    if let Some((_, true)) = &p.stop {
        x.btlines.push(format!(
            "{}, error: libbtparse cannot parse this file (biber would crash here)",
            x.filename
        ));
    }
}

/// Entries whose source maps have run, waiting to be created.
///
/// `create_entry` runs the source maps (which are order-dependent) at once,
/// but defers `_create_entry` (the field handlers, most of the work): the
/// deferred entries are built in parallel and applied in order by
/// [`Stage::flush`]. Building an entry only changes the section in ways the
/// rest of the loop cannot observe, except when it skips the entry (which
/// deletes its citekey); entries that might be skipped ([`may_skip`]) are
/// therefore created at once, after flushing. The loop flushes before it
/// logs, before it reads anything an entry's creation changes, and at its
/// end.
#[derive(Default)]
struct Stage {
    /// Per entry, in order: the source maps' messages and the entries to
    /// create.
    items: Vec<(Logger, Vec<CreateJob>)>,
    /// The keys of the entries in `items` (`exists $bibentries{$key}` must
    /// see them).
    keys: rustc_hash::FxHashSet<String>,
    /// The number of entries in `items`.
    jobs: usize,
}

/// The number of entries staged (and source-mapped ahead) at a time: enough
/// for the parallel loops, few enough that the copies of the entries they
/// hold stay small.
const STAGE_WINDOW: usize = 2048;

impl Stage {
    fn has(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    /// Create the staged entries.
    fn flush(
        &mut self,
        ctx: &mut Ctx,
        st: &mut SectionState,
        opts: &FetchOpts,
        x: &mut Extract,
    ) -> Result<(), Fatal> {
        if self.items.is_empty() {
            return Ok(());
        }
        self.keys.clear();
        self.jobs = 0;
        let items = std::mem::take(&mut self.items);
        let secnum = st.section.number.clone();
        let jobs: Vec<&CreateJob> = items.iter().flat_map(|(_, j)| j).collect();
        let built: Vec<Built> = {
            let c: &Ctx = ctx;
            par::map(&jobs, |job| build_entry(c, opts, &secnum, job))
        };
        let mut built = built.into_iter();
        for (map_log, jobs) in items {
            ctx.log.join(map_log);
            for job in &jobs {
                let b = built.next().expect("one build per job");
                apply_built(ctx, st, x, &job.key, b)?;
            }
        }
        Ok(())
    }
}

/// `create_entry` (bibtex.pm:356): runs the source maps, then creates the
/// entry (and any the maps made) or stages it ([`Stage`]). Returns false if
/// the entry was not created (skipped by a map or a field handler); the
/// stage is then empty.
#[allow(clippy::too_many_arguments)]
fn create_entry(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    x: &mut Extract,
    stage: &mut Stage,
    premapped: Option<Premapped>,
    key: &str,
    entry: &btparse::Entry,
    datasource: &str,
    rkeys: &mut Vec<String>,
) -> Result<bool, Fatal> {
    if entry.metatype != btparse::Metatype::Regular {
        return Ok(true);
    }
    let (raw, map_log, outcome) = match premapped {
        Some(p) => (p.raw, p.log, p.outcome),
        None => {
            let mut raw = to_raw(entry);
            raw.key = key.to_string();
            let mut log = ctx.log.fork();
            let outcome =
                sourcemap::apply_maps_logged(ctx.cfg, &mut log, st, "bibtex", datasource, &mut raw);
            (raw, log, outcome)
        }
    };
    let outcome = match outcome {
        Ok(o) => o,
        Err(e) => {
            stage.flush(ctx, st, opts, x)?;
            ctx.log.join(map_log);
            return Err(e);
        }
    };
    for k in &outcome.created_keys {
        rkeys.retain(|r| r != k);
    }
    if outcome.skip {
        stage.flush(ctx, st, opts, x)?;
        ctx.log.join(map_log);
        return Ok(false);
    }
    let ds = |k: &str| st.section.get_keytods(k).unwrap_or_default();
    let tbkey = raw.key.clone();
    let mut jobs = vec![CreateJob {
        key: key.to_string(),
        ds: ds(key),
        raw,
        tbkey,
    }];
    for (i, e) in outcome.new_entries.into_iter().enumerate() {
        let tbkey = outcome.tb_keys.get(i).unwrap_or(&e.key).clone();
        jobs.push(CreateJob {
            key: e.key.clone(),
            ds: ds(&e.key),
            raw: e,
            tbkey,
        });
    }
    if jobs.iter().any(|j| may_skip(ctx, opts, &j.raw)) {
        // Created now, in order.
        stage.flush(ctx, st, opts, x)?;
        ctx.log.join(map_log);
        let mut crret = true;
        for (i, job) in jobs.iter().enumerate() {
            let b = build_entry(ctx, opts, &st.section.number.clone(), job);
            let r = apply_built(ctx, st, x, &job.key, b)?;
            if i == 0 {
                crret = r;
            }
        }
        return Ok(crret);
    }
    stage.keys.extend(jobs.iter().map(|j| j.key.clone()));
    stage.jobs += jobs.len();
    stage.items.push((map_log, jobs));
    if stage.jobs >= STAGE_WINDOW {
        stage.flush(ctx, st, opts, x)?;
    }
    Ok(true)
}

/// An entry's source maps, run ahead of [`create_entry`] by [`premap`].
struct Premapped {
    log: Logger,
    outcome: Result<sourcemap::MapOutcome, Fatal>,
    raw: RawEntry,
}

/// Run the source maps of the entries `keys` of the data source `filename`
/// ahead, in parallel, when they are pure ([`sourcemap::maps_are_pure`]):
/// they then depend on nothing the creation of other entries changes.
/// `None` when they are to run in [`create_entry`].
fn premap(
    ctx: &Ctx,
    st: &SectionState,
    filename: &str,
    keys: &[String],
) -> Option<Vec<Option<Premapped>>> {
    if !par::is_parallel(keys.len()) || !sourcemap::maps_are_pure(ctx.cfg) {
        return None;
    }
    let data = ctx.run.bibcache.data.get(filename)?;
    Some(par::map(keys, |k| {
        let e = data.entries.get(k)?;
        if e.metatype != btparse::Metatype::Regular {
            return None;
        }
        let mut raw = to_raw(e);
        raw.key = k.clone();
        let mut log = ctx.log.fork();
        let outcome = sourcemap::apply_maps_pure(
            ctx.cfg,
            &mut log,
            &st.section,
            "bibtex",
            filename,
            &mut raw,
        );
        Some(Premapped { log, outcome, raw })
    }))
}

/// Whether creating `e` could skip it (the name handler's empty-name and
/// comma checks, `_name` in bibtex.pm:1201): a superset of the entries
/// that are skipped.
fn may_skip(ctx: &Ctx, opts: &FetchOpts, e: &RawEntry) -> bool {
    e.fields.iter().any(|(f, _)| {
        let value = raw_get(e, f).unwrap_or("");
        if value.is_empty() {
            return false;
        }
        let fcf = fc(f);
        let is_name = ctx
            .dm
            .field_id(&fcf)
            .map(|id| ctx.dm.field(id))
            .is_some_and(|d| {
                d.fieldtype.as_deref() == Some("list") && d.datatype.as_deref() == Some("name")
            });
        if !is_name {
            return false;
        }
        let v = nfc(value);
        btparse::split_list(&v, &opts.namesep)
            .items
            .iter()
            .any(|n| !perl_compat::perl_truthy(n) || n.contains(",,") || n.matches(',').count() > 2)
    })
}

/// What a field handler returns.
enum HRes {
    /// `undef`: nothing to set.
    None,
    Set(Value),
    /// `BIBER_SKIP_ENTRY`.
    Skip,
}

/// `biber_warn($msg, $entry)`.
pub(super) fn warn_entry(log: &mut Logger, be: &mut Entry, msg: String) {
    let w = be
        .derivedfields
        .entry("warnings")
        .or_insert_with(|| Some(Value::List(Vec::new())));
    if let Some(Value::List(l)) = w {
        l.push(msg.clone());
    }
    log.biber_warn(msg);
}

/// What `_create_entry` (bibtex.pm:940) does outside the entry it builds,
/// recorded so that entries can be built in parallel ([`build_entry`]) and
/// their effects applied in the serial order ([`apply_built`]).
pub(super) struct Build<'c> {
    cfg: &'c crate::config::Config,
    dm: &'c crate::datamodel::DataModel,
    secnum: &'c str,
    /// `RunState::skip_rawdata`.
    skip_rawdata: bool,
    log: Logger,
    /// libbtparse messages (`Extract::btlines`).
    btlines: Vec<String>,
    /// The entry's name lists and names, numbered from 0: moved into the
    /// section's arena (renumbered) when applied.
    names: NameArena,
    /// `process_entry_options` settings for the entry.
    options: Vec<(String, OptValue)>,
    annotations: Vec<AnnotationCall>,
    /// `set_np_length` calls.
    np_lengths: Vec<(String, usize)>,
    /// `del_citekey` of the entry's key (a skipped entry).
    deleted: bool,
}

/// A finished [`Build`] (without its borrows of the configuration).
struct Built {
    log: Logger,
    btlines: Vec<String>,
    names: NameArena,
    options: Vec<(String, OptValue)>,
    annotations: Vec<AnnotationCall>,
    np_lengths: Vec<(String, usize)>,
    deleted: bool,
    /// The entry, or `None` when it is skipped (`_create_entry` returns
    /// false).
    result: Result<Option<Entry>, Fatal>,
}

/// A recorded `Biber::Annotation->set_annotation` call.
struct AnnotationCall {
    scope: AnnotationScope,
    key: String,
    field: String,
    name: String,
    value: Option<String>,
    literal: bool,
}

/// One entry to create: its key, data (after source maps), the key that
/// goes into `rawdata` and its data source.
struct CreateJob {
    key: String,
    raw: RawEntry,
    tbkey: String,
    ds: String,
}

/// `_create_entry` (bibtex.pm:940) up to the section changes: builds the
/// entry and records everything else it does ([`Build`]).
fn build_entry(ctx: &Ctx, opts: &FetchOpts, secnum: &str, job: &CreateJob) -> Built {
    let mut b = Build {
        cfg: &*ctx.cfg,
        dm: ctx.dm,
        secnum,
        skip_rawdata: ctx.run.skip_rawdata,
        log: ctx.log.fork(),
        btlines: Vec::new(),
        names: NameArena::default(),
        options: Vec::new(),
        annotations: Vec::new(),
        np_lengths: Vec::new(),
        deleted: false,
    };
    let result = build_entry_in(&mut b, opts, job);
    Built {
        log: b.log,
        btlines: b.btlines,
        names: b.names,
        options: b.options,
        annotations: b.annotations,
        np_lengths: b.np_lengths,
        deleted: b.deleted,
        result,
    }
}

fn build_entry_in(
    b: &mut Build,
    opts: &FetchOpts,
    job: &CreateJob,
) -> Result<Option<Entry>, Fatal> {
    let (k, e, tbkey, ds) = (
        job.key.as_str(),
        &job.raw,
        job.tbkey.as_str(),
        job.ds.as_str(),
    );
    let mut be = Entry::default();
    be.set_field("citekey", Some(Value::Str(k.to_string())));
    debug!(b.log, "Creating biber Entry object with key '{k}'");
    if !b.skip_rawdata {
        be.set_field("rawdata", Some(Value::Str(print_s(e, tbkey))));
    }
    be.set_field("entrytype", Some(Value::Str(fc(&e.entrytype))));
    for (f, _) in &e.fields {
        let value = raw_get(e, f).unwrap_or("");
        let fcf = fc(f);
        if fcf == "options" {
            let list = opts.split_xsv(value);
            let set = entry_option_settings(&b.cfg.blx.registry, &list);
            b.options.extend(set);
        }
        if b.dm
            .is_field(&fcf, &opts.annotation_marker, &opts.named_annotation_marker)
        {
            if value.is_empty() {
                continue;
            }
            let r = run_handler(b, opts, &mut be, e, f, &fcf, value, k, ds)?;
            match r {
                HRes::None => {}
                HRes::Skip => return Ok(None),
                HRes::Set(v) => be.set_datafield(fcf, Some(v)),
            }
        } else if opts.validate_datamodel {
            warn_entry(
                &mut b.log,
                &mut be,
                format!(
                    "Datamodel: {} entry '{k}' ({ds}): Field '{f}' invalid in data model - ignoring",
                    e.entrytype
                ),
            );
        }
    }
    be.set_field("datatype", Some(Value::Str("bibtex".into())));
    Ok(Some(be))
}

/// Apply a [`build_entry`] in order: its messages, option settings,
/// annotations, name lists, name part lengths and the entry itself. Returns
/// `_create_entry`'s result (false: skipped).
fn apply_built(
    ctx: &mut Ctx,
    st: &mut SectionState,
    x: &mut Extract,
    key: &str,
    b: Built,
) -> Result<bool, Fatal> {
    let Built {
        log,
        btlines,
        names,
        options,
        annotations,
        np_lengths,
        deleted,
        result,
    } = b;
    ctx.log.join(log);
    x.btlines.extend(btlines);
    for (opt, v) in options {
        ctx.cfg.blx.set_entry(&st.section.number, key, &opt, v);
    }
    for a in annotations {
        ctx.run
            .annotations
            .set_annotation(a.scope, &a.key, &a.field, &a.name, a.value, a.literal);
    }
    let lists = st.section.names.append(names);
    for (np, len) in np_lengths {
        st.section.set_np_length(&np, len);
    }
    if deleted {
        st.section.del_citekey(key);
    }
    match result? {
        Some(mut be) => {
            for v in be
                .datafields
                .values_mut()
                .chain(be.derivedfields.values_mut())
            {
                if let Some(Value::Names(id)) = v {
                    *id = lists(*id);
                }
            }
            st.section.entries.insert(key.to_string(), be);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// `process_entry_options` (Utils.pm:1290).
pub(crate) fn process_entry_options(
    ctx: &mut Ctx,
    citekey: &str,
    options: &[String],
    secnum: &str,
) {
    for (opt, v) in entry_option_settings(&ctx.cfg.blx.registry, options) {
        ctx.cfg.blx.set_entry(secnum, citekey, &opt, v);
    }
}

/// The per-entry option values `process_entry_options` sets, in order.
fn entry_option_settings(
    reg: &crate::config::blx::Registry,
    options: &[String],
) -> Vec<(String, OptValue)> {
    let mut out = Vec::new();
    for o in options {
        // s/\s+=\s+/=/g
        let o = collapse_eq(o);
        // m/^([^=]+)=?(.+)?$/
        let (name, val) = match o.find('=') {
            Some(0) => continue,
            Some(p) => {
                let v = &o[p + 1..];
                (o[..p].to_string(), (!v.is_empty()).then(|| v.to_string()))
            }
            None if !o.is_empty() => (o.clone(), None),
            None => continue,
        };
        let val = val.unwrap_or_else(|| "1".into());
        let input = reg
            .decl(&lc(&name), Scope::Entry)
            .and_then(|d| d.input.clone());
        let oo = expand_option_input(reg, &name, Some(&val), input.as_ref());
        for (opt, v) in oo {
            let v = v.map_or(OptValue::Undef, OptValue::Str);
            out.push((opt, v));
        }
    }
    out
}

/// `s/\s+=\s+/=/g`.
pub(crate) fn collapse_eq(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut o = String::new();
    let mut i = 0;
    while i < c.len() {
        if is_perl_space(c[i]) {
            let mut j = i;
            while j < c.len() && is_perl_space(c[j]) {
                j += 1;
            }
            if j < c.len() && c[j] == '=' && j + 1 < c.len() && is_perl_space(c[j + 1]) {
                let mut k = j + 1;
                while k < c.len() && is_perl_space(c[k]) {
                    k += 1;
                }
                o.push('=');
                i = k;
                continue;
            }
            o.extend(&c[i..j]);
            i = j;
            continue;
        }
        o.push(c[i]);
        i += 1;
    }
    o
}

/// Does `field` end with an annotation marker (`_get_handler`'s
/// `m/$ann(?:$nam.+)?$/`)?
fn is_annotation_field(f: &str, ann: &str, nam: &str) -> bool {
    if ann.is_empty() {
        return true;
    }
    let f = f.strip_suffix('\n').unwrap_or(f);
    f.match_indices(ann).any(|(p, _)| {
        let rest = &f[p + ann.len()..];
        rest.is_empty()
            || (!nam.is_empty()
                && rest.starts_with(nam)
                && rest.len() > nam.len()
                && !rest[nam.len()..].contains('\n'))
    })
}

#[allow(clippy::too_many_arguments)]
fn run_handler(
    b: &mut Build,
    opts: &FetchOpts,
    be: &mut Entry,
    e: &RawEntry,
    field: &str,
    fcf: &str,
    value: &str,
    key: &str,
    ds: &str,
) -> Result<HRes, Fatal> {
    if is_annotation_field(fcf, &opts.annotation_marker, &opts.named_annotation_marker) {
        annotation(b, opts, fcf, value, key);
        return Ok(HRes::None);
    }
    // (`get_fieldtype`, `get_fieldformat`, `get_datatype` with one lookup)
    let def = b.dm.field_id(fcf).map(|id| b.dm.field(id));
    let ft = def.and_then(|d| d.fieldtype.as_deref()).unwrap_or("");
    let fmt = def
        .map(|d| d.format.as_str())
        .filter(|f| perl_compat::perl_truthy(f))
        .unwrap_or("default");
    let dt = def.and_then(|d| d.datatype.as_deref()).unwrap_or("");
    Ok(match (ft, fmt, dt) {
        ("field", "default", "code" | "datepart" | "entrykey" | "integer" | "key" | "literal") => {
            literal(b, opts, be, field, fcf, value, key)
        }
        ("field", "default", "date") => {
            datetime(&mut b.log, opts, be, e, field, value, key, ds);
            HRes::None
        }
        ("field", "default", "range") => range(b, opts, be, field, value, key),
        ("field", "default", "verbatim" | "uri") => {
            add_xdata_ref(
                &mut b.log,
                opts,
                be,
                field,
                XRef::Str(value),
                None,
                b.secnum,
            );
            HRes::Set(Value::Str(value.to_string()))
        }
        ("field", "xsv", "entrykey" | "literal" | "keyword" | "option") => {
            let v = opts.split_xsv(value);
            add_xdata_ref(&mut b.log, opts, be, field, XRef::List(&v), None, b.secnum);
            HRes::Set(Value::List(v))
        }
        ("list", "default", "key" | "literal" | "verbatim" | "uri") => {
            list(b, opts, be, field, value, dt != "uri")
        }
        ("list", "default", "name") => name_handler(b, opts, be, field, fcf, value, key),
        _ => {
            biber_error!(
                b.log,
                "No field handler for field '{fcf}' (fieldtype '{ft}', format '{fmt}', datatype '{dt}') in entry '{key}'"
            )?;
            HRes::None
        }
    })
}

/// A value passed to `add_xdata_ref`.
pub(super) enum XRef<'a> {
    Str(&'a str),
    List(&'a [String]),
}

/// `Biber::Entry::add_xdata_ref` (Entry.pm:225).
pub(super) fn add_xdata_ref(
    log: &mut Logger,
    opts: &FetchOpts,
    be: &mut Entry,
    reffield: &str,
    value: XRef,
    pos: Option<usize>,
    secnum: &str,
) -> bool {
    let push = |be: &mut Entry, o: J| {
        let refs = be
            .slots
            .entry("xdatarefs")
            .or_insert_with(|| J::Array(Vec::new()));
        if let J::Array(a) = refs {
            a.push(o);
        }
    };
    if reffield == "xdata" {
        let entries = match value {
            XRef::List(l) => J::Array(l.iter().map(|s| J::from(s.clone())).collect()),
            XRef::Str(s) => J::from(s),
        };
        let mut o = Map::new();
        o.insert("reffield".into(), J::from("xdata"));
        o.insert("refposition".into(), J::from("0"));
        o.insert("xdataentries".into(), entries);
        o.insert("xdatafield".into(), J::Null);
        o.insert("xdataposition".into(), J::from("0"));
        push(be, J::Object(o));
        return true;
    }
    let XRef::Str(v) = value else {
        return false;
    };
    // m/^$xdatamarker$xnamesep(\S+)$/xi
    let prefix = &opts.xdata_prefix;
    let v = v.strip_suffix('\n').unwrap_or(v);
    if v.len() < prefix.len() || !v.is_char_boundary(prefix.len()) {
        return false;
    }
    let (head, rest) = v.split_at(prefix.len());
    if !fc_eq(head, prefix) || rest.is_empty() || rest.chars().any(is_perl_space) {
        return false;
    }
    // m/^([^-]+)-([^-]+)(?:-(\d+))?$/
    let sep = &opts.xdatasep;
    let parts: Vec<&str> = if sep.is_empty() {
        vec![rest]
    } else {
        rest.split(sep.as_str()).collect()
    };
    let ok = |s: &str| !s.is_empty();
    let parsed = match parts.as_slice() {
        [a, b] if ok(a) && ok(b) => Some((*a, *b, None)),
        [a, b, c] if ok(a) && ok(b) && !c.is_empty() && c.chars().all(is_perl_digit) => {
            Some((*a, *b, Some(*c)))
        }
        _ => None,
    };
    let Some((xe, xf, xfp)) = parsed else {
        let ek = be.citekey().to_string();
        let bee = be.entrytype().to_string();
        warn_entry(
            log,
            be,
            format!(
                "{bee} entry '{ek}' has XDATA reference from field '{reffield}' that contains no source field (section {secnum})"
            ),
        );
        return false;
    };
    let mut o = Map::new();
    o.insert("reffield".into(), J::from(reffield));
    o.insert(
        "refposition".into(),
        J::from(pos.map_or(1, |p| p + 1).to_string()),
    );
    o.insert("xdataentries".into(), J::Array(vec![J::from(xe)]));
    o.insert("xdatafield".into(), J::from(xf));
    o.insert("xdataposition".into(), J::from(xfp.unwrap_or("*")));
    push(be, J::Object(o));
    true
}

/// `_annotation` (bibtex.pm:1007).
fn annotation(b: &mut Build, opts: &FetchOpts, field: &str, value: &str, key: &str) {
    let ann = &opts.annotation_marker;
    let nam = &opts.named_annotation_marker;
    let mut f = fc(field);
    let mut name = "default".to_string();
    // s/^(.+$ann)$nam(.+)$/$1/
    let pat = format!("{ann}{nam}");
    if !pat.is_empty()
        && let Some(p) = f
            .rmatch_indices(&pat)
            .map(|(p, _)| p)
            .find(|&p| p > 0 && f.len() > p + pat.len() && !f[p + pat.len()..].contains('\n'))
    {
        name = f[p + pat.len()..].to_string();
        f.truncate(p + ann.len());
    }
    if let Some(s) = f.strip_suffix(ann.as_str()) {
        f = s.to_string();
    }
    for a in split_semicolon(value) {
        let (count, part, mut anns) = parse_annotation_item(&a);
        let mut literal = false;
        if let Some(t) = &anns {
            let tt = t
                .trim_start_matches(is_perl_space)
                .trim_end_matches(is_perl_space);
            if tt.len() >= 3 && tt.starts_with('"') && tt.ends_with('"') {
                literal = true;
                anns = Some(tt[1..tt.len() - 1].to_string());
            }
        }
        let scope = if part.as_deref().is_some_and(perl_compat::perl_truthy) {
            AnnotationScope::Part {
                count: count.unwrap_or_default(),
                part: part.unwrap_or_default(),
            }
        } else if count.as_deref().is_some_and(perl_compat::perl_truthy) {
            AnnotationScope::Item {
                count: count.unwrap_or_default(),
            }
        } else {
            AnnotationScope::Field
        };
        b.annotations.push(AnnotationCall {
            scope,
            key: key.to_string(),
            field: f.clone(),
            name: name.clone(),
            value: anns,
            literal,
        });
    }
}

/// `split(/\s*;\s*/, $value)`.
fn split_semicolon(s: &str) -> Vec<String> {
    let mut out: Vec<String> = s
        .split(';')
        .map(|p| p.trim_matches(is_perl_space).to_string())
        .collect();
    // leading whitespace of the first field and trailing of the last are kept
    if let Some(first) = out.first_mut() {
        let orig = s.split(';').next().unwrap_or("");
        *first = orig.trim_end_matches(is_perl_space).to_string();
    }
    if out.len() > 1 {
        let orig = s.rsplit(';').next().unwrap_or("");
        let n = out.len();
        out[n - 1] = orig.trim_start_matches(is_perl_space).to_string();
    } else if let Some(only) = out.first_mut() {
        *only = s.to_string();
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// `/^\s*(\d+)?:?([^=]+)?=(.+)/`.
fn parse_annotation_item(a: &str) -> (Option<String>, Option<String>, Option<String>) {
    let s = a.trim_start_matches(is_perl_space);
    let digits_end = s
        .char_indices()
        .find(|(_, c)| !is_perl_digit(*c))
        .map_or(s.len(), |(i, _)| i);
    let count = (digits_end > 0).then(|| s[..digits_end].to_string());
    let mut rest = &s[digits_end..];
    if let Some(r) = rest.strip_prefix(':') {
        rest = r;
    }
    let Some(eq) = rest.find('=') else {
        return (None, None, None);
    };
    let part = &rest[..eq];
    let val = &rest[eq + 1..];
    let val = val.split('\n').next().unwrap_or("");
    if val.is_empty() {
        return (None, None, None);
    }
    let part = (!part.is_empty()).then(|| part.to_string());
    (count, part, Some(val.to_string()))
}

/// `_literal` (bibtex.pm:1042).
#[allow(clippy::too_many_arguments)]
fn literal(
    b: &mut Build,
    opts: &FetchOpts,
    be: &mut Entry,
    field: &str,
    fcf: &str,
    value: &str,
    key: &str,
) -> HRes {
    if add_xdata_ref(
        &mut b.log,
        opts,
        be,
        field,
        XRef::Str(value),
        None,
        b.secnum,
    ) {
        return HRes::Set(Value::Str(value.to_string()));
    }
    let dtruthy = |be: &Entry, f: &str| truthy(be.get_datafield(f).and_then(Value::as_str));
    if fcf == "year" {
        if dtruthy(be, "year") {
            return HRes::None;
        }
        if perl_compat::perl_truthy(value) && perl_compat::unicode::ucd_num(value).is_none() {
            b.log.biber_warn(format!(
                "legacy year field '{value}' in entry '{key}' is not an integer - this will probably not sort properly."
            ));
        }
    }
    if fcf == "month" {
        if dtruthy(be, "month") {
            return HRes::None;
        }
        if perl_compat::perl_truthy(value) && perl_compat::unicode::ucd_num(value).is_none() {
            b.log.biber_warn(format!(
                "legacy month field '{value}' in entry '{key}' is not an integer - this will probably not sort properly."
            ));
        }
    }
    let mut value = value.to_string();
    if fcf == "isbn" {
        let Some(isbn) = perl_compat::isbn::Isbn::new(&value).filter(|i| i.is_valid()) else {
            b.log.biber_warn(format!(
                "ISBN '{value}' in entry '{key}' is invalid - run biber with '--validate_datamodel' for details."
            ));
            return HRes::Set(Value::Str(value));
        };
        let mut isbn = isbn;
        if opts.isbn13 {
            if let Some(i) = isbn.as_isbn13() {
                isbn = i;
                value = isbn.isbn().to_string();
            }
        } else if opts.isbn10 {
            // DC-035: a 979 ISBN has no ISBN-10 form (biber crashes)
            if let Some(i) = isbn.as_isbn10() {
                isbn = i;
                value = isbn.isbn().to_string();
            }
        }
        if opts.isbn_normalise
            && let Some(s) = isbn.as_string()
        {
            value = s;
        }
    }
    if fcf == "month" {
        return match hack_month(&value) {
            Some(v) => HRes::Set(Value::Str(v)),
            None => HRes::None,
        };
    }
    if fcf == "langid"
        && let Some(m) = crate::config::locale::locale_map_r(&value)
    {
        return HRes::Set(Value::Str(m.to_string()));
    }
    HRes::Set(Value::Str(value))
}

/// `_hack_month` (bibtex.pm:1983).
fn hack_month(m: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let t = m.trim_start_matches(is_perl_space);
    let first_line = t.split('\n').next().unwrap_or("");
    let matched = first_line.len() >= 3
        && first_line.is_char_boundary(3)
        && MONTHS.contains(&first_line[..3].to_ascii_lowercase().as_str());
    if !matched || (t.contains('\n') && !t.trim_end_matches(is_perl_space).eq(first_line)) {
        return Some(m.to_string());
    }
    let three = lc(perl_compat::grapheme::gcstring::substr(t, 0, Some(3)));
    MONTHS
        .iter()
        .position(|x| *x == three)
        .map(|i| (i + 1).to_string())
}

/// `_range` (bibtex.pm:1152).
fn range(
    b: &mut Build,
    opts: &FetchOpts,
    be: &mut Entry,
    field: &str,
    value: &str,
    key: &str,
) -> HRes {
    if add_xdata_ref(
        &mut b.log,
        opts,
        be,
        field,
        XRef::Str(value),
        None,
        b.secnum,
    ) {
        return HRes::Set(Value::Str(value.to_string()));
    }
    static RES: std::sync::OnceLock<[Arc<PerlRegex>; 5]> = std::sync::OnceLock::new();
    let res = RES.get_or_init(|| {
        let c = |p: &str| {
            PerlRegex::new(p, false)
                .map(Arc::new)
                .expect("valid range regex")
        };
        [
            c(r"\s*[;,]\s*"),
            c(r"(?xms)\A\s*(\P{Pd}+)\s*\z"),
            c(r"(?xms)\A\s*(\{[^\}]+\}|[^\p{Pd} ]+)\s*(\p{Pd}+)\s*(\{[^\}]+\}|\P{Pd}*)\s*\z"),
            c(r"(?xms)\A\s*(.+)(\p{Pd}{2,})(.+)\s*\z"),
            c(r"(?xms)\A\s*(.+)(\p{Pd}+)(.+)\s*\z"),
        ]
    });
    let values: Vec<String> = res[0]
        .split(value)
        .map(|v| v.into_iter().map(Option::unwrap_or_default).collect())
        .unwrap_or_default();
    let mut out: Vec<Vec<Option<String>>> = Vec::new();
    for ovalue in values {
        let v = ovalue.replace('~', " ");
        let mut caps: Option<Vec<Option<String>>> = None;
        for re in &res[1..] {
            if let Ok(ms) = re.matches(&v)
                && let Some(g) = ms.into_iter().next()
            {
                caps = Some(
                    g.iter()
                        .map(|x| x.map(|(a, b)| v[a..b].to_string()))
                        .collect(),
                );
                break;
            }
        }
        let cap = |i: usize| caps.as_ref().and_then(|c| c.get(i).cloned().flatten());
        let start = cap(1);
        let end = if truthy(cap(2).as_deref()) {
            cap(3)
        } else {
            None
        };
        let unbrace = |s: String| -> String {
            if s.len() >= 3
                && s.starts_with('{')
                && s.ends_with('}')
                && !s[1..s.len() - 1].contains('}')
            {
                s[1..s.len() - 1].to_string()
            } else {
                s
            }
        };
        let start = start.map(unbrace);
        let end = end.map(|e| if truthy(Some(&e)) { unbrace(e) } else { e });
        if truthy(start.as_deref()) {
            out.push(vec![start, end]);
        } else {
            warn_entry(
                &mut b.log,
                be,
                format!(
                    "Range field '{field}' in entry '{key}' is malformed, falling back to literal"
                ),
            );
            out.push(vec![Some(ovalue), None]);
        }
    }
    if out.is_empty() {
        return HRes::None;
    }
    HRes::Set(Value::Ranges(out))
}

/// `_list` / `_urilist` (bibtex.pm:1446, 1474).
#[allow(clippy::too_many_arguments)]
fn list(
    b: &mut Build,
    opts: &FetchOpts,
    be: &mut Entry,
    field: &str,
    value: &str,
    strip_outer: bool,
) -> HRes {
    let v = nfc(value);
    let sl = btparse::split_list(&v, &opts.listsep);
    record_split_diags(&mut b.btlines, &sl);
    let mut out = Vec::new();
    for (i, item) in sl.items.iter().enumerate() {
        let item = nfd(item);
        let item = if strip_outer {
            remove_outer(&item).1
        } else {
            item
        };
        add_xdata_ref(
            &mut b.log,
            opts,
            be,
            field,
            XRef::Str(&item),
            Some(i),
            b.secnum,
        );
        out.push(item);
    }
    HRes::Set(Value::List(out))
}

fn record_split_diags(btlines: &mut Vec<String>, sl: &btparse::SplitList) {
    for d in &sl.diags {
        btlines.push(d.render(""));
    }
    if sl.crash && !sl.diags.iter().any(|d| d.render("").contains("error")) {
        btlines.push("error: libbtparse would abort on this list".into());
    }
}

/// `m/^(\S+)\s*$xnamesep\s*(\S+)?$/` on a name-list item.
fn namelist_option<'a>(s: &'a str, sep: &str) -> Option<(&'a str, Option<&'a str>)> {
    if sep.is_empty() {
        return None;
    }
    let s = s.strip_suffix('\n').unwrap_or(s);
    let mut search_end = s.len();
    while let Some(p) = s[..search_end].rfind(sep) {
        let left = s[..p].trim_end_matches(is_perl_space);
        let right = s[p + sep.len()..].trim_start_matches(is_perl_space);
        if !left.is_empty() && !left.chars().any(is_perl_space) && !right.chars().any(is_perl_space)
        {
            return Some((left, (!right.is_empty()).then_some(right)));
        }
        if p == 0 {
            break;
        }
        search_end = p;
    }
    None
}

/// `_name` (bibtex.pm:1201).
#[allow(clippy::too_many_arguments)]
fn name_handler(
    b: &mut Build,
    opts: &FetchOpts,
    be: &mut Entry,
    field: &str,
    fcf: &str,
    value: &str,
    key: &str,
) -> HRes {
    let secnum = b.secnum;
    let v = nfc(value);
    let sl = btparse::split_list(&v, &opts.namesep);
    record_split_diags(&mut b.btlines, &sl);
    let items: Vec<String> = sl.items.iter().map(|s| nfd(s)).collect();
    let listid = b.names.new_list();
    b.names.list_mut(listid).type_ = Some(fcf.to_string());
    let mut names: Vec<crate::model::NameId> = Vec::new();
    let nps = &opts.nameparts;
    for (i, name) in items.iter().enumerate() {
        if add_xdata_ref(
            &mut b.log,
            opts,
            be,
            field,
            XRef::Str(name),
            Some(i),
            secnum,
        ) {
            let mut n = Name::default();
            n.slots.insert("xdata", J::from(name.clone()));
            n.index = Some((names.len() + 1).to_string().into());
            names.push(b.names.new_name(n));
            continue;
        }
        if let Some((o, ov)) = namelist_option(name, &opts.xnamesep) {
            let nlo = lc(o);
            let nlov = ov.unwrap_or("1");
            let reg = &b.cfg.blx.registry;
            if reg.has_scope(&nlo, Scope::NameList) {
                let input = reg
                    .decl(&nlo, Scope::NameList)
                    .and_then(|d| d.input.clone());
                for (opt, val) in expand_option_input(reg, &nlo, Some(nlov), input.as_ref()) {
                    b.names
                        .list_mut(listid)
                        .slots
                        .insert(opt, val.map_or(J::Null, J::from));
                }
                continue;
            }
        }
        if !perl_compat::perl_truthy(name) {
            warn_entry(
                &mut b.log,
                be,
                format!(
                    "Name in key '{key}' is empty (probably consecutive 'and'): skipping entry '{key}'"
                ),
            );
            b.deleted = true;
            return HRes::Skip;
        }
        let cfg = NameCfg {
            noinit: &opts.noinit,
            fixinits: opts.fixinits,
            nameparts: nps,
            xnamesep: &opts.xnamesep,
            registry: &b.cfg.blx.registry,
        };
        let is_x = !opts.noxname
            && nps.iter().any(|np| {
                name.match_indices(np.as_str()).any(|(p, _)| {
                    name[p + np.len()..]
                        .trim_start_matches(is_perl_space)
                        .starts_with(opts.xnamesep.as_str())
                })
            });
        let parsed = if is_x {
            names::parsename_x(&cfg, name, fcf, key)
        } else {
            let escaped = name.starts_with('{') && name.ends_with('}') && name.len() > 2;
            if !escaped {
                if name.matches(',').count() > 2 {
                    b.log.biber_error_nodie(
                        crate::loc!(),
                        &format!("Name \"{name}\" has too many commas, skipping entry '{key}'"),
                    );
                    b.deleted = true;
                    return HRes::Skip;
                }
                if name.contains(",,") {
                    b.log.biber_error_nodie(
                        crate::loc!(),
                        &format!(
                            "Name \"{name}\" is malformed (consecutive commas): skipping entry '{key}'"
                        ),
                    );
                    b.deleted = true;
                    return HRes::Skip;
                }
            }
            names::parsename(&cfg, name)
        };
        b.btlines.extend(parsed.btlines.iter().cloned());
        if parsed.crash {
            b.btlines.push(format!(
                "error: libbtparse would crash on the name \"{name}\" in entry '{key}'"
            ));
        }
        for w in parsed.warnings {
            b.log.biber_warn(w);
        }
        for (np, len) in &parsed.lengths {
            b.np_lengths.push((np.clone(), *len));
        }
        let n = parsed.name;
        if lc(n.rawstring.as_deref().unwrap_or("")) == opts.others_string {
            b.names
                .list_mut(listid)
                .slots
                .insert("morenames", J::from("1"));
        } else {
            let mut n = n;
            n.index = Some((names.len() + 1).to_string().into());
            names.push(b.names.new_name(n));
        }
    }
    if names.is_empty() {
        return HRes::None;
    }
    b.names.list_mut(listid).names = names;
    HRes::Set(Value::Names(listid))
}

/// `_datetime` (bibtex.pm:1302).
#[allow(clippy::too_many_arguments)]
fn datetime(
    log: &mut Logger,
    opts: &FetchOpts,
    be: &mut Entry,
    e: &RawEntry,
    field: &str,
    date: &str,
    key: &str,
    ds: &str,
) {
    let dtype = field.strip_suffix("date").unwrap_or(field).to_string();
    let bee = be.entrytype().to_string();
    let mut sp = DateParser::default();
    let mut ep = DateParser::default();
    let r = dates::parse_date_range(&mut sp, &mut ep, date, opts.julian);
    let s1 = |v: &str| Some(Value::Str(v.to_string()));
    let Some(r) = r else {
        warn_entry(
            log,
            be,
            format!(
                "{bee} entry '{key}' ({ds}): Invalid format '{date}' of date field '{field}' - ignoring"
            ),
        );
        return;
    };
    if r.start_unknown {
        be.set_field(format!("{dtype}dateunknown"), s1("1"));
    }
    if r.end_unknown {
        be.set_field(format!("{dtype}enddateunknown"), s1("1"));
    }
    if let Some(u) = r.unspec {
        be.set_field(format!("{dtype}dateunspecified"), s1(u));
    }
    if !r.start.is_defined() {
        warn_entry(
            log,
            be,
            format!(
                "{bee} entry '{key}' ({ds}): Invalid format '{date}' of date field '{field}' - ignoring"
            ),
        );
        return;
    }
    match &r.start {
        Parsed::Date(sd) => {
            be.set_field(format!("{dtype}datesplit"), s1("1"));
            if sd.year != 0
                && dtype.is_empty()
                && let Some(y) = raw_get(e, "year").filter(|y| perl_compat::perl_truthy(y))
                && (sd.year as f64) != perl_compat::truth::numify(y)
            {
                warn_entry(
                    log,
                    be,
                    format!(
                        "Overwriting field 'year' with year value from field 'date' for entry '{key}'"
                    ),
                );
            }
            if !sp.missing("month")
                && dtype.is_empty()
                && let Some(m) = raw_get(e, "month").filter(|m| perl_compat::perl_truthy(m))
                && f64::from(sd.month) != perl_compat::truth::numify(m)
            {
                warn_entry(
                    log,
                    be,
                    format!(
                        "Overwriting field 'month' with month value from field 'date' for entry '{key}'"
                    ),
                );
            }
            if sp.julian {
                be.set_field(format!("{dtype}datejulian"), s1("1"));
            }
            if ep.julian {
                be.set_field(format!("{dtype}enddatejulian"), s1("1"));
            }
            if sp.approximate {
                be.set_field(format!("{dtype}dateapproximate"), s1("1"));
            }
            if ep.approximate {
                be.set_field(format!("{dtype}enddateapproximate"), s1("1"));
            }
            if sp.uncertain {
                be.set_field(format!("{dtype}dateuncertain"), s1("1"));
            }
            if ep.uncertain {
                be.set_field(format!("{dtype}enddateuncertain"), s1("1"));
            }
            if let Some(yd) = sp.yeardivision {
                be.set_field(format!("{dtype}yeardivision"), s1(yd));
            }
            if !sp.missing("year") {
                be.set_datafield(
                    format!("{dtype}year"),
                    s1(&sp.resolvescript(&sd.year.to_string())),
                );
                be.set_field(format!("{dtype}era"), s1(sd.era()));
            }
            if !sp.missing("month") {
                be.set_datafield(
                    format!("{dtype}month"),
                    s1(&sp.resolvescript(&sd.month.to_string())),
                );
            }
            if !sp.missing("day") {
                be.set_datafield(
                    format!("{dtype}day"),
                    s1(&sp.resolvescript(&sd.day.to_string())),
                );
            }
            if !sp.missing("time") {
                be.set_datafield(
                    format!("{dtype}hour"),
                    s1(&sp.resolvescript(&sd.hour.to_string())),
                );
                be.set_datafield(
                    format!("{dtype}minute"),
                    s1(&sp.resolvescript(&sd.minute.to_string())),
                );
                be.set_datafield(
                    format!("{dtype}second"),
                    s1(&sp.resolvescript(&sd.second.to_string())),
                );
                if let Some(tz) = &sd.tz {
                    be.set_datafield(format!("{dtype}timezone"), s1(&dates::tzformat(tz)));
                }
            }
        }
        _ => be.set_datafield(format!("{dtype}year"), s1("")),
    }
    if r.sep {
        match &r.end {
            Parsed::Date(ed) => {
                be.set_field(format!("{dtype}datesplit"), s1("1"));
                if !ep.missing("year") {
                    be.set_datafield(
                        format!("{dtype}endyear"),
                        s1(&ep.resolvescript(&ed.year.to_string())),
                    );
                    be.set_field(format!("{dtype}endera"), s1(ed.era()));
                }
                if !ep.missing("month") {
                    be.set_datafield(
                        format!("{dtype}endmonth"),
                        s1(&ep.resolvescript(&ed.month.to_string())),
                    );
                }
                if !ep.missing("day") {
                    be.set_datafield(
                        format!("{dtype}endday"),
                        s1(&ep.resolvescript(&ed.day.to_string())),
                    );
                }
                if let Some(yd) = ep.yeardivision {
                    be.set_field(format!("{dtype}endyeardivision"), s1(yd));
                    be.set_field(format!("{dtype}endseaason"), s1(yd));
                }
                if !ep.missing("time") {
                    be.set_datafield(
                        format!("{dtype}endhour"),
                        s1(&ep.resolvescript(&ed.hour.to_string())),
                    );
                    be.set_datafield(
                        format!("{dtype}endminute"),
                        s1(&ep.resolvescript(&ed.minute.to_string())),
                    );
                    be.set_datafield(
                        format!("{dtype}endsecond"),
                        s1(&ep.resolvescript(&ed.second.to_string())),
                    );
                    if let Some(tz) = &ed.tz {
                        be.set_datafield(format!("{dtype}endtimezone"), s1(&dates::tzformat(tz)));
                    }
                }
            }
            Parsed::Zero => be.set_datafield(format!("{dtype}endyear"), s1("")),
            Parsed::Undef => warn_entry(
                log,
                be,
                format!(
                    "{bee} entry '{key}' ({ds}): Invalid format '{date}' of end date field '{field}' - ignoring"
                ),
            ),
        }
    }
}

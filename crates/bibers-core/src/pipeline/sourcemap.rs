//! Source maps (`\DeclareSourcemap`, `\DeclareStyleSourcemap`, driver maps,
//! biber.conf `<sourcemap>`): the map engine of
//! `Biber::Input::file::bibtex::create_entry` (bibtex.pm:356-937, vendor/biber
//! @ v2.22), applied to a data source entry before field handlers run. Used
//! by [`super::fetch`].
//!
//! # Interface contract with `fetch`
//!
//! `fetch` turns each cached data source entry into a [`RawEntry`] exactly
//! as `Text::BibTeX` exposes it to biber after biber's **second** parse pass
//! (bibtex.pm:1505-1640), i.e. a fresh copy for every `create_entry` call
//! (biber clones the cached entry, bibtex.pm:370-371):
//!
//! * [`RawEntry::entrytype`]: `$entry->type` — the type as btparse reports
//!   it: **lowercased** by btparse (ASCII letters only: `@ARTICLE` →
//!   `article`, a non-ASCII `É` is kept), NFD;
//! * [`RawEntry::key`]: the citation key `create_entry` is called with (the
//!   cache key; NFD, case preserved);
//! * [`RawEntry::fields`]: `$entry->fieldlist` in `.bib` order with
//!   `$entry->get($f)` values. Field names are **lowercased** by btparse
//!   (ASCII only, like the type). Values are the pass-2 strings: macros
//!   expanded, `#` concatenated, whitespace collapsed, outer delimiters
//!   removed, `latex_decode`d in pass 1, NFD (the `.bib` text is NFD'd before
//!   parsing). Empty values (`title = {}`) are present with value `""`.
//!   Duplicate field names cannot occur (btparse keeps one).
//!
//! The caller then:
//! 1. records `set_keytods(key, filename)` (as biber does before
//!    `create_entry`) and calls [`apply_maps`] (or [`apply_maps_in`], which
//!    takes the data source name explicitly — needed for citekey-alias
//!    targets, where biber passes the *current* file name);
//! 2. removes every key in [`MapOutcome::created_keys`] from its list of
//!    still-wanted keys (biber does this when the entry is created, even if
//!    the original entry is then dropped);
//! 3. unless [`MapOutcome::skip`], runs `_create_entry` (field handlers) on
//!    the mapped entry, then on each of [`MapOutcome::new_entries`] (biber
//!    iterates `%newentries` in hash order; we give creation order, DC-016).
//!    A new entry's [`RawEntry::key`] is the citekey biber passes to
//!    `_create_entry`. `fieldlist` order and NFD values are maintained by
//!    this module, so iterate [`RawEntry::fields`] (or
//!    [`RawEntry::fieldlist`]) directly.
//!
//! This module itself updates the section for `entry_new`/`entry_clone`
//! steps (`add_nocite`, `add_citekeys`), queues `biber_warn`s, and logs the
//! debug messages verbatim.
//!
//! # Determinism
//!
//! `$MAPUNIQ` is generated from the wall clock in biber (BB-113). bibers
//! uses a per-run counter rendered like `Data::Uniqid::suniqid`'s base-62
//! output ([`mapuniq_id`], DC-018).

use std::cmp::Ordering;
use std::fmt;

use super::Ctx;
use crate::bcf::sourcemap::{MapStep, SourceMap, SourceMaps};
use crate::config::Config;
use crate::debug;
use crate::log::{Fatal, Logger};
use crate::model::{Section, SectionState};
use crate::quirks::Quirk;
use perl_compat::case::fc_eq;
use perl_compat::regex::{CaptureDigits, expand_imatches, imatch, ireplace, split_default_xsvsep};
use perl_compat::truth::{num_cmp_str, perl_truthy};
use perl_compat::unicode::{nfc, nfd};
use perl_compat::{fc, truth::perl_truthy_opt};

/// A data source entry during source mapping (biber's
/// `Text::BibTeX::Entry`). See the module docs for the exact contract.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawEntry {
    /// Entry type as `$entry->type` returns it (btparse-lowercased, NFD).
    pub entrytype: String,
    /// The citation key.
    pub key: String,
    /// Fields in `fieldlist` order: (name as stored, value as `get` returns
    /// it, NFD).
    pub fields: Vec<(String, String)>,
}

impl RawEntry {
    /// `$entry->get($name)`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// `$entry->exists($name)`.
    pub fn exists(&self, name: &str) -> bool {
        self.fields.iter().any(|(n, _)| n == name)
    }

    /// `$entry->set($name, encode('UTF-8', NFC($value)))` followed by reads
    /// through `get` (which NFD-normalises): the value is stored in NFD. A
    /// new field is appended to the field list; an existing one keeps its
    /// position.
    pub fn set(&mut self, name: &str, value: &str) {
        let v = nfd(value).into_owned();
        match self.fields.iter_mut().find(|(n, _)| n == name) {
            Some((_, old)) => *old = v,
            None => self.fields.push((name.to_string(), v)),
        }
    }

    /// `$entry->delete($name)`.
    pub fn delete(&mut self, name: &str) {
        self.fields.retain(|(n, _)| n != name);
    }

    /// `$entry->fieldlist`: field names, NFD-normalised as Text::BibTeX
    /// returns them (names set by maps are stored NFC, like biber does).
    pub fn fieldlist(&self) -> impl Iterator<Item = String> + '_ {
        self.fields.iter().map(|(n, _)| nfd(n).into_owned())
    }

    /// `$entry->set_type(encode('UTF-8', NFC($t)))` read back via `type`.
    fn set_type(&mut self, t: &str) {
        self.entrytype = nfd(t).into_owned();
    }
}

/// What mapping did to an entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapOutcome {
    /// The entry was deleted (`entry_null`) and must not be created (nor
    /// any of the entries created before the deletion).
    pub skip: bool,
    /// Entries created by `entry_new`/`entry_clone` steps, in creation order
    /// (a key created twice keeps its first position with the later entry).
    /// Empty when `skip`.
    pub new_entries: Vec<RawEntry>,
    /// Every key created by `entry_new`/`entry_clone` (in order, possibly
    /// repeated), also when `skip`: the caller removes them from its wanted
    /// keys (bibtex.pm:467, 499).
    pub created_keys: Vec<String>,
    /// For each of [`MapOutcome::new_entries`], the key the Text::BibTeX
    /// entry itself carries (`$entry->key`): the new key for `entry_new`,
    /// the *original* entry's key for `entry_clone` (biber's `rawdata`
    /// field is `print_s` of this entry). Added by the fetch package.
    pub tb_keys: Vec<String>,
}

/// Run-global source map state: biber's package global
/// `$Biber::Utils::MAPUNIQVAL` and the deterministic `$MAPUNIQ` counter
/// (DC-018). Lives in [`Config::sourcemap_run`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapRunState {
    /// Number of `$MAPUNIQ` values generated so far in this run.
    pub uniq_counter: u64,
    /// `$MAPUNIQVAL`: the last `$MAPUNIQ` value (`undef` initially).
    pub mapuniqval: Option<String>,
}

/// The `n`th (1-based) `$MAPUNIQ` value of a run (DC-018): `base62(10^16 +
/// n)` in `Data::Uniqid`'s alphabet (`0-9a-zA-Z`), i.e. nine characters
/// like biber's time-based `suniqid` values, but reproducible.
pub fn mapuniq_id(n: u64) -> String {
    const ALPHA: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut v = 10_000_000_000_000_000u64 + n;
    let mut out = Vec::new();
    while v > 0 {
        out.push(ALPHA[(v % 62) as usize]);
        v /= 62;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII")
}

/// `Biber::Utils::maploopreplace` (Utils.pm:1773-1786): substitute
/// `$MAPLOOP`, `$MAPUNIQVAL` and `$MAPUNIQ`, but only inside a `map_foreach`
/// loop with a (perl-)true loop value (BB-034).
///
/// With the quirk fixed (`bb034_fixed`), `$MAPUNIQ`/`$MAPUNIQVAL` are also
/// substituted outside loops (`$MAPLOOP` stays literal there).
pub(crate) fn maploopreplace(
    run: &mut MapRunState,
    s: Option<&str>,
    maploop: &str,
    bb034_fixed: bool,
) -> Option<String> {
    let s = s?;
    let in_loop = perl_truthy(maploop);
    if !in_loop && !bb034_fixed {
        return Some(s.to_string());
    }
    let mut s = if in_loop {
        s.replace("$MAPLOOP", maploop)
    } else {
        s.to_string()
    };
    s = s.replace("$MAPUNIQVAL", run.mapuniqval.as_deref().unwrap_or(""));
    if s.contains("$MAPUNIQ") {
        run.uniq_counter += 1;
        let u = mapuniq_id(run.uniq_counter);
        s = s.replace("$MAPUNIQ", &u);
        run.mapuniqval = Some(u);
    }
    Some(s)
}

/// `Biber::Utils::appendstrict_check` (Utils.pm:1917).
pub(crate) fn appendstrict_check(step: &MapStep, orig: &str, val: &str) -> String {
    if perl_truthy_opt(step.get("map_appendstrict")) {
        if perl_truthy(orig) {
            format!("{orig}{val}")
        } else {
            String::new()
        }
    } else {
        format!("{orig}{val}")
    }
}

/// The source maps applied to a data source of `datatype`, in order: all
/// user-level, all style-level, then the **first** driver-level `<maps>`
/// (bibtex.pm:136-152).
pub fn select_maps<'a>(cfg: &'a Config, datatype: &str) -> Vec<&'a SourceMaps> {
    let all = cfg.biber.sourcemap();
    let is = |m: &SourceMaps, level: &str| {
        m.datatype.as_deref().unwrap_or("") == datatype && m.level.as_deref().unwrap_or("") == level
    };
    let mut out: Vec<&SourceMaps> = all.iter().filter(|m| is(m, "user")).collect();
    out.extend(all.iter().filter(|m| is(m, "style")));
    out.extend(all.iter().find(|m| is(m, "driver")));
    out
}

/// Apply all applicable source maps to `entry` (bibtex.pm:365-920). The data
/// source name is taken from the section's `keytods` record for the entry's
/// key (which `fetch` sets before calling, as biber does); use
/// [`apply_maps_in`] to pass it explicitly.
pub fn apply_maps(
    ctx: &mut Ctx,
    st: &mut SectionState,
    datatype: &str,
    entry: &mut RawEntry,
) -> Result<MapOutcome, Fatal> {
    let ds = st
        .section
        .slots
        .get("keytods")
        .and_then(|o| o.get(&entry.key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    apply_maps_in(ctx, st, datatype, &ds, entry)
}

/// [`apply_maps`] with the data source name (`$datasource` of
/// `create_entry`: the located file name) given explicitly.
pub fn apply_maps_in(
    ctx: &mut Ctx,
    st: &mut SectionState,
    datatype: &str,
    datasource: &str,
    entry: &mut RawEntry,
) -> Result<MapOutcome, Fatal> {
    apply_maps_logged(ctx.cfg, ctx.log, st, datatype, datasource, entry)
}

/// [`apply_maps_in`] with its messages going to `log`.
pub fn apply_maps_logged(
    cfg: &mut Config,
    log: &mut Logger,
    st: &mut SectionState,
    datatype: &str,
    datasource: &str,
    entry: &mut RawEntry,
) -> Result<MapOutcome, Fatal> {
    let mut run = std::mem::take(&mut cfg.sourcemap_run);
    let res = {
        let mut eng = Engine {
            cfg,
            log,
            run: &mut run,
            sec: &mut st.section,
        };
        eng.apply(datatype, datasource, entry)
    };
    cfg.sourcemap_run = run;
    res
}

/// The `"Source mapping (type=$level, key=$key): "` prefix of biber's
/// messages, formatted only when a message is actually emitted.
struct MapPrefix<'a> {
    level: &'a str,
    key: &'a str,
}

impl fmt::Display for MapPrefix<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Source mapping (type={}, key={}): ",
            self.level, self.key
        )
    }
}

/// A created entry: the citekey it is created under and the
/// Text::BibTeX entry (whose own `key` is the original's for clones).
struct NewEntry {
    citekey: String,
    entry: RawEntry,
}

#[derive(Clone, Copy)]
enum Target {
    Main,
    New(usize),
}

struct Engine<'a> {
    cfg: &'a Config,
    log: &'a mut Logger,
    run: &'a mut MapRunState,
    sec: &'a mut Section,
}

/// `$section->add_citekeys($key)` (Section.pm:707).
fn add_citekey(sec: &mut Section, key: &str) {
    if sec.citekeys_h.contains(key) {
        return;
    }
    sec.citekeys_h.insert(key.to_string());
    sec.citekeys.push(key.to_string());
    sec.orig_order_citekeys.push(key.to_string());
}

fn target<'e>(main: &'e mut RawEntry, news: &'e mut [NewEntry], t: Target) -> &'e mut RawEntry {
    match t {
        Target::Main => main,
        Target::New(i) => &mut news[i].entry,
    }
}

/// `$smap->{map_overwrite}` defaulted to 0, overridden by the map's
/// (`$map->{map_overwrite} // $smap->{map_overwrite}`).
fn overwrite(smap: &SourceMaps, map: &SourceMap) -> bool {
    let v = map
        .map_overwrite
        .as_deref()
        .or(smap.map_overwrite.as_deref())
        .unwrap_or("0");
    perl_truthy(v)
}

/// What to do after a failed condition: skip the step, or (`map_final`)
/// the rest of the map (`next MAP`).
enum Flow {
    NextStep,
    NextMap,
}

impl Engine<'_> {
    fn regex_error(
        &mut self,
        level: &str,
        key: &str,
        e: &perl_compat::regex::RegexError,
    ) -> Result<(), Fatal> {
        // perl would die compiling the regex; unsupported constructs are DC-036.
        self.log.biber_error(
            crate::loc!(),
            &format!("Source mapping (type={level}, key={key}): {e}"),
        )
    }

    fn apply(
        &mut self,
        datatype: &str,
        datasource: &str,
        entry: &mut RawEntry,
    ) -> Result<MapOutcome, Fatal> {
        let cfg = self.cfg;
        let key = entry.key.clone();
        let secnum = self.sec.number.clone();
        let mut news: Vec<NewEntry> = Vec::new();
        let mut created_keys: Vec<String> = Vec::new();

        // kpsewhich returns absolute paths: compare the file name only.
        let test_path: &str = if is_absolute(datasource) {
            datasource.rsplit(['/', '\\']).next().unwrap_or(datasource)
        } else {
            datasource
        };

        for smap in select_maps(cfg, datatype) {
            let level = smap.level.as_deref().unwrap_or("");
            for map in &smap.maps {
                if let Some(rs) = &map.refsection {
                    if num_cmp_str(&secnum, rs) != Some(Ordering::Equal) {
                        continue;
                    }
                }
                if !map.per_type.is_empty()
                    && !map
                        .per_type
                        .iter()
                        .any(|t| fc_eq(t.as_deref().unwrap_or(""), &entry.entrytype))
                {
                    continue;
                }
                if map
                    .per_nottype
                    .iter()
                    .any(|t| fc_eq(t.as_deref().unwrap_or(""), &entry.entrytype))
                {
                    continue;
                }
                if !map.per_datasource.is_empty()
                    && !map
                        .per_datasource
                        .iter()
                        .any(|d| d.as_deref().unwrap_or("") == test_path)
                {
                    continue;
                }

                let mut last_type = entry.entrytype.clone();
                let mut last_field: Option<String> = None;
                let mut last_fieldval: Option<String> = None;
                let mut imatches: Vec<Option<String>> = Vec::new();

                let mut maploop: Vec<String> = vec![String::new()];
                if let Some(foreach) = map.map_foreach.as_deref().filter(|f| perl_truthy(f)) {
                    let ffc = fc(foreach);
                    if let Some(ds) = cfg.datafield_set(&ffc) {
                        maploop = ds.to_vec();
                    } else if let Some(fv) = entry.get(&nfc(&ffc)).filter(|v| perl_truthy(v)) {
                        maploop = split_default_xsvsep(fv);
                    } else {
                        maploop = split_default_xsvsep(foreach);
                    }
                }

                'map: for ml in &maploop {
                    'step: for step in &map.steps {
                        match self.step(
                            level,
                            &key,
                            smap,
                            map,
                            step,
                            ml,
                            entry,
                            &mut news,
                            &mut created_keys,
                            &mut last_type,
                            &mut last_field,
                            &mut last_fieldval,
                            &mut imatches,
                        )? {
                            StepResult::Continue | StepResult::Flow(Flow::NextStep) => {
                                continue 'step;
                            }
                            StepResult::Flow(Flow::NextMap) => continue 'map,
                            StepResult::Null => {
                                return Ok(MapOutcome {
                                    skip: true,
                                    new_entries: Vec::new(),
                                    created_keys,
                                    tb_keys: Vec::new(),
                                });
                            }
                        }
                    }
                }
            }
        }

        let tb_keys = news.iter().map(|n| n.entry.key.clone()).collect();
        Ok(MapOutcome {
            skip: false,
            tb_keys,
            new_entries: news
                .into_iter()
                .map(|n| RawEntry {
                    key: n.citekey,
                    ..n.entry
                })
                .collect(),
            created_keys,
        })
    }

    fn mlr(&mut self, step: &MapStep, attr: &str, ml: &str) -> Option<String> {
        let fixed = !self.cfg.quirks.on(Quirk::SmapMapuniqOutsideForeach);
        maploopreplace(self.run, step.get(attr), ml, fixed)
    }

    /// A failed condition: `map_final` → `next MAP`, else `next` (step),
    /// with biber's two debug messages.
    /// (`msg` is only formatted when debug logging is on.)
    fn fail(&mut self, step: &MapStep, prefix: &MapPrefix, msg: fmt::Arguments) -> StepResult {
        if perl_truthy_opt(step.get("map_final")) {
            debug!(
                self.log,
                "{prefix}{msg} and step has 'final' set, skipping rest of map ..."
            );
            StepResult::Flow(Flow::NextMap)
        } else {
            debug!(self.log, "{prefix}{msg}, skipping step ...");
            StepResult::Flow(Flow::NextStep)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        level: &str,
        key: &str,
        smap: &SourceMaps,
        map: &SourceMap,
        step: &MapStep,
        ml: &str,
        entry: &mut RawEntry,
        news: &mut Vec<NewEntry>,
        created_keys: &mut Vec<String>,
        last_type: &mut String,
        last_field: &mut Option<String>,
        last_fieldval: &mut Option<String>,
        imatches: &mut Vec<Option<String>>,
    ) -> Result<StepResult, Fatal> {
        let truthy = |a: &str| perl_truthy_opt(step.get(a));
        let keyp = MapPrefix { level, key };

        // entry deletion
        if truthy("map_entry_null") {
            debug!(self.log, "{keyp}Ignoring entry completely");
            return Ok(StepResult::Null);
        }

        // new entry
        if let Some(newkey) = self
            .mlr(step, "map_entry_new", ml)
            .filter(|s| perl_truthy(s))
        {
            let newkey = expand_imatches(&newkey, imatches, CaptureDigits::Single);
            let Some(newtype) = self
                .mlr(step, "map_entry_newtype", ml)
                .filter(|s| perl_truthy(s))
            else {
                self.log.biber_warn(format!(
                    "{keyp}Missing type for new entry '{newkey}', skipping step ..."
                ));
                return Ok(StepResult::Flow(Flow::NextStep));
            };
            debug!(self.log, "{keyp}Creating new entry with key '{newkey}'");
            let mut ne = RawEntry {
                key: nfd(&newkey).into_owned(),
                ..RawEntry::default()
            };
            ne.set_type(&newtype);
            debug!(
                self.log,
                "{keyp}created '{newkey}', removing from dependent list"
            );
            created_keys.push(newkey.clone());
            if truthy("map_entry_nocite") {
                self.sec.nocite_citekeys.insert(newkey.clone());
                add_citekey(self.sec, &newkey);
            }
            if self.sec.allkeys {
                add_citekey(self.sec, &newkey);
            }
            insert_new(news, newkey, ne);
        }

        // entry clone
        if let Some(clonekey) = self
            .mlr(step, "map_entry_clone", ml)
            .filter(|s| perl_truthy(s))
        {
            let clonekey = expand_imatches(&clonekey, imatches, CaptureDigits::Single);
            debug!(self.log, "{keyp}cloning entry with new key '{clonekey}'");
            debug!(
                self.log,
                "{keyp}created '{clonekey}', removing from dependent list"
            );
            created_keys.push(clonekey.clone());
            if truthy("map_entry_nocite") {
                self.sec.nocite_citekeys.insert(clonekey.clone());
                add_citekey(self.sec, &clonekey);
            }
            if self.sec.allkeys {
                add_citekey(self.sec, &clonekey);
            }
            insert_new(news, clonekey, entry.clone());
        }

        // entry target
        let (tgt, etargetkey) = match self
            .mlr(step, "map_entrytarget", ml)
            .filter(|s| perl_truthy(s))
        {
            Some(tk) => {
                let tk = expand_imatches(&tk, imatches, CaptureDigits::Single);
                match news.iter().position(|n| n.citekey == tk) {
                    Some(i) => (Target::New(i), tk),
                    None => {
                        self.log.biber_warn(format!(
                            "{keyp}Dynamically created entry target '{tk}' does not exist skipping step ..."
                        ));
                        return Ok(StepResult::Flow(Flow::NextStep));
                    }
                }
            }
            None => (Target::Main, key.to_string()),
        };
        let tp = MapPrefix {
            level,
            key: &etargetkey,
        };

        // entry type map
        if let Some(ts) = self
            .mlr(step, "map_type_source", ml)
            .filter(|s| perl_truthy(s))
        {
            let ts = fc(&ts);
            let cur = target(entry, news, tgt).entrytype.clone();
            if cur != ts {
                return Ok(self.fail(
                    step,
                    &tp,
                    format_args!("Entry type is '{cur}' but map wants '{ts}'"),
                ));
            }
            *last_type = cur;
            let t = fc(&self.mlr(step, "map_type_target", ml).unwrap_or_default());
            debug!(
                self.log,
                "{tp}Changing entry type from '{last_type}' to {t}"
            );
            target(entry, news, tgt).set_type(&t);
        }

        let mut fieldcontinue = false;
        // `my $fieldsource`: undef stringifies as "" everywhere it is used.
        let mut fieldsource = String::new();

        // negated source field map
        if let Some(nf) = self
            .mlr(step, "map_notfield", ml)
            .filter(|s| perl_truthy(s))
        {
            let nf = fc(&nf);
            if target(entry, news, tgt).exists(&nf) {
                return Ok(self.fail(step, &tp, format_args!("Field '{nf}' exists")));
            }
            fieldcontinue = true;
        }

        // cited/nocited conditions (always on the original key)
        let is_cite = self.sec.cite_citekeys.contains(key);
        let is_nocite = self.sec.nocite_citekeys.contains(key);
        let allnocite = self.sec.allkeys_nocite;
        if truthy("map_entrykey_citedornocited") && !(is_cite || is_nocite) {
            return Ok(self.fail(
                step,
                &keyp,
                format_args!("Key is neither \\cited nor \\nocited"),
            ));
        }
        if truthy("map_entrykey_cited") && !is_cite {
            return Ok(self.fail(step, &keyp, format_args!("Key is not \\cited")));
        }
        if truthy("map_entrykey_nocited") && (is_cite || (!is_nocite && !allnocite)) {
            return Ok(self.fail(step, &keyp, format_args!("Key is not \\nocited")));
        }
        if truthy("map_entrykey_allnocited") && !allnocite {
            return Ok(self.fail(step, &keyp, format_args!("Key is not \\nocite{{*}}'ed")));
        }
        // BB-031: without \nocite{*} this applies to every entry.
        let not_starnocited = if self.cfg.quirks.on(Quirk::SmapStarnocited) {
            allnocite && (is_cite || is_nocite)
        } else {
            !allnocite || is_cite || is_nocite
        };
        if truthy("map_entrykey_starnocited") && not_starnocited {
            let m = "Key is \\nocite{*}'ed but also either \\cite'd or explicitly \\nocited";
            return Ok(self.fail(step, &keyp, format_args!("{m}")));
        }

        // field map
        if let Some(fs) = self
            .mlr(step, "map_field_source", ml)
            .filter(|s| perl_truthy(s))
        {
            let fs = fc(&fs);
            if !(fs == "entrykey" || target(entry, news, tgt).exists(&fs)) {
                return Ok(self.fail(step, &tp, format_args!("No field '{fs}'")));
            }
            fieldsource = fs;
            fieldcontinue = true;
        }

        if fieldcontinue {
            *last_field = Some(fieldsource.clone()).filter(|s| !s.is_empty());
            *last_fieldval = if fieldsource == "entrykey" {
                Some(nfd(&target(entry, news, tgt).key).into_owned())
            } else {
                target(entry, news, tgt)
                    .get(&nfc(&fieldsource))
                    .map(str::to_string)
            };
            let lfv = last_fieldval.clone().unwrap_or_default();

            // The step hash is mutated in biber (map_match := notmatch/matchi
            // value); computed locally here, with the same precedence.
            let mut match_attr = "map_match";
            let mut negmatch = false;
            if truthy("map_notmatch") {
                match_attr = "map_notmatch";
                negmatch = true;
            } else if truthy("map_notmatchi") {
                match_attr = "map_notmatchi";
                negmatch = true;
            }
            let mut ci = false;
            if truthy("map_matchi") {
                match_attr = "map_matchi";
                ci = true;
            } else if truthy("map_notmatchi") {
                match_attr = "map_notmatchi";
                ci = true;
            }
            let mut matches_attr = "map_matches";
            let mut cis = false;
            if truthy("map_matchesi") {
                matches_attr = "map_matchesi";
                cis = true;
            }

            if let Some(ms) = step.get(matches_attr).filter(|s| perl_truthy(s)) {
                let ms = split_default_xsvsep(ms);
                let rs = split_default_xsvsep(step.get("map_replace").unwrap_or(""));
                if ms.len() != rs.len() {
                    debug!(
                        self.log,
                        "{tp}Different number of fixed matches vs replaces, skipping ..."
                    );
                    return Ok(StepResult::Flow(Flow::NextStep));
                }
                for (m, r) in ms.iter().zip(&rs) {
                    if (cis && fc(&lfv) == fc(m)) || lfv == *m {
                        target(entry, news, tgt).set(&nfc(&fieldsource), r);
                    }
                }
            }

            if let Some(m) = self.mlr(step, match_attr, ml).filter(|s| perl_truthy(s)) {
                if step.get("map_replace").is_some() {
                    if fieldsource == "entrykey" {
                        debug!(
                            self.log,
                            "{tp}Field '{fieldsource}' is 'entrykey' - cannot remap the value of this field, skipping ..."
                        );
                        return Ok(StepResult::Flow(Flow::NextStep));
                    }
                    let r = self.mlr(step, "map_replace", ml).unwrap_or_default();
                    debug!(
                        self.log,
                        "{tp}Doing match/replace '{m}' -> '{r}' on field '{fieldsource}'"
                    );
                    match ireplace(&lfv, &m, &r, ci) {
                        Ok(rep) => {
                            if let Some(code) = &rep.code_warning {
                                // DC-023: perl code in replacement strings is not run.
                                self.log.biber_warn(format!(
                                    "{tp}Replacement '{r}' contains Perl code ({code}), which is not executed: using the regular expression as the replacement"
                                ));
                            }
                            target(entry, news, tgt).set(&nfc(&fieldsource), &nfc(&rep.value));
                        }
                        Err(e) => {
                            self.regex_error(level, &etargetkey, &e)?;
                            return Ok(StepResult::Flow(Flow::NextStep));
                        }
                    }
                } else {
                    // BB-032: earlier captures are interpolated unescaped.
                    let m = expand_imatches(&m, imatches, CaptureDigits::Multi);
                    let res = match imatch(&lfv, &m, negmatch, ci) {
                        Ok(r) => r,
                        Err(e) => {
                            self.regex_error(level, &etargetkey, &e)?;
                            Vec::new()
                        }
                    };
                    *imatches = res;
                    if imatches.is_empty() {
                        return Ok(self.fail(
                            step,
                            &tp,
                            format_args!("Field '{fieldsource}' does not match '{m}'"),
                        ));
                    }
                }
            }

            // set to a different target
            if let Some(t) = self
                .mlr(step, "map_field_target", ml)
                .filter(|s| perl_truthy(s))
            {
                let t = fc(&t);
                if fieldsource == "entrykey" {
                    debug!(
                        self.log,
                        "{tp}Field '{fieldsource}' is 'entrykey'- cannot map this to a new field as you must have an entrykey, skipping ..."
                    );
                    return Ok(StepResult::Flow(Flow::NextStep));
                }
                if target(entry, news, tgt).exists(&t) {
                    if overwrite(smap, map) {
                        debug!(self.log, "{tp}Overwriting existing field '{t}'");
                    } else {
                        debug!(
                            self.log,
                            "{tp}Field '{fieldsource}' is mapped to field '{t}' but both are defined, skipping ..."
                        );
                        return Ok(StepResult::Flow(Flow::NextStep));
                    }
                }
                // BB-030: the value comes from the entry being mapped, not
                // from the entry target.
                let src: &RawEntry = if self.cfg.quirks.on(Quirk::SmapEntrytargetSource) {
                    entry
                } else {
                    target(entry, news, tgt)
                };
                let v = src
                    .get(&nfc(&fieldsource))
                    .map(str::to_string)
                    .unwrap_or_default();
                let e = target(entry, news, tgt);
                e.set(&nfc(&t), &v);
                e.delete(&fieldsource);
            }
        }

        // field changes
        if let Some(field) = self
            .mlr(step, "map_field_set", ml)
            .filter(|s| perl_truthy(s))
        {
            let field = fc(&field);
            if truthy("map_null") {
                debug!(self.log, "{tp}Deleting field '{field}'");
                target(entry, news, tgt).delete(&field);
            } else {
                if target(entry, news, tgt).exists(&field) && !overwrite(smap, map) {
                    if truthy("map_final") {
                        debug!(
                            self.log,
                            "{tp}Field '{field}' exists, overwrite is not set and step has 'final' set, skipping rest of map ..."
                        );
                        return Ok(StepResult::Flow(Flow::NextMap));
                    }
                    debug!(
                        self.log,
                        "{tp}Field '{field}' exists and overwrite is not set, skipping step ..."
                    );
                    return Ok(StepResult::Flow(Flow::NextStep));
                }
                let fname = nfc(&field).into_owned();
                let mut orig = String::new();
                if truthy("map_append") || truthy("map_appendstrict") {
                    orig = target(entry, news, tgt)
                        .get(&fname)
                        .filter(|v| perl_truthy(v))
                        .unwrap_or("")
                        .to_string();
                }
                let val = if truthy("map_origentrytype") {
                    if !perl_truthy(last_type) {
                        return Ok(StepResult::Flow(Flow::NextStep));
                    }
                    debug!(
                        self.log,
                        "{tp}Setting field '{field}' to '{orig}{last_type}'"
                    );
                    appendstrict_check(step, &orig, last_type)
                } else if truthy("map_origfieldval") {
                    let Some(lfv) = last_fieldval.as_deref().filter(|v| perl_truthy(v)) else {
                        return Ok(StepResult::Flow(Flow::NextStep));
                    };
                    debug!(self.log, "{tp}Setting field '{field}' to '{orig}{lfv}'");
                    appendstrict_check(step, &orig, lfv)
                } else if truthy("map_origfield") {
                    let Some(lf) = last_field.as_deref().filter(|v| perl_truthy(v)) else {
                        return Ok(StepResult::Flow(Flow::NextStep));
                    };
                    debug!(self.log, "{tp}Setting field '{field}' to '{orig}{lf}'");
                    appendstrict_check(step, &orig, lf)
                } else {
                    let fv = self.mlr(step, "map_field_value", ml).unwrap_or_default();
                    let fv = expand_imatches(&fv, imatches, CaptureDigits::Single);
                    debug!(self.log, "{tp}Setting field '{field}' to '{orig}{fv}'");
                    appendstrict_check(step, &orig, &fv)
                };
                target(entry, news, tgt).set(&fname, &val);
            }
        }
        Ok(StepResult::Continue)
    }
}

enum StepResult {
    Continue,
    Flow(Flow),
    /// `map_entry_null`: `return 0` from `create_entry`.
    Null,
}

/// `$newentries{$k} = $e`.
fn insert_new(news: &mut Vec<NewEntry>, citekey: String, entry: RawEntry) {
    match news.iter_mut().find(|n| n.citekey == citekey) {
        Some(n) => n.entry = entry,
        None => news.push(NewEntry { citekey, entry }),
    }
}

/// `File::Spec->file_name_is_absolute`.
fn is_absolute(p: &str) -> bool {
    if cfg!(windows) {
        p.starts_with(['/', '\\'])
            || (p.len() >= 2 && p.as_bytes()[1] == b':' && p.as_bytes()[0].is_ascii_alphabetic())
    } else {
        p.starts_with('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapuniq_ids() {
        assert_eq!(mapuniq_id(1).len(), 9);
        assert_ne!(mapuniq_id(1), mapuniq_id(2));
        let mut r = MapRunState::default();
        let a = maploopreplace(&mut r, Some("k:$MAPLOOP:$MAPUNIQ"), "x", false).unwrap();
        assert_eq!(a, format!("k:x:{}", mapuniq_id(1)));
        let b = maploopreplace(&mut r, Some("$MAPUNIQVAL"), "x", false).unwrap();
        assert_eq!(b, mapuniq_id(1));
        // BB-034: not substituted outside a foreach loop
        assert_eq!(
            maploopreplace(&mut r, Some("$MAPUNIQ"), "", false).unwrap(),
            "$MAPUNIQ"
        );
        assert_eq!(
            maploopreplace(&mut r, Some("$MAPLOOP"), "0", false).unwrap(),
            "$MAPLOOP"
        );
    }

    #[test]
    fn raw_entry_ops() {
        let mut e = RawEntry::default();
        e.set("a", "1");
        e.set("b", "2");
        e.set("a", "3");
        assert_eq!(
            e.fields,
            vec![("a".into(), "3".into()), ("b".into(), "2".into())]
        );
        e.delete("a");
        e.set("a", "\u{e9}");
        assert_eq!(e.get("a"), Some("e\u{301}"));
    }

    use crate::bcf::Document;
    use crate::config::BiberValue;
    use crate::datamodel::DataModel;
    use crate::env::Env;
    use std::collections::BTreeMap;

    fn step(attrs: &[(&str, &str)]) -> MapStep {
        MapStep {
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            extra: BTreeMap::new(),
        }
    }

    fn maps(level: &str, maps: Vec<Vec<MapStep>>) -> SourceMaps {
        SourceMaps {
            datatype: Some("bibtex".into()),
            level: Some(level.into()),
            map_overwrite: None,
            maps: maps
                .into_iter()
                .map(|steps| SourceMap {
                    map_overwrite: None,
                    map_foreach: None,
                    refsection: None,
                    per_datasource: vec![],
                    per_type: vec![],
                    per_nottype: vec![],
                    steps,
                    extra: BTreeMap::new(),
                })
                .collect(),
            extra: BTreeMap::new(),
        }
    }

    /// Map `entry` with `smaps` in a section citing `a` (no `\nocite{*}`).
    fn run(smaps: Vec<SourceMaps>, fix: &[Quirk], entry: &mut RawEntry) -> MapOutcome {
        let mut cfg = Config::default();
        for q in fix {
            cfg.quirks.fix(*q);
        }
        cfg.biber.set("sourcemap", BiberValue::Sourcemap(smaps));
        let dm = DataModel::default();
        let doc = Document::default();
        let env = Env::isolated(".");
        let mut log = Logger::silent();
        let mut ctx = Ctx {
            cfg: &mut cfg,
            dm: &dm,
            doc: &doc,
            log: &mut log,
            env: &env,
            run: Default::default(),
        };
        let mut st = SectionState::default();
        st.section.number = "0".into();
        st.section.cite_citekeys.insert("a".into());
        apply_maps_in(&mut ctx, &mut st, "bibtex", "x.bib", entry).unwrap()
    }

    fn entry() -> RawEntry {
        RawEntry {
            entrytype: "misc".into(),
            key: "a".into(),
            fields: vec![("title".into(), "T".into())],
        }
    }

    #[test]
    fn only_first_driver_map() {
        let mut e = entry();
        run(
            vec![
                maps(
                    "driver",
                    vec![vec![step(&[
                        ("map_field_set", "note"),
                        ("map_field_value", "1"),
                    ])]],
                ),
                maps(
                    "driver",
                    vec![vec![step(&[
                        ("map_field_set", "usera"),
                        ("map_field_value", "2"),
                    ])]],
                ),
                maps(
                    "user",
                    vec![vec![step(&[
                        ("map_field_set", "note"),
                        ("map_field_value", "u"),
                    ])]],
                ),
            ],
            &[],
            &mut e,
        );
        assert_eq!(
            e.fields,
            vec![("title".into(), "T".into()), ("note".into(), "u".into())]
        );
    }

    fn bb030(fix: &[Quirk]) -> RawEntry {
        let mut e = entry();
        let out = run(
            vec![maps(
                "user",
                vec![vec![
                    step(&[("map_entry_new", "n"), ("map_entry_newtype", "misc")]),
                    step(&[
                        ("map_entrytarget", "n"),
                        ("map_field_set", "title"),
                        ("map_field_value", "NEW"),
                    ]),
                    step(&[
                        ("map_entrytarget", "n"),
                        ("map_field_source", "title"),
                        ("map_field_target", "note"),
                    ]),
                ]],
            )],
            fix,
            &mut e,
        );
        assert_eq!(out.created_keys, vec!["n".to_string()]);
        out.new_entries.into_iter().next().unwrap()
    }

    #[test]
    fn bb030_entrytarget_source() {
        assert_eq!(bb030(&[]).fields, vec![("note".into(), "T".into())]);
        assert_eq!(
            bb030(&[Quirk::SmapEntrytargetSource]).fields,
            vec![("note".into(), "NEW".into())]
        );
    }

    #[test]
    fn bb031_starnocited() {
        let smaps = || {
            vec![maps(
                "user",
                vec![vec![step(&[
                    ("map_entrykey_starnocited", "1"),
                    ("map_field_set", "note"),
                    ("map_field_value", "STAR"),
                ])]],
            )]
        };
        let mut e = entry();
        run(smaps(), &[], &mut e);
        assert_eq!(e.get("note"), Some("STAR"));
        let mut e = entry();
        run(smaps(), &[Quirk::SmapStarnocited], &mut e);
        assert_eq!(e.get("note"), None);
    }

    #[test]
    fn bb034_mapuniq_outside_foreach() {
        let smaps = || {
            vec![maps(
                "user",
                vec![vec![step(&[
                    ("map_field_set", "note"),
                    ("map_field_value", "$MAPLOOP-$MAPUNIQ"),
                ])]],
            )]
        };
        let mut e = entry();
        run(smaps(), &[], &mut e);
        assert_eq!(e.get("note"), Some("$MAPLOOP-$MAPUNIQ"));
        let mut e = entry();
        run(smaps(), &[Quirk::SmapMapuniqOutsideForeach], &mut e);
        assert_eq!(
            e.get("note"),
            Some(format!("$MAPLOOP-{}", mapuniq_id(1)).as_str())
        );
    }
}

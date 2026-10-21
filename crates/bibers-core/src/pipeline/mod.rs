//! The processing pipeline: a function-by-function port of `Biber::prepare`
//! (Biber.pm:4474) and `Biber::process_lists` (Biber.pm:3085).
//!
//! Each phase is a function `fn(&mut Ctx, &mut SectionState [, list index])`
//! named after the Perl method it ports, so it can be tested in isolation
//! against biber's state dumps: load biber's state from just before the
//! phase, run the Rust phase, and compare with biber's state just after it
//! (see `crates/bibers/tests/common/phase.rs` and `docs/PLAN.md` §6).
//!
//! Module ownership (one feature area each):
//! - [`fetch`] — section initialisation from the `.bcf`, `fetch_data`, data
//!   source parsing, source maps and field handlers;
//! - [`sourcemap`] — the source map engine (called from `fetch`);
//! - [`resolve`] — citekey aliases, dynamic sets and related entries, XDATA,
//!   sets, crossref/xref inheritance, `validate_datamodel`;
//! - [`entries`] — per-entry processing (`process_entries_static`/`pre`/
//!   `post`/`final`, labelname/labeldate/labeltitle, hashes, visible names);
//! - [`labelalpha`] — alphabetic labels and extraalpha (called from
//!   `entries::process_entries_post`);
//! - [`uniqueness`] — uniquename/uniquelist;
//! - [`sort`] — sort schema, sort keys, sorting, sortinit/sortinithash;
//! - [`lists`] — the `process_lists` driver, list filters, context data
//!   (extra*, singletitle, …).

pub mod entries;
pub mod fetch;
pub mod labelalpha;
pub mod lists;
pub mod resolve;
pub mod sort;
pub mod sourcemap;
pub mod tool;
pub mod uniqueness;

use crate::bcf::Document;
use crate::config::Config;
use crate::datamodel::DataModel;
use crate::env::Env;
use crate::info;
use crate::log::{Fatal, Logger};
use crate::model::{DataList, SectionState};

/// Everything a phase can read or write besides the section state.
pub struct Ctx<'a> {
    /// Options (biber and biblatex, incl. per-entry options set from the
    /// `options` field during `fetch_data`) and global config state.
    pub cfg: &'a mut Config,
    /// The data model (immutable during processing).
    pub dm: &'a DataModel,
    /// The parsed control file (sections, data list declarations).
    pub doc: &'a Document,
    pub log: &'a mut Logger,
    pub env: &'a Env,
    /// Run-level state that persists across sections (biber's global and
    /// module-level state): see [`RunState`].
    pub run: RunState,
}

/// State biber keeps across sections for the whole run (globals and
/// module-level `state` variables), owned by [`Ctx`].
#[derive(Debug, Default)]
pub struct RunState {
    /// libbtparse's process-global state: the macro table (`@string`s
    /// accumulate across files and sections) and the parser's
    /// error-recovery flag.
    pub bt: btparse::BtState,
    /// `Biber::Input::file::bibtex`'s `$cache`: parsed data files (read once
    /// per run), citekey aliases, per-file reference counts.
    pub bibcache: fetch::BibCache,
    /// `Biber::Input::file::biblatexml`'s module state.
    pub bltxml: fetch::biblatexml::BltxmlRunState,
    /// `Biber::Annotation`'s global `$ANN`.
    pub annotations: crate::model::Annotations,
    /// Label generation helper state (citekey index map).
    pub labelalpha: labelalpha::LabelRunCache,
    /// Don't set the entries' `rawdata` field (their source text): set by
    /// [`prepare`] when nothing can read it ([`rawdata_unread`]). Phases run
    /// on their own (the phase tests) keep it, as biber's dumps show it.
    pub skip_rawdata: bool,
    /// Free state as soon as no later phase reads it (set by [`prepare`]):
    /// a phase run on its own (the phase tests) leaves the whole state for
    /// biber's dump.
    pub free_dead_state: bool,
    /// biber's shared `$dmh->{datefields}` once a `foreach … s/date$//`
    /// loop has mutated it in place (BB-028); `None` while pristine. See
    /// [`resolve::datefield_stems`].
    pub datefields: Option<Vec<String>>,
}

/// The result of processing: one state per processed section, in
/// processing order (sections sorted numerically, `Document::sorted_sections`).
#[derive(Debug, Default)]
pub struct Processed {
    pub sections: Vec<SectionState>,
}

/// `Biber::prepare`: process every section with citations.
pub fn prepare(ctx: &mut Ctx) -> Result<Processed, Fatal> {
    let mut out = Processed::default();
    ctx.run.skip_rawdata = rawdata_unread(ctx);
    ctx.run.free_dead_state = true;
    for secnum in sections_to_process(ctx.doc) {
        info!(ctx.log, "Processing section {secnum}");
        let mut st = fetch::init_section(ctx, &secnum)?;
        // biber's preamble list is global: carry it across sections.
        st.preamble = out.sections.last().and_then(|p| p.preamble.clone());
        prepare_section(ctx, &mut st)?;
        release_processed(ctx, &mut st);
        out.sections.push(st);
    }
    Ok(out)
}

/// Whether nothing can read the entries' `rawdata` field. biber sets it on
/// every entry (bibtex.pm:955) and never reads it itself: it can only be
/// reached by name, from the control file or config file (a sorting or
/// label template, a list filter, a data model field, …). If neither
/// mentions it (and not in tool mode, which writes data back), the field is
/// unobservable and isn't made: 20 MB for 17,000 entries.
fn rawdata_unread(ctx: &Ctx) -> bool {
    !(ctx.cfg.biber.tool() || ctx.cfg.names_rawdata || ctx.dm.get_fieldtype("rawdata").is_some())
}

/// Free what nothing reads once a section is processed (the output
/// writers read the entries and part of the lists' state):
/// - the entries' `rawdata` (their source text; only biber's state dumps
///   show it, and the writers output data model fields only);
/// - the names' disambiguation schema and the visible name counts, which
///   only the per-list phases read.
fn release_processed(ctx: &Ctx, st: &mut SectionState) {
    let rawdata = ctx.dm.get_fieldtype("rawdata").is_some();
    for e in st.section.entries.values_mut_unordered() {
        if !rawdata {
            e.derivedfields.remove(&crate::model::sym::RAWDATA);
        }
        // Done growing: the field maps' spare room goes too.
        e.datafields.shrink_to_fit();
        e.derivedfields.shrink_to_fit();
    }
    for dl in &mut st.datalists {
        if let Some(state) = &mut dl.state {
            for f in state.fields_mut().into_iter().flat_map(|m| m.values_mut()) {
                if let Some(f) = f.as_object_mut() {
                    f.shrink_to_fit();
                }
            }
        }
        release_name_data(
            dl,
            &["namedisschema"],
            &["visalpha", "visbib", "viscite", "vissort"],
            true,
        );
    }
}

/// Free the state only `uniqueness` reads, once it has run on the list `dl`
/// (the phases after it read `un`, `namedisschema` and the name hashes):
/// its counters and the names' disambiguation strings, `unall` and
/// `unmininfo`.
pub(crate) fn release_uniqueness_data(dl: &mut DataList) {
    if let Some(state) = &mut dl.state {
        for k in ["uniquenamecount", "uniquenamecount_all", "uniquelistcount"] {
            state.remove(k);
        }
    }
    release_name_data(
        dl,
        &[
            "basenamestring",
            "namestring",
            "namestrings",
            "unall",
            "unmininfo",
        ],
        &[],
        true,
    );
}

/// Remove the keys `name_keys` of every name's data and `list_keys` of
/// every name list's data in the state of `dl` (`namelistdata.{nlid}.{nid}`
/// and `namelistdata.{nlid}`), optionally releasing the maps' spare room.
fn release_name_data(dl: &mut DataList, name_keys: &[&str], list_keys: &[&str], shrink: bool) {
    let Some(state) = &mut dl.state else { return };
    for nl in state.namelistdata_mut().into_iter().flat_map(|m| m.values_mut()) {
        let Some(nl) = nl.as_object_mut() else { continue };
        nl.retain(|k, v| {
            if let Some(n) = v.as_object_mut().filter(|_| k.starts_with("#n")) {
                n.retain(|k, _| !name_keys.contains(&k));
                if shrink {
                    n.shrink_to_fit();
                }
            }
            !list_keys.contains(&k)
        });
        if shrink {
            nl.shrink_to_fit();
        }
    }
}

/// The numbers of the sections [`prepare`] processes (those with
/// citations), in processing order.
pub fn sections_to_process(doc: &Document) -> Vec<String> {
    doc.sorted_sections()
        .iter()
        .filter(|s| !s.citekeys.is_empty() || s.allkeys)
        .map(|s| s.number.clone())
        .collect()
}

/// The per-section phase sequence of `Biber::prepare` (after
/// `fetch::init_section`). Phase tests run individual steps of this.
pub fn prepare_section(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    for (_, phase) in SECTION_PHASES {
        phase(ctx, st)?;
        if *n == "validate_datamodel" && ctx.run.free_dead_state {
            // Read by data source reading and validation only.
            for slot in ["everykey", "everykey_lc", "keytods"] {
                st.section.slots.remove(slot);
            }
        }
    }
    Ok(())
}

/// A section-level phase.
pub type SectionPhase = fn(&mut Ctx, &mut SectionState) -> Result<(), Fatal>;

/// A per-list phase (`list` indexes `SectionState::datalists`).
pub type ListPhase = fn(&mut Ctx, &mut SectionState, usize) -> Result<(), Fatal>;

/// The section-level phases in `Biber::prepare` order, with the method
/// names used in biber's phase dumps.
pub const SECTION_PHASES: &[(&str, SectionPhase)] = &[
    ("fetch_data", fetch::fetch_data),
    ("process_citekey_aliases", resolve::process_citekey_aliases),
    ("instantiate_dynamic", resolve::instantiate_dynamic),
    ("resolve_alias_refs", resolve::resolve_alias_refs),
    ("resolve_xdata", resolve::resolve_xdata),
    ("cite_setmembers", resolve::cite_setmembers),
    ("preprocess_sets", resolve::preprocess_sets),
    ("calculate_interentry", resolve::calculate_interentry),
    ("process_interentry", resolve::process_interentry),
    ("validate_datamodel", resolve::validate_datamodel),
    ("postprocess_sets", resolve::postprocess_sets),
    ("process_entries_static", entries::process_entries_static),
    ("process_lists", lists::process_lists),
];

/// The per-list phases in `Biber::process_lists` order, with the method
/// names used in biber's phase dumps. List filtering happens between
/// `process_entries_final` and `generate_sortdataschema` (inside
/// [`lists::process_lists`]).
pub const LIST_PHASES: &[(&str, ListPhase)] = &[
    ("process_entries_pre", entries::process_entries_pre),
    ("uniqueness", uniqueness::uniqueness),
    ("process_visible_names", entries::process_visible_names),
    ("process_entries_post", entries::process_entries_post),
    ("process_entries_final", entries::process_entries_final),
    ("generate_sortdataschema", sort::generate_sortdataschema),
    ("generate_sortinfo", sort::generate_sortinfo),
    ("sort_list", sort::sort_list),
    ("generate_contextdata", lists::generate_contextdata),
];

/// Look up a phase by its biber method name.
pub fn section_phase(name: &str) -> Option<SectionPhase> {
    SECTION_PHASES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f)
}

/// Look up a per-list phase by its biber method name.
pub fn list_phase(name: &str) -> Option<ListPhase> {
    LIST_PHASES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f)
}

/// Install a section state's per-entry options (e.g. loaded from a biber
/// phase dump) into the option store, replacing that section's existing ones.
pub fn install_entry_options(cfg: &mut Config, st: &SectionState) {
    use crate::config::blx::OptValue;
    let opts = st
        .entry_options
        .iter()
        .map(|(k, m)| {
            let m = m
                .iter()
                .map(|(o, v)| {
                    let v = match v {
                        serde_json::Value::Null => OptValue::Undef,
                        serde_json::Value::String(s) => OptValue::Str(s.clone()),
                        other => OptValue::Str(other.to_string()),
                    };
                    (o.clone(), v)
                })
                .collect();
            (k.clone(), m)
        })
        .collect();
    cfg.blx
        .replace_section_entry_options(&st.section.number, opts);
}

/// Copy the section's per-entry options from the option store into
/// `st.entry_options` (for dumping and comparison with biber's state).
pub fn capture_entry_options(cfg: &Config, st: &mut SectionState) {
    st.entry_options = cfg
        .blx
        .section_entry_options(&st.section.number)
        .into_iter()
        .map(|(k, m)| (k, m.iter().map(|(o, v)| (o.clone(), v.to_json())).collect()))
        .collect();
}

/// Copy the annotations of the section's entries from the run-wide store
/// (`ctx.run.annotations`, biber's global `$ANN`) into `st.annotations`, the
/// per-section view the `.bbl` writer reads and the phase dumps contain.
/// Phases that change annotations call this before returning.
pub fn sync_annotations(ctx: &Ctx, st: &mut SectionState) {
    st.annotations = Some(
        ctx.run
            .annotations
            .to_dump(st.section.entries.keys().map(String::as_str)),
    );
}

/// Install a section state's annotations (e.g. loaded from a biber phase
/// dump) into the run-wide store.
pub fn install_annotations(ctx: &mut Ctx, st: &SectionState) {
    if let Some(a) = &st.annotations {
        ctx.run.annotations.install_dump(a);
    }
}

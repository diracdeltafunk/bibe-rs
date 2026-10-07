//! Section initialisation and data fetching: `fetch_data` (Biber.pm:4574),
//! data source parsing (`Biber::Input::file::bibtex`), source maps and field
//! handlers.

use super::Ctx;
use crate::log::Fatal;
use crate::model::{DataList, PerlData, Section, SectionState};
use serde_json::{Map, Value as J, json};

/// Set up a section's state as biber has it just before `fetch_data`: the
/// `Biber::Section` built while parsing the `.bcf`, its data lists, and
/// `$CONFIG->{state}` as reset by `Biber::Config::_init` (biber's `init`
/// phase dump).
pub fn init_section(ctx: &mut Ctx, secnum: &str) -> Result<SectionState, Fatal> {
    let bs = ctx
        .doc
        .section(secnum)
        .expect("prepare only processes sections from the .bcf");
    let mut section = Section {
        number: bs.number.clone(),
        citekeys: bs.citekeys.clone(),
        citekeys_h: bs.citekeys.iter().cloned().collect(),
        orig_order_citekeys: bs.orig_order_citekeys.clone(),
        allkeys: bs.allkeys,
        allkeys_nocite: bs.allkeys_nocite,
        cite_citekeys: bs.cite.clone(),
        nocite_citekeys: bs.nocite.clone(),
        ..Section::default()
    };
    let empty = || J::Object(Map::new());
    for k in [
        "bcfkeycache",
        "everykey",
        "everykey_lc",
        "keytods",
        "keytorelclone",
        "labelcache_l",
        "labelcache_v",
        "namepartlengths",
        "relclonetokey",
        "relkeys",
        "state",
        "static_keys",
    ] {
        section.slots.insert(k.into(), empty());
    }
    section.slots.insert(
        "dkeys".into(),
        J::Object(
            bs.dynamic_sets
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect(),
        ),
    );
    section.slots.insert(
        "citecount".into(),
        J::Object(
            bs.citecount
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().map_or(J::Null, J::from)))
                .collect(),
        ),
    );
    section.slots.insert(
        "seenkeys".into(),
        J::Object(
            bs.seenkeys
                .iter()
                .map(|(k, v)| (k.clone(), J::from(v.to_string())))
                .collect(),
        ),
    );
    if let Some(ds) = &bs.datasources {
        section.slots.insert(
            "datasources".into(),
            J::Array(
                ds.iter()
                    .map(|d| {
                        let mut o = Map::new();
                        for (k, v) in [
                            ("type", &d.type_),
                            ("name", &d.name),
                            ("datatype", &d.datatype),
                            ("encoding", &d.encoding),
                            ("glob", &d.glob),
                        ] {
                            if let Some(v) = v {
                                o.insert(k.into(), J::from(v.clone()));
                            }
                        }
                        J::Object(o)
                    })
                    .collect(),
            ),
        );
    }

    let datalists = ctx
        .doc
        .lists_for_section(secnum)
        .into_iter()
        .map(datalist_from_bcf)
        .collect();

    Ok(SectionState {
        section,
        datalists,
        config_state: initial_config_state(ctx),
        entry_options: Default::default(),
        // Carried over from the previous section by `prepare`.
        preamble: None,
        // No entries yet, so no annotations.
        annotations: Some(serde_json::Value::Object(Default::default())),
        shared_lists: Some(Vec::new()),
    })
}

fn datalist_from_bcf(d: &crate::bcf::DataList) -> DataList {
    let o = |x: &Option<String>| x.clone().unwrap_or_default();
    let mut dl = DataList {
        name: d.name.clone(),
        type_: d.type_.clone(),
        section: o(&d.section),
        sortingtemplatename: o(&d.sortingtemplatename),
        sortingnamekeytemplatename: o(&d.sortingnamekeytemplatename),
        uniquenametemplatename: o(&d.uniquenametemplatename),
        labelalphanametemplatename: o(&d.labelalphanametemplatename),
        namehashtemplatename: o(&d.namehashtemplatename),
        labelprefix: o(&d.labelprefix),
        ..DataList::default()
    };
    if !d.filters.is_empty() {
        let f = d
            .filters
            .iter()
            .map(|f| match f {
                crate::bcf::Filter::One { type_, value } => filter_json(type_, value),
                crate::bcf::Filter::Or(alts) => {
                    J::Array(alts.iter().map(|(t, v)| filter_json(t, v)).collect())
                }
            })
            .collect();
        dl.slots.insert("filters".into(), J::Array(f));
    }
    if d.sortinit_locale.is_some() {
        // An opaque Unicode::Collate::Locale object in biber's dump.
        dl.slots.insert(
            "sortinitcollator".into(),
            json!({"__class": "Unicode::Collate::Locale"}),
        );
    }
    dl
}

fn filter_json(t: &Option<String>, v: &Option<String>) -> PerlData {
    let mut o = Map::new();
    o.insert("type".into(), t.clone().map_or(J::Null, J::from));
    o.insert("value".into(), v.clone().map_or(J::Null, J::from));
    J::Object(o)
}

/// `$CONFIG->{state}` after `Biber::Config::_init` (Config.pm:66-112).
fn initial_config_state(ctx: &Ctx) -> PerlData {
    let orders =
        |m: &std::collections::HashMap<String, std::collections::HashMap<String, String>>| {
            J::Object(
                m.iter()
                    .map(|(s, keys)| {
                        (
                            s.clone(),
                            J::Object(
                                keys.iter()
                                    .map(|(k, v)| (k.clone(), J::from(v.clone())))
                                    .collect(),
                            ),
                        )
                    })
                    .collect(),
            )
        };
    json!({
        "citkey_aliases": {},
        "control_file_location": "",
        "crossref": [],
        "crossrefkeys": {},
        "datafiles": [],
        "graph": {},
        "internalkeyorder": orders(&ctx.cfg.internal_keyorder),
        "keyorder": orders(&ctx.cfg.keyorder),
        "uniqignore": {},
        "xdata": [],
        "xrefkeys": {},
    })
}

/// `Biber::fetch_data` (Biber.pm:4574): read every data source of the
/// section, create entries for the cited keys (or all keys), and collect
/// dependent keys (`get_dependents`). Not yet implemented: a no-op.
pub fn fetch_data(_ctx: &mut Ctx, _st: &mut SectionState) -> Result<(), Fatal> {
    Ok(())
}

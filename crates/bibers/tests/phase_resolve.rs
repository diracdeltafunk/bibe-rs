//! Dependency resolution phases (`pipeline::resolve`) match biber's state:
//! citekey aliases, dynamic sets and related entries, XDATA, sets,
//! crossref/xref counting and inheritance, data model validation.

mod common;

/// Corpus cases whose resolve phases deliberately differ from biber (or
/// whose input fetch_data cannot read yet), with the documented reason.
const EXCLUDED: &[(&str, &str)] = &[
    // tool mode / bibtex output (DC-030): bibers' setup refuses these
    ("ported/bibtex-output-1", "DC-030"),
    ("ported/full-bibtex-1", "DC-030"),
    ("ported/tool-1", "DC-030"),
    ("ported/tool-2", "DC-030"),
    ("ported/tool-3", "DC-030"),
    ("ported/tool-4", "DC-030"),
    ("ported/tool-bltxml-1", "DC-030"),
    ("ported/tool-bltxml-2", "DC-030"),
    ("ported/tool-bltxml-inout-1", "DC-030"),
];

fn cases() -> Vec<String> {
    common::phase::cases_with_prefix(&[
        "tdata/",
        "biblatex-examples/",
        "bugs/",
        "ported/",
        "styles/",
    ])
    .into_iter()
    .filter(|id| !EXCLUDED.iter().any(|(x, _)| x == id))
    .collect()
}

fn check(phase: &str, extra_ignore: &[&str]) {
    check_norm(phase, extra_ignore, None);
}

fn check_norm(phase: &str, extra_ignore: &[&str], norm: Option<common::phase::Normalise>) {
    let cases = cases();
    let ids: Vec<&str> = cases.iter().map(String::as_str).collect();
    common::phase::assert_phase_normalised(phase, &ids, extra_ignore, norm);
}

/// Sort the array at `path` (keys of the dump's `state`) by its JSON text.
fn sort_at(state: &mut serde_json::Value, path: &[&str]) {
    let mut v = state;
    for p in path {
        match v.get_mut(*p) {
            Some(x) => v = x,
            None => return,
        }
    }
    if let Some(a) = v.as_array_mut() {
        a.sort_by_key(|x| x.to_string());
    }
}

#[test]
fn process_citekey_aliases_matches_biber() {
    check("process_citekey_aliases", &[]);
}

#[test]
fn instantiate_dynamic_matches_biber() {
    check("instantiate_dynamic", &[]);
}

#[test]
fn resolve_alias_refs_matches_biber() {
    check("resolve_alias_refs", &[]);
}

/// biber resolves XDATA walking the entries in Perl hash order, so the order
/// of the `xdata` inheritance records varies (BB-112; bibers: citekey order,
/// DC-017): compare them as a multiset.
#[test]
fn resolve_xdata_matches_biber() {
    check_norm(
        "resolve_xdata",
        &[],
        Some(|s| sort_at(s, &["config_state", "xdata"])),
    );
}

#[test]
fn cite_setmembers_matches_biber() {
    check("cite_setmembers", &[]);
}

#[test]
fn preprocess_sets_matches_biber() {
    check("preprocess_sets", &[]);
}

/// Promoted crossref/xref targets are appended in Perl hash order in biber
/// (BB-061; bibers: first-reference order, DC-014): compare the citekeys as
/// a multiset (the phase only appends to them).
#[test]
fn calculate_interentry_matches_biber() {
    check_norm(
        "calculate_interentry",
        &[],
        Some(|s| {
            sort_at(s, &["section", "citekeys"]);
            sort_at(s, &["section", "orig_order_citekeys"]);
        }),
    );
}

#[test]
fn process_interentry_matches_biber() {
    check("process_interentry", &[]);
}

#[test]
fn validate_datamodel_matches_biber() {
    check("validate_datamodel", &[]);
}

#[test]
fn postprocess_sets_matches_biber() {
    check("postprocess_sets", &[]);
}

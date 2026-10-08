//! Dependency resolution phases (`pipeline::resolve`) match biber's state:
//! citekey aliases, dynamic sets and related entries, XDATA, sets,
//! crossref/xref counting and inheritance, data model validation.

mod common;

/// Corpus cases whose resolve phases deliberately differ from biber (or
/// whose input fetch_data cannot read yet), with the documented reason.
const EXCLUDED: &[(&str, &str)] = &[];

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
    let cases = cases();
    let ids: Vec<&str> = cases.iter().map(String::as_str).collect();
    common::phase::assert_phase_ignoring(phase, &ids, extra_ignore);
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

#[test]
fn resolve_xdata_matches_biber() {
    check("resolve_xdata", &[]);
}

#[test]
fn cite_setmembers_matches_biber() {
    check("cite_setmembers", &[]);
}

#[test]
fn preprocess_sets_matches_biber() {
    check("preprocess_sets", &[]);
}

#[test]
fn calculate_interentry_matches_biber() {
    check("calculate_interentry", &[]);
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

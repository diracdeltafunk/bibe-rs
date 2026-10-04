//! `init`: the section state set up from the `.bcf` before `fetch_data`
//! (`pipeline::fetch::init_section`) matches biber's.

mod common;

#[test]
fn init_section_matches_biber() {
    let cases = common::phase::cases_with_prefix(&["tdata/", "biblatex-examples/"]);
    let ids: Vec<&str> = cases.iter().map(String::as_str).collect();
    common::phase::assert_phase("init", &ids);
}

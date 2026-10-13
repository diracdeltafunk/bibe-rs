//! `fetch_data` on biblatexml data sources (`pipeline::fetch::biblatexml`)
//! matches biber's state.

mod common;

/// Every non-tool corpus case with a `.bltxml` data source.
fn bltxml_cases() -> Vec<String> {
    let root = bibers_testkit::corpus_root();
    common::phase::cases_with_prefix(&["tdata/", "bugs/", "ported/", "regress/", "bltxml/"])
        .into_iter()
        .filter(|id| !id.contains("tool"))
        // deliberate differences (biber dies, or quirks fixed)
        .filter(|id| {
            std::fs::read_to_string(root.join(id).join("case.toml"))
                .map(|t| !t.contains("allow = "))
                .unwrap_or(false)
        })
        .filter(|id| {
            std::fs::read_dir(root.join(id))
                .map(|rd| {
                    rd.flatten()
                        .any(|e| e.path().extension().is_some_and(|x| x == "bltxml"))
                })
                .unwrap_or(false)
        })
        .collect()
}

#[test]
fn fetch_data_biblatexml_matches_biber() {
    let cases = bltxml_cases();
    let ids: Vec<&str> = cases.iter().map(String::as_str).collect();
    assert!(
        ids.len() >= 20,
        "expected biblatexml corpus cases, got {ids:?}"
    );
    common::phase::assert_phase("fetch_data", &ids);
}

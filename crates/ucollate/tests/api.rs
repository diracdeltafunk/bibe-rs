//! API-level tests (expected values taken from the Perl oracle).

use std::cmp::Ordering;

use ucollate::{CollateOptions, Collator, Error, Normalization, Variable};

#[test]
fn biber_default_sort_collator() {
    let c = Collator::new(Some("en-US"), &CollateOptions::biber_sort(4, true)).unwrap();
    assert_eq!(c.locale(), "default");
    assert_eq!(c.locale_version(), None);
    assert_eq!(c.version(), "13.0.0");
    assert_eq!(c.view_sort_key("a"), "[1FA2 | 0020 | 0008 | |]");
    assert_eq!(
        c.view_sort_key("A b"),
        "[1FA2 0209 1FBC | 0020 0020 0020 | 0002 0008 0008 | |]"
    );
    assert_eq!(
        c.sort_key("a"),
        [0x1F, 0xA2, 0, 0, 0, 0x20, 0, 0, 0, 0x08, 0, 0, 0, 0]
    );
    // the truncation marker biber appends to name lists sorts after letters
    assert_eq!(
        c.view_sort_key("\u{10FFFD}"),
        "[FBE1 FFFD | 0020 | 0008 | FFFD FFFD |]"
    );
    assert_eq!(c.cmp("Doe\u{10FFFD}", "Doe Zed"), Ordering::Greater);
    assert_eq!(c.cmp("Apple", "apple"), Ordering::Less);
    // sortcase=false → level 2
    let l2 = Collator::new(Some("en-US"), &CollateOptions::biber_sort(2, false)).unwrap();
    assert_eq!(l2.cmp("Apple", "apple"), Ordering::Equal);
    assert_eq!(l2.view_sort_key("a"), "[1FA2 | 0020 | |]");
}

#[test]
fn locale_tailoring() {
    let opts = CollateOptions::biber_sort(4, true);
    let sv = Collator::new(Some("sv-SE"), &opts).unwrap();
    assert_eq!(sv.locale(), "sv");
    assert_eq!(sv.locale_version(), Some("1.31"));
    assert_eq!(sv.cmp("\u{C4}", "Z"), Ordering::Greater);
    let en = Collator::new(Some("en-US"), &opts).unwrap();
    assert_eq!(en.cmp("A\u{308}", "Z"), Ordering::Less);
    let es = Collator::new(Some("es_ES_trad"), &opts).unwrap();
    assert_eq!(es.locale(), "es__traditional");
    assert_eq!(es.cmp("chico", "cz"), Ordering::Greater);
    let ja = Collator::new(
        Some("ja"),
        &CollateOptions {
            variable: Some(Variable::NonIgnorable),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        ja.view_sort_key("\u{4E00}"),
        "[804B | 0020 | 0002 | 4E00 |]"
    );
    assert_eq!(
        ja.view_sort_key("\u{3042}"),
        "[42DA | 0020 | 0011 | 0001 |]"
    );
}

#[test]
fn option_values_for_biber_log() {
    let en = Collator::new(Some("en-US"), &CollateOptions::default()).unwrap();
    assert_eq!(en.option_value("level").as_deref(), Some("4"));
    assert_eq!(en.option_value("variable").as_deref(), Some("shifted"));
    assert_eq!(en.option_value("normalization").as_deref(), Some("NFD"));
    assert_eq!(en.option_value("upper_before_lower"), None);
    let da = Collator::new(Some("da"), &CollateOptions::default()).unwrap();
    assert_eq!(da.option_value("upper_before_lower").as_deref(), Some("1"));
    let fr = Collator::new(Some("fr-CA"), &CollateOptions::default()).unwrap();
    assert_eq!(fr.option_value("backwards").as_deref(), Some("2"));
}

#[test]
fn perl_style_options() {
    let mut o = CollateOptions::default();
    o.set_perl("level", Some("3")).unwrap();
    o.set_perl("alternate", Some("Non-Ignorable")).unwrap();
    o.set_perl("normalization", Some("prenormalized")).unwrap();
    assert_eq!(o.variable, Some(Variable::NonIgnorable));
    assert_eq!(o.normalization, Some(Normalization::Prenormalized));
    let e = o.set_perl("ignoreChar", Some("x")).unwrap_err();
    assert_eq!(
        e.to_string(),
        "change of ignoreChar via change() is not allowed!"
    );
    assert_eq!(
        o.set_perl("level", Some("0")).unwrap_err(),
        Error::InvalidOption("Illegal level 0 (in value for key 'level') lower than 1.".into())
    );
    assert!(matches!(
        o.set_perl("UCA_Version", Some("36")),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn code_point_api() {
    let c = Collator::new(None, &CollateOptions::default()).unwrap();
    // surrogates and out-of-range values, as Perl strings may contain them
    assert_eq!(
        ucollate::view_key(&c.sort_key_from_code_points(&[0xD800])),
        "[FBC1 D800 | 0020 | 0002 | FFFF |]"
    );
    assert_eq!(
        c.sort_key_from_code_points(&[0x110000]),
        c.sort_key("\u{FFFD}")
    );
    assert_eq!(
        c.sort_key_from_code_points(&"a\u{301}".chars().map(u32::from).collect::<Vec<_>>()),
        c.sort_key("\u{E1}")
    );
}

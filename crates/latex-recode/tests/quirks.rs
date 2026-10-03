//! BB-029 quirk switches and other API behaviour.

use latex_recode::{Normalization, RecodeData, RecodeOptions, Recoder};

fn nfc(s: &str) -> String {
    Normalization::Nfc.apply(s)
}

#[test]
fn bb029_reproduced_by_default() {
    let r = Recoder::builtin("base", "base", RecodeOptions::default());
    assert_eq!(r.decode("\\textdollar{}5"), "${}5");
    assert_eq!(r.decode("\\textdollar 5\\textpercent"), "$5%");
    assert_eq!(nfc(&r.decode("\\pounds{}7 \\pounds {}7")), "£{}7 £7");
    assert_eq!(r.decode("\\ss{}x"), "ßx");
}

#[test]
fn bb029_fixed() {
    let r = Recoder::builtin("base", "base", RecodeOptions::FIXED);
    assert_eq!(r.decode("\\textdollar{}5"), "\\textdollar{}5");
    assert_eq!(r.decode("\\textampersand x"), "\\textampersand x");
    assert_eq!(nfc(&r.decode("\\pounds{}7 \\pounds {}7")), "£7 £{}7");
    let only_a = RecodeOptions {
        fix_raw_specials: true,
        ..RecodeOptions::BIBER
    };
    let r = Recoder::builtin("base", "base", only_a);
    assert_eq!(nfc(&r.decode("\\pounds{}7\\textless{}")), "£{}7<{}");
}

#[test]
fn normalisation_options() {
    let r = Recoder::builtin("base", "base", RecodeOptions::BIBER);
    assert_eq!(r.decode("\\'e"), "e\u{301}");
    assert_eq!(r.decode_with("\\'e", Normalization::Nfc), "\u{e9}");
    assert_eq!(r.decode_with("\\'{\\i}", Normalization::None), "\u{ed}");
    assert!(!r.decode_may_change("plain text é"));
    assert!(r.decode_may_change("a {b}"));
}

#[test]
fn null_sets() {
    let r = Recoder::builtin("null", "null", RecodeOptions::BIBER);
    // Brace elimination still runs with the null decode set (no protection
    // of single-letter groups, which needs the diacritics type).
    assert_eq!(r.decode("{I} \\i"), "I \\i");
    assert_eq!(r.encode("ı é"), "ı é");
}

#[test]
fn custom_data_errors() {
    assert!(
        RecodeData::from_xml(
            "<texmap><maps type='letters' set='x'><map><from>a</from></map></maps></texmap>"
        )
        .is_err()
    );
    assert!(RecodeData::from_xml("not xml").is_err());
    let d = RecodeData::from_xml("<other/>").unwrap();
    assert!(d.raw().blocks.is_empty());
}

#[test]
fn set_attribute_split() {
    use latex_recode::xmldata::split_sets;
    assert_eq!(split_sets("base,full"), ["base", "full"]);
    assert_eq!(split_sets(" base , full ,"), [" base", "full"]);
    assert_eq!(split_sets(",x"), ["", "x"]);
}

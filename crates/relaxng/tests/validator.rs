//! Valid and invalid documents for every pattern kind. The expected
//! verdicts follow the RELAX NG specification; the ones marked "libxml2"
//! were also checked against libxml2 2.9.13 (xmllint / XML::LibXML).

use relaxng::{Options, Schema, ValidateError};

const RNG: &str = r#"xmlns="http://relaxng.org/ns/structure/1.0" datatypeLibrary="http://www.w3.org/2001/XMLSchema-datatypes""#;

fn schema(body: &str) -> Schema {
    let text = if body.trim_start().starts_with("<grammar") {
        body.replacen("<grammar", &format!("<grammar {RNG}"), 1)
    } else {
        format!("<grammar {RNG}><start>{body}</start></grammar>")
    };
    Schema::parse(&text).unwrap_or_else(|e| panic!("schema: {e}\n{text}"))
}

fn check(s: &Schema, valid: &[&str], invalid: &[&str]) {
    for d in valid {
        if let Err(e) = s.validate_str(d) {
            panic!("expected valid: {d}\n{e}");
        }
    }
    for d in invalid {
        match s.validate_str(d) {
            Err(ValidateError::Invalid(errs)) => assert!(!errs.is_empty()),
            Err(e) => panic!("expected invalid, got {e}: {d}"),
            Ok(()) => panic!("expected invalid: {d}"),
        }
    }
}

#[test]
fn empty_text_and_not_allowed() {
    let s = schema(r#"<element name="a"><empty/></element>"#);
    check(&s, &["<a/>", "<a> </a>", "<a><!-- c --></a>"], &["<a>x</a>", "<a><b/></a>", "<b/>"]);
    let s = schema(r#"<element name="a"><text/></element>"#);
    check(&s, &["<a/>", "<a>x</a>", "<a>x<!--c-->y</a>"], &["<a><b/></a>"]);
    let s = schema(r#"<element name="a"><choice><notAllowed/><element name="b"><empty/></element></choice></element>"#);
    check(&s, &["<a><b/></a>"], &["<a/>"]);
}

#[test]
fn group_choice_one_or_more() {
    let s = schema(
        r#"<element name="r"><element name="a"><empty/></element>
             <choice><element name="b"><empty/></element><element name="c"><empty/></element></choice>
             <oneOrMore><element name="d"><empty/></element></oneOrMore>
             <zeroOrMore><element name="e"><empty/></element></zeroOrMore>
             <optional><element name="f"><empty/></element></optional></element>"#,
    );
    check(
        &s,
        &["<r><a/><b/><d/></r>", "<r><a/><c/><d/><d/><e/><e/><f/></r>", "<r>\n <a/> <b/>\n<d/></r>"],
        &[
            "<r><b/><a/><d/></r>",
            "<r><a/><b/><c/><d/></r>",
            "<r><a/><b/></r>",
            "<r><a/><b/><d/><f/><e/></r>",
            "<r><a/><b/><d/>text</r>",
            "<r><a/><b/><d/><f/><f/></r>",
        ],
    );
}

#[test]
fn interleave_and_mixed() {
    let s = schema(
        r#"<element name="r"><interleave>
             <element name="a"><empty/></element>
             <zeroOrMore><element name="b"><empty/></element></zeroOrMore>
             <group><element name="c"><empty/></element><element name="d"><empty/></element></group>
           </interleave></element>"#,
    );
    check(
        &s,
        &["<r><a/><c/><d/></r>", "<r><b/><c/><a/><b/><d/></r>", "<r><c/><b/><d/><a/></r>"],
        &["<r><d/><c/><a/></r>", "<r><c/><d/></r>", "<r><a/><a/><c/><d/></r>"],
    );
    let s = schema(r#"<element name="p"><mixed><zeroOrMore><element name="b"><text/></element></zeroOrMore></mixed></element>"#);
    check(&s, &["<p>x<b>y</b>z<b/></p>", "<p/>"], &["<p><c/></p>"]);
}

#[test]
fn nested_interleave_libxml2_quirk() {
    let s = schema(
        r#"<element name="r"><interleave>
             <interleave><optional><element name="a"><empty/></element></optional>
                         <optional><element name="b"><empty/></element></optional></interleave>
             <interleave><optional><element name="c"><empty/></element></optional>
                         <optional><element name="d"><empty/></element></optional></interleave>
           </interleave></element>"#,
    );
    let lib = Options {
        libxml2_interleave: true,
    };
    // Valid by the specification; libxml2 rejects the first two (checked
    // with libxml2 2.9.13), accepts the others.
    for d in ["<r><a/><c/><b/><d/></r>", "<r><b/> <d/> <a/> <c/></r>"] {
        assert!(s.validate_str(d).is_ok(), "{d}");
        assert!(s.validate_str_with(d, &lib).is_err(), "{d}");
    }
    for d in ["<r><a/><c/><d/><b/></r>", "<r><c/><a/><d/><b/></r>", "<r><a/><b/><c/><d/></r>"] {
        assert!(s.validate_str_with(d, &lib).is_ok(), "{d}");
    }
}

#[test]
fn attributes() {
    let s = schema(
        r#"<element name="a"><attribute name="x"/><optional><attribute name="y"><value>1</value></attribute></optional>
             <zeroOrMore><attribute><anyName><except><name>x</name><name>y</name><nsName ns="urn:no"/></except></anyName></attribute></zeroOrMore>
           </element>"#,
    );
    check(
        &s,
        &[r#"<a x=""/>"#, r#"<a x="v" y=" 1 "/>"#, r#"<a y="1" x="v" z="q" w="r"/>"#, r#"<a xmlns:p="urn:p" x="1" p:z="1"/>"#],
        &["<a/>", r#"<a x="v" y="2"/>"#, r#"<a xmlns:n="urn:no" x="1" n:z="1"/>"#],
    );
    // An optional attribute whose value pattern is nullable.
    let s = schema(r#"<element name="a"><attribute name="x"><optional><data type="int"/></optional></attribute></element>"#);
    check(&s, &[r#"<a x=""/>"#, r#"<a x=" "/>"#, r#"<a x="3"/>"#], &[r#"<a x="q"/>"#]);
}

#[test]
fn namespaces_and_name_classes() {
    let s = schema(
        r#"<element name="p:r" xmlns:p="urn:p"><zeroOrMore><choice>
             <element ns="urn:q" name="a"><empty/></element>
             <element><nsName ns="urn:n"><except><name ns="urn:n">bad</name></except></nsName><empty/></element>
             <element><choice><name>plain</name><name ns="urn:x">other</name></choice><empty/></element>
           </choice></zeroOrMore></element>"#,
    );
    check(
        &s,
        &[
            r#"<r xmlns="urn:p"/>"#,
            r#"<x:r xmlns:x="urn:p"><a xmlns="urn:q"/><n:any xmlns:n="urn:n"/><plain/><o:other xmlns:o="urn:x"/></x:r>"#,
        ],
        &[
            "<r/>",
            r#"<r xmlns="urn:p"><a/></r>"#,
            r#"<r xmlns="urn:p"><n:bad xmlns:n="urn:n"/></r>"#,
            r#"<r xmlns="urn:p"><plain/></r>"#,
        ],
    );
}

#[test]
fn data_value_list_except() {
    let s = schema(
        r#"<element name="r">
             <element name="i"><data type="integer"><param name="minInclusive">1</param><param name="maxInclusive">10</param></data></element>
             <element name="s"><data type="string"><except><value>no</value><value type="string">x</value></except></data></element>
             <element name="l"><list><oneOrMore><choice><value>a</value><data type="int"/></choice></oneOrMore></list></element>
             <element name="b"><value type="boolean">true</value></element>
             <element name="t"><value>a b</value></element>
           </element>"#,
    );
    check(
        &s,
        &[
            "<r><i> 7 </i><s>yes</s><l> a 1  a 2 </l><b>1</b><t>  a   b </t></r>",
            "<r><i>10</i><s>no!</s><l>a</l><b>true</b><t>a b</t></r>",
        ],
        &[
            "<r><i>0</i><s>yes</s><l>a</l><b>1</b><t>a b</t></r>",
            "<r><i>1</i><s>no</s><l>a</l><b>1</b><t>a b</t></r>",
            "<r><i>1</i><s>x</s><l>a</l><b>1</b><t>a b</t></r>",
            "<r><i>1</i><s>y</s><l>a b</l><b>1</b><t>a b</t></r>",
            "<r><i>1</i><s>y</s><l></l><b>1</b><t>a b</t></r>",
            "<r><i>1</i><s>y</s><l>a</l><b>0</b><t>a b</t></r>",
            "<r><i>1</i><s>y</s><l>a</l><b>1</b><t>ab</t></r>",
            "<r><i>1<x/></i><s>y</s><l>a</l><b>1</b><t>a b</t></r>",
        ],
    );
    let s = schema(r#"<element name="q"><data type="QName"/></element>"#);
    check(&s, &[r#"<q xmlns:p="urn:p">p:x</q>"#, "<q>x</q>"], &["<q>u:x</q>", "<q>1x</q>"]);
    let s = schema(r#"<element name="q" xmlns:p="urn:p"><value type="QName">p:x</value></element>"#);
    check(&s, &[r#"<q xmlns:z="urn:p">z:x</q>"#], &[r#"<q xmlns:p="urn:other">p:x</q>"#]);
    let s = schema(r#"<element name="s"><data type="string"><param name="minLength">1</param></data></element>"#);
    check(&s, &["<s> </s>", "<s>x</s>"], &["<s/>", "<s></s>"]);
}

#[test]
fn grammar_features() {
    let s = schema(
        r#"<grammar>
             <start combine="choice"><ref name="a"/></start>
             <start combine="choice"><ref name="b"/></start>
             <define name="a"><element name="a"><ref name="attrs"/><zeroOrMore><ref name="a"/></zeroOrMore></element></define>
             <define name="attrs" combine="interleave"><attribute name="x"/></define>
             <define name="attrs" combine="interleave"><optional><attribute name="y"/></optional></define>
             <div><define name="b"><element name="b"><grammar><start><ref name="inner"/></start>
               <define name="inner"><element name="c"><parentRef name="a"/></element></define></grammar></element></define></div>
           </grammar>"#,
    );
    check(
        &s,
        &[r#"<a x="1"><a x="2" y="3"/></a>"#, r#"<b><c><a x="1"/></c></b>"#],
        &["<a/>", r#"<b><c/></b>"#, "<c/>"],
    );
}

#[test]
fn include_and_external_ref() {
    let resolver = |_: &str, href: &str| -> Result<(String, String), String> {
        let text = match href {
            "lib.rng" => format!(
                r#"<grammar {RNG}><start><element name="x"><empty/></element></start>
                   <define name="item"><element name="item"><text/></element></define></grammar>"#
            ),
            "pat.rng" => format!(r#"<element {RNG} name="ext"><empty/></element>"#),
            _ => return Err(format!("no {href}")),
        };
        Ok((href.to_string(), text))
    };
    let text = format!(
        r#"<grammar {RNG}><include href="lib.rng"><start><element name="list"><zeroOrMore><ref name="item"/></zeroOrMore><optional><externalRef href="pat.rng"/></optional></element></start></include></grammar>"#
    );
    let s = Schema::parse_with_resolver(&text, "main.rng", &resolver).unwrap();
    check(&s, &["<list><item>a</item><ext/></list>", "<list/>"], &["<x/>", "<list><ext/><item/></list>"]);
    assert!(Schema::parse(&text).is_err());
}

#[test]
fn schema_errors() {
    for bad in [
        r#"<grammar {RNG}><start><ref name="nope"/></start></grammar>"#,
        r#"<grammar {RNG}><define name="a"><element name="a"><empty/></element></define></grammar>"#,
        r#"<grammar {RNG}><start><ref name="a"/></start><define name="a"><choice><ref name="a"/><empty/></choice></define></grammar>"#,
        r#"<grammar {RNG}><start><element name="a"><data type="nosuch"/></element></start></grammar>"#,
        r#"<grammar {RNG}><start><element name="a"><data type="string"><param name="whiteSpace">collapse</param></data></element></start></grammar>"#,
        r#"<grammar {RNG}><start><element name="a"><attribute name="b"><attribute name="c"/></attribute></element></start></grammar>"#,
        r#"<grammar {RNG}><start><element name="a"><interleave><element name="b"><empty/></element><element name="b"><empty/></element></interleave></element></start></grammar>"#,
        r#"<grammar {RNG}><start><element name="a"><group><data type="int"/><element name="b"><empty/></element></group></element></start></grammar>"#,
        r#"<grammar {RNG}><start><attribute name="a"/></start></grammar>"#,
        r#"<grammar {RNG}><start><element name="a"><attribute name="xmlns"/></element></start></grammar>"#,
        r#"<grammar {RNG}><start><element name="a"><list><list><data type="int"/></list></list></element></start></grammar>"#,
        r#"<grammar {RNG}><start><element name="a"><value type="int">x</value></element></start></grammar>"#,
        r#"<grammar {RNG}><define name="a"><empty/></define><define name="a"><empty/></define><start><element name="a"><ref name="a"/></element></start></grammar>"#,
        r#"<grammar {RNG}><start><element><anyName><except><anyName/></except></anyName><empty/></element></start></grammar>"#,
        r#"<grammar {RNG}><start><element name="a"><foo/></element></start></grammar>"#,
        "<notxml",
    ] {
        let text = bad.replace("{RNG}", RNG);
        assert!(Schema::parse(&text).is_err(), "should be rejected: {text}");
    }
}

#[test]
fn messages_have_lines_and_elements() {
    let s = schema(r#"<element name="r"><element name="a"><attribute name="n"><data type="int"/></attribute></element></element>"#);
    let Err(ValidateError::Invalid(errs)) = s.validate_str("<r>\n<a n='x'/>\n</r>") else {
        panic!()
    };
    assert_eq!(errs[0].line, 2);
    assert_eq!(errs[0].element, "a");
    assert_eq!(
        errs[0].display_in("f.xml"),
        "f.xml:2: element a: Relax-NG validity error : Invalid attribute n for element a"
    );
    assert!(matches!(s.validate_str("<r>"), Err(ValidateError::NotWellFormed(_))));
}

#[test]
fn biber_schemas_compile() {
    for f in ["bcf.rng", "config.rng"] {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/schemata").join(f);
        Schema::from_file(&p).unwrap_or_else(|e| panic!("{f}: {e}"));
    }
}

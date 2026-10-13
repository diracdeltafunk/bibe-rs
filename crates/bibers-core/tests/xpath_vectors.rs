//! The XPath evaluator (`bibers_core::xmldom::xpath`) against XML::LibXML:
//! `findnodes` (node paths), `findvalue` and `exists`, as biber calls them,
//! for the expressions in `tools/oracle/probes/xpath/xpath_vectors.pl`
//! (vectors generated with the O3 perl, `tests/data/xpath/vectors.jsonl`).

use bibers_core::xmldom::Dom;
use bibers_core::xmldom::xpath::{XPath, XValue, literal_value};

const NS: &str = "http://biblatex-biber.sourceforge.net/biblatexml";

fn doc() -> String {
    // Must match the document in xpath_vectors.pl.
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<bltx:entries xmlns:bltx="{NS}" xmlns:o="http://example.org/o">
  <bltx:entry id="e1" entrytype="book">
    <bltx:names type="author" useprefix="true">
      <bltx:name gender="sm"><bltx:namepart type="family">Doe</bltx:namepart><bltx:namepart type="given"><bltx:namepart initial="J">John</bltx:namepart><bltx:namepart>Q</bltx:namepart></bltx:namepart></bltx:name>
      <bltx:name><bltx:namepart type="family">Roe</bltx:namepart></bltx:name>
    </bltx:names>
    <bltx:title>The <!-- c -->Title</bltx:title>
    <bltx:date>2001</bltx:date>
    <bltx:date type="orig"><bltx:start>1990</bltx:start><bltx:end>1992</bltx:end></bltx:date>
    <bltx:pages><bltx:list><bltx:item><bltx:start>1</bltx:start><bltx:end>5</bltx:end></bltx:item><bltx:item><bltx:start>10</bltx:start><bltx:end>12</bltx:end></bltx:item></bltx:list></bltx:pages>
    <o:title>other</o:title>
    <bltx:note>  spaced   note </bltx:note>
    <bltx:volume>3</bltx:volume>
  </bltx:entry>
  <bltx:entry id="e2" entrytype="misc"><bltx:title>Second</bltx:title><bltx:volume>12</bltx:volume></bltx:entry>
</bltx:entries>
"#
    )
}

#[test]
fn xpath_matches_xml_libxml() {
    let dom = Dom::parse(&doc()).unwrap();
    let root = dom.element_children(dom.document()).next().unwrap();
    let e1 = dom.element_children(root).next().unwrap();
    let ns = dom.in_scope_ns(e1);
    let data = include_str!("data/xpath/vectors.jsonl");
    let mut failures = Vec::new();
    for line in data.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let expr = v["expr"].as_str().unwrap();
        let res = XPath::compile(expr).and_then(|x| x.eval(&dom, e1, &ns));
        let compiles = XPath::compile(expr).is_ok();
        // findnodes
        let got_nodes: Option<Vec<String>> = match &res {
            Ok(XValue::Nodes(n)) => Some(n.iter().map(|x| dom.node_path(*x)).collect()),
            Ok(_) => Some(vec![]),
            Err(_) => None,
        };
        let want_nodes: Option<Vec<String>> = v.get("nodes").map(|n| {
            n.as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string())
                .collect()
        });
        if got_nodes != want_nodes {
            failures.push(format!("{expr}: nodes {got_nodes:?} != {want_nodes:?}"));
        }
        // findvalue
        let got_value = res.as_ref().ok().map(|r| literal_value(&dom, r));
        let want_value = v.get("value").map(|s| s.as_str().unwrap().to_string());
        if got_value != want_value {
            failures.push(format!("{expr}: value {got_value:?} != {want_value:?}"));
        }
        // exists: an evaluation error is true, a compilation error dies
        let got_exists = match (&res, compiles) {
            (_, false) => None,
            (Err(_), true) => Some(1),
            (Ok(XValue::Nodes(n)), _) => Some(i64::from(!n.is_empty())),
            (Ok(XValue::Bool(b)), _) => Some(i64::from(*b)),
            (Ok(XValue::Num(x)), _) => Some(i64::from(*x != 0.0 && !x.is_nan())),
            (Ok(XValue::Str(s)), _) => Some(i64::from(!s.is_empty())),
        };
        let want_exists = v.get("exists").map(|x| x.as_i64().unwrap());
        if got_exists != want_exists {
            failures.push(format!("{expr}: exists {got_exists:?} != {want_exists:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

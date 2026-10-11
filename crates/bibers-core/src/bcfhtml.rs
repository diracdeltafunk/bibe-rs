//! `--convert-control`: the `.bcf` rendered as HTML (`<bcf>.html`), as
//! biber does by applying its `bcf.xsl` with libxslt (Biber.pm:380-417).
//!
//! This is not an XSLT processor: it is a hand-written rendering of that
//! one stylesheet (biber 2.22's `data/bcf.xsl`), reproducing the bytes
//! libxslt (1.1.35, libxml2 2.9.13) writes for it:
//!
//! * the `html` output method with a doctype, but literal result elements
//!   in the XHTML namespace, which libxml2 serialises like XML without
//!   empty-element tags (`<hr></hr>`), without indentation, with an
//!   `xmlns` on `<html>`, and with a final newline;
//! * text escaping of `&`, `<` and `>` only; `disable-output-escaping`
//!   text (`&rarr;`, `&nbsp;`, …) written raw;
//! * the stylesheet's whitespace-only text nodes stripped, its other text
//!   nodes written verbatim (with their newlines and indentation);
//! * `xsl:sort` on text keys (see [`sort_key_cmp`]), stable;
//! * the XPath details the stylesheet relies on (first text node for
//!   `value-of`, existential node-set comparisons, `number()`).
//!
//! The stylesheet is mirrored section by section below; comments quote its
//! structure.

use std::cmp::Ordering;

use roxmltree::{Document, Node};

use crate::env::Env;
use crate::log::{Fatal, Logger};
use crate::{biber_error, info};

const BCF_NS: &str = "https://sourceforge.net/projects/biblatex";

type N<'a, 'i> = Node<'a, 'i>;

// ---------------------------------------------------------------------
// Output

struct Out {
    s: String,
}

impl Out {
    fn open(&mut self, tag: &str) {
        self.s.push('<');
        self.s.push_str(tag);
        self.s.push('>');
    }

    fn open_a(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        self.s.push('<');
        self.s.push_str(tag);
        for (k, v) in attrs {
            self.s.push(' ');
            self.s.push_str(k);
            self.s.push_str("=\"");
            for c in v.chars() {
                match c {
                    '&' => self.s.push_str("&amp;"),
                    '"' => self.s.push_str("&quot;"),
                    '<' => self.s.push_str("&lt;"),
                    '>' => self.s.push_str("&gt;"),
                    c => self.s.push(c),
                }
            }
            self.s.push('"');
        }
        self.s.push('>');
    }

    /// Open `tag` with an optional `class` attribute (from `xsl:attribute`).
    fn open_c(&mut self, tag: &str, class: Option<&str>) {
        match class {
            Some(c) => self.open_a(tag, &[("class", c)]),
            None => self.open(tag),
        }
    }

    fn close(&mut self, tag: &str) {
        self.s.push_str("</");
        self.s.push_str(tag);
        self.s.push('>');
    }

    /// `<tag></tag>`.
    fn empty(&mut self, tag: &str) {
        self.open(tag);
        self.close(tag);
    }

    /// `<tag>text</tag>` with literal text.
    fn el(&mut self, tag: &str, text: &str) {
        self.open(tag);
        self.text(text);
        self.close(tag);
    }

    /// `<tag class="c">text</tag>`.
    fn el_c(&mut self, tag: &str, class: &str, text: &str) {
        self.open_a(tag, &[("class", class)]);
        self.text(text);
        self.close(tag);
    }

    fn text(&mut self, t: &str) {
        for c in t.chars() {
            match c {
                '&' => self.s.push_str("&amp;"),
                '<' => self.s.push_str("&lt;"),
                '>' => self.s.push_str("&gt;"),
                c => self.s.push(c),
            }
        }
    }

    fn raw(&mut self, t: &str) {
        self.s.push_str(t);
    }

    /// A row of header cells: `<tr><td>a</td><td>b</td>…</tr>`.
    fn head_row(&mut self, cells: &[&str]) {
        self.open("tr");
        for c in cells {
            self.el("td", c);
        }
        self.close("tr");
    }
}

// ---------------------------------------------------------------------
// XPath helpers

fn is_bcf(n: &N<'_, '_>, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(BCF_NS)
}

/// `./bcf:name`
fn kids<'a, 'i>(n: Option<N<'a, 'i>>, name: &str) -> Vec<N<'a, 'i>> {
    match n {
        Some(n) => n.children().filter(|c| is_bcf(c, name)).collect(),
        None => Vec::new(),
    }
}

/// `./bcf:a/bcf:b`
fn kids2<'a, 'i>(n: Option<N<'a, 'i>>, a: &str, b: &str) -> Vec<N<'a, 'i>> {
    kids(n, a).into_iter().flat_map(|x| kids(Some(x), b)).collect()
}

/// `./@name` (string value; "" when absent).
fn av<'a>(n: N<'a, '_>, name: &str) -> &'a str {
    n.attributes()
        .find(|a| a.namespace().is_none() && a.name() == name)
        .map(|a| a.value())
        .unwrap_or("")
}

/// `./@name` exists.
fn has(n: N<'_, '_>, name: &str) -> bool {
    n.attributes()
        .any(|a| a.namespace().is_none() && a.name() == name)
}

/// `./@name='v'`
fn is(n: N<'_, '_>, name: &str, v: &str) -> bool {
    n.attributes()
        .any(|a| a.namespace().is_none() && a.name() == name && a.value() == v)
}

/// The text node children (`./text()`).
fn texts<'a>(n: N<'a, '_>) -> impl Iterator<Item = &'a str> {
    n.children().filter(|c| c.is_text()).map(|c| c.text().unwrap_or(""))
}

/// `value-of select="./text()"`: the first text node.
fn text1<'a>(n: N<'a, '_>) -> &'a str {
    texts(n).next().unwrap_or("")
}

/// `value-of select="./x/text()"` over several nodes: the first text node
/// in document order.
fn first_text<'a>(ns: &[N<'a, '_>]) -> &'a str {
    ns.iter().find_map(|n| texts(*n).next()).unwrap_or("")
}

/// XPath `number()` of a string.
fn xpath_number(s: &str) -> f64 {
    let t = s.trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r'));
    let body = t.strip_prefix('-').unwrap_or(t);
    let (i, f) = body.split_once('.').unwrap_or((body, ""));
    let digits = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
    if (i.is_empty() && f.is_empty()) || !digits(i) || !digits(f) {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// The `xsl:sort` comparison for `data-type="text"` keys.
fn sort_key_cmp(a: &str, b: &str) -> Ordering {
    a.as_bytes().cmp(b.as_bytes())
}

/// `xsl:for-each` with one `xsl:sort` (stable).
fn sorted<'a, 'i>(mut v: Vec<N<'a, 'i>>, key: fn(N<'a, 'i>) -> &'a str) -> Vec<N<'a, 'i>> {
    v.sort_by(|a, b| sort_key_cmp(key(*a), key(*b)));
    v
}

fn by_order<'a>(n: N<'a, '_>) -> &'a str {
    av(n, "order")
}

fn by_text<'a>(n: N<'a, '_>) -> &'a str {
    text1(n)
}

/// The `generate-string` template: `text` repeated `number(count)` times.
fn generate_string(o: &mut Out, text: &str, count: &str) {
    let mut c = xpath_number(count);
    if c.is_nan() {
        return;
    }
    // libxslt stops deep template recursion; real counts are small.
    let mut guard = 0;
    while !text.is_empty() && c > 0.0 && guard < 10_000 {
        o.text(text);
        c -= 1.0;
        guard += 1;
    }
}

// ---------------------------------------------------------------------
// The stylesheet

const STYLE: &str = "\n          h2,h3,h4 {\n            font-family: Arial,sans-serif;\n          }\n          .small {\n            font-size: 70%;\n          }\n          .key {\n            font-size: 70%;\n            padding-top: 2ex;\n          }\n          .map_meta {\n            color: #6699CC;\n          }\n          .sort_padding {\n            color: #6699CC;\n          }\n          .sort_substring {\n            color: #FF9933;\n          }\n          .sort_final {\n            background-color: #FFAAAA;\n          }\n          .map_final {\n            color: #FF0000;\n          }\n          .map_newentry {\n            color: #00BB00;\n          }\n          .map_origentrytype {\n            color: #04FF04;\n          }\n          .map_origfield {\n            color: #6699CC;\n          }\n          .map_origfieldval {\n            color: #FF9933;\n          }\n          .map_null {\n            text-decoration: line-through;\n          }\n          .map_regexp {\n            font-size: 60%;\n            font-family: \"Courier New\",monospace;\n          }\n          .la_fieldkey {\n            font-style: italic;\n            color: #AAAAAA;\n            }\n          .la_final {\n            color: #FF0000;\n          }\n          .la_substring {\n            color: #FF9933;\n          }\n          .la_compound {\n            color: #6699CC;\n          }\n          .la_names {\n            color: #04FF04;\n          }\n          .la_namessep {\n            color: #FF0000;\n          }\n          .field_xor_coerce {\n            color: #FF0000;\n          }\n          .inherit_override {\n            color: #FF0000;\n          }\n          .field_nullok {\n            background-color: #99FF99;\n          }\n          .field_skip {\n            background-color: #D0D0D0;\n          }\n          .global_entrytype_fields {\n            color: #666666;\n          }\n          ul {\n            list-style-type: none;\n            margin-left: 0;\n            margin-right: 0;\n            margin-top: 0;\n            margin-bottom: 0;\n            padding-left: 0;\n            padding-top: 0;\n            padding-bottom: 0;\n            padding-right: 0;\n          }\n          table {\n            border-width: 1px;\n            border-spacing: 2px;\n            border-style: hidden;\n            border-color: gray;\n            border-collapse: collapse;\n            background-color: white;\n          }\n          table thead {\n            background-color: #FAF0E6;\n            text-align: center;\n          }\n          table td {\n            wrap: soft;\n            vertical-align: text-top;\n            line-height: 1;\n            padding: 0px 5px 0px 5px;\n            border-width: 1px;\n            border-style: inset;\n            border-color: gray;\n          }\n        ";

/// Render a parsed `.bcf` as biber's `bcf.xsl` does.
pub fn render(doc: &Document<'_>) -> String {
    let mut o = Out {
        s: String::with_capacity(64 * 1024),
    };
    let root = doc.root_element();
    let cf = is_bcf(&root, "controlfile").then_some(root);
    o.raw("<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\">\n");
    o.open_a("html", &[("xmlns", "http://www.w3.org/1999/xhtml")]);
    o.open("head");
    o.open_a("meta", &[("http-equiv", "Content-Type"), ("content", "text/html; charset=UTF-8")]);
    o.close("meta");
    o.el("title", "BibLaTeX control file");
    o.open_a("style", &[("type", "text/css")]);
    o.raw(STYLE);
    o.close("style");
    o.close("head");
    o.open("body");
    // h2
    o.open("h2");
    o.el("tt", "BibLaTeX");
    o.text(" Control File (BCF format version: ");
    o.text(cf.map(|c| av(c, "version")).unwrap_or(""));
    o.text(" biblatex version: ");
    o.text(cf.map(|c| av(c, "bltxversion")).unwrap_or(""));
    o.text(")");
    o.close("h2");
    options(&mut o, cf);
    optionscope(&mut o, cf);
    datafieldsets(&mut o, cf);
    sourcemaps(&mut o, cf);
    labelalphanametemplates(&mut o, cf);
    labelalphatemplates(&mut o, cf);
    extradate(&mut o, cf);
    inheritance(&mut o, cf);
    sorting(&mut o, cf);
    datamodel(&mut o, cf);
    sections(&mut o, cf);
    datalists(&mut o, cf);
    o.close("body");
    o.close("html");
    o.raw("\n");
    o.s
}

// <!-- OPTIONS -->
fn options(o: &mut Out, cf: Option<N<'_, '_>>) {
    o.empty("hr");
    for opts in kids(cf, "options") {
        o.open("h3");
        o.text(av(opts, "type"));
        o.text(" options for ");
        o.el("tt", av(opts, "component"));
        o.close("h3");
        o.open("table");
        o.open("thead");
        o.head_row(&["Option", "Value"]);
        o.close("thead");
        o.open("tbody");
        for opt in kids(Some(opts), "option") {
            if is(opt, "type", "singlevalued") {
                o.open("tr");
                o.el("td", first_text(&kids(Some(opt), "key")));
                o.open("td");
                o.el("tt", first_text(&kids(Some(opt), "value")));
                o.close("td");
                o.close("tr");
            } else if is(opt, "type", "multivalued") {
                o.open("tr");
                o.el("td", first_text(&kids(Some(opt), "key")));
                o.open("td");
                o.open("tt");
                let vals = sorted(kids(Some(opt), "value"), by_order);
                let n = vals.len();
                for (i, v) in vals.into_iter().enumerate() {
                    o.text(text1(v));
                    if i + 1 != n {
                        o.raw(",&nbsp;");
                    }
                }
                o.close("tt");
                o.close("td");
                o.close("tr");
            }
        }
        o.close("tbody");
        o.close("table");
    }
}

// <!-- OPTION SCOPE -->
fn optionscope(o: &mut Out, cf: Option<N<'_, '_>>) {
    o.empty("hr");
    o.el("h3", "Option Scope");
    for os in kids(cf, "optionscope") {
        o.open("table");
        o.open("thead");
        o.open("tr");
        o.open_a("td", &[("colspan", "2")]);
        o.el("b", av(os, "type"));
        o.close("td");
        o.close("tr");
        o.head_row(&["Option", "Datatype", "Output by backend?", "Input as by backend"]);
        o.close("thead");
        o.open("tbody");
        for opt in sorted(kids(Some(os), "option"), by_text) {
            o.open("tr");
            o.el("td", text1(opt));
            o.el("td", av(opt, "datatype"));
            o.open("td");
            if has(opt, "backendout") {
                o.raw("&#10003;");
            }
            o.close("td");
            o.el("td", av(opt, "backendin"));
            o.close("tr");
        }
        o.close("tbody");
        o.close("table");
    }
}

// <!-- DATAFIELD SETS -->
fn datafieldsets(o: &mut Out, cf: Option<N<'_, '_>>) {
    o.empty("hr");
    o.el("h3", "Datafield Sets");
    o.open("table");
    o.open("thead");
    o.open("tr");
    for h in ["Name", "Members"] {
        o.open("td");
        o.el("b", h);
        o.close("td");
    }
    o.close("tr");
    o.close("thead");
    o.open("tbody");
    for set in kids(cf, "datafieldset") {
        o.open("tr");
        o.el("td", av(set, "name"));
        o.open("td");
        let members = kids(Some(set), "member");
        let n = members.len();
        for (i, m) in members.into_iter().enumerate() {
            let last = i + 1 == n;
            if has(m, "datatype") {
                o.el("b", "datatype:");
                o.text(av(m, "datatype"));
                o.raw("&nbsp;");
            }
            if has(m, "fieldtype") {
                o.el("b", "fieldtype:");
                o.text(av(m, "fieldtype"));
                if !last {
                    o.raw(",&nbsp;");
                }
            }
            if has(m, "field") {
                o.text(av(m, "field"));
                if !last {
                    o.raw(",&nbsp;");
                }
            }
        }
        o.close("td");
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
}

/// The "Key" box: `<div class="key"><u>Key</u><ul>…</ul></div>`.
fn key_box(o: &mut Out, items: impl FnOnce(&mut Out)) {
    o.open_a("div", &[("class", "key")]);
    o.el("u", "Key");
    o.open("ul");
    items(o);
    o.close("ul");
    o.close("div");
}

/// `<li><b>bold…</b>rest</li>` where `bold` is written by a closure.
fn li_b(o: &mut Out, bold: impl FnOnce(&mut Out), rest: &str) {
    o.open("li");
    o.open("b");
    bold(o);
    o.close("b");
    o.text(rest);
    o.close("li");
}

// <!-- DATASOURCE MAPPINGS -->
fn sourcemaps(o: &mut Out, cf: Option<N<'_, '_>>) {
    if kids(cf, "sourcemap").is_empty() {
        return;
    }
    o.empty("hr");
    o.el("h3", "Datasource Mappings");
    for maps in kids2(cf, "sourcemap", "maps") {
        o.open("h4");
        o.text(av(maps, "level"));
        o.text(" Mappings for datatype ");
        o.text(av(maps, "datatype"));
        o.text(" (default overwrite = \n            ");
        o.text(if has(maps, "map_overwrite") { "1" } else { "0" });
        o.text(", section = \n              ");
        if has(maps, "refsection") {
            o.text(av(maps, "refsection"));
        } else {
            o.text("all");
        }
        o.text(")");
        o.close("h4");
        for map in kids(Some(maps), "map") {
            o.open("table");
            o.open("thead");
            o.open("tr");
            o.open_a("td", &[("align", "left")]);
            o.text("Mapping (");
            if has(map, "map_overwrite") {
                o.text("overwrite = ");
                o.text(av(map, "map_overwrite"));
            } else {
                o.text("default overwrite");
            }
            for (elem, label) in [("per_type", "for types"), ("per_datasource", "for datasources")] {
                let items = kids(Some(map), elem);
                if !items.is_empty() {
                    o.text("\n                    , ");
                    o.el("b", label);
                    o.text(": \n                    ");
                    let items = sorted(items, by_text);
                    let n = items.len();
                    for (i, it) in items.into_iter().enumerate() {
                        o.text(text1(it));
                        if i + 1 != n {
                            o.text(", ");
                        }
                    }
                }
            }
            if has(map, "map_foreach") {
                o.text("\n                    , foreach loop field=");
                o.text(av(map, "map_foreach"));
            }
            o.text(")");
            o.close("td");
            o.close("tr");
            o.close("thead");
            o.open("tbody");
            for step in kids(Some(map), "map_step") {
                map_step(o, step);
            }
            o.close("tbody");
            o.close("table");
            o.empty("br");
        }
    }
    key_box(o, |o| {
        li_b(o, |o| o.el_c("span", "map_final", "condition"), ": condition must obtain or mapping terminates");
        li_b(
            o,
            |o| {
                o.text("@source");
                o.raw("&rarr;");
                o.text("@target");
            },
            ": Change source entrytype to target entrytype",
        );
        li_b(
            o,
            |o| {
                o.text("source");
                o.raw("&rarr;");
                o.text("target");
            },
            ": Change source field to target field",
        );
        li_b(o, |o| o.el_c("span", "map_null", "field"), ": Delete field");
        li_b(o, |o| o.el_c("span", "map_null", "entry"), ": Delete entry");
        li_b(o, |o| o.text("field=\"string\""), ": Set field to \"string\"");
        li_b(o, |o| o.el_c("span", "map_origentrytype", "TYPE"), ": Most recently mentioned source entrytype");
        li_b(o, |o| o.el_c("span", "map_origfield", "FIELD"), ": Most recently source field");
        li_b(o, |o| o.el_c("span", "map_origfieldval", "FIELDVAL"), ": Most recently source field value");
        li_b(
            o,
            |o| {
                o.text("field");
                o.raw("&asymp;");
                o.text("MATCH");
            },
            ": field must match Regular Expression MATCH",
        );
        li_b(
            o,
            |o| {
                o.text("field");
                o.raw("&asymp;");
                o.text("MATCH ");
                o.raw("&rarr;");
                o.text(" REPLACE");
            },
            ": Perform Regular Expression match/replace on field",
        );
    });
}

fn map_step(o: &mut Out, s: N<'_, '_>) {
    o.open("tr");
    o.open_c("td", is(s, "map_final", "1").then_some("map_final"));
    if has(s, "map_type_source") {
        o.open("span");
        o.text("@");
        o.text(av(s, "map_type_source"));
        o.close("span");
        if has(s, "map_type_target") {
            o.raw("&rarr;");
            o.text("@");
            o.text(av(s, "map_type_target"));
        }
    }
    if has(s, "map_entry_new") {
        o.open_a("span", &[("class", "map_newentry")]);
        o.text("NEW ");
        o.text(av(s, "map_entry_newtype"));
        o.text(" created with key \"");
        o.text(av(s, "map_entry_new"));
        o.text("\"");
        o.close("span");
    }
    if has(s, "map_field_source") {
        o.el("span", av(s, "map_field_source"));
        if has(s, "map_field_target") {
            o.raw("&rarr;");
            o.text(av(s, "map_field_target"));
        }
        for (a, sym) in [
            ("map_match", "&asymp;"),
            ("map_matchi", "&asymp;"),
            ("map_notmatch", "&asymp;"),
            ("map_notmatchi", "&asymp;"),
            ("map_matches", "&asymp;"),
            ("map_matchesi", "&asymp;"),
            ("map_replace", "&rarr;"),
        ] {
            if has(s, a) {
                o.raw(sym);
                o.el_c("span", "map_regexp", av(s, a));
            }
        }
    }
    for (a, t) in [
        ("map_entrykey_cited", "Entry must be CITED via \\cite{key}"),
        ("map_entrykey_nocited", "Entry must be NOCITED via \\nocite{key} or \\nocite{*}"),
        ("map_entrykey_citedornocited", "Entry must be CITED or NOCITED via \\cite{key} or \\nocite{key}"),
        ("map_entrykey_allnocited", "Entry must be NOCITED via \\nocite{}"),
        ("map_entrykey_starnocited", "Entry must only be NOCITED via \\nocite{*}"),
    ] {
        if has(s, a) {
            o.el("span", t);
        }
    }
    if has(s, "map_entry_null") {
        o.el_c("span", "map_null", "entry");
    }
    if has(s, "map_field_set") {
        o.open_c("span", is(s, "map_null", "1").then_some("map_null"));
        o.text(av(s, "map_field_set"));
        o.close("span");
        if has(s, "map_field_value") {
            o.text("=\"");
            o.text(av(s, "map_field_value"));
            o.text("\"");
        }
        for (a, class, t) in [
            ("map_origentrytype", "map_origentrytype", "TYPE"),
            ("map_origfield", "map_origfield", "FIELD"),
            ("map_origfieldval", "map_origfieldval", "FIELDVAL"),
        ] {
            if is(s, a, "1") {
                o.text("=");
                o.el_c("span", class, t);
            }
        }
    }
    if has(s, "map_entrytarget") {
        o.open_a("span", &[("class", "map_newentry")]);
        o.text("\n                          applied to target \"");
        o.text(av(s, "map_entrytarget"));
        o.text("\"");
        o.close("span");
    }
    o.close("td");
    o.close("tr");
}

// <!-- LABELALPHA NAME TEMPLATES -->
fn labelalphanametemplates(o: &mut Out, cf: Option<N<'_, '_>>) {
    let ts = kids(cf, "labelalphanametemplate");
    if ts.is_empty() {
        return;
    }
    o.empty("hr");
    o.el("h4", "Labelalpha Name Templates");
    for t in ts {
        o.open("h4");
        o.text("Template for type ");
        o.text(av(t, "type"));
        o.close("h4");
        o.open("table");
        o.open("thead");
        o.head_row(&["Order", "Pre", "Use option", "Width", "Side", "Compound", "Namepart"]);
        o.close("thead");
        o.open("tbody");
        for np in sorted(kids(Some(t), "namepart"), by_order) {
            o.open("tr");
            for a in ["order", "pre", "use", "substring_width", "substring_side", "substring_compound"] {
                o.el("td", av(np, a));
            }
            o.el("td", text1(np));
            o.close("tr");
        }
        o.close("tbody");
        o.close("table");
        o.empty("br");
    }
}

// <!-- LABELALPHA TEMPLATES -->
fn labelalphatemplates(o: &mut Out, cf: Option<N<'_, '_>>) {
    let ts = kids(cf, "labelalphatemplate");
    if ts.is_empty() {
        return;
    }
    o.empty("hr");
    o.el("h3", "Labelalpha Templates");
    for t in ts {
        o.open("h4");
        o.text("Template for type ");
        o.text(av(t, "type"));
        o.close("h4");
        o.open("table");
        o.open("thead");
        o.open("tr");
        let elems = sorted(kids(Some(t), "labelelement"), by_order);
        for e in &elems {
            o.open("td");
            o.text("Part ");
            o.text(av(*e, "order"));
            o.close("td");
        }
        o.close("tr");
        o.close("thead");
        o.open("tbody");
        o.open("tr");
        for e in &elems {
            o.open("td");
            o.open("ul");
            for p in kids(Some(*e), "labelpart") {
                o.open("li");
                let class = if is(p, "substring_compound", "1") {
                    Some("la_compound")
                } else if is(p, "final", "1") {
                    Some("la_final")
                } else {
                    None
                };
                o.open_c("span", class);
                if has(p, "ifnames") {
                    o.text("\n                                namecount=");
                    o.text(av(p, "ifnames"));
                    o.raw("&rarr;");
                }
                if is(p, "substring_side", "left") {
                    o.open_a("span", &[("class", "la_substring")]);
                    generate_string(o, ">", av(p, "substring_width"));
                    o.close("span");
                }
                o.text(text1(p));
                if has(p, "namessep") {
                    o.open_a("span", &[("class", "la_namessep")]);
                    o.text("(");
                    o.text(av(p, "namessep"));
                    o.text(")");
                    o.close("span");
                }
                if has(p, "names") {
                    o.open_a("span", &[("class", "la_names")]);
                    o.text("=");
                    o.text(av(p, "names"));
                    o.close("span");
                }
                if is(p, "substring_side", "right") {
                    o.open_a("span", &[("class", "la_substring")]);
                    generate_string(o, "<", av(p, "substring_width"));
                    o.close("span");
                }
                for (w, label, extra) in [
                    ("v", "v", "substring_width_max"),
                    ("vf", "vf", "substring_fixed_threshold"),
                ] {
                    if is(p, "substring_width", w) {
                        o.open_a("span", &[("class", "la_substring")]);
                        o.text(label);
                        if has(p, extra) {
                            o.text("/");
                            o.text(av(p, extra));
                        }
                        o.close("span");
                    }
                }
                if is(p, "substring_width", "l") {
                    o.el_c("span", "la_substring", "l");
                }
                if is(p, "noalphaothers", "1") {
                    o.open_a("span", &[("class", "la_final")]);
                    o.raw("&otimes;");
                    o.close("span");
                }
                o.close("span");
                o.close("li");
            }
            o.close("ul");
            o.close("td");
        }
        o.close("tr");
        o.close("tbody");
        o.close("table");
    }
    key_box(o, |o| {
        li_b(o, |o| o.text("Heading key"), ": Label parts are concatenated together in part order shown");
        o.open("li");
        o.el("b", "Labelpart key");
        o.text(": ");
        o.el_c("span", "la_final", "Final label, no more parts are considered");
        o.text(". \"namecount=n");
        o.raw("&rarr;");
        o.el_c("span", "la_fieldkey", "field");
        o.text("\" - conditional name ");
        o.el_c("span", "la_fieldkey", "field");
        o.text("part, only used if there as many names as specified by the range n. Substring specification: \"");
        o.el_c("span", "la_substring", ">>>");
        o.el_c("span", "la_fieldkey", "field");
        o.text("\" = use three chars from left side of field. \"");
        o.el_c("span", "la_fieldkey", "field");
        o.el_c("span", "la_substring", "<<");
        o.text("\" = use two chars from right side of field. \"");
        o.el_c("span", "la_fieldkey", "field");
        o.el_c("span", "la_substring", "v/n");
        o.text("\" = variable-width substring, max n chars. \"");
        o.el_c("span", "la_fieldkey", "field");
        o.el_c("span", "la_substring", "vf/n");
        o.text("\" = variable-width substring fixed to same length as longest string occurring at least n times. \"");
        o.el_c("span", "la_fieldkey", "field");
        o.el_c("span", "la_substring", "l");
        o.text("\" = list scope disambiguation where the label as a whole is unique, not necessarily the individual parts. \"");
        o.el_c("span", "la_fieldkey", "field");
        o.el_c("span", "la_names", "\"=range");
        o.text("=only use the names in the specified range to form the labelpart. \"");
        o.el_c("span", "la_fieldkey", "field");
        o.el_c("span", "la_namessep", "(string)");
        o.text("\"=use string as separator between all names. ");
        o.open_a("span", &[("class", "la_final")]);
        o.raw("&otimes;");
        o.close("span");
        o.text(" = suppress alphaothers");
        o.close("li");
    });
}

/// A list of text values separated by `sep` (raw when `raw_sep`).
fn text_list(o: &mut Out, items: &[N<'_, '_>], sep: &str, raw_sep: bool) {
    let n = items.len();
    for (i, it) in items.iter().enumerate() {
        o.text(text1(*it));
        if i + 1 != n {
            if raw_sep {
                o.raw(sep);
            } else {
                o.text(sep);
            }
        }
    }
}

// <!-- EXTRADATE -->
fn extradate(o: &mut Out, cf: Option<N<'_, '_>>) {
    if kids(cf, "extradatespec").is_empty() {
        return;
    }
    o.empty("hr");
    o.el("h3", "Extradate specification");
    o.open("table");
    o.open("thead");
    o.head_row(&["Fields"]);
    o.close("thead");
    o.open("tbody");
    for scope in kids2(cf, "extradatespec", "scope") {
        o.open("tr");
        o.open("td");
        text_list(o, &kids(Some(scope), "field"), ",&nbsp;", true);
        o.close("td");
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
}

/// The "Child type ≈ Parent type | Source field → Target field | Suppress"
/// header.
fn inheritance_head(o: &mut Out) {
    o.open("thead");
    o.open("tr");
    o.open("td");
    o.text("Child type ");
    o.raw("&asymp;");
    o.text(" Parent type");
    o.close("td");
    o.open("td");
    o.text("Source field ");
    o.raw("&rarr;");
    o.text(" Target field");
    o.close("td");
    o.el("td", "Suppress");
    o.close("tr");
    o.close("thead");
}

/// `target ≈ source` with `all` shown as `*`.
fn type_pair(o: &mut Out, tp: N<'_, '_>) {
    if is(tp, "target", "all") {
        o.text("*");
    } else {
        o.text(av(tp, "target"));
    }
    o.raw(" &asymp; ");
    if is(tp, "source", "all") {
        o.text("*");
    } else {
        o.text(av(tp, "source"));
    }
}

// <!-- INHERITANCE -->
fn inheritance(o: &mut Out, cf: Option<N<'_, '_>>) {
    let inh = kids(cf, "inheritance");
    if inh.is_empty() {
        return;
    }
    let defaults: Vec<N<'_, '_>> = inh.iter().flat_map(|i| kids(Some(*i), "defaults")).collect();
    let any_default = |a: &str, v: &str| defaults.iter().any(|d| is(*d, a, v));
    o.empty("hr");
    o.el("h3", "Inheritance");
    o.el("h4", "Defaults");
    o.open("table");
    inheritance_head(o);
    o.open("tbody");
    o.open("tr");
    o.open("td");
    o.text("* ");
    o.raw(" &asymp; ");
    o.text(" *");
    o.close("td");
    o.open("td");
    o.open_c("span", any_default("override_target", "true").then_some("inherit_override"));
    if any_default("inherit_all", "true") {
        o.text("\n                    * ");
        o.close("span");
        o.raw("&rarr;");
        o.open_c("span", any_default("override_target", "false").then_some("inherit_override"));
        o.text("\n                    * ");
    } else {
        o.text("\n                      *");
        o.close("span");
        o.raw("&rarr;");
        o.open_c("span", any_default("override_target", "false").then_some("inherit_override"));
        o.raw("&empty;");
    }
    o.close("span");
    o.close("td");
    o.el(
        "td",
        defaults
            .iter()
            .find(|d| has(**d, "suppress"))
            .map(|d| av(*d, "suppress"))
            .unwrap_or(""),
    );
    o.close("tr");
    for tp in defaults.iter().flat_map(|d| kids(Some(*d), "type_pair")) {
        o.open("tr");
        o.open("td");
        type_pair(o, tp);
        o.close("td");
        o.open("td");
        o.open_c("span", is(tp, "override_target", "true").then_some("inherit_override"));
        o.text("\n                        *");
        o.close("span");
        if is(tp, "inherit_all", "true") {
            o.raw(" &asymp; ");
            o.open_c("span", is(tp, "override_target", "false").then_some("inherit_override"));
            o.text("\n                        *");
        } else {
            o.raw("&rarr;");
            o.open_c("span", is(tp, "override_target", "false").then_some("inherit_override"));
            o.raw("&empty;");
        }
        o.close("span");
        o.close("td");
        o.el("td", av(tp, "suppress"));
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
    o.el("h4", "Specifications");
    o.open("table");
    inheritance_head(o);
    o.open("tbody");
    for spec in inh.iter().flat_map(|i| kids(Some(*i), "inherit")) {
        o.open("tr");
        o.open("td");
        o.open("ul");
        for tp in kids(Some(spec), "type_pair") {
            o.open("li");
            type_pair(o, tp);
            o.close("li");
        }
        o.close("ul");
        o.close("td");
        o.open("td");
        o.open("ul");
        for f in kids(Some(spec), "field") {
            o.open("li");
            if is(f, "skip", "true") {
                o.text(av(f, "source"));
                o.raw(" &rarr; ");
                o.raw("&empty;");
            } else {
                o.open_c("span", is(f, "override_target", "true").then_some("inherit_override"));
                o.text(av(f, "source"));
                o.close("span");
                o.raw(" &rarr; ");
                o.open_c("span", is(f, "override_target", "false").then_some("inherit_override"));
                o.text(av(f, "target"));
                o.close("span");
            }
            o.close("li");
        }
        o.close("ul");
        o.close("td");
        o.el("td", av(spec, "suppress"));
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
    key_box(o, |o| {
        o.open("li");
        o.el("tt", "*");
        o.text(" matches all entrytypes or fields");
        o.close("li");
        o.open("li");
        o.el("tt", "X");
        o.raw(" &asymp; ");
        o.el("tt", "Y");
        o.text(": ");
        o.el("tt", "X");
        o.text(" inherits from");
        o.el("tt", " Y");
        o.close("li");
        o.open("li");
        o.el("tt", "X");
        o.raw(" &rarr; &empty;");
        o.text(": Field ");
        o.el("tt", "X");
        o.text(" is suppressed");
        o.close("li");
        o.open("li");
        o.el("tt", "F");
        o.raw(" &rarr; ");
        o.el("tt", "F'");
        o.text(": Field ");
        o.el("tt", "F");
        o.text(" in parent becomes field ");
        o.el("tt", "F'");
        o.text(" in child. If both field ");
        o.el("tt", "F");
        o.text(" and field ");
        o.el("tt", "F'");
        o.text(" exist, field in ");
        o.el_c("span", "inherit_override", "red");
        o.text(" overrides the other.");
        o.close("li");
    });
}

/// `/bcf:controlfile/bcf:options[@component='biber']/bcf:option/
/// bcf:key[text()='K']/../bcf:value/text()` is not empty.
fn global_flag(cf: Option<N<'_, '_>>, key: &str) -> bool {
    kids(cf, "options")
        .into_iter()
        .filter(|o| is(*o, "component", "biber"))
        .flat_map(|o| kids(Some(o), "option"))
        .any(|opt| {
            kids(Some(opt), "key").iter().any(|k| texts(*k).any(|t| t == key))
                && kids(Some(opt), "value").iter().any(|v| texts(*v).next().is_some())
        })
}

// <!-- SORTING -->
fn sorting(o: &mut Out, cf: Option<N<'_, '_>>) {
    o.empty("hr");
    o.el("h3", "Global Default Sorting Options");
    o.el("h4", "Presort defaults");
    o.open("table");
    o.open("thead");
    o.head_row(&["Entrytype", "Presort default"]);
    o.close("thead");
    o.open("tbody");
    for p in kids(cf, "presort") {
        o.open("tr");
        o.open("td");
        if has(p, "type") {
            o.text(av(p, "type"));
        } else {
            o.text("\n                      ALL\n                    ");
        }
        o.close("td");
        o.open("td");
        o.el("tt", text1(p));
        o.close("td");
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
    for (title, head, elem, item) in [
        ("Sorting exclusions", "Fields excluded from sorting", "sortexclusion", "exclusion"),
        ("Sorting inclusions", "Fields forcibly included in sorting", "sortinclusion", "inclusion"),
    ] {
        o.el("h4", title);
        o.open("table");
        o.open("thead");
        o.head_row(&["Entrytype", head]);
        o.close("thead");
        o.open("tbody");
        for e in kids(cf, elem) {
            o.open("tr");
            o.el("td", av(e, "type"));
            o.open("td");
            text_list(o, &kids(Some(e), item), ",&nbsp;", true);
            o.close("td");
            o.close("tr");
        }
        o.close("tbody");
        o.close("table");
    }
    o.el("h4", "Uniquename Template");
    o.open("table");
    o.open("thead");
    o.head_row(&["Order", "Base", "Use option", "Disambiguation level", "Namepart"]);
    o.close("thead");
    o.open("tbody");
    for np in sorted(kids2(cf, "uniquenametemplate", "namepart"), by_order) {
        o.open("tr");
        for a in ["order", "base", "use", "disambiguation"] {
            o.el("td", av(np, a));
        }
        o.el("td", text1(np));
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
    o.empty("br");
    o.el("h4", "Name hash Template");
    o.open("table");
    o.open("thead");
    o.head_row(&["Order", "Scope", "Namepart"]);
    o.close("thead");
    o.open("tbody");
    for np in sorted(kids2(cf, "namehashtemplate", "namepart"), by_order) {
        o.open("tr");
        o.el("td", av(np, "order"));
        o.el("td", av(np, "hashscope"));
        o.el("td", text1(np));
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
    o.empty("br");
    o.el("h4", "Sorting Name Key Template");
    for t in kids(cf, "sortingnamekeytemplate") {
        o.open("table");
        o.open("thead");
        o.open("tr");
        o.open_a("td", &[("colspan", "2")]);
        o.el("b", av(t, "name"));
        o.text(" (visibility: ");
        o.text(av(t, "visibility"));
        o.text(")");
        o.close("td");
        o.close("tr");
        o.head_row(&["Order", "Keypart"]);
        o.close("thead");
        o.open("tbody");
        for kp in sorted(kids(Some(t), "keypart"), by_order) {
            o.open("tr");
            o.el("td", av(kp, "order"));
            o.open("td");
            let parts = sorted(kids(Some(kp), "part"), by_order);
            let n = parts.len();
            for (i, p) in parts.into_iter().enumerate() {
                o.text(text1(p));
                if has(p, "use") {
                    o.open_a("span", &[("class", "small")]);
                    o.text(" (if use");
                    o.text(text1(p));
                    o.text("=");
                    o.text(av(p, "use"));
                    o.text(")");
                    o.close("span");
                }
                if has(p, "inits") {
                    o.text(" (Initials only)");
                }
                if i + 1 != n {
                    o.text(" + ");
                }
            }
            o.close("td");
            o.close("tr");
        }
        o.close("tbody");
        o.close("table");
        o.empty("br");
    }
    o.el("h4", "Sorting Schemes");
    let sortupper = global_flag(cf, "sortupper");
    let sortcase = global_flag(cf, "sortcase");
    for spec in kids(cf, "sortingtemplate") {
        sorting_template(o, spec, sortupper, sortcase);
    }
    let tls = kids(cf, "transliteration");
    if !tls.is_empty() {
        o.el("h4", "Sorting Transliteration");
        for spec in tls {
            transliteration_spec(o, spec);
            o.empty("br");
        }
    }
}

// <xsl:template name="sorting-template">
fn sorting_template(o: &mut Out, spec: N<'_, '_>, sortupper: bool, sortcase: bool) {
    o.open("h3");
    o.text("Sorting Scheme: ");
    o.text(av(spec, "name"));
    o.close("h3");
    o.open("table");
    o.open("thead");
    o.open("tr");
    let sorts = sorted(kids(Some(spec), "sort"), by_order);
    for s in &sorts {
        let s = *s;
        o.open_c("td", is(s, "final", "1").then_some("sort_final"));
        o.raw(if is(s, "sort_direction", "descending") { "&darr;" } else { "&uarr;" });
        o.open("tt");
        let upper = if has(s, "sortupper") { is(s, "sortupper", "1") } else { sortupper };
        o.text(if upper { "Aa/" } else { "aA/" });
        let case = if has(s, "sortcase") { is(s, "sortcase", "1") } else { sortcase };
        o.text(if case { "A" } else { "a" });
        o.close("tt");
        o.close("td");
    }
    o.close("tr");
    o.close("thead");
    o.open("tbody");
    o.open("tr");
    for s in &sorts {
        o.open("td");
        o.open("ul");
        for it in sorted(kids(Some(*s), "sortitem"), by_order) {
            o.open("li");
            if is(it, "pad_side", "left") {
                o.open_a("span", &[("class", "sort_padding")]);
                generate_string(o, av(it, "pad_char"), av(it, "pad_width"));
                o.close("span");
            }
            if is(it, "substring_side", "left") {
                o.open_a("span", &[("class", "sort_substring")]);
                generate_string(o, ">", av(it, "substring_width"));
                o.close("span");
            }
            o.text(text1(it));
            if is(it, "pad_side", "right") {
                o.open_a("span", &[("class", "sort_padding")]);
                generate_string(o, av(it, "pad_char"), av(it, "pad_width"));
                o.close("span");
            }
            if is(it, "substring_side", "right") {
                o.open_a("span", &[("class", "sort_substring")]);
                generate_string(o, "<", av(it, "substring_width"));
                o.close("span");
            }
            o.close("li");
        }
        o.close("ul");
        o.close("td");
    }
    o.close("tr");
    o.close("tbody");
    o.close("table");
    key_box(o, |o| {
        li_b(o, |o| o.text("Heading Format"), ": (sort-direction)(case-order)/(case-sensitivity)");
        o.open("li");
        o.el("b", "Heading key");
        o.text(": ");
        o.raw("&uarr;");
        o.text(" = ascending sort, ");
        o.raw("&darr;");
        o.text(" = descending sort, ");
        o.el("tt", "Aa");
        o.text(" = sort uppercase before lower, ");
        o.el("tt", "aA");
        o.text(" = sort lowercase before upper, ");
        o.el("tt", "A");
        o.text(" = case-sensitive sorting, ");
        o.el("tt", "a");
        o.text(" = case-insensitive sorting, ");
        o.el_c("span", "sort_final", "sort fieldset is final master key if defined");
        o.close("li");
        o.open("li");
        o.el("b", "Field key");
        o.text(": ");
        o.el_c("span", "sort_padding", "Padding specification");
        o.text(" e.g. ");
        o.el_c("span", "sort_padding", "0000");
        o.text("field = pad field \"field\" from left with \"0\" to width 4. ");
        o.el_c("span", "sort_substring", "Substring specification");
        o.text(" e.g. field");
        o.el_c("span", "sort_substring", "<<<<");
        o.text(" = take width 4 substring from right side of field \"field\"");
        o.close("li");
    });
}

// <xsl:template name="transliteration-spec">
fn transliteration_spec(o: &mut Out, spec: N<'_, '_>) {
    o.open("table");
    o.open("thead");
    o.open("tr");
    o.open_a("td", &[("colspan", "3")]);
    o.open("b");
    o.text("Entrytype: ");
    o.text(av(spec, "entrytype"));
    o.close("b");
    o.close("td");
    o.close("tr");
    o.open("tr");
    for h in ["Target", "From", "To"] {
        o.open("td");
        o.el("b", h);
        o.close("td");
    }
    o.close("tr");
    o.close("thead");
    o.open("tbody");
    for t in kids(Some(spec), "translit") {
        o.open("tr");
        for a in ["target", "from", "to"] {
            o.el("td", av(t, a));
        }
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
}

/// `A[text()=B/text()]` for two elements: some text child of `a` equals
/// some text child of `b`.
fn text_eq(a: N<'_, '_>, b: N<'_, '_>) -> bool {
    texts(a).any(|x| texts(b).any(|y| x == y))
}

/// A comma-separated `quant (f1,f2)` constraint part.
fn quant(o: &mut Out, nodes: &[N<'_, '_>]) {
    if nodes.iter().any(|n| is(*n, "quant", "all")) {
        o.raw("&forall;");
    } else if nodes.iter().any(|n| is(*n, "quant", "one")) {
        o.raw("&exist;");
    } else if nodes.iter().any(|n| is(*n, "quant", "none")) {
        o.raw("&not;&exist;");
    }
}

// <!-- DATA MODEL -->
fn datamodel(o: &mut Out, cf: Option<N<'_, '_>>) {
    let dm = kids(cf, "datamodel");
    if dm.is_empty() {
        return;
    }
    let all = |name: &str| -> Vec<N<'_, '_>> { dm.iter().flat_map(|d| kids(Some(*d), name)).collect() };
    o.empty("hr");
    o.el("h3", "Data Model");
    o.el("h4", "Constants");
    o.open("table");
    o.open("thead");
    o.head_row(&["Name", "Type", "Value"]);
    o.close("thead");
    o.open("tbody");
    for c in all("constants").into_iter().flat_map(|c| kids(Some(c), "constant")) {
        o.open("tr");
        o.el("td", av(c, "name"));
        o.el("td", av(c, "type"));
        o.el("td", text1(c));
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
    o.el("h4", "Legal entrytypes");
    o.open("table");
    o.open("thead");
    o.head_row(&["Entrytype", "Legal fields for entrytype"]);
    o.close("thead");
    o.open("tbody");
    o.open("tr");
    o.el("td", "GLOBAL");
    o.open("td");
    o.open_a("div", &[("class", "global_entrytype_fields")]);
    let entryfields = all("entryfields");
    let global: Vec<N<'_, '_>> = entryfields
        .iter()
        .filter(|e| kids(Some(**e), "entrytype").is_empty())
        .flat_map(|e| kids(Some(*e), "field"))
        .collect();
    text_list(o, &sorted(global, by_text), ", ", false);
    o.close("div");
    o.close("td");
    o.close("tr");
    for et in all("entrytypes").into_iter().flat_map(|e| kids(Some(e), "entrytype")) {
        o.open("tr");
        o.el("td", text1(et));
        o.open("td");
        let mentions = |ef: &N<'_, '_>| kids(Some(*ef), "entrytype").iter().any(|x| text_eq(*x, et));
        if !entryfields.iter().any(mentions) {
            o.el_c("div", "global_entrytype_fields", "GLOBAL fields");
        }
        for ef in &entryfields {
            if mentions(ef) {
                o.el_c("div", "global_entrytype_fields", "GLOBAL fields");
                text_list(o, &sorted(kids(Some(*ef), "field"), by_text), ", ", false);
            }
        }
        o.close("td");
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
    o.el("h4", "Field Types");
    o.open("table");
    o.open("thead");
    o.head_row(&["Field", "Field Format", "Data type"]);
    o.close("thead");
    o.open("tbody");
    for f in all("fields").into_iter().flat_map(|f| kids(Some(f), "field")) {
        o.open("tr");
        o.open("td");
        o.text(text1(f));
        if is(f, "nullok", "true") {
            o.raw("&empty;");
        }
        if is(f, "skip_output", "true") {
            o.raw("&loz;");
        }
        o.close("td");
        o.el("td", if has(f, "format") { av(f, "format") } else { "standard" });
        o.open("td");
        o.text(av(f, "datatype"));
        o.raw("&nbsp;");
        o.text(av(f, "fieldtype"));
        o.close("td");
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
    key_box(o, |o| {
        o.open("li");
        o.raw("&empty;");
        o.text(" = field can null in ");
        o.el("tt", ".bbl");
        o.text(", ");
        o.raw("&loz;");
        o.text(" = field is not output to ");
        o.el("tt", ".bbl");
        o.close("li");
    });
    let constraints = all("constraints");
    if constraints.is_empty() {
        return;
    }
    o.empty("hr");
    o.el("h3", "Constraints");
    o.open("table");
    o.open("thead");
    o.head_row(&["Entrytypes", "Constraint"]);
    o.close("thead");
    o.open("tbody");
    for cs in constraints {
        o.open("tr");
        o.open("td");
        o.open("ul");
        for et in kids(Some(cs), "entrytype") {
            o.el("li", text1(et));
        }
        o.close("ul");
        o.close("td");
        o.open("td");
        o.open("ul");
        for c in kids(Some(cs), "constraint") {
            o.open("li");
            constraint(o, c);
            o.close("li");
        }
        o.close("ul");
        o.close("td");
        o.close("tr");
    }
    o.close("tbody");
    o.close("table");
    key_box(o, |o| {
        o.open("li");
        o.el("tt", "C");
        o.raw("&rarr;");
        o.el("tt", "C'");
        o.text(": If condition ");
        o.el("tt", "C");
        o.text(" is met then condition ");
        o.el("tt", "C'");
        o.text(" must also be met");
        o.close("li");
        for (sym, t) in [
            ("&forall;", " ( ... ): True if all fields in list exist"),
            ("&exist;", " ( ... ): True if one field in list exists"),
            ("&not;&exist;", " ( ... ): True if no fields in list exist"),
        ] {
            o.open("li");
            o.raw(sym);
            o.text(t);
            o.close("li");
        }
        o.open("li");
        o.el("tt", "n");
        o.raw("&le;");
        o.text(" ( ... ) ");
        o.raw("&le;");
        o.el("tt", "m");
        o.text(": True if fields in list are have values in the range ");
        o.el("tt", "n");
        o.text("-");
        o.el("tt", "m");
        o.close("li");
        o.open("li");
        o.raw("&oplus;");
        o.text(" ( ... ): True if at least and at most one of the fields in the list exists (XOR). If more than field in the set exists, all will be ignored except for the one in ");
        o.el_c("span", "field_xor_coerce", "red");
        o.close("li");
        o.open("li");
        o.raw("&or;");
        o.text(" ( ... ): True if at least one of the fields in the list exists (OR)");
        o.close("li");
    });
}

fn constraint(o: &mut Out, c: N<'_, '_>) {
    let fields = |nodes: &[N<'_, '_>], o: &mut Out| {
        let fs: Vec<N<'_, '_>> = nodes.iter().flat_map(|n| kids(Some(*n), "field")).collect();
        text_list(o, &fs, ",", false);
    };
    if is(c, "type", "conditional") {
        let ante = kids(Some(c), "antecedent");
        let cons = kids(Some(c), "consequent");
        quant(o, &ante);
        o.text("\n                                (\n                                ");
        fields(&ante, o);
        o.text("\n                                )\n                                ");
        o.raw("&rarr; ");
        quant(o, &cons);
        o.text("\n                                (\n                                ");
        fields(&cons, o);
        o.text("\n                                )\n                              ");
    } else if is(c, "type", "data") {
        if is(c, "datatype", "integer") {
            o.text(av(c, "rangemin"));
            o.raw("&le;");
            o.text("\n                                    (\n                                    ");
            fields(&[c], o);
            o.text("\n                                    )\n                                    ");
            o.raw("&le;");
            o.text(av(c, "rangemax"));
        } else if is(c, "datatype", "datespec") {
            o.text("\n                                    (\n                                    ");
            fields(&[c], o);
            o.text("\n                                    )\n                                    must be dates\n                                  ");
        }
    }
    if is(c, "type", "mandatory") {
        for x in kids(Some(c), "fieldxor") {
            o.raw("&oplus;");
            o.text("\n                                  (\n                                  ");
            let fs = kids(Some(x), "field");
            let n = fs.len();
            for (i, f) in fs.into_iter().enumerate() {
                o.open_c("span", is(f, "coerce", "true").then_some("field_xor_coerce"));
                o.text(text1(f));
                o.close("span");
                if i + 1 != n {
                    o.text(",");
                }
            }
            o.text("\n                                  )\n                                ");
        }
        for x in kids(Some(c), "fieldor") {
            o.raw("&or;");
            o.text("\n                                  (\n                                  ");
            text_list(o, &kids(Some(x), "field"), ",", false);
            o.text("\n                                  )\n                                ");
        }
    }
}

/// A data source `<li>`; `tail` is the whitespace text after it.
fn datasource_li(o: &mut Out, ds: N<'_, '_>, tail: &str) {
    o.open("li");
    o.text(text1(ds));
    o.text(" (");
    o.text(av(ds, "datatype"));
    o.raw("&nbsp;");
    o.text(av(ds, "type"));
    o.text(tail);
    o.close("li");
}

/// `/bcf:controlfile/bcf:bibdata[@section=S]/bcf:datasource`, with `S`
/// a node-set (empty: no match).
fn datasources<'a, 'i>(cf: Option<N<'a, 'i>>, section: Option<&str>) -> Vec<N<'a, 'i>> {
    kids(cf, "bibdata")
        .into_iter()
        .filter(|b| section.is_some_and(|s| is(*b, "section", s)))
        .flat_map(|b| kids(Some(b), "datasource"))
        .collect()
}

// Reference sections
fn sections(o: &mut Out, cf: Option<N<'_, '_>>) {
    o.empty("hr");
    o.el("h3", "Reference Sections");
    o.el("h4", "Section 0");
    o.open("table");
    o.open("thead");
    o.head_row(&["Data sources", "Citekeys", "Citekey counts", "Dynamic sets"]);
    o.close("thead");
    o.open("tbody");
    o.open("tr");
    o.open("td");
    o.open("ul");
    for ds in datasources(cf, Some("0")) {
        datasource_li(o, ds, ")\n                      ");
    }
    o.close("ul");
    o.close("td");
    let sec0: Vec<N<'_, '_>> = kids(cf, "section").into_iter().filter(|s| is(*s, "number", "0")).collect();
    let keys: Vec<N<'_, '_>> = sec0.iter().flat_map(|s| kids(Some(*s), "citekey")).collect();
    o.open("td");
    o.open("ul");
    for k in keys.iter().filter(|k| !is(**k, "type", "set")) {
        o.open("li");
        o.el("tt", text1(*k));
        o.close("li");
    }
    o.close("ul");
    o.close("td");
    o.open("td");
    o.open("ul");
    for k in sec0.iter().flat_map(|s| kids(Some(*s), "citekeycount")) {
        citekeycount_li(o, k);
    }
    o.close("ul");
    o.close("td");
    o.open("td");
    o.open("ul");
    for k in keys.iter().filter(|k| is(**k, "type", "set")) {
        o.open("li");
        o.open("tt");
        o.text(text1(*k));
        o.raw("&nbsp;");
        o.text("(");
        o.text(av(*k, "members"));
        o.text(")");
        o.close("tt");
        o.close("li");
    }
    o.close("ul");
    o.close("td");
    o.close("tr");
    o.close("tbody");
    o.close("table");
    for sec in kids(cf, "section") {
        // [@number != '0']: the attribute exists and differs.
        if !has(sec, "number") || is(sec, "number", "0") {
            continue;
        }
        let num = sec
            .attributes()
            .find(|a| a.namespace().is_none() && a.name() == "number")
            .map(|a| a.value());
        o.open("h4");
        o.text("Section ");
        o.text(num.unwrap_or(""));
        o.close("h4");
        o.open("table");
        o.open("thead");
        o.head_row(&["Data sources", "Citekeys", "Citekey counts"]);
        o.close("thead");
        o.open("tbody");
        o.open("tr");
        o.open("td");
        o.open("ul");
        for ds in datasources(cf, num) {
            datasource_li(o, ds, ")\n                        ");
        }
        o.close("ul");
        o.close("td");
        o.open("td");
        o.open("ul");
        for k in kids(Some(sec), "citekey") {
            o.open("li");
            o.el("tt", text1(k));
            o.close("li");
        }
        o.close("ul");
        o.close("td");
        o.open("td");
        o.open("ul");
        for k in kids(Some(sec), "citekeycount") {
            citekeycount_li(o, k);
        }
        o.close("ul");
        o.close("td");
        o.close("tr");
        o.close("tbody");
        o.close("table");
    }
}

fn citekeycount_li(o: &mut Out, k: N<'_, '_>) {
    o.open("li");
    o.open("tt");
    o.text(text1(k));
    o.text(" (");
    o.text(av(k, "count"));
    o.text(")");
    o.close("tt");
    o.close("li");
}

// Data lists
fn datalists(o: &mut Out, cf: Option<N<'_, '_>>) {
    o.el("h3", "Data Lists");
    for dl in kids(cf, "datalist") {
        o.open("h4");
        o.open("u");
        o.text("Data list \"");
        o.text(av(dl, "name"));
        o.text("\"");
        o.close("u");
        o.close("h4");
        o.open("table");
        o.open("thead");
        o.head_row(&["Type", "Sorting Scheme", "Labelprefix", "Uniquename Template Name", "Labelalphaname Template Name"]);
        o.close("thead");
        o.open("tbody");
        o.open("tr");
        for a in ["type", "sortscheme", "labelprefix", "uniquenametemplatename", "labelalphanametemplatename"] {
            o.el("td", av(dl, a));
        }
        o.close("tr");
        o.close("tbody");
        o.close("table");
        let filters = kids(Some(dl), "filter");
        if !filters.is_empty() {
            o.open("div");
            o.el("h6", "Filters");
            o.open("table");
            o.open("thead");
            o.head_row(&["Filter type", "Filter value"]);
            o.close("thead");
            o.open("tbody");
            for f in filters {
                o.open("tr");
                o.el("td", av(f, "type"));
                o.el("td", text1(f));
                o.close("tr");
            }
            o.close("tbody");
            o.close("table");
            o.close("div");
        }
    }
}

/// `--convert-control` (Biber.pm:380-417): write `<path>.html`.
pub fn convert_control(log: &mut Logger, env: &Env, path: &str) -> Result<(), Fatal> {
    let bytes = match std::fs::read(env.resolve(path)) {
        Ok(b) => b,
        Err(e) => {
            biber_error!(log, "Cannot read control file '{path}': {e}")?;
            return Ok(());
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    };
    let doc = match Document::parse_with_options(&text, relaxng::parsing_options()) {
        Ok(d) => d,
        Err(e) => {
            biber_error!(log, "{path} is malformed: {e}")?;
            return Ok(());
        }
    };
    let html = render(&doc);
    let out = format!("{path}.html");
    if let Err(e) = std::fs::write(env.resolve(&out), html) {
        biber_error!(log, "Cannot write '{out}': {e}")?;
        return Ok(());
    }
    info!(log, "Converted BibLaTeX control file '{path}' to '{out}'");
    Ok(())
}

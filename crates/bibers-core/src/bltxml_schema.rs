//! `Biber::DataModel::generate_bltxml_schema` (DataModel.pm:1021): the
//! RELAX NG schema for biblatexml data sources, generated from the data
//! model.
//!
//! biber iterates three Perl hashes here (`entrytypes`, `fieldtypes`,
//! `datatypes`), so the order of the entry type `<value>`s and of the
//! per-type `<ref>`s and `<define>`s varies between runs. bibers uses the
//! data model's declaration order for entry types and sorted order for field
//! and data types (one of biber's possible outputs; DC-031 report).

use crate::datamodel::DataModel;
use crate::xmlwriter::{XName, XmlWriter};

const BLTX_NS: &str = "http://biblatex-biber.sourceforge.net/biblatexml";
const RNG_NS: &str = "http://relaxng.org/ns/structure/1.0";

/// The schema text.
pub fn generate_bltxml_schema(dm: &DataModel) -> String {
    let mut w = XmlWriter::new("2", &[(BLTX_NS, "bltx"), (RNG_NS, "")]);
    fn p(s: &str) -> XName<'_> {
        XName::Plain(s)
    }
    let e = |w: &mut XmlWriter, name: &str| w.empty_tag(p(name), &[]);
    let st = |w: &mut XmlWriter, name: &str| w.start_tag(p(name), &[]);
    let el = |w: &mut XmlWriter, name: &str| w.start_tag(p("element"), &[("name", name)]);
    let attr = |w: &mut XmlWriter, name: &str| w.start_tag(p("attribute"), &[("name", name)]);
    let eattr = |w: &mut XmlWriter, name: &str| w.empty_tag(p("attribute"), &[("name", name)]);
    let rf = |w: &mut XmlWriter, name: &str| w.empty_tag(p("ref"), &[("name", name)]);
    let data = |w: &mut XmlWriter, t: &str| w.empty_tag(p("data"), &[("type", t)]);
    let val = |w: &mut XmlWriter, v: &str| w.data_element(p("value"), v, &[]);
    let end = |w: &mut XmlWriter| w.end_tag();
    // optional attribute with a data type
    let opt_attr_data = |w: &mut XmlWriter, name: &str, t: &str| {
        st(w, "optional");
        attr(w, name);
        data(w, t);
        end(w);
        end(w);
    };
    let opt_eattr = |w: &mut XmlWriter, name: &str| {
        st(w, "optional");
        eattr(w, name);
        end(w);
    };

    w.xml_decl();
    w.comment("Auto-generated from .bcf Datamodel");
    w.force_ns_decl(RNG_NS);
    w.force_ns_decl(BLTX_NS);
    w.start_tag(
        p("grammar"),
        &[(
            "datatypeLibrary",
            "http://www.w3.org/2001/XMLSchema-datatypes",
        )],
    );
    st(&mut w, "start");
    el(&mut w, "bltx:entries");
    st(&mut w, "oneOrMore");
    el(&mut w, "bltx:entry");
    eattr(&mut w, "id");
    attr(&mut w, "entrytype");
    st(&mut w, "choice");
    for et in dm.entrytypes() {
        val(&mut w, et);
    }
    end(&mut w);
    end(&mut w);
    st(&mut w, "interleave");
    let fts: Vec<String> = dm.fieldtypes().iter().map(|s| s.to_string()).collect();
    let dts: Vec<String> = dm.datatypes().iter().map(|s| s.to_string()).collect();
    let combos: Vec<(String, String)> = fts
        .iter()
        .flat_map(|ft| dts.iter().map(move |dt| (ft.clone(), dt.clone())))
        .filter(|(ft, dt)| dm.is_fields_of_type(ft, dt, None) && dt != "datepart")
        .collect();
    for (ft, dt) in &combos {
        w.comment(&format!("{dt} {ft}s"));
        rf(&mut w, &format!("{dt}{ft}"));
    }
    rf(&mut w, "mannotation");
    end(&mut w); // interleave
    end(&mut w); // entry
    end(&mut w); // oneOrMore
    end(&mut w); // entries
    end(&mut w); // start

    for (ft, dt) in &combos {
        let fields = dm.get_fields_of_type(ft, &[dt.as_str()], None);
        w.comment(&format!("{dt} {ft}s definition"));
        w.start_tag(p("define"), &[("name", &format!("{dt}{ft}"))]);
        match (ft.as_str(), dt.as_str()) {
            ("list", "name") => {
                st(&mut w, "zeroOrMore");
                el(&mut w, "bltx:names");
                st(&mut w, "choice");
                rf(&mut w, "xdata");
                st(&mut w, "group");
                w.comment("useprefix option");
                opt_attr_data(&mut w, "useprefix", "boolean");
                w.comment("sortingnamekeytemplatename option");
                opt_attr_data(&mut w, "sortingnamekeytemplatename", "string");
                w.comment("types of names elements");
                attr(&mut w, "type");
                st(&mut w, "choice");
                for f in &fields {
                    val(&mut w, f);
                }
                end(&mut w);
                end(&mut w);
                opt_attr_data(&mut w, "morenames", "boolean");
                st(&mut w, "oneOrMore");
                el(&mut w, "bltx:name");
                st(&mut w, "choice");
                rf(&mut w, "xdata");
                st(&mut w, "group");
                w.comment("useprefix option");
                opt_attr_data(&mut w, "useprefix", "boolean");
                w.comment("sortingnamekeytemplatename option");
                opt_attr_data(&mut w, "sortingnamekeytemplatename", "string");
                rf(&mut w, "gender");
                st(&mut w, "oneOrMore");
                el(&mut w, "bltx:namepart");
                attr(&mut w, "type");
                st(&mut w, "choice");
                for np in dm.get_constant_value("nameparts") {
                    val(&mut w, &np);
                }
                end(&mut w);
                end(&mut w);
                opt_eattr(&mut w, "initial");
                st(&mut w, "choice");
                st(&mut w, "oneOrMore");
                el(&mut w, "bltx:namepart");
                opt_eattr(&mut w, "initial");
                e(&mut w, "text");
                end(&mut w); // namepart
                end(&mut w); // oneOrMore
                e(&mut w, "text");
                end(&mut w); // choice
                end(&mut w); // namepart
                end(&mut w); // oneOrMore
                end(&mut w); // group
                end(&mut w); // choice
                end(&mut w); // name
                end(&mut w); // oneOrMore
                end(&mut w); // group
                end(&mut w); // choice
                end(&mut w); // names
                end(&mut w); // zeroOrMore
            }
            ("list", _) => {
                st(&mut w, "interleave");
                for f in &fields {
                    st(&mut w, "optional");
                    el(&mut w, &format!("bltx:{f}"));
                    st(&mut w, "choice");
                    rf(&mut w, "xdata");
                    st(&mut w, "choice");
                    e(&mut w, "text");
                    el(&mut w, "bltx:list");
                    st(&mut w, "oneOrMore");
                    el(&mut w, "bltx:item");
                    st(&mut w, "choice");
                    rf(&mut w, "xdata");
                    e(&mut w, "text");
                    for _ in 0..7 {
                        end(&mut w);
                    }
                    end(&mut w);
                }
                end(&mut w);
            }
            ("field", "uri") => {
                st(&mut w, "interleave");
                for f in &fields {
                    st(&mut w, "optional");
                    el(&mut w, &format!("bltx:{f}"));
                    st(&mut w, "choice");
                    rf(&mut w, "xdata");
                    data(&mut w, "anyURI");
                    end(&mut w);
                    end(&mut w);
                    end(&mut w);
                }
                end(&mut w);
            }
            ("field", "range") => {
                st(&mut w, "interleave");
                for f in &fields {
                    st(&mut w, "optional");
                    el(&mut w, &format!("bltx:{f}"));
                    st(&mut w, "choice");
                    rf(&mut w, "xdata");
                    el(&mut w, "bltx:list");
                    st(&mut w, "oneOrMore");
                    el(&mut w, "bltx:item");
                    st(&mut w, "choice");
                    rf(&mut w, "xdata");
                    st(&mut w, "group");
                    el(&mut w, "bltx:start");
                    e(&mut w, "text");
                    end(&mut w);
                    el(&mut w, "bltx:end");
                    st(&mut w, "choice");
                    e(&mut w, "text");
                    e(&mut w, "empty");
                    end(&mut w); // choice
                    end(&mut w); // end
                    end(&mut w); // group
                    end(&mut w); // choice
                    end(&mut w); // item
                    end(&mut w); // oneOrMore
                    end(&mut w); // list
                    end(&mut w); // choice
                    end(&mut w); // field
                    end(&mut w); // optional
                }
                end(&mut w);
            }
            ("field", "entrykey") => {
                st(&mut w, "interleave");
                for f in &fields {
                    st(&mut w, "optional");
                    if f == "related" {
                        el(&mut w, &format!("bltx:{f}"));
                        el(&mut w, "bltx:list");
                        st(&mut w, "oneOrMore");
                        el(&mut w, "bltx:item");
                        eattr(&mut w, "type");
                        eattr(&mut w, "ids");
                        opt_eattr(&mut w, "string");
                        opt_eattr(&mut w, "options");
                        end(&mut w); // item
                        end(&mut w); // oneOrMore
                        end(&mut w); // list
                        end(&mut w); // field
                    } else {
                        el(&mut w, &format!("bltx:{f}"));
                        st(&mut w, "choice");
                        rf(&mut w, "xdata");
                        st(&mut w, "choice");
                        st(&mut w, "list");
                        st(&mut w, "oneOrMore");
                        data(&mut w, "string");
                        end(&mut w); // oneOrMore
                        end(&mut w); // list
                        el(&mut w, "bltx:list");
                        st(&mut w, "oneOrMore");
                        el(&mut w, "bltx:item");
                        e(&mut w, "text");
                        end(&mut w); // item
                        end(&mut w); // oneOrMore
                        end(&mut w); // list
                        end(&mut w); // choice
                        end(&mut w); // choice
                        end(&mut w); // field
                    }
                    end(&mut w); // optional
                }
                end(&mut w);
            }
            ("field", "date") => {
                st(&mut w, "zeroOrMore");
                el(&mut w, "bltx:date");
                st(&mut w, "optional");
                attr(&mut w, "type");
                st(&mut w, "choice");
                for f in &fields {
                    let t = f.strip_suffix("date").unwrap_or(f);
                    if !t.is_empty() {
                        val(&mut w, t);
                    }
                }
                end(&mut w); // choice
                end(&mut w); // attribute
                end(&mut w); // optional
                st(&mut w, "choice");
                data(&mut w, "string");
                st(&mut w, "group");
                el(&mut w, "bltx:start");
                st(&mut w, "choice");
                data(&mut w, "string");
                end(&mut w); // choice
                end(&mut w); // start
                el(&mut w, "bltx:end");
                st(&mut w, "choice");
                data(&mut w, "string");
                e(&mut w, "empty");
                end(&mut w); // choice
                end(&mut w); // end
                end(&mut w); // group
                end(&mut w); // choice
                end(&mut w); // date
                end(&mut w); // zeroOrMore
            }
            ("field", _) => {
                st(&mut w, "interleave");
                for f in &fields {
                    st(&mut w, "optional");
                    el(&mut w, &format!("bltx:{f}"));
                    st(&mut w, "choice");
                    rf(&mut w, "xdata");
                    e(&mut w, "text");
                    end(&mut w);
                    end(&mut w);
                    end(&mut w);
                }
                end(&mut w);
            }
            _ => {}
        }
        end(&mut w); // define
    }

    w.comment("xdata attribute definition");
    w.start_tag(p("define"), &[("name", "xdata")]);
    st(&mut w, "optional");
    attr(&mut w, "xdata");
    e(&mut w, "text");
    end(&mut w);
    end(&mut w);
    end(&mut w);

    w.comment("gender attribute definition");
    w.start_tag(p("define"), &[("name", "gender")]);
    st(&mut w, "optional");
    attr(&mut w, "gender");
    st(&mut w, "choice");
    for g in dm.get_constant_value("gender") {
        val(&mut w, &g);
    }
    end(&mut w);
    end(&mut w);
    end(&mut w);
    end(&mut w);

    w.comment("generic annotation element definition");
    w.start_tag(p("define"), &[("name", "mannotation")]);
    st(&mut w, "zeroOrMore");
    el(&mut w, "bltx:mannotation");
    eattr(&mut w, "field");
    opt_eattr(&mut w, "name");
    opt_eattr(&mut w, "item");
    opt_eattr(&mut w, "part");
    opt_eattr(&mut w, "literal");
    e(&mut w, "text");
    end(&mut w);
    end(&mut w);
    end(&mut w);

    end(&mut w); // grammar
    w.finish()
}

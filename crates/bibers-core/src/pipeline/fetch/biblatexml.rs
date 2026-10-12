//! `Biber::Input::file::biblatexml` (vendor/biber @ v2.22,
//! `lib/Biber/Input/file/biblatexml.pm`): reading biblatexml data sources
//! (`extract_entries`), source mapping ([`smap`]), creating entries
//! (`create_entry`) and the field handlers.
//!
//! The XML is held in a mutable [`Dom`] and queried with XPath exactly as
//! biber queries it through `XML::LibXML`: element names are matched by
//! their *source* prefix (`nodeName`, normalised by stripping a literal
//! `bltx:`), and XPath name tests resolve prefixes against the namespace
//! declarations in scope at the context node (biber's document-level
//! `XPathContext` additionally registers `bltx`).
//!
//! # Where biber dies
//!
//! Several inputs make biber die with a Perl or libxml2 error (malformed
//! XML, an XPath error, a field node that its own XPath cannot find again,
//! an invalid date, a data-model field type without a biblatexml handler).
//! bibers reports these as an `ERROR` naming the data source and stops
//! (exit 2), or, where the code's intent is clear, does what was intended
//! (an invalid date is warned about and ignored; a field type with no
//! handler is warned about and ignored). These are crash fixes (proposed
//! compat entries in the DC-031 report).

pub mod smap;

use super::bibtex::{XRef, add_xdata_ref, process_entry_options, warn_entry};
use super::dates::{self, DateParser, Parsed};
use super::names::{gen_initials, join_name_parts};
use super::{FetchOpts, SecExt, encoding};
use crate::config::blx::{Scope, expand_option_input};
use crate::log::Fatal;
use crate::model::{AnnotationScope, Entry, Name, NamePart, SectionState, Value};
use crate::perlish::{fc, lc, nfd, split_comma};
use crate::pipeline::Ctx;
use crate::quirks::Quirk;
use crate::xmldom::xpath::{XPath, XPathError, XValue};
use crate::xmldom::{Dom, NodeId};
use crate::{debug, info};
use serde_json::Value as J;
use std::collections::HashMap;

/// The biblatexml namespace URI.
pub const BLTX_NS: &str = "http://biblatex-biber.sourceforge.net/biblatexml";
/// The prefix biber registers for it (and strips from node names).
pub const BLTX: &str = "bltx";

/// Run-level state of the biblatexml reader (module globals in biber).
#[derive(Clone, Debug, Default)]
pub struct BltxmlRunState {
    /// `$orig_key_order` (biblatexml.pm:33): keys per data file, appended to
    /// on every allkeys read (never reset).
    pub orig_key_order: HashMap<String, Vec<String>>,
    /// `$dm->{bltxml_schema_gen_done}`: the `.rng` has been written.
    pub schema_done: bool,
}

/// `die` in biber: log an ERROR and stop (exit 2), whatever
/// `--nodieonerror` says.
pub(super) fn die(ctx: &mut Ctx, msg: &str) -> Fatal {
    ctx.log.biber_error_nodie(crate::loc!(), msg);
    Fatal
}

/// `_norm` (biblatexml.pm:1430): lowercase and strip a leading `bltx:`.
pub fn norm(name: &str) -> String {
    let l = lc(name);
    match l.strip_prefix("bltx:") {
        Some(s) => s.to_string(),
        None => l,
    }
}

/// The namespace bindings an XPath sees from `node` (`$node->findnodes`).
fn node_ns(dom: &Dom, node: NodeId) -> Vec<(Option<String>, String)> {
    let at = if dom.is_attribute(node) {
        dom.parent(node).unwrap_or(node)
    } else if node == dom.document() || dom.parent(node).is_none() && !dom.is_element(node) {
        dom.element_children(node).next().unwrap_or(node)
    } else {
        node
    };
    dom.in_scope_ns(at)
}

/// The bindings of biber's document `XPathContext` (`bltx` registered
/// first, then those in scope at the root element).
fn xpc_ns(dom: &Dom) -> Vec<(Option<String>, String)> {
    let mut ns = vec![(Some(BLTX.to_string()), BLTX_NS.to_string())];
    ns.extend(node_ns(dom, dom.document()));
    ns
}

/// Evaluate an XPath to a node list (`findnodes`); non-node-set results
/// give an empty list.
pub(super) fn eval_nodes(
    dom: &Dom,
    node: NodeId,
    xp: &XPath,
    ns: &[(Option<String>, String)],
) -> Result<Vec<NodeId>, XPathError> {
    Ok(match xp.eval(dom, node, ns)? {
        XValue::Nodes(v) => v,
        _ => Vec::new(),
    })
}

/// `$node->findnodes($xpath)`, dying (biber croaks) on an XPath error.
pub(super) fn findnodes(
    ctx: &mut Ctx,
    dom: &Dom,
    node: NodeId,
    xpath: &str,
    file: &str,
) -> Result<Vec<NodeId>, Fatal> {
    let res = XPath::compile(xpath).and_then(|x| eval_nodes(dom, node, &x, &node_ns(dom, node)));
    res.map_err(|e| die(ctx, &xpath_die_msg(file, xpath, &e)))
}

pub(super) fn xpath_die_msg(file: &str, xpath: &str, e: &XPathError) -> String {
    format!(
        "BibLaTeXML data source '{file}': XPath error ({}) evaluating '{xpath}'",
        e.libxml_message()
    )
}

/// `$node->exists($xpath)`: an evaluation error makes libxml2 return -1,
/// which `XML::LibXML` turns into a true boolean (after printing the error
/// on stderr). Compilation errors are reported by the caller.
pub(super) fn exists(dom: &Dom, node: NodeId, xp: &XPath) -> bool {
    match eval_nodes(dom, node, xp, &node_ns(dom, node)) {
        Ok(v) => !v.is_empty(),
        Err(e) => {
            eprintln!("XPath error : {}", e.libxml_message());
            true
        }
    }
}

/// Per-`extract_entries` values.
struct Src<'a> {
    filename: &'a str,
    secnum: String,
}

/// `extract_entries` (biblatexml.pm:82). Returns the keys not found.
pub fn extract_entries(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    filename: &str,
    keys: &[String],
) -> Result<Vec<String>, Fatal> {
    let secnum = st.section.number.clone();
    let mut rkeys: Vec<String> = keys.to_vec();
    let path = ctx.env.resolve(filename);
    let bytes = std::fs::read(&path).unwrap_or_default();
    if bytes.is_empty() {
        ctx.log
            .biber_warn(format!("Data source '{filename}' is empty, ignoring"));
        return Ok(rkeys);
    }
    info!(ctx.log, "Found BibLaTeXML data file '{filename}'");
    let text = match encoding::decode(&bytes, "UTF-8") {
        Ok(t) => t,
        Err(e) => {
            return Err(die(
                ctx,
                &format!("Data file '{filename}' cannot be read in encoding 'UTF-8': {e}"),
            ));
        }
    };
    let mut dom = match Dom::parse(&nfd(&text)) {
        Ok(d) => d,
        Err(e) => {
            // biber dies with libxml2's parser error
            return Err(die(
                ctx,
                &format!("BibLaTeXML data source '{filename}' is not well-formed XML: {e}"),
            ));
        }
    };
    let src = Src {
        filename,
        secnum: secnum.clone(),
    };
    let xpc = xpc_ns(&dom);
    let doc = dom.document();
    let all_entries = XPath::compile("/bltx:entries/bltx:entry").expect("valid XPath");

    if st.section.allkeys {
        debug!(ctx.log, "All citekeys will be used for section '{secnum}'");
        let entries = eval_nodes(&dom, doc, &all_entries, &xpc)
            .map_err(|e| die(ctx, &xpath_die_msg(filename, all_entries.as_str(), &e)))?;
        for entry in entries {
            debug!(
                ctx.log,
                "Parsing BibLaTeXML entry object {}",
                dom.node_path(entry)
            );
            if !dom.has_attribute(entry, "id") {
                ctx.log.biber_warn(format!(
                    "Invalid or undefined BibLaTeXML entry key in file '{filename}', skipping ..."
                ));
                continue;
            }
            let key = dom.get_attribute(entry, "id").unwrap_or("").to_string();
            if st.section.citekey_alias.contains_key(&key) {
                ctx.log.biber_warn(format!(
                    "Citekey alias '{key}' is also a real entry key, skipping ..."
                ));
            }
            for id in findnodes(ctx, &dom, entry, "./bltx:ids/bltx:key", filename)? {
                let idstr = dom.text_content(id);
                if st.section.has_everykey(&idstr) {
                    ctx.log.biber_warn(format!(
                        "Citekey alias '{idstr}' is also a real entry key, skipping ..."
                    ));
                    continue;
                }
                match st
                    .section
                    .citekey_alias
                    .get(&idstr)
                    .filter(|o| perl_compat::perl_truthy(o))
                    .cloned()
                {
                    Some(other) => {
                        if other != key {
                            ctx.log.biber_warn(format!(
                                "Citekey alias '{idstr}' already has an alias '{other}', skipping ..."
                            ));
                        }
                    }
                    None => {
                        debug!(ctx.log, "Citekey '{idstr}' is an alias for citekey '{key}'");
                        st.section.citekey_alias.insert(idstr, key.clone());
                    }
                }
            }
            if let Some(okey) = st.section.has_badcasekey(&key) {
                // "skipping" but biber does not skip
                ctx.log.biber_warn(format!(
                    "Possible typo (case mismatch): '{key}' and '{okey}' in file '{filename}', skipping '{key}' ..."
                ));
            }
            if st.section.has_everykey(&key) {
                ctx.log.biber_warn(format!(
                    "Duplicate entry key: '{key}' in file '{filename}', skipping ..."
                ));
                continue;
            }
            st.section.add_everykey(&key);
            st.section.set_keytods(&key, filename);
            // `map_entry_null`: biber still adds the key to the citekeys (and
            // then dies in get_dependents on the missing entry); it is
            // dropped instead
            if !create_entry(ctx, st, opts, &src, &mut dom, &key, entry, true, &mut rkeys)? {
                continue;
            }
            ctx.run
                .bltxml
                .orig_key_order
                .entry(filename.to_string())
                .or_default()
                .push(key);
        }
        let order = ctx
            .run
            .bltxml
            .orig_key_order
            .get(filename)
            .cloned()
            .unwrap_or_default();
        st.section.add_citekeys(&order);
        debug!(
            ctx.log,
            "Added all citekeys to section '{secnum}': {}",
            st.section.citekeys.join(", ")
        );
    } else {
        debug!(ctx.log, "Wanted keys: {}", keys.join(", "));
        for wanted in keys {
            debug!(
                ctx.log,
                "Looking for key '{wanted}' in BibLaTeXML file '{filename}'"
            );
            // biber interpolates the key into the XPath (a key containing
            // `'` makes it die); compare the attribute directly.
            let entries: Vec<NodeId> = eval_nodes(&dom, doc, &all_entries, &xpc)
                .unwrap_or_default()
                .into_iter()
                .filter(|e| dom.get_attribute(*e, "id") == Some(wanted.as_str()))
                .collect();
            if let Some(&entry) = entries.first() {
                if entries.len() > 1 {
                    let ids: Vec<&str> = entries
                        .iter()
                        .map(|e| dom.get_attribute(*e, "id").unwrap_or(""))
                        .collect();
                    ctx.log.biber_warn(format!(
                        "Found more than one entry for key '{wanted}' in '{filename}': {} - skipping duplicates ...",
                        ids.join(",")
                    ));
                }
                debug!(
                    ctx.log,
                    "Found key '{wanted}' in BibLaTeXML file '{filename}'"
                );
                if !st.section.entries.contains_key(wanted) {
                    st.section.set_keytods(wanted, filename);
                    if !create_entry(
                        ctx, st, opts, &src, &mut dom, wanted, entry, true, &mut rkeys,
                    )? {
                        // biber keeps the citekey and dies later on the
                        // missing entry; drop it as the bibtex reader does
                        st.section.del_citekey(wanted);
                        ctx.log.biber_warn(format!(
                            "Entry with key '{wanted}' in section '{secnum}' is cited and found but not created (likely due to sourcemap)"
                        ));
                    }
                }
                rkeys.retain(|k| k != wanted);
            } else if let Some(alias_of) = id_alias(&dom, wanted) {
                // biblatexml.pm:250-268: an entry with a `<bltx:id>` child
                // naming the wanted key. biber takes the string value of *all*
                // entry ids as the real key and then dies on an invalid XPath
                // unless that "key" already exists; bibers creates the entry
                // holding the `<bltx:id>`.
                let key = all_ids(&dom);
                debug!(
                    ctx.log,
                    "Citekey '{wanted}' is an alias for citekey '{key}'"
                );
                st.section.citekey_alias.insert(wanted.clone(), key.clone());
                if !st.section.entries.contains_key(&key) {
                    st.section
                        .citekey_alias
                        .insert(wanted.clone(), alias_of.1.clone());
                    if !st.section.entries.contains_key(&alias_of.1) {
                        st.section.set_keytods(&alias_of.1, filename);
                        let k = alias_of.1.clone();
                        if create_entry(
                            ctx, st, opts, &src, &mut dom, &k, alias_of.0, true, &mut rkeys,
                        )? {
                            st.section.add_citekeys(std::slice::from_ref(&k));
                        }
                    }
                }
                rkeys.retain(|k| k != wanted);
            }
            debug!(ctx.log, "Wanted keys now: {}", rkeys.join(", "));
        }
    }
    Ok(rkeys)
}

/// The entry with a `<bltx:id>` child whose text is `wanted`, and its id.
fn id_alias(dom: &Dom, wanted: &str) -> Option<(NodeId, String)> {
    let xp = XPath::compile("/bltx:entries/bltx:entry[bltx:id]").expect("valid XPath");
    let es = eval_nodes(dom, dom.document(), &xp, &xpc_ns(dom)).ok()?;
    es.into_iter().find_map(|e| {
        dom.element_children(e)
            .any(|c| {
                dom.name(c)
                    .is_some_and(|q| q.local == "id" && q.ns.as_deref() == Some(BLTX_NS))
                    && dom
                        .children(c)
                        .iter()
                        .any(|t| dom.is_text(*t) && dom.text_content(*t) == wanted)
            })
            .then(|| (e, dom.get_attribute(e, "id").unwrap_or("").to_string()))
    })
}

/// `$xpc->findnodes("/bltx:entries/bltx:entry/@id")` stringified.
fn all_ids(dom: &Dom) -> String {
    let xp = XPath::compile("/bltx:entries/bltx:entry/@id").expect("valid XPath");
    eval_nodes(dom, dom.document(), &xp, &xpc_ns(dom))
        .unwrap_or_default()
        .iter()
        .map(|n| dom.text_content(*n))
        .collect()
}

/// `create_entry` (biblatexml.pm:290): apply the source maps (unless
/// `with_maps` is false: biber's recursive call for `map_entry_clone`),
/// then create the entry and any entries the maps created.
#[allow(clippy::too_many_arguments)]
fn create_entry(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    src: &Src,
    dom: &mut Dom,
    key: &str,
    entry: NodeId,
    with_maps: bool,
    rkeys: &mut Vec<String>,
) -> Result<bool, Fatal> {
    let news = if with_maps {
        match smap::apply(ctx, st, opts, src_ref(src), dom, key, entry, rkeys)? {
            Some(n) => n,
            None => return Ok(false),
        }
    } else {
        Vec::new()
    };
    instantiate(ctx, st, opts, src_ref(src), dom, entry, entry)?;
    for e in news {
        instantiate(ctx, st, opts, src_ref(src), dom, e, entry)?;
    }
    Ok(true)
}

/// The data source name and section, for the map engine.
#[derive(Clone, Copy)]
pub struct SrcRef<'a> {
    pub filename: &'a str,
    pub secnum: &'a str,
}

fn src_ref<'a>(s: &'a Src) -> SrcRef<'a> {
    SrcRef {
        filename: s.filename,
        secnum: &s.secnum,
    }
}

/// The entry-creation half of `create_entry` (biblatexml.pm:818-851) for
/// one XML entry `e`; `orig` is the entry being mapped (biber reads the
/// `options` node from it, not from `e`).
pub(super) fn instantiate(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &mut Dom,
    e: NodeId,
    orig: NodeId,
) -> Result<(), Fatal> {
    let k = dom.get_attribute(e, "id").unwrap_or("").to_string();
    let mut be = Entry::default();
    be.set_field("citekey", Some(Value::Str(k.clone())));
    debug!(ctx.log, "Creating entry with key '{k}'");
    be.set_field(
        "entrytype",
        dom.get_attribute(e, "entrytype")
            .map(|t| Value::Str(t.to_string())),
    );
    // uniq map { names ? @type : nodeName } $e->findnodes('*')
    let mut fields: Vec<Option<String>> = Vec::new();
    for c in dom.element_children(e).collect::<Vec<_>>() {
        let nn = dom.node_name(c);
        let f = if norm(&nn) == "names" {
            dom.get_attribute(c, "type").map(str::to_string)
        } else {
            Some(nn)
        };
        if !fields.contains(&f) {
            fields.push(f);
        }
    }
    for f in fields {
        let f = f.unwrap_or_default();
        let nf = norm(&f);
        if nf == "options"
            && let Some(node) = findnodes(ctx, dom, orig, "./bltx:options", src.filename)?
                .first()
                .copied()
        {
            let list = split_comma(&dom.text_content(node));
            process_entry_options(ctx, &k, &list, src.secnum);
        }
        if ctx
            .dm
            .is_field(&nf, &opts.annotation_marker, &opts.named_annotation_marker)
        {
            run_handler(ctx, st, opts, src, dom, &mut be, e, &f, &nf, &k)?;
        }
    }
    be.set_field("datatype", Some(Value::Str("biblatexml".into())));
    st.section.entries.insert(k, be);
    Ok(())
}

/// `_get_handler` (biblatexml.pm:1437) and the call.
#[allow(clippy::too_many_arguments)]
fn run_handler(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &mut Dom,
    be: &mut Entry,
    e: NodeId,
    f: &str,
    nf: &str,
    key: &str,
) -> Result<(), Fatal> {
    match nf {
        "related" => return related(ctx, opts, src, dom, be, e, f),
        "annotation" => return annotation(ctx, src, dom, be, e, f, nf, key),
        _ => {}
    }
    let ft = ctx.dm.get_fieldtype(nf).unwrap_or("").to_string();
    let fmt = ctx
        .dm
        .get_fieldformat(nf)
        .filter(|f| perl_compat::perl_truthy(f))
        .unwrap_or("default")
        .to_string();
    let dt = ctx.dm.get_datatype(nf).unwrap_or("").to_string();
    match (ft.as_str(), fmt.as_str(), dt.as_str()) {
        ("field", "default", "code" | "entrykey" | "integer" | "key" | "literal" | "verbatim") => {
            literal(ctx, opts, src, dom, be, e, f, nf)
        }
        ("field", "default", "date") => datetime(ctx, st, opts, src, dom, be, e, f, key),
        ("field", "default", "range") => range(ctx, opts, src, dom, be, e, f, nf),
        ("field", "default", "uri") => uri(ctx, opts, src, dom, be, e, f, nf),
        ("field", "xsv", "entrykey" | "keyword" | "option") => {
            xsv(ctx, opts, src, dom, be, e, f, nf)
        }
        ("list", "default", "entrykey" | "key" | "literal") => {
            let node = field_node(ctx, dom, e, f, src)?;
            let v = split_list(ctx, opts, src, dom, be, node, nf, false);
            be.set_datafield(nf, Some(Value::List(v)));
            Ok(())
        }
        ("list", "default", "name") => name(ctx, st, opts, src, dom, be, e, f, nf, key),
        _ => {
            // biber dies: "Can't use an undefined value as a subroutine
            // reference" (no handler for this field type in biblatexml).
            let bee = be.entrytype().to_string();
            let ds = st.section.get_keytods(key).unwrap_or_default();
            warn_entry(
                ctx,
                be,
                format!(
                    "Datamodel: {bee} entry '{key}' ({ds}): Field '{nf}' (fieldtype '{ft}', format '{fmt}', datatype '{dt}') cannot be given in BibLaTeXML data - ignoring"
                ),
            );
            Ok(())
        }
    }
}

/// `$entry->findnodes("./$f")->get_node(1)`; biber dies calling a method on
/// the `undef` it gets when the node cannot be found again (e.g. a field
/// element without the `bltx` prefix).
fn field_node(ctx: &mut Ctx, dom: &Dom, e: NodeId, f: &str, src: SrcRef) -> Result<NodeId, Fatal> {
    let xp = format!("./{f}");
    match findnodes(ctx, dom, e, &xp, src.filename)?.first() {
        Some(n) => Ok(*n),
        None => Err(die(
            ctx,
            &format!(
                "BibLaTeXML data source '{}': cannot read field element '{f}' of entry '{}' (field elements must use the '{BLTX}' namespace prefix)",
                src.filename,
                dom.get_attribute(e, "id").unwrap_or("")
            ),
        )),
    }
}

/// `"$xdatamarker$xnamesep$xdatav"`.
fn xdata_ref(opts: &FetchOpts, v: &str) -> String {
    format!("{}{}{v}", opts.xdatamarker, opts.xnamesep)
}

fn truthy_attr<'d>(dom: &'d Dom, n: NodeId, a: &str) -> Option<&'d str> {
    dom.get_attribute(n, a)
        .filter(|v| perl_compat::perl_truthy(v))
}

/// `_literal` (biblatexml.pm:899).
#[allow(clippy::too_many_arguments)]
fn literal(
    ctx: &mut Ctx,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &Dom,
    be: &mut Entry,
    e: NodeId,
    f: &str,
    nf: &str,
) -> Result<(), Fatal> {
    let node = field_node(ctx, dom, e, f, src)?;
    let mut setval = dom.text_content(node);
    if let Some(x) = truthy_attr(dom, node, "xdata") {
        let xdatav = xdata_ref(opts, x);
        add_xdata_ref(ctx, opts, be, nf, XRef::Str(&xdatav), None, src.secnum);
        setval = xdatav;
    }
    // BB-050: `fc(_norm($f) eq "$NS:eprint")` is never true, so the type and
    // class attributes of `eprint` are ignored.
    if !ctx.cfg.quirks.on(Quirk::BltxmlEprintAttrs) && fc(nf) == "eprint" {
        be.set_datafield(
            "eprinttype",
            dom.get_attribute(node, "type")
                .map(|t| Value::Str(t.to_string())),
        );
        if let Some(ec) = truthy_attr(dom, node, "class") {
            be.set_datafield("eprintclass", Some(Value::Str(ec.to_string())));
        }
    }
    be.set_datafield(nf, Some(Value::Str(setval)));
    Ok(())
}

/// `_xsv` (biblatexml.pm:928).
#[allow(clippy::too_many_arguments)]
fn xsv(
    ctx: &mut Ctx,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &Dom,
    be: &mut Entry,
    e: NodeId,
    f: &str,
    nf: &str,
) -> Result<(), Fatal> {
    let node = field_node(ctx, dom, e, f, src)?;
    if fc(nf) == "xdata" {
        let value = split_list(ctx, opts, src, dom, be, node, nf, true);
        add_xdata_ref(ctx, opts, be, "xdata", XRef::List(&value), None, src.secnum);
        be.set_datafield(nf, Some(Value::List(value)));
    } else {
        let value = split_list(ctx, opts, src, dom, be, node, nf, false);
        be.set_datafield(nf, Some(Value::List(value)));
    }
    Ok(())
}

/// `_uri` (biblatexml.pm:947).
#[allow(clippy::too_many_arguments)]
fn uri(
    ctx: &mut Ctx,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &Dom,
    be: &mut Entry,
    e: NodeId,
    f: &str,
    nf: &str,
) -> Result<(), Fatal> {
    let node = field_node(ctx, dom, e, f, src)?;
    let mut setval = dom.text_content(node);
    if let Some(x) = truthy_attr(dom, node, "xdata") {
        let xdatav = xdata_ref(opts, x);
        add_xdata_ref(ctx, opts, be, nf, XRef::Str(&xdatav), None, src.secnum);
        setval = xdatav;
    } else if !setval.contains('%') && ctx.cfg.quirks.on(Quirk::BltxmlUriEscape) {
        // BB-051: escaped here, from NFD text (the .bbl writer escapes
        // bibtex URIs from NFC text, and leaves `urlraw` alone).
        match perl_compat::uri::uri_as_string(&setval) {
            Ok(u) => setval = u,
            Err(e) => return Err(die(ctx, &e.to_string())),
        }
    }
    be.set_datafield(nf, Some(Value::Str(setval)));
    Ok(())
}

/// `_range` (biblatexml.pm:990).
#[allow(clippy::too_many_arguments)]
fn range(
    ctx: &mut Ctx,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &Dom,
    be: &mut Entry,
    e: NodeId,
    f: &str,
    nf: &str,
) -> Result<(), Fatal> {
    let node = field_node(ctx, dom, e, f, src)?;
    if let Some(x) = truthy_attr(dom, node, "xdata") {
        let xdatav = xdata_ref(opts, x);
        add_xdata_ref(ctx, opts, be, nf, XRef::Str(&xdatav), None, src.secnum);
        be.set_datafield(nf, Some(Value::List(vec![xdatav])));
        return Ok(());
    }
    let items = findnodes(ctx, dom, node, "./bltx:list/bltx:item", src.filename)?;
    if !items.is_empty() {
        let mut rl = Vec::new();
        for item in items {
            let first = |ctx: &mut Ctx, p: &str| -> Result<String, Fatal> {
                Ok(findnodes(ctx, dom, item, p, src.filename)?
                    .first()
                    .map(|n| dom.text_content(*n))
                    .unwrap_or_default())
            };
            let start = first(ctx, "./bltx:start")?;
            let end = first(ctx, "./bltx:end")?;
            rl.push(vec![Some(start), Some(end)]);
        }
        be.set_datafield(nf, Some(Value::Ranges(rl)));
    }
    Ok(())
}

/// `_split_list` (biblatexml.pm:1401).
#[allow(clippy::too_many_arguments)]
fn split_list(
    ctx: &mut Ctx,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &Dom,
    be: &mut Entry,
    node: NodeId,
    nf: &str,
    noxdata: bool,
) -> Vec<String> {
    let items = XPath::compile("./bltx:list/bltx:item")
        .ok()
        .and_then(|x| eval_nodes(dom, node, &x, &node_ns(dom, node)).ok())
        .unwrap_or_default();
    if items.is_empty() {
        return vec![dom.text_content(node)];
    }
    let mut out = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if let Some(x) = truthy_attr(dom, item, "xdata") {
            let xdatav = xdata_ref(opts, x);
            if !noxdata {
                add_xdata_ref(ctx, opts, be, nf, XRef::Str(&xdatav), Some(i), src.secnum);
            }
            out.push(xdatav);
        } else {
            out.push(dom.text_content(item));
        }
    }
    out
}

/// `_annotation` (biblatexml.pm:855).
#[allow(clippy::too_many_arguments)]
fn annotation(
    ctx: &mut Ctx,
    src: SrcRef,
    dom: &Dom,
    be: &mut Entry,
    e: NodeId,
    f: &str,
    nf: &str,
    key: &str,
) -> Result<(), Fatal> {
    for node in findnodes(ctx, dom, e, &format!("./{f}"), src.filename)? {
        let field = dom.get_attribute(node, "field");
        let name = truthy_attr(dom, node, "name")
            .unwrap_or("default")
            .to_string();
        // `literal` is stored as given in biber ('0' by default); bibers
        // keeps a flag (any true value is a literal annotation).
        let literal = truthy_attr(dom, node, "literal").is_some();
        let ann = dom.text_content(node);
        let item = dom.get_attribute(node, "item");
        let part = dom.get_attribute(node, "part");
        match field.filter(|f| perl_compat::perl_truthy(f)) {
            Some(field) => {
                let scope = if let Some(p) = part.filter(|p| perl_compat::perl_truthy(p)) {
                    AnnotationScope::Part {
                        count: item.unwrap_or("").to_string(),
                        part: p.to_string(),
                    }
                } else if let Some(i) = item.filter(|i| perl_compat::perl_truthy(i)) {
                    AnnotationScope::Item {
                        count: i.to_string(),
                    }
                } else {
                    AnnotationScope::Field
                };
                ctx.run
                    .annotations
                    .set_annotation(scope, key, field, &name, Some(ann), literal);
            }
            None => be.set_datafield(nf, Some(Value::Str(dom.text_content(node)))),
        }
    }
    Ok(())
}

/// `_related` (biblatexml.pm:883).
fn related(
    ctx: &mut Ctx,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &Dom,
    be: &mut Entry,
    e: NodeId,
    f: &str,
) -> Result<(), Fatal> {
    let node = field_node(ctx, dom, e, f, src)?;
    for item in findnodes(ctx, dom, node, "./bltx:list/bltx:item", src.filename)? {
        let split = |s: Option<&str>| match s {
            Some(s) => opts.split_xsv(s),
            None => Vec::new(),
        };
        be.set_datafield(
            "related",
            Some(Value::List(split(dom.get_attribute(item, "ids")))),
        );
        be.set_datafield(
            "relatedtype",
            dom.get_attribute(item, "type")
                .map(|t| Value::Str(t.to_string())),
        );
        if let Some(s) = truthy_attr(dom, item, "string") {
            be.set_datafield("relatedstring", Some(Value::Str(s.to_string())));
        }
        if let Some(o) = truthy_attr(dom, item, "options") {
            // BB-new: the value is read from a `relatedoptions` attribute,
            // so the list is empty.
            let v = if ctx.cfg.quirks.on(Quirk::BltxmlRelatedoptions) {
                split(dom.get_attribute(item, "relatedoptions"))
            } else {
                split(Some(o))
            };
            be.set_datafield("relatedoptions", Some(Value::List(v)));
        }
    }
    Ok(())
}

/// `_datetime` (biblatexml.pm:1018).
#[allow(clippy::too_many_arguments)]
fn datetime(
    ctx: &mut Ctx,
    st: &SectionState,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &Dom,
    be: &mut Entry,
    e: NodeId,
    f: &str,
    key: &str,
) -> Result<(), Fatal> {
    let ds = st.section.get_keytods(key).unwrap_or_default();
    let bee = be.entrytype().to_string();
    let s1 = |v: &str| Some(Value::Str(v.to_string()));
    for node in findnodes(ctx, dom, e, &format!("./{f}"), src.filename)? {
        let dt = dom.get_attribute(node, "type").unwrap_or("").to_string();
        be.set_field(format!("{dt}datesplit"), s1("1"));
        let start = findnodes(ctx, dom, node, "./bltx:start", src.filename)?;
        let (text, is_range) = match start.first() {
            Some(s) => (dom.text_content(*s), true),
            None => (dom.text_content(node), false),
        };
        // parse_date_range: always "true" in biber (a 4-element list)
        let mut sp = DateParser::default();
        let mut ep = DateParser::default();
        let r = dates::parse_date_range(&mut sp, &mut ep, &text, opts.julian);
        if let Some(r) = &r {
            if r.start_unknown {
                be.set_field(format!("{dt}dateunknown"), s1("1"));
            }
            if r.end_unknown {
                be.set_field(format!("{dt}enddateunknown"), s1("1"));
            }
        }
        match r.as_ref().map(|r| &r.start) {
            Some(Parsed::Date(sd)) => {
                if sp.julian {
                    be.set_field(format!("{dt}datejulian"), s1("1"));
                }
                if sp.approximate {
                    be.set_field(format!("{dt}dateapproximate"), s1("1"));
                }
                if sp.uncertain {
                    be.set_field(format!("{dt}dateuncertain"), s1("1"));
                }
                if let Some(u) = r.as_ref().and_then(|r| r.unspec) {
                    be.set_field(format!("{dt}dateunspecified"), s1(u));
                }
                set_date_parts(be, &dt, "", sd, &sp);
            }
            _ => {
                // biber dies calling ->year on undef/0 (the "Invalid format"
                // branch is unreachable): warn as the code intends.
                let msg = if is_range {
                    format!(
                        "Datamodel: {bee} entry '{key}' ({ds}): Invalid format '{text}' of date field '{f}' range start - ignoring"
                    )
                } else {
                    format!(
                        "{bee} entry '{key}' ({ds}): Invalid format '{text}' of date field '{f}' - ignoring"
                    )
                };
                warn_entry(ctx, be, msg);
            }
        }
        if !is_range {
            continue;
        }
        let end = findnodes(ctx, dom, node, "./bltx:end", src.filename)?;
        let etext = end.first().map(|n| dom.text_content(*n));
        let mut ep = DateParser::default();
        match dates::parse_date(&mut ep, etext.as_deref(), opts.julian) {
            Parsed::Date(ed) => {
                if ep.julian {
                    be.set_field(format!("{dt}enddatejulian"), s1("1"));
                }
                if ep.approximate {
                    be.set_field(format!("{dt}enddateapproximate"), s1("1"));
                }
                if ep.uncertain {
                    be.set_field(format!("{dt}enddateuncertain"), s1("1"));
                }
                set_date_parts(be, &dt, "end", &ed, &ep);
            }
            Parsed::Zero => be.set_datafield(format!("{dt}endyear"), s1("")),
            Parsed::Undef => {
                let et = etext.unwrap_or_default();
                warn_entry(
                    ctx,
                    be,
                    format!(
                        "{bee} entry '{key}' ({ds}): Invalid format '{et}' of date field '{f}' range end - ignoring"
                    ),
                );
            }
        }
    }
    Ok(())
}

/// The year/era/month/day/yeardivision/time fields of one date end
/// (`which` is `""` or `"end"`).
fn set_date_parts(be: &mut Entry, dt: &str, which: &str, d: &dates::Dt, p: &DateParser) {
    let s1 = |v: String| Some(Value::Str(v));
    be.set_datafield(format!("{dt}{which}year"), s1(d.year.to_string()));
    be.set_field(format!("{dt}{which}era"), s1(d.era().to_string()));
    if !p.missing("month") {
        be.set_datafield(format!("{dt}{which}month"), s1(d.month.to_string()));
    }
    if !p.missing("day") {
        be.set_datafield(format!("{dt}{which}day"), s1(d.day.to_string()));
    }
    if let Some(yd) = p.yeardivision {
        be.set_field(format!("{dt}{which}yeardivision"), s1(yd.to_string()));
    }
    if !p.missing("time") {
        be.set_datafield(format!("{dt}{which}hour"), s1(d.hour.to_string()));
        be.set_datafield(format!("{dt}{which}minute"), s1(d.minute.to_string()));
        be.set_datafield(format!("{dt}{which}second"), s1(d.second.to_string()));
        if let Some(tz) = &d.tz {
            be.set_datafield(format!("{dt}{which}timezone"), s1(dates::tzformat(tz)));
        }
    }
}

/// `_name` (biblatexml.pm:1198).
#[allow(clippy::too_many_arguments)]
fn name(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &Dom,
    be: &mut Entry,
    e: NodeId,
    f: &str,
    nf: &str,
    key: &str,
) -> Result<(), Fatal> {
    // "./bltx:names[@type='$f']": compare the attribute directly (a quote in
    // the type would make biber's XPath invalid)
    let names_xp = XPath::compile("./bltx:names").expect("valid XPath");
    let node = eval_nodes(dom, e, &names_xp, &node_ns(dom, e))
        .map_err(|x| die(ctx, &xpath_die_msg(src.filename, names_xp.as_str(), &x)))?
        .into_iter()
        .find(|n| dom.get_attribute(*n, "type") == Some(f));
    let Some(node) = node else {
        return Err(die(
            ctx,
            &format!(
                "BibLaTeXML data source '{}': cannot read the '{f}' names of entry '{key}'",
                src.filename
            ),
        ));
    };
    let listid = st.section.names.new_list();
    st.section.names.list_mut(listid).type_ = Some(f.to_string());
    let reg = &ctx.cfg.blx.registry;
    for nlo in reg.options_in_scope(Scope::NameList).to_vec() {
        if let Some(v) = dom.get_attribute(node, &nlo) {
            let input = reg
                .decl(&nlo, Scope::NameList)
                .and_then(|d| d.input.clone());
            for (o, val) in expand_option_input(reg, &nlo, Some(v), input.as_ref()) {
                st.section
                    .names
                    .list_mut(listid)
                    .slots
                    .insert(o, val.map_or(J::Null, J::from));
            }
        }
    }
    let name_nodes = findnodes(ctx, dom, node, "./bltx:name", src.filename)?;
    let mut ids = Vec::new();
    for (i, nn) in name_nodes.into_iter().enumerate() {
        if let Some(x) = truthy_attr(dom, nn, "xdata") {
            let xdatav = xdata_ref(opts, x);
            if add_xdata_ref(ctx, opts, be, nf, XRef::Str(&xdatav), Some(i), src.secnum) {
                let mut n = Name::default();
                n.slots.insert("xdata".into(), J::from(xdatav));
                n.index = Some((ids.len() + 1).to_string());
                ids.push(st.section.names.new_name(n));
                continue;
            }
        }
        let mut n = parsename(ctx, st, opts, src, dom, nn)?;
        n.index = Some((ids.len() + 1).to_string());
        ids.push(st.section.names.new_name(n));
    }
    if truthy_attr(dom, node, "morenames").is_some() {
        st.section
            .names
            .list_mut(listid)
            .slots
            .insert("morenames".into(), J::from("1"));
    }
    st.section.names.list_mut(listid).names = ids;
    be.set_datafield(nf, Some(Value::Names(listid)));
    Ok(())
}

/// `parsename` (biblatexml.pm:1268).
fn parsename(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &Dom,
    node: NodeId,
) -> Result<Name, Fatal> {
    debug!(
        ctx.log,
        "Parsing BibLaTeXML name object {}",
        dom.node_path(node)
    );
    let parts_xp = XPath::compile("./bltx:namepart").expect("valid XPath");
    let nps = eval_nodes(dom, node, &parts_xp, &node_ns(dom, node))
        .map_err(|x| die(ctx, &xpath_die_msg(src.filename, parts_xp.as_str(), &x)))?;
    let mut name = Name::default();
    for np in &opts.nameparts {
        let mut part = NamePart::default();
        // ./bltx:namepart[@type='$n'] (first)
        if let Some(&npnode) = nps
            .iter()
            .find(|p| dom.get_attribute(**p, "type") == Some(np.as_str()))
        {
            let subs = eval_nodes(dom, npnode, &parts_xp, &node_ns(dom, npnode))
                .map_err(|x| die(ctx, &xpath_die_msg(src.filename, parts_xp.as_str(), &x)))?;
            if !subs.is_empty() {
                let parts: Vec<String> = subs.iter().map(|s| dom.text_content(*s)).collect();
                let s = join_name_parts(&parts);
                debug!(ctx.log, "Found namepart '{np}': {s}");
                let mut inits = Vec::new();
                for p in &subs {
                    match truthy_attr(dom, *p, "initial") {
                        Some(i) => inits.push(i.to_string()),
                        None => inits.extend(gen_initials(&[dom.text_content(*p)])),
                    }
                }
                part.string = Some(s);
                part.initial = Some(inits);
            } else {
                let t = dom.text_content(npnode);
                if perl_compat::perl_truthy(&t) {
                    debug!(ctx.log, "Found namepart '{np}': {t}");
                    // BB-new: biber reads `initial` from the name element,
                    // not from the namepart.
                    let holder = if ctx.cfg.quirks.on(Quirk::BltxmlNamepartInitial) {
                        node
                    } else {
                        npnode
                    };
                    part.initial = Some(match truthy_attr(dom, holder, "initial") {
                        Some(i) => vec![i.to_string()],
                        None => gen_initials(std::slice::from_ref(&t)),
                    });
                    part.string = Some(t);
                }
            }
        }
        if let Some(s) = part
            .string
            .as_deref()
            .filter(|s| perl_compat::perl_truthy(s))
        {
            st.section.set_np_length(np, s.chars().count());
        }
        if let Some(i) = &part.initial {
            st.section
                .set_np_length(&format!("{np}-i"), i.concat().chars().count());
        }
        name.nameparts.insert(np.clone(), part);
    }
    name.slots.insert(
        "gender".into(),
        dom.get_attribute(node, "gender").map_or(J::Null, J::from),
    );
    if let Some(h) = truthy_attr(dom, node, "id") {
        name.slots.insert("hashid".into(), J::from(h));
    }
    name.rawstring = Some(
        ["family", "given", "prefix", "suffix"]
            .iter()
            .filter_map(|p| name.nameparts.get(*p).and_then(|x| x.string.clone()))
            .chain(
                name.nameparts
                    .iter()
                    .filter(|(k, _)| !["family", "given", "prefix", "suffix"].contains(&k.as_str()))
                    .filter_map(|(_, v)| v.string.clone()),
            )
            .collect(),
    );
    let reg = &ctx.cfg.blx.registry;
    for no in reg.options_in_scope(Scope::Name).to_vec() {
        if let Some(v) = dom.get_attribute(node, &no) {
            let input = reg.decl(&no, Scope::Name).and_then(|d| d.input.clone());
            for (o, val) in expand_option_input(reg, &no, Some(v), input.as_ref()) {
                name.slots.insert(o, val.map_or(J::Null, J::from));
            }
        }
    }
    Ok(name)
}

/// The `.rng` file biber writes for a biblatexml data source in non-tool
/// mode (Biber.pm:4629-4646): the `.bcf` name with `bcf` replaced by `rng`.
pub fn schema_file(bcf: &str) -> String {
    match bcf.strip_suffix("bcf") {
        Some(stem) => format!("{stem}rng"),
        None => bcf.to_string(),
    }
}

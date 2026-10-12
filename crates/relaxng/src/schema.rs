//! Parsing the RELAX NG XML syntax and simplifying it (specification §4)
//! into an abstract syntax with only the simple-syntax pattern forms.
//!
//! The simplification steps are applied while walking the schema tree:
//!
//! * 4.1/4.2: foreign elements and attributes are ignored, whitespace-only
//!   text is ignored and `name`/`type`/`combine` values are trimmed;
//! * 4.3/4.4: `datatypeLibrary` inheritance (per document), and `value`
//!   without `type` meaning the built-in `token`;
//! * 4.5–4.7: `externalRef` and `include` (with their override rules),
//!   through a caller-supplied resolver;
//! * 4.8–4.10: `name` attributes become name classes, `ns` inheritance
//!   (an unprefixed attribute name is in no namespace unless its
//!   `attribute` element has its own `ns`), QName resolution;
//! * 4.11–4.15: `div` flattening, multiple children grouped, `mixed`,
//!   `optional` and `zeroOrMore` rewritten;
//! * 4.16: the name-class constraints on `except` and on `xmlns`;
//! * 4.17/4.18: `combine` and nested grammars (`parentRef`).
//!
//! Steps 4.19/4.20 (element definitions, `notAllowed`/`empty`
//! propagation) happen when the abstract syntax is compiled into the
//! interned pattern graph (`pattern.rs`).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use roxmltree::{Document, Node};

use crate::{RNG_NS, Resolver, SchemaError};

pub(crate) type AstId = usize;
pub(crate) type DefId = usize;

const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns";

/// A name class (§3, simplified).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NameClass {
    Name { ns: String, local: String },
    AnyName(Option<Box<NameClass>>),
    NsName(String, Option<Box<NameClass>>),
    Choice(Box<NameClass>, Box<NameClass>),
}

impl NameClass {
    pub(crate) fn contains(&self, ns: &str, local: &str) -> bool {
        match self {
            NameClass::Name { ns: n, local: l } => n == ns && l == local,
            NameClass::AnyName(ex) => !ex.as_ref().is_some_and(|e| e.contains(ns, local)),
            NameClass::NsName(n, ex) => {
                n == ns && !ex.as_ref().is_some_and(|e| e.contains(ns, local))
            }
            NameClass::Choice(a, b) => a.contains(ns, local) || b.contains(ns, local),
        }
    }

    /// A short description for messages.
    pub(crate) fn describe(&self) -> String {
        match self {
            NameClass::Name { local, .. } => local.clone(),
            NameClass::AnyName(_) => "*".into(),
            NameClass::NsName(ns, _) => format!("{{{ns}}}*"),
            NameClass::Choice(a, b) => format!("{} | {}", a.describe(), b.describe()),
        }
    }
}

/// Namespace bindings in scope at a `value` element (for QName values).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct NsContext {
    /// `(prefix, uri)`; the empty prefix is the default namespace, which
    /// for RELAX NG is the inherited `ns` attribute.
    pub(crate) bindings: Vec<(String, String)>,
}

impl NsContext {
    pub(crate) fn resolve(&self, prefix: &str) -> Option<&str> {
        self.bindings
            .iter()
            .find(|(p, _)| p == prefix)
            .map(|(_, u)| u.as_str())
    }
}

/// The simplified abstract syntax.
#[derive(Clone, Debug)]
pub(crate) enum Ast {
    Empty,
    NotAllowed,
    Text,
    Choice(AstId, AstId),
    Interleave(AstId, AstId),
    Group(AstId, AstId),
    OneOrMore(AstId),
    List(AstId),
    Data {
        lib: String,
        ty: String,
        params: Vec<(String, String)>,
        except: Option<AstId>,
        line: u32,
    },
    Value {
        lib: String,
        ty: String,
        value: String,
        ctx: Rc<NsContext>,
        line: u32,
    },
    Attribute(NameClass, AstId),
    Element(NameClass, AstId),
    Ref(DefId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Combine {
    Choice,
    Interleave,
}

#[derive(Debug)]
pub(crate) struct Def {
    pub(crate) name: String,
    bodies: Vec<AstId>,
    combine: Option<Combine>,
    plain: u32,
    first_ref_line: Option<u32>,
    pub(crate) body: Option<AstId>,
}

/// A parsed, simplified schema.
#[derive(Debug)]
pub(crate) struct SchemaAst {
    pub(crate) nodes: Vec<Ast>,
    pub(crate) defs: Vec<Def>,
    pub(crate) start: AstId,
    /// The operands of each n-ary `interleave` (and `mixed`), keyed by the
    /// root of its binary fold. libxml2's interleave behaviour depends on
    /// these groups (see `validate::Libxml2Interleave`).
    pub(crate) nary: HashMap<AstId, Vec<AstId>>,
}

struct GrammarScope {
    parent: Option<usize>,
    names: HashMap<String, DefId>,
    start: DefId,
}

#[derive(Clone)]
struct Inh {
    ns: String,
    dtlib: String,
    grammar: Option<usize>,
    base: String,
}

/// Components overridden by an enclosing `include` (§4.7).
#[derive(Clone, Default)]
struct Overrides {
    start: bool,
    defines: HashSet<String>,
}

#[derive(Default)]
struct Found {
    start: bool,
    defines: HashSet<String>,
}

struct Parser<'r, 'a> {
    nodes: Vec<Ast>,
    defs: Vec<Def>,
    grammars: Vec<GrammarScope>,
    resolver: &'r Resolver<'a>,
    open: Vec<String>,
    nary: HashMap<AstId, Vec<AstId>>,
}

fn line_of(n: Node<'_, '_>) -> u32 {
    n.document().text_pos_at(n.range().start).row
}

fn err(n: Node<'_, '_>, msg: impl Into<String>) -> SchemaError {
    SchemaError {
        message: msg.into(),
        line: Some(line_of(n)),
    }
}

fn is_xml_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

fn trim(s: &str) -> &str {
    s.trim_matches(is_xml_ws)
}

fn is_rng(n: &Node<'_, '_>) -> bool {
    n.is_element() && n.tag_name().namespace() == Some(RNG_NS)
}

/// The RELAX NG children of `n`. Foreign elements are annotations (§4.1);
/// text other than whitespace is an error.
fn rng_children<'a, 'i>(n: Node<'a, 'i>) -> Result<Vec<Node<'a, 'i>>, SchemaError> {
    let mut out = Vec::new();
    for c in n.children() {
        if is_rng(&c) {
            out.push(c);
        } else if c.is_text() && !c.text().unwrap_or("").chars().all(is_xml_ws) {
            return Err(err(
                n,
                format!("text is not allowed in <{}>", n.tag_name().name()),
            ));
        }
    }
    Ok(out)
}

fn text_content(n: Node<'_, '_>) -> String {
    n.children()
        .filter(|c| c.is_text())
        .map(|c| c.text().unwrap_or(""))
        .collect()
}

fn attr<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.namespace().is_none() && a.name() == name)
        .map(|a| a.value())
}

fn inherit(n: Node<'_, '_>, inh: &Inh) -> Inh {
    let mut out = inh.clone();
    if let Some(ns) = attr(n, "ns") {
        out.ns = ns.to_string();
    }
    if let Some(dl) = attr(n, "datatypeLibrary") {
        out.dtlib = dl.to_string();
    }
    out
}

/// Parse and simplify the schema document `text`.
pub(crate) fn parse(
    text: &str,
    base: &str,
    resolver: &Resolver<'_>,
) -> Result<SchemaAst, SchemaError> {
    let doc =
        Document::parse_with_options(text, crate::parsing_options()).map_err(|e| SchemaError {
            message: format!("schema is not well-formed: {e}"),
            line: Some(e.pos().row),
        })?;
    let mut p = Parser {
        nodes: Vec::new(),
        defs: Vec::new(),
        grammars: Vec::new(),
        resolver,
        open: vec![base.to_string()],
        nary: HashMap::new(),
    };
    let root = doc.root_element();
    if !is_rng(&root) {
        return Err(err(
            root,
            "the root element is not in the RELAX NG namespace",
        ));
    }
    let inh = Inh {
        ns: String::new(),
        dtlib: String::new(),
        grammar: None,
        base: base.to_string(),
    };
    let start = p.pattern(root, &inh)?;
    for d in &mut p.defs {
        d.bodies.clear();
    }
    Ok(SchemaAst {
        nodes: p.nodes,
        defs: p.defs,
        start,
        nary: p.nary,
    })
}

impl Parser<'_, '_> {
    fn push(&mut self, a: Ast) -> AstId {
        self.nodes.push(a);
        self.nodes.len() - 1
    }

    fn new_def(&mut self, name: &str) -> DefId {
        self.defs.push(Def {
            name: name.to_string(),
            bodies: Vec::new(),
            combine: None,
            plain: 0,
            first_ref_line: None,
            body: None,
        });
        self.defs.len() - 1
    }

    fn fold(&mut self, ids: Vec<AstId>, f: fn(AstId, AstId) -> Ast) -> Option<AstId> {
        let mut it = ids.into_iter();
        let mut acc = it.next()?;
        for x in it {
            acc = self.push(f(acc, x));
        }
        Some(acc)
    }

    /// The children of `n` as patterns, grouped (§4.12).
    fn group_children(
        &mut self,
        n: Node<'_, '_>,
        kids: &[Node<'_, '_>],
        inh: &Inh,
    ) -> Result<AstId, SchemaError> {
        let mut ids = Vec::new();
        for &k in kids {
            ids.push(self.pattern(k, inh)?);
        }
        self.fold(ids, Ast::Group).ok_or_else(|| {
            err(
                n,
                format!(
                    "<{}> must have at least one pattern child",
                    n.tag_name().name()
                ),
            )
        })
    }

    fn load(&mut self, n: Node<'_, '_>, inh: &Inh) -> Result<(String, String), SchemaError> {
        let href = attr(n, "href")
            .ok_or_else(|| err(n, format!("<{}> without href", n.tag_name().name())))?;
        let href = trim(href);
        let (b, text) = (self.resolver)(&inh.base, href).map_err(|e| err(n, e))?;
        if self.open.contains(&b) {
            return Err(err(n, format!("'{href}' includes itself (loop)")));
        }
        Ok((b, text))
    }

    fn pattern(&mut self, n: Node<'_, '_>, inh: &Inh) -> Result<AstId, SchemaError> {
        let inh = inherit(n, inh);
        let name = n.tag_name().name();
        match name {
            "element" => {
                let kids = rng_children(n)?;
                let (nc, rest) = match attr(n, "name") {
                    Some(qn) => (self.qname(n, trim(qn), &inh.ns)?, &kids[..]),
                    None => {
                        let (first, rest) = kids.split_first().ok_or_else(|| {
                            err(n, "<element> needs a name attribute or a name class")
                        })?;
                        (self.name_class(*first, &inh, None)?, rest)
                    }
                };
                let body = self.group_children(n, rest, &inh)?;
                Ok(self.push(Ast::Element(nc, body)))
            }
            "attribute" => {
                let kids = rng_children(n)?;
                let (nc, rest) = match attr(n, "name") {
                    Some(qn) => {
                        // §4.8: no ns attribute → ns="" for the name.
                        let ns = if attr(n, "ns").is_some() {
                            inh.ns.clone()
                        } else {
                            String::new()
                        };
                        (self.qname(n, trim(qn), &ns)?, &kids[..])
                    }
                    None => {
                        let (first, rest) = kids.split_first().ok_or_else(|| {
                            err(n, "<attribute> needs a name attribute or a name class")
                        })?;
                        (self.name_class(*first, &inh, None)?, rest)
                    }
                };
                check_attribute_name(n, &nc)?;
                let body = if rest.is_empty() {
                    self.push(Ast::Text)
                } else if rest.len() > 1 {
                    return Err(err(n, "<attribute> has more than one pattern child"));
                } else {
                    self.pattern(rest[0], &inh)?
                };
                Ok(self.push(Ast::Attribute(nc, body)))
            }
            "group" | "interleave" | "choice" => {
                let kids = rng_children(n)?;
                let mut ids = Vec::new();
                for k in kids {
                    ids.push(self.pattern(k, &inh)?);
                }
                let f: fn(AstId, AstId) -> Ast = match name {
                    "group" => Ast::Group,
                    "interleave" => Ast::Interleave,
                    _ => Ast::Choice,
                };
                let operands = ids.clone();
                let r = self
                    .fold(ids, f)
                    .ok_or_else(|| err(n, format!("<{name}> must have at least one child")))?;
                if name == "interleave" && operands.len() > 1 {
                    self.nary.insert(r, operands);
                }
                Ok(r)
            }
            "optional" | "zeroOrMore" | "oneOrMore" | "mixed" | "list" => {
                let kids = rng_children(n)?;
                let p = self.group_children(n, &kids, &inh)?;
                Ok(match name {
                    "optional" => {
                        let e = self.push(Ast::Empty);
                        self.push(Ast::Choice(p, e))
                    }
                    "zeroOrMore" => {
                        let o = self.push(Ast::OneOrMore(p));
                        let e = self.push(Ast::Empty);
                        self.push(Ast::Choice(o, e))
                    }
                    "oneOrMore" => self.push(Ast::OneOrMore(p)),
                    "mixed" => {
                        let t = self.push(Ast::Text);
                        let r = self.push(Ast::Interleave(p, t));
                        self.nary.insert(r, vec![p, t]);
                        r
                    }
                    _ => self.push(Ast::List(p)),
                })
            }
            "empty" | "text" | "notAllowed" => {
                if !rng_children(n)?.is_empty() {
                    return Err(err(n, format!("<{name}> must be empty")));
                }
                Ok(self.push(match name {
                    "empty" => Ast::Empty,
                    "text" => Ast::Text,
                    _ => Ast::NotAllowed,
                }))
            }
            "ref" | "parentRef" => {
                let rn =
                    trim(attr(n, "name").ok_or_else(|| err(n, format!("<{name}> without name")))?);
                let g = inh
                    .grammar
                    .ok_or_else(|| err(n, format!("<{name} name=\"{rn}\"> outside a grammar")))?;
                let g = if name == "parentRef" {
                    self.grammars[g].parent.ok_or_else(|| {
                        err(
                            n,
                            format!("<parentRef name=\"{rn}\"> without a parent grammar"),
                        )
                    })?
                } else {
                    g
                };
                let d = match self.grammars[g].names.get(rn) {
                    Some(&d) => d,
                    None => {
                        let d = self.new_def(rn);
                        self.grammars[g].names.insert(rn.to_string(), d);
                        d
                    }
                };
                if self.defs[d].first_ref_line.is_none() {
                    self.defs[d].first_ref_line = Some(line_of(n));
                }
                Ok(self.push(Ast::Ref(d)))
            }
            "data" => {
                let ty =
                    trim(attr(n, "type").ok_or_else(|| err(n, "<data> without type"))?).to_string();
                let mut params = Vec::new();
                let mut except = None;
                for k in rng_children(n)? {
                    match k.tag_name().name() {
                        "param" if except.is_none() => {
                            let pn = trim(
                                attr(k, "name").ok_or_else(|| err(k, "<param> without name"))?,
                            );
                            params.push((pn.to_string(), text_content(k)));
                        }
                        "except" if except.is_none() => {
                            let kinh = inherit(k, &inh);
                            let mut ids = Vec::new();
                            for e in rng_children(k)? {
                                ids.push(self.pattern(e, &kinh)?);
                            }
                            except =
                                Some(self.fold(ids, Ast::Choice).ok_or_else(|| {
                                    err(k, "<except> must have at least one child")
                                })?);
                        }
                        other => return Err(err(k, format!("<{other}> is not allowed in <data>"))),
                    }
                }
                Ok(self.push(Ast::Data {
                    lib: inh.dtlib.clone(),
                    ty,
                    params,
                    except,
                    line: line_of(n),
                }))
            }
            "value" => {
                for c in n.children() {
                    if c.is_element() {
                        return Err(err(n, "<value> must contain only text"));
                    }
                }
                let (lib, ty) = match attr(n, "type") {
                    Some(t) => (inh.dtlib.clone(), trim(t).to_string()),
                    None => (String::new(), "token".to_string()),
                };
                let mut bindings = vec![(String::new(), inh.ns.clone())];
                for ns in n.namespaces() {
                    if let Some(p) = ns.name() {
                        bindings.push((p.to_string(), ns.uri().to_string()));
                    }
                }
                Ok(self.push(Ast::Value {
                    lib,
                    ty,
                    value: text_content(n),
                    ctx: Rc::new(NsContext { bindings }),
                    line: line_of(n),
                }))
            }
            "externalRef" => {
                let (b, text) = self.load(n, &inh)?;
                let doc = Document::parse_with_options(&text, crate::parsing_options())
                    .map_err(|e| err(n, format!("'{b}' is not well-formed: {e}")))?;
                let root = doc.root_element();
                if !is_rng(&root) {
                    return Err(err(n, format!("'{b}' is not a RELAX NG pattern")));
                }
                self.open.push(b.clone());
                let rinh = Inh {
                    ns: inh.ns.clone(),
                    dtlib: String::new(),
                    grammar: inh.grammar,
                    base: b,
                };
                let r = self.pattern(root, &rinh);
                self.open.pop();
                r
            }
            "grammar" => self.grammar(n, &inh),
            other => Err(err(n, format!("<{other}> is not a RELAX NG pattern"))),
        }
    }

    fn qname(&self, n: Node<'_, '_>, qn: &str, default_ns: &str) -> Result<NameClass, SchemaError> {
        match qn.split_once(':') {
            Some((prefix, local)) => {
                let uri = n
                    .lookup_namespace_uri(Some(prefix))
                    .ok_or_else(|| err(n, format!("undeclared namespace prefix in '{qn}'")))?;
                Ok(NameClass::Name {
                    ns: uri.to_string(),
                    local: local.to_string(),
                })
            }
            None => Ok(NameClass::Name {
                ns: default_ns.to_string(),
                local: qn.to_string(),
            }),
        }
    }

    /// A name class element. `except_of` is `Some("anyName")` or
    /// `Some("nsName")` inside the `except` of one (§4.16).
    fn name_class(
        &mut self,
        n: Node<'_, '_>,
        inh: &Inh,
        except_of: Option<&str>,
    ) -> Result<NameClass, SchemaError> {
        let inh = inherit(n, inh);
        let name = n.tag_name().name();
        match name {
            "name" => {
                if n.children().any(|c| c.is_element()) {
                    return Err(err(n, "<name> must contain only text"));
                }
                self.qname(n, trim(&text_content(n)), &inh.ns)
            }
            "anyName" | "nsName" => {
                if name == "anyName" && except_of.is_some() {
                    return Err(err(n, "<anyName> is not allowed in an <except>"));
                }
                if name == "nsName" && except_of == Some("nsName") {
                    return Err(err(
                        n,
                        "<nsName> is not allowed in the <except> of an <nsName>",
                    ));
                }
                let kids = rng_children(n)?;
                let mut except = None;
                for k in kids {
                    if k.tag_name().name() != "except" || except.is_some() {
                        return Err(err(
                            k,
                            format!("<{}> is not allowed in <{name}>", k.tag_name().name()),
                        ));
                    }
                    let kinh = inherit(k, &inh);
                    let mut acc: Option<NameClass> = None;
                    for e in rng_children(k)? {
                        let nc = self.name_class(e, &kinh, Some(name))?;
                        acc = Some(match acc {
                            None => nc,
                            Some(a) => NameClass::Choice(Box::new(a), Box::new(nc)),
                        });
                    }
                    except =
                        Some(Box::new(acc.ok_or_else(|| {
                            err(k, "<except> must have at least one child")
                        })?));
                }
                Ok(if name == "anyName" {
                    NameClass::AnyName(except)
                } else {
                    NameClass::NsName(inh.ns.clone(), except)
                })
            }
            "choice" => {
                let mut acc: Option<NameClass> = None;
                for e in rng_children(n)? {
                    let nc = self.name_class(e, &inh, except_of)?;
                    acc = Some(match acc {
                        None => nc,
                        Some(a) => NameClass::Choice(Box::new(a), Box::new(nc)),
                    });
                }
                acc.ok_or_else(|| err(n, "<choice> must have at least one child"))
            }
            other => Err(err(n, format!("<{other}> is not a name class"))),
        }
    }

    fn grammar(&mut self, n: Node<'_, '_>, inh: &Inh) -> Result<AstId, SchemaError> {
        let start = self.new_def("start");
        self.grammars.push(GrammarScope {
            parent: inh.grammar,
            names: HashMap::new(),
            start,
        });
        let g = self.grammars.len() - 1;
        let ginh = Inh {
            grammar: Some(g),
            ..inh.clone()
        };
        let mut found = Found::default();
        self.components(n, &ginh, &Overrides::default(), &mut found)?;
        self.finish_grammar(g, n)?;
        Ok(self.push(Ast::Ref(start)))
    }

    fn components(
        &mut self,
        n: Node<'_, '_>,
        inh: &Inh,
        ov: &Overrides,
        found: &mut Found,
    ) -> Result<(), SchemaError> {
        let g = inh.grammar.expect("components are inside a grammar");
        for c in rng_children(n)? {
            let cinh = inherit(c, inh);
            match c.tag_name().name() {
                "start" => {
                    if ov.start {
                        found.start = true;
                        continue;
                    }
                    let kids = rng_children(c)?;
                    if kids.len() != 1 {
                        return Err(err(c, "<start> must have exactly one pattern child"));
                    }
                    let body = self.pattern(kids[0], &cinh)?;
                    let d = self.grammars[g].start;
                    self.add_body(d, body, c)?;
                }
                "define" => {
                    let name =
                        trim(attr(c, "name").ok_or_else(|| err(c, "<define> without name"))?)
                            .to_string();
                    if ov.defines.contains(&name) {
                        found.defines.insert(name);
                        continue;
                    }
                    let kids = rng_children(c)?;
                    let body = self.group_children(c, &kids, &cinh)?;
                    let d = match self.grammars[g].names.get(&name) {
                        Some(&d) => d,
                        None => {
                            let d = self.new_def(&name);
                            self.grammars[g].names.insert(name.clone(), d);
                            d
                        }
                    };
                    self.add_body(d, body, c)?;
                }
                "div" => self.components(c, &cinh, ov, found)?,
                "include" => {
                    let (b, text) = self.load(c, &cinh)?;
                    let doc = Document::parse_with_options(&text, crate::parsing_options())
                        .map_err(|e| err(c, format!("'{b}' is not well-formed: {e}")))?;
                    let root = doc.root_element();
                    if !is_rng(&root) || root.tag_name().name() != "grammar" {
                        return Err(err(c, format!("'{b}' is not a RELAX NG grammar")));
                    }
                    let mut own = Overrides::default();
                    collect_own(c, &mut own)?;
                    let mut inner = ov.clone();
                    inner.start |= own.start;
                    inner.defines.extend(own.defines.iter().cloned());
                    let mut f2 = Found::default();
                    self.open.push(b.clone());
                    let rinh = inherit(
                        root,
                        &Inh {
                            ns: cinh.ns.clone(),
                            dtlib: String::new(),
                            grammar: Some(g),
                            base: b,
                        },
                    );
                    let r = self.components(root, &rinh, &inner, &mut f2);
                    self.open.pop();
                    r?;
                    if own.start && !f2.start {
                        return Err(err(
                            c,
                            "<include> overrides start, but the included grammar has none",
                        ));
                    }
                    for d in &own.defines {
                        if !f2.defines.contains(d) {
                            return Err(err(
                                c,
                                format!(
                                    "<include> overrides '{d}', which the included grammar does not define"
                                ),
                            ));
                        }
                    }
                    found.start |= ov.start && f2.start;
                    for d in f2.defines {
                        if ov.defines.contains(&d) {
                            found.defines.insert(d);
                        }
                    }
                    self.components(c, &cinh, ov, found)?;
                }
                other => return Err(err(c, format!("<{other}> is not allowed in a grammar"))),
            }
        }
        Ok(())
    }

    fn add_body(&mut self, d: DefId, body: AstId, n: Node<'_, '_>) -> Result<(), SchemaError> {
        let name = self.defs[d].name.clone();
        match attr(n, "combine").map(trim) {
            None => {
                if self.defs[d].plain > 0 {
                    return Err(err(
                        n,
                        format!("'{name}' is defined twice without a combine attribute"),
                    ));
                }
                self.defs[d].plain += 1;
            }
            Some(m) => {
                let m = match m {
                    "choice" => Combine::Choice,
                    "interleave" => Combine::Interleave,
                    other => return Err(err(n, format!("invalid combine value '{other}'"))),
                };
                if let Some(prev) = self.defs[d].combine
                    && prev != m
                {
                    return Err(err(
                        n,
                        format!("'{name}' is combined with both choice and interleave"),
                    ));
                }
                self.defs[d].combine = Some(m);
            }
        }
        self.defs[d].bodies.push(body);
        Ok(())
    }

    fn finish_grammar(&mut self, g: usize, n: Node<'_, '_>) -> Result<(), SchemaError> {
        let mut ids: Vec<DefId> = self.grammars[g].names.values().copied().collect();
        ids.sort_unstable();
        ids.insert(0, self.grammars[g].start);
        for d in ids {
            let bodies = std::mem::take(&mut self.defs[d].bodies);
            if bodies.is_empty() {
                let def = &self.defs[d];
                return Err(if d == self.grammars[g].start {
                    err(n, "the grammar has no start")
                } else {
                    SchemaError {
                        message: format!("reference to undefined pattern '{}'", def.name),
                        line: def.first_ref_line,
                    }
                });
            }
            let f: fn(AstId, AstId) -> Ast = match self.defs[d].combine {
                Some(Combine::Interleave) => Ast::Interleave,
                _ => Ast::Choice,
            };
            let body = self.fold(bodies, f);
            self.defs[d].body = body;
        }
        Ok(())
    }
}

/// The start/define components of an `include` (through `div`s).
fn collect_own(n: Node<'_, '_>, out: &mut Overrides) -> Result<(), SchemaError> {
    for c in rng_children(n)? {
        match c.tag_name().name() {
            "start" => out.start = true,
            "define" => {
                if let Some(name) = attr(c, "name") {
                    out.defines.insert(trim(name).to_string());
                }
            }
            "div" => collect_own(c, out)?,
            _ => {}
        }
    }
    Ok(())
}

/// §4.16: attribute name classes may not name `xmlns` attributes.
fn check_attribute_name(n: Node<'_, '_>, nc: &NameClass) -> Result<(), SchemaError> {
    fn bad(nc: &NameClass) -> bool {
        match nc {
            NameClass::Name { ns, local } => ns == XMLNS_NS || (ns.is_empty() && local == "xmlns"),
            NameClass::NsName(ns, _) => ns == XMLNS_NS,
            NameClass::AnyName(_) => false,
            NameClass::Choice(a, b) => bad(a) || bad(b),
        }
    }
    if bad(nc) {
        Err(err(n, "an attribute pattern may not name xmlns attributes"))
    } else {
        Ok(())
    }
}

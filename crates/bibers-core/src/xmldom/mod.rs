//! A small mutable XML DOM with the semantics biber relies on through
//! `XML::LibXML` when reading biblatexml data sources
//! (`Biber::Input::file::biblatexml`): node names with their source prefixes
//! (`nodeName`), in-scope namespace declarations (for XPath prefix
//! resolution), `textContent`, attribute access by qualified name, and the
//! tree edits biblatexml source maps make (`setAttribute`, `appendChild`,
//! `cloneNode`, `unbindNode`, …).
//!
//! Parsing uses `roxmltree` (entities expanded, whitespace-only text kept,
//! as `XML::LibXML->load_xml` with its defaults). Node identity is an index
//! into an arena; unbound nodes stay in the arena (detached), like libxml2
//! nodes that are unlinked but still referenced.
//!
//! [`xpath`] evaluates XPath 1.0 expressions over this DOM.

pub mod xpath;

use std::fmt;

/// A node in a [`Dom`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

/// An element or attribute name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QName {
    /// The prefix as written (`None` for unprefixed names).
    pub prefix: Option<String>,
    /// The local part. For elements created with a prefixed name and no
    /// namespace (`XML::LibXML::Element->new("bltx:entry")`) this is the
    /// whole string.
    pub local: String,
    /// The namespace URI.
    pub ns: Option<String>,
}

impl QName {
    /// `nodeName`: `prefix:local` or `local`.
    pub fn qualified(&self) -> String {
        match &self.prefix {
            Some(p) => format!("{p}:{}", self.local),
            None => self.local.clone(),
        }
    }
}

/// The kind of a node, with its kind-specific data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Document,
    Element {
        name: QName,
        /// Attribute nodes, in document order.
        attrs: Vec<NodeId>,
        /// Namespace declarations made on this element (`xmlns[:p]`).
        nsdefs: Vec<(Option<String>, String)>,
    },
    Attribute {
        name: QName,
        value: String,
    },
    Text(String),
    Comment(String),
    Pi {
        target: String,
        data: String,
    },
}

/// One node of the arena.
#[derive(Clone, Debug)]
pub struct Node {
    pub kind: Kind,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

/// An XML parse error (`XML::LibXML->load_xml` dies).
#[derive(Clone, Debug)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A DOM: one document plus any detached nodes created later.
#[derive(Clone, Debug, Default)]
pub struct Dom {
    nodes: Vec<Node>,
}

/// The XML namespace (always in scope as `xml`).
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

impl Dom {
    /// A DOM with an empty document node.
    pub fn new() -> Dom {
        let mut d = Dom::default();
        d.alloc(Kind::Document, None);
        d
    }

    /// The document node.
    pub fn document(&self) -> NodeId {
        NodeId(0)
    }

    /// Parse `text` (already NFD'd by the caller, as biber does).
    pub fn parse(text: &str) -> Result<Dom, ParseError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let opts = roxmltree::ParsingOptions {
            allow_dtd: true,
            nodes_limit: u32::MAX,
            ..roxmltree::ParsingOptions::default()
        };
        let doc = roxmltree::Document::parse_with_options(text, opts)
            .map_err(|e| ParseError(e.to_string()))?;
        let mut dom = Dom::new();
        let root = dom.document();
        for c in doc.root().children() {
            dom.import(text, c, root);
        }
        Ok(dom)
    }

    fn alloc(&mut self, kind: Kind, parent: Option<NodeId>) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            kind,
            parent,
            children: Vec::new(),
        });
        id
    }

    fn import(&mut self, src: &str, n: roxmltree::Node, parent: NodeId) {
        let kind = match n.node_type() {
            roxmltree::NodeType::Element => {
                let tn = n.tag_name();
                let prefix = source_prefix(src, n.range().start, tn.name()).or_else(|| {
                    tn.namespace()
                        .and_then(|u| n.lookup_prefix(u))
                        .map(str::to_string)
                });
                let name = QName {
                    prefix: prefix.filter(|p| !p.is_empty()),
                    local: tn.name().to_string(),
                    ns: tn.namespace().map(str::to_string),
                };
                let parent_ns: Vec<(Option<&str>, &str)> = n
                    .parent_element()
                    .map(|p| p.namespaces().map(|x| (x.name(), x.uri())).collect())
                    .unwrap_or_default();
                let nsdefs = n
                    .namespaces()
                    .filter(|x| !parent_ns.contains(&(x.name(), x.uri())))
                    .filter(|x| x.name() != Some("xml"))
                    .map(|x| (x.name().map(str::to_string), x.uri().to_string()))
                    .collect();
                Kind::Element {
                    name,
                    attrs: Vec::new(),
                    nsdefs,
                }
            }
            roxmltree::NodeType::Text => Kind::Text(n.text().unwrap_or("").to_string()),
            roxmltree::NodeType::Comment => Kind::Comment(n.text().unwrap_or("").to_string()),
            roxmltree::NodeType::PI => {
                let pi = n.pi();
                Kind::Pi {
                    target: pi.map(|p| p.target.to_string()).unwrap_or_default(),
                    data: pi.and_then(|p| p.value).unwrap_or("").to_string(),
                }
            }
            roxmltree::NodeType::Root => return,
        };
        let id = self.alloc(kind, Some(parent));
        self.nodes[parent.0 as usize].children.push(id);
        if n.is_element() {
            let mut attrs = Vec::new();
            for a in n.attributes() {
                let qn = &src[a.range_qname()];
                let prefix = qn.split_once(':').map(|(p, _)| p.to_string());
                let name = QName {
                    prefix,
                    local: a.name().to_string(),
                    ns: a.namespace().map(str::to_string),
                };
                let aid = self.alloc(
                    Kind::Attribute {
                        name,
                        value: a.value().to_string(),
                    },
                    Some(id),
                );
                attrs.push(aid);
            }
            if let Kind::Element { attrs: a, .. } = &mut self.nodes[id.0 as usize].kind {
                *a = attrs;
            }
            for c in n.children() {
                self.import(src, c, id);
            }
        }
    }

    /// The node data.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0 as usize]
    }

    /// The parent (the owner element for attributes).
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    /// Child nodes (not attributes).
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.node(id).children
    }

    /// Attribute nodes of an element.
    pub fn attributes(&self, id: NodeId) -> &[NodeId] {
        match &self.node(id).kind {
            Kind::Element { attrs, .. } => attrs,
            _ => &[],
        }
    }

    /// Element children.
    pub fn element_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.children(id)
            .iter()
            .copied()
            .filter(|c| self.is_element(*c))
    }

    pub fn is_element(&self, id: NodeId) -> bool {
        matches!(self.node(id).kind, Kind::Element { .. })
    }

    pub fn is_attribute(&self, id: NodeId) -> bool {
        matches!(self.node(id).kind, Kind::Attribute { .. })
    }

    pub fn is_text(&self, id: NodeId) -> bool {
        matches!(self.node(id).kind, Kind::Text(_))
    }

    /// The element or attribute name.
    pub fn name(&self, id: NodeId) -> Option<&QName> {
        match &self.node(id).kind {
            Kind::Element { name, .. } | Kind::Attribute { name, .. } => Some(name),
            _ => None,
        }
    }

    /// `nodeName`.
    pub fn node_name(&self, id: NodeId) -> String {
        match &self.node(id).kind {
            Kind::Document => "#document".into(),
            Kind::Element { name, .. } | Kind::Attribute { name, .. } => name.qualified(),
            Kind::Text(_) => "#text".into(),
            Kind::Comment(_) => "#comment".into(),
            Kind::Pi { target, .. } => target.clone(),
        }
    }

    /// `textContent` (the XPath string-value).
    pub fn text_content(&self, id: NodeId) -> String {
        match &self.node(id).kind {
            Kind::Attribute { value, .. } => value.clone(),
            Kind::Text(t) | Kind::Comment(t) => t.clone(),
            Kind::Pi { data, .. } => data.clone(),
            Kind::Document | Kind::Element { .. } => {
                let mut s = String::new();
                self.collect_text(id, &mut s);
                s
            }
        }
    }

    fn collect_text(&self, id: NodeId, out: &mut String) {
        for &c in self.children(id) {
            match &self.node(c).kind {
                Kind::Text(t) => out.push_str(t),
                Kind::Element { .. } => self.collect_text(c, out),
                _ => {}
            }
        }
    }

    /// The attribute node named `name` (`getAttributeNode`): an attribute
    /// without namespace whose local name is `name`, else one whose
    /// `prefix:local` is `name`.
    pub fn attribute_node(&self, el: NodeId, name: &str) -> Option<NodeId> {
        let attrs = self.attributes(el);
        attrs
            .iter()
            .copied()
            .find(|a| {
                self.name(*a)
                    .is_some_and(|q| q.ns.is_none() && q.prefix.is_none() && q.local == name)
            })
            .or_else(|| {
                attrs
                    .iter()
                    .copied()
                    .find(|a| self.name(*a).is_some_and(|q| q.qualified() == name))
            })
    }

    /// `getAttribute`.
    pub fn get_attribute(&self, el: NodeId, name: &str) -> Option<&str> {
        let a = self.attribute_node(el, name)?;
        match &self.node(a).kind {
            Kind::Attribute { value, .. } => Some(value),
            _ => None,
        }
    }

    /// `hasAttribute`.
    pub fn has_attribute(&self, el: NodeId, name: &str) -> bool {
        self.attribute_node(el, name).is_some()
    }

    /// `setAttribute`: replace the value of an existing attribute, or add one.
    pub fn set_attribute(&mut self, el: NodeId, name: &str, value: &str) {
        if let Some(a) = self.attribute_node(el, name) {
            self.set_value(a, value);
            return;
        }
        let (prefix, local) = match name.split_once(':') {
            Some((p, l)) => (Some(p.to_string()), l.to_string()),
            None => (None, name.to_string()),
        };
        let ns = prefix
            .as_deref()
            .and_then(|p| self.lookup_ns(el, Some(p)))
            .map(str::to_string);
        let (prefix, local) = if prefix.is_some() && ns.is_none() {
            (None, name.to_string())
        } else {
            (prefix, local)
        };
        let aid = self.alloc(
            Kind::Attribute {
                name: QName { prefix, local, ns },
                value: value.to_string(),
            },
            Some(el),
        );
        if let Kind::Element { attrs, .. } = &mut self.node_mut(el).kind {
            attrs.push(aid);
        }
    }

    /// Set an attribute's value (`setValue`) or a text node's data
    /// (`setData`).
    pub fn set_value(&mut self, id: NodeId, v: &str) {
        match &mut self.node_mut(id).kind {
            Kind::Attribute { value, .. } => *value = v.to_string(),
            Kind::Text(t) | Kind::Comment(t) => *t = v.to_string(),
            _ => {}
        }
    }

    /// The namespace URI bound to `prefix` in scope at `id` (declarations on
    /// `id` and its ancestors; `xml` is always bound).
    pub fn lookup_ns(&self, id: NodeId, prefix: Option<&str>) -> Option<&str> {
        if prefix == Some("xml") {
            return Some(XML_NS);
        }
        let mut cur = Some(id);
        while let Some(n) = cur {
            if let Kind::Element { nsdefs, .. } = &self.node(n).kind
                && let Some((_, u)) = nsdefs.iter().find(|(p, _)| p.as_deref() == prefix)
            {
                return Some(u);
            }
            cur = self.parent(n);
        }
        None
    }

    /// The namespace declarations in scope at `id` (innermost first; one per
    /// prefix), as `xmlGetNsList` gives them to XPath.
    pub fn in_scope_ns(&self, id: NodeId) -> Vec<(Option<String>, String)> {
        let mut out: Vec<(Option<String>, String)> = Vec::new();
        let mut cur = Some(id);
        while let Some(n) = cur {
            if let Kind::Element { nsdefs, .. } = &self.node(n).kind {
                for (p, u) in nsdefs {
                    if !out.iter().any(|(q, _)| q == p) {
                        out.push((p.clone(), u.clone()));
                    }
                }
            }
            cur = self.parent(n);
        }
        out
    }

    /// A new detached element (`XML::LibXML::Element->new($name)`): the
    /// whole name is the local name, with no namespace.
    pub fn new_element(&mut self, name: &str) -> NodeId {
        self.alloc(
            Kind::Element {
                name: QName {
                    prefix: None,
                    local: name.to_string(),
                    ns: None,
                },
                attrs: Vec::new(),
                nsdefs: Vec::new(),
            },
            None,
        )
    }

    /// `setNamespace($uri, $prefix)` on an element: reuse an in-scope
    /// declaration of `prefix` for `uri`, else declare it on the element.
    pub fn set_namespace(&mut self, el: NodeId, uri: &str, prefix: &str) {
        if self.lookup_ns(el, Some(prefix)) != Some(uri)
            && let Kind::Element { nsdefs, .. } = &mut self.node_mut(el).kind
        {
            nsdefs.push((Some(prefix.to_string()), uri.to_string()));
        }
        if let Kind::Element { name, .. } = &mut self.node_mut(el).kind {
            name.prefix = Some(prefix.to_string());
            name.ns = Some(uri.to_string());
        }
    }

    /// Detach a node from its parent (`unbindNode`); attributes are removed
    /// from their element.
    pub fn unbind(&mut self, id: NodeId) {
        let Some(p) = self.parent(id) else {
            return;
        };
        if self.is_attribute(id) {
            if let Kind::Element { attrs, .. } = &mut self.node_mut(p).kind {
                attrs.retain(|a| *a != id);
            }
        } else {
            self.node_mut(p).children.retain(|c| *c != id);
        }
        self.node_mut(id).parent = None;
    }

    /// `appendChild`: detach `child` from any parent and append it to
    /// `parent`. Adjacent text nodes are merged, as libxml2 does.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> NodeId {
        self.unbind(child);
        if let Kind::Text(t) = &self.node(child).kind
            && let Some(&last) = self.children(parent).last()
            && let Kind::Text(prev) = &self.node(last).kind
        {
            let merged = format!("{prev}{t}");
            self.set_value(last, &merged);
            return last;
        }
        self.node_mut(child).parent = Some(parent);
        self.node_mut(parent).children.push(child);
        child
    }

    /// `appendTextNode`.
    pub fn append_text(&mut self, parent: NodeId, text: &str) {
        let t = self.alloc(Kind::Text(text.to_string()), None);
        self.append_child(parent, t);
    }

    /// `removeChildNodes`.
    pub fn remove_children(&mut self, id: NodeId) {
        let cs = std::mem::take(&mut self.node_mut(id).children);
        for c in cs {
            self.node_mut(c).parent = None;
        }
    }

    /// `cloneNode(1)`: a detached deep copy (attributes and namespace
    /// declarations included).
    pub fn clone_deep(&mut self, id: NodeId) -> NodeId {
        let kind = match &self.node(id).kind {
            Kind::Element { name, nsdefs, .. } => Kind::Element {
                name: name.clone(),
                attrs: Vec::new(),
                nsdefs: nsdefs.clone(),
            },
            k => k.clone(),
        };
        let new = self.alloc(kind, None);
        for a in self.attributes(id).to_vec() {
            let ak = self.node(a).kind.clone();
            let na = self.alloc(ak, Some(new));
            if let Kind::Element { attrs, .. } = &mut self.node_mut(new).kind {
                attrs.push(na);
            }
        }
        for c in self.children(id).to_vec() {
            let nc = self.clone_deep(c);
            self.node_mut(nc).parent = Some(new);
            self.node_mut(new).children.push(nc);
        }
        new
    }

    /// The root of the tree `id` belongs to (the document node, or the top
    /// of a detached subtree).
    pub fn root_of(&self, id: NodeId) -> NodeId {
        let mut cur = id;
        while let Some(p) = self.parent(cur) {
            cur = p;
        }
        cur
    }

    /// A key ordering nodes in document order (within one tree; trees are
    /// ordered by their root's id): an element sorts before its attributes,
    /// which sort before its children.
    pub fn order_key(&self, id: NodeId) -> Vec<u32> {
        let mut path = Vec::new();
        let mut cur = id;
        while let Some(p) = self.parent(cur) {
            if self.is_attribute(cur) {
                let i = self
                    .attributes(p)
                    .iter()
                    .position(|a| *a == cur)
                    .unwrap_or(0);
                path.push(i as u32);
                path.push(0);
            } else {
                let i = self.children(p).iter().position(|c| *c == cur).unwrap_or(0);
                path.push(i as u32);
                path.push(1);
            }
            cur = p;
        }
        path.push(cur.0);
        path.reverse();
        path
    }

    /// `toString` (libxml2's node serialisation, no formatting): what a node
    /// object stringifies to in Perl. A namespace used in the subtree but
    /// declared outside it is declared on the serialised root, as a copied
    /// node carries it.
    pub fn to_xml_string(&self, id: NodeId) -> String {
        let mut extra: Vec<(Option<String>, String)> = Vec::new();
        if self.is_element(id) {
            self.collect_undeclared(id, &mut Vec::new(), &mut extra);
        }
        let mut out = String::new();
        self.serialise(id, &extra, &mut out);
        out
    }

    fn collect_undeclared(
        &self,
        id: NodeId,
        scope: &mut Vec<Option<String>>,
        out: &mut Vec<(Option<String>, String)>,
    ) {
        let Kind::Element {
            name,
            attrs,
            nsdefs,
        } = &self.node(id).kind
        else {
            return;
        };
        let depth = scope.len();
        scope.extend(nsdefs.iter().map(|(p, _)| p.clone()));
        let mut uses: Vec<&QName> = vec![name];
        uses.extend(attrs.iter().filter_map(|a| self.name(*a)));
        for q in uses {
            if let Some(ns) = &q.ns
                && q.prefix.as_deref() != Some("xml")
                && !scope.contains(&q.prefix)
                && !out.iter().any(|(p, _)| *p == q.prefix)
            {
                out.push((q.prefix.clone(), ns.clone()));
            }
        }
        for &c in self.children(id) {
            self.collect_undeclared(c, scope, out);
        }
        scope.truncate(depth);
    }

    fn serialise(&self, id: NodeId, extra: &[(Option<String>, String)], out: &mut String) {
        match &self.node(id).kind {
            Kind::Document => {
                for &c in self.children(id) {
                    self.serialise(c, &[], out);
                }
            }
            Kind::Text(t) => out.push_str(&escape_text(t)),
            Kind::Comment(t) => {
                out.push_str("<!--");
                out.push_str(t);
                out.push_str("-->");
            }
            Kind::Pi { target, data } => {
                out.push_str("<?");
                out.push_str(target);
                if !data.is_empty() {
                    out.push(' ');
                    out.push_str(data);
                }
                out.push_str("?>");
            }
            Kind::Attribute { name, value } => {
                out.push_str(&format!(" {}=\"{}\"", name.qualified(), escape_attr(value)));
            }
            Kind::Element {
                name,
                attrs,
                nsdefs,
            } => {
                out.push('<');
                out.push_str(&name.qualified());
                for (p, u) in nsdefs.iter().chain(extra) {
                    match p {
                        Some(p) => out.push_str(&format!(" xmlns:{p}=\"{}\"", escape_attr(u))),
                        None => out.push_str(&format!(" xmlns=\"{}\"", escape_attr(u))),
                    }
                }
                for &a in attrs {
                    self.serialise(a, &[], out);
                }
                let cs = self.children(id);
                if cs.is_empty() {
                    out.push_str("/>");
                    return;
                }
                out.push('>');
                for &c in cs {
                    self.serialise(c, &[], out);
                }
                out.push_str("</");
                out.push_str(&name.qualified());
                out.push('>');
            }
        }
    }

    /// The `nodePath` of a node (as `XML::LibXML`'s, used in debug messages).
    pub fn node_path(&self, id: NodeId) -> String {
        let Some(p) = self.parent(id) else {
            return "/".into();
        };
        let me = match &self.node(id).kind {
            Kind::Attribute { name, .. } => format!("@{}", name.qualified()),
            Kind::Element { name, .. } => {
                let same: Vec<NodeId> = self
                    .children(p)
                    .iter()
                    .copied()
                    .filter(|c| self.name(*c).is_some_and(|q| q == name))
                    .collect();
                if same.len() > 1 {
                    let i = same.iter().position(|c| *c == id).unwrap_or(0) + 1;
                    format!("{}[{i}]", name.qualified())
                } else {
                    name.qualified()
                }
            }
            Kind::Text(_) => "text()".into(),
            Kind::Comment(_) => "comment()".into(),
            Kind::Pi { target, .. } => format!("processing-instruction('{target}')"),
            Kind::Document => String::new(),
        };
        let pp = self.node_path(p);
        if pp == "/" {
            format!("/{me}")
        } else {
            format!("{pp}/{me}")
        }
    }
}

fn escape_text(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '\r' => o.push_str("&#13;"),
            c => o.push(c),
        }
    }
    o
}

fn escape_attr(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\n' => o.push_str("&#10;"),
            '\r' => o.push_str("&#13;"),
            '\t' => o.push_str("&#9;"),
            c => o.push(c),
        }
    }
    o
}

/// The qualified element name as written at byte `start` (`<prefix:local`).
fn source_prefix(src: &str, start: usize, local: &str) -> Option<String> {
    let rest = src.get(start..)?.strip_prefix('<')?;
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(rest.len());
    let qn = &rest[..end];
    match qn.split_once(':') {
        Some((p, l)) if l == local => Some(p.to_string()),
        None if qn == local => Some(String::new()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "http://biblatex-biber.sourceforge.net/biblatexml";

    #[test]
    fn parse_names_and_text() {
        let d = Dom::parse(&format!(
            r#"<?xml version="1.0"?><bltx:entries xmlns:bltx="{NS}"><bltx:entry id="a" bltx:x="1"> <bltx:title>T<![CDATA[&]]>u</bltx:title><!-- c --></bltx:entry></bltx:entries>"#
        ))
        .unwrap();
        let root = d.element_children(d.document()).next().unwrap();
        assert_eq!(d.node_name(root), "bltx:entries");
        let e = d.element_children(root).next().unwrap();
        assert_eq!(d.get_attribute(e, "id"), Some("a"));
        assert_eq!(d.get_attribute(e, "bltx:x"), Some("1"));
        assert_eq!(d.get_attribute(e, "x"), None);
        assert_eq!(d.text_content(e), " T&u");
        assert_eq!(d.lookup_ns(e, Some("bltx")), Some(NS));
        assert_eq!(d.node_path(e), "/bltx:entries/bltx:entry");
    }

    #[test]
    fn edits() {
        let mut d = Dom::parse(&format!(
            r#"<b:entries xmlns:b="{NS}"><b:t>x</b:t></b:entries>"#
        ))
        .unwrap();
        let root = d.element_children(d.document()).next().unwrap();
        let c = d.clone_deep(root);
        assert_eq!(d.text_content(c), "x");
        let e = d.new_element("names");
        d.append_child(root, e);
        d.set_namespace(e, NS, "bltx");
        assert_eq!(d.node_name(e), "bltx:names");
        assert_eq!(d.lookup_ns(e, Some("bltx")), Some(NS));
        assert_eq!(d.lookup_ns(root, Some("bltx")), None);
        d.append_text(e, "a");
        d.append_text(e, "b");
        assert_eq!(d.children(e).len(), 1);
        assert_eq!(d.text_content(e), "ab");
        d.set_attribute(e, "type", "author");
        assert!(d.has_attribute(e, "type"));
        let a = d.attribute_node(e, "type").unwrap();
        d.unbind(a);
        assert!(!d.has_attribute(e, "type"));
    }
}

//! An emulation of Perl's `XML::Writer` 0.900 as biber uses it (the bltxml
//! schema and biblatexml output): `DATA_MODE` with `DATA_INDENT`, and
//! `NAMESPACES` with a `PREFIX_MAP` and `forceNSDecl`. Output is
//! byte-identical to the module's for the calls biber makes.

use std::collections::HashMap;

/// An element name: plain, or namespaced (`[$uri, $local]` in Perl).
#[derive(Clone, Copy, Debug)]
pub enum XName<'a> {
    Plain(&'a str),
    Ns(&'a str, &'a str),
}

#[derive(Clone, Debug, Default)]
struct NsState {
    /// uri → prefix in scope (`''` for the default namespace).
    decls: HashMap<String, String>,
    default_decl: Option<String>,
}

/// The writer. Text is accumulated in a string ([`XmlWriter::finish`]).
#[derive(Debug)]
pub struct XmlWriter {
    out: String,
    data_mode: bool,
    indent: String,
    level: usize,
    stack: Vec<String>,
    has_data: bool,
    has_element: bool,
    data_stack: Vec<bool>,
    elem_stack: Vec<bool>,
    has_heading: bool,
    /// `PREFIX_MAP` (uri → prefix), without the default namespace.
    prefix_map: Vec<(String, String)>,
    default_ns: Option<String>,
    ns: NsState,
    ns_stack: Vec<NsState>,
    forced: Vec<String>,
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
        .replace('\t', "&#9;")
}

impl XmlWriter {
    /// `XML::Writer->new(NAMESPACES => 1, DATA_MODE => 1, DATA_INDENT =>
    /// $indent, PREFIX_MAP => {…})`; a prefix of `""` makes that URI the
    /// default namespace. `indent` is a number of spaces or a whitespace
    /// string (`setDataIndent`).
    pub fn new(indent: &str, prefix_map: &[(&str, &str)]) -> XmlWriter {
        let indent = if indent.chars().all(char::is_whitespace) {
            indent.to_string()
        } else {
            " ".repeat(crate::perlish::numify(indent) as usize)
        };
        let default_ns = prefix_map
            .iter()
            .find(|(_, p)| p.is_empty())
            .map(|(u, _)| u.to_string());
        let mut ns = NsState::default();
        ns.decls
            .insert("http://www.w3.org/XML/1998/namespace".into(), "xml".into());
        XmlWriter {
            out: String::new(),
            data_mode: true,
            indent,
            level: 0,
            stack: Vec::new(),
            has_data: false,
            has_element: false,
            data_stack: Vec::new(),
            elem_stack: Vec::new(),
            has_heading: false,
            prefix_map: prefix_map
                .iter()
                .filter(|(_, p)| !p.is_empty())
                .map(|(u, p)| (u.to_string(), p.to_string()))
                .collect(),
            default_ns,
            ns,
            ns_stack: Vec::new(),
            forced: Vec::new(),
        }
    }

    /// `xmlDecl` (with the constructor's `ENCODING => 'UTF-8'`).
    pub fn xml_decl(&mut self) {
        self.out
            .push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        self.has_heading = true;
    }

    /// `pi`.
    pub fn pi(&mut self, target: &str, data: &str) {
        if data.is_empty() {
            self.out.push_str(&format!("<?{target}?>"));
        } else {
            self.out.push_str(&format!("<?{target} {data}?>"));
        }
        if self.level == 0 {
            self.out.push('\n');
            self.has_heading = true;
        }
    }

    /// `comment`.
    pub fn comment(&mut self, data: &str) {
        if self.data_mode && self.level > 0 {
            self.newline_indent();
        }
        self.out.push_str(&format!("<!-- {data} -->"));
        if self.data_mode && self.level > 0 {
            self.has_element = true;
        } else if self.level == 0 {
            self.out.push('\n');
            self.has_heading = true;
        }
    }

    /// `forceNSDecl`.
    pub fn force_ns_decl(&mut self, uri: &str) {
        self.forced.push(uri.to_string());
    }

    fn newline_indent(&mut self) {
        self.out.push('\n');
        for _ in 0..self.level {
            self.out.push_str(&self.indent.clone());
        }
    }

    fn gen_prefix(&self, uri: &str) -> String {
        if let Some((_, p)) = self.prefix_map.iter().find(|(u, _)| u == uri) {
            return p.clone();
        }
        let taken: Vec<&String> = self
            .ns
            .decls
            .values()
            .chain(self.prefix_map.iter().map(|(_, p)| p))
            .collect();
        let mut i = 1;
        loop {
            let p = format!("__NS{i}");
            if !taken.contains(&&p) {
                return p;
            }
            i += 1;
        }
    }

    /// `processName` for an element or attribute name.
    fn process_name(
        &mut self,
        name: XName,
        atts: &mut Vec<(String, String)>,
        is_attr: bool,
    ) -> String {
        let (uri, local) = match name {
            XName::Plain(n) => return n.to_string(),
            XName::Ns(u, l) => (u, l),
        };
        if !is_attr && self.default_ns.as_deref() == Some(uri) {
            if self.ns.default_decl.as_deref() != Some(uri) {
                atts.push(("xmlns".into(), uri.to_string()));
                self.ns.default_decl = Some(uri.to_string());
            }
            self.ns.decls.retain(|_, p| !p.is_empty());
            self.ns.decls.insert(uri.to_string(), String::new());
            return local.to_string();
        }
        if let Some(p) = self.ns.decls.get(uri).filter(|p| !p.is_empty()) {
            return format!("{p}:{local}");
        }
        let p = self.gen_prefix(uri);
        self.ns.decls.insert(uri.to_string(), p.clone());
        atts.push((format!("xmlns:{p}"), uri.to_string()));
        format!("{p}:{local}")
    }

    fn ns_process(
        &mut self,
        name: XName,
        attrs: &[(&str, &str)],
    ) -> (String, Vec<(String, String)>) {
        let mut atts: Vec<(String, String)> = attrs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let n = self.process_name(name, &mut atts, false);
        for uri in std::mem::take(&mut self.forced) {
            let is_default = self.default_ns.as_deref() == Some(uri.as_str());
            self.process_name(XName::Ns(&uri, "dummy"), &mut atts, !is_default);
        }
        (n, atts)
    }

    fn show_attrs(&mut self, atts: &[(String, String)]) {
        for (k, v) in atts {
            if k.is_empty() {
                break;
            }
            self.out.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
        }
    }

    /// `startTag`.
    pub fn start_tag(&mut self, name: XName, attrs: &[(&str, &str)]) {
        self.ns_stack.push(self.ns.clone());
        let (n, atts) = self.ns_process(name, attrs);
        if self.data_mode && (self.has_heading || self.level > 0) {
            self.newline_indent();
        }
        self.level += 1;
        self.stack.push(n.clone());
        self.out.push('<');
        self.out.push_str(&n);
        self.show_attrs(&atts);
        self.out.push('>');
        if self.data_mode {
            self.has_element = true;
            self.data_stack.push(self.has_data);
            self.has_data = false;
            self.elem_stack.push(self.has_element);
            self.has_element = false;
        }
    }

    /// `emptyTag`.
    pub fn empty_tag(&mut self, name: XName, attrs: &[(&str, &str)]) {
        let saved = self.ns.clone();
        let (n, atts) = self.ns_process(name, attrs);
        if self.data_mode && (self.has_heading || self.level > 0) {
            self.newline_indent();
        }
        self.out.push('<');
        self.out.push_str(&n);
        self.show_attrs(&atts);
        self.out.push_str(" />");
        if self.data_mode {
            self.has_element = true;
        }
        self.ns = saved;
    }

    /// `endTag`.
    pub fn end_tag(&mut self) {
        let name = self.stack.pop().unwrap_or_default();
        self.level = self.level.saturating_sub(1);
        if self.data_mode && self.has_element {
            self.newline_indent();
        }
        self.out.push_str(&format!("</{name}>"));
        if self.data_mode {
            self.has_data = self.data_stack.pop().unwrap_or(false);
            self.has_element = self.elem_stack.pop().unwrap_or(false);
        }
        if let Some(s) = self.ns_stack.pop() {
            self.ns = s;
        }
    }

    /// `characters`.
    pub fn characters(&mut self, data: &str) {
        self.out.push_str(&escape_text(data));
        self.has_data = true;
    }

    /// `dataElement`.
    pub fn data_element(&mut self, name: XName, data: &str, attrs: &[(&str, &str)]) {
        self.start_tag(name, attrs);
        self.characters(data);
        self.end_tag();
    }

    /// `end`: the final newline; returns the document.
    pub fn finish(mut self) -> String {
        self.out.push('\n');
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_mode_and_namespaces() {
        let ns = "http://example.org/x";
        let rng = "http://relaxng.org/ns/structure/1.0";
        let mut w = XmlWriter::new("2", &[(ns, "x"), (rng, "")]);
        w.xml_decl();
        w.comment("c");
        w.force_ns_decl(rng);
        w.force_ns_decl(ns);
        w.start_tag(XName::Plain("grammar"), &[("a", "1")]);
        w.start_tag(XName::Plain("choice"), &[]);
        w.data_element(XName::Plain("value"), "v&", &[]);
        w.end_tag();
        w.comment("d");
        w.empty_tag(XName::Plain("ref"), &[("name", "n")]);
        w.end_tag();
        assert_eq!(
            w.finish(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- c -->\n\n<grammar a=\"1\" xmlns=\"http://relaxng.org/ns/structure/1.0\" xmlns:x=\"http://example.org/x\">\n  <choice>\n    <value>v&amp;</value>\n  </choice>\n  <!-- d -->\n  <ref name=\"n\" />\n</grammar>\n"
        );
        let mut w = XmlWriter::new("2", &[(ns, "bltx")]);
        w.xml_decl();
        w.start_tag(XName::Ns(ns, "entries"), &[]);
        w.data_element(XName::Ns(ns, "item"), "i", &[("id", "a")]);
        w.end_tag();
        assert_eq!(
            w.finish(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\n<bltx:entries xmlns:bltx=\"http://example.org/x\">\n  <bltx:item id=\"a\">i</bltx:item>\n</bltx:entries>\n"
        );
    }
}

//! The structured form of a fuzz input (a `.bib` file plus the citations of
//! the `.bcf`), so the minimiser can delete entries, fields and parts of
//! values, and its rendering to text.

use serde::{Deserialize, Serialize};

/// One generated test input.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FuzzInput {
    pub items: Vec<Item>,
    /// `\cite`d keys, in citation order (`nocite` = `\nocite{key}`).
    pub cites: Vec<Cite>,
    /// `\nocite{*}` (emitted after the explicit citations).
    pub nocite_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cite {
    pub key: String,
    pub nocite: bool,
}

/// A top-level item of a `.bib` file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Item {
    Entry(Entry),
    /// `@string{name = value}`
    Macro {
        name: String,
        value: Val,
    },
    /// `@preamble{value}`
    Preamble(Val),
    /// `@comment{text}`
    Comment(String),
    /// Text copied verbatim (junk between entries, deliberately broken entries).
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Entry {
    pub ty: String,
    pub key: String,
    pub fields: Vec<Field>,
    /// `@type(key, …)` instead of `@type{key, …}`.
    pub parens: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub value: Val,
}

/// A field value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Val {
    /// `{text}`
    Braced(String),
    /// `"text"`
    Quoted(String),
    /// A number or a macro name, unquoted.
    Bare(String),
    /// `a # b # …`
    Concat(Vec<Val>),
}

impl Val {
    pub fn render(&self, out: &mut String) {
        match self {
            Val::Braced(s) => {
                out.push('{');
                out.push_str(s);
                out.push('}');
            }
            Val::Quoted(s) => {
                out.push('"');
                out.push_str(s);
                out.push('"');
            }
            Val::Bare(s) => out.push_str(s),
            Val::Concat(vs) => {
                for (i, v) in vs.iter().enumerate() {
                    if i > 0 {
                        out.push_str(" # ");
                    }
                    v.render(out);
                }
            }
        }
    }
}

impl FuzzInput {
    /// The `.bib` file.
    pub fn render_bib(&self) -> String {
        let mut out = String::new();
        for item in &self.items {
            match item {
                Item::Entry(e) => {
                    let (open, close) = if e.parens { ('(', ')') } else { ('{', '}') };
                    out.push('@');
                    out.push_str(&e.ty);
                    out.push(open);
                    out.push_str(&e.key);
                    for f in &e.fields {
                        out.push_str(",\n  ");
                        out.push_str(&f.name);
                        out.push_str(" = ");
                        f.value.render(&mut out);
                    }
                    out.push('\n');
                    out.push(close);
                }
                Item::Macro { name, value } => {
                    out.push_str("@string{");
                    out.push_str(name);
                    out.push_str(" = ");
                    value.render(&mut out);
                    out.push('}');
                }
                Item::Preamble(v) => {
                    out.push_str("@preamble{");
                    v.render(&mut out);
                    out.push('}');
                }
                Item::Comment(s) => {
                    out.push_str("@comment{");
                    out.push_str(s);
                    out.push('}');
                }
                Item::Raw(s) => out.push_str(s),
            }
            out.push_str("\n\n");
        }
        out
    }

    /// The `<bcf:section>` body: one `<bcf:citekey>` per citation.
    pub fn render_citekeys(&self) -> String {
        let mut out = String::new();
        let mut order = 0;
        for c in &self.cites {
            order += 1;
            let nocite = if c.nocite { " nocite=\"1\"" } else { "" };
            out.push_str(&format!(
                "    <bcf:citekey order=\"{order}\" intorder=\"1\"{nocite}>{}</bcf:citekey>\n",
                xml_escape(&c.key)
            ));
        }
        if self.nocite_all {
            order += 1;
            out.push_str(&format!(
                "    <bcf:citekey order=\"{order}\" intorder=\"1\" nocite=\"1\">*</bcf:citekey>\n"
            ));
        }
        out
    }

    pub fn entry_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, Item::Entry(_)))
            .count()
    }
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

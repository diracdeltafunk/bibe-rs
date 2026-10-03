//! A faithful emulation of `XML::LibXML::Simple::XMLin` as biber calls it
//! (`ForceContent => 1`, `NsStrip => 1`, `KeyAttr => []`, a `ForceArray`
//! list), because several of its quirks leak into biber's behaviour
//! (docs/research/02 §3.2 item 6):
//!
//! * `ForceArray` applies to attributes as well as elements, and an attribute
//!   and a child element with the same local name merge into one array
//!   (attributes first);
//! * an element with only text becomes `{content => text}`; with attributes
//!   and text, a hash with `content`; with child elements, its text (including
//!   whitespace) is dropped; an empty element becomes `{}`;
//! * the relative order of differently named children is lost, while
//!   same-named children keep document order;
//! * a repeated element that is not force-arrayed becomes an array.
//!
//! The result is an [`XVal`] tree with Perl-like accessors. Hash key order is
//! insertion order (Perl's is random; no biber code path that we port depends
//! on it).

use std::fmt;

/// A value in an `XMLin` result tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XVal {
    /// A string (attribute value or text content).
    Str(String),
    /// A hash (element with attributes and/or children).
    Hash(XHash),
    /// An array (force-arrayed or repeated key).
    Array(Vec<XVal>),
}

/// An insertion-ordered string-keyed hash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XHash(pub Vec<(String, XVal)>);

impl XHash {
    /// Look up a key.
    pub fn get(&self, key: &str) -> Option<&XVal> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Mutable lookup.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut XVal> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// `exists $h->{key}`.
    pub fn contains(&self, key: &str) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }

    /// Insert or replace.
    pub fn insert(&mut self, key: &str, val: XVal) {
        match self.get_mut(key) {
            Some(v) => *v = val,
            None => self.0.push((key.to_string(), val)),
        }
    }

    /// `delete $h->{key}`.
    pub fn remove(&mut self, key: &str) -> Option<XVal> {
        let idx = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(idx).1)
    }

    /// Number of keys.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// No keys.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over (key, value).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &XVal)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// `$h->{key}` as a string scalar (`None` for undef or a reference).
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(XVal::as_str)
    }

    /// `$h->{content}`.
    pub fn content(&self) -> Option<&str> {
        self.str("content")
    }

    /// `$h->{key}->@*` in an rvalue loop: the elements of an array, nothing
    /// for undef. A non-array value is an error in Perl ("Not an ARRAY
    /// reference"); we return it as a single element so callers can decide.
    pub fn list(&self, key: &str) -> XList<'_> {
        XList::of(self.get(key))
    }

    /// `$h->{key}[0]` where `key` was force-arrayed.
    pub fn first(&self, key: &str) -> Option<&XVal> {
        match self.get(key)? {
            XVal::Array(a) => a.first(),
            v => Some(v),
        }
    }

    /// `$h->{key}[0]` as a string.
    pub fn first_str(&self, key: &str) -> Option<&str> {
        self.first(key).and_then(XVal::as_str)
    }

    /// `$h->{key}` as a hash.
    pub fn hash(&self, key: &str) -> Option<&XHash> {
        self.get(key).and_then(XVal::as_hash)
    }
}

/// The elements of an `XVal` viewed as a Perl list dereference.
pub struct XList<'a> {
    items: &'a [XVal],
}

impl<'a> XList<'a> {
    fn of(v: Option<&'a XVal>) -> Self {
        match v {
            None => XList { items: &[] },
            Some(XVal::Array(a)) => XList { items: a },
            Some(other) => XList {
                items: std::slice::from_ref(other),
            },
        }
    }

    /// Iterate over all elements.
    pub fn iter(&self) -> std::slice::Iter<'a, XVal> {
        self.items.iter()
    }

    /// Iterate over the hash elements (Perl code that does `$_->{x}` on each
    /// element; strings yield an empty hash, matching `undef` lookups).
    pub fn hashes(&self) -> impl Iterator<Item = &'a XHash> + 'a {
        static EMPTY: XHash = XHash(Vec::new());
        self.items.iter().map(|v| v.as_hash().unwrap_or(&EMPTY))
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// No elements.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl XVal {
    /// The string, if this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            XVal::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The hash, if this is a hash.
    pub fn as_hash(&self) -> Option<&XHash> {
        match self {
            XVal::Hash(h) => Some(h),
            _ => None,
        }
    }

    /// Convert to a JSON value in the shape Perl's JSON encoder would give
    /// (all scalars as strings).
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            XVal::Str(s) => serde_json::Value::String(s.clone()),
            XVal::Array(a) => serde_json::Value::Array(a.iter().map(XVal::to_json).collect()),
            XVal::Hash(h) => {
                let mut m = serde_json::Map::new();
                for (k, v) in h.iter() {
                    m.insert(k.to_string(), v.to_json());
                }
                serde_json::Value::Object(m)
            }
        }
    }
}

/// A `ForceArray` specification: a predicate on element/attribute local names.
pub type ForceArray = fn(&str) -> bool;

/// Error from parsing XML.
#[derive(Debug, Clone)]
pub struct XmlError(pub String);

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for XmlError {}

/// Parsing options for roxmltree matching XML::LibXML's (entities expanded,
/// no external DTD).
fn parse_opts<'a>() -> roxmltree::ParsingOptions<'a> {
    roxmltree::ParsingOptions {
        allow_dtd: true,
        nodes_limit: u32::MAX,
        ..roxmltree::ParsingOptions::default()
    }
}

/// Check that `text` is well-formed XML (`XML::LibXML->load_xml`).
pub fn check_well_formed(text: &str) -> Result<(), XmlError> {
    roxmltree::Document::parse_with_options(strip_bom(text), parse_opts())
        .map(|_| ())
        .map_err(|e| XmlError(e.to_string()))
}

fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// `XMLin($text, ForceContent => 1, NsStrip => 1, KeyAttr => [], ForceArray => …)`.
/// Returns the collapsed root element.
pub fn xmlin(text: &str, force_array: ForceArray) -> Result<XVal, XmlError> {
    let doc = roxmltree::Document::parse_with_options(strip_bom(text), parse_opts())
        .map_err(|e| XmlError(e.to_string()))?;
    Ok(collapse(doc.root_element(), force_array))
}

fn add_kv(data: &mut XHash, k: &str, v: XVal, force_array: ForceArray) {
    if let Some(existing) = data.get_mut(k) {
        // Combine duplicates into an array.
        match existing {
            XVal::Array(a) => a.push(v),
            other => {
                let prev = std::mem::replace(other, XVal::Array(Vec::new()));
                *other = XVal::Array(vec![prev, v]);
            }
        }
    } else if force_array(k) {
        data.0.push((k.to_string(), XVal::Array(vec![v])));
    } else {
        data.0.push((k.to_string(), v));
    }
}

fn collapse(node: roxmltree::Node<'_, '_>, force_array: ForceArray) -> XVal {
    let mut data = XHash::default();
    for attr in node.attributes() {
        add_kv(
            &mut data,
            attr.name(),
            XVal::Str(attr.value().to_string()),
            force_array,
        );
    }
    let nr_attrs = data.len();
    let mut nr_elems = 0usize;
    let mut text: Option<String> = None;
    for child in node.children() {
        if child.is_text() {
            text.get_or_insert_with(String::new)
                .push_str(child.text().unwrap_or(""));
            continue;
        }
        if !child.is_element() {
            continue;
        }
        nr_elems += 1;
        let v = collapse(child, force_array);
        add_kv(&mut data, child.tag_name().name(), v, force_array);
    }
    if nr_attrs + nr_elems == 0 {
        return match text {
            Some(t) => XVal::Hash(XHash(vec![("content".to_string(), XVal::Str(t))])),
            None => XVal::Hash(XHash::default()),
        };
    }
    if let Some(t) = text
        && nr_elems == 0
    {
        data.insert("content", XVal::Str(t));
    }
    XVal::Hash(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fa(n: &str) -> bool {
        matches!(n, "field" | "item")
    }

    #[test]
    fn collapse_rules() {
        let x = xmlin(
            r#"<r xmlns:b="u"><b:a>t</b:a><b:a2/><b:item field="f"><b:field>x</b:field></b:item><b:item>  </b:item><b:dup>1</b:dup><b:dup>2</b:dup><b:mixed>a<b:c/>b</b:mixed></r>"#,
            fa,
        )
        .unwrap();
        let h = x.as_hash().unwrap();
        assert_eq!(h.hash("a").unwrap().content(), Some("t"));
        assert!(h.hash("a2").unwrap().is_empty());
        let items: Vec<_> = h.list("item").hashes().collect();
        assert_eq!(items.len(), 2);
        // attribute first, then element, merged into one array
        let f: Vec<_> = items[0].list("field").iter().cloned().collect();
        assert_eq!(f[0], XVal::Str("f".into()));
        assert_eq!(f[1].as_hash().unwrap().content(), Some("x"));
        assert_eq!(items[1].content(), Some("  "));
        assert_eq!(h.list("dup").len(), 2);
        assert_eq!(h.hash("mixed").unwrap().content(), None);
    }
}

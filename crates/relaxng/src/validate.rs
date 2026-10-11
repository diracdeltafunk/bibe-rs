//! Validation by derivatives (James Clark, *An algorithm for RELAX NG
//! validation*).
//!
//! The document is consumed one event at a time: start-tag open
//! (`start_tag_open_deriv`), each attribute (`att_deriv`), start-tag
//! close (`start_tag_close_deriv`), the children (`text_deriv` and
//! recursion), end tag (`end_tag_deriv`). A document is valid iff no
//! derivative is `notAllowed` (the final pattern is then nullable).
//!
//! When a derivative is `notAllowed`, an error is recorded and validation
//! recovers so that later errors are found too: an unexpected element is
//! skipped, an invalid attribute or text is ignored, missing attributes
//! are treated as present, and an element whose content is incomplete is
//! closed anyway. Recovery never hides the first error, so the verdict is
//! that of the plain algorithm.

use std::collections::HashMap;

use roxmltree::{Document, Node};

use crate::ValidityError;
use crate::datatype::DataType;
use crate::pattern::{Compiled, EMPTY, Id, NOT_ALLOWED, Pat, TEXT};
use crate::schema::NameClass;

#[derive(Clone, Copy)]
enum AfterFn {
    /// `x -> interleave(x, p)`
    FlipInterleave(Id),
    /// `x -> interleave(p, x)`
    Interleave(Id),
    /// `x -> group(x, p)`
    FlipGroup(Id),
    /// `x -> after(x, p)`
    FlipAfter(Id),
}

struct Validator<'c, 'd> {
    c: &'c Compiled,
    a: crate::pattern::Arena,
    names: HashMap<(&'d str, &'d str), u32>,
    stod_memo: HashMap<(Id, u32), Id>,
    stcd_memo: HashMap<Id, Id>,
    etd_memo: HashMap<Id, Id>,
    errors: Vec<ValidityError>,
}

fn is_ws(s: &str) -> bool {
    s.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
}

fn line(n: Node<'_, '_>) -> u32 {
    n.document().text_pos_at(n.range().start).row
}

enum Child<'a, 'i> {
    Elem(Node<'a, 'i>),
    Text(String),
}

/// The children of `n`: elements, and text with adjacent text nodes merged
/// (comments and processing instructions are invisible).
fn children<'a, 'i>(n: Node<'a, 'i>) -> Vec<Child<'a, 'i>> {
    let mut out: Vec<Child<'a, 'i>> = Vec::new();
    for c in n.children() {
        if c.is_element() {
            out.push(Child::Elem(c));
        } else if c.is_text() {
            let t = c.text().unwrap_or("");
            if let Some(Child::Text(s)) = out.last_mut() {
                s.push_str(t);
            } else {
                out.push(Child::Text(t.to_string()));
            }
        }
    }
    out
}

pub(crate) fn validate(c: &Compiled, doc: &Document<'_>) -> Vec<ValidityError> {
    let mut v = Validator {
        c,
        a: c.arena.clone(),
        names: HashMap::new(),
        stod_memo: HashMap::new(),
        stcd_memo: HashMap::new(),
        etd_memo: HashMap::new(),
        errors: Vec::new(),
    };
    let root = doc.root_element();
    let p = v.element(c.start, root);
    if v.errors.is_empty() && !v.a.nullable(p) {
        v.error(root, "Document is incomplete".into());
    }
    v.errors
}

impl<'c, 'd> Validator<'c, 'd> {
    fn error(&mut self, n: Node<'_, '_>, message: String) {
        self.errors.push(ValidityError {
            line: line(n),
            element: n.tag_name().name().to_string(),
            message,
        });
    }

    fn name_id(&mut self, ns: &'d str, local: &'d str) -> u32 {
        let next = self.names.len() as u32;
        *self.names.entry((ns, local)).or_insert(next)
    }

    fn contains(&self, nc: u32, ns: &str, local: &str) -> bool {
        self.c.ncs[nc as usize].contains(ns, local)
    }

    // -----------------------------------------------------------------
    // Derivatives

    fn text_deriv(&mut self, p: Id, s: &str, ctx: &dyn Fn(&str) -> Option<String>) -> Id {
        match self.a.get(p) {
            Pat::Choice(x, y) => {
                let (x, y) = (self.text_deriv(x, s, ctx), self.text_deriv(y, s, ctx));
                self.a.choice(x, y)
            }
            Pat::Interleave(x, y) => {
                let dx = self.text_deriv(x, s, ctx);
                let l = self.a.interleave(dx, y);
                let dy = self.text_deriv(y, s, ctx);
                let r = self.a.interleave(x, dy);
                self.a.choice(l, r)
            }
            Pat::Group(x, y) => {
                let dx = self.text_deriv(x, s, ctx);
                let g = self.a.group(dx, y);
                if self.a.nullable(x) {
                    let dy = self.text_deriv(y, s, ctx);
                    self.a.choice(g, dy)
                } else {
                    g
                }
            }
            Pat::After(x, y) => {
                let dx = self.text_deriv(x, s, ctx);
                self.a.after(dx, y)
            }
            Pat::OneOrMore(x) => {
                let dx = self.text_deriv(x, s, ctx);
                let rest = self.a.choice(p, EMPTY);
                self.a.group(dx, rest)
            }
            Pat::Text => TEXT,
            Pat::Value(v) => {
                if DataType::equal(&self.c.vals[v as usize], s, ctx) {
                    EMPTY
                } else {
                    NOT_ALLOWED
                }
            }
            Pat::Data(d, ex) => {
                let ok = self.c.dts[d as usize].allows(s, ctx)
                    && !ex.is_some_and(|e| {
                        let de = self.text_deriv(e, s, ctx);
                        self.a.nullable(de)
                    });
                if ok { EMPTY } else { NOT_ALLOWED }
            }
            Pat::List(x) => {
                let mut q = x;
                for w in s.split([' ', '\t', '\n', '\r']).filter(|w| !w.is_empty()) {
                    q = self.text_deriv(q, w, ctx);
                }
                if self.a.nullable(q) { EMPTY } else { NOT_ALLOWED }
            }
            _ => NOT_ALLOWED,
        }
    }

    fn apply_after(&mut self, p: Id, f: AfterFn) -> Id {
        match self.a.get(p) {
            Pat::After(x, y) => {
                let fy = match f {
                    AfterFn::FlipInterleave(q) => self.a.interleave(y, q),
                    AfterFn::Interleave(q) => self.a.interleave(q, y),
                    AfterFn::FlipGroup(q) => self.a.group(y, q),
                    AfterFn::FlipAfter(q) => self.a.after(y, q),
                };
                self.a.after(x, fy)
            }
            Pat::Choice(x, y) => {
                let (x, y) = (self.apply_after(x, f), self.apply_after(y, f));
                self.a.choice(x, y)
            }
            _ => NOT_ALLOWED,
        }
    }

    fn stod(&mut self, p: Id, name: u32, ns: &str, local: &str) -> Id {
        if let Some(&r) = self.stod_memo.get(&(p, name)) {
            return r;
        }
        let r = match self.a.get(p) {
            Pat::Choice(x, y) => {
                let (x, y) = (self.stod(x, name, ns, local), self.stod(y, name, ns, local));
                self.a.choice(x, y)
            }
            Pat::Element(nc, e) => {
                if self.contains(nc, ns, local) {
                    let content = self.c.elems[e as usize].1;
                    self.a.after(content, EMPTY)
                } else {
                    NOT_ALLOWED
                }
            }
            Pat::Interleave(x, y) => {
                let dx = self.stod(x, name, ns, local);
                let l = self.apply_after(dx, AfterFn::FlipInterleave(y));
                let dy = self.stod(y, name, ns, local);
                let r = self.apply_after(dy, AfterFn::Interleave(x));
                self.a.choice(l, r)
            }
            Pat::OneOrMore(x) => {
                let dx = self.stod(x, name, ns, local);
                let rest = self.a.choice(p, EMPTY);
                self.apply_after(dx, AfterFn::FlipGroup(rest))
            }
            Pat::Group(x, y) => {
                let dx = self.stod(x, name, ns, local);
                let g = self.apply_after(dx, AfterFn::FlipGroup(y));
                if self.a.nullable(x) {
                    let dy = self.stod(y, name, ns, local);
                    self.a.choice(g, dy)
                } else {
                    g
                }
            }
            Pat::After(x, y) => {
                let dx = self.stod(x, name, ns, local);
                self.apply_after(dx, AfterFn::FlipAfter(y))
            }
            _ => NOT_ALLOWED,
        };
        self.stod_memo.insert((p, name), r);
        r
    }

    fn att_deriv(
        &mut self,
        p: Id,
        ns: &str,
        local: &str,
        value: &str,
        ctx: &dyn Fn(&str) -> Option<String>,
    ) -> Id {
        match self.a.get(p) {
            Pat::After(x, y) => {
                let dx = self.att_deriv(x, ns, local, value, ctx);
                self.a.after(dx, y)
            }
            Pat::Choice(x, y) => {
                let dx = self.att_deriv(x, ns, local, value, ctx);
                let dy = self.att_deriv(y, ns, local, value, ctx);
                self.a.choice(dx, dy)
            }
            Pat::Group(x, y) => {
                let dx = self.att_deriv(x, ns, local, value, ctx);
                let l = self.a.group(dx, y);
                let dy = self.att_deriv(y, ns, local, value, ctx);
                let r = self.a.group(x, dy);
                self.a.choice(l, r)
            }
            Pat::Interleave(x, y) => {
                let dx = self.att_deriv(x, ns, local, value, ctx);
                let l = self.a.interleave(dx, y);
                let dy = self.att_deriv(y, ns, local, value, ctx);
                let r = self.a.interleave(x, dy);
                self.a.choice(l, r)
            }
            Pat::OneOrMore(x) => {
                let dx = self.att_deriv(x, ns, local, value, ctx);
                let rest = self.a.choice(p, EMPTY);
                self.a.group(dx, rest)
            }
            Pat::Attribute(nc, x) => {
                if self.contains(nc, ns, local) && self.value_match(x, value, ctx) {
                    EMPTY
                } else {
                    NOT_ALLOWED
                }
            }
            _ => NOT_ALLOWED,
        }
    }

    fn value_match(&mut self, p: Id, s: &str, ctx: &dyn Fn(&str) -> Option<String>) -> bool {
        (self.a.nullable(p) && is_ws(s)) || {
            let d = self.text_deriv(p, s, ctx);
            self.a.nullable(d)
        }
    }

    /// `startTagCloseDeriv`; with `lenient`, remaining attribute patterns
    /// count as satisfied (error recovery).
    fn stcd(&mut self, p: Id, lenient: bool) -> Id {
        if !lenient && let Some(&r) = self.stcd_memo.get(&p) {
            return r;
        }
        let r = match self.a.get(p) {
            Pat::After(x, y) => {
                let dx = self.stcd(x, lenient);
                self.a.after(dx, y)
            }
            Pat::Choice(x, y) => {
                let (x, y) = (self.stcd(x, lenient), self.stcd(y, lenient));
                self.a.choice(x, y)
            }
            Pat::Group(x, y) => {
                let (x, y) = (self.stcd(x, lenient), self.stcd(y, lenient));
                self.a.group(x, y)
            }
            Pat::Interleave(x, y) => {
                let (x, y) = (self.stcd(x, lenient), self.stcd(y, lenient));
                self.a.interleave(x, y)
            }
            Pat::OneOrMore(x) => {
                let x = self.stcd(x, lenient);
                self.a.one_or_more(x)
            }
            Pat::Attribute(..) => {
                if lenient {
                    EMPTY
                } else {
                    NOT_ALLOWED
                }
            }
            _ => p,
        };
        if !lenient {
            self.stcd_memo.insert(p, r);
        }
        r
    }

    fn etd(&mut self, p: Id) -> Id {
        if let Some(&r) = self.etd_memo.get(&p) {
            return r;
        }
        let r = match self.a.get(p) {
            Pat::Choice(x, y) => {
                let (x, y) = (self.etd(x), self.etd(y));
                self.a.choice(x, y)
            }
            Pat::After(x, y) => {
                if self.a.nullable(x) {
                    y
                } else {
                    NOT_ALLOWED
                }
            }
            _ => NOT_ALLOWED,
        };
        self.etd_memo.insert(p, r);
        r
    }

    /// Error recovery: close the element whatever its content.
    fn after_skip(&mut self, p: Id) -> Id {
        match self.a.get(p) {
            Pat::After(_, y) => y,
            Pat::Choice(x, y) => {
                let (x, y) = (self.after_skip(x), self.after_skip(y));
                self.a.choice(x, y)
            }
            _ => NOT_ALLOWED,
        }
    }

    // -----------------------------------------------------------------
    // Diagnostics

    /// The name classes of the elements that could come next in `p`.
    fn expected_elements(&self, p: Id, out: &mut Vec<String>, depth: u32) {
        if depth > 200 {
            return;
        }
        match self.a.get(p) {
            Pat::Choice(x, y) | Pat::Interleave(x, y) => {
                self.expected_elements(x, out, depth + 1);
                self.expected_elements(y, out, depth + 1);
            }
            Pat::Group(x, y) => {
                self.expected_elements(x, out, depth + 1);
                if self.a.nullable(x) {
                    self.expected_elements(y, out, depth + 1);
                }
            }
            Pat::OneOrMore(x) | Pat::After(x, _) => self.expected_elements(x, out, depth + 1),
            Pat::Element(nc, _) => {
                let d = self.c.ncs[nc as usize].describe();
                if !out.contains(&d) {
                    out.push(d);
                }
            }
            _ => {}
        }
    }

    /// The attributes still required by `p` (after the start tag).
    fn missing_attributes(&self, p: Id, out: &mut Vec<String>, depth: u32) {
        if depth > 200 {
            return;
        }
        match self.a.get(p) {
            Pat::Choice(x, y) | Pat::Interleave(x, y) | Pat::Group(x, y) => {
                self.missing_attributes(x, out, depth + 1);
                self.missing_attributes(y, out, depth + 1);
            }
            Pat::OneOrMore(x) | Pat::After(x, _) => self.missing_attributes(x, out, depth + 1),
            Pat::Attribute(nc, _) => {
                let d = match &self.c.ncs[nc as usize] {
                    NameClass::Name { local, .. } => local.clone(),
                    other => other.describe(),
                };
                if !out.contains(&d) {
                    out.push(d);
                }
            }
            _ => {}
        }
    }

    /// Whether `p` could accept some text next.
    fn accepts_text(&self, p: Id, depth: u32) -> bool {
        if depth > 200 {
            return false;
        }
        match self.a.get(p) {
            Pat::Text | Pat::Data(..) | Pat::Value(_) | Pat::List(_) => true,
            Pat::Choice(x, y) | Pat::Interleave(x, y) => {
                self.accepts_text(x, depth + 1) || self.accepts_text(y, depth + 1)
            }
            Pat::Group(x, y) => {
                self.accepts_text(x, depth + 1)
                    || (self.a.nullable(x) && self.accepts_text(y, depth + 1))
            }
            Pat::OneOrMore(x) | Pat::After(x, _) => self.accepts_text(x, depth + 1),
            _ => false,
        }
    }

    // -----------------------------------------------------------------
    // The document walk

    fn element(&mut self, p: Id, node: Node<'d, '_>) -> Id
    where
        'c: 'd,
    {
        let tn = node.tag_name();
        let ns: &'d str = tn.namespace().unwrap_or("");
        let local: &'d str = tn.name();
        let nid = self.name_id(ns, local);
        let p1 = self.stod(p, nid, ns, local);
        if p1 == NOT_ALLOWED {
            let mut exp = Vec::new();
            self.expected_elements(p, &mut exp, 0);
            let msg = if exp.len() == 1 {
                format!("Expecting element {}, got {local}", exp[0])
            } else {
                format!("Did not expect element {local} there")
            };
            self.error(node, msg);
            return p;
        }
        let ctx = move |prefix: &str| {
            node.lookup_namespace_uri(if prefix.is_empty() { None } else { Some(prefix) })
                .map(str::to_string)
        };
        let mut p2 = p1;
        for att in node.attributes() {
            let q = self.att_deriv(p2, att.namespace().unwrap_or(""), att.name(), att.value(), &ctx);
            if q == NOT_ALLOWED {
                self.error(node, format!("Invalid attribute {} for element {local}", att.name()));
            } else {
                p2 = q;
            }
        }
        let mut p3 = self.stcd(p2, false);
        if p3 == NOT_ALLOWED {
            let mut miss = Vec::new();
            self.missing_attributes(p2, &mut miss, 0);
            let mut msg = format!("Element {local} failed to validate attributes");
            if !miss.is_empty() {
                msg.push_str(&format!(": missing attribute {}", miss.join(" or ")));
            }
            self.error(node, msg);
            p3 = self.stcd(p2, true);
            if p3 == NOT_ALLOWED {
                return self.after_skip(p1);
            }
        }
        let p4 = self.children(p3, node, &ctx);
        let p5 = self.etd(p4);
        if p5 == NOT_ALLOWED {
            let mut exp = Vec::new();
            self.expected_elements(p4, &mut exp, 0);
            let mut msg = format!("Element {local} failed to validate content");
            if exp.len() == 1 {
                msg.push_str(&format!(": expecting an element {}, got nothing", exp[0]));
            } else if !exp.is_empty() {
                msg.push_str(&format!(": expecting one of the elements {}", exp.join(", ")));
            } else if self.accepts_text(p4, 0) {
                msg.push_str(": expecting text");
            }
            self.error(node, msg);
            let r = self.after_skip(p4);
            return if r == NOT_ALLOWED { self.after_skip(p1) } else { r };
        }
        p5
    }

    fn text_error(&mut self, p: Id, node: Node<'_, '_>, s: &str) {
        let local = node.tag_name().name();
        let msg = if self.accepts_text(p, 0) {
            let t = s.trim();
            let t: String = if t.chars().count() > 60 {
                t.chars().take(57).chain("...".chars()).collect()
            } else {
                t.to_string()
            };
            format!("Element {local} failed to validate content: invalid value '{t}'")
        } else {
            format!("Element {local} has extra content: text")
        };
        self.error(node, msg);
    }

    fn children(&mut self, p: Id, node: Node<'d, '_>, ctx: &dyn Fn(&str) -> Option<String>) -> Id
    where
        'c: 'd,
    {
        let kids = children(node);
        if kids.is_empty() || (kids.len() == 1 && matches!(kids[0], Child::Text(_))) {
            let s = match kids.first() {
                Some(Child::Text(s)) => s.as_str(),
                _ => "",
            };
            let p1 = self.text_deriv(p, s, ctx);
            let r = if is_ws(s) { self.a.choice(p, p1) } else { p1 };
            if r == NOT_ALLOWED {
                self.text_error(p, node, s);
                return p;
            }
            return r;
        }
        let mut p = p;
        for k in kids {
            match k {
                Child::Text(s) => {
                    if is_ws(&s) {
                        continue;
                    }
                    let q = self.text_deriv(p, &s, ctx);
                    if q == NOT_ALLOWED {
                        self.text_error(p, node, &s);
                    } else {
                        p = q;
                    }
                }
                Child::Elem(e) => p = self.element(p, e),
            }
        }
        p
    }
}

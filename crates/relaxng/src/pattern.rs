//! Interned (hash-consed) patterns, and the compilation of the simplified
//! syntax into them (specification §4.19–4.20, §7).
//!
//! Patterns live in an [`Arena`]; equal patterns get equal ids, so
//! derivatives can be memoised by id (Clark, *An algorithm for RELAX NG
//! validation*, "Memoization"). The smart constructors apply the
//! `notAllowed`/`empty` identities of §4.20 and keep choices in a
//! canonical form (flattened, sorted, without duplicates).
//!
//! Element content is reached through an element table rather than a
//! direct child id, which lets recursive definitions (through elements)
//! form cycles while patterns stay hash-consed.

use std::collections::HashMap;

use crate::SchemaError;
use crate::datatype::{DataType, ValueSpec};
use crate::schema::{Ast, AstId, NameClass, SchemaAst};

pub(crate) type Id = u32;

pub(crate) const EMPTY: Id = 0;
pub(crate) const NOT_ALLOWED: Id = 1;
pub(crate) const TEXT: Id = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Pat {
    Empty,
    NotAllowed,
    Text,
    Choice(Id, Id),
    Interleave(Id, Id),
    Group(Id, Id),
    OneOrMore(Id),
    List(Id),
    /// A datatype (index into `Compiled::dts`) and an optional `except`.
    Data(u32, Option<Id>),
    /// A value (index into `Compiled::vals`).
    Value(u32),
    /// A name class (index into `Compiled::ncs`) and the value pattern.
    Attribute(u32, Id),
    /// A name class and an element (index into `Compiled::elems`).
    Element(u32, u32),
    After(Id, Id),
}

#[derive(Clone, Debug)]
pub(crate) struct Arena {
    pats: Vec<Pat>,
    nullable: Vec<bool>,
    map: HashMap<Pat, Id>,
}

impl Arena {
    fn new() -> Arena {
        let mut a = Arena {
            pats: Vec::new(),
            nullable: Vec::new(),
            map: HashMap::new(),
        };
        a.intern(Pat::Empty);
        a.intern(Pat::NotAllowed);
        a.intern(Pat::Text);
        a
    }

    pub(crate) fn get(&self, id: Id) -> Pat {
        self.pats[id as usize]
    }

    pub(crate) fn nullable(&self, id: Id) -> bool {
        self.nullable[id as usize]
    }

    pub(crate) fn intern(&mut self, p: Pat) -> Id {
        if let Some(&id) = self.map.get(&p) {
            return id;
        }
        let n = match p {
            Pat::Empty | Pat::Text => true,
            Pat::Choice(a, b) => self.nullable(a) || self.nullable(b),
            Pat::Group(a, b) | Pat::Interleave(a, b) => self.nullable(a) && self.nullable(b),
            Pat::OneOrMore(a) => self.nullable(a),
            _ => false,
        };
        let id = self.pats.len() as Id;
        self.pats.push(p);
        self.nullable.push(n);
        self.map.insert(p, id);
        id
    }

    fn leaves(&self, id: Id, out: &mut Vec<Id>) {
        match self.get(id) {
            Pat::Choice(a, b) => {
                self.leaves(a, out);
                self.leaves(b, out);
            }
            _ => out.push(id),
        }
    }

    pub(crate) fn choice(&mut self, a: Id, b: Id) -> Id {
        if a == NOT_ALLOWED || a == b {
            return b;
        }
        if b == NOT_ALLOWED {
            return a;
        }
        let mut v = Vec::new();
        self.leaves(a, &mut v);
        self.leaves(b, &mut v);
        v.sort_unstable();
        v.dedup();
        let mut acc = v.pop().expect("non-empty");
        while let Some(x) = v.pop() {
            acc = self.intern(Pat::Choice(x, acc));
        }
        acc
    }

    pub(crate) fn group(&mut self, a: Id, b: Id) -> Id {
        if a == NOT_ALLOWED || b == NOT_ALLOWED {
            NOT_ALLOWED
        } else if a == EMPTY {
            b
        } else if b == EMPTY {
            a
        } else {
            self.intern(Pat::Group(a, b))
        }
    }

    pub(crate) fn interleave(&mut self, a: Id, b: Id) -> Id {
        if a == NOT_ALLOWED || b == NOT_ALLOWED {
            NOT_ALLOWED
        } else if a == EMPTY {
            b
        } else if b == EMPTY {
            a
        } else {
            self.intern(Pat::Interleave(a, b))
        }
    }

    pub(crate) fn after(&mut self, a: Id, b: Id) -> Id {
        if a == NOT_ALLOWED || b == NOT_ALLOWED {
            NOT_ALLOWED
        } else {
            self.intern(Pat::After(a, b))
        }
    }

    pub(crate) fn one_or_more(&mut self, a: Id) -> Id {
        if a == NOT_ALLOWED || a == EMPTY {
            a
        } else {
            self.intern(Pat::OneOrMore(a))
        }
    }

    fn list(&mut self, a: Id) -> Id {
        if a == NOT_ALLOWED {
            a
        } else {
            self.intern(Pat::List(a))
        }
    }

    fn attribute(&mut self, nc: u32, a: Id) -> Id {
        if a == NOT_ALLOWED {
            a
        } else {
            self.intern(Pat::Attribute(nc, a))
        }
    }
}

/// A compiled schema.
#[derive(Clone, Debug)]
pub(crate) struct Compiled {
    pub(crate) arena: Arena,
    pub(crate) start: Id,
    pub(crate) ncs: Vec<NameClass>,
    /// Per element pattern: its name class and its content.
    pub(crate) elems: Vec<(u32, Id)>,
    pub(crate) dts: Vec<DataType>,
    pub(crate) vals: Vec<ValueSpec>,
    /// The groups (operands) of each n-ary interleave of the schema.
    pub(crate) interleaves: HashMap<Id, Vec<Id>>,
}

#[derive(Clone, Copy)]
enum DefState {
    New,
    InProgress,
    Done(Id),
}

struct Compiler<'a> {
    ast: &'a SchemaAst,
    c: Compiled,
    defs: Vec<DefState>,
    work: Vec<(u32, AstId)>,
}

fn serr(line: Option<u32>, message: String) -> SchemaError {
    SchemaError { message, line }
}

/// Compile a simplified schema (after checking §7's restrictions).
pub(crate) fn compile(ast: &SchemaAst) -> Result<Compiled, SchemaError> {
    check::check(ast)?;
    let mut k = Compiler {
        ast,
        c: Compiled {
            arena: Arena::new(),
            start: NOT_ALLOWED,
            ncs: Vec::new(),
            elems: Vec::new(),
            dts: Vec::new(),
            vals: Vec::new(),
            interleaves: HashMap::new(),
        },
        defs: vec![DefState::New; ast.defs.len()],
        work: Vec::new(),
    };
    k.c.start = k.pat(ast.start)?;
    while let Some((e, body)) = k.work.pop() {
        let p = k.pat(body)?;
        k.c.elems[e as usize].1 = p;
    }
    Ok(k.c)
}

impl Compiler<'_> {
    fn nc(&mut self, nc: &NameClass) -> u32 {
        if let Some(i) = self.c.ncs.iter().position(|x| x == nc) {
            return i as u32;
        }
        self.c.ncs.push(nc.clone());
        (self.c.ncs.len() - 1) as u32
    }

    fn pat(&mut self, a: AstId) -> Result<Id, SchemaError> {
        if let Some(ops) = self.ast.nary.get(&a) {
            let mut ids = Vec::with_capacity(ops.len());
            for &o in ops {
                ids.push(self.pat(o)?);
            }
            let mut acc = ids[0];
            for &x in &ids[1..] {
                acc = self.c.arena.interleave(acc, x);
            }
            if matches!(self.c.arena.get(acc), Pat::Interleave(..)) {
                let groups: Vec<Id> = ids.into_iter().filter(|&g| g != EMPTY).collect();
                self.c.interleaves.entry(acc).or_insert(groups);
            }
            return Ok(acc);
        }
        Ok(match &self.ast.nodes[a] {
            Ast::Empty => EMPTY,
            Ast::NotAllowed => NOT_ALLOWED,
            Ast::Text => TEXT,
            Ast::Choice(x, y) => {
                let (x, y) = (self.pat(*x)?, self.pat(*y)?);
                self.c.arena.choice(x, y)
            }
            Ast::Group(x, y) => {
                let (x, y) = (self.pat(*x)?, self.pat(*y)?);
                self.c.arena.group(x, y)
            }
            Ast::Interleave(x, y) => {
                let (x, y) = (self.pat(*x)?, self.pat(*y)?);
                self.c.arena.interleave(x, y)
            }
            Ast::OneOrMore(x) => {
                let x = self.pat(*x)?;
                self.c.arena.one_or_more(x)
            }
            Ast::List(x) => {
                let x = self.pat(*x)?;
                self.c.arena.list(x)
            }
            Ast::Data {
                lib,
                ty,
                params,
                except,
                line,
            } => {
                let dt = DataType::new(lib, ty, params).map_err(|m| serr(Some(*line), m))?;
                self.c.dts.push(dt);
                let d = (self.c.dts.len() - 1) as u32;
                let ex = match except {
                    Some(e) => Some(self.pat(*e)?).filter(|&e| e != NOT_ALLOWED),
                    None => None,
                };
                self.c.arena.intern(Pat::Data(d, ex))
            }
            Ast::Value {
                lib,
                ty,
                value,
                ctx,
                line,
            } => {
                let v = ValueSpec::new(lib, ty, value, ctx).map_err(|m| serr(Some(*line), m))?;
                self.c.vals.push(v);
                let i = (self.c.vals.len() - 1) as u32;
                self.c.arena.intern(Pat::Value(i))
            }
            Ast::Attribute(nc, x) => {
                let n = self.nc(nc);
                let x = self.pat(*x)?;
                self.c.arena.attribute(n, x)
            }
            Ast::Element(nc, x) => {
                let n = self.nc(nc);
                let e = self.c.elems.len() as u32;
                self.c.elems.push((n, NOT_ALLOWED));
                self.work.push((e, *x));
                self.c.arena.intern(Pat::Element(n, e))
            }
            Ast::Ref(d) => match self.defs[*d] {
                DefState::Done(id) => id,
                DefState::InProgress => {
                    return Err(serr(
                        None,
                        format!(
                            "recursive reference to '{}' that is not inside an element",
                            self.ast.defs[*d].name
                        ),
                    ));
                }
                DefState::New => {
                    self.defs[*d] = DefState::InProgress;
                    let body = self.ast.defs[*d].body.expect("checked when the grammar ended");
                    let id = self.pat(body)?;
                    self.defs[*d] = DefState::Done(id);
                    id
                }
            },
        })
    }
}

/// The restrictions of specification §7 on the simplified syntax.
pub(crate) mod check {
    use std::collections::{HashMap, HashSet};

    use crate::SchemaError;
    use crate::schema::{Ast, AstId, NameClass, SchemaAst};

    /// §7.2 content types.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Ct {
        Empty,
        Complex,
        Simple,
    }

    fn groupable(a: Ct, b: Ct) -> bool {
        a == Ct::Empty || b == Ct::Empty || (a == Ct::Complex && b == Ct::Complex)
    }

    /// What a pattern contributes, looking through definitions but not into
    /// elements or attributes.
    #[derive(Clone, Default)]
    struct Info {
        /// Content type (`None`: no content type, i.e. not allowed).
        ct: Option<Ct>,
        elements: Vec<NameClass>,
        attributes: Vec<NameClass>,
        text: bool,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
    struct Flags {
        in_attribute: bool,
        in_one_or_more: bool,
        in_one_or_more_group: bool,
        in_list: bool,
        in_except: bool,
        in_start: bool,
    }

    struct Checker<'a> {
        ast: &'a SchemaAst,
        info: HashMap<usize, Info>,
        visiting: HashSet<usize>,
        seen: HashSet<(AstId, Flags)>,
    }

    fn e(msg: impl Into<String>) -> SchemaError {
        SchemaError {
            message: msg.into(),
            line: None,
        }
    }

    pub(crate) fn check(ast: &SchemaAst) -> Result<(), SchemaError> {
        let mut c = Checker {
            ast,
            info: HashMap::new(),
            visiting: HashSet::new(),
            seen: HashSet::new(),
        };
        let start_flags = Flags {
            in_start: true,
            ..Flags::default()
        };
        c.paths(ast.start, start_flags)?;
        Ok(())
    }

    /// Representative names for the overlap test of two name classes.
    fn reps(nc: &NameClass, out: &mut Vec<(String, String)>) {
        const ILLEGAL: &str = "\u{0}";
        match nc {
            NameClass::Name { ns, local } => out.push((ns.clone(), local.clone())),
            NameClass::AnyName(ex) => {
                out.push((ILLEGAL.into(), ILLEGAL.into()));
                if let Some(x) = ex {
                    reps(x, out);
                }
            }
            NameClass::NsName(ns, ex) => {
                out.push((ns.clone(), ILLEGAL.into()));
                if let Some(x) = ex {
                    reps(x, out);
                }
            }
            NameClass::Choice(a, b) => {
                reps(a, out);
                reps(b, out);
            }
        }
    }

    fn overlap(a: &NameClass, b: &NameClass) -> bool {
        let mut r = Vec::new();
        reps(a, &mut r);
        reps(b, &mut r);
        r.iter().any(|(ns, l)| a.contains(ns, l) && b.contains(ns, l))
    }

    fn any_overlap(a: &[NameClass], b: &[NameClass]) -> bool {
        a.iter().any(|x| b.iter().any(|y| overlap(x, y)))
    }

    impl Checker<'_> {
        /// §7.1 prohibited paths (context-dependent), and the element
        /// content checks of §7.2–7.4 for each element reached.
        fn paths(&mut self, a: AstId, f: Flags) -> Result<(), SchemaError> {
            if !self.seen.insert((a, f)) {
                return Ok(());
            }
            let simple_forbidden = |what: &str| -> Result<(), SchemaError> {
                if f.in_start {
                    return Err(e(format!("start may only contain elements, not {what}")));
                }
                Ok(())
            };
            match &self.ast.nodes[a] {
                Ast::Empty => {
                    simple_forbidden("empty")?;
                    if f.in_except {
                        return Err(e("empty is not allowed in data/except"));
                    }
                }
                Ast::NotAllowed => {}
                Ast::Text => {
                    simple_forbidden("text")?;
                    if f.in_list || f.in_except {
                        return Err(e("text is not allowed in list or data/except"));
                    }
                }
                Ast::Choice(x, y) => {
                    self.paths(*x, f)?;
                    self.paths(*y, f)?;
                }
                Ast::Group(x, y) | Ast::Interleave(x, y) => {
                    let inter = matches!(self.ast.nodes[a], Ast::Interleave(..));
                    simple_forbidden(if inter { "interleave" } else { "group" })?;
                    if f.in_except || (inter && f.in_list) {
                        return Err(e("group/interleave is not allowed here (data/except or list)"));
                    }
                    let g = Flags {
                        in_one_or_more_group: f.in_one_or_more_group || f.in_one_or_more,
                        ..f
                    };
                    self.paths(*x, g)?;
                    self.paths(*y, g)?;
                    let (ix, iy) = (self.info(*x)?, self.info(*y)?);
                    if inter {
                        if any_overlap(&ix.elements, &iy.elements) {
                            return Err(e("elements with overlapping names in both operands of an interleave"));
                        }
                        if ix.text && iy.text {
                            return Err(e("text in both operands of an interleave"));
                        }
                    }
                    if any_overlap(&ix.attributes, &iy.attributes) {
                        return Err(e("duplicate attribute names in a group or interleave"));
                    }
                }
                Ast::OneOrMore(x) => {
                    simple_forbidden("oneOrMore")?;
                    if f.in_except {
                        return Err(e("oneOrMore is not allowed in data/except"));
                    }
                    self.paths(
                        *x,
                        Flags {
                            in_one_or_more: true,
                            ..f
                        },
                    )?;
                }
                Ast::List(x) => {
                    simple_forbidden("list")?;
                    if f.in_list || f.in_except {
                        return Err(e("list is not allowed in list or data/except"));
                    }
                    self.paths(*x, Flags { in_list: true, ..f })?;
                }
                Ast::Data { except, .. } => {
                    simple_forbidden("data")?;
                    if let Some(x) = except {
                        self.paths(*x, Flags { in_except: true, ..f })?;
                    }
                }
                Ast::Value { .. } => simple_forbidden("value")?,
                Ast::Attribute(_, x) => {
                    simple_forbidden("attribute")?;
                    if f.in_attribute || f.in_list || f.in_except || f.in_one_or_more_group {
                        return Err(e(
                            "attribute is not allowed here (in an attribute, list, data/except, or a group/interleave inside oneOrMore)",
                        ));
                    }
                    self.paths(
                        *x,
                        Flags {
                            in_attribute: true,
                            ..Flags::default()
                        },
                    )?;
                }
                Ast::Element(_, x) => {
                    if f.in_attribute || f.in_list || f.in_except {
                        return Err(e("element is not allowed in an attribute, list or data/except"));
                    }
                    let i = self.info(*x)?;
                    if i.ct.is_none() && !self.not_allowed(*x) {
                        return Err(e("element content mixes data with elements or text"));
                    }
                    self.paths(*x, Flags::default())?;
                }
                Ast::Ref(d) => {
                    let body = self.ast.defs[*d].body.expect("defined");
                    self.paths(body, f)?;
                }
            }
            Ok(())
        }

        fn not_allowed(&self, a: AstId) -> bool {
            match &self.ast.nodes[a] {
                Ast::NotAllowed => true,
                Ast::Ref(d) => self.ast.defs[*d].body.is_some_and(|b| self.not_allowed(b)),
                _ => false,
            }
        }

        fn info(&mut self, a: AstId) -> Result<Info, SchemaError> {
            if let Some(i) = self.info.get(&a) {
                return Ok(i.clone());
            }
            if !self.visiting.insert(a) {
                // A recursive reference outside elements; reported by the
                // compiler.
                return Ok(Info {
                    ct: Some(Ct::Complex),
                    ..Info::default()
                });
            }
            let i = match &self.ast.nodes[a] {
                Ast::Empty => Info {
                    ct: Some(Ct::Empty),
                    ..Info::default()
                },
                Ast::NotAllowed => Info::default(),
                Ast::Text => Info {
                    ct: Some(Ct::Complex),
                    text: true,
                    ..Info::default()
                },
                Ast::Data { .. } | Ast::Value { .. } | Ast::List(_) => Info {
                    ct: Some(Ct::Simple),
                    ..Info::default()
                },
                Ast::Attribute(nc, _) => Info {
                    ct: Some(Ct::Empty),
                    attributes: vec![nc.clone()],
                    ..Info::default()
                },
                Ast::Element(nc, _) => Info {
                    ct: Some(Ct::Complex),
                    elements: vec![nc.clone()],
                    ..Info::default()
                },
                Ast::Ref(d) => {
                    let b = self.ast.defs[*d].body.expect("defined");
                    self.info(b)?
                }
                Ast::Choice(x, y) => {
                    let (ix, iy) = (self.info(*x)?, self.info(*y)?);
                    let ct = match (ix.ct, iy.ct) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        (a, None) => a,
                        (None, b) => b,
                    };
                    merge(ct, ix, iy)
                }
                Ast::Group(x, y) | Ast::Interleave(x, y) => {
                    let (ix, iy) = (self.info(*x)?, self.info(*y)?);
                    let ct = match (ix.ct, iy.ct) {
                        (Some(a), Some(b)) if groupable(a, b) => Some(a.max(b)),
                        _ => None,
                    };
                    merge(ct, ix, iy)
                }
                Ast::OneOrMore(x) => {
                    let mut ix = self.info(*x)?;
                    ix.ct = ix.ct.filter(|&c| groupable(c, c));
                    ix
                }
            };
            self.visiting.remove(&a);
            self.info.insert(a, i.clone());
            Ok(i)
        }
    }

    fn merge(ct: Option<Ct>, mut a: Info, b: Info) -> Info {
        a.ct = ct;
        a.elements.extend(b.elements);
        a.attributes.extend(b.attributes);
        a.text |= b.text;
        a
    }
}

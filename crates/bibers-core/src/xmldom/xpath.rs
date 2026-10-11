//! XPath 1.0 (W3C Recommendation, 16 November 1999) over [`Dom`],
//! implemented from the specification: the full expression grammar, all
//! axes except `namespace` (which selects nothing), and the core function
//! library.
//!
//! Prefixes in name tests are resolved against the namespace declarations
//! given with the evaluation context (biber's `$node->findnodes` passes the
//! declarations in scope at the context node; its document-level
//! `XML::LibXML::XPathContext` registers `bltx`). An unbound prefix is an
//! evaluation error ([`XPathError::UndefinedPrefix`]), raised when its step
//! is evaluated, as in libxml2.

use super::{Dom, Kind, NodeId};
use std::collections::HashMap;
use std::fmt;

/// An XPath compilation or evaluation error.
#[derive(Clone, Debug, PartialEq)]
pub enum XPathError {
    /// The expression does not parse (`Invalid expression`, …).
    Syntax {
        expr: String,
        pos: usize,
        msg: String,
    },
    /// A name test uses a prefix with no namespace binding.
    UndefinedPrefix(String),
    /// A call to an unknown function.
    UnknownFunction(String),
    /// A function called with the wrong number of arguments.
    Arity(String),
    /// A node-set was required (path step or union on a non-node-set).
    InvalidType,
    /// A variable reference (no variables are bound).
    UndefinedVariable(String),
}

impl fmt::Display for XPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XPathError::Syntax { expr, pos, msg } => {
                write!(f, "{msg} in XPath '{expr}' at offset {pos}")
            }
            XPathError::UndefinedPrefix(p) => write!(f, "Undefined namespace prefix '{p}'"),
            XPathError::UnknownFunction(n) => write!(f, "Unregistered function '{n}'"),
            XPathError::Arity(n) => write!(f, "Invalid number of arguments to '{n}'"),
            XPathError::InvalidType => f.write_str("Invalid type"),
            XPathError::UndefinedVariable(v) => write!(f, "Undefined variable '${v}'"),
        }
    }
}

impl XPathError {
    /// The message libxml2 prints for this error class.
    pub fn libxml_message(&self) -> &'static str {
        match self {
            XPathError::Syntax { .. } => "Invalid expression",
            XPathError::UndefinedPrefix(_) => "Undefined namespace prefix",
            XPathError::UnknownFunction(_) => "Unregistered function",
            XPathError::Arity(_) => "Invalid number of arguments",
            XPathError::InvalidType => "Invalid type",
            XPathError::UndefinedVariable(_) => "Undefined variable",
        }
    }
}

/// The result of an expression.
#[derive(Clone, Debug, PartialEq)]
pub enum XValue {
    /// Nodes in document order, without duplicates.
    Nodes(Vec<NodeId>),
    Bool(bool),
    Num(f64),
    Str(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Ancestor,
    AncestorOrSelf,
    Attribute,
    Child,
    Descendant,
    DescendantOrSelf,
    Following,
    FollowingSibling,
    Namespace,
    Parent,
    Preceding,
    PrecedingSibling,
    SelfAxis,
}

impl Axis {
    fn from_name(n: &str) -> Option<Axis> {
        Some(match n {
            "ancestor" => Axis::Ancestor,
            "ancestor-or-self" => Axis::AncestorOrSelf,
            "attribute" => Axis::Attribute,
            "child" => Axis::Child,
            "descendant" => Axis::Descendant,
            "descendant-or-self" => Axis::DescendantOrSelf,
            "following" => Axis::Following,
            "following-sibling" => Axis::FollowingSibling,
            "namespace" => Axis::Namespace,
            "parent" => Axis::Parent,
            "preceding" => Axis::Preceding,
            "preceding-sibling" => Axis::PrecedingSibling,
            "self" => Axis::SelfAxis,
            _ => return None,
        })
    }

    fn is_reverse(self) -> bool {
        matches!(
            self,
            Axis::Ancestor
                | Axis::AncestorOrSelf
                | Axis::Preceding
                | Axis::PrecedingSibling
                | Axis::Parent
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
enum NodeTest {
    /// `prefix:local` or `local`.
    Name(Option<String>, String),
    /// `*`
    Any,
    /// `prefix:*`
    NsAny(String),
    Node,
    Text,
    Comment,
    Pi(Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
struct Step {
    axis: Axis,
    test: NodeTest,
    preds: Vec<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Union,
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    /// A location path (`absolute`: starts at the root).
    Path {
        absolute: bool,
        steps: Vec<Step>,
    },
    /// A filter expression followed by an optional relative path.
    Filter {
        primary: Box<Expr>,
        preds: Vec<Expr>,
        steps: Vec<Step>,
    },
    Literal(String),
    Number(f64),
    Var(String),
    Func(String, Vec<Expr>),
}

// ---------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    LParen,
    RParen,
    LBrack,
    RBrack,
    Dot,
    DotDot,
    At,
    Comma,
    ColonColon,
    Slash,
    DSlash,
    Pipe,
    Plus,
    Minus,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Star,
    Literal(String),
    Number(f64),
    Var(String),
    /// An NCName or QName (`p:l`), or `p:*`.
    Name(String),
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || (c as u32) > 0x7f
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '.' || c == '-' || c == '\u{b7}'
}

fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, XPathError> {
    let err = |pos: usize, msg: &str| XPathError::Syntax {
        expr: src.to_string(),
        pos,
        msg: msg.to_string(),
    };
    let c: Vec<(usize, char)> = src.char_indices().collect();
    let at = |i: usize| c.get(i).map(|x| x.1);
    let off = |i: usize| c.get(i).map_or(src.len(), |x| x.0);
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i].1;
        let p = c[i].0;
        if is_xml_space(ch) {
            i += 1;
            continue;
        }
        let two = |t: Tok, n: usize, out: &mut Vec<(Tok, usize)>| {
            out.push((t, p));
            n
        };
        i += match ch {
            '(' => two(Tok::LParen, 1, &mut out),
            ')' => two(Tok::RParen, 1, &mut out),
            '[' => two(Tok::LBrack, 1, &mut out),
            ']' => two(Tok::RBrack, 1, &mut out),
            '@' => two(Tok::At, 1, &mut out),
            ',' => two(Tok::Comma, 1, &mut out),
            '|' => two(Tok::Pipe, 1, &mut out),
            '+' => two(Tok::Plus, 1, &mut out),
            '-' => two(Tok::Minus, 1, &mut out),
            '=' => two(Tok::Eq, 1, &mut out),
            '*' => two(Tok::Star, 1, &mut out),
            ':' if at(i + 1) == Some(':') => two(Tok::ColonColon, 2, &mut out),
            '!' if at(i + 1) == Some('=') => two(Tok::Ne, 2, &mut out),
            '<' if at(i + 1) == Some('=') => two(Tok::Le, 2, &mut out),
            '>' if at(i + 1) == Some('=') => two(Tok::Ge, 2, &mut out),
            '<' => two(Tok::Lt, 1, &mut out),
            '>' => two(Tok::Gt, 1, &mut out),
            '/' if at(i + 1) == Some('/') => two(Tok::DSlash, 2, &mut out),
            '/' => two(Tok::Slash, 1, &mut out),
            '.' if at(i + 1) == Some('.') => two(Tok::DotDot, 2, &mut out),
            '.' if !at(i + 1).is_some_and(|d| d.is_ascii_digit()) => two(Tok::Dot, 1, &mut out),
            '"' | '\'' => {
                let mut j = i + 1;
                while j < c.len() && c[j].1 != ch {
                    j += 1;
                }
                if j >= c.len() {
                    return Err(err(p, "Unfinished literal"));
                }
                out.push((Tok::Literal(src[off(i + 1)..off(j)].to_string()), p));
                j + 1 - i
            }
            d if d.is_ascii_digit() || d == '.' => {
                let mut j = i;
                while at(j).is_some_and(|d| d.is_ascii_digit()) {
                    j += 1;
                }
                if at(j) == Some('.') {
                    j += 1;
                    while at(j).is_some_and(|d| d.is_ascii_digit()) {
                        j += 1;
                    }
                }
                let s = &src[p..off(j)];
                out.push((Tok::Number(s.parse::<f64>().unwrap_or(f64::NAN)), p));
                j - i
            }
            '$' => {
                let mut j = i + 1;
                if !at(j).is_some_and(is_name_start) {
                    return Err(err(p, "Invalid expression"));
                }
                while at(j).is_some_and(|x| is_name_char(x) || x == ':') {
                    j += 1;
                }
                out.push((Tok::Var(src[off(i + 1)..off(j)].to_string()), p));
                j - i
            }
            n if is_name_start(n) => {
                let mut j = i;
                while at(j).is_some_and(is_name_char) {
                    j += 1;
                }
                // QName or prefix:*
                if at(j) == Some(':') && at(j + 1) != Some(':') {
                    if at(j + 1) == Some('*') {
                        j += 2;
                    } else if at(j + 1).is_some_and(is_name_start) {
                        j += 1;
                        while at(j).is_some_and(is_name_char) {
                            j += 1;
                        }
                    }
                }
                out.push((Tok::Name(src[p..off(j)].to_string()), p));
                j - i
            }
            _ => return Err(err(p, "Invalid expression")),
        };
    }
    Ok(out)
}

// ---------------------------------------------------------------- parser

struct Parser<'a> {
    src: &'a str,
    toks: Vec<(Tok, usize)>,
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|t| &t.0)
    }

    fn peek_at(&self, k: usize) -> Option<&Tok> {
        self.toks.get(self.i + k).map(|t| &t.0)
    }

    fn err(&self, msg: &str) -> XPathError {
        XPathError::Syntax {
            expr: self.src.to_string(),
            pos: self.toks.get(self.i).map_or(self.src.len(), |t| t.1),
            msg: msg.to_string(),
        }
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok, msg: &str) -> Result<(), XPathError> {
        if self.eat(t) {
            Ok(())
        } else {
            Err(self.err(msg))
        }
    }

    fn is_name(&self, n: &str) -> bool {
        matches!(self.peek(), Some(Tok::Name(x)) if x == n)
    }

    fn expr(&mut self) -> Result<Expr, XPathError> {
        let mut l = self.and_expr()?;
        while self.is_name("or") {
            self.i += 1;
            let r = self.and_expr()?;
            l = Expr::Bin(BinOp::Or, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn and_expr(&mut self) -> Result<Expr, XPathError> {
        let mut l = self.eq_expr()?;
        while self.is_name("and") {
            self.i += 1;
            let r = self.eq_expr()?;
            l = Expr::Bin(BinOp::And, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn eq_expr(&mut self) -> Result<Expr, XPathError> {
        let mut l = self.rel_expr()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Eq) => BinOp::Eq,
                Some(Tok::Ne) => BinOp::Ne,
                _ => return Ok(l),
            };
            self.i += 1;
            let r = self.rel_expr()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn rel_expr(&mut self) -> Result<Expr, XPathError> {
        let mut l = self.add_expr()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Lt) => BinOp::Lt,
                Some(Tok::Le) => BinOp::Le,
                Some(Tok::Gt) => BinOp::Gt,
                Some(Tok::Ge) => BinOp::Ge,
                _ => return Ok(l),
            };
            self.i += 1;
            let r = self.add_expr()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn add_expr(&mut self) -> Result<Expr, XPathError> {
        let mut l = self.mul_expr()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Add,
                Some(Tok::Minus) => BinOp::Sub,
                _ => return Ok(l),
            };
            self.i += 1;
            let r = self.mul_expr()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn mul_expr(&mut self) -> Result<Expr, XPathError> {
        let mut l = self.unary_expr()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinOp::Mul,
                Some(Tok::Name(n)) if n == "div" => BinOp::Div,
                Some(Tok::Name(n)) if n == "mod" => BinOp::Mod,
                _ => return Ok(l),
            };
            self.i += 1;
            let r = self.unary_expr()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn unary_expr(&mut self) -> Result<Expr, XPathError> {
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Neg(Box::new(self.unary_expr()?)));
        }
        self.union_expr()
    }

    fn union_expr(&mut self) -> Result<Expr, XPathError> {
        let mut l = self.path_expr()?;
        while self.eat(&Tok::Pipe) {
            let r = self.path_expr()?;
            l = Expr::Bin(BinOp::Union, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn starts_filter(&self) -> bool {
        match self.peek() {
            Some(Tok::Literal(_) | Tok::Number(_) | Tok::Var(_) | Tok::LParen) => true,
            Some(Tok::Name(n)) => {
                self.peek_at(1) == Some(&Tok::LParen)
                    && !matches!(
                        n.as_str(),
                        "node" | "text" | "comment" | "processing-instruction"
                    )
            }
            _ => false,
        }
    }

    fn path_expr(&mut self) -> Result<Expr, XPathError> {
        if self.starts_filter() {
            let primary = self.primary()?;
            let mut preds = Vec::new();
            while self.peek() == Some(&Tok::LBrack) {
                preds.push(self.predicate()?);
            }
            let mut steps = Vec::new();
            if matches!(self.peek(), Some(Tok::Slash | Tok::DSlash)) {
                self.relative_steps(&mut steps, true)?;
            }
            if preds.is_empty() && steps.is_empty() {
                return Ok(primary);
            }
            return Ok(Expr::Filter {
                primary: Box::new(primary),
                preds,
                steps,
            });
        }
        let mut steps = Vec::new();
        match self.peek() {
            Some(Tok::Slash) => {
                self.i += 1;
                if self.starts_step() {
                    self.step_into(&mut steps)?;
                    self.relative_steps(&mut steps, false)?;
                }
                Ok(Expr::Path {
                    absolute: true,
                    steps,
                })
            }
            Some(Tok::DSlash) => {
                self.relative_steps(&mut steps, true)?;
                Ok(Expr::Path {
                    absolute: true,
                    steps,
                })
            }
            _ => {
                if !self.starts_step() {
                    return Err(self.err("Invalid expression"));
                }
                self.step_into(&mut steps)?;
                self.relative_steps(&mut steps, false)?;
                Ok(Expr::Path {
                    absolute: false,
                    steps,
                })
            }
        }
    }

    /// `(('/' | '//') Step)*`; with `leading`, the first separator is
    /// mandatory and already the current token.
    fn relative_steps(&mut self, steps: &mut Vec<Step>, _leading: bool) -> Result<(), XPathError> {
        loop {
            match self.peek() {
                Some(Tok::Slash) => {
                    self.i += 1;
                }
                Some(Tok::DSlash) => {
                    self.i += 1;
                    steps.push(Step {
                        axis: Axis::DescendantOrSelf,
                        test: NodeTest::Node,
                        preds: Vec::new(),
                    });
                }
                _ => return Ok(()),
            }
            if !self.starts_step() {
                return Err(self.err("Invalid expression"));
            }
            self.step_into(steps)?;
        }
    }

    fn starts_step(&self) -> bool {
        matches!(
            self.peek(),
            Some(Tok::Dot | Tok::DotDot | Tok::At | Tok::Star | Tok::Name(_))
        )
    }

    fn step_into(&mut self, steps: &mut Vec<Step>) -> Result<(), XPathError> {
        if self.eat(&Tok::Dot) {
            steps.push(Step {
                axis: Axis::SelfAxis,
                test: NodeTest::Node,
                preds: Vec::new(),
            });
            return Ok(());
        }
        if self.eat(&Tok::DotDot) {
            steps.push(Step {
                axis: Axis::Parent,
                test: NodeTest::Node,
                preds: Vec::new(),
            });
            return Ok(());
        }
        let mut axis = Axis::Child;
        if self.eat(&Tok::At) {
            axis = Axis::Attribute;
        } else if let (Some(Tok::Name(n)), Some(Tok::ColonColon)) = (self.peek(), self.peek_at(1)) {
            axis = Axis::from_name(n).ok_or_else(|| self.err("Unknown axis"))?;
            self.i += 2;
        }
        let test = match self.peek().cloned() {
            Some(Tok::Star) => {
                self.i += 1;
                NodeTest::Any
            }
            Some(Tok::Name(n)) => {
                self.i += 1;
                if self.peek() == Some(&Tok::LParen) {
                    self.i += 1;
                    let t = match n.as_str() {
                        "node" => NodeTest::Node,
                        "text" => NodeTest::Text,
                        "comment" => NodeTest::Comment,
                        "processing-instruction" => {
                            let lit = match self.peek().cloned() {
                                Some(Tok::Literal(l)) => {
                                    self.i += 1;
                                    Some(l)
                                }
                                _ => None,
                            };
                            NodeTest::Pi(lit)
                        }
                        _ => return Err(self.err("Invalid node type")),
                    };
                    self.expect(&Tok::RParen, "Invalid expression")?;
                    t
                } else if let Some(p) = n.strip_suffix(":*") {
                    NodeTest::NsAny(p.to_string())
                } else if let Some((p, l)) = n.split_once(':') {
                    NodeTest::Name(Some(p.to_string()), l.to_string())
                } else {
                    NodeTest::Name(None, n)
                }
            }
            _ => return Err(self.err("Invalid expression")),
        };
        let mut preds = Vec::new();
        while self.peek() == Some(&Tok::LBrack) {
            preds.push(self.predicate()?);
        }
        steps.push(Step { axis, test, preds });
        Ok(())
    }

    fn predicate(&mut self) -> Result<Expr, XPathError> {
        self.expect(&Tok::LBrack, "Invalid predicate")?;
        if self.peek() == Some(&Tok::RBrack) {
            return Err(self.err("Invalid predicate"));
        }
        let e = self.expr()?;
        self.expect(&Tok::RBrack, "Invalid predicate")?;
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, XPathError> {
        match self.peek().cloned() {
            Some(Tok::Literal(l)) => {
                self.i += 1;
                Ok(Expr::Literal(l))
            }
            Some(Tok::Number(n)) => {
                self.i += 1;
                Ok(Expr::Number(n))
            }
            Some(Tok::Var(v)) => {
                self.i += 1;
                Ok(Expr::Var(v))
            }
            Some(Tok::LParen) => {
                self.i += 1;
                let e = self.expr()?;
                self.expect(&Tok::RParen, "Invalid expression")?;
                Ok(e)
            }
            Some(Tok::Name(n)) => {
                self.i += 2; // name and '('
                let mut args = Vec::new();
                if !self.eat(&Tok::RParen) {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(&Tok::Comma) {
                            continue;
                        }
                        self.expect(&Tok::RParen, "Invalid expression")?;
                        break;
                    }
                }
                Ok(Expr::Func(n, args))
            }
            _ => Err(self.err("Invalid expression")),
        }
    }
}

/// A compiled XPath expression.
#[derive(Clone, Debug, PartialEq)]
pub struct XPath {
    src: String,
    expr: Expr,
}

impl XPath {
    /// Compile `src` (`XML::LibXML::XPathExpression->new`).
    pub fn compile(src: &str) -> Result<XPath, XPathError> {
        let toks = lex(src)?;
        let mut p = Parser { src, toks, i: 0 };
        if p.peek().is_none() {
            return Err(p.err("Empty expression"));
        }
        let expr = p.expr()?;
        if p.peek().is_some() {
            return Err(p.err("Invalid expression"));
        }
        Ok(XPath {
            src: src.to_string(),
            expr,
        })
    }

    /// The expression text.
    pub fn as_str(&self) -> &str {
        &self.src
    }

    /// Evaluate with `node` as the context node; `ns` are the prefix
    /// bindings available to name tests.
    pub fn eval(
        &self,
        dom: &Dom,
        node: NodeId,
        ns: &[(Option<String>, String)],
    ) -> Result<XValue, XPathError> {
        let ev = Eval { dom, ns };
        ev.eval(&self.expr, node, 1, 1)
    }
}

// ---------------------------------------------------------------- evaluation

struct Eval<'a> {
    dom: &'a Dom,
    ns: &'a [(Option<String>, String)],
}

impl Eval<'_> {
    fn eval(&self, e: &Expr, node: NodeId, pos: usize, size: usize) -> Result<XValue, XPathError> {
        Ok(match e {
            Expr::Literal(s) => XValue::Str(s.clone()),
            Expr::Number(n) => XValue::Num(*n),
            Expr::Var(v) => return Err(XPathError::UndefinedVariable(v.clone())),
            Expr::Neg(x) => XValue::Num(-self.number(&self.eval(x, node, pos, size)?)),
            Expr::Path { absolute, steps } => {
                let start = if *absolute {
                    self.dom.root_of(node)
                } else {
                    node
                };
                XValue::Nodes(self.steps(vec![start], steps)?)
            }
            Expr::Filter {
                primary,
                preds,
                steps,
            } => {
                let v = self.eval(primary, node, pos, size)?;
                if preds.is_empty() && steps.is_empty() {
                    return Ok(v);
                }
                let XValue::Nodes(mut nodes) = v else {
                    return Err(XPathError::InvalidType);
                };
                for p in preds {
                    nodes = self.filter(&nodes, p)?;
                }
                XValue::Nodes(self.steps(nodes, steps)?)
            }
            Expr::Func(name, args) => self.func(name, args, node, pos, size)?,
            Expr::Bin(op, l, r) => match op {
                BinOp::Or => {
                    let a = self.boolean(&self.eval(l, node, pos, size)?);
                    XValue::Bool(a || self.boolean(&self.eval(r, node, pos, size)?))
                }
                BinOp::And => {
                    let a = self.boolean(&self.eval(l, node, pos, size)?);
                    XValue::Bool(a && self.boolean(&self.eval(r, node, pos, size)?))
                }
                BinOp::Union => {
                    let a = self.eval(l, node, pos, size)?;
                    let b = self.eval(r, node, pos, size)?;
                    match (a, b) {
                        (XValue::Nodes(mut a), XValue::Nodes(b)) => {
                            a.extend(b);
                            XValue::Nodes(self.doc_order(a))
                        }
                        _ => return Err(XPathError::InvalidType),
                    }
                }
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    let a = self.eval(l, node, pos, size)?;
                    let b = self.eval(r, node, pos, size)?;
                    XValue::Bool(self.compare(*op, &a, &b))
                }
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                    let a = self.number(&self.eval(l, node, pos, size)?);
                    let b = self.number(&self.eval(r, node, pos, size)?);
                    XValue::Num(match op {
                        BinOp::Add => a + b,
                        BinOp::Sub => a - b,
                        BinOp::Mul => a * b,
                        BinOp::Div => a / b,
                        _ => a % b,
                    })
                }
            },
        })
    }

    fn steps(&self, mut nodes: Vec<NodeId>, steps: &[Step]) -> Result<Vec<NodeId>, XPathError> {
        for s in steps {
            nodes = self.step(&nodes, s)?;
        }
        Ok(nodes)
    }

    fn resolve(&self, prefix: &str) -> Result<&str, XPathError> {
        self.ns
            .iter()
            .find(|(p, _)| p.as_deref() == Some(prefix))
            .map(|(_, u)| u.as_str())
            .or_else(|| (prefix == "xml").then_some(super::XML_NS))
            .ok_or_else(|| XPathError::UndefinedPrefix(prefix.to_string()))
    }

    fn step(&self, input: &[NodeId], s: &Step) -> Result<Vec<NodeId>, XPathError> {
        // Resolve the name test's prefix up front (libxml2 does so per step).
        let uri = match &s.test {
            NodeTest::Name(Some(p), _) | NodeTest::NsAny(p) => Some(self.resolve(p)?.to_string()),
            _ => None,
        };
        let mut out = Vec::new();
        for &n in input {
            let cand: Vec<NodeId> = self
                .axis(n, s.axis)
                .into_iter()
                .filter(|c| self.test(*c, s.axis, &s.test, uri.as_deref()))
                .collect();
            let mut cand = cand;
            for p in &s.preds {
                cand = self.filter(&cand, p)?;
            }
            out.extend(cand);
        }
        if input.len() == 1 && !s.axis.is_reverse() {
            return Ok(out);
        }
        Ok(self.doc_order(out))
    }

    /// Apply a predicate to `nodes` (given in axis order).
    fn filter(&self, nodes: &[NodeId], p: &Expr) -> Result<Vec<NodeId>, XPathError> {
        let size = nodes.len();
        let mut out = Vec::new();
        for (i, &n) in nodes.iter().enumerate() {
            let v = self.eval(p, n, i + 1, size)?;
            let keep = match v {
                XValue::Num(x) => x == (i + 1) as f64,
                other => self.boolean(&other),
            };
            if keep {
                out.push(n);
            }
        }
        Ok(out)
    }

    fn test(&self, n: NodeId, axis: Axis, t: &NodeTest, uri: Option<&str>) -> bool {
        let principal_attr = axis == Axis::Attribute;
        let kind = &self.dom.node(n).kind;
        match t {
            NodeTest::Node => true,
            NodeTest::Text => matches!(kind, Kind::Text(_)),
            NodeTest::Comment => matches!(kind, Kind::Comment(_)),
            NodeTest::Pi(lit) => match kind {
                Kind::Pi { target, .. } => lit.as_ref().is_none_or(|l| l == target),
                _ => false,
            },
            NodeTest::Any | NodeTest::NsAny(_) | NodeTest::Name(..) => {
                let name = match (kind, principal_attr) {
                    (Kind::Element { name, .. }, false) | (Kind::Attribute { name, .. }, true) => {
                        name
                    }
                    _ => return false,
                };
                match t {
                    NodeTest::Any => true,
                    NodeTest::NsAny(_) => name.ns.as_deref() == uri,
                    NodeTest::Name(p, l) => {
                        name.local == *l
                            && match p {
                                Some(_) => name.ns.as_deref() == uri,
                                None => name.ns.is_none(),
                            }
                    }
                    _ => false,
                }
            }
        }
    }

    /// The nodes on `axis` from `n`, in axis order.
    fn axis(&self, n: NodeId, axis: Axis) -> Vec<NodeId> {
        let d = self.dom;
        let is_attr = d.is_attribute(n);
        match axis {
            Axis::SelfAxis => vec![n],
            Axis::Child => {
                if is_attr {
                    vec![]
                } else {
                    d.children(n).to_vec()
                }
            }
            Axis::Attribute => d.attributes(n).to_vec(),
            Axis::Namespace => vec![],
            Axis::Parent => d.parent(n).into_iter().collect(),
            Axis::Ancestor | Axis::AncestorOrSelf => {
                let mut v = Vec::new();
                if axis == Axis::AncestorOrSelf {
                    v.push(n);
                }
                let mut cur = d.parent(n);
                while let Some(p) = cur {
                    v.push(p);
                    cur = d.parent(p);
                }
                v
            }
            Axis::Descendant | Axis::DescendantOrSelf => {
                let mut v = Vec::new();
                if axis == Axis::DescendantOrSelf {
                    v.push(n);
                }
                if !is_attr {
                    self.descendants(n, &mut v);
                }
                v
            }
            Axis::FollowingSibling | Axis::PrecedingSibling => {
                if is_attr {
                    return vec![];
                }
                let Some(p) = d.parent(n) else {
                    return vec![];
                };
                let sib = d.children(p);
                let i = sib.iter().position(|c| *c == n).unwrap_or(0);
                if axis == Axis::FollowingSibling {
                    sib[i + 1..].to_vec()
                } else {
                    sib[..i].iter().rev().copied().collect()
                }
            }
            Axis::Following | Axis::Preceding => {
                let root = d.root_of(n);
                let mut all = vec![root];
                self.descendants(root, &mut all);
                // an attribute's position is its element's
                let anchor = if is_attr { d.parent(n).unwrap_or(n) } else { n };
                let i = all.iter().position(|c| *c == anchor).unwrap_or(0);
                if axis == Axis::Following {
                    let mut skip = Vec::new();
                    if !is_attr {
                        self.descendants(anchor, &mut skip);
                    }
                    all[i + 1..]
                        .iter()
                        .copied()
                        .filter(|c| !skip.contains(c))
                        .collect()
                } else {
                    let mut anc = Vec::new();
                    let mut cur = d.parent(anchor);
                    while let Some(p) = cur {
                        anc.push(p);
                        cur = d.parent(p);
                    }
                    all[..i]
                        .iter()
                        .rev()
                        .copied()
                        .filter(|c| !anc.contains(c))
                        .collect()
                }
            }
        }
    }

    fn descendants(&self, n: NodeId, out: &mut Vec<NodeId>) {
        for &c in self.dom.children(n) {
            out.push(c);
            self.descendants(c, out);
        }
    }

    /// Sort into document order and remove duplicates.
    fn doc_order(&self, mut v: Vec<NodeId>) -> Vec<NodeId> {
        let mut keys: HashMap<NodeId, Vec<u32>> = HashMap::new();
        for n in &v {
            keys.entry(*n).or_insert_with(|| self.dom.order_key(*n));
        }
        v.sort_by(|a, b| keys[a].cmp(&keys[b]));
        v.dedup();
        v
    }

    fn string_value(&self, n: NodeId) -> String {
        self.dom.text_content(n)
    }

    fn string(&self, v: &XValue) -> String {
        match v {
            XValue::Str(s) => s.clone(),
            XValue::Num(n) => number_to_string(*n),
            XValue::Bool(b) => if *b { "true" } else { "false" }.into(),
            XValue::Nodes(ns) => ns
                .first()
                .map(|n| self.string_value(*n))
                .unwrap_or_default(),
        }
    }

    fn number(&self, v: &XValue) -> f64 {
        match v {
            XValue::Num(n) => *n,
            XValue::Bool(b) => f64::from(u8::from(*b)),
            _ => string_to_number(&self.string(v)),
        }
    }

    fn boolean(&self, v: &XValue) -> bool {
        match v {
            XValue::Bool(b) => *b,
            XValue::Num(n) => *n != 0.0 && !n.is_nan(),
            XValue::Str(s) => !s.is_empty(),
            XValue::Nodes(ns) => !ns.is_empty(),
        }
    }

    fn compare(&self, op: BinOp, a: &XValue, b: &XValue) -> bool {
        use XValue::*;
        match (a, b) {
            (Nodes(x), Nodes(y)) => x.iter().any(|m| {
                let sm = self.string_value(*m);
                y.iter()
                    .any(|n| self.cmp_atomic(op, &Str(sm.clone()), &Str(self.string_value(*n))))
            }),
            (Nodes(x), Bool(_)) => self.cmp_atomic(op, &Bool(!x.is_empty()), b),
            (Bool(_), Nodes(y)) => self.cmp_atomic(op, a, &Bool(!y.is_empty())),
            (Nodes(x), other) => x
                .iter()
                .any(|m| self.cmp_atomic(op, &self.atom_like(*m, other), other)),
            (other, Nodes(y)) => y
                .iter()
                .any(|n| self.cmp_atomic(op, other, &self.atom_like(*n, other))),
            _ => self.cmp_atomic(op, a, b),
        }
    }

    /// A node's value converted to the type of `like` (number or string).
    fn atom_like(&self, n: NodeId, like: &XValue) -> XValue {
        let s = self.string_value(n);
        match like {
            XValue::Num(_) => XValue::Num(string_to_number(&s)),
            _ => XValue::Str(s),
        }
    }

    fn cmp_atomic(&self, op: BinOp, a: &XValue, b: &XValue) -> bool {
        use XValue::*;
        match op {
            BinOp::Eq | BinOp::Ne => {
                let eq = if matches!(a, Bool(_)) || matches!(b, Bool(_)) {
                    self.boolean(a) == self.boolean(b)
                } else if matches!(a, Num(_)) || matches!(b, Num(_)) {
                    self.number(a) == self.number(b)
                } else {
                    self.string(a) == self.string(b)
                };
                (op == BinOp::Eq) == eq
            }
            _ => {
                let (x, y) = (self.number(a), self.number(b));
                match op {
                    BinOp::Lt => x < y,
                    BinOp::Le => x <= y,
                    BinOp::Gt => x > y,
                    _ => x >= y,
                }
            }
        }
    }

    fn func(
        &self,
        name: &str,
        args: &[Expr],
        node: NodeId,
        pos: usize,
        size: usize,
    ) -> Result<XValue, XPathError> {
        let arity = |ok: bool| {
            if ok {
                Ok(())
            } else {
                Err(XPathError::Arity(name.to_string()))
            }
        };
        let a = |i: usize| self.eval(&args[i], node, pos, size);
        let s_arg = |i: usize| -> Result<String, XPathError> {
            if args.len() > i {
                Ok(self.string(&a(i)?))
            } else {
                Ok(self.string_value(node))
            }
        };
        let n = args.len();
        Ok(match name {
            "last" => {
                arity(n == 0)?;
                XValue::Num(size as f64)
            }
            "position" => {
                arity(n == 0)?;
                XValue::Num(pos as f64)
            }
            "count" => {
                arity(n == 1)?;
                match a(0)? {
                    XValue::Nodes(v) => XValue::Num(v.len() as f64),
                    _ => return Err(XPathError::InvalidType),
                }
            }
            "id" => {
                arity(n == 1)?;
                XValue::Nodes(vec![])
            }
            "local-name" | "namespace-uri" | "name" => {
                arity(n <= 1)?;
                let target = if n == 1 {
                    match a(0)? {
                        XValue::Nodes(v) => v.first().copied(),
                        _ => return Err(XPathError::InvalidType),
                    }
                } else {
                    Some(node)
                };
                let q = target.and_then(|t| self.dom.name(t));
                XValue::Str(match (name, q) {
                    (_, None) => String::new(),
                    ("local-name", Some(q)) => q.local.clone(),
                    ("namespace-uri", Some(q)) => q.ns.clone().unwrap_or_default(),
                    (_, Some(q)) => q.qualified(),
                })
            }
            "string" => {
                arity(n <= 1)?;
                XValue::Str(s_arg(0)?)
            }
            "concat" => {
                arity(n >= 2)?;
                let mut s = String::new();
                for i in 0..n {
                    s.push_str(&self.string(&a(i)?));
                }
                XValue::Str(s)
            }
            "starts-with" | "contains" | "substring-before" | "substring-after" => {
                arity(n == 2)?;
                let x = self.string(&a(0)?);
                let y = self.string(&a(1)?);
                match name {
                    "starts-with" => XValue::Bool(x.starts_with(&y)),
                    "contains" => XValue::Bool(x.contains(&y)),
                    "substring-before" => {
                        XValue::Str(x.find(&y).map(|i| x[..i].to_string()).unwrap_or_default())
                    }
                    _ => XValue::Str(
                        x.find(&y)
                            .map(|i| x[i + y.len()..].to_string())
                            .unwrap_or_default(),
                    ),
                }
            }
            "substring" => {
                arity(n == 2 || n == 3)?;
                let s: Vec<char> = self.string(&a(0)?).chars().collect();
                let start = round_half_up(self.number(&a(1)?));
                let len = if n == 3 {
                    round_half_up(self.number(&a(2)?))
                } else {
                    f64::INFINITY
                };
                let end = start + len;
                let out: String = s
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        let p = (*i + 1) as f64;
                        p >= start && p < end
                    })
                    .map(|(_, c)| *c)
                    .collect();
                XValue::Str(out)
            }
            "string-length" => {
                arity(n <= 1)?;
                XValue::Num(s_arg(0)?.chars().count() as f64)
            }
            "normalize-space" => {
                arity(n <= 1)?;
                let s = s_arg(0)?;
                XValue::Str(
                    s.split(is_xml_space)
                        .filter(|w| !w.is_empty())
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            }
            "translate" => {
                arity(n == 3)?;
                let s = self.string(&a(0)?);
                let from: Vec<char> = self.string(&a(1)?).chars().collect();
                let to: Vec<char> = self.string(&a(2)?).chars().collect();
                XValue::Str(
                    s.chars()
                        .filter_map(|c| match from.iter().position(|f| *f == c) {
                            Some(i) => to.get(i).copied(),
                            None => Some(c),
                        })
                        .collect(),
                )
            }
            "boolean" => {
                arity(n == 1)?;
                XValue::Bool(self.boolean(&a(0)?))
            }
            "not" => {
                arity(n == 1)?;
                XValue::Bool(!self.boolean(&a(0)?))
            }
            "true" | "false" => {
                arity(n == 0)?;
                XValue::Bool(name == "true")
            }
            "lang" => {
                arity(n == 1)?;
                XValue::Bool(false)
            }
            "number" => {
                arity(n <= 1)?;
                if n == 1 {
                    XValue::Num(self.number(&a(0)?))
                } else {
                    XValue::Num(string_to_number(&self.string_value(node)))
                }
            }
            "sum" => {
                arity(n == 1)?;
                match a(0)? {
                    XValue::Nodes(v) => XValue::Num(
                        v.iter()
                            .map(|x| string_to_number(&self.string_value(*x)))
                            .sum(),
                    ),
                    _ => return Err(XPathError::InvalidType),
                }
            }
            "floor" | "ceiling" | "round" => {
                arity(n == 1)?;
                let x = self.number(&a(0)?);
                XValue::Num(match name {
                    "floor" => x.floor(),
                    "ceiling" => x.ceil(),
                    _ => round_half_up(x),
                })
            }
            _ => return Err(XPathError::UnknownFunction(name.to_string())),
        })
    }
}

fn round_half_up(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() {
        x
    } else {
        (x + 0.5).floor()
    }
}

/// XPath `number()` of a string.
pub fn string_to_number(s: &str) -> f64 {
    let t = s.trim_matches(is_xml_space);
    let body = t.strip_prefix('-').unwrap_or(t);
    let ok = !body.is_empty()
        && body.chars().all(|c| c.is_ascii_digit() || c == '.')
        && body.matches('.').count() <= 1
        && body != ".";
    if !ok {
        return f64::NAN;
    }
    t.parse().unwrap_or(f64::NAN)
}

/// XPath `string()` of a number.
pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if n == n.trunc() && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{n}");
    if s.contains('e') {
        // plain decimal notation
        let p = format!("{n:.20}");
        let p = p.trim_end_matches('0').trim_end_matches('.');
        return p.to_string();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "http://biblatex-biber.sourceforge.net/biblatexml";

    fn doc() -> (Dom, NodeId) {
        let d = Dom::parse(&format!(
            r#"<bltx:entries xmlns:bltx="{NS}"><bltx:entry id="e1" entrytype="book"><bltx:names type="author"><bltx:name useprefix="true"><bltx:namepart type="family">A</bltx:namepart></bltx:name><bltx:name><bltx:namepart type="family">B</bltx:namepart><bltx:namepart type="given">C</bltx:namepart></bltx:name></bltx:names><bltx:title>T</bltx:title><bltx:date>2000</bltx:date><bltx:date type="orig">1999</bltx:date></bltx:entry><bltx:entry id="e2" entrytype="misc"/></bltx:entries>"#
        ))
        .unwrap();
        let root = d.element_children(d.document()).next().unwrap();
        let e = d.element_children(root).next().unwrap();
        (d, e)
    }

    fn find(d: &Dom, n: NodeId, x: &str) -> Result<Vec<NodeId>, XPathError> {
        let ns = d.in_scope_ns(n);
        match XPath::compile(x)?.eval(d, n, &ns)? {
            XValue::Nodes(v) => Ok(v),
            _ => Ok(vec![]),
        }
    }

    fn val(d: &Dom, n: NodeId, x: &str) -> String {
        find(d, n, x)
            .unwrap()
            .iter()
            .map(|n| d.text_content(*n))
            .collect()
    }

    #[test]
    fn paths() {
        let (d, e) = doc();
        assert_eq!(val(&d, e, "./bltx:title"), "T");
        assert_eq!(
            val(
                &d,
                e,
                "./bltx:names[@type='author']/bltx:name[2]/bltx:namepart[@type='family']"
            ),
            "B"
        );
        assert_eq!(
            val(
                &d,
                e,
                "./bltx:names[@type='author']/bltx:name[1]/@useprefix"
            ),
            "true"
        );
        assert_eq!(find(&d, e, "*").unwrap().len(), 4);
        assert_eq!(
            val(&d, e, "/bltx:entries/bltx:entry[@id='e2']/@entrytype"),
            "misc"
        );
        assert_eq!(val(&d, e, "./bltx:date[not(@type)]"), "2000");
        assert_eq!(val(&d, e, "./bltx:date[last()]"), "1999");
        assert_eq!(val(&d, e, ".//bltx:namepart[. = 'C']"), "C");
        assert_eq!(val(&d, e, "./bltx:title/text()"), "T");
        assert_eq!(val(&d, e, "./@id | ./bltx:title"), "e1T");
        assert_eq!(val(&d, e, "(./bltx:names//bltx:namepart)[last()]"), "C");
        assert_eq!(val(&d, e, "./bltx:title/.."), d.text_content(e));
        assert_eq!(
            val(&d, e, "./bltx:title/preceding-sibling::*[1]/@type"),
            "author"
        );
        assert_eq!(find(&d, e, "./title").unwrap().len(), 0);
        assert!(matches!(
            find(&d, e, "./foo:title"),
            Err(XPathError::UndefinedPrefix(_))
        ));
        assert!(matches!(
            XPath::compile("./bltx:title[@id='a'b']"),
            Err(XPathError::Syntax { .. })
        ));
        assert!(XPath::compile("/[@id='x']").is_err());
    }

    #[test]
    fn functions_and_numbers() {
        let (d, e) = doc();
        let ev = |x: &str| {
            XPath::compile(x)
                .unwrap()
                .eval(&d, e, &d.in_scope_ns(e))
                .unwrap()
        };
        assert_eq!(ev("count(./bltx:date)"), XValue::Num(2.0));
        assert_eq!(ev("concat('a', 'b', 1)"), XValue::Str("ab1".into()));
        assert_eq!(
            ev("substring('12345', 1.5, 2.6)"),
            XValue::Str("234".into())
        );
        assert_eq!(ev("normalize-space('  a  b ')"), XValue::Str("a b".into()));
        assert_eq!(ev("1 div 0 > 2 and 5 mod 2 = 1"), XValue::Bool(true));
        assert_eq!(ev("./bltx:date = 1999"), XValue::Bool(true));
        assert_eq!(number_to_string(0.5), "0.5");
        assert_eq!(number_to_string(-3.0), "-3");
    }
}

//! Inheritance records (`Biber::Config::set_inheritance`,
//! `get_inheritance`, `is_inheritance_path`, Config.pm:1119-1171).
//!
//! biber keeps a list of `{s, t}` records per inheritance type in
//! `$CONFIG->{state}{crossref|xdata}`, appending a duplicate every time an
//! inheritance is repeated (crossref cascades, XDATA re-resolution). It looks
//! records up with a linear scan and checks for loops with an unmemoised
//! depth-first search over that list: cubic for many children of a parent
//! that has its own parent (BB-110). The record list itself is observable
//! state, so it is kept exactly; lookups use a set of edges and the loop
//! check a visited-set search, with the same results.

use crate::model::SectionState;
use serde_json::{Value as J, json};
use std::collections::{HashMap, HashSet};

/// The inheritance graph of one type (`crossref` or `xdata`).
pub struct InhGraph {
    kind: &'static str,
    edges: HashSet<(String, String)>,
    adj: HashMap<String, Vec<String>>,
    /// Whether a loop has been found (then every check searches).
    cyclic: bool,
    /// Whether the last `set_inheritance` added a new edge.
    last_new: bool,
}

impl InhGraph {
    /// The records of `kind` in `st.config_state`.
    pub fn load(st: &SectionState, kind: &'static str) -> Self {
        let mut g = InhGraph {
            kind,
            edges: HashSet::new(),
            adj: HashMap::new(),
            cyclic: false,
            last_new: false,
        };
        if let Some(J::Array(a)) = st.config_state.get(kind) {
            for r in a {
                let s = r.get("s").and_then(J::as_str).unwrap_or("");
                let t = r.get("t").and_then(J::as_str).unwrap_or("");
                g.add(s, t);
            }
        }
        g
    }

    fn add(&mut self, s: &str, t: &str) -> bool {
        let new = self.edges.insert((s.to_string(), t.to_string()));
        if new {
            self.adj.entry(s.to_string()).or_default().push(t.to_string());
        }
        new
    }

    /// `set_inheritance($type, $source, $target)`.
    pub fn set_inheritance(&mut self, st: &mut SectionState, s: &str, t: &str) {
        self.last_new = self.add(s, t);
        if !st.config_state.is_object() {
            st.config_state = json!({});
        }
        let recs = st
            .config_state
            .as_object_mut()
            .expect("object")
            .entry(self.kind)
            .or_insert_with(|| J::Array(Vec::new()));
        if !recs.is_array() {
            *recs = J::Array(Vec::new());
        }
        recs.as_array_mut()
            .expect("array")
            .push(json!({"s": s, "t": t}));
    }

    /// `get_inheritance($type, $source, $target)`.
    pub fn get_inheritance(&self, s: &str, t: &str) -> bool {
        self.edges.contains(&(s.to_string(), t.to_string()))
    }

    /// `is_inheritance_path($type, $e1, $e2)`, called right after
    /// `set_inheritance($type, $e2, $e1)`: is there a path (of length one or
    /// more) from `e1` to `e2`? Re-adding an existing edge to a graph without
    /// loops cannot create one, so that case needs no search.
    pub fn is_path_after_set(&mut self, e1: &str, e2: &str) -> bool {
        if !self.last_new && !self.cyclic {
            return false;
        }
        let mut seen: HashSet<&str> = HashSet::new();
        let mut stack = vec![e1];
        let mut found = false;
        'outer: while let Some(n) = stack.pop() {
            if !seen.insert(n) {
                continue;
            }
            for t in self.adj.get(n).into_iter().flatten() {
                if t == e2 {
                    found = true;
                    break 'outer;
                }
                stack.push(t);
            }
        }
        if found {
            self.cyclic = true;
        }
        found
    }
}

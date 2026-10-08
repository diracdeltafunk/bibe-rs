//! Bookkeeping of list values shared by reference between entries.
//!
//! biber copies list fields by reference when cloning, inheriting and
//! resolving XDATA (`$self->set_datafield($f, $parent->get_field($f))`), so
//! an in-place change of one entry's list shows in every entry sharing it.
//! [`SharedLists`] tracks these groups of `(citekey, field)` while a phase
//! runs and writes them back to [`SectionState::shared_lists`] (the shape of
//! the phase dumps, which the `.bbl` writer reads).

use crate::model::{Entry, NameArena, NameList, NameListId, SectionState, Value};
use std::collections::{BTreeSet, HashMap};

type Slot = (String, String);

/// Groups of data fields whose (array) values are one Perl array.
#[derive(Debug, Default)]
pub struct SharedLists {
    of: HashMap<Slot, usize>,
    groups: Vec<BTreeSet<Slot>>,
}

impl SharedLists {
    /// The groups recorded in `st`.
    pub fn load(st: &SectionState) -> Self {
        let mut s = SharedLists::default();
        for g in st.shared_lists.iter().flatten() {
            let i = s.groups.len();
            s.groups.push(g.iter().cloned().collect());
            for m in g {
                s.of.insert(m.clone(), i);
            }
        }
        s
    }

    /// Write the groups back to `st` (groups of two or more, each sorted,
    /// sorted by first member, as `BiberPhaseDump.pm` lists them).
    pub fn store(self, st: &mut SectionState) {
        let mut out: Vec<Vec<Slot>> = self
            .groups
            .into_iter()
            .filter(|g| g.len() > 1)
            .map(|g| g.into_iter().collect())
            .collect();
        out.sort();
        st.shared_lists = Some(out);
    }

    /// `(key, field)` got a value of its own (or was deleted).
    pub fn unshare(&mut self, key: &str, field: &str) {
        let slot = (key.to_string(), field.to_string());
        if let Some(i) = self.of.remove(&slot) {
            self.groups[i].remove(&slot);
        }
    }

    /// Drop every field of `key` (the entry was replaced).
    pub fn unshare_entry(&mut self, key: &str, e: &Entry) {
        for f in e.datafields.keys() {
            self.unshare(key, f);
        }
    }

    /// `(dkey, dfield)` now holds the same array as `(skey, sfield)`.
    pub fn share(&mut self, dkey: &str, dfield: &str, skey: &str, sfield: &str) {
        if dkey == skey && dfield == sfield {
            return;
        }
        self.unshare(dkey, dfield);
        let src = (skey.to_string(), sfield.to_string());
        let i = match self.of.get(&src) {
            Some(&i) => i,
            None => {
                let i = self.groups.len();
                self.groups.push(BTreeSet::from([src.clone()]));
                self.of.insert(src, i);
                i
            }
        };
        let dst = (dkey.to_string(), dfield.to_string());
        self.groups[i].insert(dst.clone());
        self.of.insert(dst, i);
    }

    /// Every `(key, field)` sharing the array of `(key, field)`, itself
    /// included.
    pub fn group(&self, key: &str, field: &str) -> Vec<Slot> {
        let slot = (key.to_string(), field.to_string());
        match self.of.get(&slot) {
            Some(&i) => self.groups[i].iter().cloned().collect(),
            None => vec![slot],
        }
    }
}

/// Is this value a Perl array reference (what `shared_lists` tracks)?
pub fn is_array(v: &Value) -> bool {
    matches!(
        v,
        Value::List(_) | Value::Ranges(_) | Value::Other(serde_json::Value::Array(_))
    )
}

/// A deep copy of a name list (new list and name ids), for the
/// `shared-namelists` quirk fixed (BB-026).
pub fn clone_names(arena: &mut NameArena, id: NameListId) -> NameListId {
    let src = arena.list(id).clone();
    let names = src
        .names
        .iter()
        .map(|n| {
            let name = arena.name(*n).clone();
            arena.new_name(name)
        })
        .collect();
    let nid = arena.new_list();
    *arena.list_mut(nid) = NameList {
        id: nid,
        names,
        ..src
    };
    nid
}

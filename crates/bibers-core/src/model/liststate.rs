//! [`ListState`]: a data list's `state` hash (`$dlist->{state}`).
//!
//! biber keeps per-list processing state in one big Perl hash. Two of its
//! sub-hashes have an entry per citekey or per name list (`fields` and
//! `namelistdata`) and are read and written for every entry in every
//! per-list phase, so they are kept in hash maps by key here; everything
//! else stays generic. Values are compact [`PV`](super::PV)s.
//! [`ListState::from_json`] and [`ListState::to_json`] convert to and from
//! the dumped form (and are exact inverses).

use super::PerlData;
use super::pv::{PMap, PV as J};
use rustc_hash::FxHashMap;

/// The sub-hashes kept by key.
const FIELDS: &str = "fields";
const NAMELISTDATA: &str = "namelistdata";

/// `$dlist->{state}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListState {
    /// `{fields}`: citekey → the entry's per-list fields (`None`: absent).
    fields: Option<FxHashMap<String, J>>,
    /// `{namelistdata}`: name list id → its data (`None`: absent).
    namelistdata: Option<FxHashMap<String, J>>,
    /// The other keys.
    rest: PMap,
    /// The state as found when it was not a hash (kept as is).
    raw: Option<PerlData>,
}

/// A first-level sub-hash of the state, `$state->{k0}`.
pub enum Level1<'a> {
    Keyed(&'a mut FxHashMap<String, J>),
    Plain(&'a mut PMap),
}

impl<'a> Level1<'a> {
    pub fn get(&self, k: &str) -> Option<&J> {
        match self {
            Level1::Keyed(m) => m.get(k),
            Level1::Plain(m) => m.get(k),
        }
    }

    /// [`Level1::get`] for the lifetime of the borrow of the state.
    pub fn into_get(self, k: &str) -> Option<&'a J> {
        match self {
            Level1::Keyed(m) => m.get(k),
            Level1::Plain(m) => m.get(k),
        }
    }

    /// `$h->{k}` if present, mutably.
    pub fn into_get_mut(self, k: &str) -> Option<&'a mut J> {
        match self {
            Level1::Keyed(m) => m.get_mut(k),
            Level1::Plain(m) => m.get_mut(k),
        }
    }

    /// `$h->{k}` as an lvalue (created as `undef` if absent).
    pub fn slot(self, k: &str) -> &'a mut J {
        match self {
            Level1::Keyed(m) => {
                if !m.contains_key(k) {
                    m.insert(k.to_string(), J::Null);
                }
                m.get_mut(k).expect("just inserted")
            }
            Level1::Plain(m) => {
                if !m.contains_key(k) {
                    m.insert(k.to_string(), J::Null);
                }
                m.get_mut(k).expect("just inserted")
            }
        }
    }

    /// `$h->{k}` as a hash, autovivified (a non-hash value is replaced).
    pub fn hash(self, k: &str) -> &'a mut J {
        let v = self.slot(k);
        if !v.is_object() {
            *v = J::Object(PMap::new());
        }
        v
    }

    pub fn insert(self, k: &str, v: J) {
        match self {
            Level1::Keyed(m) => match m.get_mut(k) {
                Some(slot) => *slot = v,
                None => {
                    m.insert(k.to_string(), v);
                }
            },
            Level1::Plain(m) => match m.get_mut(k) {
                Some(slot) => *slot = v,
                None => {
                    m.insert(k.to_string(), v);
                }
            },
        }
    }

    pub fn contains_key(&self, k: &str) -> bool {
        match self {
            Level1::Keyed(m) => m.contains_key(k),
            Level1::Plain(m) => m.contains_key(k),
        }
    }
}

impl ListState {
    /// The state of a dump.
    pub fn from_json(v: PerlData) -> ListState {
        let serde_json::Value::Object(_) = v else {
            return ListState {
                raw: Some(v),
                ..Default::default()
            };
        };
        let J::Object(mut m) = J::from_json(v) else {
            unreachable!()
        };
        let mut split = |k: &str| match m.remove(k) {
            Some(J::Object(o)) => Some(o.into_iter().collect()),
            Some(other) => {
                // Not a hash: kept generic (and replaced on first access).
                m.insert(k.to_string(), other);
                None
            }
            None => None,
        };
        let fields = split(FIELDS);
        let namelistdata = split(NAMELISTDATA);
        ListState {
            fields,
            namelistdata,
            rest: m,
            raw: None,
        }
    }

    /// The state as dumped.
    pub fn to_json(&self) -> PerlData {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        let mut m = self.rest.to_json();
        for (k, v) in [(FIELDS, &self.fields), (NAMELISTDATA, &self.namelistdata)] {
            if let Some(h) = v {
                m.insert(
                    k.to_string(),
                    PerlData::Object(h.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()),
                );
            }
        }
        PerlData::Object(m)
    }

    /// `$state->{k0}` as a hash, autovivified (as is the state itself).
    pub fn level1(&mut self, k0: &str) -> Level1<'_> {
        if self.raw.take().is_some() {
            *self = ListState::default();
        }
        let keyed = match k0 {
            FIELDS => Some(&mut self.fields),
            NAMELISTDATA => Some(&mut self.namelistdata),
            _ => None,
        };
        if let Some(h) = keyed {
            // A non-hash value found in a dump is replaced, as Perl's
            // autovivification would die/replace; keep the generic path.
            if self.rest.remove(k0).is_some() {
                *h = None;
            }
            return Level1::Keyed(h.get_or_insert_with(FxHashMap::default));
        }
        if !matches!(self.rest.get(k0), Some(J::Object(_))) {
            self.rest.insert(k0.to_string(), J::Object(PMap::new()));
        }
        match self.rest.get_mut(k0) {
            Some(J::Object(m)) => Level1::Plain(m),
            _ => unreachable!(),
        }
    }

    /// rvalue `$state->{k0}{k1}` without vivifying anything.
    pub fn peek2(&self, k0: &str, k1: &str) -> Option<&J> {
        if self.raw.is_some() {
            return None;
        }
        let keyed = match k0 {
            FIELDS => Some(&self.fields),
            NAMELISTDATA => Some(&self.namelistdata),
            _ => None,
        };
        match keyed {
            Some(h) if !self.rest.contains_key(k0) => h.as_ref()?.get(k1),
            _ => self.rest.get(k0)?.get(k1),
        }
    }

    /// Take `$state->{namelistdata}` out (if it is a hash).
    pub fn take_namelistdata(&mut self) -> Option<FxHashMap<String, J>> {
        if self.rest.contains_key(NAMELISTDATA) {
            return None;
        }
        self.namelistdata.take()
    }

    /// `$state->{fields}` by citekey, if it is a hash.
    pub fn fields_mut(&mut self) -> Option<&mut FxHashMap<String, J>> {
        if self.rest.contains_key(FIELDS) {
            return None;
        }
        self.fields.as_mut()
    }

    /// `$state->{namelistdata}` by name list id, if it is a hash.
    pub fn namelistdata_mut(&mut self) -> Option<&mut FxHashMap<String, J>> {
        if self.rest.contains_key(NAMELISTDATA) {
            return None;
        }
        self.namelistdata.as_mut()
    }

    /// Set `$state->{namelistdata}` (`None`: absent).
    pub fn set_namelistdata(&mut self, h: Option<FxHashMap<String, J>>) {
        if self.raw.take().is_some() {
            *self = ListState::default();
        }
        if h.is_some() {
            self.rest.remove(NAMELISTDATA);
        }
        self.namelistdata = h;
    }

    /// `$state->{k} = v` for a key other than `fields`/`namelistdata`.
    pub fn insert(&mut self, k: &str, v: J) {
        debug_assert!(k != FIELDS && k != NAMELISTDATA);
        if self.raw.take().is_some() {
            *self = ListState::default();
        }
        self.rest.insert(k.to_string(), v);
    }

    /// Remove `$state->{k}` for a key other than `fields`/`namelistdata`.
    pub fn remove(&mut self, k: &str) -> Option<J> {
        debug_assert!(k != FIELDS && k != NAMELISTDATA);
        self.rest.remove(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_round_trip() {
        for v in [
            json!({}),
            json!(null),
            json!("x"),
            json!({"fields": {"a": {"x": "1"}, "b": {}}, "namelistdata": {"#nl1": {"ul": "2", "#n3": {"un": ["base", ["family"]]}}}, "seen_labelname": {"h": "2"}}),
            json!({"fields": "odd", "namelistdata": {}}),
        ] {
            assert_eq!(ListState::from_json(v.clone()).to_json(), v);
        }
    }

    #[test]
    fn vivification() {
        let mut s = ListState::default();
        s.level1("fields").hash("k");
        s.level1("seen").insert("x", J::from("1"));
        assert_eq!(
            s.to_json(),
            json!({"fields": {"k": {}}, "seen": {"x": "1"}})
        );
        assert_eq!(s.peek2("seen", "x"), Some(&J::from("1")));
        assert_eq!(s.peek2("namelistdata", "x"), None);
        let mut odd = ListState::from_json(json!({"fields": "odd"}));
        odd.level1("fields");
        assert_eq!(odd.to_json(), json!({"fields": {}}));
    }
}

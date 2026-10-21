//! [`PV`]: a compact Perl data value for state that is kept in biber's
//! generic shape (a data list's `state`, [`super::ListState`]).
//!
//! It is `serde_json::Value` with a smaller object type. A
//! `serde_json::Map` is a `BTreeMap<String, Value>`, which costs a 600-byte
//! node per map however few keys it has, plus one allocation per key; the
//! list state has a few small hashes per entry and per name (130,000 of them
//! for 17,000 entries), so that was most of its memory. A [`PMap`] is a
//! sorted `Vec` of its pairs while it is small and a `BTreeMap` once it
//! grows large; both iterate in key byte order, like `serde_json::Map`
//! (biber's `sort keys`). Strings and small-map keys of up to 24 bytes (most
//! of them: name parts, `un` levels, counts, name ids) are stored inline.
//!
//! The API mirrors the parts of `serde_json`'s that the pipeline uses, so
//! code ported against `serde_json::Value` reads the same.

use compact_str::CompactString;
use serde_json::{Map, Number, Value as J};
use std::collections::BTreeMap;
use std::fmt;

/// A Perl value in the dumped shape (scalars are strings; `undef` is
/// `Null`). See the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PV {
    #[default]
    Null,
    Bool(bool),
    Number(Number),
    /// A string; up to 24 bytes are stored inline.
    String(CompactString),
    Array(Vec<PV>),
    Object(PMap),
}

impl PV {
    pub fn is_null(&self) -> bool {
        matches!(self, PV::Null)
    }

    pub fn is_object(&self) -> bool {
        matches!(self, PV::Object(_))
    }

    pub fn is_array(&self) -> bool {
        matches!(self, PV::Array(_))
    }

    pub fn is_string(&self) -> bool {
        matches!(self, PV::String(_))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            PV::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<PV>> {
        match self {
            PV::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<PV>> {
        match self {
            PV::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&PMap> {
        match self {
            PV::Object(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut PMap> {
        match self {
            PV::Object(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            PV::Number(n) => n.as_f64(),
            _ => None,
        }
    }

    /// `$v->{k}` of a hash or `$v->[i]` of an array (`serde_json::Value::get`).
    pub fn get<I: PIndex>(&self, i: I) -> Option<&PV> {
        i.index_into(self)
    }

    /// Take the value, leaving `Null`.
    pub fn take(&mut self) -> PV {
        std::mem::take(self)
    }

    /// From the dumped shape.
    pub fn from_json(v: J) -> PV {
        match v {
            J::Null => PV::Null,
            J::Bool(b) => PV::Bool(b),
            J::Number(n) => PV::Number(n),
            J::String(s) => PV::String(s.into()),
            J::Array(a) => PV::Array(a.into_iter().map(PV::from_json).collect()),
            J::Object(m) => PV::Object(m.into_iter().map(|(k, v)| (k, PV::from_json(v))).collect()),
        }
    }

    /// The dumped shape.
    pub fn to_json(&self) -> J {
        match self {
            PV::Null => J::Null,
            PV::Bool(b) => J::Bool(*b),
            PV::Number(n) => J::Number(n.clone()),
            PV::String(s) => J::String(s.to_string()),
            PV::Array(a) => J::Array(a.iter().map(PV::to_json).collect()),
            PV::Object(m) => J::Object(m.to_json()),
        }
    }
}

/// `serde_json`'s `Display`: the value as JSON text.
impl fmt::Display for PV {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_json(), f)
    }
}

impl From<String> for PV {
    fn from(s: String) -> PV {
        PV::String(s.into())
    }
}

impl From<&String> for PV {
    fn from(s: &String) -> PV {
        PV::String(s.as_str().into())
    }
}

impl From<&str> for PV {
    fn from(s: &str) -> PV {
        PV::String(s.into())
    }
}

impl From<Vec<PV>> for PV {
    fn from(a: Vec<PV>) -> PV {
        PV::Array(a)
    }
}

impl From<PMap> for PV {
    fn from(m: PMap) -> PV {
        PV::Object(m)
    }
}

static NULL: PV = PV::Null;

/// An index into a [`PV`]: a hash key or an array position.
pub trait PIndex {
    fn index_into(self, v: &PV) -> Option<&PV>;
}

impl PIndex for &str {
    fn index_into(self, v: &PV) -> Option<&PV> {
        v.as_object()?.get(self)
    }
}

impl PIndex for &String {
    fn index_into(self, v: &PV) -> Option<&PV> {
        v.as_object()?.get(self)
    }
}

impl PIndex for usize {
    fn index_into(self, v: &PV) -> Option<&PV> {
        v.as_array()?.get(self)
    }
}

/// `$v->{k}` (`undef` if absent), like `serde_json`'s indexing.
impl std::ops::Index<&str> for PV {
    type Output = PV;
    fn index(&self, k: &str) -> &PV {
        self.get(k).unwrap_or(&NULL)
    }
}

/// `$v->[i]` (`undef` if absent), like `serde_json`'s indexing.
impl std::ops::Index<usize> for PV {
    type Output = PV;
    fn index(&self, i: usize) -> &PV {
        self.get(i).unwrap_or(&NULL)
    }
}

/// A key of a small [`PMap`].
type Key = CompactString;

/// A hash of [`PV`]s, iterated in key order. See the [module docs](self).
#[derive(Clone, Default)]
pub struct PMap(Repr);

#[derive(Clone)]
enum Repr {
    /// Sorted by key.
    Small(Vec<(Key, PV)>),
    Large(Box<BTreeMap<Box<str>, PV>>),
}

impl Default for Repr {
    fn default() -> Self {
        Repr::Small(Vec::new())
    }
}

/// Above this size a map is a `BTreeMap` (inserting into a sorted `Vec` is
/// linear).
const SMALL_MAX: usize = 24;

impl PMap {
    pub fn new() -> PMap {
        PMap::default()
    }

    pub fn len(&self) -> usize {
        match &self.0 {
            Repr::Small(v) => v.len(),
            Repr::Large(m) => m.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, k: &str) -> Option<&PV> {
        match &self.0 {
            Repr::Small(v) => search(v, k).ok().map(|i| &v[i].1),
            Repr::Large(m) => m.get(k),
        }
    }

    pub fn get_mut(&mut self, k: &str) -> Option<&mut PV> {
        match &mut self.0 {
            Repr::Small(v) => match search(v, k) {
                Ok(i) => Some(&mut v[i].1),
                Err(_) => None,
            },
            Repr::Large(m) => m.get_mut(k),
        }
    }

    pub fn contains_key(&self, k: &str) -> bool {
        self.get(k).is_some()
    }

    /// Insert `v` at `k`; the old value if `k` was present.
    pub fn insert(&mut self, k: String, v: PV) -> Option<PV> {
        match &mut self.0 {
            Repr::Small(items) => match search(items, &k) {
                Ok(i) => Some(std::mem::replace(&mut items[i].1, v)),
                Err(i) => {
                    if items.len() < SMALL_MAX {
                        let len = items.len();
                        if len == items.capacity() {
                            // Grow by half (these maps are many and small).
                            items.reserve_exact((len / 2).max(2));
                        }
                        items.insert(i, (Key::from(k), v));
                    } else {
                        let mut m: BTreeMap<Box<str>, PV> = std::mem::take(items)
                            .into_iter()
                            .map(|(k, v)| (Box::from(k.as_str()), v))
                            .collect();
                        m.insert(k.into_boxed_str(), v);
                        self.0 = Repr::Large(Box::new(m));
                    }
                    None
                }
            },
            Repr::Large(m) => m.insert(k.into_boxed_str(), v),
        }
    }

    pub fn remove(&mut self, k: &str) -> Option<PV> {
        match &mut self.0 {
            Repr::Small(v) => search(v, k).ok().map(|i| v.remove(i).1),
            Repr::Large(m) => m.remove(k),
        }
    }

    /// The entry of `k`, for in-place manipulation (`serde_json::Map::entry`).
    pub fn entry(&mut self, k: impl Into<String>) -> PEntry<'_> {
        PEntry {
            map: self,
            key: k.into(),
        }
    }

    /// The pairs in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &PV)> {
        let (small, large) = match &self.0 {
            Repr::Small(v) => (Some(v.iter().map(|(k, v)| (k.as_str(), v))), None),
            Repr::Large(m) => (None, Some(m.iter().map(|(k, v)| (&**k, v)))),
        };
        small
            .into_iter()
            .flatten()
            .chain(large.into_iter().flatten())
    }

    /// The pairs in key order, values mutable.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut PV)> {
        let (small, large) = match &mut self.0 {
            Repr::Small(v) => (
                Some(v.iter_mut().map(|(k, v)| (k.as_str(), v))),
                None,
            ),
            Repr::Large(m) => (None, Some(m.iter_mut().map(|(k, v)| (&**k, v)))),
        };
        small
            .into_iter()
            .flatten()
            .chain(large.into_iter().flatten())
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &PV> {
        self.iter().map(|(_, v)| v)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut PV> {
        self.iter_mut().map(|(_, v)| v)
    }

    /// Keep the pairs for which `f` is true.
    pub fn retain(&mut self, mut f: impl FnMut(&str, &mut PV) -> bool) {
        match &mut self.0 {
            Repr::Small(v) => v.retain_mut(|(k, v)| f(k.as_str(), v)),
            Repr::Large(m) => m.retain(|k, v| f(k, v)),
        }
    }

    /// Release unused capacity.
    pub fn shrink_to_fit(&mut self) {
        if let Repr::Small(v) = &mut self.0 {
            v.shrink_to_fit();
        }
    }

    /// The dumped shape.
    pub fn to_json(&self) -> Map<String, J> {
        self.iter().map(|(k, v)| (k.to_string(), v.to_json())).collect()
    }
}

/// An entry of a [`PMap`] ([`PMap::entry`]).
pub struct PEntry<'a> {
    map: &'a mut PMap,
    key: String,
}

impl<'a> PEntry<'a> {
    /// The value, inserting `v` if absent.
    pub fn or_insert(self, v: PV) -> &'a mut PV {
        self.or_insert_with(|| v)
    }

    /// The value, inserting `f()` if absent.
    pub fn or_insert_with(self, f: impl FnOnce() -> PV) -> &'a mut PV {
        if !self.map.contains_key(&self.key) {
            self.map.insert(self.key.clone(), f());
        }
        self.map.get_mut(&self.key).expect("present")
    }
}

/// Position of `k` in a small map's sorted pairs.
fn search(v: &[(Key, PV)], k: &str) -> Result<usize, usize> {
    v.binary_search_by(|(x, _)| x.as_str().cmp(k))
}

impl PartialEq for PMap {
    fn eq(&self, other: &PMap) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl fmt::Debug for PMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl FromIterator<(String, PV)> for PMap {
    fn from_iter<I: IntoIterator<Item = (String, PV)>>(iter: I) -> PMap {
        let mut m = PMap::new();
        m.extend(iter);
        m
    }
}

impl Extend<(String, PV)> for PMap {
    fn extend<I: IntoIterator<Item = (String, PV)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl IntoIterator for PMap {
    type Item = (String, PV);
    type IntoIter = Box<dyn Iterator<Item = (String, PV)>>;
    fn into_iter(self) -> Self::IntoIter {
        match self.0 {
            Repr::Small(v) => Box::new(
                v.into_iter()
                    .map(|(k, v)| (k.as_str().to_string(), v)),
            ),
            Repr::Large(m) => Box::new(m.into_iter().map(|(k, v)| (k.into_string(), v))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_round_trip_and_order() {
        let v = json!({"z": "1", "un": ["base", ["family"]], "#n12": {}, "a": null, "b": {"c": 1.5, "d": true}});
        let p = PV::from_json(v.clone());
        assert_eq!(p.to_json(), v);
        let keys: Vec<&str> = p.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["#n12", "a", "b", "un", "z"]);
        assert_eq!(p["un"][0], PV::from("base"));
        assert_eq!(p.get("nope"), None);
    }

    #[test]
    fn small_and_large_agree() {
        let mut m = PMap::new();
        let mut b: BTreeMap<String, PV> = BTreeMap::new();
        for i in (0..100).rev() {
            let k = format!("k{}", (i * 37) % 100);
            m.insert(k.clone(), PV::from(i.to_string()));
            b.insert(k, PV::from(i.to_string()));
            assert_eq!(m.len(), b.len());
            assert!(m.iter().map(|(k, v)| (k.to_string(), v.clone())).eq(b.clone()));
        }
        assert_eq!(m.remove("k5"), b.remove("k5"));
        assert_eq!(m.get("k6"), b.get("k6"));
        let small: PMap = [("x".to_string(), PV::Null)].into_iter().collect();
        let mut large = m.clone();
        large.retain(|k, _| k == "x");
        large.insert("x".into(), PV::Null);
        assert_eq!(small, large);
    }
}

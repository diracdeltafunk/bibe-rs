//! [`SymMap`]: a small map keyed by interned names, replacing
//! `BTreeMap<String, V>` in the model.
//!
//! The model's maps are small (an entry has a few dozen fields, a name four
//! parts) and looked up far more often than changed. A `SymMap` is a `Vec` of
//! `(Sym, V)` pairs kept sorted by name:
//!
//! - lookups compare pointers ([`Sym`] equality), never strings;
//! - iteration is in the byte order of the names, exactly as the
//!   `BTreeMap<String, V>` it replaces (biber's `sort keys`);
//! - one allocation per map instead of one per node plus one per key.
//!
//! The API mirrors the parts of `BTreeMap`'s that the pipeline uses, so code
//! written against `BTreeMap<String, V>` keeps compiling: `get("title")`,
//! `insert("title".to_string(), v)`, `iter()`, `keys()`, `entry(k)`, ….
//! Iterators yield `&Sym` keys where `BTreeMap` yields `&String`; `Sym`
//! derefs to `str` and compares with strings.

use super::sym::{IntoSym, Sym, SymKey};
use std::fmt;

/// A map from interned names to `V`, iterated in name order. See the
/// [module docs](self).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SymMap<V> {
    items: Vec<(Sym, V)>,
}

impl<V> Default for SymMap<V> {
    fn default() -> Self {
        SymMap { items: Vec::new() }
    }
}

impl<V: fmt::Debug> fmt::Debug for SymMap<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<V> SymMap<V> {
    /// An empty map.
    pub const fn new() -> Self {
        SymMap { items: Vec::new() }
    }

    /// An empty map with room for `n` entries.
    pub fn with_capacity(n: usize) -> Self {
        SymMap {
            items: Vec::with_capacity(n),
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// The number of entries the map can hold without reallocating.
    pub fn capacity(&self) -> usize {
        self.items.capacity()
    }

    /// Release unused capacity.
    pub fn shrink_to_fit(&mut self) {
        self.items.shrink_to_fit();
    }

    /// Position of `k` (pointer comparison).
    #[inline]
    fn position(&self, k: Sym) -> Option<usize> {
        self.items.iter().position(|(s, _)| *s == k)
    }

    /// Where `k` is or would be inserted.
    #[inline]
    fn search(&self, k: Sym) -> Result<usize, usize> {
        self.items.binary_search_by(|(s, _)| s.cmp(&k))
    }

    /// Insert at position `i`, growing by half the length (at least 4)
    /// rather than doubling: these maps are many and small, and they are
    /// rarely grown after they are built.
    fn insert_at(&mut self, i: usize, k: Sym, v: V) {
        let len = self.items.len();
        if len == self.items.capacity() {
            self.items.reserve_exact((len / 2).max(4));
        }
        self.items.insert(i, (k, v));
    }

    /// The value of `k` (by symbol: no lookup of the name).
    #[inline]
    pub fn get_sym(&self, k: Sym) -> Option<&V> {
        self.items.iter().find(|(s, _)| *s == k).map(|(_, v)| v)
    }

    /// `BTreeMap::get`.
    #[inline]
    pub fn get<K: SymKey + ?Sized>(&self, k: &K) -> Option<&V> {
        self.get_sym(k.to_sym()?)
    }

    /// `BTreeMap::get_mut`.
    pub fn get_mut<K: SymKey + ?Sized>(&mut self, k: &K) -> Option<&mut V> {
        let i = self.position(k.to_sym()?)?;
        Some(&mut self.items[i].1)
    }

    /// `BTreeMap::get_key_value`.
    pub fn get_key_value<K: SymKey + ?Sized>(&self, k: &K) -> Option<(&Sym, &V)> {
        let i = self.position(k.to_sym()?)?;
        let (s, v) = &self.items[i];
        Some((s, v))
    }

    /// `BTreeMap::contains_key`.
    #[inline]
    pub fn contains_key<K: SymKey + ?Sized>(&self, k: &K) -> bool {
        k.to_sym().is_some_and(|s| self.position(s).is_some())
    }

    /// `BTreeMap::insert`: the old value if `k` was present.
    pub fn insert(&mut self, k: impl IntoSym, v: V) -> Option<V> {
        let k = k.into_sym();
        match self.search(k) {
            Ok(i) => Some(std::mem::replace(&mut self.items[i].1, v)),
            Err(i) => {
                self.insert_at(i, k, v);
                None
            }
        }
    }

    /// `BTreeMap::remove`.
    pub fn remove<K: SymKey + ?Sized>(&mut self, k: &K) -> Option<V> {
        let i = self.position(k.to_sym()?)?;
        Some(self.items.remove(i).1)
    }

    /// `BTreeMap::remove_entry`.
    pub fn remove_entry<K: SymKey + ?Sized>(&mut self, k: &K) -> Option<(Sym, V)> {
        let i = self.position(k.to_sym()?)?;
        Some(self.items.remove(i))
    }

    /// `BTreeMap::entry`.
    pub fn entry(&mut self, k: impl IntoSym) -> Entry<'_, V> {
        let k = k.into_sym();
        match self.search(k) {
            Ok(i) => Entry::Occupied(OccupiedEntry { map: self, i }),
            Err(i) => Entry::Vacant(VacantEntry {
                map: self,
                i,
                key: k,
            }),
        }
    }

    /// `BTreeMap::retain`.
    pub fn retain(&mut self, mut f: impl FnMut(&Sym, &mut V) -> bool) {
        self.items.retain_mut(|(k, v)| f(k, v));
    }

    /// `BTreeMap::append`: moves all of `other` into `self`.
    pub fn append(&mut self, other: &mut SymMap<V>) {
        for (k, v) in std::mem::take(&mut other.items) {
            self.insert(k, v);
        }
    }

    pub fn iter(&self) -> Iter<'_, V> {
        Iter(self.items.iter())
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, V> {
        IterMut(self.items.iter_mut())
    }

    pub fn keys(&self) -> impl DoubleEndedIterator<Item = &Sym> + ExactSizeIterator + Clone {
        self.items.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl DoubleEndedIterator<Item = &V> + ExactSizeIterator + Clone {
        self.items.iter().map(|(_, v)| v)
    }

    pub fn values_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut V> + ExactSizeIterator {
        self.items.iter_mut().map(|(_, v)| v)
    }

    pub fn into_keys(self) -> impl DoubleEndedIterator<Item = Sym> + ExactSizeIterator {
        self.items.into_iter().map(|(k, _)| k)
    }

    pub fn into_values(self) -> impl DoubleEndedIterator<Item = V> + ExactSizeIterator {
        self.items.into_iter().map(|(_, v)| v)
    }

    pub fn first_key_value(&self) -> Option<(&Sym, &V)> {
        self.items.first().map(|(k, v)| (k, v))
    }

    pub fn last_key_value(&self) -> Option<(&Sym, &V)> {
        self.items.last().map(|(k, v)| (k, v))
    }
}

impl<V, K: SymKey + ?Sized> std::ops::Index<&K> for SymMap<V> {
    type Output = V;
    /// `map["key"]`; panics if the key is absent.
    fn index(&self, k: &K) -> &V {
        self.get(k).expect("key not in SymMap")
    }
}

impl<V, K: IntoSym> FromIterator<(K, V)> for SymMap<V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut items: Vec<(Sym, V)> = iter.into_iter().map(|(k, v)| (k.into_sym(), v)).collect();
        // Stable sort, then keep the *last* of equal keys (`BTreeMap`'s
        // collect overwrites earlier values).
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out: Vec<(Sym, V)> = Vec::with_capacity(items.len());
        for (k, v) in items {
            match out.last_mut() {
                Some(last) if last.0 == k => last.1 = v,
                _ => out.push((k, v)),
            }
        }
        SymMap { items: out }
    }
}

impl<V, K: IntoSym> Extend<(K, V)> for SymMap<V> {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl<V, K: IntoSym, const N: usize> From<[(K, V); N]> for SymMap<V> {
    fn from(a: [(K, V); N]) -> Self {
        a.into_iter().collect()
    }
}

/// Iterator over `(&Sym, &V)` in name order.
#[derive(Clone)]
pub struct Iter<'a, V>(std::slice::Iter<'a, (Sym, V)>);

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (&'a Sym, &'a V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (k, v))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<V> DoubleEndedIterator for Iter<'_, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, v)| (k, v))
    }
}

impl<V> ExactSizeIterator for Iter<'_, V> {}

/// Iterator over `(&Sym, &mut V)` in name order.
pub struct IterMut<'a, V>(std::slice::IterMut<'a, (Sym, V)>);

impl<'a, V> Iterator for IterMut<'a, V> {
    type Item = (&'a Sym, &'a mut V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (&*k, v))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<V> DoubleEndedIterator for IterMut<'_, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, v)| (&*k, v))
    }
}

impl<V> ExactSizeIterator for IterMut<'_, V> {}

impl<'a, V> IntoIterator for &'a SymMap<V> {
    type Item = (&'a Sym, &'a V);
    type IntoIter = Iter<'a, V>;
    fn into_iter(self) -> Iter<'a, V> {
        self.iter()
    }
}

impl<'a, V> IntoIterator for &'a mut SymMap<V> {
    type Item = (&'a Sym, &'a mut V);
    type IntoIter = IterMut<'a, V>;
    fn into_iter(self) -> IterMut<'a, V> {
        self.iter_mut()
    }
}

impl<V> IntoIterator for SymMap<V> {
    type Item = (Sym, V);
    type IntoIter = std::vec::IntoIter<(Sym, V)>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

/// `btree_map::Entry`.
pub enum Entry<'a, V> {
    Occupied(OccupiedEntry<'a, V>),
    Vacant(VacantEntry<'a, V>),
}

pub struct OccupiedEntry<'a, V> {
    map: &'a mut SymMap<V>,
    i: usize,
}

pub struct VacantEntry<'a, V> {
    map: &'a mut SymMap<V>,
    i: usize,
    key: Sym,
}

impl<'a, V> Entry<'a, V> {
    pub fn or_insert(self, default: V) -> &'a mut V {
        self.or_insert_with(|| default)
    }

    pub fn or_insert_with(self, f: impl FnOnce() -> V) -> &'a mut V {
        match self {
            Entry::Occupied(o) => o.into_mut(),
            Entry::Vacant(v) => v.insert(f()),
        }
    }

    pub fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }

    pub fn and_modify(mut self, f: impl FnOnce(&mut V)) -> Self {
        if let Entry::Occupied(o) = &mut self {
            f(o.get_mut());
        }
        self
    }

    pub fn key(&self) -> &Sym {
        match self {
            Entry::Occupied(o) => o.key(),
            Entry::Vacant(v) => &v.key,
        }
    }
}

impl<'a, V> OccupiedEntry<'a, V> {
    pub fn key(&self) -> &Sym {
        &self.map.items[self.i].0
    }
    pub fn get(&self) -> &V {
        &self.map.items[self.i].1
    }
    pub fn get_mut(&mut self) -> &mut V {
        &mut self.map.items[self.i].1
    }
    pub fn into_mut(self) -> &'a mut V {
        &mut self.map.items[self.i].1
    }
    pub fn insert(&mut self, v: V) -> V {
        std::mem::replace(self.get_mut(), v)
    }
    pub fn remove(self) -> V {
        self.map.items.remove(self.i).1
    }
}

impl<'a, V> VacantEntry<'a, V> {
    pub fn key(&self) -> &Sym {
        &self.key
    }
    pub fn insert(self, v: V) -> &'a mut V {
        self.map.insert_at(self.i, self.key, v);
        &mut self.map.items[self.i].1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behaves_like_a_btreemap() {
        use std::collections::BTreeMap;
        let keys = [
            "title",
            "author",
            "year",
            "Title",
            "",
            "authortype",
            "year",
            "zz",
            "a",
        ];
        let mut m = SymMap::new();
        let mut b = BTreeMap::new();
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(m.insert(*k, i), b.insert(k.to_string(), i));
        }
        assert_eq!(
            m.iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<Vec<_>>(),
            b.iter().map(|(k, v)| (k.clone(), *v)).collect::<Vec<_>>()
        );
        assert_eq!(m.get("year"), b.get("year"));
        assert_eq!(m.get("nothere-xyz"), None);
        assert_eq!(m.remove("author"), b.remove("author"));
        assert!(!m.contains_key("author"));
        *m.entry("new").or_insert(0) += 5;
        *b.entry("new".to_string()).or_insert(0) += 5;
        m.retain(|k, _| k.as_str() != "zz");
        b.retain(|k, _| k != "zz");
        assert_eq!(
            m.iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<Vec<_>>(),
            b.iter().map(|(k, v)| (k.clone(), *v)).collect::<Vec<_>>()
        );
        let c: SymMap<usize> = [("b", 1), ("a", 2), ("b", 3)].into_iter().collect();
        assert_eq!(
            c.iter().map(|(k, v)| (k.as_str(), *v)).collect::<Vec<_>>(),
            [("a", 2), ("b", 3)]
        );
    }
}

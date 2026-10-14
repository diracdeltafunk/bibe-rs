//! [`EntryMap`]: a section's entries by citekey (`bibentries`).
//!
//! Entries are looked up by key all the time (every phase, every entry,
//! often several times) and iterated rarely. A `BTreeMap<String, Entry>`
//! costs a few dozen string comparisons per lookup on large sections (keys
//! share long prefixes); this map hashes the key once. Iteration is in key
//! order, exactly like the `BTreeMap` it replaces: the sorted key list is
//! built on first use and kept until a key is added or removed.
//!
//! The API mirrors the parts of `BTreeMap`'s that the pipeline uses.

use super::Entry;
use rustc_hash::FxHashMap;
use std::borrow::Borrow;
use std::fmt;
use std::hash::Hash;
use std::sync::OnceLock;

/// Citekey → entry, iterated in key order. See the [module docs](self).
#[derive(Clone, Default)]
pub struct EntryMap {
    map: FxHashMap<String, Entry>,
    /// The keys of `map`, sorted; reset when a key is added or removed.
    sorted: OnceLock<Vec<String>>,
}

impl PartialEq for EntryMap {
    fn eq(&self, other: &EntryMap) -> bool {
        self.map == other.map
    }
}

impl fmt::Debug for EntryMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl EntryMap {
    pub fn new() -> EntryMap {
        EntryMap::default()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.sorted = OnceLock::new();
    }

    #[inline]
    pub fn get<Q>(&self, k: &Q) -> Option<&Entry>
    where
        String: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.get(k)
    }

    #[inline]
    pub fn get_mut<Q>(&mut self, k: &Q) -> Option<&mut Entry>
    where
        String: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.get_mut(k)
    }

    #[inline]
    pub fn contains_key<Q>(&self, k: &Q) -> bool
    where
        String: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.contains_key(k)
    }

    /// `BTreeMap::get_key_value`.
    pub fn get_key_value<Q>(&self, k: &Q) -> Option<(&String, &Entry)>
    where
        String: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.get_key_value(k)
    }

    /// `BTreeMap::insert`: the old entry if `k` was present.
    pub fn insert(&mut self, k: String, e: Entry) -> Option<Entry> {
        if let Some(old) = self.map.get_mut(&k) {
            return Some(std::mem::replace(old, e));
        }
        self.sorted = OnceLock::new();
        self.map.insert(k, e)
    }

    pub fn remove<Q>(&mut self, k: &Q) -> Option<Entry>
    where
        String: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let e = self.map.remove(k)?;
        self.sorted = OnceLock::new();
        Some(e)
    }

    /// `BTreeMap::retain`.
    pub fn retain(&mut self, mut f: impl FnMut(&String, &mut Entry) -> bool) {
        let n = self.map.len();
        self.map.retain(|k, v| f(k, v));
        if self.map.len() != n {
            self.sorted = OnceLock::new();
        }
    }

    /// The keys, sorted.
    fn sorted_keys(&self) -> &[String] {
        self.sorted.get_or_init(|| {
            let mut v: Vec<String> = self.map.keys().cloned().collect();
            v.sort_unstable();
            v
        })
    }

    /// Keys in order.
    pub fn keys(&self) -> std::slice::Iter<'_, String> {
        self.sorted_keys().iter()
    }

    /// Entries in key order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&String, &Entry)> + ExactSizeIterator {
        self.sorted_keys().iter().map(|k| (k, &self.map[k]))
    }

    /// Entries in key order.
    pub fn values(&self) -> impl DoubleEndedIterator<Item = &Entry> + ExactSizeIterator {
        self.sorted_keys().iter().map(|k| &self.map[k])
    }

    /// Mutable entries in key order.
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = (&String, &mut Entry)> {
        let mut v: Vec<(&String, &mut Entry)> = self.map.iter_mut().collect();
        v.sort_unstable_by(|a, b| a.0.cmp(b.0));
        v.into_iter()
    }

    /// Mutable entries in key order.
    pub fn values_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut Entry> {
        self.iter_mut().map(|(_, e)| e)
    }

    /// Mutable entries in no particular order (cheaper than
    /// [`EntryMap::values_mut`] when the order does not matter).
    pub fn values_mut_unordered(&mut self) -> impl Iterator<Item = &mut Entry> {
        self.map.values_mut()
    }
}

impl<Q> std::ops::Index<&Q> for EntryMap
where
    String: Borrow<Q>,
    Q: Hash + Eq + ?Sized,
{
    type Output = Entry;
    /// `entries[key]`; panics if the key is absent.
    fn index(&self, k: &Q) -> &Entry {
        self.map.get(k).expect("no entry for key")
    }
}

impl FromIterator<(String, Entry)> for EntryMap {
    fn from_iter<I: IntoIterator<Item = (String, Entry)>>(iter: I) -> EntryMap {
        EntryMap {
            map: iter.into_iter().collect(),
            sorted: OnceLock::new(),
        }
    }
}

impl Extend<(String, Entry)> for EntryMap {
    fn extend<I: IntoIterator<Item = (String, Entry)>>(&mut self, iter: I) {
        for (k, e) in iter {
            self.insert(k, e);
        }
    }
}

impl<'a> IntoIterator for &'a EntryMap {
    type Item = (&'a String, &'a Entry);
    type IntoIter = Box<dyn Iterator<Item = (&'a String, &'a Entry)> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterates_in_key_order() {
        let mut m = EntryMap::new();
        for k in ["b", "a", "c:1", "c", "B"] {
            m.insert(k.to_string(), Entry::default());
        }
        assert_eq!(m.keys().collect::<Vec<_>>(), ["B", "a", "b", "c", "c:1"]);
        m.remove("a");
        m.insert("aa".into(), Entry::default());
        assert_eq!(m.keys().collect::<Vec<_>>(), ["B", "aa", "b", "c", "c:1"]);
        assert_eq!(m.values_mut().count(), 5);
        assert!(m.contains_key("c:1"));
        let _ = &m["b"];
    }
}

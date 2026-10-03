//! A byte trie over UTF-8 keys, used to emulate Perl alternations of literal
//! keys (`foo|foobar|…`).
//!
//! Because keys and the subject are both valid UTF-8, a key that matches the
//! subject's bytes starting at a char boundary also ends on a char boundary.

const NONE: u32 = u32::MAX;

#[derive(Clone, Debug, Default)]
struct Node {
    /// Sorted by byte.
    children: Vec<(u8, u32)>,
    value: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Trie {
    nodes: Vec<Node>,
    /// Dense child table for the root.
    root: Box<[u32; 256]>,
}

impl Default for Trie {
    fn default() -> Self {
        Self::new()
    }
}

impl Trie {
    pub(crate) fn new() -> Self {
        Trie {
            nodes: vec![Node {
                children: Vec::new(),
                value: NONE,
            }],
            root: Box::new([NONE; 256]),
        }
    }

    #[inline]
    fn child(&self, node: u32, b: u8) -> u32 {
        if node == 0 {
            return self.root[b as usize];
        }
        let ch = &self.nodes[node as usize].children;
        match ch.binary_search_by_key(&b, |&(k, _)| k) {
            Ok(i) => ch[i].1,
            Err(_) => NONE,
        }
    }

    pub(crate) fn insert(&mut self, key: &str, value: u32) {
        let mut node = 0u32;
        for &b in key.as_bytes() {
            let next = self.child(node, b);
            node = if next == NONE {
                let id = self.nodes.len() as u32;
                self.nodes.push(Node {
                    children: Vec::new(),
                    value: NONE,
                });
                if node == 0 {
                    self.root[b as usize] = id;
                } else {
                    let ch = &mut self.nodes[node as usize].children;
                    let pos = ch.partition_point(|&(k, _)| k < b);
                    ch.insert(pos, (b, id));
                }
                id
            } else {
                next
            };
        }
        self.nodes[node as usize].value = value;
    }

    /// Exact lookup.
    pub(crate) fn get(&self, key: &[u8]) -> Option<u32> {
        let mut node = 0u32;
        for &b in key {
            node = self.child(node, b);
            if node == NONE {
                return None;
            }
        }
        let v = self.nodes[node as usize].value;
        (v != NONE).then_some(v)
    }

    /// Whether some key starts with byte `b`.
    #[inline]
    pub(crate) fn starts_with_byte(&self, b: u8) -> bool {
        self.root[b as usize] != NONE
    }

    /// Whether the empty string is a key.
    #[inline]
    pub(crate) fn has_empty(&self) -> bool {
        self.nodes[0].value != NONE
    }

    /// Appends to `out` every key that is a prefix of `text[start..]`, as
    /// `(end, value)`, in increasing length order.
    #[inline]
    pub(crate) fn prefixes(&self, text: &[u8], start: usize, out: &mut Vec<(usize, u32)>) {
        out.clear();
        let mut node = 0u32;
        if self.nodes[0].value != NONE {
            out.push((start, self.nodes[0].value));
        }
        for (i, &b) in text[start..].iter().enumerate() {
            node = self.child(node, b);
            if node == NONE {
                return;
            }
            let v = self.nodes[node as usize].value;
            if v != NONE {
                out.push((start + i + 1, v));
            }
        }
    }

    /// The shortest key that is a prefix of `text[start..]`.
    #[inline]
    pub(crate) fn shortest_prefix(&self, text: &[u8], start: usize) -> Option<(usize, u32)> {
        let mut node = 0u32;
        if self.nodes[0].value != NONE {
            return Some((start, self.nodes[0].value));
        }
        for (i, &b) in text[start..].iter().enumerate() {
            node = self.child(node, b);
            if node == NONE {
                return None;
            }
            let v = self.nodes[node as usize].value;
            if v != NONE {
                return Some((start + i + 1, v));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_in_length_order() {
        let mut t = Trie::new();
        t.insert("text", 0);
        t.insert("textthorn", 1);
        t.insert("t", 2);
        let mut v = Vec::new();
        t.prefixes(b"\\textthornvari", 1, &mut v);
        assert_eq!(v, vec![(2, 2), (5, 0), (10, 1)]);
        assert_eq!(t.shortest_prefix(b"xtext", 1), Some((2, 2)));
        assert_eq!(t.get(b"text"), Some(0));
        assert_eq!(t.get(b"tex"), None);
    }
}

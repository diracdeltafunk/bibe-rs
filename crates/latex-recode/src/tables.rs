//! Decode/encode tables for one (decode set, encode set) pair, built exactly
//! like `Biber::LaTeX::Recode::init_sets`.

use std::collections::{HashMap, HashSet};

use unicode_normalization::UnicodeNormalization;

use crate::RecodeOptions;
use crate::trie::Trie;
use crate::xmldata::RawData;

/// The recode map types, in `init_sets`' `@types` order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ty {
    Letters = 0,
    Diacritics,
    Punctuation,
    Symbols,
    NegatedSymbols,
    Superscripts,
    CmdSuperscripts,
    Dings,
    Greek,
}

pub(crate) const TYPE_NAMES: [&str; 9] = [
    "letters",
    "diacritics",
    "punctuation",
    "symbols",
    "negatedsymbols",
    "superscripts",
    "cmdsuperscripts",
    "dings",
    "greek",
];

const ALL_TYPES: [Ty; 9] = [
    Ty::Letters,
    Ty::Diacritics,
    Ty::Punctuation,
    Ty::Symbols,
    Ty::NegatedSymbols,
    Ty::Superscripts,
    Ty::CmdSuperscripts,
    Ty::Dings,
    Ty::Greek,
];

pub(crate) fn nfd(s: &str) -> String {
    if s.is_ascii() {
        s.to_owned()
    } else {
        s.nfd().collect()
    }
}

/// The keys of one type's map, as an alternation matcher.
#[derive(Clone, Debug)]
pub(crate) struct KeyMap {
    pub(crate) trie: Trie,
    pub(crate) keys: Vec<String>,
    pub(crate) values: Vec<String>,
    /// Distinct key lengths in bytes (for the decode look-behind emulation).
    pub(crate) key_lens: Vec<usize>,
    /// Per key: does it end with `[A-Za-z]` (decode diacritics conditional).
    pub(crate) ends_ascii_letter: Vec<bool>,
}

impl KeyMap {
    fn new(map: HashMap<String, String>) -> KeyMap {
        let mut entries: Vec<(String, String)> = map.into_iter().collect();
        entries.sort();
        let mut trie = Trie::new();
        let mut keys = Vec::with_capacity(entries.len());
        let mut values = Vec::with_capacity(entries.len());
        let mut key_lens = Vec::new();
        let mut ends_ascii_letter = Vec::with_capacity(entries.len());
        for (i, (k, v)) in entries.into_iter().enumerate() {
            trie.insert(&k, i as u32);
            if !key_lens.contains(&k.len()) {
                key_lens.push(k.len());
            }
            // Perl `m:[A-Za-z]$:`; `$` also matches before a final newline.
            let kb = k.strip_suffix('\n').unwrap_or(&k).as_bytes();
            ends_ascii_letter.push(kb.last().is_some_and(u8::is_ascii_alphabetic));
            keys.push(k);
            values.push(v);
        }
        key_lens.sort_unstable();
        KeyMap {
            trie,
            keys,
            values,
            key_lens,
            ends_ascii_letter,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DecodeTables {
    pub(crate) types: [Option<KeyMap>; 9],
}

#[derive(Clone, Debug)]
pub(crate) struct EncodeTables {
    /// `latex_encode` returns its input unchanged for the set name `null`.
    pub(crate) null: bool,
    pub(crate) types: [Option<KeyMap>; 9],
    /// `$remap_e_raw`: Unicode strings (from any type) to insert as is.
    pub(crate) raw: HashSet<String>,
}

impl DecodeTables {
    pub(crate) fn get(&self, ty: Ty) -> Option<&KeyMap> {
        self.types[ty as usize].as_ref()
    }
}

impl EncodeTables {
    pub(crate) fn get(&self, ty: Ty) -> Option<&KeyMap> {
        self.types[ty as usize].as_ref()
    }
}

fn in_set(sets: &[String], set: &str) -> bool {
    sets.iter().any(|s| s == set)
}

/// TeX special characters that must not appear raw in a `.bbl` (BB-029a fix).
fn has_tex_special(v: &str) -> bool {
    v.contains(['$', '%', '&', '#', '_', '^', '~', '\\', '{', '}'])
}

pub(crate) fn build_decode(data: &RawData, set: &str, opts: &RecodeOptions) -> DecodeTables {
    let excludes: Vec<String> = data.decode_exclude.iter().map(|c| nfd(c)).collect();
    let types = ALL_TYPES.map(|ty| {
        let name = TYPE_NAMES[ty as usize];
        let mut map: Option<HashMap<String, String>> = None;
        for block in data.blocks.iter().filter(|b| b.ty == name) {
            if !in_set(&block.sets, set) {
                continue;
            }
            for m in &block.maps {
                map.get_or_insert_with(HashMap::new)
                    .insert(nfd(&m.from), nfd(&m.to));
            }
        }
        let mut map = map?;
        for e in &excludes {
            map.remove(e);
        }
        if opts.fix_raw_specials {
            map.retain(|_, v| !has_tex_special(v));
        }
        if map.is_empty() {
            // Perl joins an empty key list into an empty alternation, which
            // matches the empty string; `$map->{''}` is undef, i.e. ''.
            map.insert(String::new(), String::new());
        }
        Some(KeyMap::new(map))
    });
    DecodeTables { types }
}

pub(crate) fn build_encode(data: &RawData, set: &str) -> EncodeTables {
    let excludes: Vec<String> = data.encode_exclude.iter().map(|c| nfd(c)).collect();
    let mut raw = HashSet::new();
    let types = ALL_TYPES.map(|ty| {
        let name = TYPE_NAMES[ty as usize];
        let mut map: Option<HashMap<String, String>> = None;
        for block in data.blocks.iter().filter(|b| b.ty == name) {
            if !in_set(&block.sets, set) {
                continue;
            }
            for m in &block.maps {
                map.get_or_insert_with(HashMap::new)
                    .insert(nfd(&m.to), nfd(&m.from));
            }
            // Preferred encodings override, per block.
            for m in block.maps.iter().filter(|m| m.preferred) {
                map.get_or_insert_with(HashMap::new)
                    .insert(nfd(&m.to), nfd(&m.from));
            }
            for m in block.maps.iter().filter(|m| m.raw) {
                raw.insert(nfd(&m.to));
            }
        }
        let mut map = map?;
        for e in &excludes {
            map.remove(e);
        }
        // An empty key (or an empty alternation) would match the empty string
        // at every position; such data is not supported (see crate docs).
        map.remove("");
        if map.is_empty() {
            return None;
        }
        Some(KeyMap::new(map))
    });
    EncodeTables {
        null: set == "null",
        types,
        raw,
    }
}

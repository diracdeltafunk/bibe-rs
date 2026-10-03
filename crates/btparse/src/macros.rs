//! The macro (`@string`) table (spec §4).

use std::collections::HashMap;

/// btparse's macro table: process-global in libbtparse, accumulating across
/// files and refsections unless explicitly cleared. Names are compared
/// ASCII-case-insensitively (stored lower-cased).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MacroTable {
    map: HashMap<String, String>,
}

/// The month macros biber installs before reading data (`%MONTHS`).
pub const BIBER_MONTHS: [(&str, &str); 12] = [
    ("jan", "1"),
    ("feb", "2"),
    ("mar", "3"),
    ("apr", "4"),
    ("may", "5"),
    ("jun", "6"),
    ("jul", "7"),
    ("aug", "8"),
    ("sep", "9"),
    ("oct", "10"),
    ("nov", "11"),
    ("dec", "12"),
];

/// The month macros Text::BibTeX defines when it is loaded.
pub const TEXT_BIBTEX_MONTHS: [(&str, &str); 12] = [
    ("jan", "January"),
    ("feb", "February"),
    ("mar", "March"),
    ("apr", "April"),
    ("may", "May"),
    ("jun", "June"),
    ("jul", "July"),
    ("aug", "August"),
    ("sep", "September"),
    ("oct", "October"),
    ("nov", "November"),
    ("dec", "December"),
];

pub(crate) fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl MacroTable {
    /// An empty table (state after `delete_all_macros`).
    pub fn new() -> Self {
        Self::default()
    }

    /// The table as Text::BibTeX has it right after loading (month names).
    pub fn text_bibtex_initial() -> Self {
        let mut t = Self::new();
        for (k, v) in TEXT_BIBTEX_MONTHS {
            t.map.insert(k.to_string(), v.to_string());
        }
        t
    }

    /// The table as biber has it before reading its first data source
    /// (Text::BibTeX defaults, then `jan`..`dec` redefined to `1`..`12`).
    pub fn biber_initial() -> Self {
        let mut t = Self::text_bibtex_initial();
        t.install_biber_months();
        t
    }

    /// biber's `delete_macro(m); add_macro_text(m, n)` for each month
    /// (Biber.pm `fetch_data`). Silent.
    pub fn install_biber_months(&mut self) {
        for (k, v) in BIBER_MONTHS {
            self.map.insert(k.to_string(), v.to_string());
        }
    }

    /// `add_macro_text`: define or redefine. Returns `true` if a definition
    /// was overridden (btparse then warns `overriding existing definition of
    /// macro "<name>"`, which biber ignores).
    pub fn define(&mut self, name: &str, text: &str) -> bool {
        self.map.insert(fold(name), text.to_string()).is_some()
    }

    /// `delete_macro`.
    pub fn delete(&mut self, name: &str) {
        self.map.remove(&fold(name));
    }

    /// `delete_all_macros` (biber's `--clrmacros`).
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// `macro_text`: the expansion, if defined.
    pub fn get(&self, name: &str) -> Option<&str> {
        if name.bytes().any(|b| b.is_ascii_uppercase()) {
            self.map.get(&fold(name)).map(String::as_str)
        } else {
            self.map.get(name).map(String::as_str)
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// All definitions (unordered).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

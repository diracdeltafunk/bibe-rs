//! Perl `Encode` for the encodings biber users name in `--input-encoding`
//! and `--output-encoding`: UTF-8 and the single-byte encodings of
//! `Encode::Byte`/`Encode` core (ISO-8859-*, Windows and DOS code pages,
//! KOI8, Mac encodings), with the aliases `Encode::find_encoding` resolves
//! and biber's own (`Constants.pm:367-375`: `ansinew`, `applemac`,
//! `applemacce`, …). Tables are generated from oracle O3
//! (`tools/oracle/probes/perl-compat/gen_encoding_tables.pl`).
//!
//! Multi-byte encodings other than UTF-8 are not supported
//! ([`find_encoding`] returns `None`).
//!
//! ```
//! use perl_compat::encoding::find_encoding;
//! let e = find_encoding("latin1").unwrap();
//! assert_eq!(e.name(), "iso-8859-1");
//! assert!(e.can_encode('é') && !e.can_encode('€'));
//! assert_eq!(e.encode_perlqq("é€"), b"\xe9\\x{20ac}".to_vec());
//! assert_eq!(find_encoding("applemacce").unwrap().name(), "MacCentralEurRoman");
//! ```

use crate::tables::encodings::{ALIASES, ByteEncoding, ENCODINGS};

/// An encoding found by [`find_encoding`].
#[derive(Clone, Copy)]
pub struct Encoding(Inner);

#[derive(Clone, Copy)]
enum Inner {
    /// UTF-8 (`utf8`/`UTF-8` in Perl).
    Utf8,
    /// A single-byte encoding.
    Byte(&'static ByteEncoding),
}

impl std::fmt::Debug for Encoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Encoding({})", self.name())
    }
}

/// `Encode::find_encoding($name)` (case-insensitive), restricted to the
/// supported encodings.
pub fn find_encoding(name: &str) -> Option<Encoding> {
    let l = name.to_ascii_lowercase();
    if matches!(l.as_str(), "utf8" | "utf-8" | "utf-8-strict") {
        return Some(Encoding::UTF8);
    }
    let canon = ALIASES
        .binary_search_by(|(a, _)| (*a).cmp(l.as_str()))
        .ok()
        .map(|i| ALIASES[i].1)?;
    ENCODINGS
        .iter()
        .find(|e| e.name == canon)
        .map(|b| Encoding(Inner::Byte(b)))
}

impl Encoding {
    /// UTF-8.
    pub const UTF8: Encoding = Encoding(Inner::Utf8);

    /// The canonical Encode name (`$enc->name`).
    pub fn name(&self) -> &'static str {
        match self.0 {
            Inner::Utf8 => "utf-8-strict",
            Inner::Byte(b) => b.name,
        }
    }

    fn byte(b: &ByteEncoding, c: char) -> Option<u8> {
        b.encode
            .binary_search_by_key(&(c as u32), |(cp, _)| *cp)
            .ok()
            .map(|i| b.encode[i].1)
    }

    /// Can `c` be encoded?
    pub fn can_encode(&self, c: char) -> bool {
        match self.0 {
            Inner::Utf8 => true,
            Inner::Byte(b) => Self::byte(b, c).is_some(),
        }
    }

    /// `encode($enc, $s, sub {"\0"}) =~ /\0/`: does `s` contain a character
    /// that cannot be encoded (or a NUL)?
    pub fn has_unencodable(&self, s: &str) -> bool {
        s.chars().any(|c| c == '\0' || !self.can_encode(c))
    }

    /// Encode as a `:encoding(…)` output layer does (`FB_PERLQQ`):
    /// unmappable characters become `\x{XXXX}` text.
    pub fn encode_perlqq(&self, s: &str) -> Vec<u8> {
        match self.0 {
            Inner::Utf8 => s.as_bytes().to_vec(),
            Inner::Byte(b) => {
                let mut out = Vec::with_capacity(s.len());
                for c in s.chars() {
                    match Self::byte(b, c) {
                        Some(x) => out.push(x),
                        None => out.extend(format!("\\x{{{:04x}}}", c as u32).bytes()),
                    }
                }
                out
            }
        }
    }

    /// Decode bytes (`decode($enc, $bytes)`, default check: undefined bytes
    /// and malformed UTF-8 become U+FFFD).
    pub fn decode(&self, bytes: &[u8]) -> String {
        match self.0 {
            Inner::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
            Inner::Byte(b) => bytes
                .iter()
                .map(|x| char::from_u32(b.decode[*x as usize]).unwrap_or('\u{FFFD}'))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_sorted() {
        assert!(ALIASES.windows(2).all(|w| w[0].0 < w[1].0));
        for e in ENCODINGS {
            assert!(e.encode.windows(2).all(|w| w[0].0 < w[1].0), "{}", e.name);
        }
    }

    #[test]
    fn round_trips() {
        let e = find_encoding("cp1252").unwrap();
        assert_eq!(e.decode(&e.encode_perlqq("Œuvre €5")), "Œuvre €5");
        let e = find_encoding("latin9").unwrap();
        assert_eq!(e.name(), "iso-8859-15");
        assert!(e.can_encode('€'));
        assert!(find_encoding("UTF-8").is_some());
        assert!(find_encoding("shiftjis").is_none());
    }
}

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

    /// Size of the buffer of a PerlIO `:encoding(…)` layer
    /// (`ext/PerlIO-encoding/encoding.xs`, `PerlIOEncode_get_base`).
    pub const PERLIO_ENCODING_BUFSIZ: usize = 1024;

    /// Encode as writing `s` through a PerlIO `:encoding(…)` layer really
    /// does, including its buffer-boundary bug: the layer collects the
    /// string's internal UTF-8 bytes in a 1024-byte buffer and encodes each
    /// full buffer separately, so a multi-byte character that straddles a
    /// buffer boundary is split. If the leading fragment is a prefix of the
    /// UTF-8 of some encodable character, `Encode` stops there and the
    /// fragment is carried into the next buffer (the character survives, and
    /// later boundaries shift by the carried length). Otherwise the leading
    /// fragment becomes one `\x{fffd}` and each byte of the trailing
    /// fragment another one. Verified against perl 5.42 / Encode (O3).
    /// UTF-8 output is unaffected.
    pub fn encode_perlio_layer(&self, s: &str) -> Vec<u8> {
        let Inner::Byte(b) = self.0 else {
            return s.as_bytes().to_vec();
        };
        const FFFD: &[u8] = b"\\x{fffd}";
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0; // next stream byte not yet in a buffer
        let mut carry: Vec<u8> = Vec::new();
        while i < bytes.len() || !carry.is_empty() {
            let take = (Self::PERLIO_ENCODING_BUFSIZ - carry.len()).min(bytes.len() - i);
            let mut buf = std::mem::take(&mut carry);
            buf.extend_from_slice(&bytes[i..i + take]);
            i += take;
            let last = i == bytes.len();
            let mut j = 0;
            while j < buf.len() {
                let lead = buf[j];
                let need = match lead {
                    0x00..=0x7F => 1,
                    0xC0..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    0xF0..=0xF7 => 4,
                    _ => 0, // continuation byte without a lead byte
                };
                if need == 0 {
                    out.extend_from_slice(FFFD);
                    j += 1;
                } else if j + need <= buf.len() {
                    let c = std::str::from_utf8(&buf[j..j + need])
                        .ok()
                        .and_then(|x| x.chars().next())
                        .unwrap_or('\u{FFFD}');
                    match Self::byte(b, c) {
                        Some(x) => out.push(x),
                        None => out.extend(format!("\\x{{{:04x}}}", c as u32).bytes()),
                    }
                    j += need;
                } else {
                    // A character cut by the buffer boundary.
                    let frag = &buf[j..];
                    if !last && Self::is_encodable_prefix(b, frag) {
                        carry = frag.to_vec();
                    } else {
                        out.extend_from_slice(FFFD);
                    }
                    j = buf.len();
                }
            }
        }
        out
    }

    /// Is `frag` a proper prefix of the UTF-8 form of an encodable character?
    fn is_encodable_prefix(b: &ByteEncoding, frag: &[u8]) -> bool {
        let mut tmp = [0u8; 4];
        b.encode.iter().any(|(cp, _)| {
            char::from_u32(*cp).is_some_and(|c| {
                let u = c.encode_utf8(&mut tmp).as_bytes();
                u.len() > frag.len() && u.starts_with(frag)
            })
        })
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

    /// Expected outputs recorded from perl 5.42 (O3): `print` of `"a" x
    /// (1024 - split) . $ch . "Z"` to a `:encoding($enc)` file handle.
    #[test]
    fn perlio_layer_buffer_splits() {
        let tail = |enc: &str, ch: &str, split: usize| -> String {
            let e = find_encoding(enc).unwrap();
            let s = format!("{}{ch}Z", "a".repeat(1024 - split));
            let out = e.encode_perlio_layer(&s);
            String::from_utf8_lossy(&out[1024 - split..]).into_owned()
        };
        let f = |n: usize| format!("{}Z", "\\x{fffd}".repeat(n));
        assert_eq!(tail("ascii", "é", 1), f(2));
        assert_eq!(tail("ascii", "λ", 1), f(2));
        assert_eq!(tail("ascii", "ﬁ", 1), f(3));
        assert_eq!(tail("ascii", "ﬁ", 2), f(2));
        assert_eq!(tail("ascii", "😀", 1), f(4));
        assert_eq!(tail("ascii", "😀", 2), f(3));
        assert_eq!(tail("ascii", "😀", 3), f(2));
        assert_eq!(tail("latin1", "é", 1), "\u{FFFD}Z"); // byte E9 (lossy view)
        assert_eq!(tail("latin1", "λ", 1), f(2));
        assert_eq!(tail("latin1", "€", 1), f(3));
        assert_eq!(tail("latin1", "€", 2), f(2));
        assert_eq!(tail("cp1252", "€", 1), "\u{FFFD}Z"); // byte 80
        assert_eq!(tail("cp1252", "€", 2), "\u{FFFD}Z");
        assert_eq!(tail("cp1252", "\u{2010}", 1), "\\x{2010}Z");
        assert_eq!(tail("cp1252", "\u{2010}", 2), "\\x{2010}Z");
        // A carried fragment shifts the next boundary (1023 + 1024).
        let e = find_encoding("latin1").unwrap();
        for (pos, want) in [(2046, "\\x{fffd}\\x{fffd}Z"), (2047, "\\x{03bb}Z")] {
            let s = format!("{}é{}λZ", "a".repeat(1023), "b".repeat(pos - 1025));
            let out = e.encode_perlio_layer(&s);
            assert_eq!(String::from_utf8_lossy(&out[pos - 1..]), want, "λ at {pos}");
        }
        // Short strings and UTF-8 are unaffected.
        assert_eq!(e.encode_perlio_layer("é€"), e.encode_perlqq("é€"));
        assert_eq!(Encoding::UTF8.encode_perlio_layer("λ"), "λ".as_bytes());
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

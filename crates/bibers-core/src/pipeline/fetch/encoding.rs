//! Strict decoding of data source files in a Perl `Encode` encoding
//! (`File::Slurper::read_text($filename, $encoding)`, Utils.pm:171).

use perl_compat::encoding::{Lookup, lookup};

/// File::Slurper reads names matching `/^utf-?8\b/i` with `:utf8_strict`,
/// everything else with `:raw:encoding($encoding)`.
fn is_utf8_strict_layer(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    let rest = l.strip_prefix("utf-8").or_else(|| l.strip_prefix("utf8"));
    rest.is_some_and(|r| !r.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
}

/// Decode `bytes` in the Perl encoding `name`, failing on any byte sequence
/// that is invalid in it (Perl's strict `:encoding()` read). The error is a
/// short description (biber appends Perl's `$@`; docs/compat.md DC-046).
pub fn decode(bytes: &[u8], name: &str, utf16_chunks: bool) -> Result<String, String> {
    if is_utf8_strict_layer(name) {
        let s = String::from_utf8(bytes.to_vec()).map_err(|e| {
            let b = bytes[e.utf8_error().valid_up_to()];
            format!("utf8 \"\\x{b:02X}\" does not map to Unicode")
        })?;
        // PerlIO::utf8_strict also rejects noncharacters.
        if let Some(c) = s.chars().find(|c| {
            let v = *c as u32;
            (v & 0xFFFE) == 0xFFFE || (0xFDD0..=0xFDEF).contains(&v)
        }) {
            return Err(format!(
                "Can't interchange noncharacter code point U+{:04X}",
                c as u32
            ));
        }
        return Ok(s);
    }
    match lookup(name) {
        Lookup::Found(e) => e.decode_layer_chunked(bytes, utf16_chunks),
        Lookup::Unsupported(c) => Err(format!(
            "Encoding '{c}' is not supported by bibers (docs/compat.md DC-034)"
        )),
        Lookup::Unknown => Err(format!("Unknown encoding '{name}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_strictness() {
        assert_eq!(decode(b"a\xe9", "latin1", true).unwrap(), "a\u{e9}");
        assert_eq!(decode(b"\xa4", "latin9", true).unwrap(), "\u{20ac}");
        assert_eq!(decode(b"\x80", "cp1252", true).unwrap(), "\u{20ac}");
        assert!(decode(b"\x81", "cp1252", true).is_err());
        assert!(decode(b"\xe9", "UTF-8", true).is_err());
        assert!(decode(b"\xe9", "ascii", true).is_err());
        assert_eq!(decode(b"\xb1", "latin2", true).unwrap(), "\u{105}");
        assert!(decode(b"x", "nonsense", true).is_err());
        assert_eq!(decode(b"\x82\xa0", "sjis", true).unwrap(), "\u{3042}");
        assert_eq!(decode(b"\xff\xfea\x00", "UTF-16", true).unwrap(), "a");
        assert!(is_utf8_strict_layer("UTF-8") && is_utf8_strict_layer("utf8"));
        assert!(!is_utf8_strict_layer("utf8x"));
    }
}

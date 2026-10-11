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
pub fn decode(bytes: &[u8], name: &str) -> Result<String, String> {
    if is_utf8_strict_layer(name) {
        return String::from_utf8(bytes.to_vec()).map_err(|e| {
            let b = bytes[e.utf8_error().valid_up_to()];
            format!("utf8 \"\\x{b:02X}\" does not map to Unicode")
        });
    }
    match lookup(name) {
        Lookup::Found(e) => e.decode_layer(bytes),
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
        assert_eq!(decode(b"a\xe9", "latin1").unwrap(), "a\u{e9}");
        assert_eq!(decode(b"\xa4", "latin9").unwrap(), "\u{20ac}");
        assert_eq!(decode(b"\x80", "cp1252").unwrap(), "\u{20ac}");
        assert!(decode(b"\x81", "cp1252").is_err());
        assert!(decode(b"\xe9", "UTF-8").is_err());
        assert!(decode(b"\xe9", "ascii").is_err());
        assert_eq!(decode(b"\xb1", "latin2").unwrap(), "\u{105}");
        assert!(decode(b"x", "nonsense").is_err());
        assert_eq!(decode(b"\x82\xa0", "sjis").unwrap(), "\u{3042}");
        assert_eq!(decode(b"\xff\xfea\x00", "UTF-16").unwrap(), "a");
        assert!(is_utf8_strict_layer("UTF-8") && is_utf8_strict_layer("utf8"));
        assert!(!is_utf8_strict_layer("utf8x"));
    }
}

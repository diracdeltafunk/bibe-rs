//! Strict decoding of data source files in a Perl `Encode` encoding
//! (`File::Slurper::read_text($filename, $encoding)`, Utils.pm:171).

use super::sbcs::TABLES;

/// The canonical form of a Perl encoding name we support, or `None`.
fn canonical(name: &str) -> Option<String> {
    let n = name.trim().to_ascii_lowercase().replace('_', "-");
    let latin = |k: u32| -> Option<&'static str> {
        Some(match k {
            1 => "iso-8859-1",
            2 => "iso-8859-2",
            3 => "iso-8859-3",
            4 => "iso-8859-4",
            5 => "iso-8859-9",
            6 => "iso-8859-10",
            7 => "iso-8859-13",
            8 => "iso-8859-14",
            9 => "iso-8859-15",
            10 => "iso-8859-16",
            _ => return None,
        })
    };
    Some(match n.as_str() {
        "utf8" | "utf-8" | "utf-8-strict" => "utf-8".into(),
        "ascii" | "us-ascii" | "ansi-x3.4-1968" | "iso-646-us" => "ascii".into(),
        "l1" | "cp819" => "iso-8859-1".into(),
        "cyrillic" => "iso-8859-5".into(),
        "arabic" => "iso-8859-6".into(),
        "greek" => "iso-8859-7".into(),
        "hebrew" => "iso-8859-8".into(),
        "thai" => "iso-8859-11".into(),
        "macroman" | "mac-roman" => "MacRoman".into(),
        "koi8-r" | "koi8-u" => n,
        _ => {
            if let Some(k) = n.strip_prefix("latin") {
                return latin(k.parse().ok()?).map(str::to_string);
            }
            for p in ["iso-8859", "iso8859"] {
                if let Some(k) = n.strip_prefix(p) {
                    let k: u32 = k.trim_start_matches('-').parse().ok()?;
                    return Some(format!("iso-8859-{k}"));
                }
            }
            for p in ["windows", "win", "cp"] {
                if let Some(k) = n.strip_prefix(p) {
                    let k: u32 = k.trim_start_matches('-').parse().ok()?;
                    return Some(format!("cp{k}"));
                }
            }
            return None;
        }
    })
}

/// Decode `bytes` in the Perl encoding `name`, failing on any byte sequence
/// that is invalid in it (Perl's strict `:encoding()` read). The error is a
/// short description (biber appends Perl's `$@`).
pub fn decode(bytes: &[u8], name: &str) -> Result<String, String> {
    let Some(enc) = canonical(name) else {
        return Err(format!("Unknown encoding '{name}'"));
    };
    match enc.as_str() {
        "utf-8" => String::from_utf8(bytes.to_vec()).map_err(|e| {
            let b = bytes[e.utf8_error().valid_up_to()];
            format!("utf8 \"\\x{b:02X}\" does not map to Unicode")
        }),
        "ascii" => {
            if let Some(b) = bytes.iter().find(|b| **b >= 0x80) {
                Err(format!("ascii \"\\x{b:02X}\" does not map to Unicode"))
            } else {
                Ok(String::from_utf8(bytes.to_vec()).expect("ASCII"))
            }
        }
        "iso-8859-1" => Ok(bytes.iter().map(|&b| b as char).collect()),
        other => {
            let Some((_, table)) = TABLES.iter().find(|(n, _)| *n == other) else {
                return Err(format!("Unknown encoding '{name}'"));
            };
            let mut s = String::with_capacity(bytes.len());
            for &b in bytes {
                if b < 0x80 {
                    s.push(b as char);
                    continue;
                }
                let u = table[(b - 0x80) as usize];
                match char::from_u32(u32::from(u)) {
                    Some(c) if u != 0xFFFF => s.push(c),
                    _ => return Err(format!("{other} \"\\x{b:02X}\" does not map to Unicode")),
                }
            }
            Ok(s)
        }
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
    }
}

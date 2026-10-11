//! `Encode::Unicode` (Unicode.xs 2.20): UTF-16, UTF-32 and UCS-2, with and
//! without byte order.

use super::xs::utf8_any;

/// One `Encode::Unicode` encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UniEnc {
    pub name: &'static str,
    /// 2 or 4.
    pub size: usize,
    /// `Some(true)` big-endian, `Some(false)` little-endian, `None`: BOM.
    pub be: Option<bool>,
    pub ucs2: bool,
}

pub(crate) const ALL: &[UniEnc] = &[
    UniEnc {
        name: "UCS-2BE",
        size: 2,
        be: Some(true),
        ucs2: true,
    },
    UniEnc {
        name: "UCS-2LE",
        size: 2,
        be: Some(false),
        ucs2: true,
    },
    UniEnc {
        name: "UTF-16",
        size: 2,
        be: None,
        ucs2: false,
    },
    UniEnc {
        name: "UTF-16BE",
        size: 2,
        be: Some(true),
        ucs2: false,
    },
    UniEnc {
        name: "UTF-16LE",
        size: 2,
        be: Some(false),
        ucs2: false,
    },
    UniEnc {
        name: "UTF-32",
        size: 4,
        be: None,
        ucs2: false,
    },
    UniEnc {
        name: "UTF-32BE",
        size: 4,
        be: Some(true),
        ucs2: false,
    },
    UniEnc {
        name: "UTF-32LE",
        size: 4,
        be: Some(false),
        ucs2: false,
    },
];

pub(crate) fn find(name: &str) -> Option<&'static UniEnc> {
    ALL.iter().find(|e| e.name == name)
}

const FBCHAR: u32 = 0xFFFD;

fn pack(out: &mut Vec<u8>, size: usize, be: bool, v: u32) {
    let b = v.to_be_bytes();
    let b = &b[4 - size..];
    if be {
        out.extend_from_slice(b);
    } else {
        out.extend(b.iter().rev());
    }
}

fn unpack(s: &[u8], size: usize, be: bool) -> u32 {
    let mut v = 0u32;
    if be {
        for b in &s[..size] {
            v = (v << 8) | *b as u32;
        }
    } else {
        for b in s[..size].iter().rev() {
            v = (v << 8) | *b as u32;
        }
    }
    v
}

fn is_nonchar(c: u32) -> bool {
    (c & 0xFFFE) == 0xFFFE || (0xFDD0..=0xFDEF).contains(&c)
}

/// State kept by a "renewed" (PerlIO) object: the byte order once known.
#[derive(Default, Clone, Copy)]
pub(crate) struct State {
    pub be: Option<bool>,
}

impl UniEnc {
    /// `encode`: returns the output and the offset of the first unconsumed
    /// byte (a partial UTF-8 character at the end is left).
    pub fn encode(&self, src: &[u8], st: &mut State) -> (Vec<u8>, usize) {
        let mut out = Vec::with_capacity((src.len() + 1) * self.size);
        let be = match self.be.or(st.be) {
            Some(b) => b,
            None => {
                pack(&mut out, self.size, true, 0xFEFF);
                st.be = Some(true);
                true
            }
        };
        let mut s = 0;
        while s < src.len() {
            let skip = match src[s] {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 => 4,
                0xF8..=0xFB => 5,
                0xFC..=0xFD => 6,
                0xFE..=0xFF => 7,
                _ => 1,
            };
            if s + skip > src.len() {
                break;
            }
            let (mut ord, len) = utf8_any(&src[s..]);
            // UTF8_DISALLOW_ILLEGAL_INTERCHANGE: malformations, surrogates
            // and noncharacters come back as 0.
            let malformed = ord == 0xFFFD && !src[s..].starts_with("\u{FFFD}".as_bytes());
            if malformed || is_nonchar(ord) || (0xD800..=0xDFFF).contains(&ord) {
                ord = 0;
            }
            if (self.size != 4 && (ord > 0xFFFF)) || (ord == 0 && src[s] != 0) {
                if self.ucs2 || ord == 0 {
                    pack(&mut out, self.size, be, FBCHAR);
                } else {
                    let hi = ((ord - 0x10000) >> 10) + 0xD800;
                    let lo = ((ord - 0x10000) & 0x3FF) + 0xDC00;
                    pack(&mut out, self.size, be, hi);
                    pack(&mut out, self.size, be, lo);
                }
            } else {
                pack(&mut out, self.size, be, ord);
            }
            s += len;
        }
        (out, s)
    }

    /// `decode` with CHECK = `FB_CROAK|STOP_AT_PARTIAL`, one PerlIO chunk.
    /// Returns the decoded text and the offset of the first unconsumed byte.
    pub fn decode_croak(&self, src: &[u8], st: &mut State) -> Result<(String, usize), String> {
        let size = self.size;
        let mut s = 0;
        let mut be = self.be.or(st.be);
        if be.is_none() && size <= src.len() {
            let bom = unpack(src, size, true);
            s = size;
            be = Some(true);
            if bom != 0xFEFF {
                if bom == 0xFFFE {
                    be = Some(false);
                } else if bom == 0xFFFE_0000 {
                    be = Some(false);
                } else {
                    s = 0;
                }
            }
            st.be = be;
        }
        let be = be.unwrap_or(true);
        let name = self.name;
        let mut out = String::new();
        while s + size <= src.len() {
            let mut ord = unpack(&src[s..], size, be);
            s += size;
            if (0xD800..=0xDFFF).contains(&ord) {
                if self.ucs2 || size == 4 {
                    return Err(format!("{name}:no surrogates allowed {ord:x}"));
                }
                if ord >= 0xDC00 {
                    return Err(format!("{name}:Malformed HI surrogate {ord:x}"));
                }
                if s + size > src.len() {
                    // STOP_AT_PARTIAL: leave it, but the "Partial character"
                    // check below still croaks (Unicode.xs).
                    s -= size;
                    break;
                }
                let lo = unpack(&src[s..], size, be);
                s += size;
                if !(0xDC00..=0xDFFF).contains(&lo) {
                    return Err(format!("{name}:Malformed LO surrogate {ord:x}"));
                }
                ord = 0x10000 + ((ord - 0xD800) << 10) + (lo - 0xDC00);
            }
            if is_nonchar(ord) {
                return Err(format!("{name}:Unicode character {ord:x} is illegal"));
            }
            match char::from_u32(ord) {
                Some(c) => out.push(c),
                None => {
                    return Err(format!(
                        "Use of code point 0x{ord:X} is not allowed; the permissible max is 0x10FFFF"
                    ));
                }
            }
        }
        if s < src.len() {
            return Err(format!("{name}:Partial character"));
        }
        Ok((out, s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes() {
        let e = find("UTF-16").unwrap();
        let (o, n) = e.encode("a\u{1F600}".as_bytes(), &mut State::default());
        assert_eq!(o, b"\xfe\xff\x00a\xd8\x3d\xde\x00");
        assert_eq!(n, 5);
        let e = find("UCS-2BE").unwrap();
        let (o, _) = e.encode("a\u{1F600}\u{FFFF}".as_bytes(), &mut State::default());
        assert_eq!(o, b"\x00a\xff\xfd\xff\xfd");
        let (o, n) = e.encode(b"a\xe3\x81", &mut State::default());
        assert_eq!((o.as_slice(), n), (&b"\x00a"[..], 1));
    }

    #[test]
    fn decodes() {
        let e = find("UTF-16").unwrap();
        let (s, _) = e
            .decode_croak(b"\xff\xfea\x00", &mut State::default())
            .unwrap();
        assert_eq!(s, "a");
        let (s, _) = e.decode_croak(b"\x00a", &mut State::default()).unwrap();
        assert_eq!(s, "a");
        assert!(e.decode_croak(b"\x00a\x00", &mut State::default()).is_err());
    }
}

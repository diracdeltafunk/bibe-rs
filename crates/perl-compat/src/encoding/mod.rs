//! Perl `Encode` (3.21, as bundled with biber 2.22) for the encodings biber
//! users name in `--input-encoding`/`--output-encoding` (and the `.bcf`'s
//! `bibencoding`/`texencoding`): name resolution exactly as
//! `Encode::find_encoding` does it (with biber's own aliases), and the
//! conversions of
//!
//! - every table-driven encoding (`Encode::XS`: single-byte, EBCDIC, the CJK
//!   multi-byte encodings of Encode::CN/JP/KR/TW, Mac encodings, …), by
//!   running Encode's own compiled tables through a port of its engine;
//! - `Encode::Unicode` (UTF-16/32, UCS-2, with and without BOM);
//! - `Encode::utf8` (`utf8`, `utf-8-strict`);
//! - the 7-bit CJK encodings implemented in Perl (`iso-2022-jp`,
//!   `iso-2022-jp-1`, `7bit-jis`, `iso-2022-kr`, `hz`).
//!
//! `UTF-7`, `gsm0338` and the `MIME-*` encodings are known but not supported
//! ([`lookup`] returns [`Lookup::Unsupported`]).
//!
//! Output goes through [`Encoding::encode_layer`], a simulation of a PerlIO
//! `:encoding(…)` layer: Perl encodes the stream in 1024-byte chunks of its
//! internal UTF-8 (BB-152) and line by line for `needs_lines` encodings.
//!
//! ```
//! use perl_compat::encoding::find_encoding;
//! let e = find_encoding("latin1").unwrap();
//! assert_eq!(e.name(), "iso-8859-1");
//! assert!(e.can_encode('é') && !e.can_encode('€'));
//! assert_eq!(e.encode_layer("é€", true), b"\xe9\\x{20ac}".to_vec());
//! assert_eq!(find_encoding("applemacce").unwrap().name(), "MacCentralEurRoman");
//! assert_eq!(find_encoding("Shift_JIS").unwrap().encode_layer("あ", true), b"\x82\xa0");
//! ```

mod alias;
mod iso2022;
mod unicode;
mod xs;

use xs::{Check, Dir};

/// An encoding found by [`find_encoding`].
#[derive(Clone, Copy)]
pub struct Encoding(Inner);

#[derive(Clone, Copy)]
enum Inner {
    /// `utf-8-strict` (`strict`) or `utf8`.
    Utf8 {
        strict: bool,
    },
    Xs(&'static xs::XsEnc),
    Uni(&'static unicode::UniEnc),
    Iso(iso2022::Kind),
}

impl std::fmt::Debug for Encoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Encoding({})", self.name())
    }
}

/// Result of [`lookup`].
#[derive(Debug)]
pub enum Lookup {
    /// A supported encoding.
    Found(Encoding),
    /// Known to `Encode` but not implemented here (canonical name).
    Unsupported(&'static str),
    /// `Encode::find_encoding` returns `undef`.
    Unknown,
}

/// `Encode::find_encoding($name)`.
pub fn lookup(name: &str) -> Lookup {
    let Some(canon) = alias::find(name) else {
        return Lookup::Unknown;
    };
    let inner = match canon {
        "utf8" => Inner::Utf8 { strict: false },
        "utf-8-strict" => Inner::Utf8 { strict: true },
        _ => {
            if let Some(x) = xs::find(canon) {
                Inner::Xs(x)
            } else if let Some(u) = unicode::find(canon) {
                Inner::Uni(u)
            } else if let Some(k) = iso2022::find(canon) {
                Inner::Iso(k)
            } else {
                return Lookup::Unsupported(canon);
            }
        }
    };
    Lookup::Found(Encoding(inner))
}

/// `Encode::find_encoding($name)`, restricted to the supported encodings.
pub fn find_encoding(name: &str) -> Option<Encoding> {
    match lookup(name) {
        Lookup::Found(e) => Some(e),
        _ => None,
    }
}

/// Canonical names of all encodings `Encode` knows in biber 2.22.
pub fn known_names() -> &'static [&'static str] {
    alias::KNOWN
}

/// Per-layer state of a "renewed" encoding object (`Encode::Unicode`
/// remembers the byte order once it has written or read the BOM).
#[derive(Default)]
struct LayerState {
    uni: unicode::State,
}

impl Encoding {
    /// UTF-8 (`utf-8-strict`, what `UTF-8` resolves to).
    pub const UTF8: Encoding = Encoding(Inner::Utf8 { strict: true });

    /// The canonical Encode name (`$enc->name`).
    pub fn name(&self) -> &'static str {
        match self.0 {
            Inner::Utf8 { strict: true } => "utf-8-strict",
            Inner::Utf8 { strict: false } => "utf8",
            Inner::Xs(x) => {
                // 'static: the tables live in a static OnceLock.
                x.name.as_str()
            }
            Inner::Uni(u) => u.name,
            Inner::Iso(k) => match k {
                iso2022::Kind::Jis7Bit => "7bit-jis",
                iso2022::Kind::Iso2022Jp => "iso-2022-jp",
                iso2022::Kind::Iso2022Jp1 => "iso-2022-jp-1",
                iso2022::Kind::Iso2022Kr => "iso-2022-kr",
                iso2022::Kind::Hz => "hz",
            },
        }
    }

    /// Is this UTF-8 (strict or lax)?
    pub fn is_utf8(&self) -> bool {
        matches!(self.0, Inner::Utf8 { .. })
    }

    /// `$enc->needs_lines`: a PerlIO layer flushes after every newline.
    pub fn needs_lines(&self) -> bool {
        matches!(self.0, Inner::Iso(_))
    }

    /// `encode($utf8, $chk)` on one PerlIO buffer: the output and the offset
    /// of the first byte left in the source (carried into the next buffer).
    fn encode_chunk(&self, buf: &[u8], chk: Check<'_>, st: &mut LayerState) -> (Vec<u8>, usize) {
        match self.0 {
            Inner::Utf8 { strict } => utf8_encode(buf, strict, chk),
            Inner::Xs(x) => xs::encode_method(x, Dir::Encode, buf, chk).unwrap_or_default(),
            Inner::Uni(u) => u.encode(buf, &mut st.uni),
            Inner::Iso(k) => k.encode(buf, chk),
        }
    }

    /// The bytes a PerlIO `:encoding(<self>)` output layer writes for `s`
    /// (`$PerlIO::encoding::fallback`: unmappable characters become
    /// `\x{XXXX}`). With `chunked`, Perl's 1024-byte buffering is reproduced
    /// (BB-152: a character cut by a buffer boundary may become
    /// `\x{fffd}`s); without, each character is encoded whole.
    pub fn encode_layer(&self, s: &str, chunked: bool) -> Vec<u8> {
        const BUFSIZ: usize = 1024; // PerlIOEncode_get_base
        struct Layer<'a> {
            enc: &'a Encoding,
            st: LayerState,
            buf: Vec<u8>,
            out: Vec<u8>,
            bufsiz: usize,
        }
        impl Layer<'_> {
            /// `PerlIOEncode_flush` (write case).
            fn flush(&mut self) {
                if !self.buf.is_empty() {
                    let (o, rest) = self
                        .enc
                        .encode_chunk(&self.buf, Check::PERLIO, &mut self.st);
                    self.out.extend(o);
                    let rest = rest.min(self.buf.len());
                    self.buf.drain(..rest);
                }
            }
            /// `PerlIOBuf_write`.
            fn write(&mut self, mut p: &[u8]) {
                while !p.is_empty() {
                    let n = (self.bufsiz - self.buf.len()).min(p.len());
                    self.buf.extend_from_slice(&p[..n]);
                    p = &p[n..];
                    if self.buf.len() >= self.bufsiz {
                        self.flush();
                    }
                }
            }
        }
        let mut l = Layer {
            enc: self,
            st: LayerState::default(),
            buf: Vec::with_capacity(BUFSIZ),
            out: Vec::with_capacity(s.len()),
            bufsiz: if chunked { BUFSIZ } else { usize::MAX },
        };
        if self.needs_lines() {
            // `PerlIOEncode_write` with NEEDS_LINES: flush after each newline.
            for piece in s.as_bytes().split_inclusive(|b| *b == b'\n') {
                l.write(piece);
                if piece.ends_with(b"\n") {
                    l.flush();
                }
            }
        } else {
            l.write(s.as_bytes());
        }
        l.flush(); // close
        l.out
    }

    /// `encode($enc, $s, sub {"\0"})`: the encoding of the complete string
    /// `s` with unmappable characters replaced by NUL (biber's
    /// "can this entry be encoded" test, bbl.pm:741).
    pub fn encode_nul_fallback(&self, s: &str) -> Vec<u8> {
        self.encode_cb(s, &|_| vec![0u8])
    }

    /// `encode($enc, $s, $coderef)`: unmappable characters are replaced by
    /// `cb(code point)` (where the encoding supports a code-reference CHECK).
    pub fn encode_cb(&self, s: &str, cb: &dyn Fn(u32) -> Vec<u8>) -> Vec<u8> {
        let chk = Check {
            die: false,
            perlqq: true,
            nonzero: true,
            cb: Some(cb),
        };
        let mut st = LayerState::default();
        self.encode_chunk(s.as_bytes(), chk, &mut st).0
    }

    /// `encode($enc, $s, sub {"\0"}) =~ /\0/`. For UTF-16/32 this is true for
    /// almost any string (biber's test misfires; reproduced).
    pub fn has_unencodable(&self, s: &str) -> bool {
        self.encode_nul_fallback(s).contains(&0)
    }

    /// Can `c` be encoded (on its own)?
    pub fn can_encode(&self, c: char) -> bool {
        let mut b = [0u8; 4];
        let s = c.encode_utf8(&mut b);
        c == '\0' || !self.encode_nul_fallback(s).contains(&0)
    }

    /// Read `bytes` through a `:raw:encoding(<self>)` layer with
    /// `STOP_AT_PARTIAL|FB_CROAK` (`File::Slurper::read_text`). The error is
    /// Encode's croak message (without perl's ` at FILE line N.`).
    pub fn decode_layer(&self, bytes: &[u8]) -> Result<String, String> {
        match self.0 {
            Inner::Utf8 { strict } => utf8_decode(bytes, strict),
            Inner::Xs(x) => {
                // A partial character at the very end is silently dropped
                // (PerlIOEncode_fill hits EOF with it still pending).
                let (o, _) = xs::encode_method(x, Dir::Decode, bytes, Check::CROAK)?;
                Ok(String::from_utf8(o).expect("Encode output is UTF-8"))
            }
            Inner::Uni(u) => {
                let mut st = unicode::State::default();
                let mut out = String::new();
                for chunk in bytes.chunks(1024) {
                    let (s, _) = u.decode_croak(chunk, &mut st)?;
                    out.push_str(&s);
                }
                Ok(out)
            }
            Inner::Iso(k) => {
                let mut out = String::new();
                for line in bytes.split_inclusive(|b| *b == b'\n') {
                    out.push_str(&k.decode_line(line)?);
                }
                Ok(out)
            }
        }
    }

    /// Decode leniently (`decode($enc, $bytes)`, CHECK 0 for table
    /// encodings: undefined bytes become U+FFFD). For tests.
    pub fn decode_lossy(&self, bytes: &[u8]) -> String {
        match self.0 {
            Inner::Xs(x) => {
                let (o, _) =
                    xs::encode_method(x, Dir::Decode, bytes, Check::DEFAULT).unwrap_or_default();
                String::from_utf8_lossy(&o).into_owned()
            }
            _ => self
                .decode_layer(bytes)
                .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned()),
        }
    }
}

/// `Encode::utf8::decode` through a PerlIO layer (`process_utf8` with
/// `FB_CROAK|STOP_AT_PARTIAL`): a partial character at the end is dropped;
/// strict UTF-8 rejects surrogates, noncharacters and code points above
/// U+10FFFF. Lax `utf8` accepts those, but Rust strings cannot hold
/// surrogates or values above U+10FFFF: they become U+FFFD (as DC-019).
fn utf8_decode(bytes: &[u8], strict: bool) -> Result<String, String> {
    let name = if strict { "UTF-8" } else { "utf8" };
    let err = |s: &[u8]| {
        let esc: String = s.iter().map(|b| format!("\\x{b:02X}")).collect();
        format!("{name} \"{esc}\" does not map to Unicode")
    };
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            out.push(b as char);
            i += 1;
            continue;
        }
        let need = match b {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            0x80..=0xBF => return Err(err(&bytes[i..i + 1])),
            _ => {
                // perl's extended UTF-8 (5 to 13 bytes): never acceptable.
                let n = match b {
                    0xF8..=0xFB => 5,
                    0xFC..=0xFD => 6,
                    0xFE => 7,
                    _ => 13,
                };
                let cont = bytes[i + 1..]
                    .iter()
                    .take(n - 1)
                    .take_while(|c| (0x80..=0xBF).contains(*c))
                    .count();
                return Err(err(&bytes[i..i + 1 + cont]));
            }
        };
        let cont = bytes[i + 1..]
            .iter()
            .take(need - 1)
            .take_while(|c| (0x80..=0xBF).contains(*c))
            .count();
        if cont < need - 1 {
            if i + 1 + cont == bytes.len() && valid_partial(&bytes[i..], need, strict) {
                break; // partial character at EOF: dropped
            }
            return Err(err(&bytes[i..i + 1 + cont]));
        }
        let seq = &bytes[i..i + need];
        let mut v = (b as u32) & (0x7F >> need);
        for c in &seq[1..] {
            v = (v << 6) | (*c as u32 & 0x3F);
        }
        let min = [0, 0, 0x80, 0x800, 0x10000][need];
        if v < min {
            return Err(err(seq)); // overlong
        }
        let surrogate = (0xD800..=0xDFFF).contains(&v);
        let nonchar = (v & 0xFFFE) == 0xFFFE || (0xFDD0..=0xFDEF).contains(&v);
        if strict && (surrogate || nonchar || v > 0x10FFFF) {
            return Err(err(seq));
        }
        out.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
        i += need;
    }
    Ok(out)
}

/// `is_utf8_valid_partial_char_flags`: can the incomplete sequence `p`
/// (lead byte expecting `need` bytes) still become an acceptable character?
fn valid_partial(p: &[u8], need: usize, strict: bool) -> bool {
    let mut lo = (p[0] as u32) & (0x7F >> need);
    for c in &p[1..] {
        lo = (lo << 6) | (*c as u32 & 0x3F);
    }
    let missing = need - p.len();
    let hi = (lo << (6 * missing)) | ((1 << (6 * missing)) - 1);
    let lo = lo << (6 * missing);
    let min = [0, 0, 0x80, 0x800, 0x10000][need];
    let lo = lo.max(min);
    if lo > hi {
        return false;
    }
    if !strict {
        return true;
    }
    // Some value in [lo, hi] that is neither a surrogate, a noncharacter
    // nor above U+10FFFF.
    let hi = hi.min(0x10FFFF);
    let mut v = lo;
    while v <= hi {
        if (0xD800..=0xDFFF).contains(&v) {
            v = 0xE000;
            continue;
        }
        if (0xFDD0..=0xFDEF).contains(&v) {
            v = 0xFDF0;
            continue;
        }
        if (v & 0xFFFE) == 0xFFFE {
            v += 1;
            continue;
        }
        return true;
    }
    false
}

/// `Encode::utf8::encode` (Encode.xs:677, `process_utf8`): strict UTF-8
/// replaces noncharacters (`\x{FDD0}`); a partial character at the end is
/// left in the source (`STOP_AT_PARTIAL`); lax `utf8` copies everything.
fn utf8_encode(buf: &[u8], strict: bool, chk: Check<'_>) -> (Vec<u8>, usize) {
    if !strict {
        return (buf.to_vec(), buf.len());
    }
    let mut out = Vec::with_capacity(buf.len());
    let mut i = 0;
    while i < buf.len() {
        let (c, n) = xs::utf8_any(&buf[i..]);
        let need = match buf[i] {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        if n < need && i + n == buf.len() {
            break; // valid partial character
        }
        let nonchar = (c & 0xFFFE) == 0xFFFE || (0xFDD0..=0xFDEF).contains(&c);
        let malformed = n < need || (c == 0xFFFD && !buf[i..].starts_with("\u{FFFD}".as_bytes()));
        if nonchar || malformed {
            match chk.cb {
                Some(cb) => out.extend(cb(c)),
                None if n == 1 && malformed => out.extend(format!("\\x{:02X}", buf[i]).bytes()),
                None => out.extend(format!("\\x{{{c:04X}}}").bytes()),
            }
        } else {
            out.extend_from_slice(&buf[i..i + n]);
        }
        i += n;
    }
    (out, i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected outputs recorded from perl 5.42 (O3): `print` of `"a" x
    /// (1024 - split) . $ch . "Z"` to a `:encoding($enc)` file handle.
    #[test]
    fn perlio_layer_buffer_splits() {
        let tail = |enc: &str, ch: &str, split: usize| -> String {
            let e = find_encoding(enc).unwrap();
            let s = format!("{}{ch}Z", "a".repeat(1024 - split));
            let out = e.encode_layer(&s, true);
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
        // E2 has an entry in cp1252's root page, so E2 81 is carried even
        // though no cp1252 character starts with E2 81.
        assert_eq!(tail("cp1252", "\u{2041}", 2), "\\x{2041}Z");
        assert_eq!(tail("cp1252", "\u{3042}", 2), f(2));
        assert_eq!(tail("shiftjis", "\u{3042}", 2), "\u{FFFD}\u{FFFD}Z"); // 82 A0
        assert_eq!(tail("iso-2022-jp", "\u{3042}", 2), f(1));
        assert_eq!(tail("hz", "\u{4E00}", 2), "~{R;~}Z");
        // A carried fragment shifts the next boundary (1023 + 1024).
        let e = find_encoding("latin1").unwrap();
        for (pos, want) in [(2046, "\\x{fffd}\\x{fffd}Z"), (2047, "\\x{03bb}Z")] {
            let s = format!("{}é{}λZ", "a".repeat(1023), "b".repeat(pos - 1025));
            let out = e.encode_layer(&s, true);
            assert_eq!(String::from_utf8_lossy(&out[pos - 1..]), want, "λ at {pos}");
        }
        // Short strings and UTF-8 are unaffected.
        assert_eq!(e.encode_layer("é€", true), e.encode_layer("é€", false));
        assert_eq!(Encoding::UTF8.encode_layer("λ", true), "λ".as_bytes());
    }

    #[test]
    fn layer_specials() {
        // Recorded from O3.
        let e = find_encoding("UTF-8").unwrap();
        assert_eq!(
            e.encode_layer("a\u{FDD0}\u{FFFF}b", true),
            b"a\\x{FDD0}\\x{FFFF}b"
        );
        let e = find_encoding("UTF-16").unwrap();
        let s = format!("{}\u{3042}z", "a".repeat(1022));
        let out = e.encode_layer(&s, true);
        assert_eq!(out.len(), 2050);
        assert_eq!(&out[2046..], b"\x30\x42\x00z");
        let e = find_encoding("iso-2022-kr").unwrap();
        assert_eq!(e.encode_layer("a\nb", true), b"\x1b$)Ca\n\x1b$)Cb");
    }

    #[test]
    fn nul_check() {
        assert!(
            find_encoding("cp1252")
                .unwrap()
                .has_unencodable("a\u{2041}")
        );
        assert!(!find_encoding("cp1252").unwrap().has_unencodable("a€"));
        assert!(find_encoding("UTF-16").unwrap().has_unencodable("a"));
        assert!(
            !find_encoding("iso-2022-kr")
                .unwrap()
                .has_unencodable("a\u{3042}\u{2041}")
        );
        assert!(
            find_encoding("iso-2022-jp")
                .unwrap()
                .has_unencodable("\u{2041}")
        );
    }

    #[test]
    fn round_trips() {
        let e = find_encoding("cp1252").unwrap();
        assert_eq!(
            e.decode_layer(&e.encode_layer("Œuvre €5", true)).unwrap(),
            "Œuvre €5"
        );
        let e = find_encoding("latin9").unwrap();
        assert_eq!(e.name(), "iso-8859-15");
        assert!(e.can_encode('€'));
        assert!(find_encoding("UTF-8").is_some());
        assert!(matches!(lookup("UTF-7"), Lookup::Unsupported("UTF-7")));
        assert!(matches!(lookup("gb18030"), Lookup::Unknown));
    }
}

//! The Perl-implemented 7-bit CJK encodings: `Encode::JP::JIS7`
//! (`7bit-jis`, `iso-2022-jp`, `iso-2022-jp-1`, with `Encode::JP::H2Z`),
//! `Encode::KR::2022_KR` (`iso-2022-kr`) and `Encode::CN::HZ` (`hz`). All
//! three have `needs_lines`, so a PerlIO layer calls them line by line.

use super::xs::{self, Check, Dir};

/// Which encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `7bit-jis` (no h2z, JIS X 0212 allowed).
    Jis7Bit,
    /// `iso-2022-jp` (h2z, no JIS X 0212).
    Iso2022Jp,
    /// `iso-2022-jp-1` (h2z, JIS X 0212).
    Iso2022Jp1,
    Iso2022Kr,
    Hz,
}

pub(crate) fn find(name: &str) -> Option<Kind> {
    Some(match name {
        "7bit-jis" => Kind::Jis7Bit,
        "iso-2022-jp" => Kind::Iso2022Jp,
        "iso-2022-jp-1" => Kind::Iso2022Jp1,
        "iso-2022-kr" => Kind::Iso2022Kr,
        "hz" => Kind::Hz,
        _ => return None,
    })
}

const ESC_ASC: &[u8] = b"\x1b(B";
const ESC_KANA: &[u8] = b"\x1b(I";
const ESC_0208: &[u8] = b"\x1b$B";
const ESC_0212: &[u8] = b"\x1b$(D";
const ESC_KR: &[u8] = b"\x1b$)C";

fn xs(name: &str) -> &'static xs::XsEnc {
    xs::find(name).expect("table encoding")
}

impl Kind {
    /// `encode($utf8, $chk)` on one chunk. Returns the output and the
    /// offset of the first unconsumed input byte (what the encoding leaves
    /// in `$_[1]`).
    pub fn encode(self, src: &[u8], chk: Check<'_>) -> (Vec<u8>, usize) {
        match self {
            Kind::Jis7Bit | Kind::Iso2022Jp | Kind::Iso2022Jp1 => {
                // `$_[1] = '' if $chk`: a partial character is dropped.
                let (mut oct, _) =
                    xs::encode_method(xs("euc-jp"), Dir::Encode, src, chk).unwrap_or_default();
                if self != Kind::Jis7Bit {
                    oct = h2z(&oct);
                }
                (euc_jis(&oct, self != Kind::Iso2022Jp), src.len())
            }
            Kind::Iso2022Kr => {
                let (oct, _) = xs::encode_method(xs("euc-kr"), Dir::Encode, src, Check::FB_PERLQQ)
                    .unwrap_or_default();
                (euc_iso(&oct), src.len())
            }
            Kind::Hz => hz_encode(src, chk),
        }
    }

    /// `decode($octets, $chk)` on one line with CHECK = `FB_CROAK`.
    pub fn decode_line(self, line: &[u8]) -> Result<String, String> {
        match self {
            Kind::Jis7Bit | Kind::Iso2022Jp | Kind::Iso2022Jp1 => {
                // Residue: from the first non-ASCII byte on (dropped).
                let end = line.iter().position(|b| *b >= 0x80).unwrap_or(line.len());
                let mut s = jis_euc(&line[..end]);
                if let Some(p) = s.iter().position(|b| *b == 0x1b) {
                    s.truncate(p);
                }
                let (o, _) = xs::encode_method(xs("euc-jp"), Dir::Decode, &s, Check::FB_PERLQQ)?;
                Ok(String::from_utf8(o).expect("utf8"))
            }
            Kind::Iso2022Kr => {
                let mut s = iso_euc(line);
                if let Some(p) = s.iter().position(|b| *b == 0x1b) {
                    s.truncate(p);
                }
                let (o, _) = xs::encode_method(xs("euc-kr"), Dir::Decode, &s, Check::FB_PERLQQ)?;
                Ok(String::from_utf8(o).expect("utf8"))
            }
            Kind::Hz => hz_decode(line),
        }
    }
}

fn is_euc(b: u8) -> bool {
    (0xa1..=0xfe).contains(&b)
}

/// `Encode::JP::H2Z::h2z` (half-width katakana → full-width, EUC-JP).
fn h2z(s: &[u8]) -> Vec<u8> {
    const H2Z: [[u8; 2]; 63] = [
        [0xa1, 0xa3],
        [0xa1, 0xd6],
        [0xa1, 0xd7],
        [0xa1, 0xa2],
        [0xa1, 0xa6],
        [0xa5, 0xf2],
        [0xa5, 0xa1],
        [0xa5, 0xa3],
        [0xa5, 0xa5],
        [0xa5, 0xa7],
        [0xa5, 0xa9],
        [0xa5, 0xe3],
        [0xa5, 0xe5],
        [0xa5, 0xe7],
        [0xa5, 0xc3],
        [0xa1, 0xbc],
        [0xa5, 0xa2],
        [0xa5, 0xa4],
        [0xa5, 0xa6],
        [0xa5, 0xa8],
        [0xa5, 0xaa],
        [0xa5, 0xab],
        [0xa5, 0xad],
        [0xa5, 0xaf],
        [0xa5, 0xb1],
        [0xa5, 0xb3],
        [0xa5, 0xb5],
        [0xa5, 0xb7],
        [0xa5, 0xb9],
        [0xa5, 0xbb],
        [0xa5, 0xbd],
        [0xa5, 0xbf],
        [0xa5, 0xc1],
        [0xa5, 0xc4],
        [0xa5, 0xc6],
        [0xa5, 0xc8],
        [0xa5, 0xca],
        [0xa5, 0xcb],
        [0xa5, 0xcc],
        [0xa5, 0xcd],
        [0xa5, 0xce],
        [0xa5, 0xcf],
        [0xa5, 0xd2],
        [0xa5, 0xd5],
        [0xa5, 0xd8],
        [0xa5, 0xdb],
        [0xa5, 0xde],
        [0xa5, 0xdf],
        [0xa5, 0xe0],
        [0xa5, 0xe1],
        [0xa5, 0xe2],
        [0xa5, 0xe4],
        [0xa5, 0xe6],
        [0xa5, 0xe8],
        [0xa5, 0xe9],
        [0xa5, 0xea],
        [0xa5, 0xeb],
        [0xa5, 0xec],
        [0xa5, 0xed],
        [0xa5, 0xef],
        [0xa5, 0xf3],
        [0xa1, 0xab],
        [0xa1, 0xac],
    ];
    // %_D2Z: kana + (semi-)voiced mark.
    const D2Z: [(u8, u8, [u8; 2]); 26] = [
        (0xb6, 0xde, [0xa5, 0xac]),
        (0xb7, 0xde, [0xa5, 0xae]),
        (0xb8, 0xde, [0xa5, 0xb0]),
        (0xb9, 0xde, [0xa5, 0xb2]),
        (0xba, 0xde, [0xa5, 0xb4]),
        (0xbb, 0xde, [0xa5, 0xb6]),
        (0xbc, 0xde, [0xa5, 0xb8]),
        (0xbd, 0xde, [0xa5, 0xba]),
        (0xbe, 0xde, [0xa5, 0xbc]),
        (0xbf, 0xde, [0xa5, 0xbe]),
        (0xc0, 0xde, [0xa5, 0xc0]),
        (0xc1, 0xde, [0xa5, 0xc2]),
        (0xc2, 0xde, [0xa5, 0xc5]),
        (0xc3, 0xde, [0xa5, 0xc7]),
        (0xc4, 0xde, [0xa5, 0xc9]),
        (0xca, 0xde, [0xa5, 0xd0]),
        (0xcb, 0xde, [0xa5, 0xd3]),
        (0xcc, 0xde, [0xa5, 0xd6]),
        (0xcd, 0xde, [0xa5, 0xd9]),
        (0xce, 0xde, [0xa5, 0xdc]),
        (0xca, 0xdf, [0xa5, 0xd1]),
        (0xcb, 0xdf, [0xa5, 0xd4]),
        (0xcc, 0xdf, [0xa5, 0xd7]),
        (0xcd, 0xdf, [0xa5, 0xda]),
        (0xce, 0xdf, [0xa5, 0xdd]),
        (0xb3, 0xde, [0xa5, 0xf4]),
    ];
    let kana = |b: u8| (0xa1..=0xdf).contains(&b);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        // s/(\x8e[\xa1-\xdf](?:\x8e[\xde\xdf])?)/.../g
        if s[i] == 0x8e && i + 1 < s.len() && kana(s[i + 1]) {
            let k = s[i + 1];
            let mark = (i + 3 < s.len() && s[i + 2] == 0x8e && matches!(s[i + 3], 0xde | 0xdf))
                .then(|| s[i + 3]);
            if let Some(m) = mark {
                if let Some((_, _, z)) = D2Z.iter().find(|(a, b, _)| *a == k && *b == m) {
                    out.extend_from_slice(z);
                } else {
                    out.extend_from_slice(&H2Z[(k - 0xa1) as usize]);
                    out.extend_from_slice(&H2Z[(m - 0xa1) as usize]);
                }
                i += 4;
            } else {
                out.extend_from_slice(&H2Z[(k - 0xa1) as usize]);
                i += 2;
            }
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// `Encode::JP::JIS7::euc_jis`.
fn euc_jis(s: &[u8], jis0212: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 8);
    let mut i = 0;
    let run = |i: usize, unit: &dyn Fn(usize) -> usize| -> usize {
        let mut j = i;
        loop {
            let n = unit(j);
            if n == 0 {
                break;
            }
            j += n;
        }
        j
    };
    while i < s.len() {
        // ((?:EUC_C)+|(?:EUC_KANA)+|(?:EUC_0212)+), tried in this order.
        let c = run(i, &|j| {
            if j + 1 < s.len() && is_euc(s[j]) && is_euc(s[j + 1]) {
                2
            } else {
                0
            }
        });
        let (end, esc) = if c > i {
            (c, ESC_0208)
        } else {
            let k = run(i, &|j| {
                if j + 1 < s.len() && s[j] == 0x8e && (0xa1..=0xdf).contains(&s[j + 1]) {
                    2
                } else {
                    0
                }
            });
            if k > i {
                (k, ESC_KANA)
            } else {
                let z = run(i, &|j| {
                    if j + 2 < s.len() && s[j] == 0x8f && is_euc(s[j + 1]) && is_euc(s[j + 2]) {
                        3
                    } else {
                        0
                    }
                });
                if z > i {
                    (z, ESC_0212)
                } else {
                    out.push(s[i]);
                    i += 1;
                    continue;
                }
            }
        };
        out.extend_from_slice(esc);
        for &b in &s[i..end] {
            if b == 0x8e || b == 0x8f {
                continue;
            }
            if esc == ESC_0212 && !jis0212 {
                out.push(b'?');
            } else {
                out.push(b - 0x80);
            }
        }
        out.extend_from_slice(ESC_ASC);
        i = end;
    }
    // s/ESC_ASC(ESC_KANA|ESC_0212|ESC_0208)/$1/g
    let mut res = Vec::with_capacity(out.len());
    let mut i = 0;
    while i < out.len() {
        if out[i..].starts_with(ESC_ASC)
            && [ESC_KANA, ESC_0212, ESC_0208]
                .iter()
                .any(|e| out[i + ESC_ASC.len()..].starts_with(e))
        {
            i += ESC_ASC.len();
            continue;
        }
        res.push(out[i]);
        i += 1;
    }
    res
}

/// `Encode::JP::JIS7::jis_euc` (without the residue, handled by the caller).
fn jis_euc(s: &[u8]) -> Vec<u8> {
    // s/(?:(JIS_0212)|JIS_0208|(ISO_ASC)|(JIS_KANA))([^\e]*)/.../g
    let escs: [(&[u8], u8); 7] = [
        (b"\x1b$(D", 1),
        (b"\x1b$@", 0),
        (b"\x1b$B", 0),
        (b"\x1b&@\x1b$B", 0),
        (b"\x1b(B", 2),
        (b"\x1b(J", 2),
        (b"\x1b(I", 3),
    ];
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let hit = escs.iter().find(|(e, _)| s[i..].starts_with(e));
        let Some((e, kind)) = hit else {
            out.push(s[i]);
            i += 1;
            continue;
        };
        i += e.len();
        let end = s[i..]
            .iter()
            .position(|b| *b == 0x1b)
            .map_or(s.len(), |p| i + p);
        for &b in &s[i..end] {
            if *kind == 2 {
                out.push(b);
                continue;
            }
            let b2 = if (0x21..=0x7e).contains(&b) {
                b + 0x80
            } else {
                b
            };
            match kind {
                3 if (0xa1..=0xdf).contains(&b2) => out.extend([0x8e, b2]),
                _ => out.push(b2),
            }
        }
        if *kind == 1 {
            // s/([\xa1-\xfe][\xa1-\xfe])/\x8f$1/g over the converted chunk
            let start = out.len() - (end - i);
            let chunk = out.split_off(start);
            let mut j = 0;
            while j < chunk.len() {
                if j + 1 < chunk.len() && is_euc(chunk[j]) && is_euc(chunk[j + 1]) {
                    out.extend([0x8f, chunk[j], chunk[j + 1]]);
                    j += 2;
                } else {
                    out.push(chunk[j]);
                    j += 1;
                }
            }
        }
        i = end;
    }
    out
}

/// `Encode::KR::2022_KR::euc_iso`.
fn euc_iso(s: &[u8]) -> Vec<u8> {
    let mut out = ESC_KR.to_vec();
    let mut i = 0;
    while i < s.len() {
        let mut j = i;
        while j + 1 < s.len() && is_euc(s[j]) && is_euc(s[j + 1]) {
            j += 2;
        }
        if j > i {
            out.push(0x0e);
            out.extend(s[i..j].iter().map(|b| b - 0x80));
            out.push(0x0f);
            i = j;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// `Encode::KR::2022_KR::iso_euc` (without the residue).
fn iso_euc(s: &[u8]) -> Vec<u8> {
    let mut t = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with(ESC_KR) {
            i += ESC_KR.len();
        } else {
            t.push(s[i]);
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        if t[i] == 0x0e
            && let Some(p) = t[i + 1..].iter().position(|b| *b == 0x0f)
        {
            for &b in &t[i + 1..i + 1 + p] {
                out.push(if (0x21..=0x7e).contains(&b) {
                    b + 0x80
                } else {
                    b
                });
            }
            i += p + 2;
        } else {
            out.push(t[i]);
            i += 1;
        }
    }
    out
}

/// `Encode::CN::HZ::encode`.
fn hz_encode(src: &[u8], chk: Check<'_>) -> (Vec<u8>, usize) {
    let gb = xs("gb2312-raw");
    let mut out = Vec::new();
    let mut ascii = true;
    let mut i = 0;
    while i < src.len() {
        if src[i] < 0x80 {
            let j = src[i..]
                .iter()
                .position(|b| *b >= 0x80)
                .map_or(src.len(), |p| i + p);
            if !ascii {
                out.extend_from_slice(b"~}");
                ascii = true;
            }
            for &b in &src[i..j] {
                if b == b'~' {
                    out.extend_from_slice(b"~~");
                } else {
                    out.push(b);
                }
            }
            i = j;
            continue;
        }
        let need = match src[i] {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            _ => 4,
        };
        if i + need > src.len() {
            break; // malformed (partial) UTF-8: left in $_[1]
        }
        let (tmp, _) =
            xs::encode_method(gb, Dir::Encode, &src[i..i + need], chk).unwrap_or_default();
        if tmp.len() == 2 {
            if ascii {
                out.extend_from_slice(b"~{");
                ascii = false;
            }
            out.extend_from_slice(&tmp);
        } else if !tmp.is_empty() {
            if !ascii {
                out.extend_from_slice(b"~}");
                ascii = true;
            }
            out.extend_from_slice(&tmp);
        }
        i += need;
    }
    if !ascii {
        out.extend_from_slice(b"~}");
    }
    (out, i)
}

/// `Encode::CN::HZ::decode` with CHECK = `FB_CROAK`.
fn hz_decode(s: &[u8]) -> Result<String, String> {
    let gb = xs("gb2312-raw");
    let mut ret = String::new();
    let mut ascii = true;
    let mut i = 0;
    while i < s.len() {
        if ascii {
            let j = s[i..]
                .iter()
                .position(|b| !(*b <= 0x7d || *b == 0x7f))
                .map_or(s.len(), |p| i + p);
            if j > i {
                ret.extend(s[i..j].iter().map(|b| *b as char));
                i = j;
            } else if s[i..].starts_with(b"~~") {
                ret.push('~');
                i += 2;
            } else if s[i..].starts_with(b"~\n") {
                i += 2;
            } else if s[i..].starts_with(b"~{") {
                ascii = false;
                i += 2;
            } else {
                break;
            }
        } else {
            let mut j = i;
            while j + 1 < s.len()
                && (0x21..=0x77).contains(&s[j])
                && (0x21..=0x7e).contains(&s[j + 1])
            {
                j += 2;
            }
            if j > i {
                let (o, _) = xs::encode_method(gb, Dir::Decode, &s[i..j], Check::CROAK)?;
                ret.push_str(std::str::from_utf8(&o).expect("utf8"));
                i = j;
            } else if s[i..].starts_with(b"~}") {
                ascii = true;
                i += 2;
            } else {
                break;
            }
        }
    }
    Ok(ret)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(k: Kind, s: &str) -> Vec<u8> {
        k.encode(s.as_bytes(), Check::PERLIO).0
    }

    #[test]
    fn matches_perl() {
        // Recorded from perl 5.42 / Encode 3.21 (O3).
        assert_eq!(
            enc(Kind::Iso2022Jp, "a\u{3042}\u{FF71}\u{4E02}b"),
            b"a\x1b$B$\"%\"\x1b$(D??\x1b(Bb"
        );
        assert_eq!(
            enc(Kind::Jis7Bit, "\u{FF71}\u{3042}"),
            b"\x1b(I1\x1b$B$\"\x1b(B"
        );
        assert_eq!(enc(Kind::Iso2022Jp1, "\u{4E02}"), b"\x1b$(D0!\x1b(B");
        assert_eq!(
            enc(Kind::Iso2022Kr, "a\u{AC00}b\u{3042}"),
            b"\x1b$)Ca\x0e0!\x0fb\x0e*\"\x0f"
        );
        assert_eq!(enc(Kind::Hz, "a\u{4E00}b\u{3042}"), b"a~{R;~}b~{$\"~}");
    }

    #[test]
    fn decodes() {
        assert_eq!(
            Kind::Iso2022Jp
                .decode_line(b"a\x1b$B$\"%\"\x1b(Bb")
                .unwrap(),
            "a\u{3042}\u{30A2}b"
        );
        assert_eq!(Kind::Hz.decode_line(b"a~{R;~}b").unwrap(), "a\u{4E00}b");
        assert_eq!(
            Kind::Iso2022Kr.decode_line(b"\x1b$)Ca\x0e0!\x0fb").unwrap(),
            "a\u{AC00}b"
        );
    }
}

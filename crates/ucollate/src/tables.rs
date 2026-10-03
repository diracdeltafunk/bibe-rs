//! The compiled DUCET 13.0.0 and the derived (implicit) collation elements of UCA 43.

/// A collation element: four weights plus the "variable" flag (`*` in allkeys syntax).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub(crate) struct Ce {
    pub p: u16,
    pub s: u16,
    pub t: u16,
    pub q: u16,
    pub var: bool,
}

impl Ce {
    pub(crate) const fn new(p: u16, s: u16, t: u16, q: u16, var: bool) -> Ce {
        Ce { p, s, t, q, var }
    }
}

include!(concat!(env!("OUT_DIR"), "/ducet.rs"));

const PRESENT: u32 = 0x8000_0000;

/// Raw DUCET table entry for a single code point (see build.rs for the encoding).
#[inline]
fn stage(cp: u32) -> u32 {
    if cp >= 0x110000 {
        return 0;
    }
    let blk = DUCET_STAGE1[(cp >> 7) as usize] as usize;
    DUCET_STAGE2[(blk << 7) | (cp as usize & 0x7F)]
}

#[inline]
fn slice(v: u32) -> &'static [Ce] {
    let start = (v & 0xF_FFFF) as usize;
    let len = ((v >> 20) & 0xFF) as usize;
    &DUCET_CES[start..start + len]
}

/// The raw DUCET entry of a code point, to be decoded with [`raw_single`] and
/// [`raw_maxlen`].
#[inline]
pub(crate) fn ducet_raw(cp: u32) -> u32 {
    stage(cp)
}

/// The mapping of a raw entry.
#[inline]
pub(crate) fn raw_single(v: u32) -> Option<&'static [Ce]> {
    if v & PRESENT != 0 {
        Some(slice(v))
    } else {
        None
    }
}

/// Length (in code points) of the longest DUCET contraction starting with the code
/// point of a raw entry (0 if there is none).
#[inline]
pub(crate) fn raw_maxlen(v: u32) -> usize {
    ((v >> 28) & 0x7) as usize
}

/// The DUCET mapping of a single code point.
#[inline]
pub(crate) fn ducet_single(cp: u32) -> Option<&'static [Ce]> {
    raw_single(stage(cp))
}

/// Length (in code points) of the longest DUCET contraction starting with `cp`
/// (0 if there is none).
#[cfg(test)]
pub(crate) fn ducet_maxlen(cp: u32) -> usize {
    raw_maxlen(stage(cp))
}

/// The DUCET mapping of a contraction (2 or 3 code points).
pub(crate) fn ducet_contraction(cps: &[u32]) -> Option<&'static [Ce]> {
    if !(2..=3).contains(&cps.len()) {
        return None;
    }
    let mut key = 0u64;
    for &cp in cps {
        if cp >= 0x110000 {
            return None;
        }
        key = key << 21 | (cp as u64 + 1);
    }
    key <<= 21 * (3 - cps.len() as u32);
    DUCET_CONTR_KEYS
        .binary_search(&key)
        .ok()
        .map(|i| slice(DUCET_CONTR_VALS[i]))
}

/// Iterate over all DUCET contractions (for tests and data checks).
#[cfg(test)]
pub(crate) fn ducet_contractions() -> impl Iterator<Item = (Vec<u32>, &'static [Ce])> {
    DUCET_CONTR_KEYS
        .iter()
        .zip(DUCET_CONTR_VALS.iter())
        .map(|(&k, &v)| {
            let mut cps = Vec::with_capacity(3);
            for i in (0..3).rev() {
                let c = (k >> (21 * i)) & 0x1F_FFFF;
                if c != 0 {
                    cps.push(c as u32 - 1);
                }
            }
            (cps, slice(v))
        })
}

/// Is `cp` a CJK unified ideograph as far as UCA 43 (`UCA_Version` 43) is concerned?
/// Returns `Some(true)` for the "core" ideographs (CJK Unified Ideographs block and the
/// twelve unified ideographs in the compatibility block), `Some(false)` for the
/// extension blocks, `None` otherwise.
#[inline]
pub(crate) fn cjk_unified(cp: u32) -> Option<bool> {
    match cp {
        0x4E00..=0x9FFC => Some(true),
        0xFA0E | 0xFA0F | 0xFA11 | 0xFA13 | 0xFA14 | 0xFA1F | 0xFA21 | 0xFA23 | 0xFA24 | 0xFA27
        | 0xFA28 | 0xFA29 => Some(true),
        0x3400..=0x4DBF
        | 0x20000..=0x2A6DD
        | 0x2A700..=0x2B734
        | 0x2B740..=0x2B81D
        | 0x2B820..=0x2CEA1
        | 0x2CEB0..=0x2EBE0
        | 0x30000..=0x3134A => Some(false),
        _ => None,
    }
}

/// Push the two derived ("implicit") collation elements of `cp` (UTS #10 §10.1, UCA 13.0).
#[inline]
pub(crate) fn push_implicit(cp: u32, out: &mut Vec<Ce>) {
    let (aaaa, bbbb) = match cp {
        // Assigned code points (Unicode 13.0) of the Tangut, Nushu and Khitan blocks;
        // unassigned ones in those blocks get the generic FBC0-family weights.
        0x17000..=0x187F7 | 0x18800..=0x18AFF | 0x18D00..=0x18D08 => {
            (0xFB00, (cp - 0x17000) | 0x8000)
        }
        0x1B170..=0x1B2FB => (0xFB01, (cp - 0x1B170) | 0x8000),
        0x18B00..=0x18CD5 => (0xFB02, (cp - 0x18B00) | 0x8000),
        _ => {
            let base = match cjk_unified(cp) {
                Some(true) => 0xFB40,
                Some(false) => 0xFB80,
                None => 0xFBC0,
            };
            (base + (cp >> 15), (cp & 0x7FFF) | 0x8000)
        }
    };
    let q = cp as u16;
    out.push(Ce::new(aaaa as u16, 0x20, 0x02, q, false));
    out.push(Ce::new(bbbb as u16, 0, 0, q, false));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_lookups() {
        assert_eq!(
            ducet_single(0x61),
            Some(&[Ce::new(0x1FA2, 0x20, 0x02, 0, false)][..])
        );
        assert!(ducet_single(0x2D).unwrap()[0].var);
        assert_eq!(ducet_single(0x4E00), None);
        assert_eq!(ducet_maxlen(0x6C), 2); // l·
        assert_eq!(ducet_maxlen(0x0FB2), 3);
        assert!(ducet_contraction(&[0x0FB2, 0x0F71, 0x0F80]).is_some());
        assert!(ducet_contraction(&[0x0FB2, 0x0F71]).is_none());
        assert_eq!(ducet_contractions().count(), 939);
        for (k, v) in ducet_contractions() {
            assert_eq!(ducet_contraction(&k), Some(v));
        }
    }
}

//! The `overrideCJK` tables of `Unicode::Collate::CJK::*` 1.31 (used by the ja, ko and
//! zh tailorings), compiled into `data/cjk.txt`.

use std::sync::OnceLock;

use rustc_hash::FxHashMap;

use crate::Error;
use crate::tables::Ce;

/// One overrideCJK callback's results for the CJK unified ideographs.
#[derive(Debug)]
pub(crate) struct CjkTable {
    /// Primary weights (0 = no override) for U+3400..U+FAFF, indexed by `cp - 0x3400`.
    bmp: Vec<u16>,
    /// Primary weights for U+20000..U+3134F, indexed by `cp - 0x20000`.
    sip: Vec<u16>,
    /// Multi-element results (Korean): code point → collation elements, with
    /// [`Ce::q`] already filled in.
    complex: FxHashMap<u32, Box<[Ce]>>,
}

const BMP_BASE: u32 = 0x3400;
const BMP_END: u32 = 0xFAFF;
const SIP_BASE: u32 = 0x20000;
const SIP_END: u32 = 0x3134F;

/// The result of the override callback for a code point.
pub(crate) enum Override<'a> {
    /// A single primary weight, i.e. the element `[.p.0020.0002.cp]`.
    Primary(u16),
    /// Several elements.
    Elements(&'a [Ce]),
}

impl CjkTable {
    #[inline]
    pub(crate) fn get(&self, cp: u32) -> Option<Override<'_>> {
        let w = match cp {
            BMP_BASE..=BMP_END => self.bmp[(cp - BMP_BASE) as usize],
            SIP_BASE..=SIP_END => self.sip[(cp - SIP_BASE) as usize],
            _ => 0,
        };
        if w != 0 {
            return Some(Override::Primary(w));
        }
        if self.complex.is_empty() {
            return None;
        }
        self.complex.get(&cp).map(|c| Override::Elements(c))
    }
}

static CJK_TXT: &str = include_str!("../data/cjk.txt");

/// Names of the overrideCJK tables, as used in `data/locales.txt`.
const NAMES: [&str; 7] = [
    "Big5", "GB2312", "JISX0208", "Korean", "Pinyin", "Stroke", "Zhuyin",
];

fn hex(s: &str) -> Result<u32, Error> {
    u32::from_str_radix(s, 16).map_err(|_| Error::InvalidEntry(format!("bad hex {s} in cjk.txt")))
}

fn parse(name: &str) -> Result<CjkTable, Error> {
    let mut t = CjkTable {
        bmp: vec![0; (BMP_END - BMP_BASE + 1) as usize],
        sip: vec![0; (SIP_END - SIP_BASE + 1) as usize],
        complex: FxHashMap::default(),
    };
    let header = format!("@table {name}");
    let mut on = false;
    for line in CJK_TXT.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with("@table ") {
            on = line == header;
            continue;
        }
        if !on {
            continue;
        }
        if let Some((cp, items)) = line.split_once('=') {
            let cp = hex(cp)?;
            let q = cp as u16;
            let mut ces = Vec::new();
            for item in items.split_whitespace() {
                if let Some(arr) = item.strip_prefix('[') {
                    let w = arr
                        .trim_end_matches(']')
                        .split('.')
                        .map(hex)
                        .collect::<Result<Vec<u32>, _>>()?;
                    let g = |i: usize| w.get(i).copied().unwrap_or(0) as u16;
                    ces.push(Ce::new(g(0), g(1), g(2), g(3), false));
                } else {
                    ces.push(Ce::new(hex(item)? as u16, 0x20, 0x02, q, false));
                }
            }
            t.complex.insert(cp, ces.into_boxed_slice());
        } else {
            let mut it = line.split_whitespace();
            let mut w = hex(it.next().unwrap_or(""))?;
            for cp in it {
                let cp = hex(cp)?;
                let slot = match cp {
                    BMP_BASE..=BMP_END => &mut t.bmp[(cp - BMP_BASE) as usize],
                    SIP_BASE..=SIP_END => &mut t.sip[(cp - SIP_BASE) as usize],
                    _ => {
                        return Err(Error::InvalidEntry(format!(
                            "code point {cp:04X} out of range in cjk.txt"
                        )));
                    }
                };
                debug_assert!(w != 0 && w <= 0xFFFF);
                *slot = w as u16;
                w += 1;
            }
        }
    }
    Ok(t)
}

/// The (lazily parsed, cached) table with the given name.
pub(crate) fn table(name: &str) -> Result<&'static CjkTable, Error> {
    static TABLES: [OnceLock<CjkTable>; 7] = [const { OnceLock::new() }; 7];
    let i = NAMES
        .iter()
        .position(|n| *n == name)
        .ok_or_else(|| Error::InvalidOption(format!("unknown CJK table {name}")))?;
    if let Some(t) = TABLES[i].get() {
        return Ok(t);
    }
    let t = parse(name)?;
    Ok(TABLES[i].get_or_init(|| t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables() {
        let j = table("JISX0208").unwrap();
        assert!(matches!(j.get(0x4E9C), Some(Override::Primary(0x8000))));
        assert!(matches!(j.get(0x4E00), Some(Override::Primary(0x804B))));
        let k = table("Korean").unwrap();
        match k.get(0x4E00) {
            Some(Override::Elements(e)) => {
                assert_eq!(e.len(), 3);
                assert_eq!(e[0], Ce::new(0x4180, 0x21, 2, 0x4E00, false));
                assert_eq!(e[1], Ce::new(0x4207, 0x20, 2, 0x4E00, false));
            }
            _ => panic!(),
        }
        for n in NAMES {
            table(n).unwrap();
        }
    }
}

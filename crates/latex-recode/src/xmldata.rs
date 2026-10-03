//! Reading a recode data file (`recode_data.xml` format, `texmap.rnc`).
//!
//! This module is shared with `build.rs` (via `#[path]`), so it depends on
//! nothing but `roxmltree` and `std`.
//!
//! It mirrors the XPath queries of `Biber::LaTeX::Recode::init_sets`:
//! `/texmap/maps[@type=…]`, `map`, the *first* `from`/`to` child of each `map`
//! (`findnodes(...)->shift()`), `map[from[@preferred]]`, `map[from[@raw]]`,
//! `/texmap/decode_exclude/char`, `/texmap/encode_exclude/char`, and
//! `textContent` for values. No normalisation is applied here.

/// One `<map>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawMap {
    /// `textContent` of the first `<from>` (the LaTeX side).
    pub from: String,
    /// `textContent` of the first `<to>` (the Unicode side).
    pub to: String,
    /// Some `<from>` child has a `preferred` attribute (any value).
    pub preferred: bool,
    /// Some `<from>` child has a `raw` attribute (any value).
    pub raw: bool,
}

/// One `<maps type=… set=…>` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawMaps {
    /// The `type` attribute (`letters`, `diacritics`, …).
    pub ty: String,
    /// The `set` attribute split like Perl's `split(/\s*,\s*/, …)`.
    pub sets: Vec<String>,
    /// The `<map>` children, in document order.
    pub maps: Vec<RawMap>,
}

/// A whole recode data file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawData {
    /// All `/texmap/maps` blocks, in document order.
    pub blocks: Vec<RawMaps>,
    /// `/texmap/decode_exclude/char` values.
    pub decode_exclude: Vec<String>,
    /// `/texmap/encode_exclude/char` values.
    pub encode_exclude: Vec<String>,
}

/// Perl `split(/\s*,\s*/, $s)`: leading/trailing whitespace of the whole
/// string is kept, trailing empty fields are dropped.
pub fn split_sets(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let n = s.split(',').count();
    for (i, piece) in s.split(',').enumerate() {
        let mut p = piece;
        if i > 0 {
            p = p.trim_start_matches(perl_space);
        }
        if i + 1 < n {
            p = p.trim_end_matches(perl_space);
        }
        out.push(p.to_string());
    }
    while out.last().is_some_and(|p| p.is_empty()) {
        out.pop();
    }
    out
}

// Perl `\s` for the ASCII/Latin-1 range plus the Unicode spaces.
fn perl_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\x0B'
            | '\x0C'
            | '\r'
            | ' '
            | '\u{85}'
            | '\u{A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

fn text_content(n: roxmltree::Node<'_, '_>) -> String {
    let mut s = String::new();
    for d in n.descendants() {
        if d.is_text() {
            s.push_str(d.text().unwrap_or(""));
        }
    }
    s
}

/// Parse a recode data file.
pub fn parse(xml: &str) -> Result<RawData, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    let mut data = RawData::default();
    if root.tag_name().name() != "texmap" {
        // XPath "/texmap/..." finds nothing.
        return Ok(data);
    }
    for el in root.children().filter(|n| n.is_element()) {
        match el.tag_name().name() {
            "maps" => {
                let ty = el.attribute("type").unwrap_or("").to_string();
                let sets = el.attribute("set").map(split_sets).unwrap_or_default();
                let mut maps = Vec::new();
                for m in el
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "map")
                {
                    let froms: Vec<_> = m
                        .children()
                        .filter(|n| n.is_element() && n.tag_name().name() == "from")
                        .collect();
                    let to = m
                        .children()
                        .find(|n| n.is_element() && n.tag_name().name() == "to");
                    let (Some(from), Some(to)) = (froms.first(), to) else {
                        return Err(format!(
                            "<map> without <from> or <to> in <maps type=\"{ty}\"> (line {})",
                            doc.text_pos_at(m.range().start).row
                        ));
                    };
                    maps.push(RawMap {
                        from: text_content(*from),
                        to: text_content(to),
                        preferred: froms.iter().any(|f| f.has_attribute("preferred")),
                        raw: froms.iter().any(|f| f.has_attribute("raw")),
                    });
                }
                data.blocks.push(RawMaps { ty, sets, maps });
            }
            name @ ("decode_exclude" | "encode_exclude") => {
                let v: Vec<String> = el
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "char")
                    .map(text_content)
                    .collect();
                if name == "decode_exclude" {
                    data.decode_exclude.extend(v);
                } else {
                    data.encode_exclude.extend(v);
                }
            }
            _ => {}
        }
    }
    Ok(data)
}

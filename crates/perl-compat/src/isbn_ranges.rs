//! Parser for the ISBN International `RangeMessage.xml`, replicating
//! `Business::ISBN::Data::_parse_range_message` (20260218.001). Shared by
//! `build.rs` (which compiles the vendored file into static tables) and by the
//! runtime `ISBN_RANGE_MESSAGE` override.
//!
//! This file must not use anything outside `std` (it is `#[path]`-included by
//! the build script).

/// One registration group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGroup {
    /// `"978"` or `"979"`.
    pub ean_prefix: String,
    /// The registration group code (e.g. `"0"`, `"65"`, `"99937"`).
    pub group: String,
    /// The agency name, whitespace-trimmed (entities are not decoded).
    pub agency: String,
    /// Flattened `(low, high)` publisher ranges, truncated to the rule length,
    /// rules of length 0 omitted, in file order.
    pub ranges: Vec<(String, String)>,
}

/// Find `<tag>…</tag>` occurrences (non-greedy, like perl's `(.*?)`).
fn all_between<'a>(s: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find(open) {
        let after = &rest[i + open.len()..];
        match after.find(close) {
            Some(j) => {
                out.push(&after[..j]);
                rest = &after[j + close.len()..];
            }
            None => break,
        }
    }
    out
}

/// First `<tag>(.*?)</tag>` *within a single line* (perl regex without `/s`).
fn first_between_line<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let mut rest = s;
    while let Some(i) = rest.find(open) {
        let after = &rest[i + open.len()..];
        let line_end = after.find('\n').unwrap_or(after.len());
        if let Some(j) = after[..line_end].find(close) {
            return Some(&after[..j]);
        }
        rest = after;
    }
    None
}

/// Parse a `RangeMessage.xml` document.
pub fn parse_range_message(data: &str) -> Vec<ParsedGroup> {
    let mut out = Vec::new();
    let Some(groups_blk) = all_between(data, "<RegistrationGroups>", "</RegistrationGroups>")
        .into_iter()
        .next()
    else {
        return out;
    };
    for group in all_between(groups_blk, "<Group>", "</Group>") {
        // m|<Prefix>(97[89]-)?([0-9]+?)</Prefix>|
        let mut ean = "978".to_string();
        let mut code = None;
        let mut rest = group;
        while let Some(i) = rest.find("<Prefix>") {
            let after = &rest[i + "<Prefix>".len()..];
            let (pre, digits_start) = if after.starts_with("979-") {
                ("979", 4)
            } else if after.starts_with("978-") {
                ("978", 4)
            } else {
                ("978", 0)
            };
            let d = &after[digits_start..];
            let n = d.bytes().take_while(u8::is_ascii_digit).count();
            if n > 0 && d[n..].starts_with("</Prefix>") {
                ean = pre.to_string();
                code = Some(d[..n].to_string());
                break;
            }
            rest = after;
        }
        let Some(code) = code else { continue };
        let agency = first_between_line(group, "<Agency>", "</Agency>")
            .unwrap_or("")
            .trim_matches(|c: char| c.is_whitespace())
            .to_string();
        let mut ranges = Vec::new();
        for rule in all_between(group, "<Rule>", "</Rule>") {
            if rule.contains("<Length>0</Length>") {
                continue;
            }
            let range = first_between_line(rule, "<Range>", "</Range>").unwrap_or("");
            let len: usize = first_between_line(rule, "<Length>", "</Length>")
                .and_then(|l| l.trim().parse().ok())
                .unwrap_or(0);
            let (lo, hi) = range.split_once('-').unwrap_or((range, ""));
            let cut = |s: &str| s.chars().take(len).collect::<String>();
            ranges.push((cut(lo), cut(hi)));
        }
        out.push(ParsedGroup { ean_prefix: ean, group: code, agency, ranges });
    }
    out
}

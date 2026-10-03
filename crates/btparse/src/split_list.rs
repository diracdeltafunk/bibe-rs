//! `Text::BibTeX::split_list` as biber calls it (spec §5).

use crate::diag::{Diag, DiagKind};

/// Result of [`split_list`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitList<'a> {
    /// The pieces, borrowed from the input. Never trimmed.
    pub items: Vec<&'a str>,
    /// Warnings in btparse's order, rendered as biber sees them (no file/line:
    /// biber passes `undef` for both).
    pub diags: Vec<Diag>,
    /// `true` if btparse would abort the process on this input
    /// (`internal error: stop == start for substring N`); `items` is then a
    /// best effort and `diags` ends with that message.
    pub crash: bool,
}

/// Split `s` on `delim` like btparse (spec §5): a delimiter occurrence counts
/// only at brace depth 0, preceded and followed by exactly one ASCII space
/// that belongs to it, and compares ASCII-case-insensitively *on the string
/// side only* (the delimiter itself is not lower-cased).
pub fn split_list<'a>(s: &'a str, delim: &str) -> SplitList<'a> {
    split_list_desc(s, delim, "substring")
}

/// As [`split_list`] with btparse's `description` argument (used in the
/// "empty <description>" warning). biber always uses the default.
pub fn split_list_desc<'a>(s: &'a str, delim: &str, desc: &str) -> SplitList<'a> {
    let b = s.as_bytes();
    let d = delim.as_bytes();
    let mut out = SplitList {
        items: Vec::new(),
        diags: Vec::new(),
        crash: false,
    };
    if b.is_empty() {
        return out;
    }
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut i = 0usize;
    let mut unmatched_close = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            if depth == 0 {
                unmatched_close += 1;
                out.diags.push(Diag::new(
                    DiagKind::Bare,
                    0,
                    "warning: unmatched '}' (ignoring)",
                ));
            } else {
                depth -= 1;
            }
        } else if depth == 0
            && !d.is_empty()
            && i >= 1
            && b[i - 1] == b' '
            && i + d.len() < b.len()
            && b[i + d.len()] == b' '
            && b[i..i + d.len()]
                .iter()
                .zip(d)
                .all(|(x, y)| x.to_ascii_lowercase() == *y)
        {
            let stop = i - 1;
            push_piece(&mut out, s, start, stop, desc);
            start = i + d.len() + 1;
            i += d.len();
            continue;
        }
        i += 1;
    }
    let _ = unmatched_close;
    if depth > 0 {
        out.diags.push(Diag::new(
            DiagKind::Bare,
            0,
            "warning: unmatched '{' (ignoring)",
        ));
    }
    push_piece(&mut out, s, start, b.len(), desc);
    out
}

fn push_piece<'a>(out: &mut SplitList<'a>, s: &'a str, start: usize, stop: usize, desc: &str) {
    let idx = out.items.len();
    if stop == start {
        if !out.crash {
            out.crash = true;
            out.diags.push(Diag::new(
                DiagKind::Bare,
                0,
                format!("internal error: stop == start for substring {idx}"),
            ));
        }
        out.items.push("");
    } else if stop < start {
        out.diags.push(Diag::new(
            DiagKind::Bare,
            0,
            format!("{desc} {}, warning: empty {desc}", idx + 1),
        ));
        out.items.push("");
    } else {
        out.items.push(&s[start..stop]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples() {
        assert_eq!(split_list("A and B", "and").items, ["A", "B"]);
        assert_eq!(split_list("A  and   B", "and").items, ["A ", "  B"]);
        assert_eq!(split_list("A AND B", "and").items, ["A", "B"]);
        assert_eq!(split_list("A and B", "AND").items, ["A and B"]);
        let r = split_list("A and and B", "and");
        assert_eq!(r.items, ["A", "", "B"]);
        assert_eq!(
            r.diags[0].render(""),
            "substring 2, warning: empty substring"
        );
        assert!(split_list(" and B", "and").crash);
        assert!(split_list("", "and").items.is_empty());
    }
}

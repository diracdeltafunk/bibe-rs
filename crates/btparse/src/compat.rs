//! biber's two-pass round trip (spec §9).
//!
//! biber parses each `.bib` twice: pass 1 (`parse_decode`) parses the file,
//! LaTeX-decodes every non-verbatim field value and writes a new BibTeX text,
//! which pass 2 (`cache_data`) parses again. The artefacts of that round trip
//! (decoded values with unbalanced braces corrupting the second parse, ...)
//! are visible in biber's output, so the pipeline reproduces them by running
//! the same text through [`crate::BtState::parse`] twice. This module builds
//! the pass-1 text. Two round-trip artefacts that make biber fail are fixed
//! (docs/compat.md DC-002, DC-003): multi-macro `@string`s keep their commas
//! here, and the caller leaves values undecoded when decoding would unbalance
//! their braces.

use std::borrow::Cow;

use crate::bib::{Entry, Metatype};

/// Append biber's pass-1 re-serialisation of `entry` to `out`
/// (`Biber::Input::file::bibtex::parse_decode`). `decode(field, value)` is
/// applied to each regular-entry field value (biber: `latex_decode` unless the
/// field is a verbatim field); `undef` values are written as empty.
pub fn reserialise_entry<'e>(
    entry: &'e Entry,
    out: &mut String,
    mut decode: impl FnMut(&'e str, &'e str) -> Cow<'e, str>,
) {
    match entry.metatype {
        Metatype::Regular => {
            out.push('@');
            out.push_str(&entry.entry_type);
            out.push('{');
            out.push_str(entry.key.as_deref().unwrap_or(""));
            out.push_str(",\n");
            for (name, value) in entry.fieldlist_values() {
                let v = decode(name, value.unwrap_or(""));
                out.push_str("  ");
                out.push_str(name);
                out.push_str(" = {");
                out.push_str(&v);
                out.push_str("},\n");
            }
            out.push_str("\n}\n\n");
        }
        Metatype::Preamble => {
            out.push_str("@PREAMBLE{\"");
            out.push_str(entry.value.as_deref().unwrap_or(""));
            out.push_str("\"}\n");
        }
        Metatype::Comment => {
            out.push_str("@COMMENT{");
            out.push_str(entry.value.as_deref().unwrap_or(""));
            out.push_str("}\n");
        }
        Metatype::MacroDef => {
            // biber writes no separator between definitions, so a multi-macro
            // `@string` is a fatal syntax error in its pass 2 (BB-002).
            // bibers fixes this (docs/compat.md DC-002): definitions are
            // comma-separated, as in the source.
            out.push_str("@STRING{");
            for (i, (name, value)) in entry.fieldlist_values().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(name);
                out.push_str(" = {");
                out.push_str(value.unwrap_or(""));
                out.push('}');
            }
            out.push_str("}\n");
        }
    }
}

/// Pass-1 text for a whole file.
pub fn reserialise<'e>(
    entries: &'e [Entry],
    mut decode: impl FnMut(&'e str, &'e str) -> Cow<'e, str>,
) -> String {
    let mut out = String::new();
    for e in entries {
        reserialise_entry(e, &mut out, &mut decode);
    }
    out
}

/// Does `{value}` lex back as exactly one braced string whose content is
/// `value`? False if the braces in `value` are unbalanced (e.g. a decoded
/// `\char"7B`), in which case pass 2 parses something else.
pub fn braces_round_trip(value: &str) -> bool {
    let mut depth = 0i64;
    for b in value.bytes() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

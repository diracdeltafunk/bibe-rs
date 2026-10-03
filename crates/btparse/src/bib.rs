//! The `.bib` lexer and parser (spec §2–§3), reproducing btparse's token
//! modes, LL(1) error reporting and resynchronisation, and value
//! post-processing.

use crate::diag::{Diag, DiagKind};
use crate::macros::MacroTable;

/// Classification of an entry by its type (spec §2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Metatype {
    /// Any type other than the three below.
    Regular,
    /// `@comment`
    Comment,
    /// `@preamble`
    Preamble,
    /// `@string`
    MacroDef,
}

/// One `name = value` of a regular or `@string` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Lower-cased (ASCII) field name.
    pub name: String,
    /// The post-processed value; `None` when btparse yields `undef` (no value
    /// was parsed, or the value is a single undefined macro).
    pub value: Option<String>,
    /// Line of the field name.
    pub line: u32,
}

/// One entry as `Text::BibTeX::Entry->new($file)` returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub metatype: Metatype,
    /// Lower-cased (ASCII) entry type.
    pub entry_type: String,
    /// The key of a regular entry, case preserved. `None` if absent/unparsed.
    pub key: Option<String>,
    /// Fields in source order, *including* duplicates (btparse's
    /// `fieldlist`). See [`Entry::get`] for `get` semantics.
    pub fields: Vec<Field>,
    /// The value of `@preamble`/`@comment` entries (`None` = undef; the Perl
    /// `value` method then returns `""`).
    pub value: Option<String>,
    /// `parse_ok`: false after any syntax or lexical *error*.
    pub parse_ok: bool,
    /// Line of the `@`.
    pub line: u32,
    /// Diagnostics printed while this entry was read (including warnings
    /// about junk *after* it, which btparse emits when it reads ahead).
    pub diags: Vec<Diag>,
}

impl Entry {
    /// `$entry->get($field)`: the value of the *last* occurrence.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .rev()
            .find(|f| f.name == name)
            .and_then(|f| f.value.as_deref())
    }

    /// `$entry->exists($field)`.
    pub fn exists(&self, name: &str) -> bool {
        self.fields.iter().any(|f| f.name == name)
    }

    /// `map { [$_, $entry->get($_)] } $entry->fieldlist`: every occurrence
    /// paired with the value of the last occurrence of that name.
    pub fn fieldlist_values(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.fields
            .iter()
            .map(|f| (f.name.as_str(), self.get(&f.name)))
    }
}

/// What the parser yields per call to `Text::BibTeX::Entry->new`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Entry(Entry),
    /// Text::BibTeX returned false although input remains (the entry type
    /// was missing), so biber's read loop stops here. If `fatal`, libbtparse
    /// terminated the process instead (biber dies).
    Stop {
        diags: Vec<Diag>,
        fatal: bool,
    },
}

/// libbtparse's process-global state: the macro table and the parser's
/// error-recovery flag (spec §3.4), both of which persist across files.
#[derive(Clone, Debug)]
pub struct BtState {
    pub macros: MacroTable,
    /// PCCTS `zzresynch`'s static `consumed` flag.
    resync_consumed: bool,
}

impl Default for BtState {
    fn default() -> Self {
        Self::new(MacroTable::text_bibtex_initial())
    }
}

impl BtState {
    pub fn new(macros: MacroTable) -> Self {
        BtState {
            macros,
            resync_consumed: true,
        }
    }

    /// State as in biber before reading its first data source.
    pub fn biber() -> Self {
        Self::new(MacroTable::biber_initial())
    }

    /// Parse a whole file, collecting what biber's `while (Entry->new)` loop
    /// sees: entries up to the first [`Item::Stop`], plus diagnostics.
    pub fn parse(&mut self, src: &str) -> ParsedFile {
        let mut p = Parser::new(src, self);
        let mut entries = Vec::new();
        let mut stop = None;
        for item in p.by_ref() {
            match item {
                Item::Entry(e) => entries.push(e),
                Item::Stop { diags, fatal } => {
                    stop = Some((diags, fatal));
                    break;
                }
            }
        }
        let trailing = std::mem::take(&mut p.pending);
        ParsedFile {
            entries,
            stop,
            trailing_diags: trailing,
        }
    }
}

/// Result of [`BtState::parse`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedFile {
    pub entries: Vec<Entry>,
    /// `Some` if reading stopped early (see [`Item::Stop`]).
    pub stop: Option<(Vec<Diag>, bool)>,
    /// Diagnostics printed after the last entry was returned.
    pub trailing_diags: Vec<Diag>,
}

impl ParsedFile {
    /// All diagnostics in print order.
    pub fn all_diags(&self) -> impl Iterator<Item = &Diag> {
        self.entries
            .iter()
            .flat_map(|e| e.diags.iter())
            .chain(self.stop.iter().flat_map(|(d, _)| d.iter()))
            .chain(self.trailing_diags.iter())
    }
}

// ---------------------------------------------------------------------------
// Lexer

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Top,
    AfterAt,
    AfterType,
    InEntry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Tk {
    At,
    Number,
    Name,
    EntryOpen,
    EntryClose,
    Equals,
    Hash,
    Comma,
    Str,
    /// the `"` btparse returns after a comment entry with a bad delimiter
    Junk,
    Eof,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Tk,
    /// For Name/Number: the text. For Str: the processed content *without*
    /// delimiters. Otherwise empty.
    text: Vec<u8>,
    /// For Str: opening delimiter (`{`, `"` or `(`).
    delim: u8,
    /// For EntryOpen/EntryClose: the character.
    ch: u8,
    line: u32,
}

impl Token {
    fn simple(kind: Tk, ch: u8, line: u32) -> Self {
        Token {
            kind,
            text: Vec::new(),
            delim: 0,
            ch,
            line,
        }
    }

    /// The text btparse shows in `found "<text>"`.
    fn found_text(&self) -> String {
        match self.kind {
            Tk::Name | Tk::Number => String::from_utf8_lossy(&self.text).into_owned(),
            Tk::Str => {
                let close = match self.delim {
                    b'{' => b'}',
                    b'(' => b')',
                    d => d,
                };
                let mut v = Vec::with_capacity(self.text.len() + 2);
                v.push(self.delim);
                v.extend_from_slice(&self.text);
                v.push(close);
                printf_mangle(&String::from_utf8_lossy(&v))
            }
            Tk::At => "@".into(),
            Tk::Equals => "=".into(),
            Tk::Hash => "#".into(),
            Tk::Comma => ",".into(),
            Tk::Junk => "\"".into(),
            Tk::EntryOpen | Tk::EntryClose => (self.ch as char).to_string(),
            Tk::Eof => String::new(),
        }
    }
}

/// btparse passes the offending token text to `printf` as part of the
/// format string (spec §8.3), so `%` sequences in it are interpreted.
fn printf_mangle(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        // flags, width, precision, length modifiers
        let mut width = 0usize;
        let mut left = false;
        let mut in_prec = false;
        while let Some(&n) = it.peek() {
            match n {
                '-' => left = true,
                '.' => in_prec = true,
                '0'..='9' if !in_prec => {
                    width = width
                        .saturating_mul(10)
                        .saturating_add(n as usize - '0' as usize);
                }
                '+' | ' ' | '#' | '0'..='9' | '\'' | 'h' | 'l' | 'L' | 'q' | 'j' | 'z' | 't' => {}
                _ => break,
            }
            it.next();
        }
        match it.next() {
            None => {}
            Some('%') => {
                let pad = " ".repeat(width.saturating_sub(1).min(4096));
                if left {
                    out.push('%');
                    out.push_str(&pad);
                } else {
                    out.push_str(&pad);
                    out.push('%');
                }
            }
            Some('d' | 'i' | 'o' | 'u' | 'x' | 'X') => out.push('0'),
            Some('s') => out.push_str("(null)"),
            Some('c') => {}
            Some(other) => out.push(other),
        }
    }
    out
}

fn describe(t: Tk) -> &'static str {
    match t {
        Tk::Number => "number",
        Tk::Name => "name (entry type, key, field, or macro name)",
        Tk::EntryOpen => "start of entry (\"{\" or \"(\")",
        Tk::EntryClose => "end of entry (\"}\" or \")\")",
        Tk::Str => "quoted string ({...} or \"...\")",
        Tk::Equals => "\"=\"",
        Tk::Comma => "\",\"",
        Tk::Hash => "\"#\"",
        Tk::At => "\"@\"",
        Tk::Junk | Tk::Eof => "end of input",
    }
}

#[inline]
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Bytes that may appear in a NAME token outside the top level (spec §2.2).
#[inline]
fn is_name_byte(b: u8) -> bool {
    !(b < 0x20
        || b == 0x7f
        || matches!(
            b,
            b' ' | b'"' | b'#' | b'%' | b'\'' | b'(' | b')' | b',' | b'=' | b'{' | b'}' | b'\\'
        ))
}

/// Characters that are invalid tokens (reported, then skipped together with
/// the following byte).
#[inline]
fn is_invalid_byte(b: u8) -> bool {
    (b < 0x20 && !is_ws(b)) || b == 0x7f || b == b'\'' || b == b'\\'
}

fn metatype_of(name: &[u8]) -> Metatype {
    if name.eq_ignore_ascii_case(b"comment") {
        Metatype::Comment
    } else if name.eq_ignore_ascii_case(b"preamble") {
        Metatype::Preamble
    } else if name.eq_ignore_ascii_case(b"string") {
        Metatype::MacroDef
    } else {
        Metatype::Regular
    }
}

/// Piece state while reading string text (spec §2.6).
struct Pieces {
    /// inside a newline piece: whitespace becomes spaces
    in_w: bool,
    /// at the first character of a piece
    at_start: bool,
}

impl Pieces {
    fn new() -> Self {
        Pieces {
            in_w: false,
            at_start: true,
        }
    }

    fn boundary(&mut self) {
        self.in_w = false;
        self.at_start = true;
    }
}

struct Lexer<'s> {
    src: &'s [u8],
    pos: usize,
    line: u32,
    mode: Mode,
    metatype: Metatype,
    opener: u8,
    junk: usize,
    /// the previous top-level character was junk (a `%` then does not start
    /// a comment)
    junk_run: bool,
}

impl<'s> Lexer<'s> {
    fn warn(&self, out: &mut Vec<Diag>, msg: impl Into<String>) {
        out.push(Diag::new(DiagKind::Warning, self.line, msg));
    }

    /// Skip a `%` comment. Returns false (after reporting) if it runs into
    /// the end of input without a newline.
    fn skip_comment(&mut self, out: &mut Vec<Diag>) -> bool {
        let start = self.pos;
        match self.src[self.pos..].iter().position(|&b| b == b'\n') {
            Some(off) => {
                self.pos += off + 1;
                self.line += 1;
                true
            }
            None => {
                // At the top level, a run without blanks is lexed as junk
                // instead (spec §2.3); otherwise it is an invalid token.
                let rest = &self.src[start..];
                let junk = self.mode == Mode::Top && !rest.iter().any(|&b| b == b' ' || b == b'\t');
                if !junk {
                    let text = String::from_utf8_lossy(&self.src[start..]).into_owned();
                    out.push(Diag::new(DiagKind::InvalidToken, self.line, text));
                }
                self.pos = self.src.len();
                false
            }
        }
    }

    fn invalid(&mut self, out: &mut Vec<Diag>) {
        let b = self.src[self.pos];
        let text = (b as char).to_string();
        out.push(Diag::new(DiagKind::InvalidToken, self.line, text));
        // the offending byte and the one after it are dropped (a dropped
        // newline is not counted)
        self.pos = (self.pos + 2).min(self.src.len());
    }

    /// Read a `{...}` (depth-balanced) or `"..."` string starting at `pos`
    /// (which holds the delimiter). Returns None at end of input.
    fn read_string(&mut self, out: &mut Vec<Diag>) -> Option<Token> {
        let delim = self.src[self.pos];
        self.pos += 1;
        let mut text = Vec::new();
        let mut depth: usize = if delim == b'{' { 1 } else { 0 };
        let mut ps = Pieces::new();
        let start_line = self.line;
        let mut runaway_warned = false;
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            self.pos += 1;
            if b == b'\n' && !runaway_warned && self.looks_like_field(self.pos) {
                runaway_warned = true;
                out.push(Diag::new(
                    DiagKind::Warning,
                    self.line + 1,
                    format!("possible runaway string started at line {start_line}"),
                ));
            }
            match b {
                b'{' => {
                    depth += 1;
                    ps.boundary();
                    text.push(b);
                }
                b'}' => {
                    ps.boundary();
                    if depth == 0 {
                        out.push(Diag::new(
                            DiagKind::Error,
                            self.line,
                            "unbalanced braces: too many }'s",
                        ));
                        text.push(b);
                        continue;
                    }
                    depth -= 1;
                    if depth == 0 && delim == b'{' {
                        return Some(Token {
                            kind: Tk::Str,
                            text,
                            delim,
                            ch: 0,
                            line: self.line,
                        });
                    }
                    text.push(b);
                }
                b'"' if delim == b'"' && depth == 0 => {
                    return Some(Token {
                        kind: Tk::Str,
                        text,
                        delim,
                        ch: 0,
                        line: self.line,
                    });
                }
                _ => self.piece_char(b, &mut text, &mut ps),
            }
        }
        None
    }

    /// Does the line starting at `i` look like `name =` (spec §2.6)?
    fn looks_like_field(&self, mut i: usize) -> bool {
        let s = self.src;
        while i < s.len() && matches!(s[i], b' ' | b'\t') {
            i += 1;
        }
        if i >= s.len() || !s[i].is_ascii_alphabetic() {
            return false;
        }
        while i < s.len()
            && (s[i].is_ascii_alphanumeric()
                || matches!(s[i], b':' | b'+' | b'/' | b'\'' | b'.' | b'-'))
        {
            i += 1;
        }
        while i < s.len() && matches!(s[i], b' ' | b'\t') {
            i += 1;
        }
        i < s.len() && s[i] == b'='
    }

    /// One character of string text other than a brace or the closing
    /// delimiter (spec §2.6: newline/tab conversion by "pieces").
    fn piece_char(&mut self, b: u8, text: &mut Vec<u8>, ps: &mut Pieces) {
        match b {
            b'\n' => {
                self.line += 1;
                text.push(b' ');
                ps.in_w = true;
                ps.at_start = false;
            }
            b'"' => {
                text.push(b);
                ps.boundary();
            }
            b'\r' => ps.at_start = false,
            b'(' | b')' => {
                text.push(b);
                ps.boundary();
            }
            b'\\' => {
                text.push(b);
                ps.in_w = false;
                ps.at_start = false;
            }
            b'\t' | 0x0b | 0x0c if ps.in_w => text.push(b' '),
            b'\t'
                if ps.at_start
                    && matches!(
                        self.src.get(self.pos),
                        None | Some(b'{' | b'}' | b'"' | b'\n' | b'(' | b')')
                    ) =>
            {
                text.push(b' ');
                ps.at_start = false;
            }
            _ => {
                text.push(b);
                ps.at_start = false;
            }
        }
    }

    /// Read the body of `@comment{...}` / `@comment(...)`.
    fn read_comment(&mut self, out: &mut Vec<Diag>) -> Option<Token> {
        let delim = self.src[self.pos];
        if delim == b'{' {
            return self.read_string(out);
        }
        self.pos += 1;
        let mut text = Vec::new();
        let mut pdepth = 0usize;
        let mut bdepth = 0usize;
        let mut ps = Pieces::new();
        let start_line = self.line;
        let mut runaway_warned = false;
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            self.pos += 1;
            if b == b'\n' && !runaway_warned && self.looks_like_field(self.pos) {
                runaway_warned = true;
                out.push(Diag::new(
                    DiagKind::Warning,
                    self.line + 1,
                    format!("possible runaway string started at line {start_line}"),
                ));
            }
            match b {
                b'(' => {
                    pdepth += 1;
                    text.push(b);
                }
                b')' => {
                    if pdepth == 0 {
                        if bdepth > 0 {
                            out.push(Diag::new(
                                DiagKind::Error,
                                self.line,
                                "unbalanced braces: too many {'s",
                            ));
                        }
                        return Some(Token {
                            kind: Tk::Str,
                            text,
                            delim,
                            ch: 0,
                            line: self.line,
                        });
                    }
                    pdepth -= 1;
                    text.push(b);
                }
                b'{' => {
                    bdepth += 1;
                    ps.boundary();
                    text.push(b);
                }
                b'}' => {
                    ps.boundary();
                    if bdepth == 0 {
                        out.push(Diag::new(
                            DiagKind::Error,
                            self.line,
                            "unbalanced braces: too many }'s",
                        ));
                    } else {
                        bdepth -= 1;
                    }
                    text.push(b);
                }
                _ => self.piece_char(b, &mut text, &mut ps),
            }
        }
        None
    }

    fn next(&mut self, out: &mut Vec<Diag>) -> Token {
        loop {
            if self.pos >= self.src.len() {
                return Token::simple(Tk::Eof, 0, self.line);
            }
            let b = self.src[self.pos];
            if self.mode == Mode::Top {
                match b {
                    b'@' => {
                        self.junk_run = false;
                        if self.junk > 0 {
                            self.warn(
                                out,
                                format!("{} characters of junk seen at toplevel", self.junk),
                            );
                            self.junk = 0;
                        }
                        self.pos += 1;
                        self.mode = Mode::AfterAt;
                        return Token::simple(Tk::At, b'@', self.line);
                    }
                    b'%' if !self.junk_run => {
                        self.skip_comment(out);
                    }
                    b'\n' => {
                        self.junk_run = false;
                        self.line += 1;
                        self.pos += 1;
                    }
                    _ if is_ws(b) => {
                        self.junk_run = false;
                        self.pos += 1;
                    }
                    _ => {
                        self.junk_run = true;
                        self.junk += 1;
                        self.pos += 1;
                    }
                }
                continue;
            }
            match b {
                b'\n' => {
                    self.line += 1;
                    self.pos += 1;
                }
                _ if is_ws(b) => self.pos += 1,
                b'%' => {
                    self.skip_comment(out);
                }
                b'{' | b'(' => match self.mode {
                    Mode::AfterType => {
                        if self.metatype == Metatype::Comment {
                            match self.read_comment(out) {
                                Some(t) => {
                                    self.mode = Mode::Top;
                                    return t;
                                }
                                None => return Token::simple(Tk::Eof, 0, self.line),
                            }
                        }
                        self.pos += 1;
                        self.opener = b;
                        self.mode = Mode::InEntry;
                        return Token::simple(Tk::EntryOpen, b, self.line);
                    }
                    Mode::InEntry if b == b'{' => {
                        return match self.read_string(out) {
                            Some(t) => t,
                            None => Token::simple(Tk::Eof, 0, self.line),
                        };
                    }
                    _ => {
                        self.warn(
                            out,
                            format!(
                                "\"{}\" in strange place -- should get a syntax error",
                                b as char
                            ),
                        );
                        self.pos += 1;
                        return Token::simple(Tk::EntryOpen, b, self.line);
                    }
                },
                b'}' | b')' => {
                    self.pos += 1;
                    if self.mode == Mode::InEntry {
                        let want = if self.opener == b'(' { b')' } else { b'}' };
                        if b != want {
                            self.warn(
                                out,
                                format!(
                                    "entry started with \"{}\", but ends with \"{}\"",
                                    self.opener as char, b as char
                                ),
                            );
                        }
                        self.mode = Mode::Top;
                    } else {
                        self.warn(
                            out,
                            format!(
                                "\"{}\" in strange place -- should get a syntax error",
                                b as char
                            ),
                        );
                    }
                    return Token::simple(Tk::EntryClose, b, self.line);
                }
                b'"' => {
                    if self.mode != Mode::InEntry {
                        if self.mode == Mode::AfterType && self.metatype == Metatype::Comment {
                            out.push(Diag::new(
                                DiagKind::Error,
                                self.line,
                                "comment entries must be delimited by either braces or parentheses",
                            ));
                            self.pos += 1;
                            self.mode = Mode::Top;
                            return Token::simple(Tk::Junk, b'"', self.line);
                        }
                        self.warn(out, "start of string seen at weird place");
                    }
                    return match self.read_string(out) {
                        Some(t) => t,
                        None => Token::simple(Tk::Eof, 0, self.line),
                    };
                }
                b'#' => {
                    self.pos += 1;
                    return Token::simple(Tk::Hash, b, self.line);
                }
                b'=' => {
                    self.pos += 1;
                    return Token::simple(Tk::Equals, b, self.line);
                }
                b',' => {
                    self.pos += 1;
                    return Token::simple(Tk::Comma, b, self.line);
                }
                _ if is_invalid_byte(b) => self.invalid(out),
                _ => {
                    let start = self.pos;
                    while self.pos < self.src.len() && is_name_byte(self.src[self.pos]) {
                        self.pos += 1;
                    }
                    let text = self.src[start..self.pos].to_vec();
                    let kind = if text.iter().all(u8::is_ascii_digit) {
                        Tk::Number
                    } else {
                        Tk::Name
                    };
                    if self.mode == Mode::AfterAt && kind == Tk::Name {
                        self.mode = Mode::AfterType;
                        self.metatype = metatype_of(&text);
                    }
                    return Token {
                        kind,
                        text,
                        delim: 0,
                        ch: 0,
                        line: self.line,
                    };
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parser

/// A piece of a value before post-processing.
enum Piece {
    Str(Vec<u8>),
    Num(Vec<u8>),
    Macro(Vec<u8>, u32),
}

struct RawField {
    name: Vec<u8>,
    line: u32,
    pieces: Option<Vec<Piece>>,
}

/// Streaming parser over one file. Yields one [`Item`] per
/// `Text::BibTeX::Entry->new` call.
pub struct Parser<'s, 'st> {
    lx: Lexer<'s>,
    st: &'st mut BtState,
    la: Option<Token>,
    pending: Vec<Diag>,
    ok: bool,
    done: bool,
}

/// Token sets for resynchronisation (bit per [`Tk`]).
type Set = u32;
const fn bit(t: Tk) -> Set {
    1 << (t as u32)
}
const FOLLOW_ENTRY: Set = bit(Tk::At);
const FOLLOW_CONTENTS: Set = bit(Tk::EntryClose);
const FOLLOW_FIELD: Set = bit(Tk::Comma) | bit(Tk::EntryClose);
const FOLLOW_SIMPLE: Set = bit(Tk::Hash) | bit(Tk::Comma) | bit(Tk::EntryClose);

impl<'s, 'st> Parser<'s, 'st> {
    pub fn new(src: &'s str, st: &'st mut BtState) -> Self {
        Parser {
            lx: Lexer {
                src: src.as_bytes(),
                pos: 0,
                line: 1,
                mode: Mode::Top,
                metatype: Metatype::Regular,
                opener: b'{',
                junk: 0,
                junk_run: false,
            },
            st,
            la: None,
            pending: Vec::new(),
            ok: true,
            done: false,
        }
    }

    fn la(&self) -> &Token {
        self.la.as_ref().expect("lookahead")
    }

    fn lk(&self) -> Tk {
        self.la().kind
    }

    fn consume(&mut self) -> Token {
        let next = self.lx.next(&mut self.pending);
        if self.pending.iter().any(|d| d.kind == DiagKind::Error) {
            self.ok = false;
        }
        std::mem::replace(self.la.as_mut().expect("lookahead"), next)
    }

    fn syntax(&mut self, msg: String) {
        let line = self.la().line;
        self.pending
            .push(Diag::new(DiagKind::SyntaxError, line, msg));
        self.ok = false;
    }

    /// `syntax error: found "X", expected ...` for lookahead not in `expected`.
    fn error_expected(&mut self, expected: &[Tk], suffix: &str) {
        let mut exp: Vec<Tk> = expected.to_vec();
        exp.sort();
        let list = match exp.len() {
            1 => describe(exp[0]).to_string(),
            n => {
                let head: Vec<&str> = exp[..n - 1].iter().map(|&t| describe(t)).collect();
                format!("one of: {} or {}", head.join(", "), describe(exp[n - 1]))
            }
        };
        let msg = if self.lk() == Tk::Eof {
            format!("at end of input, expected {list}{suffix}")
        } else {
            format!(
                "found \"{}\", expected {list}{suffix}",
                self.la().found_text()
            )
        };
        self.syntax(msg);
    }

    /// Failed semantic predicate: `syntax error: found "X"`.
    fn error_predicate(&mut self) {
        let msg = format!("found \"{}\"", self.la().found_text());
        self.syntax(msg);
    }

    /// PCCTS `zzresynch` (spec §3.4).
    fn resync(&mut self, set: Set) {
        if !self.st.resync_consumed {
            self.consume();
            return;
        }
        let in_set = |t: Tk| set & bit(t) != 0 || t == Tk::Eof;
        if in_set(self.lk()) {
            self.st.resync_consumed = false;
            return;
        }
        while !in_set(self.lk()) {
            self.consume();
        }
        self.st.resync_consumed = true;
    }

    fn simple_value(&mut self, pieces: &mut Vec<Piece>) {
        match self.lk() {
            Tk::Str => {
                let t = self.consume();
                pieces.push(Piece::Str(t.text));
            }
            Tk::Number => {
                let t = self.consume();
                pieces.push(Piece::Num(t.text));
            }
            Tk::Name => {
                let t = self.consume();
                pieces.push(Piece::Macro(t.text, t.line));
            }
            _ => {
                self.error_expected(&[Tk::Number, Tk::Name, Tk::Str], "");
                self.resync(FOLLOW_SIMPLE);
            }
        }
    }

    fn value(&mut self) -> Vec<Piece> {
        let mut pieces = Vec::new();
        self.simple_value(&mut pieces);
        while self.lk() == Tk::Hash {
            self.consume();
            self.simple_value(&mut pieces);
        }
        pieces
    }

    fn field(&mut self, fields: &mut Vec<RawField>) {
        // the name is checked before the next token is read (spec section 3.2)
        if self.la().text.first().is_some_and(u8::is_ascii_digit) {
            let msg = format!(
                "invalid field name \"{}\": cannot start with digit",
                String::from_utf8_lossy(&self.la().text)
            );
            let line = self.la().line;
            self.pending
                .push(Diag::new(DiagKind::SyntaxError, line, msg));
            self.ok = false;
        }
        let t = self.consume(); // NAME
        let idx = fields.len();
        fields.push(RawField {
            name: t.text,
            line: t.line,
            pieces: None,
        });
        if self.lk() != Tk::Equals {
            self.error_expected(&[Tk::Equals], "");
            self.resync(FOLLOW_FIELD);
            return;
        }
        self.consume();
        let pieces = self.value();
        fields[idx].pieces = Some(pieces);
    }

    fn fields(&mut self, fields: &mut Vec<RawField>) {
        loop {
            match self.lk() {
                Tk::Name => {
                    self.field(fields);
                    if self.lk() == Tk::Comma {
                        self.consume();
                        continue;
                    }
                    return;
                }
                Tk::EntryClose => return,
                _ => {
                    self.error_expected(&[Tk::Name, Tk::EntryClose], "");
                    self.resync(FOLLOW_CONTENTS);
                    return;
                }
            }
        }
    }

    /// Post-process pieces into a value (spec §3.5). `collapse`: field and
    /// preamble values; macro definitions are not collapsed.
    fn expand(
        &mut self,
        pieces: &[Piece],
        collapse: bool,
        diags: &mut Vec<Diag>,
    ) -> Option<String> {
        let mut out: Vec<u8> = Vec::new();
        let mut defined_any = false;
        for p in pieces {
            match p {
                Piece::Str(t) | Piece::Num(t) => {
                    out.extend_from_slice(t);
                    defined_any = true;
                }
                Piece::Macro(name, line) => {
                    let name = String::from_utf8_lossy(name);
                    match self.st.macros.get(&name) {
                        Some(v) => {
                            out.extend_from_slice(v.as_bytes());
                            defined_any = true;
                        }
                        None => diags.push(Diag::new(
                            DiagKind::Warning,
                            *line,
                            format!("undefined macro \"{name}\""),
                        )),
                    }
                }
            }
        }
        if pieces.is_empty() || (pieces.len() == 1 && !defined_any) {
            return None;
        }
        if collapse {
            out = collapse_spaces(&out);
        }
        Some(String::from_utf8_lossy(&out).into_owned())
    }

    fn parse_entry(&mut self) -> Item {
        self.ok = true;
        let mut diags_before = std::mem::take(&mut self.pending);
        if self.lk() != Tk::At {
            // only reachable after an unusual recovery
            self.error_expected(&[Tk::At], "");
            self.resync(FOLLOW_ENTRY);
            diags_before.append(&mut self.pending);
            return Item::Stop {
                diags: diags_before,
                fatal: false,
            };
        }
        let at_line = self.la().line;
        self.consume();
        if self.lk() != Tk::Name {
            self.error_expected(&[Tk::Name], "");
            self.resync(FOLLOW_ENTRY);
            diags_before.append(&mut self.pending);
            return Item::Stop {
                diags: diags_before,
                fatal: false,
            };
        }
        let ty = self.consume();
        let metatype = metatype_of(&ty.text);
        let entry_type = String::from_utf8_lossy(&ty.text).to_ascii_lowercase();
        let mut key: Option<String> = None;
        let mut fields: Vec<RawField> = Vec::new();
        let mut value_pieces: Option<Vec<Piece>> = None;
        let mut comment: Option<Vec<u8>> = None;

        match self.lk() {
            Tk::Str => {
                if metatype == Metatype::Comment {
                    let t = self.consume();
                    comment = Some(t.text);
                } else if metatype == Metatype::Preamble {
                    // the predicate fails but the string is kept as the value
                    self.pending.push(Diag::new(
                        DiagKind::Bare,
                        0,
                        "semantic error; failed predicate: '   metatype == BTE_COMMENT '",
                    ));
                    let t = self.consume();
                    value_pieces = Some(vec![Piece::Str(t.text)]);
                } else {
                    diags_before.append(&mut self.pending);
                    diags_before.push(Diag::new(
                        DiagKind::Bare,
                        0,
                        "semantic error; failed predicate: '   metatype == BTE_COMMENT '",
                    ));
                    diags_before.push(Diag::new(
                        DiagKind::Bare,
                        0,
                        "fatal error: bt_postprocess_field: invalid AST node (not a field)",
                    ));
                    self.done = true;
                    return Item::Stop {
                        diags: diags_before,
                        fatal: true,
                    };
                }
            }
            Tk::EntryOpen => {
                self.consume();
                // contents
                let la = self.lk();
                match la {
                    Tk::Name | Tk::Number | Tk::EntryClose | Tk::Str => match metatype {
                        Metatype::Regular if matches!(la, Tk::Name | Tk::Number) => {
                            let t = self.consume();
                            key = Some(String::from_utf8_lossy(&t.text).into_owned());
                            if self.lk() == Tk::Comma {
                                self.consume();
                                self.fields(&mut fields);
                            } else {
                                self.error_expected(&[Tk::Comma], "");
                                self.resync(FOLLOW_CONTENTS);
                            }
                        }
                        Metatype::MacroDef if matches!(la, Tk::Name | Tk::EntryClose) => {
                            self.fields(&mut fields);
                        }
                        Metatype::Preamble if matches!(la, Tk::Name | Tk::Number | Tk::Str) => {
                            value_pieces = Some(self.value());
                        }
                        _ => {
                            self.error_predicate();
                            self.resync(FOLLOW_CONTENTS);
                        }
                    },
                    _ => {
                        self.error_expected(&[Tk::Number, Tk::Name, Tk::EntryClose, Tk::Str], "");
                        self.resync(FOLLOW_CONTENTS);
                    }
                }
                if self.lk() == Tk::EntryClose {
                    self.consume();
                } else {
                    self.error_expected(&[Tk::EntryClose], " (skipping to next \"@\")");
                    self.lx.mode = Mode::Top;
                    self.lx.junk = 0;
                    self.resync(FOLLOW_ENTRY);
                }
            }
            _ => {
                self.error_expected(&[Tk::EntryOpen, Tk::Str], "");
                self.resync(FOLLOW_ENTRY);
            }
        }

        // post-processing
        let mut diags = diags_before;
        diags.append(&mut self.pending);
        let parse_ok = self.ok;
        let mut out_fields = Vec::with_capacity(fields.len());
        let mut raw_pieces: Vec<Option<Vec<Piece>>> = Vec::with_capacity(fields.len());
        for f in fields {
            let name = String::from_utf8_lossy(&f.name).to_ascii_lowercase();
            let value = match &f.pieces {
                Some(p) => self.expand(p, metatype != Metatype::MacroDef, &mut diags),
                None => None,
            };
            let pieces = f.pieces;
            // a definition whose value failed to parse defines nothing
            if metatype == Metatype::MacroDef
                && pieces.as_ref().is_some_and(|p| !p.is_empty())
                && self.st.macros.define(&name, value.as_deref().unwrap_or(""))
            {
                diags.push(Diag::new(
                    DiagKind::Warning,
                    f.line,
                    format!("overriding existing definition of macro \"{name}\""),
                ));
            }
            out_fields.push(Field {
                name,
                value,
                line: f.line,
            });
            raw_pieces.push(pieces);
        }
        if metatype == Metatype::MacroDef {
            // The values handed back are expanded again once all definitions
            // of the entry are in place (spec §3.5).
            for (f, p) in out_fields.iter_mut().zip(&raw_pieces) {
                if let Some(p) = p {
                    f.value = self.expand(p, false, &mut diags);
                }
            }
        }
        let value = match metatype {
            Metatype::Preamble => match &value_pieces {
                Some(p) => self.expand(p, true, &mut diags),
                None => None,
            },
            Metatype::Comment => {
                comment.map(|c| String::from_utf8_lossy(&collapse_spaces(&c)).into_owned())
            }
            _ => None,
        };
        // diagnostics printed during post-processing belong to this entry
        Item::Entry(Entry {
            metatype,
            entry_type,
            key,
            fields: out_fields,
            value,
            parse_ok,
            line: at_line,
            diags,
        })
    }
}

impl Parser<'_, '_> {
    /// Diagnostics printed after the last item was returned (e.g. an
    /// unterminated `%` comment at the end of the file).
    pub fn take_pending(&mut self) -> Vec<Diag> {
        std::mem::take(&mut self.pending)
    }
}

impl Iterator for Parser<'_, '_> {
    type Item = Item;

    fn next(&mut self) -> Option<Item> {
        if self.done {
            return None;
        }
        if self.la.is_none() {
            let t = self.lx.next(&mut self.pending);
            let eof = t.kind == Tk::Eof;
            self.la = Some(t);
            if eof {
                // no entry at all: btparse reports the missing "@" and
                // resynchronises (which matters for the next file, §3.4)
                self.error_expected(&[Tk::At], "");
                self.resync(FOLLOW_ENTRY);
            }
        }
        // Text::BibTeX does not parse again once the input stream is
        // exhausted, even if the lookahead was an `@` (spec §3.1)
        if self.lk() == Tk::Eof || (self.lk() == Tk::At && self.lx.pos >= self.lx.src.len()) {
            self.done = true;
            return None;
        }
        let item = self.parse_entry();
        if matches!(item, Item::Stop { .. }) {
            self.done = true;
        }
        Some(item)
    }
}

/// btparse's whitespace collapsing: runs of ASCII spaces become one space,
/// leading and trailing spaces are removed. Other whitespace is untouched.
pub fn collapse_spaces(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut pending_space = false;
    for &b in s {
        if b == b' ' {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(b' ');
                pending_space = false;
            }
            out.push(b);
        }
    }
    out
}

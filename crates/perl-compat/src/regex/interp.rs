//! Perl double-quoted-string (`qq`) interpolation of sourcemap replacement
//! strings, as biber evaluates them: `Safe->new->reval(qq{"$val_replace"})`
//! once per match inside `s{$re}{…}eg` (`Utils.pm:1173-1185`).
//!
//! Supported exactly (verified against the oracle):
//! * captures `$1`…`$n`, `${n}`, `${ n }`, `$ 1`; `$0` is empty in the Safe
//!   compartment; `$01` is a syntax error;
//! * `$&`, `` $` ``, `$'`, `$+` (last matched group), `$^N` (most recently
//!   closed group), `$-[n]`, `$+[n]`, `$#-`, `$#+`, `@-`, `@+` (joined with
//!   an empty `$"`);
//! * every other variable is an empty package variable of the compartment:
//!   `$x`, `$x::y`, `${x}`, `@x`, `@{x}`, `$x[…]`, `$x{…}`, `$1[0]`,
//!   `$x->[…]`, `$#x` (= -1), …; a few special variables have fixed values
//!   (`$$` = process id, `$:` = `" \n-"`, `$^` = `STDOUT_TOP`, `$~` =
//!   `STDOUT`, `$^W` = 0, `$=` = 60, …);
//! * escapes `\t \n \r \f \b \a \e \0 \NNN \o{} \xHH \x{} \cX \N{U+…}`, `\$`,
//!   `\@`, `\\`, `\"`, unknown escapes (`\z` → `z`);
//! * case modifiers `\u \l \U \L \F \Q \E` with perl's rules (implicit `\E`
//!   when `\U`/`\L`/`\F` follow each other with text between, `\L\u` swapped
//!   to `\u\L`, `\u` applying to the next term only).
//!
//! Errors make biber insert the stringified regex (`(?^u:…)`) for the match;
//! [`Template::parse`] reports them as [`InterpError::Syntax`]. Perl *code*
//! (`@{[ … ]}`, `${\ … }`, `${ expr }`, `$-{…}`, `$+{name}`, `\N{NAME}`) is
//! not executed: [`InterpError::Code`] → the same fallback plus a warning
//! (deliberate difference DC-023; perl evaluates the code when it compiles).

use crate::case::{fc, lc, lcfirst, uc, ucfirst};
use crate::unicode::{in_ranges, is_perl_digit, is_perl_word};

/// Why a replacement string could not be interpolated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterpError {
    /// A perl syntax error (biber silently falls back to the regex string).
    Syntax(String),
    /// Perl code that bibers does not execute (DC-023).
    Code(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    L,
    U,
    F,
    Q,
}

#[derive(Debug, Clone, PartialEq)]
enum Var {
    Group(usize),
    Match,
    Pre,
    Post,
    LastParen,
    LastClosed,
    /// `$-[n]` (`true`) / `$+[n]` (`false`)
    Offset(bool, i64),
    /// `$#-` / `$#+`
    LastIndex(bool),
    /// `@-` / `@+`
    OffsetsJoined(bool),
    Pid,
    Const(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Lit(String),
    Var(Var),
    Start(Mode),
    End,
    First(bool),
}

/// A parsed replacement string.
#[derive(Debug, Clone)]
pub struct Template {
    toks: Vec<Tok>,
}

/// The match a replacement is evaluated for (byte offsets into `subject`).
pub struct MatchCtx<'a> {
    /// The whole string being substituted.
    pub subject: &'a str,
    /// Group 0..=n spans (`None` = did not participate).
    pub groups: &'a [Option<(usize, usize)>],
}

impl MatchCtx<'_> {
    fn group(&self, n: usize) -> &str {
        match self.groups.get(n).copied().flatten() {
            Some((s, e)) => &self.subject[s..e],
            None => "",
        }
    }

    fn char_off(&self, b: usize) -> usize {
        self.subject[..b].chars().count()
    }

    fn last_paren(&self) -> usize {
        (1..self.groups.len())
            .rev()
            .find(|&i| self.groups[i].is_some())
            .unwrap_or(0)
    }

    fn last_closed(&self) -> usize {
        let mut best: Option<(usize, usize)> = None; // (end, group)
        for (i, g) in self.groups.iter().enumerate().skip(1) {
            if let Some((_, e)) = g {
                if best.is_none_or(|(be, _)| *e > be) {
                    best = Some((*e, i));
                }
            }
        }
        best.map_or(0, |b| b.1)
    }

    fn eval(&self, v: &Var) -> String {
        match v {
            Var::Group(n) => self.group(*n).to_string(),
            Var::Match => self.group(0).to_string(),
            Var::Pre => self.groups[0]
                .map_or("", |(s, _)| &self.subject[..s])
                .to_string(),
            Var::Post => self.groups[0]
                .map_or("", |(_, e)| &self.subject[e..])
                .to_string(),
            Var::LastParen => {
                let n = self.last_paren();
                if n == 0 {
                    String::new()
                } else {
                    self.group(n).to_string()
                }
            }
            Var::LastClosed => {
                let n = self.last_closed();
                if n == 0 {
                    String::new()
                } else {
                    self.group(n).to_string()
                }
            }
            Var::Offset(start, n) => {
                let len = if *start {
                    self.last_paren() + 1
                } else {
                    self.groups.len()
                };
                let idx = if *n < 0 { len as i64 + n } else { *n };
                if idx < 0 || idx as usize >= len {
                    return String::new();
                }
                match self.groups.get(idx as usize).copied().flatten() {
                    Some((s, e)) => self.char_off(if *start { s } else { e }).to_string(),
                    None => String::new(),
                }
            }
            Var::LastIndex(start) => {
                if *start {
                    self.last_paren().to_string()
                } else {
                    (self.groups.len() - 1).to_string()
                }
            }
            Var::OffsetsJoined(start) => {
                let len = if *start {
                    self.last_paren() + 1
                } else {
                    self.groups.len()
                };
                let mut out = String::new();
                for i in 0..len {
                    if let Some((s, e)) = self.groups[i] {
                        out.push_str(&self.char_off(if *start { s } else { e }).to_string());
                    }
                }
                out
            }
            Var::Pid => std::process::id().to_string(),
            Var::Const(c) => (*c).to_string(),
        }
    }
}

fn syntax<T>(m: &str) -> Result<T, InterpError> {
    Err(InterpError::Syntax(m.to_string()))
}

fn code<T>(m: &str) -> Result<T, InterpError> {
    Err(InterpError::Code(m.to_string()))
}

/// Skip a balanced `[...]`/`{...}` starting at `chars[i]`; returns the index
/// after it.
fn skip_balanced(chars: &[char], i: usize) -> Result<usize, InterpError> {
    let (open, close) = match chars[i] {
        '[' => ('[', ']'),
        _ => ('{', '}'),
    };
    let mut depth = 0;
    let mut j = i;
    while j < chars.len() {
        let c = chars[j];
        if c == '\\' {
            j += 2;
            continue;
        }
        if c == '"' {
            return syntax("quote inside subscript");
        }
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Ok(j + 1);
            }
        }
        j += 1;
    }
    syntax("unterminated subscript")
}

/// Consume subscripts (`[..]`, `{..}`, `->[..]`, `->{..}`) after a variable;
/// returns the new index and whether any were consumed.
fn skip_subscripts(chars: &[char], mut i: usize) -> Result<(usize, bool), InterpError> {
    let mut any = false;
    loop {
        match chars.get(i) {
            Some('[' | '{') => {
                i = skip_balanced(chars, i)?;
                any = true;
            }
            Some('-')
                if chars.get(i + 1) == Some(&'>')
                    && matches!(chars.get(i + 2), Some('[' | '{')) =>
            {
                i = skip_balanced(chars, i + 2)?;
                any = true;
            }
            _ => return Ok((i, any)),
        }
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

/// Length (in chars) of an identifier with optional `::` package parts.
fn ident_len(chars: &[char], i: usize) -> usize {
    let mut j = i;
    loop {
        if j < chars.len() && (is_ident_start(chars[j]) || (j > i && is_perl_word(chars[j]))) {
            j += 1;
            while j < chars.len() && is_perl_word(chars[j]) {
                j += 1;
            }
        }
        if chars.get(j) == Some(&':') && chars.get(j + 1) == Some(&':') {
            j += 2;
            continue;
        }
        return j - i;
    }
}

/// Value of a single-character punctuation variable in biber's Safe
/// compartment.
fn punct_var(c: char) -> Option<Var> {
    Some(match c {
        '&' => Var::Match,
        '`' => Var::Pre,
        '\'' => Var::Post,
        '+' => Var::LastParen,
        '$' => Var::Pid,
        ':' => Var::Const(" \n-"),
        '~' => Var::Const("STDOUT"),
        '^' => Var::Const("STDOUT_TOP"),
        '=' => Var::Const("60"),
        '-' | '%' | '|' | '?' | '.' | '[' => Var::Const("0"),
        ';' => Var::Const("\u{1c}"),
        ']' => Var::Const("5.042000"),
        '!' | '/' | ',' | '0' | '_' | '@' | '<' | '>' | '(' | ')' => Var::Const(""),
        _ => return None,
    })
}

impl Template {
    /// Parse a replacement string.
    pub fn parse(src: &str) -> Result<Template, InterpError> {
        let chars: Vec<char> = src.chars().collect();
        let mut toks: Vec<Tok> = Vec::new();
        let mut lit = String::new();
        let mut i = 0;
        let flush = |lit: &mut String, toks: &mut Vec<Tok>| {
            if !lit.is_empty() {
                toks.push(Tok::Lit(std::mem::take(lit)));
            }
        };
        while i < chars.len() {
            let c = chars[i];
            match c {
                '"' => return syntax("unescaped double quote"),
                '\\' => {
                    let Some(&n) = chars.get(i + 1) else {
                        return syntax("trailing backslash");
                    };
                    i += 2;
                    match n {
                        'u' | 'l' => {
                            flush(&mut lit, &mut toks);
                            toks.push(Tok::First(n == 'u'));
                        }
                        'U' | 'L' | 'F' | 'Q' => {
                            flush(&mut lit, &mut toks);
                            let m = match n {
                                'U' => Mode::U,
                                'L' => Mode::L,
                                'F' => Mode::F,
                                _ => Mode::Q,
                            };
                            if m != Mode::Q {
                                if matches!(
                                    toks.last(),
                                    Some(Tok::Start(Mode::L | Mode::U | Mode::F))
                                ) {
                                    return syntax("adjacent case modifiers");
                                }
                                // \L\u → \u\L
                                if chars.get(i) == Some(&'\\')
                                    && matches!(chars.get(i + 1), Some('u' | 'l'))
                                {
                                    toks.push(Tok::First(chars[i + 1] == 'u'));
                                    i += 2;
                                }
                            }
                            toks.push(Tok::Start(m));
                        }
                        'E' => {
                            flush(&mut lit, &mut toks);
                            toks.push(Tok::End);
                        }
                        't' => lit.push('\t'),
                        'n' => lit.push('\n'),
                        'r' => lit.push('\r'),
                        'f' => lit.push('\x0C'),
                        'b' => lit.push('\x08'),
                        'a' => lit.push('\x07'),
                        'e' => lit.push('\x1B'),
                        '0'..='7' => {
                            let mut v = n.to_digit(8).unwrap_or(0);
                            let mut k = 0;
                            while k < 2 && i < chars.len() && ('0'..='7').contains(&chars[i]) {
                                v = v * 8 + chars[i].to_digit(8).unwrap_or(0);
                                i += 1;
                                k += 1;
                            }
                            lit.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
                        }
                        'o' if chars.get(i) == Some(&'{') => {
                            let end = chars[i..]
                                .iter()
                                .position(|&c| c == '}')
                                .ok_or(InterpError::Syntax("\\o{".into()))?;
                            let body: String = chars[i + 1..i + end]
                                .iter()
                                .filter(|c| !c.is_whitespace())
                                .collect();
                            if body.is_empty() {
                                return syntax("empty \\o{}");
                            }
                            let digits: String = body
                                .chars()
                                .take_while(|c| ('0'..='7').contains(c))
                                .collect();
                            let v = u32::from_str_radix(
                                if digits.is_empty() { "0" } else { &digits },
                                8,
                            )
                            .unwrap_or(0);
                            lit.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
                            i += end + 1;
                        }
                        'x' => {
                            if chars.get(i) == Some(&'{') {
                                let end = chars[i..]
                                    .iter()
                                    .position(|&c| c == '}')
                                    .ok_or(InterpError::Syntax("\\x{".into()))?;
                                let body: String = chars[i + 1..i + end]
                                    .iter()
                                    .filter(|c| !c.is_whitespace())
                                    .collect();
                                let digits: String = body
                                    .chars()
                                    .filter(|&c| c != '_')
                                    .take_while(char::is_ascii_hexdigit)
                                    .collect();
                                let v = u32::from_str_radix(
                                    if digits.is_empty() { "0" } else { &digits },
                                    16,
                                )
                                .unwrap_or(0);
                                lit.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
                                i += end + 1;
                            } else {
                                let mut v = 0;
                                let mut k = 0;
                                while k < 2 && i < chars.len() && chars[i].is_ascii_hexdigit() {
                                    v = v * 16 + chars[i].to_digit(16).unwrap_or(0);
                                    i += 1;
                                    k += 1;
                                }
                                lit.push(char::from_u32(v).unwrap_or('\0'));
                            }
                        }
                        'c' => {
                            let Some(&x) = chars.get(i) else {
                                return syntax("\\c at end");
                            };
                            i += 1;
                            let u = x.to_ascii_uppercase() as u32;
                            lit.push(char::from_u32(u ^ 64).unwrap_or('\0'));
                        }
                        'N' => {
                            if chars.get(i) != Some(&'{') {
                                return syntax("\\N without braces");
                            }
                            let end = chars[i..]
                                .iter()
                                .position(|&c| c == '}')
                                .ok_or(InterpError::Syntax("\\N{".into()))?;
                            let body: String = chars[i + 1..i + end].iter().collect();
                            let body = body.trim();
                            match body.strip_prefix("U+") {
                                Some(h)
                                    if !h.is_empty()
                                        && h.chars().all(|c| c.is_ascii_hexdigit()) =>
                                {
                                    let v = u32::from_str_radix(h, 16).unwrap_or(0);
                                    lit.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
                                }
                                _ => return code("\\N{NAME} needs charnames"),
                            }
                            i += end + 1;
                        }
                        other => lit.push(other),
                    }
                }
                '$' => {
                    flush(&mut lit, &mut toks);
                    let (tok, ni) = Self::parse_scalar(&chars, i + 1)?;
                    toks.push(tok);
                    i = ni;
                }
                '@' => {
                    let n = chars.get(i + 1).copied();
                    match n {
                        Some('-' | '+') => {
                            flush(&mut lit, &mut toks);
                            toks.push(Tok::Var(Var::OffsetsJoined(n == Some('-'))));
                            i += 2;
                        }
                        Some('{') => {
                            let mut j = i + 2;
                            while chars.get(j).is_some_and(|c| c.is_whitespace()) {
                                j += 1;
                            }
                            let k = ident_len(&chars, j);
                            let mut e = j + k;
                            while chars.get(e).is_some_and(|c| c.is_whitespace()) {
                                e += 1;
                            }
                            if k > 0 && chars.get(e) == Some(&'}') {
                                i = e + 1;
                            } else if chars[j..].iter().find(|c| !c.is_whitespace()) == Some(&'[') {
                                return code("@{[ … ]}");
                            } else {
                                return code("@{ … }");
                            }
                        }
                        Some('$') => {
                            let k = ident_len(&chars, i + 2);
                            if k == 0 {
                                lit.push('@');
                                i += 1;
                            } else {
                                i = skip_subscripts(&chars, i + 2 + k)?.0;
                            }
                        }
                        Some(x) if is_ident_start(x) || x.is_ascii_digit() || x == ':' => {
                            let k = if x.is_ascii_digit() {
                                chars[i + 1..]
                                    .iter()
                                    .take_while(|c| c.is_ascii_digit())
                                    .count()
                            } else {
                                ident_len(&chars, i + 1)
                            };
                            if k == 0 {
                                lit.push('@');
                                i += 1;
                            } else {
                                i = skip_subscripts(&chars, i + 1 + k)?.0;
                            }
                        }
                        _ => {
                            lit.push('@');
                            i += 1;
                        }
                    }
                }
                _ => {
                    lit.push(c);
                    i += 1;
                }
            }
        }
        flush(&mut lit, &mut toks);
        Ok(Template { toks })
    }

    /// Parse after a `$` at `chars[i]`.
    fn parse_scalar(chars: &[char], mut i: usize) -> Result<(Tok, usize), InterpError> {
        // whitespace is allowed between `$` and a name
        let ws = chars[i.min(chars.len())..]
            .iter()
            .take_while(|c| **c == ' ' || **c == '\t')
            .count();
        if ws > 0
            && chars
                .get(i + ws)
                .is_some_and(|&c| c.is_ascii_digit() || is_ident_start(c))
        {
            i += ws;
        }
        let Some(&c) = chars.get(i) else {
            return syntax("$ at end of string");
        };
        if c.is_ascii_digit() {
            let k = chars[i..].iter().take_while(|c| c.is_ascii_digit()).count();
            if c == '0' && k > 1 {
                return syntax("$0 followed by digits");
            }
            let n: usize = chars[i..i + k]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(usize::MAX);
            let (ni, sub) = skip_subscripts(chars, i + k)?;
            let tok = if sub || n == 0 {
                Tok::Var(Var::Const(""))
            } else {
                Tok::Var(Var::Group(n))
            };
            return Ok((tok, ni));
        }
        if is_perl_digit(c) {
            return syntax("non-ASCII digit variable");
        }
        if c == '{' {
            let mut j = i + 1;
            while chars.get(j).is_some_and(|c| c.is_whitespace()) {
                j += 1;
            }
            let k = chars[j..].iter().take_while(|c| c.is_ascii_digit()).count();
            if k > 0 {
                let mut e = j + k;
                while chars.get(e).is_some_and(|c| c.is_whitespace()) {
                    e += 1;
                }
                if chars.get(e) == Some(&'}') {
                    let n: usize = chars[j..j + k]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .unwrap_or(usize::MAX);
                    let tok = if n == 0 {
                        Tok::Var(Var::Const(""))
                    } else {
                        Tok::Var(Var::Group(n))
                    };
                    return Ok((tok, e + 1));
                }
            }
            if chars.get(j) == Some(&'^') {
                let k = chars[j + 1..]
                    .iter()
                    .take_while(|c| is_perl_word(**c))
                    .count();
                if chars.get(j + 1 + k) == Some(&'}') {
                    let name: String = chars[j + 1..j + 1 + k].iter().collect();
                    let tok = match name.as_str() {
                        "N" => Tok::Var(Var::LastClosed),
                        "W" => Tok::Var(Var::Const("0")),
                        _ => Tok::Var(Var::Const("")),
                    };
                    return Ok((tok, j + 2 + k));
                }
            }
            let k = ident_len(chars, j);
            if k > 0 {
                let mut e = j + k;
                while chars.get(e).is_some_and(|c| c.is_whitespace()) {
                    e += 1;
                }
                if chars.get(e) == Some(&'}') {
                    return Ok((Tok::Var(Var::Const("")), e + 1));
                }
            }
            if chars.get(i + 1..).is_some_and(|r| !r.contains(&'}')) {
                return syntax("unterminated ${");
            }
            return code("${ … } expression");
        }
        if c == '#' {
            match chars.get(i + 1) {
                Some('-') => return Ok((Tok::Var(Var::LastIndex(true)), i + 2)),
                Some('+') => return Ok((Tok::Var(Var::LastIndex(false)), i + 2)),
                Some('{') => {
                    let k = ident_len(chars, i + 2);
                    if k > 0 && chars.get(i + 2 + k) == Some(&'}') {
                        return Ok((Tok::Var(Var::Const("-1")), i + 3 + k));
                    }
                    return code("$#{ … }");
                }
                Some(&x) if is_ident_start(x) || x == '$' => {
                    let st = if x == '$' { i + 2 } else { i + 1 };
                    let k = ident_len(chars, st);
                    return Ok((Tok::Var(Var::Const("-1")), st + k));
                }
                _ => return syntax("$#"),
            }
        }
        if c == '^' {
            if let Some(&x) = chars.get(i + 1) {
                if x.is_ascii_uppercase() || matches!(x, '[' | ']' | '^' | '_' | '?' | '\\') {
                    let tok = match x {
                        'N' => Tok::Var(Var::LastClosed),
                        'W' => Tok::Var(Var::Const("0")),
                        _ => Tok::Var(Var::Const("")),
                    };
                    return Ok((tok, i + 2));
                }
            }
            return Ok((Tok::Var(Var::Const("STDOUT_TOP")), i + 1));
        }
        if c == ':' && chars.get(i + 1) == Some(&':') {
            let k = ident_len(chars, i);
            let (ni, _) = skip_subscripts(chars, i + k)?;
            return Ok((Tok::Var(Var::Const("")), ni));
        }
        if is_ident_start(c) {
            let k = ident_len(chars, i);
            let (ni, _) = skip_subscripts(chars, i + k)?;
            return Ok((Tok::Var(Var::Const("")), ni));
        }
        if c == '$'
            && chars
                .get(i + 1)
                .is_some_and(|&x| is_ident_start(x) || x == '{' || x.is_ascii_digit())
        {
            // $$name / $$1: deref of a non-reference
            let k = if chars[i + 1].is_ascii_digit() {
                chars[i + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_digit())
                    .count()
            } else {
                ident_len(chars, i + 1)
            };
            if k == 0 {
                return code("$${ … }");
            }
            let (ni, _) = skip_subscripts(chars, i + 1 + k)?;
            return Ok((Tok::Var(Var::Const("")), ni));
        }
        if c == '"' {
            return syntax("$\" in string");
        }
        if matches!(c, '-' | '+') && chars.get(i + 1) == Some(&'[') {
            // $-[n] / $+[n]
            let end = chars[i + 1..]
                .iter()
                .position(|&c| c == ']')
                .ok_or(InterpError::Syntax("$-[".into()))?;
            let body: String = chars[i + 2..i + 1 + end].iter().collect();
            let body = body.trim();
            return match body.parse::<i64>() {
                Ok(n) => Ok((Tok::Var(Var::Offset(c == '-', n)), i + 2 + end)),
                Err(_) => code("$-[ expr ]"),
            };
        }
        if matches!(c, '-' | '+') && chars.get(i + 1) == Some(&'{') {
            return code("%- / %+ (named captures)");
        }
        match punct_var(c) {
            Some(v) => {
                let (ni, sub) = skip_subscripts(chars, i + 1)?;
                Ok((Tok::Var(if sub { Var::Const("") } else { v }), ni))
            }
            None => syntax("bad variable"),
        }
    }

    /// Evaluate for one match.
    pub fn eval(&self, m: &MatchCtx<'_>) -> String {
        struct Frame {
            mode: Option<Mode>,
            first: Option<bool>,
            buf: String,
            is_first: bool,
        }
        fn apply(mode: Option<Mode>, s: String) -> String {
            match mode {
                None => s,
                Some(Mode::L) => lc(&s),
                Some(Mode::U) => uc(&s),
                Some(Mode::F) => fc(&s),
                Some(Mode::Q) => quotemeta(&s),
            }
        }
        fn close(stack: &mut Vec<Frame>) {
            let f = stack.pop().unwrap_or(Frame {
                mode: None,
                first: None,
                buf: String::new(),
                is_first: false,
            });
            let mut s = apply(f.mode, f.buf);
            if let Some(up) = f.first {
                s = if up { ucfirst(&s) } else { lcfirst(&s) };
            }
            if let Some(top) = stack.last_mut() {
                top.buf.push_str(&s);
            }
        }
        fn close_first(stack: &mut Vec<Frame>) {
            if stack.len() > 1 && stack.last().is_some_and(|f| f.is_first) {
                close(stack);
            }
        }
        let mut stack = vec![Frame {
            mode: None,
            first: None,
            buf: String::new(),
            is_first: false,
        }];
        let mut pending: Option<bool> = None;
        for t in &self.toks {
            match t {
                Tok::Lit(_) | Tok::Var(_) => {
                    let s = match t {
                        Tok::Lit(l) => l.clone(),
                        Tok::Var(v) => m.eval(v),
                        _ => String::new(),
                    };
                    // `\u` applies ucfirst to everything up to the next case
                    // modifier (`"\u$2x"` is `ucfirst($2 . "x")`).
                    if let Some(up) = pending.take() {
                        stack.push(Frame {
                            mode: None,
                            first: Some(up),
                            buf: String::new(),
                            is_first: true,
                        });
                    }
                    if let Some(top) = stack.last_mut() {
                        top.buf.push_str(&s);
                    }
                }
                Tok::First(up) => {
                    close_first(&mut stack);
                    pending = Some(*up);
                }
                Tok::Start(mode) => {
                    close_first(&mut stack);
                    if *mode != Mode::Q
                        && stack.len() > 1
                        && matches!(
                            stack.last().and_then(|f| f.mode),
                            Some(Mode::L | Mode::U | Mode::F)
                        )
                    {
                        close(&mut stack);
                    }
                    stack.push(Frame {
                        mode: Some(*mode),
                        first: pending.take(),
                        buf: String::new(),
                        is_first: false,
                    });
                }
                Tok::End => {
                    close_first(&mut stack);
                    if stack.len() > 1 {
                        close(&mut stack);
                    }
                }
            }
        }
        while stack.len() > 1 {
            close(&mut stack);
        }
        stack.pop().map(|f| f.buf).unwrap_or_default()
    }
}

/// Perl `quotemeta` on a character string.
pub fn quotemeta(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if in_ranges(crate::tables::props::QUOTEMETA, c as u32) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

//! XML Schema regular expressions (*XML Schema Part 2*, appendix F),
//! translated into the syntax of the `regex` crate.
//!
//! XSD regexes are implicitly anchored, have no `^`/`$` anchors (both are
//! ordinary characters), and add the `\i`/`\c` name-character escapes,
//! `\p{IsBlock}` block escapes and character-class subtraction
//! (`[a-z-[aeiou]]`). Everything is mapped onto Unicode-aware `regex`
//! constructs; class members are emitted as `\x{…}` escapes.

use regex::Regex;

/// Compile an XSD pattern into an anchored `regex::Regex`.
pub(crate) fn compile(pattern: &str) -> Result<Regex, String> {
    let mut p = Parser {
        chars: pattern.chars().collect(),
        pos: 0,
    };
    let body = p.reg_exp()?;
    if p.pos != p.chars.len() {
        return Err(format!(
            "invalid regular expression '{pattern}': unexpected '{}'",
            p.chars[p.pos]
        ));
    }
    Regex::new(&format!("^(?:{body})$"))
        .map_err(|e| format!("invalid regular expression '{pattern}': {e}"))
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

fn lit(c: char) -> String {
    format!("\\x{{{:X}}}", c as u32)
}

/// XML 1.0 (fifth edition) NameStartChar ranges.
const NAME_START: &[(u32, u32)] = &[
    (0x3A, 0x3A),
    (0x41, 0x5A),
    (0x5F, 0x5F),
    (0x61, 0x7A),
    (0xC0, 0xD6),
    (0xD8, 0xF6),
    (0xF8, 0x2FF),
    (0x370, 0x37D),
    (0x37F, 0x1FFF),
    (0x200C, 0x200D),
    (0x2070, 0x218F),
    (0x2C00, 0x2FEF),
    (0x3001, 0xD7FF),
    (0xF900, 0xFDCF),
    (0xFDF0, 0xFFFD),
    (0x10000, 0xEFFFF),
];

/// The extra NameChar ranges.
const NAME_EXTRA: &[(u32, u32)] = &[
    (0x2D, 0x2E),
    (0x30, 0x39),
    (0xB7, 0xB7),
    (0x300, 0x36F),
    (0x203F, 0x2040),
];

fn ranges(rs: &[&[(u32, u32)]]) -> String {
    let mut s = String::new();
    for r in rs {
        for &(a, b) in *r {
            s.push_str(&format!("\\x{{{a:X}}}-\\x{{{b:X}}}"));
        }
    }
    s
}

/// Unicode blocks for `\p{IsX}` (the commonly used ones).
const BLOCKS: &[(&str, u32, u32)] = &[
    ("BasicLatin", 0x0000, 0x007F),
    ("Latin-1Supplement", 0x0080, 0x00FF),
    ("LatinExtended-A", 0x0100, 0x017F),
    ("LatinExtended-B", 0x0180, 0x024F),
    ("IPAExtensions", 0x0250, 0x02AF),
    ("SpacingModifierLetters", 0x02B0, 0x02FF),
    ("CombiningDiacriticalMarks", 0x0300, 0x036F),
    ("Greek", 0x0370, 0x03FF),
    ("GreekandCoptic", 0x0370, 0x03FF),
    ("Cyrillic", 0x0400, 0x04FF),
    ("Armenian", 0x0530, 0x058F),
    ("Hebrew", 0x0590, 0x05FF),
    ("Arabic", 0x0600, 0x06FF),
    ("Devanagari", 0x0900, 0x097F),
    ("Thai", 0x0E00, 0x0E7F),
    ("LatinExtendedAdditional", 0x1E00, 0x1EFF),
    ("GreekExtended", 0x1F00, 0x1FFF),
    ("GeneralPunctuation", 0x2000, 0x206F),
    ("SuperscriptsandSubscripts", 0x2070, 0x209F),
    ("CurrencySymbols", 0x20A0, 0x20CF),
    ("LetterlikeSymbols", 0x2100, 0x214F),
    ("NumberForms", 0x2150, 0x218F),
    ("Arrows", 0x2190, 0x21FF),
    ("MathematicalOperators", 0x2200, 0x22FF),
    ("BoxDrawing", 0x2500, 0x257F),
    ("GeometricShapes", 0x25A0, 0x25FF),
    ("MiscellaneousSymbols", 0x2600, 0x26FF),
    ("CJKSymbolsandPunctuation", 0x3000, 0x303F),
    ("Hiragana", 0x3040, 0x309F),
    ("Katakana", 0x30A0, 0x30FF),
    ("CJKUnifiedIdeographs", 0x4E00, 0x9FFF),
    ("HangulSyllables", 0xAC00, 0xD7AF),
    ("PrivateUse", 0xE000, 0xF8FF),
    ("AlphabeticPresentationForms", 0xFB00, 0xFB4F),
    ("HalfwidthandFullwidthForms", 0xFF00, 0xFFEF),
    ("Specials", 0xFFF0, 0xFFFF),
];

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn err<T>(&self, msg: &str) -> Result<T, String> {
        let s: String = self.chars.iter().collect();
        Err(format!("invalid regular expression '{s}': {msg}"))
    }

    fn reg_exp(&mut self) -> Result<String, String> {
        let mut out = self.branch()?;
        while self.eat('|') {
            out.push('|');
            out.push_str(&self.branch()?);
        }
        Ok(out)
    }

    fn branch(&mut self) -> Result<String, String> {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            out.push_str(&atom);
            out.push_str(&self.quantifier()?);
        }
        Ok(out)
    }

    fn quantifier(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(c @ ('?' | '*' | '+')) => {
                self.pos += 1;
                Ok(c.to_string())
            }
            Some('{') => {
                self.pos += 1;
                let start = self.pos;
                while self.peek().is_some_and(|c| c != '}') {
                    self.pos += 1;
                }
                let q: String = self.chars[start..self.pos].iter().collect();
                if !self.eat('}') {
                    return self.err("unterminated quantifier");
                }
                let valid = match q.split_once(',') {
                    None => !q.is_empty() && q.bytes().all(|b| b.is_ascii_digit()),
                    Some((a, b)) => {
                        !a.is_empty()
                            && a.bytes().all(|x| x.is_ascii_digit())
                            && b.bytes().all(|x| x.is_ascii_digit())
                            && (b.is_empty() || a.parse::<u64>().ok() <= b.parse::<u64>().ok())
                    }
                };
                if !valid {
                    return self.err("invalid quantifier");
                }
                Ok(format!("{{{q}}}"))
            }
            _ => Ok(String::new()),
        }
    }

    fn atom(&mut self) -> Result<String, String> {
        let c = self.peek().expect("caller checked");
        match c {
            '(' => {
                self.pos += 1;
                let inner = self.reg_exp()?;
                if !self.eat(')') {
                    return self.err("unbalanced parenthesis");
                }
                Ok(format!("(?:{inner})"))
            }
            '[' => self.class_expr(),
            '.' => {
                self.pos += 1;
                Ok("[^\\n\\r]".into())
            }
            '\\' => self.escape(false),
            '?' | '*' | '+' | '{' | '}' | ']' => self.err(&format!("unexpected '{c}'")),
            _ => {
                self.pos += 1;
                Ok(lit(c))
            }
        }
    }

    /// An escape at `\`. Outside a class returns a pattern; inside
    /// (`in_class`) returns class items.
    fn escape(&mut self, in_class: bool) -> Result<String, String> {
        self.pos += 1;
        let Some(c) = self.peek() else {
            return self.err("trailing backslash");
        };
        self.pos += 1;
        let wrap = |items: String| {
            if in_class {
                items
            } else {
                format!("[{items}]")
            }
        };
        Ok(match c {
            'n' => wrap(lit('\n')),
            'r' => wrap(lit('\r')),
            't' => wrap(lit('\t')),
            '\\' | '|' | '.' | '?' | '*' | '+' | '(' | ')' | '{' | '}' | '-' | '[' | ']' | '^' => {
                wrap(lit(c))
            }
            's' => wrap("\\x{20}\\x{9}\\x{A}\\x{D}".into()),
            'S' => wrap("[^\\x{20}\\x{9}\\x{A}\\x{D}]".into()),
            'd' => wrap("\\p{Nd}".into()),
            'D' => wrap("\\P{Nd}".into()),
            'w' => wrap("[^\\p{P}\\p{Z}\\p{C}]".into()),
            'W' => wrap("[\\p{P}\\p{Z}\\p{C}]".into()),
            'i' => wrap(format!("[{}]", ranges(&[NAME_START]))),
            'I' => wrap(format!("[^{}]", ranges(&[NAME_START]))),
            'c' => wrap(format!("[{}]", ranges(&[NAME_START, NAME_EXTRA]))),
            'C' => wrap(format!("[^{}]", ranges(&[NAME_START, NAME_EXTRA]))),
            'p' | 'P' => {
                if !self.eat('{') {
                    return self.err("expected '{' after \\p");
                }
                let start = self.pos;
                while self.peek().is_some_and(|c| c != '}') {
                    self.pos += 1;
                }
                let prop: String = self.chars[start..self.pos].iter().collect();
                if !self.eat('}') {
                    return self.err("unterminated \\p{");
                }
                let neg = c == 'P';
                let items = if let Some(block) = prop.strip_prefix("Is") {
                    let Some(&(_, a, b)) = BLOCKS.iter().find(|(n, _, _)| *n == block) else {
                        return self.err(&format!("unsupported block '{prop}'"));
                    };
                    format!("[{}\\x{{{a:X}}}-\\x{{{b:X}}}]", if neg { "^" } else { "" })
                } else {
                    const CATS: &[&str] = &[
                        "L", "Lu", "Ll", "Lt", "Lm", "Lo", "M", "Mn", "Mc", "Me", "N", "Nd", "Nl",
                        "No", "P", "Pc", "Pd", "Ps", "Pe", "Pi", "Pf", "Po", "Z", "Zs", "Zl", "Zp",
                        "S", "Sm", "Sc", "Sk", "So", "C", "Cc", "Cf", "Co", "Cn",
                    ];
                    if !CATS.contains(&prop.as_str()) {
                        return self.err(&format!("unknown property '{prop}'"));
                    }
                    format!("\\{}{{{prop}}}", if neg { 'P' } else { 'p' })
                };
                wrap(items)
            }
            _ => return self.err(&format!("invalid escape '\\{c}'")),
        })
    }

    /// `[` charGroup `]`, returned as a bracketed class.
    fn class_expr(&mut self) -> Result<String, String> {
        self.pos += 1; // '['
        let neg = self.peek() == Some('^') && {
            self.pos += 1;
            true
        };
        let mut items = String::new();
        let mut first = true;
        loop {
            let Some(c) = self.peek() else {
                return self.err("unterminated character class");
            };
            match c {
                ']' if !first => {
                    self.pos += 1;
                    break;
                }
                '-' if self.peek2() == Some('[') && !first => {
                    self.pos += 1;
                    let sub = self.class_expr()?;
                    if !self.eat(']') {
                        return self.err("expected ']' after class subtraction");
                    }
                    let base = format!("[{}{items}]", if neg { "^" } else { "" });
                    return Ok(format!("[{base}&&[^{sub}]]"));
                }
                '[' => return self.err("unescaped '[' in character class"),
                _ => {}
            }
            first = false;
            // One charRange or charClassEsc.
            let (lo, is_char) = if c == '\\' {
                let save = self.pos;
                let next = self.peek2();
                match next {
                    Some(
                        'n' | 'r' | 't' | '\\' | '|' | '.' | '?' | '*' | '+' | '(' | ')' | '{'
                        | '}' | '-' | '[' | ']' | '^',
                    ) => {
                        self.pos += 2;
                        let ch = match next.unwrap() {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            other => other,
                        };
                        (Some(ch), true)
                    }
                    _ => {
                        self.pos = save;
                        items.push_str(&self.escape(true)?);
                        (None, false)
                    }
                }
            } else {
                self.pos += 1;
                (Some(c), true)
            };
            if !is_char {
                continue;
            }
            let lo = lo.unwrap();
            if self.peek() == Some('-') && !matches!(self.peek2(), Some('[' | ']') | None) {
                self.pos += 1;
                let hi = match self.peek() {
                    Some('\\') => {
                        let e = self.peek2();
                        self.pos += 2;
                        match e {
                            Some('n') => '\n',
                            Some('r') => '\r',
                            Some('t') => '\t',
                            Some(
                                ch @ ('\\' | '|' | '.' | '?' | '*' | '+' | '(' | ')' | '{' | '}'
                                | '-' | '[' | ']' | '^'),
                            ) => ch,
                            _ => return self.err("invalid range end"),
                        }
                    }
                    Some(h) => {
                        self.pos += 1;
                        h
                    }
                    None => return self.err("unterminated range"),
                };
                if hi < lo {
                    return self.err("range out of order");
                }
                items.push_str(&format!("{}-{}", lit(lo), lit(hi)));
            } else {
                items.push_str(&lit(lo));
            }
        }
        Ok(format!("[{}{items}]", if neg { "^" } else { "" }))
    }
}

#[cfg(test)]
mod tests {
    use super::compile;

    fn m(p: &str, s: &str) -> bool {
        compile(p).unwrap().is_match(s)
    }

    #[test]
    fn basics() {
        assert!(m(r"\d+t?", "12t"));
        assert!(!m(r"\d+t?", "12tt"));
        assert!(m("use.*", "useabc"));
        assert!(!m("use.*", "xuse"));
        assert!(m(r"\S+", "abc") && !m(r"\S+", "a b"));
        assert!(m("a^b$", "a^b$"));
        assert!(m("(ab|cd){2}", "abcd") && !m("(ab|cd){2}", "ab"));
        assert!(m("[a-z-[aeiou]]+", "xyz") && !m("[a-z-[aeiou]]+", "xa"));
        assert!(m("[^a-c]", "d") && !m("[^a-c]", "a"));
        assert!(m(r"[\d-]+", "1-2"));
        assert!(m(r"\i\c*", "_a1") && !m(r"\i\c*", "1a"));
        assert!(m(r"\p{Lu}\p{Ll}*", "Abc") && !m(r"\p{Lu}", "a"));
        assert!(m(r"\p{IsBasicLatin}+", "abc") && !m(r"\p{IsBasicLatin}", "é"));
        assert!(m(".", "x") && !m(".", "\n"));
        assert!(m(r"[\-\[\]]+", "-[]"));
    }

    #[test]
    fn errors() {
        for p in ["(", "a{2,1}", "[a", "\\q", "*a", "a{,2}", r"\p{Foo}"] {
            assert!(compile(p).is_err(), "{p}");
        }
    }
}

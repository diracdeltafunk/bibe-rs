//! `Encode::find_encoding` name resolution: `Encode::getEncoding` and
//! `Encode::Alias::find_alias` (Encode 3.21, Alias.pm 2.25), plus biber's own
//! `define_alias` calls (Constants.pm:364-376).
//!
//! The alias list is searched newest first; each rule's result is resolved
//! again with `find_encoding` (with Perl's recursion guard), and the last
//! resort is a case-insensitive match against every encoding name Encode
//! knows (`keys %Encoding, keys %ExtModule`).

use std::sync::LazyLock;

use regex::Regex;

/// Every encoding name `Encode` knows in biber 2.22 (`Encode->encodings(":all")`
/// in oracle O3; Encode::HanExtra and Encode::JIS2K are not loaded by biber, so
/// `gb18030`, `euc-jisx0213` etc. are unknown).
pub(crate) const KNOWN: &[&str] = &[
    "7bit-jis",
    "AdobeStandardEncoding",
    "AdobeSymbol",
    "AdobeZdingbat",
    "ascii",
    "ascii-ctrl",
    "big5-eten",
    "big5-hkscs",
    "cp1006",
    "cp1026",
    "cp1047",
    "cp1250",
    "cp1251",
    "cp1252",
    "cp1253",
    "cp1254",
    "cp1255",
    "cp1256",
    "cp1257",
    "cp1258",
    "cp37",
    "cp424",
    "cp437",
    "cp500",
    "cp737",
    "cp775",
    "cp850",
    "cp852",
    "cp855",
    "cp856",
    "cp857",
    "cp858",
    "cp860",
    "cp861",
    "cp862",
    "cp863",
    "cp864",
    "cp865",
    "cp866",
    "cp869",
    "cp874",
    "cp875",
    "cp932",
    "cp936",
    "cp949",
    "cp950",
    "dingbats",
    "euc-cn",
    "euc-jp",
    "euc-kr",
    "gb12345-raw",
    "gb2312-raw",
    "gsm0338",
    "hp-roman8",
    "hz",
    "iso-2022-jp",
    "iso-2022-jp-1",
    "iso-2022-kr",
    "iso-8859-1",
    "iso-8859-10",
    "iso-8859-11",
    "iso-8859-13",
    "iso-8859-14",
    "iso-8859-15",
    "iso-8859-16",
    "iso-8859-2",
    "iso-8859-3",
    "iso-8859-4",
    "iso-8859-5",
    "iso-8859-6",
    "iso-8859-7",
    "iso-8859-8",
    "iso-8859-9",
    "iso-ir-165",
    "jis0201-raw",
    "jis0208-raw",
    "jis0212-raw",
    "johab",
    "koi8-f",
    "koi8-r",
    "koi8-u",
    "ksc5601-raw",
    "MacArabic",
    "MacCentralEurRoman",
    "MacChineseSimp",
    "MacChineseTrad",
    "MacCroatian",
    "MacCyrillic",
    "MacDingbats",
    "MacFarsi",
    "MacGreek",
    "MacHebrew",
    "MacIcelandic",
    "MacJapanese",
    "MacKorean",
    "MacRoman",
    "MacRomanian",
    "MacRumanian",
    "MacSami",
    "MacSymbol",
    "MacThai",
    "MacTurkish",
    "MacUkrainian",
    "MIME-B",
    "MIME-Header",
    "MIME-Header-ISO_2022_JP",
    "MIME-Q",
    "nextstep",
    "null",
    "posix-bc",
    "shiftjis",
    "symbol",
    "UCS-2BE",
    "UCS-2LE",
    "UTF-16",
    "UTF-16BE",
    "UTF-16LE",
    "UTF-32",
    "UTF-32BE",
    "UTF-32LE",
    "UTF-7",
    "utf-8-strict",
    "utf8",
    "viscii",
];

/// What an alias rule produces.
enum Target {
    /// A fixed name.
    Name(&'static str),
    /// A name computed from the match (`undef` → `None`).
    Fn(fn(&regex::Captures) -> Option<String>),
}

enum Pat {
    /// A string alias, compared case-insensitively (`lc($find) eq lc($alias)`).
    Str(&'static str),
    Re(Regex),
    /// `qr/\bGB[-_ ]?2312(?!-?raw)/i` (the `regex` crate has no lookahead).
    Gb2312,
}

struct Rule {
    pat: Pat,
    target: Target,
}

fn re(p: &str) -> Pat {
    Pat::Re(Regex::new(p).expect("alias regex"))
}

/// The alias list, oldest first (`init_aliases`, then biber's additions).
static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    use Target::{Fn as F, Name as N};
    let latin2iso = |c: &regex::Captures| -> Option<String> {
        // @Latin2iso = (0, 1, 2, 3, 4, 9, 10, 13, 14, 15, 16); $1 =~ \d+ (Unicode digits)
        const L: [u32; 11] = [0, 1, 2, 3, 4, 9, 10, 13, 14, 15, 16];
        let n: usize = perl_num(&c[1])?;
        L.get(n).map(|v| format!("iso-8859-{v}"))
    };
    let winlatin = |c: &regex::Captures| -> Option<String> {
        let cp = match c[1].to_ascii_lowercase().as_str() {
            "latin1" => "1252",
            "latin2" => "1250",
            "cyrillic" => "1251",
            "greek" => "1253",
            "turkish" => "1254",
            "hebrew" => "1255",
            "arabic" => "1256",
            "baltic" => "1257",
            "vietnamese" => "1258",
            _ => "",
        };
        Some(format!("cp{cp}"))
    };
    let r = |p: Pat, t: Target| Rule { pat: p, target: t };
    vec![
        r(re(r"(?s)^(.*)$"), F(|c| Some(crate::lc(&c[1])))),
        r(re(r"(?i)^(unicode-1-1-)?UTF-?7$"), N("UTF-7")),
        r(re(r"(?i)^UCS-?2-?LE$"), N("UCS-2LE")),
        r(re(r"(?i)^UCS-?2-?(BE)?$"), N("UCS-2BE")),
        r(
            re(r"(?i)^UCS-?4-?(BE|LE|)?$"),
            F(|c| {
                Some(crate::uc(&format!(
                    "UTF-32{}",
                    c.get(1).map_or("", |m| m.as_str())
                )))
            }),
        ),
        r(re(r"(?i)^iso-10646-1$"), N("UCS-2BE")),
        r(
            re(r"(?i)^UTF-?(16|32)-?BE$"),
            F(|c| Some(format!("UTF-{}BE", &c[1]))),
        ),
        r(
            re(r"(?i)^UTF-?(16|32)-?LE$"),
            F(|c| Some(format!("UTF-{}LE", &c[1]))),
        ),
        r(
            re(r"(?i)^UTF-?(16|32)$"),
            F(|c| Some(format!("UTF-{}", &c[1]))),
        ),
        r(re(r"(?i)^(?:US-?)ascii$"), N("ascii")),
        r(Pat::Str("C"), N("ascii")),
        r(re(r"(?i)\b(?:ISO[-_]?)?646(?:[-_]?US)?$"), N("ascii")),
        r(
            re(r"(?i)\biso[-_]?(\d+)[-_](\d+)$"),
            F(|c| Some(format!("iso-{}-{}", &c[1], &c[2]))),
        ),
        r(re(r"(?i)\biso[-_]8859[-_]8[-_]I$"), N("iso-8859-8")),
        r(
            re(r"(?i)\biso8859(\d+)$"),
            F(|c| Some(format!("iso-8859-{}", &c[1]))),
        ),
        r(
            re(r"(?i)\b(?:hp-)?(arabic|greek|hebrew|kana|roman|thai|turkish)8$"),
            F(|c| Some(format!("{}8", &c[1]))),
        ),
        r(re(r"(?i)\bANSI[-_]?X3\.4[-_]?1968$"), N("ascii")),
        r(re(r"(?is)^(.+)@euro$"), F(|c| Some(c[1].to_string()))),
        r(re(r"(?i)\b(?:iso[-_]?)?latin[-_]?(\d+)$"), F(latin2iso)),
        r(
            re(
                r"(?i)\bwin(latin[12]|cyrillic|baltic|greek|turkish|hebrew|arabic|baltic|vietnamese)$",
            ),
            F(winlatin),
        ),
        r(Pat::Str("ascii"), N("US-ascii")),
        r(Pat::Str("cyrillic"), N("iso-8859-5")),
        r(Pat::Str("arabic"), N("iso-8859-6")),
        r(Pat::Str("greek"), N("iso-8859-7")),
        r(Pat::Str("hebrew"), N("iso-8859-8")),
        r(Pat::Str("thai"), N("iso-8859-11")),
        r(re(r"(?i)\btis-?620\b"), N("iso-8859-11")),
        r(
            re(r"(?i)\b(?:cp|ibm|ms|windows)[-_ ]?(\d{2,4})$"),
            F(|c| Some(format!("cp{}", &c[1]))),
        ),
        r(
            re(r"(?is)^(?:x[_-])?mac[_-](.*)$"),
            F(|c| Some(format!("mac{}", &c[1]))),
        ),
        r(re(r"(?i)^macintosh$"), N("MacRoman")),
        r(re(r"(?i)^macce$"), N("MacCentralEurRoman")),
        r(
            re(r"(?i)\bkoi8[\s\-_]*([ru])$"),
            F(|c| Some(format!("koi8-{}", &c[1]))),
        ),
        r(re(r"(?i)\beuc.*cn$"), N("euc-cn")),
        r(re(r"(?i)\bcn.*euc$"), N("euc-cn")),
        r(re(r"(?i)^gbk$"), N("cp936")),
        r(Pat::Gb2312, N("euc-cn")),
        r(re(r"(?i)\bjis$"), N("7bit-jis")),
        r(re(r"(?i)\beuc.*jp$"), N("euc-jp")),
        r(re(r"(?i)\bjp.*euc$"), N("euc-jp")),
        r(re(r"(?i)\bujis$"), N("euc-jp")),
        r(re(r"(?i)\bshift.*jis$"), N("shiftjis")),
        r(re(r"(?i)\bsjis$"), N("shiftjis")),
        r(re(r"(?i)\bwindows-31j$"), N("cp932")),
        r(re(r"(?i)\beuc.*kr$"), N("euc-kr")),
        r(re(r"(?i)\bkr.*euc$"), N("euc-kr")),
        r(re(r"(?i)(?:x-)?uhc$"), N("cp949")),
        r(re(r"(?i)(?:x-)?windows-949$"), N("cp949")),
        r(re(r"(?i)\bks_c_5601-1987$"), N("cp949")),
        r(re(r"(?i)\bbig-?5$"), N("big5-eten")),
        r(re(r"(?i)\bbig5-?et(?:en)?$"), N("big5-eten")),
        r(re(r"(?i)\btca[-_]?big5$"), N("big5-eten")),
        r(re(r"(?i)\bbig5-?hk(?:scs)?$"), N("big5-hkscs")),
        r(re(r"(?i)\bhk(?:scs)?[-_]?big5$"), N("big5-hkscs")),
        r(re(r"(?i)cp65000"), N("UTF-7")),
        r(re(r"(?i)cp65001"), N("utf-8-strict")),
        r(re(r"(?i)\bUTF-8$"), N("utf-8-strict")),
        r(
            re(r"^([^\s_]+)[\s_]+([^\s_]*)$"),
            F(|c| Some(format!("{}-{}", &c[1], &c[2]))),
        ),
        // biber, Constants.pm:367-376
        r(Pat::Str("ansinew"), N("cp1252")),
        r(Pat::Str("applemac"), N("MacRoman")),
        r(Pat::Str("applemacce"), N("MacCentralEurRoman")),
        r(Pat::Str("next"), N("MacRoman")),
        r(Pat::Str("x-mac-roman"), N("MacRoman")),
        r(Pat::Str("x-mac-centeuro"), N("MacCentralEurRoman")),
        r(Pat::Str("x-mac-cyrillic"), N("MacCyrillic")),
        r(Pat::Str("x-nextstep"), N("MacRoman")),
        r(Pat::Str("x-ascii"), N("ascii")),
        r(Pat::Str("lutf8"), N("UTF-8")),
        r(Pat::Str("utf8x"), N("UTF-8")),
    ]
});

/// A string of Unicode decimal digits (`\d+`) as a number (perl numifies
/// only ASCII digits; other digits make `$Latin2iso[$1]` index 0).
fn perl_num(s: &str) -> Option<usize> {
    let ascii: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if ascii.is_empty() {
        return Some(0);
    }
    ascii.parse().ok().or(Some(usize::MAX))
}

fn gb2312_matches(s: &str) -> bool {
    static P: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bGB[-_ ]?2312").unwrap());
    // Try every start position, as perl does after a failed lookahead.
    let mut start = 0;
    while let Some(m) = P.find_at(s, start) {
        let rest = s[m.end()..].to_ascii_lowercase();
        let rest = rest.strip_prefix('-').unwrap_or(&rest);
        if !rest.starts_with("raw") {
            return true;
        }
        start = m.start() + s[m.start()..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// `Encode::find_encoding($name)`: the canonical name, or `None`.
pub(crate) fn find(name: &str) -> Option<&'static str> {
    let mut guard = Vec::new();
    get_encoding(name, &mut guard)
}

fn exact(name: &str) -> Option<&'static str> {
    KNOWN.iter().copied().find(|k| *k == name)
}

fn get_encoding(name: &str, guard: &mut Vec<String>) -> Option<&'static str> {
    let name: String = name.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(k) = exact(&name) {
        return Some(k);
    }
    let lc = crate::lc(&name);
    if let Some(k) = exact(&lc) {
        return Some(k);
    }
    if let Some(k) = find_alias(&name, guard) {
        return Some(k);
    }
    if lc != name
        && let Some(k) = find_alias(&lc, guard)
    {
        return Some(k);
    }
    None
}

fn find_alias(find: &str, guard: &mut Vec<String>) -> Option<&'static str> {
    if guard.iter().any(|g| g == find) {
        return None; // recursion guard: $Alias{$find} = undef
    }
    guard.push(find.to_string());
    let mut found = None;
    for rule in RULES.iter().rev() {
        let new = match &rule.pat {
            Pat::Str(a) => {
                (crate::lc(find) == crate::lc(a)).then(|| target_name(&rule.target, None))
            }
            Pat::Re(r) => r
                .captures(find)
                .map(|c| target_name(&rule.target, Some(&c))),
            Pat::Gb2312 => gb2312_matches(find).then(|| target_name(&rule.target, None)),
        };
        let Some(Some(new)) = new else { continue };
        if new == find {
            continue;
        }
        if let Some(k) = get_encoding(&new, guard) {
            found = Some(k);
            break;
        }
    }
    if found.is_none() {
        let lcf = crate::lc(find);
        // `keys %Encoding, keys %ExtModule`: the last case-insensitive match
        // wins, but names are unique case-insensitively.
        found = KNOWN.iter().copied().find(|k| crate::lc(k) == lcf);
    }
    guard.pop();
    found
}

fn target_name(t: &Target, c: Option<&regex::Captures>) -> Option<String> {
    match t {
        Target::Name(n) => Some((*n).to_string()),
        Target::Fn(f) => f(c.expect("regex rule")),
    }
}

#[cfg(test)]
mod tests {
    use super::find;

    #[test]
    fn common_names() {
        for (n, want) in [
            ("UTF-8", Some("utf-8-strict")),
            ("utf8", Some("utf8")),
            ("latin1", Some("iso-8859-1")),
            ("Shift_JIS", Some("shiftjis")),
            ("sjis", Some("shiftjis")),
            ("x-sjis", Some("shiftjis")),
            ("ms932", Some("cp932")),
            ("windows-31j", Some("cp932")),
            ("GBK", Some("cp936")),
            ("gb2312", Some("euc-cn")),
            ("gb2312-raw", Some("gb2312-raw")),
            ("big5", Some("big5-eten")),
            ("UTF16", Some("UTF-16")),
            ("ucs2", Some("UCS-2BE")),
            ("UCS-4", Some("UTF-32")),
            ("applemacce", Some("MacCentralEurRoman")),
            ("lutf8", Some("utf-8-strict")),
            ("ISO-2022-JP", Some("iso-2022-jp")),
            ("jis", Some("7bit-jis")),
            ("gb18030", None),
            ("bogus", None),
        ] {
            assert_eq!(find(n), want, "{n}");
        }
    }
}

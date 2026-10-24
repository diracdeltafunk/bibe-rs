//! `URI->new($s)->as_string` (URI 5.34) and `URI::Escape::escape_char`.
//!
//! biber call sites: `Output/bbl.pm:557` (`\verb{url}` etc., the escaped form
//! of every `uri`-datatype verbatim field, from NFC text), `Output/bbl.pm:587`
//! (`uri` verbatim lists), `Input/file/biblatexml.pm:974` (biblatexml `uri`
//! fields, escaped at input from NFD text *unless the value contains `%`*,
//! BB-051).
//!
//! What `URI->new` does to a string, reproduced here:
//! 1. strips one `<…>` / `<URL:…>` wrapper, then one `"…"` wrapper (only if
//!    the content has no newline), then leading/trailing whitespace;
//! 2. for server-based schemes (`http`, `https`, `ftp`, `ldap`, `irc`,
//!    `ssh`, … — every scheme whose URI 5.34 class inherits `URI::_server`),
//!    a host containing characters outside the URI character set is
//!    converted with IDNA (`URI::_idna`: lower-case each label with perl `lc`
//!    — the "nameprep" is just `lc` — then punycode, `xn--` prefix; labels
//!    must be 1-63 characters, otherwise the host is left alone);
//! 3. every character outside `` ;/?:@&=+$,A-Za-z0-9-_.!~*'()%# `` is
//!    replaced by the `%XX` escapes of its UTF-8 bytes (`%` itself is kept, so
//!    existing escapes survive and a literal `%` is *not* escaped);
//! 4. `%5B`/`%5D` are turned back into `[`/`]` in the host part (everywhere
//!    for `mailto:`), except for `data:`, `ldapi:`, `urn:`, `sqlite:` URIs;
//! 5. `irc:`/`ircs:` URIs get a leading `/%23` of the path unescaped to `/#`;
//! 6. `geo:` URIs are parsed: a malformed one makes `URI->new` die ("Badly
//!    formed geo uri", which aborts biber) → [`UriError::BadGeo`]; at the poles
//!    the longitude is rewritten to 0 with all numbers reformatted as `%f`.
//!
//! Assumes the `URI_HAS_RESERVED_SQUARE_BRACKETS` environment variable is
//! unset when biber starts (its default).

use crate::unicode::{is_perl_digit, is_perl_space};

/// `URI->new` died.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    /// `URI::geo` croaked: "Badly formed geo uri".
    BadGeo,
}

impl std::fmt::Display for UriError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UriError::BadGeo => f.write_str("Badly formed geo uri"),
        }
    }
}

impl std::error::Error for UriError {}

/// Is `c` in `$URI::uric` (or `#`, which `_uric_escape` also keeps)?
#[inline]
fn is_uric_or_hash(c: char) -> bool {
    c.is_ascii_alphanumeric() || ";/?:@&=+$,-_.!~*'()%#".contains(c)
}

/// `$URI::uric4host`: uric plus `[` `]`.
#[inline]
fn is_uric4host(c: char) -> bool {
    c.is_ascii_alphanumeric() || ";/?:@&=+$,-_.!~*'()%[]".contains(c)
}

/// `URI::Escape::escape_char` for one character: `%XX` per UTF-8 byte.
pub fn escape_char(c: char, out: &mut String) {
    let mut buf = [0u8; 4];
    for b in c.encode_utf8(&mut buf).bytes() {
        out.push('%');
        out.push(
            char::from_digit(u32::from(b >> 4), 16)
                .unwrap_or('0')
                .to_ascii_uppercase(),
        );
        out.push(
            char::from_digit(u32::from(b & 15), 16)
                .unwrap_or('0')
                .to_ascii_uppercase(),
        );
    }
}

/// Schemes whose URI 5.34 implementor class inherits from `URI::_server`.
const SERVER_SCHEMES: &[&str] = &[
    "ftp", "ftpes", "ftps", "gopher", "http", "https", "icap", "icaps", "irc", "ircs", "ldap",
    "ldaps", "mms", "news", "nntp", "nntps", "pop", "rlogin", "rsync", "rtsp", "rtspu", "scp",
    "sftp", "sip", "sips", "smb", "smtp", "snews", "ssh", "telnet", "tn3270", "ws", "wss",
];

/// Length of a `$scheme_re` (`[a-zA-Z][a-zA-Z0-9.+\-]*`) match followed by
/// `:` at the start of `s`, if any.
fn scheme_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.is_empty() || !b[0].is_ascii_alphabetic() {
        return None;
    }
    let n = 1 + b[1..]
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-'))
        .count();
    (b.get(n) == Some(&b':')).then_some(n)
}

/// Strip `^<(?:URL:)?(.*)>$` / `^"(.*)"$` (perl `$` = end or before a final
/// newline; `.` excludes newline).
fn strip_wrapper(s: &str, open: char, close: char, url_prefix: bool) -> Option<String> {
    let (body, nl) = match s.strip_suffix('\n') {
        Some(b) => (b, "\n"),
        None => (s, ""),
    };
    let inner = body.strip_prefix(open)?.strip_suffix(close)?;
    let inner = if url_prefix {
        inner.strip_prefix("URL:").unwrap_or(inner)
    } else {
        inner
    };
    let _ = url_prefix;
    if inner.contains('\n') {
        return None;
    }
    Some(format!("{inner}{nl}"))
}

/// `URI->new($s)->as_string`.
pub fn uri_as_string(s: &str) -> Result<String, UriError> {
    // new(): wrappers and whitespace
    let mut u = s.to_string();
    if let Some(x) = strip_wrapper(&u, '<', '>', true) {
        // `(?:URL:)?` is optional; `.*` then absorbs it if the greedy match
        // prefers… it does not: the optional group is tried first.
        u = x;
    }
    if let Some(x) = strip_wrapper(&u, '"', '"', false) {
        u = x;
    }
    let u = u
        .trim_start_matches(is_perl_space)
        .trim_end_matches(is_perl_space)
        .to_string();

    let scheme = scheme_len(&u).map(|n| u[..n].to_ascii_lowercase());
    let server = scheme
        .as_deref()
        .is_some_and(|sc| SERVER_SCHEMES.contains(&sc));
    let mut out = if server {
        server_uric_escape(&u)
    } else {
        uric_escape(&u)
    };
    match scheme.as_deref() {
        Some("irc" | "ircs") => irc_fix(&mut out),
        Some("geo") => out = geo_init(out)?,
        _ => {}
    }
    Ok(out)
}

/// `URI::_uric_escape` (generic): escape, then `_fix_uric_escape_for_host_part`.
fn uric_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_uric_or_hash(c) {
            out.push(c);
        } else {
            escape_char(c, &mut out);
        }
    }
    fix_host_part(&mut out);
    out
}

fn replace_brackets(s: &str) -> String {
    // s/\%5B/[/gi; s/\%5D/]/gi
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let b = s.as_bytes();
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && b[i + 1] == b'5' {
            match b[i + 2] {
                b'B' | b'b' => {
                    out.push('[');
                    i += 3;
                    continue;
                }
                b'D' | b'd' => {
                    out.push(']');
                    i += 3;
                    continue;
                }
                _ => {}
            }
        }
        let c = s[i..].chars().next().unwrap_or('\0');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn fix_host_part(s: &mut String) {
    if !s.contains('%') {
        return;
    }
    for p in ["data:", "ldapi:", "urn:", "sqlite:", "sqlite3:"] {
        if s.starts_with(p) {
            return;
        }
    }
    if s.starts_with("mailto:") {
        *s = replace_brackets(s);
        return;
    }
    // m{^((?:$scheme_re:)?)//([^/?\#]+)(.*)$}s
    let start = scheme_len(s).map_or(0, |n| n + 1);
    let Some(rest) = s[start..].strip_prefix("//") else {
        return;
    };
    let alen = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if alen == 0 {
        return;
    }
    let orig = rest[..alen].to_string();
    // ($user, $host) = $orig =~ /^(.*@)?([^@]*)$/  (escaped: no newlines)
    let (user, host) = match orig.rfind('@') {
        Some(i) => (&orig[..=i], &orig[i + 1..]),
        None => ("", orig.as_str()),
    };
    // $host =~ s/(:\d+)$//
    let (host, port) = split_port(host, |c| c.is_ascii_digit());
    let host = replace_brackets(host);
    let new = format!("{user}{host}{port}");
    if let Some(i) = s.find(&orig) {
        s.replace_range(i..i + orig.len(), &new);
    }
}

/// Split a trailing `:\d+` port off a host (`s/(:\d+)\z//`).
fn split_port(host: &str, is_digit: impl Fn(char) -> bool) -> (&str, &str) {
    let trimmed = host.trim_end_matches(&is_digit);
    if trimmed.len() < host.len() && trimmed.ends_with(':') {
        let i = trimmed.len() - 1;
        (&host[..i], &host[i..])
    } else {
        (host, "")
    }
}

/// `URI::_server::_uric_escape`: IDNA-encode the host, then the generic
/// escaping.
fn server_uric_escape(s: &str) -> String {
    // m,^((?:$URI::scheme_re:)?)//([^/?\#]*)(.*)$,s
    let start = scheme_len(s).map_or(0, |n| n + 1);
    if let Some(rest) = s[start..].strip_prefix("//") {
        let alen = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (auth, tail) = rest.split_at(alen);
        // $ui = $host =~ s/(.*@)// ? $1 : ""   (`.` excludes "\n"; leftmost)
        let (ui, host) = split_userinfo(auth);
        let (host, port) = split_port(&host, is_perl_digit);
        if host.chars().any(|c| !is_uric4host(c))
            && let Some(h) = idna_encode(host)
        {
            let rebuilt = format!("{}//{ui}{h}{port}{tail}", &s[..start]);
            return uric_escape(&rebuilt);
        }
    }
    uric_escape(s)
}

/// `$host =~ s/(.*@)//` with perl's leftmost-longest semantics where `.`
/// does not match a newline. Returns (userinfo, host-without-it).
fn split_userinfo(auth: &str) -> (String, String) {
    let mut line_start = 0;
    for line in auth.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        if let Some(at) = body.rfind('@') {
            let ui = &auth[line_start..=line_start + at];
            let host = format!("{}{}", &auth[..line_start], &auth[line_start + at + 1..]);
            return (ui.to_string(), host);
        }
        line_start += line.len();
    }
    (String::new(), auth.to_string())
}

/// `URI::_idna::encode`; `None` if it croaks.
fn idna_encode(domain: &str) -> Option<String> {
    let mut labels: Vec<&str> = domain.split('.').collect();
    let mut last_empty = false;
    if labels.len() > 1 && labels.last() == Some(&"") {
        labels.pop();
        last_empty = true;
    }
    let mut out = Vec::with_capacity(labels.len());
    for l in labels {
        out.push(to_ascii(l)?);
    }
    let mut s = out.join(".");
    if last_empty {
        s.push('.');
    }
    Some(s)
}

fn check_size(label: String) -> Option<String> {
    let n = label.chars().count();
    (n > 0 && n <= 63).then_some(label)
}

fn to_ascii(label: &str) -> Option<String> {
    if label.is_ascii() {
        return check_size(label.to_string());
    }
    let label = crate::case::lc(label); // nameprep is just lc()
    if label.is_ascii() {
        return check_size(label);
    }
    if label.starts_with("xn--") {
        return None;
    }
    check_size(format!("xn--{}", encode_punycode(&label)))
}

/// RFC 3492 punycode encoding, as `URI::_punycode::encode_punycode`.
pub fn encode_punycode(input: &str) -> String {
    const BASE: u64 = 36;
    const TMIN: u64 = 1;
    const TMAX: u64 = 26;
    const SKEW: u64 = 38;
    const DAMP: u64 = 700;
    let cps: Vec<u64> = input.chars().map(|c| u64::from(c as u32)).collect();
    let mut out = String::new();
    for &c in &cps {
        if c < 0x80 {
            out.push(char::from(c as u8));
        }
    }
    let b = out.len() as u64;
    let mut h = b;
    if b > 0 && (h as usize) < cps.len() {
        out.push('-');
    }
    let adapt = |delta: u64, numpoints: u64, first: bool| -> u64 {
        let mut delta = if first { delta / DAMP } else { delta / 2 };
        delta += delta / numpoints;
        let mut k = 0;
        while delta > ((BASE - TMIN) * TMAX) / 2 {
            delta /= BASE - TMIN;
            k += BASE;
        }
        k + ((BASE - TMIN + 1) * delta) / (delta + SKEW)
    };
    let digit = |d: u64| -> char {
        if d <= 25 {
            char::from(b'a' + d as u8)
        } else {
            char::from(b'0' + (d - 26) as u8)
        }
    };
    let mut n: u64 = 128;
    let mut delta: u64 = 0;
    let mut bias: u64 = 72;
    while (h as usize) < cps.len() {
        let m = cps.iter().copied().filter(|&c| c >= n).min().unwrap_or(n);
        delta += (m - n) * (h + 1);
        n = m;
        for &c in &cps {
            if c < n {
                delta += 1;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = if k <= bias {
                        TMIN
                    } else if k >= bias + TMAX {
                        TMAX
                    } else {
                        k - bias
                    };
                    if q < t {
                        break;
                    }
                    out.push(digit(t + (q - t) % (BASE - t)));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                out.push(digit(q));
                bias = adapt(delta, h + 1, h == b);
                delta = 0;
                h += 1;
            }
        }
        delta += 1;
        n += 1;
    }
    out
}

/// `URI::irc`: `_init` turns a leading path `/#` into `/%23`, and
/// `as_string` turns a leading `/%23` back into `/#`; the net effect is that
/// an escaped `/%23` at that position is unescaped.
fn irc_fix(s: &mut String) {
    let mut i = 0;
    if let Some(c) = s.find([':', '/', '?', '#'])
        && c > 0
        && s.as_bytes()[c] == b':'
    {
        i = c + 1;
    }
    if s[i..].starts_with("//") {
        let rest = &s[i + 2..];
        i += 2 + rest.find(['/', '?', '#']).unwrap_or(rest.len());
    }
    if s[i..].starts_with("/%23") {
        s.replace_range(i..i + 4, "/#");
    }
}

/// `URI::Split::uri_split` → (scheme, auth, path, query, frag) byte ranges.
fn uri_split(s: &str) -> (Option<&str>, Option<&str>, &str, Option<&str>, Option<&str>) {
    let mut rest = s;
    let mut scheme = None;
    if let Some(c) = rest.find([':', '/', '?', '#'])
        && c > 0
        && rest.as_bytes()[c] == b':'
    {
        scheme = Some(&rest[..c]);
        rest = &rest[c + 1..];
    }
    let mut auth = None;
    if let Some(r) = rest.strip_prefix("//") {
        let n = r.find(['/', '?', '#']).unwrap_or(r.len());
        auth = Some(&r[..n]);
        rest = &r[n..];
    }
    let n = rest.find(['?', '#']).unwrap_or(rest.len());
    let path = &rest[..n];
    rest = &rest[n..];
    let mut query = None;
    if let Some(r) = rest.strip_prefix('?') {
        let n = r.find('#').unwrap_or(r.len());
        query = Some(&r[..n]);
        rest = &r[n..];
    }
    let frag = rest.strip_prefix('#');
    (scheme, auth, path, query, frag)
}

struct Geo<'a> {
    lat: &'a str,
    alt: Option<&'a str>,
    crs: Option<&'a str>,
    unc: Option<&'a str>,
    parm: Option<&'a str>,
}

fn take_num(s: &str, signed: bool) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    let mut i = 0;
    if signed && b.first() == Some(&b'-') {
        i = 1;
    }
    let digits = |from: usize| -> usize {
        s[from..]
            .chars()
            .take_while(|c| is_perl_digit(*c))
            .map(char::len_utf8)
            .sum::<usize>()
    };
    let d = digits(i);
    if d == 0 {
        return None;
    }
    i += d;
    if b.get(i) == Some(&b'.') {
        let d2 = digits(i + 1);
        if d2 > 0 {
            i += 1 + d2;
        }
    }
    Some((&s[..i], &s[i..]))
}

fn at_end(s: &str) -> bool {
    s.is_empty() || s == "\n"
}

fn parse_geo(path: &str) -> Option<Geo<'_>> {
    let (lat, r) = take_num(path, true)?;
    let r = r.strip_prefix(',')?;
    let (_lon, mut r) = take_num(r, true)?;
    let mut alt = None;
    if let Some(x) = r.strip_prefix(',')
        && let Some((a, rr)) = take_num(x, true)
    {
        alt = Some(a);
        r = rr;
    }
    let mut crs = None;
    if let Some(x) = r.strip_prefix(";crs=") {
        let n: usize = x
            .chars()
            .take_while(|c| crate::unicode::is_perl_word(*c))
            .map(char::len_utf8)
            .sum();
        if n > 0 && (at_end(&x[n..]) || x[n..].starts_with(';')) {
            crs = Some(&x[..n]);
            r = &x[n..];
        }
    }
    let mut unc = None;
    if let Some(x) = r.strip_prefix(";u=")
        && let Some((u, rr)) = take_num(x, false)
        && (at_end(rr) || rr.starts_with(';'))
    {
        unc = Some(u);
        r = rr;
    }
    let mut parm = None;
    if !at_end(r) {
        // (?:;\w+=[^;]*)+ to the end
        let whole = r.strip_suffix('\n').unwrap_or(r);
        let mut x = whole;
        while !x.is_empty() {
            let y = x.strip_prefix(';')?;
            let n: usize = y
                .chars()
                .take_while(|c| crate::unicode::is_perl_word(*c))
                .map(char::len_utf8)
                .sum();
            if n == 0 || !y[n..].starts_with('=') {
                return None;
            }
            let z = &y[n + 1..];
            x = &z[z.find(';').unwrap_or(z.len())..];
        }
        parm = Some(&whole[1..]);
    }
    Some(Geo {
        lat,
        alt,
        crs,
        unc,
        parm,
    })
}

/// `URI::geo::_num`: `sprintf '%f'`, then drop an all-zero fraction.
fn geo_num(n: &str) -> String {
    let v = crate::truth::PerlNum::parse(n).as_f64();
    let s = format!("{v:.6}");
    match s.find('.') {
        Some(i) if s[i + 1..].bytes().all(|b| b == b'0') => s[..i].to_string(),
        _ => s,
    }
}

fn geo_init(s: String) -> Result<String, UriError> {
    let (scheme, auth, path, query, frag) = uri_split(&s);
    let g = parse_geo(path).ok_or(UriError::BadGeo)?;
    let lat = crate::truth::PerlNum::parse(g.lat).as_f64();
    if lat != 90.0 && lat != -90.0 {
        return Ok(s);
    }
    // $self->longitude(0): re-format through _format and uri_join.
    let mut parts = vec![geo_num(g.lat), geo_num("0")];
    if let Some(a) = g.alt {
        parts.push(a.to_string());
    }
    let mut fields = vec![parts.join(",")];
    if let Some(c) = g.crs {
        fields.push(format!("crs={}", geo_num(c)));
    }
    if let Some(u) = g.unc {
        fields.push(format!("u={}", geo_num(u)));
    }
    if let Some(p) = g.parm {
        fields.push(p.to_string());
    }
    let mut path = fields.join(";");
    let mut out = scheme.map(|sc| format!("{sc}:")).unwrap_or_default();
    if let Some(a) = auth {
        let mut ea = String::new();
        for c in a.chars() {
            if matches!(c, '/' | '?' | '#') {
                escape_char(c, &mut ea)
            } else {
                ea.push(c)
            }
        }
        out.push_str("//");
        out.push_str(&ea);
        if !path.is_empty() && !path.starts_with('/') {
            path.insert(0, '/');
        }
    } else if path.starts_with("//") {
        out.push_str("//");
    }
    let mut ep = String::new();
    for c in path.chars() {
        if matches!(c, '?' | '#') {
            escape_char(c, &mut ep)
        } else {
            ep.push(c)
        }
    }
    out.push_str(&ep);
    if let Some(q) = query {
        out.push('?');
        out.push_str(&q.replace('#', "%23"));
    }
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(
            uri_as_string("http://example.com/a b").unwrap(),
            "http://example.com/a%20b"
        );
        assert_eq!(
            uri_as_string("<URL:http://x.org/é>").unwrap(),
            "http://x.org/%C3%A9"
        );
        assert_eq!(
            uri_as_string("http://bücher.de/x").unwrap(),
            "http://xn--bcher-kva.de/x"
        );
        assert_eq!(
            uri_as_string("http://x.org/100%").unwrap(),
            "http://x.org/100%"
        );
        assert_eq!(uri_as_string("foo bar").unwrap(), "foo%20bar");
        assert_eq!(uri_as_string("http://[::1]/").unwrap(), "http://[::1]/");
        assert_eq!(uri_as_string("geo:foo"), Err(UriError::BadGeo));
        assert_eq!(uri_as_string("geo:90,10").unwrap(), "geo:90,0");
        assert_eq!(encode_punycode("münchen"), "mnchen-3ya");
    }
}

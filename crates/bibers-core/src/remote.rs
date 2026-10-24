//! Remote data sources (DC-032): `http://`, `https://` and `ftp://` `.bib`
//! files, fetched as biber does in `Biber::Utils::locate_data_file`
//! (Utils.pm:226-305) with LWP::UserAgent 6 (`LWP::Protocol::http`,
//! `::https` over IO::Socket::SSL, `::ftp` over Net::FTP).
//!
//! What is reproduced:
//! - the request: `GET` with `User-Agent: Mozilla/5.0` and
//!   `Zotero-Allowed-Request: 1`, `Authorization: Basic` from URL user
//!   information, the URL escaped as `URI->new` does;
//! - up to 7 redirects (301, 302, 303, 307, 308; relative `Location`s are
//!   resolved), after which the last 3xx is the result;
//! - `env_proxy` (`http_proxy`, `https_proxy` via `CONNECT`, `ftp_proxy`,
//!   `no_proxy`, upper-case variants);
//! - the status line in `Could not fetch '…' (HTTP error: <code> <message>)`:
//!   the server's reason phrase, else HTTP::Status's, and LWP's own
//!   `500 Can't connect to host:port (reason)` messages;
//! - TLS trust as biber sets it up: the Mozilla CA list (`webpki-roots`, as
//!   biber's bundled Mozilla::CA) unless `PERL_LWP_SSL_CA_FILE`/`_CA_PATH`
//!   (or `HTTPS_CA_FILE`/`_DIR`) are set; with `--ssl-nointernalca`, the
//!   first existing system bundle biber knows (and `/etc/ssl/certs/`,
//!   `/etc/pki/tls/` as a CA directory), else IO::Socket::SSL's defaults
//!   (`SSL_CERT_FILE`, `SSL_CERT_DIR`, `/etc/ssl/cert.pem`), else Mozilla's.
//!   `--ssl-noverify-host` (or `PERL_LWP_SSL_VERIFY_HOSTNAME=0`) turns off
//!   the host name check only: the certificate chain is still verified;
//! - FTP as `LWP::Protocol::ftp` drives Net::FTP: anonymous login
//!   (`anonymous@`), `PWD`, `TYPE I`, `CWD` per directory, `MDTM`, passive
//!   `RETR`, and its 401/404/500 messages.
//!
//! Not reproduced: `~/.netrc` credentials for FTP, HTTP authentication
//! challenges (401 + `WWW-Authenticate` without URL credentials fails as in
//! biber, since biber never sets credentials), compressed transfer codings
//! (bibers does not send `TE`).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

/// LWP::UserAgent's default timeout.
const TIMEOUT: Duration = Duration::from_secs(180);

/// Does biber treat `source` as remote (`m/\A(?:http|ftp)(s?):\/\//xms`,
/// case-sensitive)? Returns whether it is a TLS scheme (`$1`).
pub fn remote_scheme(source: &str) -> Option<bool> {
    for (p, tls) in [
        ("http://", false),
        ("https://", true),
        ("ftp://", false),
        ("ftps://", true),
    ] {
        if source.starts_with(p) {
            return Some(tls);
        }
    }
    None
}

/// TLS options (biber's `--ssl-*` options and the `PERL_LWP_SSL_*`
/// environment).
#[derive(Clone, Debug, Default)]
pub struct TlsEnv {
    /// `PERL_LWP_SSL_CA_FILE` (or `HTTPS_CA_FILE`).
    pub ca_file: Option<String>,
    /// `PERL_LWP_SSL_CA_PATH` (or `HTTPS_CA_DIR`).
    pub ca_path: Option<String>,
    /// LWP's `verify_hostname`.
    pub verify_hostname: bool,
}

/// The effective process environment, with the variables biber sets in
/// `%ENV` (it changes its own environment, so later fetches see them).
#[derive(Clone, Debug, Default)]
pub struct EnvOverrides {
    vars: BTreeMap<String, String>,
}

impl EnvOverrides {
    pub fn get(&self, k: &str) -> Option<String> {
        self.vars.get(k).cloned().or_else(|| std::env::var(k).ok())
    }
    fn exists(&self, k: &str) -> bool {
        self.get(k).is_some()
    }
    fn set(&mut self, k: &str, v: &str) {
        self.vars.insert(k.to_string(), v.to_string());
    }
}

/// biber's CA setup for HTTPS/FTPS sources (Utils.pm:241-275), applied to
/// the (simulated) environment.
pub fn setup_ssl_env(env: &mut EnvOverrides, nointernalca: bool, noverify_host: bool) {
    if !env.exists("PERL_LWP_SSL_CA_FILE") && !env.exists("PERL_LWP_SSL_CA_PATH") && !nointernalca {
        // Mozilla::CA's cacert.pem: represented by the built-in list.
        env.set("PERL_LWP_SSL_CA_FILE", MOZILLA_CA);
    }
    if !env.exists("PERL_LWP_SSL_CA_FILE") {
        for b in [
            "/etc/ssl/certs/ca-certificates.crt",
            "/etc/pki/tls/certs/ca-bundle.crt",
            "/etc/ssl/ca-bundle.pem",
        ] {
            if std::path::Path::new(b).exists() {
                env.set("PERL_LWP_SSL_CA_FILE", b);
                break;
            }
        }
        for p in ["/etc/ssl/certs/", "/etc/pki/tls/"] {
            if std::path::Path::new(p).is_dir() {
                env.set("PERL_LWP_SSL_CA_PATH", p);
                break;
            }
        }
    }
    if noverify_host {
        env.set("PERL_LWP_SSL_VERIFY_HOSTNAME", "0");
    }
}

/// Stand-in path for Mozilla::CA's bundle.
const MOZILLA_CA: &str = "<Mozilla::CA>";

/// `LWP::UserAgent->new`'s `ssl_opts` from the environment.
fn tls_env(env: &EnvOverrides) -> TlsEnv {
    let nonempty = |k: &str| env.get(k).filter(|v| !v.is_empty());
    let verify_hostname = if let Some(v) = env.get("PERL_LWP_SSL_VERIFY_HOSTNAME") {
        perl_compat::perl_truthy(&v)
    } else {
        !(nonempty("HTTPS_CA_FILE").is_some() || nonempty("HTTPS_CA_DIR").is_some())
    };
    TlsEnv {
        ca_file: nonempty("PERL_LWP_SSL_CA_FILE").or_else(|| nonempty("HTTPS_CA_FILE")),
        ca_path: nonempty("PERL_LWP_SSL_CA_PATH").or_else(|| nonempty("HTTPS_CA_DIR")),
        verify_hostname,
    }
}

// ---------------------------------------------------------------- URLs

/// A parsed (escaped) URL.
#[derive(Clone, Debug)]
struct Url {
    scheme: String,
    userinfo: Option<String>,
    host: String,
    port: u16,
    /// Path and query (`/` if empty).
    path_query: String,
    /// The escaped URL without fragment.
    full: String,
}

fn default_port(scheme: &str) -> u16 {
    match scheme {
        "https" => 443,
        "ftp" => 21,
        "ftps" => 990,
        _ => 80,
    }
}

fn parse_url(s: &str) -> Result<Url, String> {
    let esc = perl_compat::uri::uri_as_string(s).map_err(|e| format!("{e:?}"))?;
    let esc = esc.split('#').next().unwrap_or("").to_string();
    let (scheme, rest) = esc
        .split_once("://")
        .ok_or_else(|| "URL must be absolute".to_string())?;
    let scheme = scheme.to_ascii_lowercase();
    let (auth, pq) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let (userinfo, hostport) = match auth.rfind('@') {
        Some(i) => (Some(auth[..i].to_string()), &auth[i + 1..]),
        None => (None, auth),
    };
    let (host, port) = if let Some(h) = hostport.strip_prefix('[') {
        let (h, r) = h.split_once(']').unwrap_or((h, ""));
        (h.to_string(), r.strip_prefix(':').unwrap_or(""))
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p),
            None => (hostport.to_string(), ""),
        }
    };
    let port = if port.is_empty() {
        default_port(&scheme)
    } else {
        port.parse().map_err(|_| format!("Bad port '{port}'"))?
    };
    let path_query = if pq.starts_with('/') {
        pq.to_string()
    } else {
        format!("/{pq}")
    };
    Ok(Url {
        scheme,
        userinfo,
        host,
        port,
        path_query,
        full: esc,
    })
}

impl Url {
    /// `host:port` as LWP's `$url->host_port` (always with the port).
    fn host_port(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// The `Host` header: the authority without user information.
    fn host_header(&self) -> String {
        let auth = self
            .full
            .split_once("://")
            .map(|(_, r)| r.split(['/', '?']).next().unwrap_or(""))
            .unwrap_or("");
        auth.rsplit_once('@').map_or(auth, |(_, h)| h).to_string()
    }

    /// Resolve a `Location` against this URL (RFC 3986 §5.2).
    fn join(&self, loc: &str) -> Result<Url, String> {
        let loc = perl_compat::uri::uri_as_string(loc).map_err(|e| format!("{e:?}"))?;
        if let Some(i) = loc.find(':')
            && loc[..i]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            && i > 0
        {
            return parse_url(&loc);
        }
        let origin = format!(
            "{}://{}",
            self.scheme,
            self.full
                .split_once("://")
                .map(|(_, r)| r.split(['/', '?']).next().unwrap_or(""))
                .unwrap_or("")
        );
        if let Some(r) = loc.strip_prefix("//") {
            return parse_url(&format!("{}://{r}", self.scheme));
        }
        let (base_path, _) = self
            .path_query
            .split_once('?')
            .unwrap_or((&self.path_query, ""));
        let target = if loc.is_empty() {
            self.path_query.clone()
        } else if loc.starts_with('/') {
            remove_dots(&loc)
        } else if loc.starts_with('?') {
            format!("{base_path}{loc}")
        } else {
            let dir = &base_path[..=base_path.rfind('/').unwrap_or(0)];
            remove_dots(&format!("{dir}{loc}"))
        };
        parse_url(&format!("{origin}{target}"))
    }
}

fn remove_dots(p: &str) -> String {
    let (path, q) = match p.split_once('?') {
        Some((a, b)) => (a, Some(b)),
        None => (p, None),
    };
    let mut out: Vec<&str> = Vec::new();
    let segs: Vec<&str> = path.split('/').collect();
    let n = segs.len();
    for (i, s) in segs.iter().enumerate() {
        match *s {
            "." => {
                if i == n - 1 {
                    out.push("");
                }
            }
            ".." => {
                if out.len() > 1 {
                    out.pop();
                }
                if i == n - 1 {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let mut r = out.join("/");
    if !r.starts_with('/') {
        r.insert(0, '/');
    }
    if let Some(q) = q {
        r.push('?');
        r.push_str(q);
    }
    r
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in input.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for (i, sh) in [18, 12, 6, 0].iter().enumerate() {
            if i <= c.len() {
                out.push(T[((n >> sh) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

// ---------------------------------------------------------------- HTTP

/// HTTP::Status's message for a code (`status_message`).
fn status_message(code: u16) -> Option<&'static str> {
    Some(match code {
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        103 => "Early Hints",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        208 => "Already Reported",
        226 => "IM Used",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        305 => "Use Proxy",
        306 => "(Unused)",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        423 => "Locked",
        424 => "Failed Dependency",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        449 => "Retry with",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        506 => "Variant Also Negotiates",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        509 => "Bandwidth Limit Exceeded",
        510 => "Not Extended",
        511 => "Network Authentication Required",
        _ => return None,
    })
}

/// A response: code, message, headers, body.
struct Response {
    code: u16,
    message: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn error(code: u16, message: impl Into<String>) -> Response {
        Response {
            code,
            message: message.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
    fn header(&self, k: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }
    fn is_success(&self) -> bool {
        (200..300).contains(&self.code)
    }
    /// `HTTP::Response::status_line`.
    fn status_line(&self) -> String {
        let m = if self.message.is_empty() {
            status_message(self.code).unwrap_or("Unknown code")
        } else {
            &self.message
        };
        format!("{} {m}", self.code)
    }
}

/// An I/O error's text as perl's `$!` shows it.
fn errno_text(e: &std::io::Error) -> String {
    // Rust and perl agree on strerror()/gai_strerror() texts.
    crate::perlish::errno_text(e)
        .trim_start_matches("failed to lookup address information: ")
        .to_string()
}

fn connect_tcp(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let addrs = (host, port).to_socket_addrs()?;
    let mut last = None;
    for a in addrs {
        match TcpStream::connect_timeout(&a, TIMEOUT) {
            Ok(s) => {
                s.set_read_timeout(Some(TIMEOUT))?;
                s.set_write_timeout(Some(TIMEOUT))?;
                return Ok(s);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no address")))
}

/// LWP's "Can't connect to host:port (reason)".
fn cant_connect(host: &str, port: u16, reason: &str) -> Response {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    Response::error(500, format!("Can't connect to {host}:{port} ({reason})"))
}

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

/// Read a CRLF/LF line (without the terminator).
fn read_line<R: BufRead>(r: &mut R) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    if r.read_until(b'\n', &mut buf)? == 0 {
        return Ok(None);
    }
    while buf.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// Send `GET target` over `s` and read the response.
fn http_exchange(
    s: &mut dyn Stream,
    target: &str,
    headers: &[(String, String)],
) -> Result<Response, String> {
    let mut req = format!("GET {target} HTTP/1.1\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())
        .and_then(|_| s.flush())
        .map_err(|e| errno_text(&e))?;
    let mut r = BufReader::new(s);
    let status = read_line(&mut r)
        .map_err(|e| errno_text(&e))?
        .ok_or_else(|| "Server closed connection without sending any data back".to_string())?;
    let mut parts = status.splitn(3, ' ');
    let proto = parts.next().unwrap_or("");
    if !proto.starts_with("HTTP/") {
        return Err(format!("Bad response status line: '{status}'"));
    }
    let code: u16 = parts
        .next()
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| format!("Bad response status line: '{status}'"))?;
    let message = parts.next().unwrap_or("").trim().to_string();
    let mut headers: Vec<(String, String)> = Vec::new();
    while let Some(l) = read_line(&mut r).map_err(|e| errno_text(&e))? {
        if l.is_empty() {
            break;
        }
        if (l.starts_with(' ') || l.starts_with('\t'))
            && let Some(last) = headers.last_mut()
        {
            last.1.push(' ');
            last.1.push_str(l.trim());
            continue;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let mut resp = Response {
        code,
        message,
        headers,
        body: Vec::new(),
    };
    let chunked = resp
        .header("Transfer-Encoding")
        .is_some_and(|t| t.to_ascii_lowercase().contains("chunked"));
    if chunked {
        loop {
            let l = read_line(&mut r)
                .map_err(|e| errno_text(&e))?
                .ok_or("Premature end of chunked content")?;
            let size = u64::from_str_radix(l.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| format!("Bad chunk-size in HTTP response: {l}"))?;
            if size == 0 {
                while let Some(t) = read_line(&mut r).map_err(|e| errno_text(&e))? {
                    if t.is_empty() {
                        break;
                    }
                }
                break;
            }
            // Stream the chunk: the advertised size is untrusted (no
            // allocation up front), and a short chunk is a fetch error.
            let got = (&mut r)
                .take(size)
                .read_to_end(&mut resp.body)
                .map_err(|e| errno_text(&e))?;
            if (got as u64) < size {
                return Err("Premature end of chunked content".into());
            }
            read_line(&mut r).map_err(|e| errno_text(&e))?;
        }
    } else if let Some(n) = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
    {
        r.take(n)
            .read_to_end(&mut resp.body)
            .map_err(|e| errno_text(&e))?;
    } else if !(code == 204 || code == 304 || (100..200).contains(&code)) {
        r.read_to_end(&mut resp.body).map_err(|e| errno_text(&e))?;
    }
    Ok(resp)
}

/// The proxy to use for `url` (`env_proxy` + `_need_proxy`).
fn proxy_for(url: &Url, env: &EnvOverrides) -> Option<Url> {
    // `sort keys %ENV`: upper-case names come first and win.
    let get = |name: &str| {
        env.get(&name.to_ascii_uppercase())
            .filter(|v| !v.is_empty())
            .or_else(|| env.get(name).filter(|v| !v.is_empty()))
    };
    let p = get(&format!("{}_proxy", url.scheme))?;
    if let Some(no) = get("no_proxy") {
        for d in no.split(',').map(str::trim) {
            let d = d.trim_start_matches('.');
            if d == "*" {
                return None;
            }
            if !d.is_empty() && (url.host == d || url.host.ends_with(&format!(".{d}"))) {
                return None;
            }
        }
    }
    parse_url(&p).ok()
}

/// One `simple_request` over HTTP(S): no redirects.
fn http_simple(url: &Url, tls: &TlsEnv, env: &EnvOverrides) -> Response {
    let proxy = proxy_for(url, env);
    let https = url.scheme == "https";
    let (host, port) = match &proxy {
        Some(p) => (p.host.clone(), p.port),
        None => (url.host.clone(), url.port),
    };
    let tcp = match connect_tcp(&host, port) {
        Ok(s) => s,
        Err(e) => {
            if proxy.is_none() || !https {
                return cant_connect(&host, port, &errno_text(&e));
            }
            return cant_connect(&host, port, &errno_text(&e));
        }
    };
    let mut stream: Box<dyn Stream> = Box::new(tcp);
    if https {
        if let Some(p) = &proxy {
            // CONNECT tunnel.
            let mut h = vec![("Host".to_string(), url.host_port())];
            if let Some(ui) = &p.userinfo {
                h.insert(
                    0,
                    (
                        "Proxy-Authorization".into(),
                        format!("Basic {}", base64(&percent_decode(ui))),
                    ),
                );
            }
            let mut s = stream;
            let mut req = format!("CONNECT {} HTTP/1.1\r\n", url.host_port());
            for (k, v) in &h {
                req.push_str(&format!("{k}: {v}\r\n"));
            }
            req.push_str("\r\n");
            if let Err(e) = s.write_all(req.as_bytes()) {
                return Response::error(500, errno_text(&e));
            }
            let mut line = Vec::new();
            let mut b = [0u8; 1];
            while !line.ends_with(b"\r\n\r\n") && !line.ends_with(b"\n\n") {
                match s.read(&mut b) {
                    Ok(1) => line.push(b[0]),
                    _ => break,
                }
            }
            let head = String::from_utf8_lossy(&line).into_owned();
            let code: u16 = head
                .split_whitespace()
                .nth(1)
                .and_then(|c| c.parse().ok())
                .unwrap_or(500);
            if !(200..300).contains(&code) {
                let msg = head
                    .lines()
                    .next()
                    .unwrap_or("")
                    .splitn(3, ' ')
                    .nth(2)
                    .unwrap_or("");
                let r = Response::error(code, msg.trim());
                return Response::error(
                    500,
                    format!("establishing SSL tunnel failed: {}", r.status_line()),
                );
            }
            stream = s;
        }
        match tls_connect(stream, &url.host, tls) {
            Ok(s) => stream = s,
            Err(reason) => return cant_connect(&url.host, url.port, &reason),
        }
    }
    let mut headers = vec![("Connection".to_string(), "close".to_string())];
    if let Some(ui) = &url.userinfo {
        let dec = percent_decode(ui);
        headers.push(("Authorization".into(), format!("Basic {}", base64(&dec))));
    }
    if let (Some(p), false) = (&proxy, https)
        && let Some(ui) = &p.userinfo
    {
        headers.push((
            "Proxy-Authorization".into(),
            format!("Basic {}", base64(&percent_decode(ui))),
        ));
    }
    headers.push(("Host".into(), url.host_header()));
    headers.push(("User-Agent".into(), "Mozilla/5.0".into()));
    headers.push(("Zotero-Allowed-Request".into(), "1".into()));
    let target = if proxy.is_some() && !https {
        url.full.clone()
    } else {
        url.path_query.clone()
    };
    match http_exchange(stream.as_mut(), &target, &headers) {
        Ok(r) => r,
        Err(e) => Response::error(500, e),
    }
}

// ---------------------------------------------------------------- TLS

fn load_certs(path: &str, out: &mut rustls::RootCertStore) {
    use rustls_pki_types::CertificateDer;
    use rustls_pki_types::pem::PemObject;
    if let Ok(it) = CertificateDer::pem_file_iter(path) {
        for c in it.flatten() {
            let _ = out.add(c);
        }
    }
}

/// The trust anchors LWP/IO::Socket::SSL would use.
fn roots(tls: &TlsEnv) -> rustls::RootCertStore {
    let mut store = rustls::RootCertStore::empty();
    let mozilla = |store: &mut rustls::RootCertStore| {
        store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    };
    let add_file = |store: &mut rustls::RootCertStore, f: &str| {
        if f == MOZILLA_CA {
            mozilla(store);
        } else {
            load_certs(f, store);
        }
    };
    let add_dir = |store: &mut rustls::RootCertStore, d: &str| {
        if let Ok(rd) = std::fs::read_dir(d) {
            let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            files.sort();
            for p in files {
                if p.is_file() {
                    load_certs(&p.to_string_lossy(), store);
                }
            }
        }
    };
    if tls.ca_file.is_some() || tls.ca_path.is_some() {
        if let Some(f) = &tls.ca_file {
            add_file(&mut store, f);
        }
        if let Some(d) = &tls.ca_path {
            add_dir(&mut store, d);
        }
        return store;
    }
    // IO::Socket::SSL::default_ca, then Mozilla::CA.
    if let Ok(f) = std::env::var("SSL_CERT_FILE")
        && std::path::Path::new(&f).is_file()
    {
        load_certs(&f, &mut store);
        return store;
    }
    if let Ok(d) = std::env::var("SSL_CERT_DIR")
        && std::path::Path::new(&d).is_dir()
    {
        add_dir(&mut store, &d);
        return store;
    }
    if std::path::Path::new("/etc/ssl/cert.pem").is_file() {
        load_certs("/etc/ssl/cert.pem", &mut store);
        return store;
    }
    mozilla(&mut store);
    store
}

/// Verifies the chain but not the host name (`verify_hostname => 0`).
#[derive(Debug)]
struct NoHostnameVerifier(Arc<rustls::client::WebPkiServerVerifier>);

impl rustls::client::danger::ServerCertVerifier for NoHostnameVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls_pki_types::CertificateDer<'_>,
        intermediates: &[rustls_pki_types::CertificateDer<'_>],
        server_name: &rustls_pki_types::ServerName<'_>,
        ocsp: &[u8],
        now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        match self
            .0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
        {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::NotValidForName
                | rustls::CertificateError::NotValidForNameContext { .. },
            )) => Ok(rustls::client::danger::ServerCertVerified::assertion()),
            r => r,
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls_pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls_pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

fn tls_connect(
    stream: Box<dyn Stream>,
    host: &str,
    tls: &TlsEnv,
) -> Result<Box<dyn Stream>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = Arc::new(roots(tls));
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    let config = if tls.verify_hostname {
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        let v = rustls::client::WebPkiServerVerifier::builder_with_provider(roots, provider)
            .build()
            .map_err(|e| e.to_string())?;
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoHostnameVerifier(v)))
            .with_no_client_auth()
    };
    let name = rustls_pki_types::ServerName::try_from(host.to_string())
        .map_err(|_| format!("Bad hostname '{host}'"))?;
    let conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
    let mut s = rustls::StreamOwned::new(conn, stream);
    // Handshake now, so certificate errors are connect errors (as in LWP).
    while s.conn.is_handshaking() {
        if let Err(e) = s.conn.complete_io(&mut s.sock) {
            let msg = match e.get_ref().and_then(|x| x.downcast_ref::<rustls::Error>()) {
                Some(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::NotValidForName
                    | rustls::CertificateError::NotValidForNameContext { .. },
                )) => "hostname verification failed".to_string(),
                Some(rustls::Error::InvalidCertificate(_)) => {
                    "certificate verify failed".to_string()
                }
                Some(other) => format!("SSL connect attempt failed {other}"),
                None => format!("SSL connect attempt failed {}", errno_text(&e)),
            };
            return Err(msg);
        }
    }
    Ok(Box::new(s))
}

// ---------------------------------------------------------------- FTP

struct Ftp {
    r: BufReader<TcpStream>,
    w: TcpStream,
    code: u16,
    message: String,
}

impl Ftp {
    /// Read a (possibly multi-line) reply.
    fn response(&mut self) -> std::io::Result<u16> {
        let mut msg = Vec::new();
        let first = read_line(&mut self.r)?.unwrap_or_default();
        let code: u16 = first.get(..3).and_then(|c| c.parse().ok()).unwrap_or(0);
        msg.push(first.get(4..).unwrap_or("").to_string());
        if first.as_bytes().get(3) == Some(&b'-') {
            let end = format!("{code} ");
            while let Some(l) = read_line(&mut self.r)? {
                if l.starts_with(&end) {
                    msg.push(l[4..].to_string());
                    break;
                }
                msg.push(
                    l.strip_prefix(&format!("{code}-"))
                        .unwrap_or(&l)
                        .to_string(),
                );
            }
        }
        self.code = code;
        self.message = msg.join("\n");
        Ok(code)
    }
    fn cmd(&mut self, c: &str) -> std::io::Result<u16> {
        self.w.write_all(format!("{c}\r\n").as_bytes())?;
        self.w.flush()?;
        self.response()
    }
    fn ok(&mut self, c: &str) -> std::io::Result<bool> {
        Ok((200..400).contains(&self.cmd(c)?))
    }
    /// PASV and connect the data connection.
    fn pasv(&mut self) -> std::io::Result<Option<TcpStream>> {
        if self.cmd("PASV")? != 227 {
            return Ok(None);
        }
        let m = self.message.clone();
        let nums: Vec<u16> = m
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if nums.len() < 6 {
            return Ok(None);
        }
        let n = &nums[nums.len() - 6..];
        let host = format!("{}.{}.{}.{}", n[0], n[1], n[2], n[3]);
        Ok(Some(connect_tcp(&host, n[4] * 256 + n[5])?))
    }
}

fn ftp_get(url: &Url) -> Response {
    let tcp = match connect_tcp(&url.host, url.port) {
        Ok(s) => s,
        Err(e) => return Response::error(500, errno_text(&e)),
    };
    let w = match tcp.try_clone() {
        Ok(w) => w,
        Err(e) => return Response::error(500, errno_text(&e)),
    };
    let mut f = Ftp {
        r: BufReader::new(tcp),
        w,
        code: 0,
        message: String::new(),
    };
    match ftp_session(&mut f, url) {
        Ok(r) => r,
        Err(e) => Response::error(500, errno_text(&e)),
    }
}

fn ftp_session(f: &mut Ftp, url: &Url) -> std::io::Result<Response> {
    if !(200..300).contains(&f.response()?) {
        return Ok(Response::error(500, f.message.clone()));
    }
    // URI::ftp: user defaults to "anonymous", whose password is "anonymous@".
    let (user, pass) = match &url.userinfo {
        Some(ui) => {
            let (u, p) = match ui.split_once(':') {
                Some((u, p)) => (u, Some(p)),
                None => (ui.as_str(), None),
            };
            let u = String::from_utf8_lossy(&percent_decode(u)).into_owned();
            let p = p.map(|p| String::from_utf8_lossy(&percent_decode(p)).into_owned());
            let p =
                p.or_else(|| (u == "anonymous" || u == "ftp").then(|| "anonymous@".to_string()));
            (u, p)
        }
        None => ("anonymous".to_string(), Some("anonymous@".to_string())),
    };
    let mut ok = f.cmd(&format!("USER {user}"))?;
    if (300..400).contains(&ok) {
        ok = f.cmd(&format!("PASS {}", pass.unwrap_or_default()))?;
    }
    if !(200..300).contains(&ok) {
        let m = f.message.trim_end_matches('\n').to_string();
        return Ok(Response::error(401, m));
    }
    f.cmd("PWD")?;
    let path = url.path_query.split('?').next().unwrap_or("");
    let mut segs: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let last = segs.pop().unwrap_or_default();
    let (file_esc, params) = match last.split_once(';') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (last, String::new()),
    };
    let file = String::from_utf8_lossy(&percent_decode(&file_esc)).into_owned();
    let ascii = params.split(';').any(|p| p == "type=a");
    f.cmd(if ascii { "TYPE A" } else { "TYPE I" })?;
    for s in &segs {
        let s = String::from_utf8_lossy(&percent_decode(s)).into_owned();
        if !f.ok(&format!("CWD {s}"))? {
            return Ok(Response::error(404, format!("Can't chdir to {s}")));
        }
    }
    if !file.is_empty() {
        f.cmd(&format!("MDTM {file}"))?;
        if let Some(mut data) = f.pasv()? {
            let c = f.cmd(&format!("RETR {file}"))?;
            if (100..200).contains(&c) {
                let mut body = Vec::new();
                data.read_to_end(&mut body)?;
                drop(data);
                let fin = f.response()?;
                if !(200..300).contains(&fin) {
                    return Ok(Response::error(
                        500,
                        format!("FTP close response: {fin} {}", f.message),
                    ));
                }
                if ascii {
                    body = String::from_utf8_lossy(&body)
                        .replace("\r\n", "\n")
                        .into_bytes();
                }
                return Ok(Response {
                    code: 200,
                    message: "OK".into(),
                    headers: Vec::new(),
                    body,
                });
            }
        }
        if !(400..600).contains(&f.code) {
            return Ok(Response::error(400, format!("FTP return code {}", f.code)));
        }
        if !f.ok(&format!("CWD {file}"))? {
            return Ok(Response::error(404, format!("File '{file}' not found")));
        }
    }
    // A directory: LWP returns the listing (text/ftp-dir-listing).
    let mut lines = Vec::new();
    if let Some(mut data) = f.pasv()?
        && (100..200).contains(&f.cmd("LIST")?)
    {
        let mut b = Vec::new();
        data.read_to_end(&mut b)?;
        drop(data);
        f.response()?;
        for l in String::from_utf8_lossy(&b).lines() {
            lines.push(l.trim_end_matches('\r').to_string());
        }
    }
    let mut body = lines.join("\n");
    body.push('\n');
    Ok(Response {
        code: 200,
        message: "OK".into(),
        headers: Vec::new(),
        body: body.into_bytes(),
    })
}

// ---------------------------------------------------------------- request

/// `$ua->request(GET $source)`: follow redirects; the final response.
fn request(source: &str, env: &EnvOverrides) -> Response {
    let tls = tls_env(env);
    let mut url = match parse_url(source) {
        Ok(u) => u,
        Err(e) => return Response::error(400, e),
    };
    let mut redirects = 0;
    loop {
        let resp = match url.scheme.as_str() {
            "http" | "https" => http_simple(&url, &tls, env),
            "ftp" => match proxy_for(&url, env) {
                Some(_) => http_simple(&url, &tls, env),
                None => ftp_get(&url),
            },
            s => Response::error(501, format!("Protocol scheme '{s}' is not supported")),
        };
        if !matches!(resp.code, 301 | 302 | 303 | 307 | 308) || redirects >= 7 {
            return resp;
        }
        let loc = resp.header("Location").unwrap_or("").to_string();
        let next = match url.join(&loc) {
            Ok(u) => u,
            Err(_) => return resp,
        };
        if next.scheme == "file" {
            return resp;
        }
        url = next;
        redirects += 1;
    }
}

/// Run-wide remote state: biber's `%REMOTE_MAP` cache, its temp directory
/// and the environment changes it made.
#[derive(Clone, Debug, Default)]
pub struct RemoteState {
    /// URL → downloaded file.
    pub cache: BTreeMap<String, String>,
    pub env: EnvOverrides,
    tempdir: Option<std::path::PathBuf>,
    _guard: Option<Arc<tempfile::TempDir>>,
}

impl RemoteState {
    /// biber's temp directory (`biber_tmp_XXXX`), created on first use.
    pub fn tempdir(&mut self, keep: bool) -> std::io::Result<std::path::PathBuf> {
        if let Some(d) = &self.tempdir {
            return Ok(d.clone());
        }
        let td = tempfile::Builder::new()
            .prefix("biber_tmp_")
            .rand_bytes(4)
            .tempdir()?;
        let p = td.path().to_path_buf();
        if keep {
            let _ = td.keep();
        } else {
            self._guard = Some(Arc::new(td));
        }
        self.tempdir = Some(p.clone());
        Ok(p)
    }
}

/// The remote branch of `locate_data_file` (Utils.pm:231-305): fetch
/// `source` into biber's temp directory (once per run) and return the local
/// path. A failed fetch is an ERROR; with `--nodieonerror` the (empty) temp
/// file is used, as in biber.
pub fn locate(
    cfg: &mut crate::config::Config,
    log: &mut crate::log::Logger,
    source: &str,
    tls: bool,
) -> Result<String, crate::log::Fatal> {
    crate::info!(
        log,
        "Data source '{source}' is a remote BibTeX data source - fetching ..."
    );
    if let Some(p) = cfg.remote.cache.get(source) {
        let p = p.clone();
        crate::info!(log, "Found '{source}' in remote source cache");
        return Ok(p);
    }
    if tls {
        let b = &cfg.biber;
        let nointernalca = b.get_str("ssl-nointernalca").is_some();
        let noverify = b.get_str("ssl-noverify-host").is_some();
        setup_ssl_env(&mut cfg.remote.env, nointernalca, noverify);
    }
    let keep = cfg.biber.is_true("noremove_tmp_dir");
    let dir = match cfg.remote.tempdir(keep) {
        Ok(d) => d,
        Err(e) => {
            crate::biber_error!(log, "Cannot create temporary directory: {e}")?;
            return Ok(String::new());
        }
    };
    let (path, err) = match fetch_to_file(source, &dir, &cfg.remote.env) {
        Ok(r) => r,
        Err(e) => {
            crate::biber_error!(
                log,
                "Cannot write temporary file in '{}': {e}",
                dir.display()
            )?;
            return Ok(String::new());
        }
    };
    if let Some(status) = err {
        crate::biber_error!(log, "Could not fetch '{source}' (HTTP error: {status})")?;
    }
    cfg.remote.cache.insert(source.to_string(), path.clone());
    Ok(path)
}

/// Fetch `source` into a new `biber_remote_data_source_XXXXX.bib` file in
/// `dir`. Returns the file's path and, if the fetch failed, the status line
/// (the file then stays empty, as in biber).
pub fn fetch_to_file(
    source: &str,
    dir: &std::path::Path,
    env: &EnvOverrides,
) -> std::io::Result<(String, Option<String>)> {
    let tf = tempfile::Builder::new()
        .prefix("biber_remote_data_source_")
        .suffix(".bib")
        .rand_bytes(5)
        .tempfile_in(dir)?;
    let (mut file, path) = tf.keep().map_err(|e| e.error)?;
    let path = path.to_string_lossy().into_owned();
    let resp = request(source, env);
    if !resp.is_success() {
        return Ok((path, Some(resp.status_line())));
    }
    file.write_all(&resp.body)?;
    Ok((path, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        let u = parse_url("http://user:p%40ss@example.org:8080/a b.bib?x=1#frag").unwrap();
        assert_eq!(u.host, "example.org");
        assert_eq!(u.port, 8080);
        assert_eq!(u.path_query, "/a%20b.bib?x=1");
        assert_eq!(u.host_header(), "example.org:8080");
        assert_eq!(base64(b"user:p@ss"), "dXNlcjpwQHNz");
        let j = u.join("../c/d.bib").unwrap();
        assert_eq!(j.path_query, "/c/d.bib");
        let j = u.join("/x").unwrap();
        assert_eq!(j.full, "http://user:p%40ss@example.org:8080/x");
        let j = u.join("https://other/y").unwrap();
        assert_eq!((j.scheme.as_str(), j.port), ("https", 443));
        assert_eq!(remote_scheme("HTTP://x"), None);
        assert_eq!(remote_scheme("ftps://x"), Some(true));
    }

    #[test]
    fn status_lines() {
        assert_eq!(
            Response::error(503, "").status_line(),
            "503 Service Unavailable"
        );
        assert_eq!(Response::error(599, "").status_line(), "599 Unknown code");
        assert_eq!(Response::error(404, "Nope").status_line(), "404 Nope");
    }
}

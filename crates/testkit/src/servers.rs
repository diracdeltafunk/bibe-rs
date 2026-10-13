//! Local HTTP, HTTPS and FTP servers for the remote data source tests
//! (DC-032): bibers' own tests (`crates/bibers/tests/remote.rs`) and the
//! O1/O3 differential runs (`cargo xtask remote`). Network-free: everything
//! listens on 127.0.0.1 with an OS-assigned port.
//!
//! The TLS material in `crates/testkit/data/tls/` is test-only: a CA
//! (`ca.pem`), a certificate for `localhost`/`127.0.0.1` and one for
//! `wrong.invalid` (hostname mismatch).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

/// One canned HTTP response.
#[derive(Clone, Debug)]
pub struct Route {
    pub status: u16,
    /// The reason phrase (LWP puts it in its status line verbatim).
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Send the body with `Transfer-Encoding: chunked`.
    pub chunked: bool,
}

impl Route {
    /// `200 OK` with `body`.
    pub fn ok(body: impl Into<Vec<u8>>) -> Route {
        Route {
            status: 200,
            reason: "OK".into(),
            headers: vec![("Content-Type".into(), "text/plain".into())],
            body: body.into(),
            chunked: false,
        }
    }

    /// A status with an HTML-ish body.
    pub fn status(status: u16, reason: &str) -> Route {
        Route {
            status,
            reason: reason.into(),
            headers: vec![("Content-Type".into(), "text/html".into())],
            body: format!("<html><body>{status} {reason}</body></html>\n").into_bytes(),
            chunked: false,
        }
    }

    /// A redirect to `location`.
    pub fn redirect(status: u16, location: &str) -> Route {
        let mut r = Route::status(
            status,
            if status == 301 {
                "Moved Permanently"
            } else {
                "Found"
            },
        );
        r.headers.push(("Location".into(), location.into()));
        r
    }

    pub fn chunked(mut self) -> Route {
        self.chunked = true;
        self
    }
}

/// Which certificate an HTTPS server presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cert {
    /// Valid for `localhost` and `127.0.0.1`, signed by the test CA.
    Localhost,
    /// Signed by the test CA, for `wrong.invalid` only.
    WrongHost,
}

/// Directory of the test TLS material.
pub fn tls_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/tls")
}

/// The test CA certificate (for `PERL_LWP_SSL_CA_FILE`).
pub fn ca_file() -> PathBuf {
    tls_dir().join("ca.pem")
}

/// A running server; requests are recorded (request line and headers).
pub struct Server {
    pub port: u16,
    pub log: Arc<Mutex<Vec<String>>>,
}

impl Server {
    /// `scheme://127.0.0.1:port`.
    pub fn base(&self, scheme: &str) -> String {
        format!("{scheme}://127.0.0.1:{}", self.port)
    }
    /// The recorded requests so far.
    pub fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

fn tls_config(cert: Cert) -> Arc<rustls::ServerConfig> {
    use rustls_pki_types::pem::PemObject;
    use rustls_pki_types::{CertificateDer, PrivateKeyDer};
    let name = match cert {
        Cert::Localhost => "localhost",
        Cert::WrongHost => "wronghost",
    };
    let d = tls_dir();
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_file_iter(d.join(format!("{name}.pem")))
            .expect("cert")
            .collect::<Result<_, _>>()
            .expect("cert");
    let key = PrivateKeyDer::from_pem_file(d.join(format!("{name}.key"))).expect("key");
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("server config");
    Arc::new(cfg)
}

/// Start an HTTP (or, with `tls`, HTTPS) server answering `routes` (by
/// path, including any query string); other paths get `404 Not Found`.
pub fn http(routes: BTreeMap<String, Route>, tls: Option<Cert>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let routes = Arc::new(routes);
    let tls = tls.map(tls_config);
    let log2 = log.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let routes = routes.clone();
            let log = log2.clone();
            let tls = tls.clone();
            thread::spawn(move || {
                let _ = match tls {
                    Some(cfg) => {
                        let Ok(conn) = rustls::ServerConnection::new(cfg) else {
                            return;
                        };
                        let mut s = rustls::StreamOwned::new(conn, stream);
                        serve_http(&mut s, &routes, &log)
                    }
                    None => {
                        let mut s = stream;
                        serve_http(&mut s, &routes, &log)
                    }
                };
            });
        }
    });
    Server { port, log }
}

fn serve_http<S: Read + Write>(
    s: &mut S,
    routes: &BTreeMap<String, Route>,
    log: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if s.read(&mut b)? == 0 {
            return Ok(());
        }
        head.push(b[0]);
        if head.len() > 65536 {
            return Ok(());
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    log.lock().unwrap().push(head.clone());
    let target = head.split_whitespace().nth(1).unwrap_or("/").to_string();
    // Absolute-form (proxy) requests: keep the path.
    let path = match target.find("://") {
        Some(i) => target[i + 3..]
            .find('/')
            .map_or("/".to_string(), |j| target[i + 3 + j..].to_string()),
        None => target,
    };
    let route = routes
        .get(&path)
        .cloned()
        .unwrap_or_else(|| Route::status(404, "Not Found"));
    let mut out = format!("HTTP/1.1 {} {}\r\n", route.status, route.reason).into_bytes();
    for (k, v) in &route.headers {
        out.extend(format!("{k}: {v}\r\n").bytes());
    }
    if route.chunked {
        out.extend(b"Transfer-Encoding: chunked\r\n");
    } else {
        out.extend(format!("Content-Length: {}\r\n", route.body.len()).bytes());
    }
    out.extend(b"Connection: close\r\n\r\n");
    if route.chunked {
        for c in route.body.chunks(7) {
            out.extend(format!("{:x}\r\n", c.len()).bytes());
            out.extend_from_slice(c);
            out.extend(b"\r\n");
        }
        out.extend(b"0\r\n\r\n");
    } else {
        out.extend_from_slice(&route.body);
    }
    s.write_all(&out)?;
    s.flush()
}

/// Start a minimal passive-mode FTP server serving `files` (path without the
/// leading `/`, e.g. `pub/a.bib`). User `bad` is refused; anyone else may log
/// in. Commands are recorded.
pub fn ftp(files: BTreeMap<String, Vec<u8>>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let files = Arc::new(files);
    let log2 = log.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let files = files.clone();
            let log = log2.clone();
            thread::spawn(move || {
                let _ = serve_ftp(stream, &files, &log);
            });
        }
    });
    Server { port, log }
}

fn serve_ftp(
    stream: TcpStream,
    files: &BTreeMap<String, Vec<u8>>,
    log: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    let mut w = stream.try_clone()?;
    let mut r = BufReader::new(stream);
    let reply = |w: &mut TcpStream, s: &str| w.write_all(format!("{s}\r\n").as_bytes());
    reply(&mut w, "220 bibers test FTP server ready.")?;
    let mut cwd = String::new();
    let mut user = String::new();
    let mut pasv: Option<TcpListener> = None;
    let dirs: Vec<String> = files
        .keys()
        .flat_map(|k| {
            let parts: Vec<&str> = k.split('/').collect();
            (1..parts.len()).map(move |i| parts[..i].join("/"))
        })
        .collect();
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let line = line.trim_end().to_string();
        log.lock().unwrap().push(line.clone());
        let (cmd, arg) = line.split_once(' ').unwrap_or((&line, ""));
        let path_of = |cwd: &str, a: &str| {
            let a = a.trim_start_matches('/');
            if cwd.is_empty() {
                a.to_string()
            } else {
                format!("{cwd}/{a}")
            }
        };
        match cmd.to_ascii_uppercase().as_str() {
            "USER" => {
                user = arg.to_string();
                reply(&mut w, "331 Please specify the password.")?
            }
            "PASS" => {
                if user == "bad" {
                    reply(&mut w, "530 Login incorrect.")?
                } else {
                    reply(&mut w, "230 Login successful.")?
                }
            }
            "PWD" => reply(&mut w, "257 \"/\" is the current directory")?,
            "TYPE" => reply(&mut w, "200 Switching to Binary mode.")?,
            "CWD" => {
                let p = if arg == "/" {
                    String::new()
                } else {
                    path_of(&cwd, arg)
                };
                if p.is_empty() || dirs.contains(&p) {
                    cwd = p;
                    reply(&mut w, "250 Directory successfully changed.")?
                } else {
                    reply(&mut w, "550 Failed to change directory.")?
                }
            }
            "MDTM" => {
                if files.contains_key(&path_of(&cwd, arg)) {
                    reply(&mut w, "213 20240101000000")?
                } else {
                    reply(&mut w, "550 Could not get file modification time.")?
                }
            }
            "PASV" => {
                let l = TcpListener::bind("127.0.0.1:0")?;
                let p = l.local_addr()?.port();
                pasv = Some(l);
                reply(
                    &mut w,
                    &format!(
                        "227 Entering Passive Mode (127,0,0,1,{},{}).",
                        p >> 8,
                        p & 0xff
                    ),
                )?
            }
            "RETR" => match (files.get(&path_of(&cwd, arg)), pasv.take()) {
                (Some(data), Some(l)) => {
                    reply(
                        &mut w,
                        &format!(
                            "150 Opening BINARY mode data connection for {arg} ({} bytes).",
                            data.len()
                        ),
                    )?;
                    let (mut d, _) = l.accept()?;
                    d.write_all(data)?;
                    drop(d);
                    reply(&mut w, "226 Transfer complete.")?
                }
                (None, _) => reply(&mut w, "550 Failed to open file.")?,
                (_, None) => reply(&mut w, "425 Use PORT or PASV first.")?,
            },
            "LIST" | "NLST" => match pasv.take() {
                Some(l) => {
                    reply(&mut w, "150 Here comes the directory listing.")?;
                    let (d, _) = l.accept()?;
                    drop(d);
                    reply(&mut w, "226 Directory send OK.")?
                }
                None => reply(&mut w, "425 Use PORT or PASV first.")?,
            },
            "QUIT" => {
                reply(&mut w, "221 Goodbye.")?;
                return Ok(());
            }
            _ => reply(&mut w, "502 Command not implemented.")?,
        }
    }
}

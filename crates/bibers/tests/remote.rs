//! Remote data sources (DC-032) against local HTTP, HTTPS and FTP servers
//! (network-free). Expected messages are biber 2.22's, recorded with
//! `cargo xtask remote --impl o3` against the same servers; that command
//! also compares bibers with O1/O3 over more scenarios.
//!
//! Also: biber's t/remote-files.t (with a local copy of the remote `.bib`),
//! and the error paths of `--output-encoding` and `--recodedata`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use bibers_testkit::servers::{self, Cert, Route};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const BIB: &str = "@BOOK{remote1,\n  AUTHOR = {Edward Encalcer},\n  YEAR = {1999},\n  TITLE = {A remote title},\n  PUBLISHER = {A press}\n}\n";

/// A `.bcf` (t/tdata/encoding6.bcf) whose data sources are `urls`.
fn bcf(urls: &[String]) -> String {
    let t = std::fs::read_to_string(repo().join("tests/corpus/ported/encoding-4/encoding6.bcf"))
        .unwrap();
    let ds =
        "    <bcf:datasource type=\"file\" datatype=\"bibtex\">encoding6.bib</bcf:datasource>\n";
    let new: String = urls
        .iter()
        .map(|u| ds.replace("encoding6.bib", &u.replace('&', "&amp;")))
        .collect();
    t.replace(ds, &new)
}

struct Run {
    code: i32,
    bbl: String,
    stdout: String,
}

impl Run {
    /// The WARN/ERROR lines of the screen output.
    fn messages(&self) -> Vec<String> {
        self.stdout
            .lines()
            .filter(|l| l.starts_with("WARN - ") || l.starts_with("ERROR - "))
            .map(str::to_string)
            .collect()
    }
}

fn run_in(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bibers"));
    cmd.arg("--noconf").args(args).current_dir(dir);
    for v in [
        "PERL_LWP_SSL_CA_FILE",
        "PERL_LWP_SSL_CA_PATH",
        "PERL_LWP_SSL_VERIFY_HOSTNAME",
        "HTTPS_CA_FILE",
        "HTTPS_CA_DIR",
        "http_proxy",
        "HTTP_PROXY",
        "https_proxy",
        "HTTPS_PROXY",
        "ftp_proxy",
        "FTP_PROXY",
        "no_proxy",
        "NO_PROXY",
    ] {
        cmd.env_remove(v);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let bbl = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .find(|e| e.path().extension().is_some_and(|x| x == "bbl"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .unwrap_or_default();
    Run {
        code: out.status.code().unwrap_or(-1),
        bbl,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
    }
}

fn run(urls: &[String], args: &[&str], env: &[(&str, &str)]) -> Run {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("remote.bcf"), bcf(urls)).unwrap();
    let mut a = args.to_vec();
    a.push("remote.bcf");
    run_in(dir.path(), &a, env)
}

fn routes() -> BTreeMap<String, Route> {
    let mut r = BTreeMap::new();
    r.insert("/a.bib".to_string(), Route::ok(BIB));
    r.insert("/chunked.bib".to_string(), Route::ok(BIB).chunked());
    r.insert(
        "/gone.bib".to_string(),
        Route::status(410, "Gone Away Forever"),
    );
    r.insert("/noreason.bib".to_string(), Route::status(503, ""));
    r.insert("/redir.bib".to_string(), Route::redirect(302, "/a.bib"));
    r.insert("/loop.bib".to_string(), Route::redirect(302, "/loop.bib"));
    r
}

#[test]
fn http_fetch_and_errors() {
    let s = servers::http(routes(), None);
    let base = s.base("http");
    let r = run(&[format!("{base}/a.bib")], &[], &[]);
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert!(r.bbl.contains("\\field{title}{A remote title}"));
    assert!(r.stdout.contains(&format!(
        "INFO - Data source '{base}/a.bib' is a remote BibTeX data source - fetching ..."
    )));
    let req = s.requests().join("");
    assert!(req.starts_with("GET /a.bib HTTP/1.1\r\n"), "{req}");
    assert!(req.contains("User-Agent: Mozilla/5.0\r\n"));
    assert!(req.contains("Zotero-Allowed-Request: 1\r\n"));

    let r = run(&[format!("{base}/chunked.bib")], &[], &[]);
    assert!(r.bbl.contains("\\field{title}{A remote title}"));

    for (path, status) in [
        ("/missing.bib", "404 Not Found"),
        ("/gone.bib", "410 Gone Away Forever"),
        ("/noreason.bib", "503 Service Unavailable"),
        ("/loop.bib", "302 Found"),
    ] {
        let r = run(&[format!("{base}{path}")], &[], &[]);
        assert_eq!(r.code, 2);
        assert_eq!(
            r.messages(),
            vec![format!(
                "ERROR - Could not fetch '{base}{path}' (HTTP error: {status})"
            )]
        );
    }
    // --nodieonerror: the empty temp file is read.
    let r = run(&[format!("{base}/missing.bib")], &["--nodieonerror"], &[]);
    let m = r.messages();
    assert_eq!(m.len(), 2, "{m:?}");
    assert!(m[1].starts_with("WARN - Data source '") && m[1].contains("biber_remote_data_source_"));

    let r = run(&[format!("{base}/redir.bib")], &[], &[]);
    assert_eq!(r.code, 0);
    // The same source twice: fetched once (as in biber).
    let n = s.requests().len();
    let r = run(
        &[format!("{base}/a.bib"), format!("{base}/a.bib")],
        &[],
        &[],
    );
    assert_eq!(r.code, 0);
    assert_eq!(s.requests().len(), n + 1);
    // Only lower-case schemes are remote.
    let up = format!("HTTP://127.0.0.1:{}/a.bib", s.port);
    let r = run(std::slice::from_ref(&up), &[], &[]);
    assert_eq!(r.messages(), vec![format!("ERROR - Cannot find '{up}'!")]);
    // strerror() texts are platform-specific.
    if cfg!(unix) {
        let r = run(&["http://127.0.0.1:1/a.bib".to_string()], &[], &[]);
        assert_eq!(
            r.messages(),
            vec!["ERROR - Could not fetch 'http://127.0.0.1:1/a.bib' (HTTP error: 500 Can't connect to 127.0.0.1:1 (Connection refused))".to_string()]
        );
    }
}

#[test]
fn https_trust() {
    let ca = servers::ca_file();
    let ca = ca.to_str().unwrap();
    let ok = servers::http(routes(), Some(Cert::Localhost));
    let wrong = servers::http(routes(), Some(Cert::WrongHost));
    let url = format!("{}/a.bib", ok.base("https"));
    let wurl = format!("https://localhost:{}/a.bib", wrong.port);

    let r = run(
        std::slice::from_ref(&url),
        &[],
        &[("PERL_LWP_SSL_CA_FILE", ca)],
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    // The Mozilla list does not trust the test CA; --ssl-noverify-host only
    // disables the host name check.
    let port = ok.port;
    for args in [&[][..], &["--ssl-noverify-host"][..]] {
        let r = run(std::slice::from_ref(&url), args, &[]);
        assert_eq!(
            r.messages(),
            vec![format!(
                "ERROR - Could not fetch '{url}' (HTTP error: 500 Can't connect to 127.0.0.1:{port} (certificate verify failed))"
            )]
        );
    }
    let r = run(
        std::slice::from_ref(&wurl),
        &[],
        &[("PERL_LWP_SSL_CA_FILE", ca)],
    );
    assert_eq!(
        r.messages(),
        vec![format!(
            "ERROR - Could not fetch '{wurl}' (HTTP error: 500 Can't connect to localhost:{} (hostname verification failed))",
            wrong.port
        )]
    );
    let r = run(
        std::slice::from_ref(&wurl),
        &["--ssl-noverify-host"],
        &[("PERL_LWP_SSL_CA_FILE", ca)],
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
}

#[test]
fn ftp() {
    let mut files = BTreeMap::new();
    files.insert("pub/a.bib".to_string(), BIB.as_bytes().to_vec());
    let s = servers::ftp(files);
    let base = s.base("ftp");
    let r = run(&[format!("{base}/pub/a.bib")], &[], &[]);
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert!(r.bbl.contains("\\field{title}{A remote title}"));
    assert_eq!(
        s.requests(),
        [
            "USER anonymous",
            "PASS anonymous@",
            "PWD",
            "TYPE I",
            "CWD pub",
            "MDTM a.bib",
            "PASV",
            "RETR a.bib"
        ]
    );
    for (url, status) in [
        (
            format!("{base}/pub/missing.bib"),
            "404 File 'missing.bib' not found",
        ),
        (format!("{base}/nodir/a.bib"), "404 Can't chdir to nodir"),
        (
            format!("ftp://bad:x@127.0.0.1:{}/pub/a.bib", s.port),
            "401 Login incorrect.",
        ),
        (
            format!("ftps://127.0.0.1:{}/pub/a.bib", s.port),
            "501 Protocol scheme 'ftps' is not supported",
        ),
    ] {
        let r = run(std::slice::from_ref(&url), &[], &[]);
        assert_eq!(
            r.messages(),
            vec![format!(
                "ERROR - Could not fetch '{url}' (HTTP error: {status})"
            )]
        );
    }
}

/// biber's t/remote-files.t, with the remote `.bib` served locally.
#[test]
fn remote_files_t() {
    let bib = "@article{SchillerCND2010,\n  author = {Schiller, Todd W. and Chen, Yixin and El Naqa, Issam and Deasy, Joseph O.},\n  title = {Modeling Radiation-induced Lung Injury Risk with an Ensemble of Support Vector Machines},\n  journal = {Neurocomputing},\n  volume = {73},\n  number = {10-12},\n  month = jun,\n  year = {2010},\n  doi = {10.1016/j.neucom.2009.09.023}\n}\n";
    let mut r = BTreeMap::new();
    r.insert("/schiller.bib".to_string(), Route::ok(bib));
    let s = servers::http(r, None);
    let dir = tempfile::tempdir().unwrap();
    let t = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/remote/remote-files.bcf"),
    )
    .unwrap();
    let t = t.replace(
        "https://raw.githubusercontent.com/twschiller/public-bib/refs/heads/master/schiller.bib",
        &format!("{}/schiller.bib", s.base("http")),
    );
    std::fs::write(dir.path().join("remote-files.bcf"), t).unwrap();
    let r = run_in(
        dir.path(),
        &[
            "--sortlocale=en_GB.UTF-8",
            "--nodieonerror",
            "remote-files.bcf",
        ],
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    let dl1 = r#"    \entry{SchillerCND2010}{article}{}{}
      \name{author}{4}{}{%
        {{un=0,uniquepart=base,hash=c606849f9ce94faa18c52562c39b6f92}{%
           family={Schiller},
           familyi={S\bibinitperiod},
           given={Todd\bibnamedelima W.},
           giveni={T\bibinitperiod\bibinitdelim W\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=20154ea5f879b7b3256febae2ee215b6}{%
           family={Chen},
           familyi={C\bibinitperiod},
           given={Yixin},
           giveni={Y\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=47c2d9d369b1f19efa50d61551e7a69b}{%
           family={El\bibnamedelima Naqa},
           familyi={E\bibinitperiod\bibinitdelim N\bibinitperiod},
           given={Issam},
           giveni={I\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=f967f6b51246cf632661681073e1b6d8}{%
           family={Deasy},
           familyi={D\bibinitperiod},
           given={Joseph\bibnamedelima O.},
           giveni={J\bibinitperiod\bibinitdelim O\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{cc9430ab59f948048f76cb58659fc218}
      \strng{fullhash}{104a7a1c8b6937b5c35dc05b94f446b9}
      \strng{fullhashraw}{104a7a1c8b6937b5c35dc05b94f446b9}
      \strng{bibnamehash}{cc9430ab59f948048f76cb58659fc218}
      \strng{authorbibnamehash}{cc9430ab59f948048f76cb58659fc218}
      \strng{authornamehash}{cc9430ab59f948048f76cb58659fc218}
      \strng{authorfullhash}{104a7a1c8b6937b5c35dc05b94f446b9}
      \strng{authorfullhashraw}{104a7a1c8b6937b5c35dc05b94f446b9}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{journaltitle}{Neurocomputing}
      \field{month}{6}
      \field{number}{10-12}
      \field{title}{Modeling Radiation-induced Lung Injury Risk with an Ensemble of Support Vector Machines}
      \field{volume}{73}
      \field{year}{2010}
      \verb{doi}
      \verb 10.1016/j.neucom.2009.09.023
      \endverb
    \endentry
"#;
    assert!(r.bbl.contains(dl1), "{}", r.bbl);
}

#[test]
fn output_encoding_errors() {
    let dir = tempfile::tempdir().unwrap();
    let c = repo().join("tests/corpus/ported/encoding-4");
    for f in ["encoding6.bcf", "encoding6.bib"] {
        std::fs::copy(c.join(f), dir.path().join(f)).unwrap();
    }
    // biber dies (a Perl error, 0-byte .bbl); bibers reports an ERROR.
    let r = run_in(
        dir.path(),
        &["--output-encoding=gb18030", "encoding6.bcf"],
        &[],
    );
    assert_eq!(r.code, 2);
    assert!(
        r.messages()
            .contains(&"ERROR - Unknown output encoding 'gb18030'".to_string())
    );
    let r = run_in(
        dir.path(),
        &["--output-encoding=UTF-7", "encoding6.bcf"],
        &[],
    );
    assert_eq!(r.code, 2);
    assert!(r.messages()[0].contains("is not supported"));
    // A missing --recodedata file (biber: "Couldn't open : No such file").
    // The lookup goes through kpsewhich, which CI runners lack.
    if std::process::Command::new("kpsewhich")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping the --recodedata part: no kpsewhich");
        return;
    }
    let r = run_in(
        dir.path(),
        &["--recodedata=nonexistent-recode.xml", "encoding6.bcf"],
        &[],
    );
    assert_eq!(r.code, 2);
    assert!(
        r.stdout
            .contains("INFO - Using user-defined recode data file ''")
    );
    assert!(
        r.messages()
            .contains(&"ERROR - Cannot find recode data file 'nonexistent-recode.xml'".to_string()),
        "{}",
        r.stdout
    );
}

/// A server that answers every request with `response`, then closes.
fn raw_server(response: &'static [u8]) -> String {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut s = s;
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let _ = s.write_all(response);
        }
    });
    format!("http://{addr}")
}

/// Malformed chunked bodies are fetch errors (exit 2), never panics: the
/// advertised chunk size is not trusted.
#[test]
fn hostile_chunked_responses() {
    for resp in [
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nFFFFFFFFFFFFFFFF\r\nabc"[..],
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n10\r\nabc"[..],
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1FFFFFFFFFFFFFFFF\r\nabc"[..],
    ] {
        let base = raw_server(resp);
        let r = run(&[format!("{base}/a.bib")], &[], &[]);
        assert_eq!(r.code, 2, "{}", r.stdout);
        assert!(
            r.messages().iter().any(|m| m.starts_with("ERROR - ")),
            "{}",
            r.stdout
        );
    }
}

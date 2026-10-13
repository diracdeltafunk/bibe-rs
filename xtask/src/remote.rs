//! `cargo xtask remote`: remote data source scenarios (DC-032) run against
//! local HTTP, HTTPS and FTP servers (`bibers_testkit::servers`), for any
//! implementation, compared with a reference implementation.
//!
//! ```text
//! cargo xtask remote [--impl bibers] [--ref o3] [-v] [FILTER...]
//! ```
//!
//! Each scenario writes a `.bcf` whose data source is a URL, runs the
//! implementation, and records the exit code, the `.bbl`, the normalised
//! WARN/ERROR messages (temp file names → `<REMOTE>`, ports → `<PORT>`) and
//! what the server saw (request lines and selected headers). Scenarios
//! needing DNS (`dns-*`) are skipped unless `--dns` is given.

use anyhow::{Context, Result, bail};
use bibers_testkit::normalize::blg_messages;
use bibers_testkit::servers::{self, Cert, Route, Server};
use bibers_testkit::{Runner, repo_root};
use regex::Regex;
use std::collections::BTreeMap;

use std::process::Command;

const BIB: &str = "@BOOK{remote1,\n  AUTHOR = {Edward Encalcer},\n  YEAR = {1999},\n  TITLE = {A remote title},\n  PUBLISHER = {A press}\n}\n";

struct Servers {
    http: Server,
    https: Server,
    https_wrong: Server,
    ftp: Server,
}

fn start() -> Servers {
    let mut r = BTreeMap::new();
    r.insert("/a.bib".to_string(), Route::ok(BIB));
    r.insert("/a%20b.bib".to_string(), Route::ok(BIB));
    r.insert("/caf%C3%A9.bib".to_string(), Route::ok(BIB));
    r.insert("/chunked.bib".to_string(), Route::ok(BIB).chunked());
    r.insert("/empty.bib".to_string(), Route::ok(""));
    r.insert(
        "/gone.bib".to_string(),
        Route::status(410, "Gone Away Forever"),
    );
    r.insert("/err.bib".to_string(), Route::status(500, "Kaputt"));
    r.insert("/noreason.bib".to_string(), Route::status(503, ""));
    r.insert("/redir.bib".to_string(), Route::redirect(302, "/a.bib"));
    r.insert(
        "/redir-abs.bib".to_string(),
        Route::redirect(301, "REPLACED"),
    );
    r.insert("/loop.bib".to_string(), Route::redirect(302, "/loop.bib"));
    r.insert("/q.bib?x=1&y=2".to_string(), Route::ok(BIB));
    let http = servers::http(r.clone(), None);
    // redir-abs: absolute Location on the same server.
    let mut r2 = r.clone();
    r2.insert(
        "/redir-abs.bib".into(),
        Route::redirect(301, &format!("http://127.0.0.1:{}/a.bib", http.port)),
    );
    // A remote control file whose data source is on the HTTP server.
    let doc = bcf(&[format!("http://127.0.0.1:{}/a.bib", http.port)]).expect("bcf");
    r2.insert("/doc.bcf".into(), Route::ok(doc));
    let https = servers::http(r2, Some(Cert::Localhost));
    let https_wrong = servers::http(r, Some(Cert::WrongHost));
    let mut f = BTreeMap::new();
    f.insert("pub/a.bib".to_string(), BIB.as_bytes().to_vec());
    f.insert("top.bib".to_string(), BIB.as_bytes().to_vec());
    let ftp = servers::ftp(f);
    Servers {
        http,
        https,
        https_wrong,
        ftp,
    }
}

/// A scenario: URLs (`{http}`, `{https}`, `{httpsw}`, `{ftp}` expand to
/// the server bases), extra arguments, environment.
struct Scenario {
    name: &'static str,
    urls: &'static [&'static str],
    args: &'static [&'static str],
    env: &'static [(&'static str, &'static str)],
}

const fn sc(
    name: &'static str,
    urls: &'static [&'static str],
    args: &'static [&'static str],
    env: &'static [(&'static str, &'static str)],
) -> Scenario {
    Scenario {
        name,
        urls,
        args,
        env,
    }
}

const CA: &str = "{ca}";

const SCENARIOS: &[Scenario] = &[
    sc("http-ok", &["{http}/a.bib"], &[], &[]),
    sc("http-chunked", &["{http}/chunked.bib"], &[], &[]),
    sc("http-empty", &["{http}/empty.bib"], &[], &[]),
    sc("http-404", &["{http}/missing.bib"], &[], &[]),
    sc(
        "http-404-nodie",
        &["{http}/missing.bib"],
        &["--nodieonerror"],
        &[],
    ),
    sc("http-410-reason", &["{http}/gone.bib"], &[], &[]),
    sc("http-500", &["{http}/err.bib"], &[], &[]),
    sc("http-503-noreason", &["{http}/noreason.bib"], &[], &[]),
    sc("http-redirect", &["{http}/redir.bib"], &[], &[]),
    sc("http-redirect-loop", &["{http}/loop.bib"], &[], &[]),
    sc("http-space", &["{http}/a b.bib"], &[], &[]),
    sc("http-nonascii", &["{http}/café.bib"], &[], &[]),
    sc("http-query", &["{http}/q.bib?x=1&y=2"], &[], &[]),
    sc(
        "http-userinfo",
        &["http://user:p%40ss@127.0.0.1:{httpport}/a.bib"],
        &[],
        &[],
    ),
    sc("http-twice", &["{http}/a.bib", "{http}/a.bib"], &[], &[]),
    sc("http-refused", &["http://127.0.0.1:1/a.bib"], &[], &[]),
    sc(
        "http-uppercase-scheme",
        &["HTTP://127.0.0.1:{httpport}/a.bib"],
        &[],
        &[],
    ),
    sc(
        "http-proxy",
        &["http://proxied.invalid/a.bib"],
        &[],
        &[("http_proxy", "{http}")],
    ),
    sc(
        "https-ca-file",
        &["{https}/a.bib"],
        &[],
        &[("PERL_LWP_SSL_CA_FILE", CA)],
    ),
    sc(
        "https-redirect-to-http",
        &["{https}/redir-abs.bib"],
        &[],
        &[("PERL_LWP_SSL_CA_FILE", CA)],
    ),
    sc("https-untrusted", &["{https}/a.bib"], &[], &[]),
    sc(
        "https-noverify",
        &["{https}/a.bib"],
        &["--ssl-noverify-host"],
        &[],
    ),
    sc(
        "https-wronghost",
        &["{httpsw}/a.bib"],
        &[],
        &[("PERL_LWP_SSL_CA_FILE", CA)],
    ),
    sc(
        "https-wronghost-noverify",
        &["{httpsw}/a.bib"],
        &["--ssl-noverify-host"],
        &[("PERL_LWP_SSL_CA_FILE", CA)],
    ),
    sc(
        "https-env-noverify",
        &["{https}/a.bib"],
        &[],
        &[("PERL_LWP_SSL_VERIFY_HOSTNAME", "0")],
    ),
    sc(
        "https-nointernalca",
        &["{https}/a.bib"],
        &["--ssl-nointernalca"],
        &[],
    ),
    sc(
        "https-nointernalca-certfile",
        &["{https}/a.bib"],
        &["--ssl-nointernalca"],
        &[("SSL_CERT_FILE", CA)],
    ),
    sc("ftp-ok", &["{ftp}/pub/a.bib"], &[], &[]),
    sc("ftp-top", &["{ftp}/top.bib"], &[], &[]),
    sc("ftp-missing", &["{ftp}/pub/missing.bib"], &[], &[]),
    sc("ftp-nodir", &["{ftp}/nodir/a.bib"], &[], &[]),
    sc(
        "ftp-badlogin",
        &["ftp://bad:x@127.0.0.1:{ftpport}/pub/a.bib"],
        &[],
        &[],
    ),
    sc("ftp-refused", &["ftp://127.0.0.1:1/a.bib"], &[], &[]),
    sc("ftps", &["ftps://127.0.0.1:{ftpport}/pub/a.bib"], &[], &[]),
    sc("dns-fail", &["http://nonexistent.invalid/a.bib"], &[], &[]),
    // No URLs: the arguments name the control file.
    sc(
        "bcf-remote",
        &[],
        &["--nolog", "-O", "remote.bbl", "{https}/doc.bcf"],
        &[("PERL_LWP_SSL_CA_FILE", CA)],
    ),
    sc(
        "bcf-remote-404",
        &[],
        &["--nolog", "-O", "remote.bbl", "{https}/nodoc.bcf"],
        &[("PERL_LWP_SSL_CA_FILE", CA)],
    ),
];

fn expand(s: &str, sv: &Servers) -> String {
    s.replace("{https}", &sv.https.base("https"))
        .replace(
            "{httpsw}",
            &format!("https://localhost:{}", sv.https_wrong.port),
        )
        .replace("{http}", &sv.http.base("http"))
        .replace("{ftp}", &sv.ftp.base("ftp"))
        .replace("{httpport}", &sv.http.port.to_string())
        .replace("{ftpport}", &sv.ftp.port.to_string())
        .replace("{ca}", &servers::ca_file().to_string_lossy())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;")
}

/// The template `.bcf` with its data source(s) replaced by `urls`.
fn bcf(urls: &[String]) -> Result<String> {
    let p = repo_root().join("tests/corpus/ported/encoding-4/encoding6.bcf");
    let t = std::fs::read_to_string(&p).with_context(|| p.display().to_string())?;
    let ds =
        "    <bcf:datasource type=\"file\" datatype=\"bibtex\">encoding6.bib</bcf:datasource>\n";
    if !t.contains(ds) {
        bail!("template .bcf changed");
    }
    let new: String = urls
        .iter()
        .map(|u| ds.replace("encoding6.bib", &xml_escape(u)))
        .collect();
    Ok(t.replace(ds, &new))
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    exit: Option<i32>,
    bbl: String,
    messages: Vec<String>,
    requests: Vec<String>,
}

fn normalise(s: &str, sv: &Servers) -> String {
    let re = Regex::new(r"[^\s']*biber_remote_data_source_\w{5}\.bib").unwrap();
    let s = re.replace_all(s, "<REMOTE>").into_owned();
    let mut s = s;
    for (p, n) in [
        (sv.http.port, "HTTP"),
        (sv.https.port, "HTTPS"),
        (sv.https_wrong.port, "HTTPSW"),
        (sv.ftp.port, "FTP"),
    ] {
        s = s.replace(&format!(":{p}"), &format!(":<{n}PORT>"));
    }
    s
}

fn run_one(r: &Runner, scn: &Scenario, sv: &Servers) -> Result<Outcome> {
    let dir = tempfile::Builder::new()
        .prefix("bibers-remote-")
        .tempdir()?;
    let urls: Vec<String> = scn.urls.iter().map(|u| expand(u, sv)).collect();
    if !urls.is_empty() {
        std::fs::write(dir.path().join("remote.bcf"), bcf(&urls)?)?;
    }
    for s in [&sv.http, &sv.https, &sv.https_wrong, &sv.ftp] {
        s.log.lock().unwrap().clear();
    }
    let (prog, lead) = r.argv.split_first().expect("argv");
    let mut cmd = Command::new(prog);
    cmd.args(lead)
        .arg("--noconf")
        .args(scn.args.iter().map(|a| expand(a, sv)))
        .args(if urls.is_empty() {
            None
        } else {
            Some("remote.bcf")
        })
        .current_dir(dir.path());
    for v in [
        "PERL_LWP_SSL_CA_FILE",
        "PERL_LWP_SSL_CA_PATH",
        "PERL_LWP_SSL_VERIFY_HOSTNAME",
        "HTTPS_CA_FILE",
        "HTTPS_CA_DIR",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "http_proxy",
        "https_proxy",
        "ftp_proxy",
        "no_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "FTP_PROXY",
        "NO_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ] {
        cmd.env_remove(v);
    }
    for (k, v) in bibers_testkit::run::BASE_ENV {
        cmd.env(k, v);
    }
    for (k, v) in scn.env {
        cmd.env(k, expand(v, sv));
    }
    let out = cmd.output()?;
    let bbl = std::fs::read_to_string(dir.path().join("remote.bbl")).unwrap_or_default();
    let blg = std::fs::read_to_string(dir.path().join("remote.blg")).unwrap_or_default();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let screen: Vec<String> = stdout
        .lines()
        .filter(|l| l.starts_with("WARN - ") || l.starts_with("ERROR - "))
        .map(|l| normalise(l, sv))
        .collect();
    let messages = blg_messages(&blg, dir.path())
        .into_iter()
        .map(|m| normalise(&m, sv))
        .collect();
    let mut requests = Vec::new();
    for s in [&sv.http, &sv.https, &sv.https_wrong, &sv.ftp] {
        for req in s.requests() {
            // Request line and the headers biber sets.
            for l in req.lines() {
                let ll = l.to_ascii_lowercase();
                if l.starts_with("GET ")
                    || l.starts_with("CONNECT ")
                    || ll.starts_with("user-agent:")
                    || ll.starts_with("zotero-allowed-request:")
                    || ll.starts_with("authorization:")
                    || ll.starts_with("host:")
                    || (!l.contains(':') && !l.is_empty())
                {
                    requests.push(normalise(l, sv));
                }
            }
        }
    }
    Ok(Outcome {
        exit: out.status.code(),
        bbl: normalise(&bbl, sv),
        messages: if blg.is_empty() { screen } else { messages },
        requests,
    })
}

pub fn remote(args: &[String]) -> Result<std::process::ExitCode> {
    let mut imp = "bibers".to_string();
    let mut reference = "o3".to_string();
    let mut verbose = false;
    let mut dns = false;
    let mut filters = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--impl" => imp = it.next().context("--impl needs a value")?.clone(),
            "--ref" => reference = it.next().context("--ref needs a value")?.clone(),
            "--dns" => dns = true,
            "-v" => verbose = true,
            f => filters.push(f.to_string()),
        }
    }
    let a = crate::runner(&imp, false)?;
    let b = crate::runner(&reference, false)?;
    let sv = start();
    let mut fails = 0;
    for scn in SCENARIOS {
        if !filters.is_empty() && !filters.iter().any(|f| scn.name.contains(f.as_str())) {
            continue;
        }
        if scn.name.starts_with("dns-") && !dns {
            continue;
        }
        let x = run_one(&a, scn, &sv)?;
        let y = run_one(&b, scn, &sv)?;
        let same = x == y;
        if !same {
            fails += 1;
        }
        println!(
            "{:5} {:32} {} exit {:?} / {} exit {:?}",
            if same { "same" } else { "DIFF" },
            scn.name,
            a.name,
            x.exit,
            b.name,
            y.exit
        );
        if verbose || !same {
            for (what, l, r) in [
                ("messages", x.messages.join("\n"), y.messages.join("\n")),
                ("requests", x.requests.join("\n"), y.requests.join("\n")),
                ("bbl", x.bbl.clone(), y.bbl.clone()),
            ] {
                if verbose || l != r {
                    println!("  -- {what} ({}):\n{}", a.name, indent(&l));
                    println!("  -- {what} ({}):\n{}", b.name, indent(&r));
                }
            }
        }
    }
    println!("{fails} scenario(s) differ");
    Ok(if fails == 0 {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    })
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("     {l}\n")).collect()
}

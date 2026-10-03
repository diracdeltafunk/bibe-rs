//! The `bibers` program flow, following bin/biber: command line, options and
//! config file, logging, output file, control file, data model. The
//! processing pipeline itself is not implemented yet (milestone M2).

use std::collections::BTreeMap;
use std::io::Write;

use bibers_core::bcf::{self, ParseArgs, Parsed};
use bibers_core::config::{self, CliOptions, CliValue};
use bibers_core::env::{Env, basename, catfile};
use bibers_core::log::{Level, LogSettings, Logger};
use bibers_core::quirks::Quirks;
use bibers_core::{biber_error, info};

use crate::cli::{self, Prelude, Special};
use crate::getopt::Val;

/// The bibers version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `--version` output (DC-040).
pub fn version_text() -> String {
    format!(
        "bibers version: {VERSION} (biber {} compatible)\n",
        bibers_core::BIBER_VERSION
    )
}

/// The log banner (DC-040).
pub fn banner() -> String {
    format!(
        "This is bibers {VERSION} (biber {} compatible)",
        bibers_core::BIBER_VERSION
    )
}

/// What `prelude` prints for bibers' own answers.
pub fn special(s: Special) -> (u8, String, String) {
    match s {
        Special::Version => (0, version_text(), String::new()),
        // DC-030: tool mode (and its config file) is deferred.
        Special::ToolConfig => (
            2,
            String::new(),
            "bibers: ERROR - tool mode (--tool, --tool-config) is not yet supported (docs/compat.md DC-030)\n"
                .into(),
        ),
    }
}

fn to_cli(opts: BTreeMap<String, Val>) -> CliOptions {
    opts.into_iter()
        .map(|(k, v)| {
            let cv = match v {
                Val::Scalar(s) => CliValue::Str(s),
                Val::Hash(h) => CliValue::Map(h),
                Val::Array(a) => CliValue::Str(a.join(",")),
            };
            (k, cv)
        })
        .collect()
}

/// Deferred or dropped features (docs/compat.md §3, §4): the message to
/// print, if the options request one.
pub fn unsupported(opts: &BTreeMap<String, Val>) -> Option<String> {
    let s = |k: &str| match opts.get(k) {
        Some(Val::Scalar(v)) => Some(v.as_str()),
        _ => None,
    };
    let of = s("output_format").unwrap_or("bbl");
    let msg = match of {
        "dot" => Some(
            "output format 'dot' is not supported (deprecated upstream; docs/compat.md DC-021)",
        ),
        "bblxml" => Some(
            "output format 'bblxml' is not supported (deprecated upstream; docs/compat.md DC-020)",
        ),
        "bibjson" => Some(
            "output format 'bibjson' is not supported (unimplemented upstream; docs/compat.md DC-022)",
        ),
        _ => None,
    };
    if let Some(m) = msg {
        return Some(m.to_string());
    }
    if opts.contains_key("tool") {
        return Some("tool mode (--tool) is not yet supported (docs/compat.md DC-030)".into());
    }
    if of == "bibtex" {
        return Some("output format 'bibtex' is not yet supported (docs/compat.md DC-030)".into());
    }
    if of == "biblatexml" {
        return Some(
            "output format 'biblatexml' is not yet supported (docs/compat.md DC-031)".into(),
        );
    }
    for (k, flag) in [
        ("validate_control", "--validate-control"),
        ("validate_config", "--validate-config"),
        ("validate_bltxml", "--validate-bltxml"),
        ("validate_bblxml", "--validate-bblxml"),
    ] {
        if s(k).is_some_and(|v| !v.is_empty() && v != "0") {
            return Some(format!(
                "RNG validation ({flag}) is not yet supported (docs/compat.md DC-033)"
            ));
        }
    }
    if s("convert_control").is_some_and(|v| !v.is_empty() && v != "0") {
        return Some("--convert-control (XSLT) is not yet supported".into());
    }
    None
}

/// Output file name derivation (bin/biber:353-392), with BB-068 (bibtex
/// output names) and BB-069 (`-O -` with `--output-directory`) fixed.
pub fn output_file(cfg: &config::Config) -> Result<String, String> {
    let b = &cfg.biber;
    let truthy = |v: Option<&str>| v.is_some_and(|v| !v.is_empty() && v != "0");
    let mut outfile = if truthy(b.get_str("output_file")) {
        b.get_str("output_file").unwrap_or("").to_string()
    } else {
        let bcf = b.get_str("bcf").unwrap_or("");
        let stem = bcf.strip_suffix("bcf").unwrap_or(bcf);
        match b.output_format() {
            "dot" => format!("{stem}dot"),
            // BB-068: biber uses s/\..+$/_biber.bib/ on the whole path.
            "bibtex" => format!("{}_biber.bib", bcf.strip_suffix(".bcf").unwrap_or(bcf)),
            "bbl" => format!("{stem}bbl"),
            "bblxml" => format!("{stem}bblxml"),
            of => {
                return Err(format!(
                    "output_format option '{of}' only makes sense in tool mode (--tool)"
                ));
            }
        }
    };
    if let Some(outdir) = b.get_str("output_directory").filter(|d| truthy(Some(d)))
        && outfile != "-"
    {
        outfile = catfile(outdir, basename(&outfile));
    }
    Ok(outfile)
}

fn eprint_flush(s: &str) {
    if !s.is_empty() {
        let mut e = std::io::stderr().lock();
        let _ = e.write_all(s.as_bytes());
        let _ = e.flush();
    }
}

fn print_flush(s: &str) {
    if !s.is_empty() {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(s.as_bytes());
        let _ = o.flush();
    }
}

/// Run bibers with `args` (without the program name). Returns the exit code.
pub fn main(args: &[String], env: &Env) -> u8 {
    bibers_core::log::mark_program_start();
    let (mut opts, argv) = match cli::prelude(args, special) {
        Prelude::Exit {
            code,
            stdout,
            stderr,
        } => {
            print_flush(&stdout);
            eprint_flush(&stderr);
            return code;
        }
        Prelude::Run { opts, argv, stderr } => {
            eprint_flush(&stderr);
            (opts, argv)
        }
    };
    let mut quirks = Quirks::default();
    if let Some(Val::Scalar(list)) = opts.remove("fix_quirks")
        && let Err(e) = quirks.apply_fix_list(&list)
    {
        eprint_flush(&format!("bibers: {e}\n"));
        return 2;
    }
    if let Some(msg) = unsupported(&opts) {
        eprint_flush(&format!("bibers: ERROR - {msg}\n"));
        return 2;
    }

    let mut cli_opts = to_cli(opts);
    let init = match config::initopts(&mut cli_opts, &argv, env, quirks, None) {
        Ok(i) => i,
        Err(e) => {
            eprint_flush(&format!("bibers: ERROR - {}\n", e.0));
            return 2;
        }
    };
    for n in &init.conf_notes {
        eprint_flush(&format!("{n}\n"));
    }
    let b = &init.config.biber;
    let settings = LogSettings::new(
        b.trace(),
        b.debug(),
        b.num("quiet") as i64,
        b.is_true("nolog"),
        b.is_true("onlylog"),
    );
    let logpath = init.logfile.as_deref().map(|p| env.resolve(p));
    let mut log = match Logger::new(settings, logpath.as_deref()) {
        Ok(l) => l,
        Err(e) => {
            eprint_flush(&format!(
                "bibers: ERROR - cannot write log file '{}': {e}\n",
                init.logfile.as_deref().unwrap_or("")
            ));
            return 2;
        }
    };
    log.set_nodieonerror(b.nodieonerror());
    init.log_startup(&mut log, &banner());
    let mut cfg = init.config;
    let now = chrono::Local::now().format("%a %b %e, %Y, %H:%M:%S");
    log.logfile_info(bibers_core::loc!(), &format!("=== {now}"));

    let outfile = match output_file(&cfg) {
        Ok(o) => o,
        Err(msg) => {
            // biber prints this on STDOUT (BB-074).
            print_flush(&format!("{msg}\n"));
            return 2;
        }
    };
    // The output file is opened (truncated) before the .bcf is read, so an
    // early failure leaves an empty file (biber behaviour; DC-012 open).
    if outfile != "-"
        && let Err(e) = std::fs::File::create(env.resolve(&outfile))
    {
        let _ = biber_error!(log, "Cannot open output file '{outfile}': {e}");
        return log.display_end();
    }

    let bcf_path = cfg.biber.get_str("bcf").unwrap_or("").to_string();
    let parsed = bcf::parse_ctrlfile(
        &mut cfg,
        &mut log,
        env,
        &ParseArgs {
            ctrl_file: &bcf_path,
            output_target: Some(&outfile),
        },
    );
    let mut doc = match parsed {
        Err(_) => return log.display_end(),
        // BB-070: biber exits 0 here without printing its warning.
        Ok(Parsed::NoDataSources) => {
            log.flush();
            return 0;
        }
        Ok(Parsed::Document(d)) => d,
    };
    for e in cfg.postprocess_biber_opts() {
        if biber_error!(log, "{e}").is_err() {
            return log.display_end();
        }
    }
    if cfg.biber.debug() {
        log.set_main_level(Level::Debug);
    }
    if bcf::process_setup(&mut cfg, &mut log, &mut doc).is_err() {
        return log.display_end();
    }
    info!(
        log,
        "bibers {VERSION} has read the control file and data model; processing is not yet implemented"
    );
    let _ = biber_error!(
        log,
        "processing not yet implemented (bibers {VERSION} is a development version)"
    );
    log.display_end()
}

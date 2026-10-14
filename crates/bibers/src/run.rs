//! The `bibers` program flow, following bin/biber: command line, options and
//! config file, logging, output file, control file, data model. The
//! processing pipeline itself is not implemented yet (milestone M2).

use std::collections::BTreeMap;
use std::io::Write;

use bibers_core::bcf::{self, ParseArgs, Parsed};
use bibers_core::config::{self, CliOptions, CliValue};
use bibers_core::env::{Env, basename, catfile};
use bibers_core::log::{Level, LogSettings, Logger};
use bibers_core::output::OutputFormat;
use bibers_core::pipeline::{self, Ctx, tool};
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
        // DC-055: bibers has no data directory, so it prints its built-in
        // biber-tool.conf instead of that file's path.
        Special::ToolConfig => (
            0,
            bibers_core::config::conf_file::BIBER_TOOL_CONF.to_string(),
            String::new(),
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
    // DC-033: all RNG validation is implemented except for the dropped
    // bblxml format.
    if s("validate_bblxml").is_some_and(|v| !v.is_empty() && v != "0") {
        return Some(
            "RNG validation (--validate-bblxml) is not yet supported (docs/compat.md DC-033)"
                .into(),
        );
    }
    None
}

/// The tool-mode output name for data source `dsn`: biber's
/// `s/\.[^.]+$/<suffix>/r` (bin/biber:352-358) when the file name has an
/// extension. DC-011 (BB-013): without one, biber's substitution either
/// fails, so the output overwrites the input, or matches a `.` in a
/// directory name; bibers appends the suffix to the name instead.
pub fn tool_output_name(dsn: &str, suffix: &str) -> String {
    let base = basename(dsn);
    match base.rfind('.') {
        Some(p) if p + 1 < base.len() => {
            let cut = dsn.len() - (base.len() - p);
            format!("{}{suffix}", &dsn[..cut])
        }
        _ => format!("{dsn}{suffix}"),
    }
}

/// Output file name derivation (bin/biber:353-392), with BB-068 (bibtex
/// output names), BB-013 (tool output names) and BB-069 (`-O -` with
/// `--output-directory`) fixed.
pub fn output_file(cfg: &config::Config) -> Result<String, String> {
    let b = &cfg.biber;
    let truthy = |v: Option<&str>| v.is_some_and(|v| !v.is_empty() && v != "0");
    let mut outfile = if truthy(b.get_str("output_file")) {
        b.get_str("output_file").unwrap_or("").to_string()
    } else if b.tool() {
        let dsn = b.get_str("dsn").unwrap_or("");
        let suffix = match b.output_format() {
            "bibtex" => "_bibertool.bib",
            "biblatexml" => "_bibertool.bltxml",
            _ => "_bibertool.json",
        };
        tool_output_name(dsn, suffix)
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
    run(args, env, false)
}

/// [`main`] for a process that exits as soon as it returns: the run's
/// state (entries, data model, control file) is left for the OS to reclaim
/// instead of being freed piece by piece, which is a few percent of a run.
pub fn main_then_exit(args: &[String], env: &Env) -> u8 {
    run(args, env, true)
}

fn run(args: &[String], env: &Env, exiting: bool) -> u8 {
    bibers_core::log::mark_program_start();
    match setup(args, env) {
        SetupResult::Exit(code) => code,
        SetupResult::Ready(mut s) => {
            s.exiting = exiting;
            process(*s, env)
        }
    }
}

/// The state after option parsing, the control file and `process_setup`:
/// ready for the processing pipeline.
pub struct Setup {
    pub cfg: config::Config,
    pub doc: bcf::Document,
    pub log: Logger,
    /// The output file (`-` for stdout).
    pub outfile: String,
    /// Tool mode (`--tool` on the command line; `--output-format=bibtex`
    /// sets the `tool` option without being tool mode, Biber.pm:1133).
    pub tool: bool,
    /// The output format.
    pub format: OutputFormat,
    /// The process exits after [`process`]: don't free the run's state.
    pub exiting: bool,
}

/// Result of [`setup`].
pub enum SetupResult {
    Ready(Box<Setup>),
    /// Stop with this exit code (errors already reported).
    Exit(u8),
}

/// Everything bin/biber does before `prepare`: command line, options and
/// config file, logging, output file, control file, `process_setup`.
pub fn setup(args: &[String], env: &Env) -> SetupResult {
    use SetupResult::Exit;
    let (mut opts, argv) = match cli::prelude(args, special) {
        Prelude::Exit {
            code,
            stdout,
            stderr,
        } => {
            print_flush(&stdout);
            eprint_flush(&stderr);
            return Exit(code);
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
        return Exit(2);
    }
    if let Some(msg) = unsupported(&opts) {
        eprint_flush(&format!("bibers: ERROR - {msg}\n"));
        return Exit(2);
    }
    // `exists($opts->{tool})` (bin/biber:400, 440).
    let tool = opts.contains_key("tool");

    let mut cli_opts = to_cli(opts);
    let init = match config::initopts(&mut cli_opts, &argv, env, quirks, None) {
        Ok(i) => i,
        Err(e) => {
            eprint_flush(&format!("bibers: ERROR - {}\n", e.0));
            return Exit(2);
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
            return Exit(2);
        }
    };
    log.set_nodieonerror(b.nodieonerror());
    init.log_startup(&mut log, &banner());
    // Biber.pm:95-102: validate the config file that was read (DC-033).
    if init.config.biber.is_true("validate_config")
        && let Some(cf) = init.configfile.as_deref()
        && bibers_core::xmlcheck::validate_biber_xml(
            &mut log,
            env,
            cf,
            bibers_core::xmlcheck::SchemaKind::Config,
            init.config.quirks,
        )
        .is_err()
    {
        return Exit(log.display_end());
    }
    let mut cfg = init.config;
    // Biber->new: Biber::LaTeX::Recode->init_sets (DC-045).
    if bibers_core::recodedata::load(&mut cfg, &mut log, env).is_err() {
        return Exit(log.display_end());
    }
    let now = jiff::Zoned::now().strftime("%a %b %e, %Y, %H:%M:%S");
    log.logfile_info(bibers_core::loc!(), &format!("=== {now}"));

    let outfile = match output_file(&cfg) {
        Ok(o) => o,
        Err(msg) => {
            // biber prints this on STDOUT (BB-074).
            print_flush(&format!("{msg}\n"));
            return Exit(2);
        }
    };
    let Some(format) = OutputFormat::from_name(cfg.biber.output_format()) else {
        // Unsupported formats are rejected by `unsupported` above.
        let _ = biber_error!(
            log,
            "Output format '{}' is not supported",
            cfg.biber.output_format()
        );
        return Exit(log.display_end());
    };
    // The `.bbl` is opened (truncated) before the .bcf is read, so an early
    // failure leaves an empty file (biber behaviour; DC-012 open). Data
    // formats open it only when writing.
    if outfile != "-"
        && format.opens_output_early()
        && let Err(e) = std::fs::File::create(env.resolve(&outfile))
    {
        let _ = biber_error!(log, "Cannot open output file '{outfile}': {e}");
        return Exit(log.display_end());
    }
    // The biblatexml writer opens its file and writes the document head at
    // once (Output/biblatexml.pm:52-90), so a later fatal error leaves it.
    if outfile != "-"
        && format == bibers_core::output::OutputFormat::Biblatexml
        && let Err(e) = std::fs::write(
            env.resolve(&outfile),
            bibers_core::output::biblatexml::header(&cfg),
        )
    {
        let _ = biber_error!(log, "Cannot open output file '{outfile}': {e}");
        return Exit(log.display_end());
    }

    let mut doc = if tool {
        // Fake the necessary .bcf parts (bin/biber:400-403).
        tool::tool_mode_setup(&mut cfg, &argv)
    } else {
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
        match parsed {
            Err(_) => return Exit(log.display_end()),
            // BB-070: biber exits 0 here without printing its warning.
            Ok(Parsed::NoDataSources) => {
                log.flush();
                return Exit(0);
            }
            Ok(Parsed::Document(d)) => d,
        }
    };
    for e in cfg.postprocess_biber_opts() {
        if biber_error!(log, "{e}").is_err() {
            return Exit(log.display_end());
        }
    }
    if cfg.biber.debug() {
        log.set_main_level(Level::Debug);
    }
    let setup = if tool {
        // `process_setup_tool`, the first step of `prepare_tool`.
        tool::process_setup_tool(&mut cfg, &mut log)
    } else {
        bcf::process_setup(&mut cfg, &mut log, &mut doc)
    };
    if setup.is_err() {
        return Exit(log.display_end());
    }
    SetupResult::Ready(Box::new(Setup {
        cfg,
        doc,
        log,
        outfile,
        tool,
        format,
        exiting: false,
    }))
}

/// Run the processing pipeline and write the output (`prepare` and
/// `output` in bin/biber).
pub fn process(s: Setup, env: &Env) -> u8 {
    let Setup {
        mut cfg,
        doc,
        mut log,
        outfile,
        tool,
        format,
        exiting,
    } = s;
    let dm = cfg.dm.take().expect("process_setup builds the data model");
    let mut ctx = Ctx {
        cfg: &mut cfg,
        dm: &dm,
        doc: &doc,
        log: &mut log,
        env,
        run: Default::default(),
    };
    let prepared = if tool {
        tool::prepare_tool(&mut ctx)
    } else {
        pipeline::prepare(&mut ctx)
    };
    let processed = match prepared {
        Ok(p) => p,
        Err(_) => return log.display_end(),
    };
    let text = match format.render(&mut ctx, &processed) {
        Ok(Some(t)) => t,
        Ok(None) => {
            let _ = biber_error!(
                ctx.log,
                "output not yet implemented (bibers {VERSION} is a development version)"
            );
            return log.display_end();
        }
        Err(_) => return log.display_end(),
    };
    let bytes = if format.encodes_output() {
        // bbl.pm:676-677, 779 (`-` is stdout; biber logs it by that name too).
        let enc = bibers_core::output::bbl::output_encoding_name(&ctx);
        info!(ctx.log, "Writing '{outfile}' with encoding '{enc}'");
        if ctx.cfg.biber.is_true("output_safechars") {
            info!(ctx.log, "{}", format.safechars_message());
        }
        match bibers_core::output::bbl::encode_output(&mut ctx, &text) {
            Ok(b) => b,
            Err(_) => return log.display_end(),
        }
    } else {
        text.into_bytes()
    };
    if outfile == "-" {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(&bytes);
        let _ = o.flush();
    } else if let Err(e) = std::fs::write(env.resolve(&outfile), &bytes) {
        let _ = biber_error!(ctx.log, "Cannot write output file '{outfile}': {e}");
        return log.display_end();
    }
    info!(ctx.log, "Output to {outfile}");
    if format.after_write(&mut ctx, &outfile).is_err() {
        return log.display_end();
    }
    if exiting {
        // Everything but the configuration, whose remote-source temporary
        // directory is removed on drop.
        std::mem::forget((ctx.run, processed, dm, doc));
    }
    show_tmp_dir(&mut cfg, &mut log);
    log.display_end()
}

/// `display_end`'s `--show-tmp-dir` report (Biber.pm:120-127). biber's temp
/// directory only holds files in bibers when remote sources were fetched.
fn show_tmp_dir(cfg: &mut config::Config, log: &mut Logger) {
    if !cfg.biber.is_true("show_tmp_dir") {
        return;
    }
    if cfg.biber.is_true("noremove_tmp_dir") {
        if let Ok(d) = cfg.remote.tempdir(true) {
            info!(log, "TEMP DIR: {}", d.display());
        }
    } else {
        log.biber_warn("--noremove-tmp-dir was not set, no temporary directory to show");
    }
}

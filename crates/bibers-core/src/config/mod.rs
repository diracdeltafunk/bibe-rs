//! Run configuration: biber options, biblatex options, datafield sets and
//! the other process-global state of `Biber::Config`, made explicit.
//!
//! Precedence (docs/research/02 §3.4):
//! * biber options: command line > biber.conf > `.bcf` > defaults (the
//!   `.bcf` only sets options that are not explicit);
//! * biblatex options: `.bcf` > hard-coded defaults > biber.conf (BB-053:
//!   the defaults are applied after the config file; quirk
//!   `blx-defaults-override-conf`).

pub mod biber_opts;
pub mod blx;
pub mod conf_file;
pub mod locale;

use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value};

pub use biber_opts::{BiberOptions, BiberValue, PatternOpt, Provenance};
pub use blx::{BlxOptions, OptValue, Registry, Scope};

use crate::datamodel::DataModel;
use crate::env::{Env, basename, catfile};
use crate::quirks::{Quirk, Quirks};

/// A datafield set member before resolution against the data model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DfsMember {
    /// A field name.
    Field(String),
    /// All fields of a field type and/or data type.
    Types {
        /// `fieldtype`.
        fieldtype: Option<String>,
        /// `datatype`.
        datatype: Option<String>,
    },
}

/// The run configuration.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// Biber options.
    pub biber: BiberOptions,
    /// biblatex options and their scope registry.
    pub blx: BlxOptions,
    /// `%DATAFIELD_SETS` before resolution.
    pub datafield_sets: BTreeMap<String, Vec<DfsMember>>,
    /// `%DATAFIELD_SETS` after `_resolve_datafieldsets`.
    pub datafield_sets_resolved: BTreeMap<String, Vec<String>>,
    /// `$CONFIG->{state}{keyorder}`: section → key → order.
    pub keyorder: HashMap<String, HashMap<String, String>>,
    /// `$CONFIG->{state}{internalkeyorder}`.
    pub internal_keyorder: HashMap<String, HashMap<String, String>>,
    /// The located control file.
    pub ctrlfile_path: Option<String>,
    /// `%CONFIG_OUTPUT_FIELDREPLACE`.
    pub output_field_replace: BTreeMap<String, String>,
    /// Reproduced biber bugs.
    pub quirks: Quirks,
    /// The data model (built in `process_setup`).
    pub dm: Option<DataModel>,
    /// Run-global source map state (`$MAPUNIQVAL`, the `$MAPUNIQ` counter).
    pub sourcemap_run: crate::pipeline::sourcemap::MapRunState,
    /// A user recode data file (`--recodedata`), loaded by
    /// [`crate::recodedata::load`]; `None`: biber's built-in `recode_data.xml`.
    pub recodedata: Option<std::sync::Arc<latex_recode::RecodeData>>,
    /// Remote data sources and control files: `%REMOTE_MAP`, biber's temp
    /// directory and the `%ENV` changes biber makes for HTTPS (DC-032).
    pub remote: crate::remote::RemoteState,
}

/// A command-line option value after `GetOptions` and bin/biber's
/// post-processing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliValue {
    /// `=s`, flags (`1`), counters and integers, as strings.
    Str(String),
    /// A hash (`collate_options`, `dot_include` after splitting).
    Map(BTreeMap<String, String>),
    /// `undef` (e.g. `configfile` when discovery found nothing).
    Undef,
}

/// The `$opts` hash bin/biber passes to `Biber->new`.
pub type CliOptions = BTreeMap<String, CliValue>;

impl Config {
    /// A fresh configuration with the hard-coded biber defaults.
    pub fn new(quirks: Quirks) -> Self {
        Config {
            biber: BiberOptions::with_defaults(),
            quirks,
            ..Config::default()
        }
    }

    /// The LaTeX recoder for the `decodecharsset`/`output_safecharsset`
    /// options (`Biber::LaTeX::Recode::init_sets`), over the user recode
    /// data file if one was given.
    pub fn recoder(
        &self,
        opts: latex_recode::RecodeOptions,
    ) -> std::sync::Arc<latex_recode::Recoder> {
        let d = self.biber.get_str("decodecharsset").unwrap_or("base");
        let e = self.biber.get_str("output_safecharsset").unwrap_or("base");
        match &self.recodedata {
            Some(data) => std::sync::Arc::new(latex_recode::Recoder::new(data, d, e, opts)),
            None => latex_recode::Recoder::builtin(d, e, opts),
        }
    }

    /// `setconfigfileoption`: sets the biber option and, if the option is a
    /// known biblatex option at any scope, its GLOBAL biblatex value.
    pub fn set_configfile_option(&mut self, opt: &str, val: BiberValue) {
        if self.blx.registry.is_declared(opt) {
            let ov = match &val {
                BiberValue::Undef => OptValue::Undef,
                BiberValue::Str(s) => OptValue::Str(s.clone()),
                _ => OptValue::Undef,
            };
            self.blx.set_global_raw(opt, ov);
        }
        self.biber.set_configfile(opt, val);
    }

    /// `set_keyorder`.
    pub fn set_keyorder(&mut self, sec: &str, key: &str, order: Option<&str>) {
        let m = self.keyorder.entry(sec.to_string()).or_default();
        match order {
            Some(o) => {
                m.insert(key.to_string(), o.to_string());
            }
            None => {
                m.remove(key);
            }
        }
    }

    /// `get_keyorder`.
    pub fn get_keyorder(&self, sec: &str, key: &str) -> Option<&str> {
        self.keyorder.get(sec)?.get(key).map(String::as_str)
    }

    /// `postprocess_biber_opts` (Config.pm:756): `sortcase`/`sortupper`
    /// `true`/`false` → `1`/`0`; anything else but `1`/`0` is an error.
    /// Returns the error messages to report.
    pub fn postprocess_biber_opts(&mut self) -> Vec<String> {
        let mut errs = Vec::new();
        for opt in ["sortcase", "sortupper"] {
            let Some(v) = self.biber.get(opt).cloned() else {
                continue;
            };
            let s = v.as_str().map(str::to_string);
            let nv = match s.as_deref() {
                Some("true") => Some("1".to_string()),
                Some("false") => Some("0".to_string()),
                _ => None,
            };
            if let Some(nv) = &nv {
                // A direct hash store: explicitness flags are kept.
                self.biber.set(opt, nv.as_str());
            }
            let cur = self.biber.get_str(opt);
            if cur != Some("1") && cur != Some("0") {
                errs.push(format!("Invalid value for option '{opt}'"));
            }
        }
        errs
    }

    /// `normalise_utf8` (Utils.pm:1066).
    pub fn normalise_utf8(&mut self) {
        for opt in ["input_encoding", "output_encoding"] {
            if let Some(v) = self.biber.get_str(opt) {
                let l = v.to_ascii_lowercase();
                if l == "utf8" || l == "utf-8" {
                    self.biber.set(opt, "UTF-8");
                }
            }
        }
    }

    /// `_resolve_datafieldsets` (Biber.pm:1253): needs the data model.
    pub fn resolve_datafieldsets(&mut self) {
        let Some(dm) = &self.dm else { return };
        let mut out = BTreeMap::new();
        for (name, members) in &self.datafield_sets {
            let mut fs = Vec::new();
            for m in members {
                match m {
                    DfsMember::Field(f) => fs.push(f.clone()),
                    DfsMember::Types {
                        fieldtype,
                        datatype,
                    } => {
                        let ft = fieldtype
                            .as_deref()
                            .filter(|s| crate::perlish::truthy(Some(s)));
                        let dt = datatype
                            .as_deref()
                            .filter(|s| crate::perlish::truthy(Some(s)));
                        match (ft, dt) {
                            (Some(ft), Some(dt)) => {
                                fs.extend(dm.get_fields_of_type(ft, &[dt], None))
                            }
                            (Some(ft), None) => fs.extend(dm.get_fields_of_fieldtype(ft)),
                            (None, Some(dt)) => fs.extend(dm.get_fields_of_datatype(&[dt])),
                            (None, None) => {}
                        }
                    }
                }
            }
            out.insert(name.clone(), fs);
        }
        self.datafield_sets_resolved = out;
    }

    /// A resolved datafield set.
    pub fn datafield_set(&self, name: &str) -> Option<&[String]> {
        self.datafield_sets_resolved.get(name).map(Vec::as_slice)
    }

    /// JSON dump of the biber options (all values as biber stores them).
    pub fn biber_json(&self) -> Value {
        let mut m = Map::new();
        for (k, slot) in self.biber.iter() {
            m.insert(k.to_string(), biber_value_json(&slot.value));
        }
        Value::Object(m)
    }
}

/// JSON of a biber option value, in biber's shape.
pub fn biber_value_json(v: &BiberValue) -> Value {
    match v {
        BiberValue::Undef => Value::Null,
        BiberValue::Str(s) => s.clone().into(),
        BiberValue::List(l) => Value::Array(
            l.iter()
                .map(|x| x.clone().map_or(Value::Null, Value::String))
                .collect(),
        ),
        BiberValue::Map(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), v.clone().into()))
                .collect(),
        ),
        BiberValue::Patterns(p) => Value::Array(
            p.iter()
                .map(|x| {
                    let mut o = Map::new();
                    if x.has_name {
                        o.insert(
                            "name".into(),
                            x.name.clone().map_or(Value::Null, Value::String),
                        );
                    }
                    o.insert(
                        "value".into(),
                        x.value.clone().map_or(Value::Null, Value::String),
                    );
                    Value::Object(o)
                })
                .collect(),
        ),
        BiberValue::Sourcemap(s) => Value::Array(s.iter().map(|x| x.to_json()).collect()),
    }
}

/// The result of option initialisation (`_initopts`).
#[derive(Debug)]
pub struct Init {
    /// The configuration.
    pub config: Config,
    /// The config file that was read, if any (`$opts->{configfile}`).
    pub configfile: Option<String>,
    /// The `.blg` path, unless logging to a file is off.
    pub logfile: Option<String>,
    /// The `.blg` name biber computed (even with `--nolog`).
    pub biberlog: Option<String>,
    /// Notes from config-file parsing (printed to STDERR by biber's `carp`).
    pub conf_notes: Vec<String>,
}

impl Init {
    /// The messages `_initopts` logs right after setting up logging
    /// (Config.pm:300-313). `banner` is the "This is …" line (DC-040).
    pub fn log_startup(&self, log: &mut crate::log::Logger, banner: &str) {
        let nolog = self.config.biber.is_true("nolog");
        if !nolog {
            let tool = if self.config.biber.tool() {
                " running in TOOL mode"
            } else {
                ""
            };
            crate::info!(log, "{banner}{tool}");
        }
        if let Some(cf) = &self.configfile {
            crate::info!(log, "Config file is '{}'", crate::perlish::nfc(cf));
        }
        if !nolog {
            crate::info!(
                log,
                "Logfile is '{}'",
                self.biberlog.as_deref().unwrap_or("")
            );
        }
        if self.config.biber.debug() {
            log.screen_info(&format!(
                "DEBUG mode: all messages are logged to '{}'",
                self.biberlog.as_deref().unwrap_or("")
            ));
        }
    }
}

/// The hard-coded biblatex defaults (`%CONFIG_DEFAULT_BIBLATEX`).
pub const DEFAULT_BIBLATEX: &[(&str, &str)] = &[
    ("sortingtemplatename", "tool"),
    ("useauthor", "1"),
    ("useeditor", "1"),
    ("usetranslator", "1"),
    ("maxbibnames", "100"),
    ("maxitems", "100"),
    ("minbibnames", "100"),
    ("maxalphanames", "100"),
    ("maxcitenames", "100"),
    ("maxsortnames", "100"),
    ("minalphanames", "100"),
    ("mincitenames", "100"),
    ("minsortnames", "100"),
    ("minitems", "100"),
    ("useprefix", "0"),
];

fn apply_blx_defaults(cfg: &mut Config) {
    for (k, v) in DEFAULT_BIBLATEX {
        cfg.blx.set_global(k, OptValue::Str((*v).to_string()));
    }
}

/// `_initopts` (Config.pm:115-313), up to (not including) logger set-up.
///
/// `opts` is bin/biber's `$opts` hash; `argv` the remaining positional
/// arguments; `tool_conf` the path of biber-tool.conf for tool mode.
pub fn initopts(
    opts: &mut CliOptions,
    argv: &[String],
    env: &Env,
    quirks: Quirks,
    tool_conf: Option<&str>,
) -> Result<Init, conf_file::ConfError> {
    let mut cfg = Config::new(quirks);
    let mut notes = Vec::new();

    if !opts.contains_key("noconf") {
        let given = match opts.get("configfile") {
            Some(CliValue::Str(s)) => Some(s.clone()),
            _ => None,
        };
        // BB-066: a --configfile that is not an existing file is silently
        // replaced by the default search.
        if !given.as_deref().is_some_and(|g| env.is_file(g)) {
            let found = conf_file::config_file(env);
            opts.insert(
                "configfile".into(),
                found.map_or(CliValue::Undef, CliValue::Str),
            );
        }
    }

    let is_tool =
        matches!(opts.get("tool"), Some(CliValue::Str(s)) if crate::perlish::truthy(Some(s)));
    if is_tool && let Some(tc) = tool_conf {
        let conf = conf_file::read_conf(env, tc)?;
        notes.extend(conf_file::apply_conf(&mut cfg, &conf));
    }

    let configfile = match opts.get("configfile") {
        Some(CliValue::Str(s)) => Some(s.clone()),
        _ => None,
    };
    if !quirks.on(Quirk::BlxDefaultsOverrideConf) {
        apply_blx_defaults(&mut cfg);
    }
    if let Some(cf) = &configfile {
        let conf = conf_file::read_conf(env, cf)?;
        notes.extend(conf_file::apply_conf(&mut cfg, &conf));
    }
    // BB-053: biber applies the biblatex defaults after the config file.
    if quirks.on(Quirk::BlxDefaultsOverrideConf) {
        apply_blx_defaults(&mut cfg);
    }

    // Command-line overrides everything else.
    for (k, v) in opts.iter() {
        if k == "collate_options" {
            let mut coll = cfg.biber.collate_options().cloned().unwrap_or_default();
            if let CliValue::Map(m) = v {
                for (ck, cv) in m {
                    coll.insert(ck.clone(), cv.clone());
                }
            }
            cfg.set_configfile_option("collate_options", BiberValue::Map(coll));
        } else {
            let bv = match v {
                CliValue::Str(s) => BiberValue::Str(s.clone()),
                CliValue::Map(m) => BiberValue::Map(m.clone()),
                CliValue::Undef => BiberValue::Undef,
            };
            cfg.biber.set_cmdline(k, bv);
        }
    }

    if cfg.biber.tool() {
        if let Some(dsn) = argv.first() {
            cfg.biber.set("dsn", dsn.as_str());
        }
    } else if let Some(a0) = argv.first() {
        let bcf = if a0.ends_with(".bcf") {
            a0.clone()
        } else {
            format!("{a0}.bcf")
        };
        cfg.biber.set("bcf", bcf);
    }

    let mut biberlog = None;
    if let Some(log) = cfg
        .biber
        .get_str("logfile")
        .filter(|l| crate::perlish::truthy(Some(l)))
    {
        let log = log.strip_suffix(".blg").unwrap_or(log);
        biberlog = Some(format!("{log}.blg"));
    } else if argv.is_empty() {
        cfg.biber.set("nolog", "1");
    } else {
        let a0 = &argv[0];
        let b = a0.strip_suffix(".bcf").unwrap_or(a0);
        biberlog = Some(format!("{b}.blg"));
    }
    if let Some(outdir) = cfg
        .biber
        .get_str("output_directory")
        .filter(|d| crate::perlish::truthy(Some(d)))
    {
        // biber does this even when $biberlog is undef (catfile($outdir, ''))
        let file = biberlog.as_deref().map(basename).unwrap_or("");
        biberlog = Some(catfile(outdir, file));
    }

    if let Some(ofrs) = cfg
        .biber
        .get_str("output_field_replace")
        .map(str::to_string)
        && crate::perlish::truthy(Some(&ofrs))
    {
        for ofr in crate::perlish::split_comma(&ofrs) {
            if let Some((f, r)) = ofr.split_once(':')
                && !f.is_empty()
                && !r.is_empty()
                && !r.contains(':')
            {
                cfg.output_field_replace
                    .insert(f.to_string(), r.to_string());
            }
        }
    }

    let biberlog_all = biberlog.clone();
    let logfile = if cfg.biber.is_true("nolog") {
        None
    } else {
        biberlog
    };
    Ok(Init {
        config: cfg,
        configfile,
        logfile,
        biberlog: biberlog_all,
        conf_notes: notes,
    })
}

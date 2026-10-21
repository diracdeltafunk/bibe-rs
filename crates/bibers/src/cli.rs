//! bin/biber's command line: the `GetOptions` spec (bin/biber:33-138) and
//! the option post-processing up to `Biber->new` (bin/biber:141-329).

use std::collections::BTreeMap;

use crate::getopt::{Getopt, Val};

/// biber's option specs, verbatim (bin/biber:35-136).
pub const SPECS: &[&str] = &[
    "annotation_marker|annotation-marker=s",
    "bibencoding=s",
    "bblencoding=s",
    "bblsafechars",
    "bblsafecharsset=s",
    "cache",
    "clrmacros",
    "collate_options|collate-options|c=s",
    "configfile|g=s",
    "convert_control|convert-control",
    "debug|D",
    "decodecharsset=s",
    "dieondatamodel",
    "dot_include|dot-include:s@",
    "fastsort|f",
    "fixinits",
    "glob_datasources|glob-datasources",
    "help|h|?",
    "input_directory|input-directory=s",
    "input_encoding|input-encodinge=s",
    "input_format|input-format=s",
    "isbn10",
    "isbn13",
    "isbn_normalise|isbn-normalise",
    "listsep=s",
    "logfile=s",
    "mincrossrefs|m=s",
    "named_annotation_marker|named-annotation-marker=s",
    "namesep=s",
    "no_bblxml_schema|no-bblxml-schema",
    "no_bltxml_schema|no-bltxml-schema",
    "noconf",
    "no_default_datamodel|no-default-datamodel",
    "nodieonerror",
    "nolog",
    "nostdmacros",
    "onlylog",
    "others_string|others-string=s",
    "outfile=s",
    "outformat=s",
    "output_align|output-align",
    "output_all_macrodefs|output-all-macrodefs",
    "output_annotation_marker|output-annotation-marker=s",
    "output_directory|output-directory=s",
    "output_encoding|output-encoding|E=s",
    "output_fieldcase|output-fieldcase=s",
    "output_field_order|output-field-order=s",
    "output_field_replace|output-field-replace=s",
    "output_file|output-file|O=s",
    "output_format|output-format=s",
    "output_indent|output-indent=s",
    "output_legacy_dates|output-legacy-dates",
    "output_listsep|output-listsep=s",
    "output_macro_fields|output-macro-fields=s",
    "output_named_annotation_marker|output-named-annotation-marker=s",
    "output_namesep|output-namesep=s",
    "output_no_macrodefs|output-no-macrodefs",
    "output_resolve|output-resolve",
    "output_resolve_xdata|output-resolve-xdata",
    "output_resolve_crossrefs|output-resolve-crossrefs",
    "output_resolve_sets|output-resolve-sets",
    "output_safechars|output-safechars",
    "output_safecharsset|output-safecharsset=s",
    "output_xdatamarker|output-xdatamarker=s",
    "output_xdatasep|output-xdatasep=s",
    "output_xname|output-xname",
    "output_xnamesep|output-xnamesep=s",
    "quiet|q+",
    "recodedata=s",
    "noremove_tmp_dir|noremove-tmp-dir",
    "noskipduplicates",
    "noxname",
    "show_tmp_dir|show-tmp-dir",
    "sortdebug",
    "sortcase=s",
    "sortlocale|l=s",
    "sortupper=s",
    "ssl-nointernalca",
    "ssl-noverify-host",
    "strip_comments|strip-comments",
    "tool",
    "tool_align|tool-align",
    "tool_config|tool-config",
    "tool_fieldcase|tool-fieldcase=s",
    "tool_noremove_missing_dependants|tool-noremove-missing-dependants",
    "tool_indent|tool-indent=s",
    "tool_resolve|tool-resolve",
    "trace|T",
    "u",
    "U",
    "validate_bblxml|validate-bblxml",
    "validate_bltxml|validate-bltxml",
    "validate_config|validate-config",
    "validate_control|validate-control",
    "validate_datamodel|validate-datamodel|V",
    "version|v",
    "winunicode|W",
    "wraplines|w:80",
    "xdatamarker=s",
    "xdatasep=s",
    "xnamesep=s",
    "xsvsep=s",
];

/// bibers-only options (DC-050, DC-068). They match only when spelt out in
/// full, so they never make one of biber's abbreviations ambiguous.
pub const EXTENSION_SPECS: &[&str] = &["fix_quirks|fix-quirks=s", "threads=i"];

/// The full `--help` text (biber's POD rendered by `pod2usage -verbose 2`).
pub const HELP: &str = include_str!("help.txt");

/// The short usage text (`pod2usage -verbose 0`).
pub const USAGE: &str = include_str!("usage.txt");

/// What the prelude decided.
#[derive(Debug, PartialEq, Eq)]
pub enum Prelude {
    /// Exit now with this code, after printing `stdout`/`stderr`.
    Exit {
        /// The exit code.
        code: u8,
        /// Text for STDOUT.
        stdout: String,
        /// Text for STDERR.
        stderr: String,
    },
    /// Carry on with these options (`$opts`) and positional arguments.
    Run {
        /// The option hash.
        opts: BTreeMap<String, Val>,
        /// `@ARGV`.
        argv: Vec<String>,
        /// Messages already printed to STDERR that do not stop the run
        /// (deprecation notices).
        stderr: String,
    },
}

/// A special action requested on the command line that bibers answers
/// itself.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Special {
    /// `--version`
    Version,
    /// `--tool-config`
    ToolConfig,
}

/// The parser for biber's options plus bibers' extensions.
pub fn parser() -> Getopt {
    let mut g = Getopt::new(SPECS, &["collate_options"]);
    for s in EXTENSION_SPECS {
        g.add_exact_only(s);
    }
    g
}

fn truthy(v: Option<&Val>) -> bool {
    match v {
        None => false,
        Some(Val::Scalar(s)) => !s.is_empty() && s != "0",
        Some(_) => true,
    }
}

fn scalar(v: Option<&Val>) -> Option<&str> {
    match v {
        Some(Val::Scalar(s)) => Some(s),
        _ => None,
    }
}

/// Run `GetOptions` and bin/biber's checks/rewrites. `version_text` and
/// `tool_config_text` are what bibers prints for `--version` (DC-040) and
/// `--tool-config`.
pub fn prelude(argv: &[String], special: impl Fn(Special) -> (u8, String, String)) -> Prelude {
    let parsed = parser().parse(argv);
    let mut err = String::new();
    for w in &parsed.warnings {
        err.push_str(w);
        err.push('\n');
    }
    if !parsed.ok() {
        err.push_str(USAGE);
        return Prelude::Exit {
            code: 2,
            stdout: String::new(),
            stderr: err,
        };
    }
    let mut opts = parsed.opts;
    let mut args = parsed.rest;
    // collate_options always exists (a hash ref passed as linkage).
    opts.entry("collate_options".into())
        .or_insert_with(|| Val::Hash(BTreeMap::new()));
    let exit = |code: u8, stdout: String, stderr: String| Prelude::Exit {
        code,
        stdout,
        stderr,
    };
    if opts.contains_key("help") {
        return exit(0, HELP.to_string(), String::new());
    }
    if opts.contains_key("version") {
        let (c, o, e) = special(Special::Version);
        return exit(c, o, e);
    }
    if opts.contains_key("cache") {
        return exit(
            0,
            "No cache - you are not running the PAR::Packer executable version of biber\n".into(),
            String::new(),
        );
    }
    if opts.contains_key("tool_config") {
        let (c, o, e) = special(Special::ToolConfig);
        return exit(c, o, e);
    }
    if args.is_empty() {
        return exit(2, String::new(), USAGE.to_string());
    }

    // Legacy aliases (folded only when the value is true).
    for (from, to) in [
        ("tool_align", "output_align"),
        ("tool_fieldcase", "output_fieldcase"),
        ("tool_indent", "output_indent"),
        ("tool_resolve", "output_resolve"),
        ("bibencoding", "input_encoding"),
        ("bblencoding", "output_encoding"),
        ("bblsafechars", "output_safechars"),
        ("bblsafecharsset", "output_safecharsset"),
        ("outfile", "output_file"),
        ("outformat", "output_format"),
    ] {
        if truthy(opts.get(from)) {
            let v = opts.remove(from).expect("present");
            opts.insert(to.into(), v);
        }
    }
    if truthy(opts.get("u")) {
        opts.insert("input_encoding".into(), Val::Scalar("UTF-8".into()));
        opts.remove("u");
    }
    if truthy(opts.get("U")) {
        opts.insert("output_encoding".into(), Val::Scalar("UTF-8".into()));
        opts.remove("U");
    }
    if opts.remove("output_resolve").is_some() {
        for k in [
            "output_resolve_xdata",
            "output_resolve_crossrefs",
            "output_resolve_sets",
        ] {
            opts.insert(k.into(), Val::Scalar("1".into()));
        }
    }
    if truthy(opts.get("output_macro_fields")) {
        err.push_str("Biber: Deprecated option 'output-macro-fields' - no longer needed as macros are auto-detected on output\n");
    }
    if truthy(opts.get("output_format")) {
        let of = scalar(opts.get("output_format")).unwrap_or("").to_string();
        if !matches!(
            of.as_str(),
            "bbl" | "dot" | "bibtex" | "biblatexml" | "bblxml" | "bibjson"
        ) {
            err.push_str(&format!(
                "Biber: Unknown output format '{of}', must be one of 'bbl', 'dot', 'bibtex', 'biblatexml', 'bblxml', 'bibjson'\n"
            ));
            return exit(2, String::new(), err);
        }
        if of == "bblxml" || of == "dot" {
            err.push_str(&format!(
                "Biber: Deprecated output format '{of}' - this will be removed in a future version\n"
            ));
        }
    }
    let tool = opts.contains_key("tool");
    if tool && !opts.contains_key("input_format") {
        let a0 = args[0].strip_suffix('\n').unwrap_or(&args[0]);
        if a0.ends_with(".bib") {
            opts.insert("input_format".into(), Val::Scalar("bibtex".into()));
        } else if a0.ends_with(".bltxml") {
            opts.insert("input_format".into(), Val::Scalar("biblatexml".into()));
        }
    }
    if tool
        && let Some(of) = opts.get("output_format")
        && !matches!(scalar(Some(of)), Some("bibtex" | "biblatexml" | "bibjson"))
    {
        err.push_str("Biber: Output format in tool mode must be one of 'bibtex', 'biblatexml' or 'bibjson'\n");
        return exit(2, String::new(), err);
    }
    if !opts.contains_key("output_format") {
        let d = if tool { "bibtex" } else { "bbl" };
        opts.insert("output_format".into(), Val::Scalar(d.into()));
    }
    if opts.contains_key("isbn10") && opts.contains_key("isbn13") {
        err.push_str("Biber: Select only one of 'isbn10' or 'isbn13' but not both\n");
        return exit(2, String::new(), err);
    }
    if let Some(v) = opts.get("output_indent") {
        let s = scalar(Some(v)).unwrap_or("");
        let s = s.strip_suffix('\n').unwrap_or(s);
        let digits = s.strip_suffix('t').unwrap_or(s);
        if digits.is_empty() || !digits.chars().all(|c| c.is_numeric()) {
            err.push_str("Biber: Invalid non-numeric argument for 'output_indent' option!\n");
            return exit(2, String::new(), err);
        }
    }
    if let Some(v) = opts.get("output_fieldcase") {
        let s = scalar(Some(v)).unwrap_or("").to_ascii_lowercase();
        let s = s.strip_suffix('\n').unwrap_or(&s);
        if !matches!(s, "upper" | "lower" | "title") {
            err.push_str("Biber: Invalid argument for 'output_fieldcase' option - must be one of 'upper', 'lower' or 'title'!\n");
            return exit(2, String::new(), err);
        }
    }
    if opts.contains_key("dot_include") && scalar(opts.get("output_format")) != Some("dot") {
        err.push_str("Biber: DOT output format specified but output format is not DOT!\n");
        return exit(2, String::new(), err);
    }
    if let Some(Val::Array(vals)) = opts.get("dot_include") {
        let joined = vals.join(",");
        // split(/,/, …): trailing empty fields are dropped.
        let mut parts: Vec<&str> = joined.split(',').collect();
        while parts.last() == Some(&"") {
            parts.pop();
        }
        let h: BTreeMap<String, String> = parts
            .iter()
            .map(|p| (p.to_lowercase(), "1".to_string()))
            .collect();
        for g in h.keys() {
            if !["section", "field", "crossref", "xref", "xdata", "related"].contains(&g.as_str()) {
                err.push_str(&format!(
                    "Biber: '{g}' is an invalid output type for DOT output\n"
                ));
                return exit(2, String::new(), err);
            }
        }
        opts.insert("dot_include".into(), Val::Hash(h));
    }
    if opts.contains_key("input_format") && !tool {
        err.push_str("Biber: 'input_format' option is only valid in tool mode\n");
        return exit(2, String::new(), err);
    }
    if let Some(v) = opts.get("input_format") {
        let s = scalar(Some(v)).unwrap_or("");
        let l = s.to_ascii_lowercase();
        let l = l.strip_suffix('\n').unwrap_or(&l);
        if !matches!(l, "bibtex" | "biblatexml" | "") {
            err.push_str(&format!(
                "Biber: {s} is an invalid input format in tool mode\n"
            ));
            return exit(2, String::new(), err);
        }
    }
    args.shrink_to_fit();
    Prelude::Run {
        opts,
        argv: args,
        stderr: err,
    }
}

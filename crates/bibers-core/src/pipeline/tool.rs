//! Tool mode (`--tool`): `Biber::tool_mode_setup` (Biber.pm:282),
//! `process_setup_tool` (Biber.pm:1237) and `prepare_tool` (Biber.pm:4520).
//!
//! Tool mode is data-source centric: there is no `.bcf`. One pseudo-section
//! `99999` holds every entry of the data sources named on the command line
//! (`allkeys`), with one `entry` data list sorted by the `tool` sorting
//! template of biber-tool.conf. Most of the processing pipeline is skipped
//! (see [`TOOL_PHASES`] and `lists::process_lists`).

use super::{Ctx, Processed, SectionPhase, fetch, lists, resolve};
use crate::bcf::{self, DataList, DataSource, Document};
use crate::config::{BiberValue, Config, OptValue};
use crate::log::{Fatal, Logger};

/// The tool-mode pseudo-section.
pub const TOOL_SECTION: &str = "99999";

/// `tool_mode_setup` (Biber.pm:282-334): fake the parts of the control file
/// tool mode needs. `argv` are the data sources (`@ARGV`).
pub fn tool_mode_setup(cfg: &mut Config, argv: &[String]) -> Document {
    let mut section = bcf::Section::new(TOOL_SECTION);
    let opt = |o: &str| cfg.biber.get_str(o).map(str::to_string);
    section.datasources = Some(
        argv.iter()
            .map(|a| DataSource {
                type_: Some("file".into()),
                name: Some(a.clone()),
                datatype: opt("input_format"),
                encoding: opt("input_encoding"),
                glob: None,
            })
            .collect(),
    );
    section.allkeys = true;

    // Always resolve date meta-information in tool mode.
    for o in ["dateapproximate", "dateera", "dateuncertain"] {
        cfg.blx.set_global(o, OptValue::Str("1".into()));
    }
    cfg.blx
        .set_global("namestrunchandling", OptValue::Str("0".into()));

    let mut dl = DataList::default_entry(TOOL_SECTION, "tool");
    dl.name = "tool/global//global/global/global".into();
    // Locale just needs a default here.
    cfg.blx
        .set_global("sortlocale", OptValue::Str("en_US".into()));

    // User maps come from the config file: force "user" level.
    if let Some(BiberValue::Sourcemap(maps)) = cfg.biber.get("sourcemap").cloned() {
        let maps = maps
            .into_iter()
            .map(|mut m| {
                m.level = Some("user".into());
                m
            })
            .collect();
        cfg.biber.set("sourcemap", BiberValue::Sourcemap(maps));
    }

    Document {
        sections: vec![section],
        datalists: vec![dl],
    }
}

/// `process_setup_tool` (Biber.pm:1237-1251).
pub fn process_setup_tool(cfg: &mut Config, log: &mut Logger) -> Result<(), Fatal> {
    bcf::setup_datamodel(cfg, log)
}

/// The section phases of `prepare_tool` (Biber.pm:4535-4546), in order.
/// Unlike `prepare`, there is no citekey alias processing, dynamic set
/// instantiation, set member citing, set post-processing or static entry
/// processing, and XDATA is resolved after crossrefs.
pub const TOOL_PHASES: &[(&str, SectionPhase)] = &[
    ("fetch_data", fetch::fetch_data),
    ("resolve_alias_refs", resolve::resolve_alias_refs),
    ("preprocess_sets", resolve::preprocess_sets),
    ("calculate_interentry", resolve::calculate_interentry),
    ("process_interentry", resolve::process_interentry),
    ("resolve_xdata", resolve::resolve_xdata),
    ("validate_datamodel", resolve::validate_datamodel),
    ("process_lists", lists::process_lists),
];

/// `prepare_tool` (Biber.pm:4520-4549): process the tool-mode section.
pub fn prepare_tool(ctx: &mut Ctx) -> Result<Processed, Fatal> {
    let mut st = fetch::init_section(ctx, TOOL_SECTION)?;
    for (_, phase) in TOOL_PHASES {
        phase(ctx, &mut st)?;
    }
    Ok(Processed { sections: vec![st] })
}

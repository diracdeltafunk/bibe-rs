//! The bibers processing pipeline: control file and configuration, data
//! model, data sources, dependency resolution, labels, uniqueness, sorting and
//! output. See `docs/PLAN.md` §5 for the architecture.
//!
//! Implemented so far (M1): the configuration front end — [`config`]
//! (biber/biblatex options, biber.conf), [`bcf`] (the control file),
//! [`datamodel`], [`log`] (the `.blg`/stdout logger).

pub mod bcf;
pub mod config;
pub mod datamodel;
pub mod env;
pub mod files;
pub mod log;
pub mod model;
pub mod output;
pub mod perlish;
pub mod pipeline;
pub mod quirks;
pub mod recodedata;
pub mod remote;
pub mod xmlin;

/// The biber version bibers is compatible with.
pub const BIBER_VERSION: &str = "2.22";

/// The `.bbl` format version biber 2.22 writes.
pub const BBL_VERSION: &str = "3.3";

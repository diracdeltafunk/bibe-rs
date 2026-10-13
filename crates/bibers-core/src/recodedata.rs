//! `--recodedata`: a user recode data file replacing biber's built-in
//! `recode_data.xml` (`Biber::LaTeX::Recode::init_sets`, Recode.pm:78-120,
//! called from `Biber->new`).
//!
//! biber locates the file with `kpsewhich` only (so `./file.xml` is found
//! through TeX's search path, which includes the current directory) and logs
//! the path kpsewhich printed. Where biber dies with a Perl error (file not
//! found: `Couldn't open : No such file or directory`; malformed XML: a
//! libxml2 error), bibers reports an ERROR instead (docs/compat.md, proposed
//! DC row for `--recodedata` errors).

use std::sync::Arc;

use crate::config::Config;
use crate::env::Env;
use crate::log::{Fatal, Logger};
use crate::{biber_error, info};

/// Load the `--recodedata` file, if the option is set, into `cfg.recodedata`.
pub fn load(cfg: &mut Config, log: &mut Logger, env: &Env) -> Result<(), Fatal> {
    let Some(name) = cfg
        .biber
        .get_str("recodedata")
        .filter(|v| !v.is_empty() && *v != "0")
        .map(str::to_string)
    else {
        return Ok(());
    };
    let path = if env.use_kpsewhich {
        if !env.can_run_kpsewhich() {
            biber_error!(
                log,
                "Can't run kpsewhich to look for output_safechars data file: "
            )?;
            return Ok(());
        }
        env.kpsewhich(&name)
    } else {
        // Test environments without kpsewhich: the current directory only.
        let p = format!("./{name}");
        env.exists(&p).then_some(p)
    };
    info!(
        log,
        "Using user-defined recode data file '{}'",
        path.as_deref().unwrap_or("")
    );
    let Some(path) = path else {
        biber_error!(log, "Cannot find recode data file '{name}'")?;
        return Ok(());
    };
    let xml = match std::fs::read(env.resolve(&path)) {
        Ok(b) => match String::from_utf8(b) {
            Ok(s) => s,
            Err(_) => {
                biber_error!(log, "Recode data file '{path}' is not valid UTF-8")?;
                return Ok(());
            }
        },
        Err(e) => {
            biber_error!(log, "Cannot read recode data file '{path}': {e}")?;
            return Ok(());
        }
    };
    match latex_recode::RecodeData::from_xml(&xml) {
        Ok(d) => cfg.recodedata = Some(Arc::new(d)),
        Err(e) => {
            biber_error!(log, "Cannot parse recode data file '{path}': {}", e.0)?;
        }
    }
    Ok(())
}

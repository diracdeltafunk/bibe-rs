//! Locating input files (`locate_data_file`, Utils.pm:226-388).

use crate::env::{Env, catfile, dirname_with_slash};
use crate::perlish::{nfc, nfd};

/// `file_exist_check`: the name itself, or its NFC or NFD variant, if it
/// exists.
pub fn file_exist_check(env: &Env, name: &str) -> Option<String> {
    if env.exists(name) {
        return Some(name.to_string());
    }
    let c = nfc(name);
    if env.exists(&c) {
        return Some(c);
    }
    let d = nfd(name);
    if env.exists(&d) {
        return Some(d);
    }
    None
}

/// Is this a remote (`http`, `https`, `ftp`, `ftps`) source?
pub fn is_remote(source: &str) -> bool {
    ["http://", "https://", "ftp://", "ftps://"]
        .iter()
        .any(|p| source.starts_with(p))
}

/// Where to look for a data file. `ctrlfile_path` is the located `.bcf`
/// (`undef` while locating the `.bcf` itself).
pub struct Lookup<'a> {
    /// `--input-directory`.
    pub input_directory: Option<&'a str>,
    /// `--output-directory`.
    pub output_directory: Option<&'a str>,
    /// The control file's path, once known.
    pub ctrlfile_path: Option<&'a str>,
}

/// `locate_data_file` for a local source: absolute path, input (else
/// output) directory, cwd, the control file's directory, then `kpsewhich`.
/// Returns `None` when not found (biber then reports `Cannot find '<source>'!`).
/// Remote sources are not handled here (DC-032).
pub fn locate_data_file(env: &Env, lookup: &Lookup<'_>, source: &str) -> Option<String> {
    let sourcepath = source;
    let foundfile = if let Some(indir) = lookup
        .input_directory
        .filter(|d| !d.is_empty() && *d != "0")
    {
        Some(catfile(indir, sourcepath))
    } else {
        lookup
            .output_directory
            .filter(|d| !d.is_empty() && *d != "0")
            .map(|outdir| catfile(outdir, sourcepath))
    };
    if std::path::Path::new(sourcepath).is_absolute()
        && let Some(f) = file_exist_check(env, sourcepath)
    {
        return Some(f);
    }
    if let Some(ff) = &foundfile
        && let Some(f) = file_exist_check(env, ff)
    {
        return Some(f);
    }
    if let Some(f) = file_exist_check(env, sourcepath) {
        return Some(f);
    }
    if let Some(cfp) = lookup.ctrlfile_path.filter(|c| !c.is_empty() && *c != "0") {
        let dir = dirname_with_slash(cfp);
        let path = format!("{dir}{sourcepath}");
        if let Some(f) = file_exist_check(env, &path) {
            return Some(f);
        }
    }
    if let Some(found) = env.kpsewhich(sourcepath) {
        // biber returns file_exist_check's result even if it is undef.
        return file_exist_check(env, &found);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("in")).unwrap();
        std::fs::create_dir(dir.path().join("ctl")).unwrap();
        std::fs::write(dir.path().join("in/a.bib"), "").unwrap();
        std::fs::write(dir.path().join("a.bib"), "").unwrap();
        std::fs::write(dir.path().join("ctl/b.bib"), "").unwrap();
        let env = Env::isolated(dir.path());
        let lk = Lookup {
            input_directory: Some("in"),
            output_directory: None,
            ctrlfile_path: Some("ctl/x.bcf"),
        };
        assert_eq!(
            locate_data_file(&env, &lk, "a.bib").as_deref(),
            Some("in/a.bib")
        );
        assert_eq!(
            locate_data_file(&env, &lk, "b.bib").as_deref(),
            Some("ctl/b.bib")
        );
        assert_eq!(locate_data_file(&env, &lk, "c.bib"), None);
    }
}

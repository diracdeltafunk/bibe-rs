//! The process environment the front end consults (cwd, home directories,
//! `kpsewhich`), made explicit so tests can run without changing the
//! process-global working directory.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Environment for file lookups.
#[derive(Clone, Debug)]
pub struct Env {
    /// The working directory relative paths are resolved against.
    pub cwd: PathBuf,
    /// `$HOME`.
    pub home: Option<String>,
    /// `$XDG_CONFIG_HOME`.
    pub xdg_config_home: Option<String>,
    /// `%APPDATA%`.
    pub appdata: Option<String>,
    /// Whether `kpsewhich` may be run.
    pub use_kpsewhich: bool,
}

impl Env {
    /// The real process environment.
    pub fn from_process() -> Self {
        Env {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            home: std::env::var("HOME").ok(),
            xdg_config_home: std::env::var("XDG_CONFIG_HOME").ok(),
            appdata: std::env::var("APPDATA").ok(),
            use_kpsewhich: true,
        }
    }

    /// An environment rooted at `cwd` with no home directories and no
    /// `kpsewhich` (for tests).
    pub fn isolated(cwd: impl Into<PathBuf>) -> Self {
        Env {
            cwd: cwd.into(),
            home: None,
            xdg_config_home: None,
            appdata: None,
            use_kpsewhich: false,
        }
    }

    /// Resolve a path the way the OS would from `cwd`.
    pub fn resolve(&self, p: &str) -> PathBuf {
        let path = Path::new(p);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        }
    }

    /// `-e $p`.
    pub fn exists(&self, p: &str) -> bool {
        !p.is_empty() && self.resolve(p).exists()
    }

    /// `-f $p`.
    pub fn is_file(&self, p: &str) -> bool {
        !p.is_empty() && self.resolve(p).is_file()
    }

    /// Run `kpsewhich <name>` and return its first output line, if any.
    pub fn kpsewhich(&self, name: &str) -> Option<String> {
        if !self.use_kpsewhich {
            return None;
        }
        let out = Command::new("kpsewhich")
            .arg(name)
            .current_dir(&self.cwd)
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout).into_owned();
        let s = s.strip_suffix('\n').unwrap_or(&s);
        let s = s.strip_suffix('\r').unwrap_or(s);
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    }
}

/// `File::Spec->catfile($dir, $file)` (Unix semantics).
pub fn catfile(dir: &str, file: &str) -> String {
    if dir.is_empty() {
        return file.to_string();
    }
    let d = dir.trim_end_matches('/');
    if d.is_empty() {
        format!("/{file}")
    } else {
        format!("{d}/{file}")
    }
}

/// The file part of `File::Spec->splitpath($p)`.
pub fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// The directory part of `File::Spec->splitpath($p)` (with trailing `/`).
pub fn dirname_with_slash(p: &str) -> &str {
    match p.rfind('/') {
        Some(i) => &p[..=i],
        None => "",
    }
}

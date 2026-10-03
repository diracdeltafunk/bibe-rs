//! The `.blg`/stdout logger, reproducing biber's Log::Log4perl set-up
//! (Config.pm:257-313) and its warning/error bookkeeping (Utils.pm:463-487,
//! Biber.pm:117-139).
//!
//! * Log file lines: `[<ms>] <location>> <LEVEL> - <message>`. biber's
//!   location is the Perl `File.pm:line` of the logging call; ours is the Rust
//!   `file.rs:line` (DC-041). Nothing between `[` and `> ` contains `> `, which
//!   latexmk relies on.
//! * Screen (stdout) lines: `<LEVEL> - <message>`.
//! * `biber_warn` messages are queued and emitted at the end by
//!   [`Logger::display_end`], followed by `WARNINGS: n`; `biber_error` logs
//!   immediately, counts, and is fatal unless `--nodieonerror`.
//! * Levels: `--trace` > `--debug` > default INFO > `-q` ERROR > `-qq` FATAL;
//!   the screen never shows more than INFO; `--nolog` disables the file;
//!   `--onlylog` disables the screen.

use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Instant;

use crate::perlish::nfc;

/// Log levels, in Log4perl order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// TRACE
    Trace,
    /// DEBUG
    Debug,
    /// INFO
    Info,
    /// WARN
    Warn,
    /// ERROR
    Error,
    /// FATAL
    Fatal,
    /// OFF (threshold only)
    Off,
}

impl Level {
    /// The Log4perl level name.
    pub fn name(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
            Level::Fatal => "FATAL",
            Level::Off => "OFF",
        }
    }
}

/// A source location tag for the log file (DC-041).
#[derive(Clone, Copy, Debug)]
pub struct Loc {
    /// Base name of the Rust source file.
    pub file: &'static str,
    /// Line number.
    pub line: u32,
}

impl Loc {
    /// Build from `file!()`/`line!()`.
    pub fn new(file: &'static str, line: u32) -> Self {
        let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
        Loc { file: base, line }
    }
}

/// The current source location, for [`Logger::log`].
#[macro_export]
macro_rules! loc {
    () => {
        $crate::log::Loc::new(file!(), line!())
    };
}

/// Log at INFO with the current location: `info!(logger, "fmt", args…)`.
#[macro_export]
macro_rules! info {
    ($lg:expr, $($arg:tt)*) => {
        $lg.log($crate::log::Level::Info, $crate::loc!(), &format!($($arg)*))
    };
}

/// Log at DEBUG with the current location.
#[macro_export]
macro_rules! debug {
    ($lg:expr, $($arg:tt)*) => {
        if $lg.is_debug() {
            $lg.log($crate::log::Level::Debug, $crate::loc!(), &format!($($arg)*))
        }
    };
}

/// Log at TRACE with the current location.
#[macro_export]
macro_rules! trace {
    ($lg:expr, $($arg:tt)*) => {
        if $lg.is_trace() {
            $lg.log($crate::log::Level::Trace, $crate::loc!(), &format!($($arg)*))
        }
    };
}

/// `biber_error` with the current location: evaluates to
/// `Result<(), Fatal>`.
#[macro_export]
macro_rules! biber_error {
    ($lg:expr, $($arg:tt)*) => {
        $lg.biber_error($crate::loc!(), &format!($($arg)*))
    };
}

/// A fatal error: biber would have called `display_end` and exited with
/// code 2. The message has already been logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fatal;

impl fmt::Display for Fatal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("fatal error (already logged)")
    }
}

impl std::error::Error for Fatal {}

static START: OnceLock<Instant> = OnceLock::new();

/// Record the program start time for the `[ms]` log column. Call first thing
/// in `main`; otherwise the first logger creation counts as the start.
pub fn mark_program_start() {
    let _ = START.get_or_init(Instant::now);
}

fn elapsed_ms() -> u128 {
    START.get_or_init(Instant::now).elapsed().as_millis()
}

/// Logger settings, derived from the biber options (Config.pm:274-313).
#[derive(Clone, Debug)]
pub struct LogSettings {
    /// Level of the `main` category.
    pub main_level: Level,
    /// Threshold of the log file appender (`Off` = no file).
    pub file_level: Level,
    /// Threshold of the screen appender.
    pub screen_level: Level,
}

impl LogSettings {
    /// Compute from `trace`/`debug`/`quiet`/`nolog`/`onlylog`.
    pub fn new(trace: bool, debug: bool, quiet: i64, nolog: bool, onlylog: bool) -> Self {
        let main = if trace {
            Level::Trace
        } else if debug {
            Level::Debug
        } else if quiet == 1 {
            Level::Error
        } else if quiet > 1 {
            Level::Fatal
        } else {
            Level::Info
        };
        let file = if nolog { Level::Off } else { main };
        let screen = if onlylog {
            Level::Off
        } else if quiet == 1 {
            Level::Error
        } else if quiet > 1 {
            Level::Fatal
        } else {
            Level::Info
        };
        LogSettings {
            main_level: main,
            file_level: file,
            screen_level: screen,
        }
    }
}

enum Sink {
    File(BufWriter<File>),
    Memory(Vec<String>),
}

/// The logger: writes the `.blg` and stdout, queues warnings and counts
/// errors. One per run.
pub struct Logger {
    settings: LogSettings,
    file: Option<Sink>,
    screen: Option<Sink>,
    warnings: Vec<String>,
    errors: u32,
    nodieonerror: bool,
}

impl Logger {
    /// A logger writing the screen to stdout and the log file to `logfile`
    /// (created/truncated now, like Log4perl's `clobber` mode). With
    /// `file_level == Off` no file is created.
    pub fn new(settings: LogSettings, logfile: Option<&Path>) -> io::Result<Self> {
        let file = match (settings.file_level, logfile) {
            (Level::Off, _) | (_, None) => None,
            (_, Some(p)) => Some(Sink::File(BufWriter::new(File::create(p)?))),
        };
        Ok(Logger {
            settings,
            file,
            screen: None,
            warnings: Vec::new(),
            errors: 0,
            nodieonerror: false,
        })
    }

    /// A logger that records both streams in memory (for tests and
    /// in-process use). Screen lines are prefixed with `S:`, file lines with
    /// `F:` (without the `[ms]` column).
    pub fn memory(settings: LogSettings) -> Self {
        Logger {
            file: if settings.file_level == Level::Off {
                None
            } else {
                Some(Sink::Memory(Vec::new()))
            },
            screen: Some(Sink::Memory(Vec::new())),
            settings,
            warnings: Vec::new(),
            errors: 0,
            nodieonerror: false,
        }
    }

    /// A silent logger (everything off) that still counts errors and queues
    /// warnings.
    pub fn silent() -> Self {
        Logger {
            settings: LogSettings {
                main_level: Level::Off,
                file_level: Level::Off,
                screen_level: Level::Off,
            },
            file: None,
            screen: Some(Sink::Memory(Vec::new())),
            warnings: Vec::new(),
            errors: 0,
            nodieonerror: false,
        }
    }

    /// Set `--nodieonerror`.
    pub fn set_nodieonerror(&mut self, v: bool) {
        self.nodieonerror = v;
    }

    /// `$logger->level($DEBUG)` after the `.bcf` set `debug` (bin/biber:426).
    /// Only the category level changes; appender thresholds stay.
    pub fn set_main_level(&mut self, level: Level) {
        self.settings.main_level = level;
    }

    /// `$logger->is_debug()`.
    pub fn is_debug(&self) -> bool {
        self.settings.main_level <= Level::Debug
    }

    /// `$logger->is_trace()`.
    pub fn is_trace(&self) -> bool {
        self.settings.main_level <= Level::Trace
    }

    fn write_file(&mut self, level: Level, loc: Loc, msg: &str) {
        if level < self.settings.file_level {
            return;
        }
        match &mut self.file {
            Some(Sink::File(w)) => {
                let _ = writeln!(
                    w,
                    "[{}] {}:{}> {} - {}",
                    elapsed_ms(),
                    loc.file,
                    loc.line,
                    level.name(),
                    msg
                );
            }
            Some(Sink::Memory(v)) => v.push(format!(
                "F:{}:{}> {} - {}",
                loc.file,
                loc.line,
                level.name(),
                msg
            )),
            None => {}
        }
    }

    fn write_screen(&mut self, level: Level, msg: &str) {
        if level < self.settings.screen_level {
            return;
        }
        match &mut self.screen {
            None => {
                let out = io::stdout();
                let mut l = out.lock();
                let _ = writeln!(l, "{} - {}", level.name(), msg);
            }
            Some(Sink::Memory(v)) => v.push(format!("S:{} - {}", level.name(), msg)),
            Some(Sink::File(w)) => {
                let _ = writeln!(w, "{} - {}", level.name(), msg);
            }
        }
    }

    /// Log to the `main` category (log file and screen).
    pub fn log(&mut self, level: Level, loc: Loc, msg: &str) {
        if level < self.settings.main_level {
            return;
        }
        self.write_file(level, loc, msg);
        self.write_screen(level, msg);
    }

    /// Log to the `screen` category only (e.g. the `DEBUG mode` notice).
    pub fn screen_info(&mut self, msg: &str) {
        self.write_screen(Level::Info, msg);
    }

    /// Log to the `logfile` category only (the `=== <date>` line).
    pub fn logfile_info(&mut self, loc: Loc, msg: &str) {
        self.write_file(Level::Info, loc, msg);
    }

    /// `biber_warn`: queue a warning for [`Logger::display_end`].
    pub fn biber_warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }

    /// `biber_error($msg)`: log an ERROR and count it. Returns `Err(Fatal)`
    /// unless `--nodieonerror`; the caller must then stop and call
    /// [`Logger::display_end`] (biber exits with code 2).
    pub fn biber_error(&mut self, loc: Loc, msg: &str) -> Result<(), Fatal> {
        self.biber_error_nodie(loc, msg);
        if self.nodieonerror {
            Ok(())
        } else {
            Err(Fatal)
        }
    }

    /// `biber_error($msg, 1)`: log and count, never fatal.
    pub fn biber_error_nodie(&mut self, loc: Loc, msg: &str) {
        self.log(Level::Error, loc, msg);
        self.errors += 1;
    }

    /// Queued warnings so far.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Number of errors so far.
    pub fn errors(&self) -> u32 {
        self.errors
    }

    /// Lines recorded by a [`Logger::memory`] logger (screen lines then
    /// file lines are interleaved in emission order per sink).
    pub fn memory_lines(&self) -> (Vec<String>, Vec<String>) {
        let get = |s: &Option<Sink>| match s {
            Some(Sink::Memory(v)) => v.clone(),
            _ => Vec::new(),
        };
        (get(&self.screen), get(&self.file))
    }

    /// `display_end` (Biber.pm:117): emit the queued warnings (NFC) and the
    /// `WARNINGS:`/`ERRORS:` summary. Returns the process exit code: 2 if any
    /// error was reported, else 0.
    pub fn display_end(&mut self) -> u8 {
        if !self.warnings.is_empty() {
            let ws = std::mem::take(&mut self.warnings);
            for w in &ws {
                self.log(Level::Warn, crate::loc!(), &nfc(w));
            }
            let n = ws.len();
            self.warnings = ws;
            self.log(Level::Info, crate::loc!(), &format!("WARNINGS: {n}"));
        }
        let code = if self.errors > 0 {
            let n = self.errors;
            self.log(Level::Info, crate::loc!(), &format!("ERRORS: {n}"));
            2
        } else {
            0
        };
        self.flush();
        code
    }

    /// Flush the log file.
    pub fn flush(&mut self) {
        if let Some(Sink::File(w)) = &mut self.file {
            let _ = w.flush();
        }
        let _ = io::stdout().flush();
    }
}

impl Drop for Logger {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_summary() {
        let mut lg = Logger::memory(LogSettings::new(false, false, 0, false, false));
        lg.log(Level::Info, crate::loc!(), "hello");
        lg.log(Level::Debug, crate::loc!(), "hidden");
        lg.biber_warn("w1");
        lg.set_nodieonerror(true);
        assert!(lg.biber_error(crate::loc!(), "bad").is_ok());
        assert_eq!(lg.display_end(), 2);
        let (screen, file) = lg.memory_lines();
        assert_eq!(
            screen,
            vec![
                "S:INFO - hello",
                "S:ERROR - bad",
                "S:WARN - w1",
                "S:INFO - WARNINGS: 1",
                "S:INFO - ERRORS: 1"
            ]
        );
        assert_eq!(file.len(), 5);
        assert!(file[0].starts_with("F:log.rs:"));
    }

    #[test]
    fn quiet() {
        let mut lg = Logger::memory(LogSettings::new(false, false, 1, false, false));
        lg.log(Level::Info, crate::loc!(), "x");
        lg.log(Level::Warn, crate::loc!(), "w");
        lg.log(Level::Error, crate::loc!(), "e");
        let (screen, _) = lg.memory_lines();
        assert_eq!(screen, vec!["S:ERROR - e"]);
    }
}

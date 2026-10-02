//! Normalisations applied to outputs before comparison (`docs/PLAN.md` §6.3).
//!
//! These undo nondeterminism in biber itself (Perl hash order) and strip
//! run-specific noise. They are applied to *both* sides of every comparison.

use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

/// Normalise `.bbl` text: sort the comma-separated per-name/per-namelist
/// option lists, which biber emits in random hash order (BB-060, DC-013):
///
/// - `\name{author}{2}{<opts>}{%`
/// - `{{<opts>}{%` (one name inside a `\name` block)
pub fn bbl(text: &str) -> String {
    static NAMELIST: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(\\name\{[^}]*\}\{\d+\}\{)([^}]*)(\}\{%)").unwrap());
    static NAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^(\s*\{\{)([^}]*)(\}\{%)$").unwrap());
    let sort_opts = |caps: &regex::Captures| {
        let mut opts: Vec<&str> = caps[2].split(',').collect();
        opts.sort_unstable();
        format!("{}{}{}", &caps[1], opts.join(","), &caps[3])
    };
    let text = NAMELIST.replace_all(text, sort_opts);
    NAME.replace_all(&text, sort_opts).into_owned()
}

/// Extract the WARN and ERROR messages from a `.blg`, normalised and sorted
/// (their order is nondeterministic in biber, BB-065).
///
/// Strips the `[ms] File:line>` prefix, replaces the work directory with
/// `<WORK>`, and replaces btparse temp-file paths (`…/<md5>_<pid>.utf8`) with
/// `<TMPBIB>` (DC-042).
pub fn blg_messages(blg: &str, workdir: &Path) -> Vec<String> {
    static LINE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\[\d+\]\s+\S+>\s+(WARN|ERROR)\s+-\s+(.*)$").unwrap());
    static TMPBIB: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\S*/[0-9a-f]{32}_\d+\.utf8").unwrap());
    // macOS reports /var/... temp dirs as /private/var/... depending on how
    // the path was obtained; normalise both spellings.
    let work = workdir.to_string_lossy().into_owned();
    let work_alt = work
        .strip_prefix("/private")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("/private{work}"));
    let mut msgs: Vec<String> = blg
        .lines()
        .filter_map(|l| LINE.captures(l))
        .map(|c| {
            let msg = c[2].replace(&work, "<WORK>").replace(&work_alt, "<WORK>");
            let msg = TMPBIB.replace_all(&msg, "<TMPBIB>");
            format!("{} - {}", &c[1], msg)
        })
        .collect();
    msgs.sort();
    msgs
}

/// Lines of `text`, sorted (for [`crate::CompareMode::UnorderedLines`]).
pub fn sorted_lines(text: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_unstable();
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_name_options() {
        let a = "      \\name{author}{1}{useprefix=true,giveninits=false}{%\n\
                 \x20       {{un=0,uniquepart=base,hash=abc}{%\n";
        let b = "      \\name{author}{1}{giveninits=false,useprefix=true}{%\n\
                 \x20       {{hash=abc,un=0,uniquepart=base}{%\n";
        assert_eq!(bbl(a), bbl(b));
    }

    #[test]
    fn extracts_messages() {
        let blg = "[0] Config.pm:1> INFO - This is Biber 2.22\n\
                   [12] Biber.pm:2> WARN - I didn't find a database entry for 'x' (section 0)\n\
                   [13] Utils.pm:3> ERROR - BibTeX subsystem: /var/folders/zz/T/0123456789abcdef0123456789abcdef_42.utf8, line 3, syntax error\n";
        let msgs = blg_messages(blg, Path::new("/tmp/w"));
        assert_eq!(
            msgs,
            vec![
                "ERROR - BibTeX subsystem: <TMPBIB>, line 3, syntax error".to_string(),
                "WARN - I didn't find a database entry for 'x' (section 0)".to_string(),
            ]
        );
    }
}

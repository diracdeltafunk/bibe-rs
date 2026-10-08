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
/// `<WORK>`, replaces btparse temp-file paths (`…/<md5>_<pid>.utf8`) with
/// `<TMPBIB>` (DC-042), and replaces Perl source locations embedded in die
/// messages (`at /…/par-…/inc/lib/File/Slurper.pm line 59.`), which differ
/// between the PAR binary, the source build and machines, with
/// `at <PERLSRC> line <N>`.
pub fn blg_messages(blg: &str, workdir: &Path) -> Vec<String> {
    static LINE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\[\d+\]\s+\S+>\s+(WARN|ERROR)\s+-\s+(.*)$").unwrap());
    static TMPBIB: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[^\s']*/[0-9a-f]{32}_\d+\.utf8").unwrap());
    static PERLSRC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bat \S+\.p[ml] line \d+").unwrap());
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
            let msg = PERLSRC.replace_all(&msg, "at <PERLSRC> line <N>");
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
                   [13] Utils.pm:3> ERROR - BibTeX subsystem: /var/folders/zz/T/0123456789abcdef0123456789abcdef_42.utf8, line 3, syntax error\n\
                   [14] Biber.pm:4> ERROR - Data file 'a.bib' cannot be read in encoding 'utf8': Can't decode ill-formed UTF-8 octet sequence <A6> at /var/folders/T/par-62656e/cache-bc/inc/lib/File/Slurper.pm line 59.\n";
        let msgs = blg_messages(blg, Path::new("/tmp/w"));
        assert_eq!(
            msgs,
            vec![
                "ERROR - BibTeX subsystem: <TMPBIB>, line 3, syntax error".to_string(),
                "ERROR - Data file 'a.bib' cannot be read in encoding 'utf8': Can't decode ill-formed UTF-8 octet sequence <A6> at <PERLSRC> line <N>.".to_string(),
                "WARN - I didn't find a database entry for 'x' (section 0)".to_string(),
            ]
        );
    }
}

#!/usr/bin/env python3
"""Watch biber and biblatex upstream for changes bibers may need to port.

Reports:
  * biber: new release tags after the version vendor/biber is pinned to, and
    commits on biber's `dev` branch since that pin that touch processing code
    (lib/, data/, bin/) or tests (t/). Edits that only change $VERSION or
    $BETA_VERSION are ignored, as are build and packaging changes (dist/ etc.).
  * biblatex: new release tags after BIBLATEX_BASELINE, a change of the control
    file version (\\blx@bcfversion) on `dev` from bibers' BCF_VERSION, and
    commits on `dev` since the baseline that touch the default data model
    (blx-dm.def).

With --issue, the report goes to one open GitHub issue labelled
`upstream-watch` in this repository: created if none is open, otherwise a new
comment, but only when the findings differ from what that issue already holds
(each report carries a hash of its findings). Without --issue the report is
printed. Needs `gh` with a token that can read public repos (and write issues
for --issue).

Bump BIBLATEX_BASELINE when bibers moves to a new biblatex; the biber baseline
follows the vendor/biber submodule pin automatically.
"""

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

BIBER = "plk/biber"
BIBLATEX = "plk/biblatex"
BIBLATEX_BASELINE = "v3.22"
BRANCH = "dev"

BIBER_PATHS = ["lib", "data", "bin", "t"]
BIBLATEX_DM = "tex/latex/biblatex/blx-dm.def"
BIBLATEX_STY = "tex/latex/biblatex/biblatex.sty"

LABEL = "upstream-watch"
TITLE = "Upstream changes in biber/biblatex to review"
MARKER = "upstream-watch-key"

# A changed line that only sets a version number.
VERSION_LINE = re.compile(r"^[+-]\s*our\s+\$(BETA_)?VERSION\s*=\s*['\"]?[\d.]+['\"]?\s*;")


def gh(*args, raw=False):
    cmd = ["gh", "api", *args]
    if raw:
        cmd += ["-H", "Accept: application/vnd.github.raw"]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    return out if raw else json.loads(out)


def gh_paged(path):
    out = subprocess.run(
        ["gh", "api", "--paginate", "--slurp", path], check=True, capture_output=True, text=True
    ).stdout
    return [item for page in json.loads(out) for item in page]


def version_tuple(tag):
    nums = re.findall(r"\d+", tag)
    return tuple(int(n) for n in nums) if nums else None


def newer_tags(repo, baseline):
    base = version_tuple(baseline)
    tags = []
    for t in gh_paged(f"repos/{repo}/tags?per_page=100"):
        v = version_tuple(t["name"])
        if v and v > base:
            tags.append(t["name"])
    return sorted(tags, key=version_tuple)


def ahead_commits(repo, base):
    """Commits on BRANCH not in base (oldest first), the count, and the merge base's date."""
    cmp = gh(f"repos/{repo}/compare/{base}...{BRANCH}")
    if cmp["ahead_by"] > len(cmp["commits"]):
        sys.exit(f"{repo}: {cmp['ahead_by']} commits ahead, more than the compare API lists")
    return cmp["commits"], cmp["ahead_by"], cmp["merge_base_commit"]["commit"]["committer"]["date"]


def commits_touching(repo, paths, candidates, since):
    """The candidate commits that touch any of paths."""
    want = {c["sha"] for c in candidates}
    hit = set()
    if not want:
        return []
    for p in paths:
        for c in gh_paged(f"repos/{repo}/commits?sha={BRANCH}&path={p}&since={since}&per_page=100"):
            if c["sha"] in want:
                hit.add(c["sha"])
    return [c for c in candidates if c["sha"] in hit]


def relevant_files(repo, sha, paths):
    """Files in the commit under paths, leaving out version-only edits."""
    files = []
    for f in gh(f"repos/{repo}/commits/{sha}")["files"]:
        name = f["filename"]
        if not any(name == p or name.startswith(p + "/") for p in paths):
            continue
        changed = [
            line
            for line in (f.get("patch") or "").splitlines()
            if line[:1] in "+-" and not line.startswith(("+++", "---")) and line[1:].strip()
        ]
        if changed and all(VERSION_LINE.match(line) for line in changed):
            continue
        files.append(name)
    return files


def first_line(c):
    return c["commit"]["message"].splitlines()[0]


def biber_version_at(ref):
    src = gh(f"repos/{BIBER}/contents/lib/Biber/Config.pm?ref={ref}", raw=True)
    return re.search(r"our \$VERSION = '([^']+)'", src).group(1)


def biber_pin():
    out = subprocess.run(
        ["git", "ls-tree", "HEAD", "vendor/biber"], cwd=ROOT, check=True, capture_output=True, text=True
    ).stdout.split()
    return out[2]


def bibers_bcf_version():
    src = (ROOT / "crates/bibers-core/src/bcf/mod.rs").read_text()
    return re.search(r'pub const BCF_VERSION: &str = "([^"]+)"', src).group(1)


def collect(biber_base=None, biblatex_base=BIBLATEX_BASELINE):
    findings = []  # (key, markdown line)
    sections = []

    # biber
    pin = biber_base or biber_pin()
    pin_version = biber_version_at(pin)
    lines = []
    for tag in newer_tags(BIBER, pin_version):
        findings.append((f"biber-tag:{tag}", None))
        lines.append(f"- New release tag **{tag}** (https://github.com/{BIBER}/releases/tag/{tag})")
    commits, ahead, since = ahead_commits(BIBER, pin)
    touched = commits_touching(BIBER, BIBER_PATHS, commits, since)
    for c in touched:
        files = relevant_files(BIBER, c["sha"], BIBER_PATHS)
        if not files:
            continue
        findings.append((f"biber:{c['sha']}", None))
        shown = ", ".join(f"`{f}`" for f in files[:8]) + (f" and {len(files) - 8} more" if len(files) > 8 else "")
        lines.append(
            f"- [`{c['sha'][:7]}`]({c['html_url']}) {c['commit']['author']['date'][:10]} "
            f"{first_line(c)}: {shown}"
        )
    sections.append(
        f"### biber (pinned at {pin_version}, `{pin[:7]}`; `{BRANCH}` is {ahead} commits ahead)\n\n"
        + ("\n".join(lines) if lines else "Nothing to port.")
    )

    # biblatex
    lines = []
    for tag in newer_tags(BIBLATEX, biblatex_base):
        findings.append((f"biblatex-tag:{tag}", None))
        lines.append(f"- New release tag **{tag}** (https://github.com/{BIBLATEX}/releases/tag/{tag})")
    sty = gh(f"repos/{BIBLATEX}/contents/{BIBLATEX_STY}?ref={BRANCH}", raw=True)
    m = re.search(r"\\def\\blx@bcfversion\{([^}]+)\}", sty)
    ours = bibers_bcf_version()
    if not m:
        findings.append(("biblatex-bcf:unreadable", None))
        lines.append(f"- Could not find `\\blx@bcfversion` in `{BIBLATEX_STY}` on `{BRANCH}`; check by hand.")
    elif m.group(1) != ours:
        findings.append((f"biblatex-bcf:{m.group(1)}", None))
        lines.append(
            f"- **Control file version changed:** `{BRANCH}` writes `.bcf` version {m.group(1)}; "
            f"bibers accepts {ours}. A new biber (and bibers) release will be needed."
        )
    commits, ahead, since = ahead_commits(BIBLATEX, biblatex_base)
    for c in commits_touching(BIBLATEX, [BIBLATEX_DM], commits, since):
        findings.append((f"biblatex-dm:{c['sha']}", None))
        lines.append(
            f"- [`{c['sha'][:7]}`]({c['html_url']}) {c['commit']['author']['date'][:10]} "
            f"{first_line(c)}: default data model (`blx-dm.def`)"
        )
    sections.append(
        f"### biblatex (baseline {biblatex_base}, `.bcf` {ours}; `{BRANCH}` is {ahead} commits ahead)\n\n"
        + ("\n".join(lines) if lines else "Nothing to port.")
    )

    key = hashlib.sha256("\n".join(sorted(k for k, _ in findings)).encode()).hexdigest()[:16]
    return findings, sections, key


def report(sections, key):
    return (
        "The weekly upstream check (`tools/upstream/watch.py`) found changes that bibers may need to follow. "
        "Build and packaging changes are left out.\n\n"
        + "\n\n".join(sections)
        + "\n\nWhen the changes are ported, bump the `vendor/biber` pin or `BIBLATEX_BASELINE` "
        "in `tools/upstream/watch.py`, then close this issue."
        + f"\n\n<!-- {MARKER}: {key} -->\n"
    )


def post(body, key, repo):
    issues = json.loads(
        subprocess.run(
            ["gh", "issue", "list", "-R", repo, "--label", LABEL, "--state", "open",
             "--json", "number,body,comments", "--limit", "5"],
            check=True, capture_output=True, text=True,
        ).stdout
    )
    tag = f"<!-- {MARKER}: {key} -->"
    if issues:
        issue = issues[0]
        texts = [issue["body"]] + [c["body"] for c in issue["comments"]]
        if any(tag in t for t in texts):
            print(f"#{issue['number']} already has these findings ({key}).")
            return
        subprocess.run(["gh", "issue", "comment", str(issue["number"]), "-R", repo, "--body", body], check=True)
        print(f"Commented on #{issue['number']} ({key}).")
    else:
        subprocess.run(
            ["gh", "label", "create", LABEL, "-R", repo, "--force", "--color", "C5DEF5",
             "--description", "Upstream biber/biblatex changes to review"],
            check=True, capture_output=True,
        )
        subprocess.run(
            ["gh", "issue", "create", "-R", repo, "--title", TITLE, "--label", LABEL, "--body", body], check=True
        )
        print(f"Opened an issue ({key}).")


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--issue", metavar="OWNER/REPO", help="post findings to an issue in this repository")
    ap.add_argument("--biber-base", metavar="REF", help="compare biber from this ref instead of the submodule pin")
    ap.add_argument("--biblatex-base", metavar="REF", default=BIBLATEX_BASELINE, help="biblatex baseline tag or ref")
    args = ap.parse_args()
    findings, sections, key = collect(args.biber_base, args.biblatex_base)
    body = report(sections, key)
    if not args.issue:
        print(body)
        return
    if not findings:
        print("Nothing to report.")
        return
    post(body, key, args.issue)


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Watch biber and biblatex upstream for changes bibers may need to port.

bibers follows biber *releases*: an upstream change is ported (and a quirk
default flipped, see DC-051) only once a release contains it. So there are two
reports.

Releases (actionable):
  * new biber release tags after the version vendor/biber is pinned to;
  * new biblatex release tags after BIBLATEX_BASELINE.

Preview (information only, nothing to port yet):
  * commits on biber's `dev` branch since the pin that touch processing code
    (lib/, data/, bin/) or tests (t/). Edits that only change $VERSION or
    $BETA_VERSION are ignored, as are build and packaging changes (dist/ etc.).
    Commits titled "Fix for #N", where N is an issue we filed (docs/upstream/),
    are marked with the BB- numbers it tracks;
  * a change of the control file version (\\blx@bcfversion) on biblatex's
    `dev` from bibers' BCF_VERSION, and commits on `dev` since the baseline
    that touch the default data model (blx-dm.def).

With --issue, each report goes to its own open GitHub issue in this repository.
The release report goes to the issue labelled `upstream-watch`: created if none
is open, otherwise a new comment, but only when the findings differ from what
that issue already holds (each report carries a hash of its findings). The
preview report goes to the issue labelled `upstream-preview`, whose body is
rewritten in place when the findings change; editing a body sends no
notifications, so dev activity stays quiet. Without --issue both reports are
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
TITLE = "New biber/biblatex release to port"
MARKER = "upstream-watch-key"

PREVIEW_LABEL = "upstream-preview"
PREVIEW_TITLE = "Upstream dev activity in biber/biblatex (preview, nothing to port yet)"
PREVIEW_MARKER = "upstream-preview-key"

# "Posted as https://github.com/plk/biber/issues/520 (...). Tracks BB-149 ..."
FILED = re.compile(r"plk/biber/issues/(\d+)\b.*?Tracks ([^(\n]*?)\s*\(docs/biber-bugs\.md\)")
FIX_FOR = re.compile(r"^Fix(?:es|ed)? for #(\d+)\b", re.IGNORECASE)

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


def filed_issues():
    """Our upstream issues on biber (docs/upstream/), as {number: "BB-149"}."""
    filed = {}
    for f in sorted((ROOT / "docs/upstream").glob("*.md")):
        m = FILED.search(f.read_text())
        if m:
            filed[int(m.group(1))] = m.group(2).strip()
    return filed


def key_of(findings):
    return hashlib.sha256("\n".join(sorted(findings)).encode()).hexdigest()[:16]


def collect(biber_base=None, biblatex_base=BIBLATEX_BASELINE):
    """Release findings and sections, then preview findings and sections."""
    rel, rel_sections = [], []
    pre, pre_sections = [], []

    # biber
    pin = biber_base or biber_pin()
    pin_version = biber_version_at(pin)
    lines = []
    for tag in newer_tags(BIBER, pin_version):
        rel.append(f"biber-tag:{tag}")
        lines.append(f"- New release tag **{tag}** (https://github.com/{BIBER}/releases/tag/{tag})")
    rel_sections.append(
        f"### biber (pinned at {pin_version}, `{pin[:7]}`)\n\n" + ("\n".join(lines) if lines else "No new release.")
    )

    filed = filed_issues()
    commits, ahead, since = ahead_commits(BIBER, pin)
    touched = commits_touching(BIBER, BIBER_PATHS, commits, since)
    lines = []
    for c in touched:
        files = relevant_files(BIBER, c["sha"], BIBER_PATHS)
        if not files:
            continue
        pre.append(f"biber:{c['sha']}")
        shown = ", ".join(f"`{f}`" for f in files[:8]) + (f" and {len(files) - 8} more" if len(files) > 8 else "")
        m = FIX_FOR.match(first_line(c))
        ours = f" **(our issue, {filed[int(m.group(1))]})**" if m and int(m.group(1)) in filed else ""
        lines.append(
            f"- [`{c['sha'][:7]}`]({c['html_url']}) {c['commit']['author']['date'][:10]} "
            f"{first_line(c)}{ours}: {shown}"
        )
    pre_sections.append(
        f"### biber `{BRANCH}` ({ahead} commits ahead of {pin_version})\n\n"
        + ("\n".join(lines) if lines else "No processing-code changes.")
    )

    # biblatex
    lines = []
    for tag in newer_tags(BIBLATEX, biblatex_base):
        rel.append(f"biblatex-tag:{tag}")
        lines.append(f"- New release tag **{tag}** (https://github.com/{BIBLATEX}/releases/tag/{tag})")
    rel_sections.append(
        f"### biblatex (baseline {biblatex_base})\n\n" + ("\n".join(lines) if lines else "No new release.")
    )

    lines = []
    sty = gh(f"repos/{BIBLATEX}/contents/{BIBLATEX_STY}?ref={BRANCH}", raw=True)
    m = re.search(r"\\def\\blx@bcfversion\{([^}]+)\}", sty)
    ours = bibers_bcf_version()
    if not m:
        pre.append("biblatex-bcf:unreadable")
        lines.append(f"- Could not find `\\blx@bcfversion` in `{BIBLATEX_STY}` on `{BRANCH}`; check by hand.")
    elif m.group(1) != ours:
        pre.append(f"biblatex-bcf:{m.group(1)}")
        lines.append(
            f"- **Control file version changed:** `{BRANCH}` writes `.bcf` version {m.group(1)}; "
            f"bibers accepts {ours}. The next biblatex release will need a new biber (and bibers) release, "
            "so start on it before biblatex ships."
        )
    commits, ahead, since = ahead_commits(BIBLATEX, biblatex_base)
    for c in commits_touching(BIBLATEX, [BIBLATEX_DM], commits, since):
        pre.append(f"biblatex-dm:{c['sha']}")
        lines.append(
            f"- [`{c['sha'][:7]}`]({c['html_url']}) {c['commit']['author']['date'][:10]} "
            f"{first_line(c)}: default data model (`blx-dm.def`)"
        )
    pre_sections.append(
        f"### biblatex `{BRANCH}` ({ahead} commits ahead of {biblatex_base}; bibers reads `.bcf` {ours})\n\n"
        + ("\n".join(lines) if lines else "No control file or data model changes.")
    )

    return (rel, rel_sections, key_of(rel)), (pre, pre_sections, key_of(pre))


def release_report(sections, key):
    return (
        "The weekly upstream check (`tools/upstream/watch.py`) found a new upstream release for bibers to follow.\n\n"
        + "\n\n".join(sections)
        + "\n\nPort the release (changes already reviewed in the `upstream-preview` issue are a head start; "
        "upstream fixes flip quirk defaults now, see DC-051), then bump the `vendor/biber` pin or "
        "`BIBLATEX_BASELINE` in `tools/upstream/watch.py` and close this issue."
        + f"\n\n<!-- {MARKER}: {key} -->\n"
    )


def preview_report(sections, key):
    return (
        "Unreleased changes on biber's and biblatex's `dev` branches, from the weekly upstream check "
        "(`tools/upstream/watch.py`). **Nothing here is ported until a release contains it**; this is for "
        "checking upstream fixes of our issues and planning ahead. Build and packaging changes are left out. "
        "This body is rewritten in place when the findings change, so it sends no notifications.\n\n"
        + "\n\n".join(sections)
        + f"\n\n<!-- {PREVIEW_MARKER}: {key} -->\n"
    )


def open_issue(repo, label):
    issues = json.loads(
        subprocess.run(
            ["gh", "issue", "list", "-R", repo, "--label", label, "--state", "open",
             "--json", "number,body,comments", "--limit", "5"],
            check=True, capture_output=True, text=True,
        ).stdout
    )
    return issues[0] if issues else None


def create_issue(repo, label, description, title, body):
    subprocess.run(
        ["gh", "label", "create", label, "-R", repo, "--force", "--color", "C5DEF5", "--description", description],
        check=True, capture_output=True,
    )
    subprocess.run(["gh", "issue", "create", "-R", repo, "--title", title, "--label", label, "--body", body], check=True)


def post_release(body, key, repo):
    """New comment on the open release issue (or a new issue), only when the findings change."""
    issue = open_issue(repo, LABEL)
    tag = f"<!-- {MARKER}: {key} -->"
    if issue:
        texts = [issue["body"]] + [c["body"] for c in issue["comments"]]
        if any(tag in t for t in texts):
            print(f"#{issue['number']} already has these release findings ({key}).")
            return
        subprocess.run(["gh", "issue", "comment", str(issue["number"]), "-R", repo, "--body", body], check=True)
        print(f"Commented on #{issue['number']} ({key}).")
    else:
        create_issue(repo, LABEL, "New upstream biber/biblatex release to port", TITLE, body)
        print(f"Opened a release issue ({key}).")


def post_preview(body, key, repo):
    """Rewrite the open preview issue's body (no notification), or open one."""
    issue = open_issue(repo, PREVIEW_LABEL)
    if issue:
        if f"<!-- {PREVIEW_MARKER}: {key} -->" in issue["body"]:
            print(f"#{issue['number']} already has these preview findings ({key}).")
            return
        subprocess.run(["gh", "issue", "edit", str(issue["number"]), "-R", repo, "--body", body], check=True)
        print(f"Updated #{issue['number']} ({key}).")
    else:
        create_issue(repo, PREVIEW_LABEL, "Unreleased upstream biber/biblatex changes (information only)",
                     PREVIEW_TITLE, body)
        print(f"Opened a preview issue ({key}).")


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--issue", metavar="OWNER/REPO", help="post findings to an issue in this repository")
    ap.add_argument("--biber-base", metavar="REF", help="compare biber from this ref instead of the submodule pin")
    ap.add_argument("--biblatex-base", metavar="REF", default=BIBLATEX_BASELINE, help="biblatex baseline tag or ref")
    args = ap.parse_args()
    (rel, rel_sections, rel_key), (pre, pre_sections, pre_key) = collect(args.biber_base, args.biblatex_base)
    rel_body = release_report(rel_sections, rel_key)
    pre_body = preview_report(pre_sections, pre_key)
    if not args.issue:
        print("## Releases\n\n" + (rel_body if rel else "No new release.\n"))
        print("## Preview\n\n" + (pre_body if pre else "No dev activity to preview.\n"))
        return
    if rel:
        post_release(rel_body, rel_key, args.issue)
    else:
        print("No new release.")
    if pre:
        post_preview(pre_body, pre_key, args.issue)
    else:
        print("No dev activity to preview.")


if __name__ == "__main__":
    sys.exit(main())

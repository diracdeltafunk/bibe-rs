#!/usr/bin/env python3
"""Write tests/corpus/LICENSES.md: where the corpus's third-party files come
from and under which licences, from TeX Live's package database.

    tools/corpus/corpus_licenses.py [--tlpdb PATH] [--check]

For every case it finds the files not written for bibe-rs (TeX Live
documentation sources and their .bib files, biblatex's examples, biber's test
data) and the TeX Live package each comes from, with that package's
catalogue licence. Files copied from TeX Live are compared byte for byte with
the installed originals: they must be unmodified (LPPL §6). Generated control
files (.bcf, written by biblatex and the style when compiling) are attributed
to biblatex and the style's package. `--check` only verifies (exit 1 if the
file is stale or a copied file differs from TeX Live's).
"""

import argparse
import glob
import hashlib
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "tests/corpus"
OUT = CORPUS / "LICENSES.md"

# TeX Live catalogue licence names → SPDX ids (where one exists).
SPDX = {
    "lppl": "LPPL-1.0+", "lppl1": "LPPL-1.0", "lppl1.2": "LPPL-1.2", "lppl1.3": "LPPL-1.3c",
    "lppl1.3a": "LPPL-1.3a", "lppl1.3c": "LPPL-1.3c", "gpl": "GPL", "gpl2": "GPL-2.0",
    "gpl2+": "GPL-2.0-or-later", "gpl3": "GPL-3.0", "gpl3+": "GPL-3.0-or-later",
    "mit": "MIT", "pd": "public domain", "cc-by-sa-4": "CC-BY-SA-4.0",
    "cc-by-4": "CC-BY-4.0", "cc0": "CC0-1.0", "fdl": "GFDL", "ofl": "OFL-1.1",
    "bsd": "BSD", "apache2": "Apache-2.0", "artistic2": "Artistic-2.0",
    "other-free": "other free", "collection": "collection",
}


def spdx(lic):
    return " AND ".join(SPDX.get(x, x) for x in lic.split())


def load_tlpdb(path):
    """(file path relative to the TeX Live root → package, package → licence)."""
    owner, licence = {}, {}
    name = None
    for line in open(path, encoding="utf-8", errors="replace"):
        if line.startswith("name "):
            name = line[5:].strip()
            # architecture-specific binaries: name.arch, not interesting here
        elif line.startswith("catalogue-license ") and name:
            licence[name] = line.split(None, 1)[1].strip()
        elif line.startswith(" ") and name and "." not in name:
            f = line.strip().split()[0]
            if f.startswith("RELOC/"):
                f = "texmf-dist/" + f[len("RELOC/"):]
            owner[f] = name
    return owner, licence


def kpsewhich(name):
    try:
        return subprocess.run(
            ["kpsewhich", name], capture_output=True, text=True, check=False
        ).stdout.strip()
    except FileNotFoundError:
        return ""


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def main():
    ap = argparse.ArgumentParser()
    tl = sorted(glob.glob("/usr/local/texlive/*/tlpkg/texlive.tlpdb"))
    ap.add_argument("--tlpdb", default=tl[-1] if tl else None)
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    if not args.tlpdb:
        sys.exit("no TeX Live package database found (--tlpdb)")
    texroot = Path(args.tlpdb).parents[1]
    owner, licence = load_tlpdb(args.tlpdb)
    by_name = defaultdict(list)  # file name → TeX Live paths
    for f in owner:
        by_name[os.path.basename(f)].append(f)

    rows = defaultdict(lambda: {"cases": set(), "files": set()})  # (origin, pkg) → …
    problems = []
    biber_lic = "Artistic-2.0"

    for case_toml in sorted(CORPUS.glob("*/*/case.toml")):
        case = case_toml.parent
        cid = f"{case.parent.name}/{case.name}"
        text = case_toml.read_text(encoding="utf-8")
        src = (re.search(r'^source = "(.*)"', text, re.M) or [None, ""])[1]
        tags = re.findall(r'"style:([^"]+)"', text)
        files = [p for p in case.iterdir() if p.is_file() and p.name != "case.toml"]
        if src.startswith("vendor/biber/"):
            r = rows[("biber test data (vendor/biber/t)", "biber")]
            r["cases"].add(cid)
            continue
        tex_src = re.match(r"(?:\$TEXMFDIST|texmf-dist)/(\S+)", src)
        if tex_src:
            path = "texmf-dist/" + tex_src.group(1)
            pkg = owner.get(path, "?")
            for p in files:
                # .bib/.tex copied from the package; the .bcf is generated
                if p.suffix in (".bcf",):
                    continue
                cands = [f for f in by_name.get(p.name, []) if owner[f] == pkg] or by_name.get(
                    p.name, []
                )
                fpkg = owner[cands[0]] if cands else "?"
                same = any(
                    (texroot / f).exists() and sha(texroot / f) == sha(p) for f in cands
                )
                name = p.name
                if p.name == "biblatex-examples.bib":
                    fpkg = "biblatex"
                elif not same:
                    # Written by the document itself when compiled (LaTeX's
                    # `filecontents`): the document's package, unmodified.
                    doc = texroot / path
                    doctext = doc.read_text(errors="replace") if doc.exists() else ""
                    if "filecontents" in doctext and (p.stem in doctext or p.stem == Path(path).stem):
                        fpkg = pkg
                        name = f"{p.name}*"
                    else:
                        problems.append(f"{cid}/{p.name}: not an unmodified copy of a TeX Live file")
                rows[("TeX Live documentation", fpkg)]["cases"].add(cid)
                rows[("TeX Live documentation", fpkg)]["files"].add(name)
            rows[("TeX Live documentation", pkg)]["cases"].add(cid)
            continue
        if case.parent.name == "styles" and tags:
            for t in tags:
                bbx = kpsewhich(f"{t}.bbx")
                rel = os.path.relpath(bbx, texroot) if bbx else ""
                pkg = owner.get(rel, "?")
                rows[("generated .bcf (style data)", pkg)]["cases"].add(cid)
            continue
    # biblatex-examples.bib in shared/
    rows[("TeX Live documentation", "biblatex")]["files"].add("shared/biblatex-examples.bib")
    exp = texroot / "texmf-dist/doc/latex/biblatex/examples/biblatex-examples.bib"
    if exp.exists() and sha(exp) != sha(CORPUS / "shared/biblatex-examples.bib"):
        problems.append("shared/biblatex-examples.bib differs from TeX Live's")

    out = [
        "# Licences of the corpus's third-party files",
        "",
        "Generated by `tools/corpus/corpus_licenses.py` from TeX Live's package database"
        f" ({Path(args.tlpdb).parents[1].name}); do not edit by hand.",
        "",
        "Most corpus inputs were written for bibe-rs (Artistic-2.0, like the rest of the"
        " repository). The files below come from elsewhere and keep their own licences."
        " They are test data only: none of them is compiled into bibers. Files copied from"
        " TeX Live are unmodified copies (checked byte for byte against TeX Live). Generated"
        " `.bcf` control files and expected outputs contain data from biblatex, the style"
        " packages and biber, under those packages' licences.",
        "",
        "Files marked * are written by the package's document itself when it is compiled"
        " (LaTeX `filecontents`), so they are not separate files in TeX Live.",
        "",
        "| Origin | TeX Live package | Licence | Cases | Copied files |",
        "|---|---|---|--:|---|",
    ]
    for (origin, pkg), r in sorted(rows.items()):
        lic = biber_lic if pkg == "biber" else spdx(licence.get(pkg, "?"))
        files = ", ".join(f"`{f}`" for f in sorted(r["files"])) or "—"
        out.append(f"| {origin} | {pkg} | {lic} | {len(r['cases'])} | {files} |")
    out.append("")
    out.append(
        "biblatex (and so every generated `.bcf`) is LPPL-1.3c. biber and its test data"
        " (`vendor/biber/t/`) are Artistic-2.0. The expected outputs (`expected/`) were"
        " produced by biber 2.22 from these inputs."
    )
    text = "\n".join(out) + "\n"
    for p in problems:
        print("problem:", p, file=sys.stderr)
    if args.check:
        stale = not OUT.exists() or OUT.read_text(encoding="utf-8") != text
        if stale:
            print("tests/corpus/LICENSES.md is stale", file=sys.stderr)
        sys.exit(1 if stale or problems else 0)
    OUT.write_text(text, encoding="utf-8")
    print(f"wrote {OUT.relative_to(ROOT)}: {len(rows)} rows, {len(problems)} problems")
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Generate tests/corpus/biblatex-examples/: one case per biblatex example
document ($TEXMFDIST/doc/latex/biblatex/examples/*.tex).

Each document gets one `<engine> -draftmode -interaction=nonstopmode` run in
a temporary directory to produce its .bcf. Documents that load xeCJK are run
with xelatex, those that load fontspec with lualatex, the rest with pdflatex. The case keeps the .tex (for
reference/regeneration), the .bcf, and any data files the .bcf references
apart from biblatex-examples.bib, which comes from tests/corpus/shared/.

Usage: python3 tools/corpus/gen_examples.py [NAME-REGEX]
Then:  cargo xtask golden biblatex-examples/
"""

import concurrent.futures as cf
import re
import shutil
import sys
import tempfile
from pathlib import Path

from corpuslib import (CORPUS, bcf_datasources, ensure_shared_examples_bib, latex_error, log,
                       run_latex, texmfdist, write_case)

SUITE = CORPUS / "biblatex-examples"

# Examples left out of the corpus. (None needs the BibTeX backend: all 52
# ship a *-biber.pdf.)
SKIP: dict[str, str] = {
    "95-customlists": "its sourcemap keys new entries with $MAPUNIQ, a time-based random "
                      "string, so every biber run gives a different .bbl (BB-113, DC-018)",
}


def engine_for(tex: str) -> str:
    if re.search(r"\\usepackage(\[[^]]*\])?\{xeCJK\}", tex):
        return "xelatex"
    return "lualatex" if re.search(r"\\usepackage(\[[^]]*\])?\{fontspec\}", tex) else "pdflatex"


def build(texfile: Path) -> str:
    exdir = texfile.parent
    stem = texfile.stem
    tex = texfile.read_text(encoding="utf-8")
    engine = engine_for(tex)
    with tempfile.TemporaryDirectory(prefix="bibers-ex-") as td:
        work = Path(td)
        shutil.copyfile(texfile, work / texfile.name)
        # Companion files (datamodels, .bib) sharing the document's stem.
        for f in exdir.glob(f"{stem}.*"):
            if f.suffix in (".dbx", ".bib"):
                shutil.copyfile(f, work / f.name)
        r = run_latex(work, texfile.name, engine)
        bcf = work / f"{stem}.bcf"
        if not bcf.exists():
            return f"FAILED ({engine}): no .bcf; {latex_error(work, stem)}"
        files = {texfile.name: texfile, bcf.name: bcf.read_bytes()}
        shared = []
        for typ, datatype, path in bcf_datasources(bcf):
            if path == "biblatex-examples.bib":
                if "biblatex-examples.bib" not in shared:
                    shared.append(path)
                continue
            src = work / path
            if not src.exists():
                return f"FAILED: data source {path} not found"
            files[path] = src.read_bytes()
        for f in exdir.glob(f"{stem}.dbx"):
            files[f.name] = f  # for reference only; biber reads the datamodel from the .bcf
        tags = ["suite:biblatex-examples", f"engine:{engine}"]
        m = re.search(r"\\usepackage\[([^]]*)\]\{biblatex\}", tex)
        if m:
            sm = re.search(r"(?:^|,)\s*style=([\w-]+)", m.group(1))
            if sm:
                tags.append(f"style:{sm.group(1)}")
        warn = "" if r.returncode == 0 else f" [LaTeX exit {r.returncode}: {latex_error(work, stem)}]"
        spec = dict(
            description=f"biblatex example {texfile.name}",
            source=f"$TEXMFDIST/doc/latex/biblatex/examples/{texfile.name} "
                   f"({engine} -draftmode, tools/corpus/gen_examples.py)",
            args=["--noconf", bcf.name],
            outputs=[f"{stem}.bbl"],
            shared=shared,
            tags=tags,
        )
        write_case(SUITE / stem, spec, files)
        return f"ok ({engine}){warn}"


def main() -> None:
    pat = re.compile(sys.argv[1] if len(sys.argv) > 1 else ".")
    ensure_shared_examples_bib()
    exdir = texmfdist() / "doc" / "latex" / "biblatex" / "examples"
    texs = [t for t in sorted(exdir.glob("*.tex")) if pat.search(t.stem)]
    todo = []
    for t in texs:
        if t.stem in SKIP:
            log(f"  skip {t.stem}: {SKIP[t.stem]}")
        else:
            todo.append(t)
    with cf.ThreadPoolExecutor(max_workers=8) as ex:
        for t, res in zip(todo, ex.map(build, todo)):
            log(f"{t.stem:40} {res}")


if __name__ == "__main__":
    main()

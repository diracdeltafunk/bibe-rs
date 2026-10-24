#!/usr/bin/env python3
"""serial == parallel (DC-068) over every .bib file in TeX Live.

Each file is processed like tools/corpus/texlive_diff.py does (the .bcf of
tests/corpus/styles/authoryear: authoryear, \\nocite{*}), by bibers with
`--threads=1` and with `--threads=8` and every parallel loop forced on
(`BIBERS_PAR_MIN_ITEMS=1`). The `.bbl`, the `.blg` (without its `[ms]`
column and date line), stdout, stderr and the exit code must be identical.
TeX Live's .bib files are messy (syntax errors, odd macros, duplicates),
which exercises the fallbacks of the parallel code.

Usage (from a scratch directory; build first with
`cargo build --release -p bibers`):
  find $(kpsewhich -var-value TEXMFDIST) -name '*.bib' -size -2M > list.txt
  python3 /path/to/tools/corpus/texlive_threads.py
Prints the differing files (kept under work/) and a summary.
"""
import concurrent.futures as cf
import os
import pathlib
import re
import shutil
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
bcf = open(REPO / "tests/corpus/styles/authoryear/authoryear.bcf", encoding="utf-8").read()
OUT = pathlib.Path(os.environ.get("OUT", ".")).resolve()
bibers = os.environ.get("BIBERS", str(REPO / "target/release/bibers"))
files = [l.strip() for l in open(os.environ.get("LIST", "list.txt")) if l.strip()]


def strip_blg(blg):
    out = []
    for line in blg.splitlines():
        line = re.sub(r"^\[\d+\] ", "", line)
        if " - === " not in line:
            out.append(line)
    return "\n".join(out)


def run(i, f):
    d = OUT / "work" / str(i)
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    shutil.copy(f, d / "data.bib")
    (d / "t.bcf").write_text(bcf.replace(">biblatex-examples.bib<", ">data.bib<"), encoding="utf-8")
    res = []
    for n, force in ((1, False), (8, True)):
        env = {"LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8", "PATH": "/usr/bin:/bin"}
        if force:
            env["BIBERS_PAR_MIN_ITEMS"] = "1"
        try:
            r = subprocess.run(
                [bibers, "--noconf", f"--threads={n}", "t.bcf"],
                cwd=d, capture_output=True, timeout=600, env=env,
            )
            code, so, se = r.returncode, r.stdout, r.stderr
        except subprocess.TimeoutExpired:
            code, so, se = "timeout", b"", b""
        bbl = (d / "t.bbl").read_bytes() if (d / "t.bbl").exists() else None
        blg = (d / "t.blg").read_text("utf-8", "replace") if (d / "t.blg").exists() else ""
        res.append((code, bbl, strip_blg(blg), strip_blg(so.decode("utf-8", "replace")), se))
        for x in ("t.bbl", "t.blg"):
            if (d / x).exists():
                (d / x).rename(d / f"{n}-{x}")
    same = res[0] == res[1]
    if same:
        shutil.rmtree(d)
    return f, same, res[0][0]


diff = 0
with cf.ThreadPoolExecutor(4) as ex:
    for f, same, code in ex.map(lambda a: run(*a), enumerate(files)):
        if not same:
            diff += 1
            print(f"DIFF {f}", flush=True)
print(f"{len(files)} files, {diff} differ")
sys.exit(1 if diff else 0)

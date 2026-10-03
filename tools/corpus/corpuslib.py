"""Shared helpers for the corpus generator scripts in tools/corpus/.

A generator builds case directories under tests/corpus/<suite>/<case>/:
the inputs plus a case.toml (format: tests/corpus/README.md). It never
writes expected/; run `cargo xtask golden <filter>` afterwards.

Re-running a generator rewrites each case's inputs and case.toml but keeps
an existing expected/ directory, so goldens only change when the oracle
output changes.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORPUS = REPO / "tests" / "corpus"
SHARED = CORPUS / "shared"
TDATA = REPO / "vendor" / "biber" / "t" / "tdata"

# Files that must never end up in a case directory.
JUNK_SUFFIXES = {".aux", ".log", ".pdf", ".run.xml", ".bbl", ".blg", ".out", ".toc",
                 ".idx", ".ind", ".ilg", ".fls", ".fdb_latexmk", ".synctex.gz"}


def texmfdist() -> Path:
    out = subprocess.run(["kpsewhich", "-var-value", "TEXMFDIST"], check=True,
                         capture_output=True, text=True).stdout.strip()
    return Path(out)


def biblatex_examples_bib() -> Path:
    out = subprocess.run(["kpsewhich", "biblatex-examples.bib"], check=True,
                         capture_output=True, text=True).stdout.strip()
    return Path(out)


def ensure_shared_examples_bib() -> None:
    """Copy biblatex-examples.bib into tests/corpus/shared/ (if changed)."""
    SHARED.mkdir(parents=True, exist_ok=True)
    src = biblatex_examples_bib()
    dst = SHARED / "biblatex-examples.bib"
    if not dst.exists() or dst.read_bytes() != src.read_bytes():
        shutil.copyfile(src, dst)


def _toml_value(v) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, str):
        s = v.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
        return f'"{s}"'
    if isinstance(v, (list, tuple)):
        return "[" + ", ".join(_toml_value(x) for x in v) + "]"
    if isinstance(v, dict):
        return "{ " + ", ".join(f"{k} = {_toml_value(x)}" for k, x in v.items()) + " }"
    raise TypeError(f"unsupported TOML value {v!r}")


# Key order in the emitted case.toml.
_ORDER = ["description", "source", "args", "outputs", "shared", "log", "env",
          "compare", "tags", "allow", "timeout_secs"]


def case_toml(spec: dict) -> str:
    unknown = set(spec) - set(_ORDER)
    if unknown:
        raise KeyError(f"unknown case.toml keys {unknown}")
    lines = []
    for k in _ORDER:
        if k in spec and spec[k] not in (None, [], {}):
            lines.append(f"{k} = {_toml_value(spec[k])}")
    return "\n".join(lines) + "\n"


def write_case(case_dir: Path, spec: dict, files: dict[str, Path | bytes | str],
               notes: str | None = None, allow_junk: tuple[str, ...] = ()) -> None:
    """(Re)create the inputs of a case.

    files maps the file name inside the case dir to a source path, or to
    literal content (bytes/str). Stale input files (anything but case.toml
    and expected/) are removed first.
    """
    case_dir.mkdir(parents=True, exist_ok=True)
    for p in case_dir.iterdir():
        if p.name in ("expected",):
            continue
        if p.is_dir():
            shutil.rmtree(p)
        else:
            p.unlink()
    for name, src in files.items():
        if "/" in name:
            raise ValueError(f"case inputs must be flat: {name}")
        if name not in allow_junk and any(name.endswith(s) for s in JUNK_SUFFIXES):
            raise ValueError(f"refusing to put {name} into a case")
        dst = case_dir / name
        if isinstance(src, Path):
            shutil.copyfile(src, dst)
        elif isinstance(src, bytes):
            dst.write_bytes(src)
        else:
            dst.write_text(src, encoding="utf-8")
    if notes:
        spec = dict(spec)
        spec["description"] = spec.get("description", "") + notes
    (case_dir / "case.toml").write_text(case_toml(spec), encoding="utf-8")


def bcf_datasources(bcf: Path) -> list[tuple[str, str, str]]:
    """(type, datatype, path) of every <bcf:datasource> in a .bcf."""
    text = bcf.read_text(encoding="utf-8")
    out = []
    for m in re.finditer(r"<bcf:datasource([^>]*)>([^<]*)</bcf:datasource>", text):
        attrs = dict(re.findall(r'(\w+)="([^"]*)"', m.group(1)))
        out.append((attrs.get("type", "file"), attrs.get("datatype", "bibtex"), m.group(2).strip()))
    return out


def run_latex(workdir: Path, texname: str, engine: str = "pdflatex",
              timeout: int = 300) -> subprocess.CompletedProcess:
    """One draft-mode LaTeX run (enough to write the .bcf)."""
    env = dict(os.environ)
    env.setdefault("SOURCE_DATE_EPOCH", "0")
    return subprocess.run(
        [engine, "-draftmode", "-interaction=nonstopmode", texname],
        cwd=workdir, capture_output=True, text=True, errors="replace",
        env=env, timeout=timeout)


def latex_error(workdir: Path, stem: str) -> str:
    log = workdir / f"{stem}.log"
    if not log.exists():
        return "no log"
    lines = log.read_text(errors="replace").splitlines()
    errs = [l for l in lines if l.startswith("!")]
    return "; ".join(errs[:3]) or "no '!' line in log"


def log(msg: str) -> None:
    print(msg, file=sys.stderr, flush=True)

#!/usr/bin/env python3
"""Synthetic scaled inputs, for the corpus and for benchmarks.

Shapes (N = number of entries), from the perf studies in docs/research/03 §5
and 05 §6 (scratch/semantics/perf/gen.py, scratch/output/make_synth.py):

  base      distinct authors, authoryear, no uniqueness           (linear)
  un        one shared surname, distinct givens, uniquename=full  (linear)
  ul        shared 2-name prefix in 5-name lists, uniquelist      (linear, 2x base)
  ul3       pairs of identical 4-name lists, uniquelist           (quadratic in biber, BB-109)
  alpha     distinct authors, alphabetic style                    (slightly super-linear)
  cr0       N children -> one crossref parent                     (linear)
  cr        N children -> parent -> grandparent                   (cubic in biber, BB-110)
  examples  biblatex-examples.bib duplicated with renamed keys    (realistic data, ~102 entries/copy)
  mixed     a blend of the above (the committed corpus case)

Everything is deterministic: the same N gives the same .bib.

Usage:
  python3 tools/corpus/gen_scale.py corpus
      (Re)write the committed case tests/corpus/scale/mixed-1000/.
  python3 tools/corpus/gen_scale.py bench [--out DIR] [SHAPE:N ...]
      Write benchmark inputs (default DIR target/bench-corpus/, gitignored;
      default set: every shape at 1000/2000/4000, cr at 500/1000, examples at
      x10/x40/x180). Each <DIR>/<shape>-<N>/ holds doc.tex, doc.bcf, the .bib
      and a case.toml in corpus format. Run one with e.g.
        (cd target/bench-corpus/ul3-2000 && time biber --noconf doc.bcf)
      or point $BIBERS at a binary and use tools/corpus/bench.sh.
"""

import re
import sys
import tempfile
from pathlib import Path

from corpuslib import (CORPUS, REPO, biblatex_examples_bib, latex_error, log, run_latex,
                       write_case)

OPTIONS = {
    "base": "style=authoryear,uniquename=false,uniquelist=false",
    "un": "style=authoryear,uniquename=full,uniquelist=false",
    "ul": "style=authoryear,uniquelist=true,uniquename=init,maxcitenames=2,mincitenames=1",
    "ul3": "style=authoryear,uniquelist=true,uniquename=false,maxcitenames=2,mincitenames=1",
    "alpha": "style=alphabetic",
    "cr0": "style=authoryear",
    "cr": "style=authoryear",
    "examples": "style=authoryear",
    "mixed": "style=authoryear,uniquelist=true,uniquename=init,maxcitenames=2,mincitenames=1,mincrossrefs=3",
}


def nm(i: int) -> str:
    """A distinct surname for every i (base-26 letters)."""
    s = ""
    while True:
        s += chr(ord("a") + i % 26)
        i //= 26
        if i == 0:
            break
    return "Q" + s


def entries(kind: str, n: int, prefix: str = "k") -> list[str]:
    bib = []
    for i in range(n):
        k = f"{prefix}{i}"
        if kind == "ul":
            bib.append(f"@book{{{k}, author={{Smith, Ann and Jones, Bob and {nm(i)}, Cy and "
                       f"{nm(i + 1)}, Di and {nm(i + 2)}, Ed}}, title={{T{i}}}, year={{2000}}}}")
        elif kind == "un":
            bib.append(f"@book{{{k}, author={{Smith, {nm(i)}}}, title={{T{i}}}, year={{2000}}}}")
        elif kind == "alpha":
            bib.append(f"@book{{{k}, author={{{nm(i)}, Ann}}, title={{T{i}}}, year={{2000}}}}")
        elif kind == "base":
            bib.append(f"@book{{{k}, author={{{nm(i)}, Ann}}, title={{T{i}}}, year={{{1900 + i % 100}}}}}")
        elif kind == "ul3":
            j = i // 2
            bib.append(f"@book{{{k}, author={{{nm(j)}, Ann and Tee, Bob and Ute, Cy and Vee, Di}}, "
                       f"title={{T{i}}}, year={{2000}}}}")
        elif kind == "cr0":
            if i == 0:
                bib.append(f"@mvbook{{{prefix}P, title={{Parent}}, year={{1990}}}}")
            bib.append(f"@book{{{k}, crossref={{{prefix}P}}, author={{{nm(i)}, Ann}}, title={{T{i}}}}}")
        elif kind == "cr":
            if i == 0:
                bib.append(f"@mvbook{{{prefix}G, title={{Grand}}, year={{1990}}}}\n"
                           f"@mvbook{{{prefix}P, crossref={{{prefix}G}}, title={{Parent}}}}")
            bib.append(f"@book{{{k}, crossref={{{prefix}P}}, author={{{nm(i)}, Ann}}, title={{T{i}}}}}")
        else:
            raise ValueError(kind)
    return bib


def examples_bib(copies: int) -> str:
    """biblatex-examples.bib duplicated `copies` times, keys renamed (and
    references to them: crossref/xref/xdata/related/entryset) suffixed -c<i>."""
    src = biblatex_examples_bib().read_text(encoding="utf-8")
    parts = re.split(r"(?m)^(?=@)", src)
    strings = [p for p in parts if re.match(r"@string", p, re.I)]
    ents = [p for p in parts if p.startswith("@") and not re.match(r"@(string|comment|preamble)", p, re.I)]
    keys = {re.match(r"@\w+\s*\{\s*([^,\s]+)\s*,", e).group(1) for e in ents}

    def ren(e: str, i: int) -> str:
        e = re.sub(r"^(@\w+\s*\{\s*)([^,\s]+)(\s*,)", lambda m: m.group(1) + m.group(2) + f"-c{i}" + m.group(3),
                   e, count=1)

        def fix(m):
            vals = [v.strip() for v in m.group(3).split(",")]
            vals = [v + f"-c{i}" if v in keys else v for v in vals]
            return m.group(1) + m.group(2) + ",".join(vals) + m.group(4)
        return re.sub(r"(?im)^(\s*(?:crossref|xref|xdata|related|entryset)\s*=\s*)(\{)([^}]*)(\})", fix, e)

    out = ["".join(strings)]
    for i in range(copies):
        out += [ren(e, i) for e in ents]
    return "".join(out)


def mixed_bib(n: int) -> str:
    """A blend: 30% base, 20% un, 20% ul3 pairs, 20% ul, 10% crossref children
    split over parents with a shared grandparent, plus dates/pages variety."""
    parts = []
    nb, nu, n3, nl = int(n * .3), int(n * .2), int(n * .2), int(n * .2)
    nc = n - nb - nu - n3 - nl
    parts += entries("base", nb, "b")
    parts += entries("un", nu, "u")
    parts += entries("ul3", n3, "p")
    parts += entries("ul", nl, "l")
    # crossref children: groups of 10 under parents that share a grandparent
    parts.append("@mvbook{G, title={Grand Series}, year={1980}, publisher={Pub}}")
    for g in range((nc + 9) // 10):
        parts.append(f"@book{{vol{g}, crossref={{G}}, title={{Volume {g}}}, date={{{1981 + g % 30}-0{1 + g % 9}}}}}")
    for i in range(nc):
        parts.append(f"@inbook{{c{i}, crossref={{vol{i // 10}}}, author={{{nm(i)}, Bo and {nm(i + 7)}, Al}}, "
                     f"title={{Chapter {i}}}, pages={{{10 * i + 1}--{10 * i + 9}}}}}")
    return "\n".join(parts) + "\n"


def bib_for(kind: str, n: int) -> str:
    if kind == "examples":
        return examples_bib(n)
    if kind == "mixed":
        return mixed_bib(n)
    return "\n".join(entries(kind, n)) + "\n"


def tex_for(kind: str, bibname: str) -> str:
    return (
        "% Generated by tools/corpus/gen_scale.py; do not edit.\n"
        "\\documentclass{article}\n"
        f"\\usepackage[backend=biber,{OPTIONS[kind]}]{{biblatex}}\n"
        f"\\addbibresource{{{bibname}}}\n"
        "\\begin{document}\n\\nocite{*}\n\\printbibliography\n\\end{document}\n"
    )


_bcf_cache: dict[str, bytes] = {}


def bcf_for(tex: str) -> bytes:
    if tex not in _bcf_cache:
        with tempfile.TemporaryDirectory(prefix="bibers-scale-") as td:
            work = Path(td)
            (work / "doc.tex").write_text(tex, encoding="utf-8")
            run_latex(work, "doc.tex")
            bcf = work / "doc.bcf"
            if not bcf.exists():
                sys.exit(f"LaTeX failed: {latex_error(work, 'doc')}")
            _bcf_cache[tex] = bcf.read_bytes()
    return _bcf_cache[tex]


def make(dest: Path, kind: str, n: int, tags: list[str]) -> None:
    bibname = f"{kind}-{n}.bib"
    tex = tex_for(kind, bibname)
    label = f"x{n} copies of biblatex-examples.bib" if kind == "examples" else f"{n} entries"
    spec = dict(
        description=f"synthetic '{kind}' shape, {label} (\\nocite{{*}}, {OPTIONS[kind]})",
        source=f"generated by tools/corpus/gen_scale.py ({kind}, {n})",
        args=["--noconf", "doc.bcf"],
        outputs=["doc.bbl"],
        tags=tags,
        timeout_secs=600,
    )
    write_case(dest, spec, {"doc.tex": tex, "doc.bcf": bcf_for(tex), bibname: bib_for(kind, n)})


DEFAULT_BENCH = ([f"{k}:{n}" for k in ("base", "un", "ul", "ul3", "alpha", "cr0") for n in (1000, 2000, 4000)]
                 + ["cr:500", "cr:1000", "examples:10", "examples:40", "examples:180", "mixed:4000"])


def main() -> None:
    args = sys.argv[1:]
    if not args or args[0] not in ("corpus", "bench"):
        sys.exit(__doc__)
    if args[0] == "corpus":
        make(CORPUS / "scale" / "mixed-1000", "mixed", 1000, ["suite:scale", "scale:mixed"])
        log("wrote tests/corpus/scale/mixed-1000")
        return
    out = REPO / "target" / "bench-corpus"
    rest = args[1:]
    if rest[:1] == ["--out"]:
        out = Path(rest[1]).resolve()
        rest = rest[2:]
    for spec in rest or DEFAULT_BENCH:
        kind, n = spec.split(":")
        make(out / f"{kind}-{n}", kind, int(n), ["suite:bench", f"scale:{kind}"])
        log(f"wrote {out / f'{kind}-{n}'}")


if __name__ == "__main__":
    main()

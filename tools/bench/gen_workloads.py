#!/usr/bin/env python3
"""Generate the benchmark workloads of docs/PLAN.md §1 under target/bench/.

Each workload is a directory with a `.bcf` and its `.bib`:

- `examples-xN`: biblatex-examples.bib replicated N times (keys and the keys
  in crossref/xref/xdata/entryset/related/ids suffixed `-i`), `\\nocite{*}`,
  authoryear. `examples-x1` is the plain 100-entry document.
- `uniquelist-N`: N entries sharing one 5-name author list (biber's
  uniquelist bookkeeping is quadratic in this shape, BB-109).
- `crossref-N`: N children crossref one parent, which crossrefs a grandparent
  (biber's crossref cascade bookkeeping is cubic, BB-110).

All use the authoryear `.bcf` of tests/corpus/styles/authoryear (uniquelist
and uniquename on, maxcitenames=3). Deterministic; safe to re-run.
"""

import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "tests/corpus"
OUT = Path(os.environ.get("BIBERS_BENCH_DIR", ROOT / "target/bench"))
BCF = (CORPUS / "styles/authoryear/authoryear.bcf").read_text(encoding="utf-8")
EXAMPLES = (CORPUS / "shared/biblatex-examples.bib").read_text(encoding="utf-8")

REF_FIELDS = ("crossref", "xref", "xdata", "entryset", "related", "ids")


def bcf_for(bib):
    return BCF.replace(">biblatex-examples.bib<", f">{bib}<")


def write(name, bib_text):
    d = OUT / name
    d.mkdir(parents=True, exist_ok=True)
    (d / "doc.bib").write_text(bib_text, encoding="utf-8")
    (d / "doc.bcf").write_text(bcf_for("doc.bib"), encoding="utf-8")
    print(f"{name}: {bib_text.count(chr(10) + '@') + bib_text.startswith('@')} @-items")


def split_examples():
    """(@string items, other items) of biblatex-examples.bib, as text."""
    items = re.split(r"(?m)^(?=@)", EXAMPLES)
    head, rest = items[0], items[1:]
    strings = [i for i in rest if re.match(r"@string\b", i, re.I)]
    entries = [i for i in rest if not re.match(r"@(string|preamble|comment)\b", i, re.I)]
    return head + "".join(strings), entries


def suffix_entry(text, i):
    """Suffix the entry key and the keys it refers to with `-i`."""
    text = re.sub(r"^(@\w+\{)([^,\s]+)", lambda m: f"{m[1]}{m[2]}-{i}", text, count=1)

    def refs(m):
        keys = ",".join(f"{k.strip()}-{i}" for k in m[3].split(",") if k.strip())
        return f"{m[1]}{m[2]}{keys}}}"

    return re.sub(
        r"(?mi)^(\s*(?:%s)\s*=\s*)(\{)([^}]*)\}" % "|".join(REF_FIELDS), refs, text
    )


def examples(n):
    head, entries = split_examples()
    if n == 1:
        return EXAMPLES
    return head + "".join(suffix_entry(e, i) for i in range(n) for e in entries)


def uniquelist(n):
    out = []
    for i in range(n):
        out.append(
            f"@article{{u{i},\n  author = {{Smith, John and Doe, Jane and Roe, Rick"
            f" and Poe, Edgar and Moe, Mary}},\n  title = {{Title {i}}},\n"
            f"  journaltitle = {{Journal}},\n  date = {{{1900 + i % 120}}},\n}}\n\n"
        )
    return "".join(out)


def crossref(n):
    out = [
        "@mvproceedings{grand,\n  editor = {Ed, Grand},\n  title = {Grand Title},\n"
        "  date = {2000},\n  location = {Here},\n  publisher = {Pub},\n}\n\n",
        "@proceedings{parent,\n  crossref = {grand},\n  title = {Parent Title},\n"
        "  volume = {1},\n}\n\n",
    ]
    for i in range(n):
        out.append(
            f"@inproceedings{{c{i},\n  crossref = {{parent}},\n  author = {{Author{i}, A.}},\n"
            f"  title = {{Paper {i}}},\n  pages = {{{i}--{i + 9}}},\n}}\n\n"
        )
    return "".join(out)


WORKLOADS = {
    "examples-x1": lambda: examples(1),
    "examples-x10": lambda: examples(10),
    "examples-x46": lambda: examples(46),
    "examples-x184": lambda: examples(184),
    "uniquelist-4000": lambda: uniquelist(4000),
    "crossref-2000": lambda: crossref(2000),
}


def main():
    names = sys.argv[1:] or list(WORKLOADS)
    for name in names:
        write(name, WORKLOADS[name]())


if __name__ == "__main__":
    main()

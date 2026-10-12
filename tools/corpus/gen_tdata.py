#!/usr/bin/env python3
"""Generate tests/corpus/tdata/: one case per vendor/biber/t/tdata/*.bcf.

Each case runs `biber --noconf <x>.bcf` with only the data files that the
.bcf's <bcf:datasource> elements reference. Remote data sources are skipped
(DC-032, and the corpus must not need the network).

Usage: python3 tools/corpus/gen_tdata.py [NAME-REGEX]
Then:  cargo xtask golden tdata/
"""

import re
import sys
from pathlib import Path

from corpuslib import CORPUS, TDATA, bcf_datasources, log, write_case

SUITE = CORPUS / "tdata"

# Which biber test script exercises each .bcf (for the description).
def users_of(bcf_name: str) -> list[str]:
    tdir = TDATA.parent
    return sorted(p.name for p in tdir.glob("*.t") if bcf_name in p.read_text(errors="replace"))


def xdata_noloop_bib() -> str:
    """xdata.bib without the circular-XDATA fixtures (whose fatal error names a
    random cycle, BB-112), so the rest of the XDATA machinery gets a
    deterministic .bbl."""
    s = (TDATA / "xdata.bib").read_text(encoding="utf-8")
    s = re.sub(r"% Loop detection test.*?(?=% Granular XDATA tests)", "", s, flags=re.S)
    s = re.sub(r"% Granular loop detection XDATA test.*?(?=% annotation inheritance)", "", s, flags=re.S)
    assert "loop" not in s.lower(), "xdata.bib changed upstream; update xdata_noloop_bib()"
    return s


def xdata_noloop_bltxml() -> str:
    s = (TDATA / "xdata.bltxml").read_text(encoding="utf-8")
    s = re.sub(r'\s*<bltx:entry id="bltxlxd[^"]*".*?</bltx:entry>', "", s, flags=re.S)
    assert "loop" not in s.lower(), "xdata.bltxml changed upstream; update xdata_noloop_bltxml()"
    return s


# .bcf files deliberately not turned into a plain case.
SKIP = {
    "xdata": "dies with a nondeterministic 'Circular XDATA inheritance' error (BB-112); "
             "no harness compare mode makes it stable, so it has no case; "
             "tdata/xdata-noloop covers the rest",
}

# Extra cases beyond the plain `--noconf x.bcf` run.
EXTRA = {
    # xdata.bcf minus the circular-reference fixtures (see xdata_noloop_bib).
    "xdata-noloop": dict(
        bcf="xdata.bcf",
        replace={"xdata.bib": xdata_noloop_bib, "xdata.bltxml": xdata_noloop_bltxml},
        args=["--noconf", "xdata.bcf"],
        description="xdata.bcf with the circular-XDATA entries removed from xdata.bib/.bltxml "
                    "(the full fixture dies with a nondeterministic error, BB-112)",
        tags=["feature:xdata", "feature:biblatexml", "deferred:DC-031"],
    ),
    # t/encoding.t sets data source and output encodings through the API;
    # these are the command-line equivalents.
    "encoding1-latin9": dict(
        bcf="encoding1.bcf", args=["--noconf", "--input-encoding=latin9", "encoding1.bcf"],
        description="latin9 .bib -> UTF-8 .bbl (t/encoding.t)", tags=["feature:encoding"]),
    "encoding3-latin1-cp1252": dict(
        bcf="encoding3.bcf",
        args=["--noconf", "--input-encoding=latin1", "--output-encoding=cp1252", "encoding3.bcf"],
        description="latin1 .bib -> CP1252 .bbl (t/encoding.t)",
        tags=["feature:encoding"]),
    "encoding4-latin2-latin3": dict(
        bcf="encoding4.bcf",
        args=["--noconf", "--input-encoding=latin2", "--output-encoding=latin3", "encoding4.bcf"],
        description="latin2 .bib -> latin3 .bbl (t/encoding.t)",
        tags=["feature:encoding"]),
    "encoding4-latin2-latin1": dict(
        bcf="encoding4.bcf",
        args=["--noconf", "--input-encoding=latin2", "--output-encoding=latin1", "encoding4.bcf"],
        description="latin2 .bib -> latin1 .bbl: unencodable characters (t/encoding.t)",
        tags=["feature:encoding"]),
    "encoding5-latin1": dict(
        bcf="encoding5.bcf", args=["--noconf", "--output-encoding=latin1", "encoding5.bcf"],
        description="UTF-8 .bib -> latin1 .bbl (t/encoding.t)",
        tags=["feature:encoding"]),
    "encoding6-safechars": dict(
        bcf="encoding6.bcf", args=["--noconf", "--output-safechars", "encoding6.bcf"],
        description="UTF-8 .bib -> UTF-8 .bbl with --output-safechars (t/encoding.t)",
        tags=["feature:encoding"]),
    "encoding6-safecharsset-full": dict(
        bcf="encoding6.bcf",
        args=["--noconf", "--output-safechars", "--output-safecharsset=full", "encoding6.bcf"],
        description="UTF-8 .bib -> UTF-8 .bbl, --output-safecharsset=full (t/encoding.t)",
        tags=["feature:encoding"]),
    # t/configfile.t: general.bcf with the test biber.conf and --mincrossrefs=7.
    "general-configfile": dict(
        bcf="general.bcf",
        extra_files=["biber-test.conf"],
        args=["--configfile=biber-test.conf", "--mincrossrefs=7", "general.bcf"],
        description="general.bcf with t/tdata/biber-test.conf (as in t/configfile.t)",
        tags=["feature:configfile"],
    ),
}


def make(name: str, bcf: Path, args: list[str], extra_files: list[str], description: str,
         tags: list[str], replace: dict | None = None) -> str | None:
    files = {bcf.name: bcf}
    tags = ["suite:tdata"] + tags
    for typ, datatype, path in bcf_datasources(bcf):
        if re.match(r"^[a-z]+://", path):
            return f"remote data source {path} (DC-032; corpus must work offline)"
        if "/" in path:
            return f"data source in a subdirectory: {path}"
        src = TDATA / path
        if not src.exists():
            return f"data source {path} not found in t/tdata"
        files[path] = src
        if datatype == "biblatexml":
            tags.append("feature:biblatexml")
            tags.append("deferred:DC-031")
    for f in extra_files:
        files[f] = TDATA / f
    for f, gen in (replace or {}).items():
        files[f] = gen()
    tags = sorted(set(tags), key=tags.index)
    stem = bcf.stem
    used = users_of(bcf.name)
    spec = dict(
        description=description + (f" (used by t/{', t/'.join(used)})" if used and name == stem else ""),
        source=f"vendor/biber/t/tdata/{bcf.name}",
        args=args,
        outputs=[f"{stem}.bbl"],
        tags=tags,
    )
    write_case(SUITE / name, spec, files)
    return None


def main() -> None:
    pat = re.compile(sys.argv[1] if len(sys.argv) > 1 else ".")
    skipped = {}
    n = 0
    for bcf in sorted(TDATA.glob("*.bcf")):
        name = bcf.stem
        if not pat.search(name):
            continue
        if name in SKIP:
            skipped[name] = SKIP[name]
            continue
        why = make(name, bcf, ["--noconf", bcf.name], [], f"biber t/tdata/{bcf.name}", [])
        if why:
            skipped[name] = why
        else:
            n += 1
    for name, e in EXTRA.items():
        if not pat.search(name):
            continue
        why = make(name, TDATA / e["bcf"], e["args"], e.get("extra_files", []), e["description"],
                   e["tags"], e.get("replace"))
        if why:
            skipped[name] = why
        else:
            n += 1
    log(f"tdata: wrote {n} cases")
    for k, v in skipped.items():
        log(f"  skipped {k}: {v}")


if __name__ == "__main__":
    main()

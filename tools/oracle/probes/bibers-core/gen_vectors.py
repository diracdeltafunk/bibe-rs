#!/usr/bin/env python3
"""Generate the bibers-core/bibers differential vectors from oracle O3.

    python3 tools/oracle/probes/bibers-core/gen_vectors.py config
    python3 tools/oracle/probes/bibers-core/gen_vectors.py getopt

config: for every corpus case with a .bcf (tool-mode cases excluded), runs
probe.pl in `config` mode with the case's argv and writes
crates/bibers-core/tests/data/bcf/cases.json plus a content-addressed pool
(pool/<sha>.json) for large state parts shared between cases (data models,
option registries, ...).

getopt: runs bin/biber's option parsing over a matrix of argv's and writes
crates/bibers/tests/data/getopt.jsonl.
"""
import hashlib
import json
import os
import subprocess
import sys
import tomllib

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../.."))
PROBE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "run-probe.sh")
POOL_MIN = 512


def run_probe(mode, jobs):
    inp = "".join(json.dumps(j) + "\n" for j in jobs)
    out = subprocess.run([PROBE, mode], input=inp.encode(), capture_output=True, check=True).stdout
    return [json.loads(l) for l in out.decode().splitlines() if l.strip()]


def config():
    corpus = os.path.join(ROOT, "tests/corpus")
    jobs = []
    for dirpath, _, files in sorted(os.walk(corpus)):
        if "case.toml" not in files:
            continue
        case = tomllib.load(open(os.path.join(dirpath, "case.toml"), "rb"))
        args = case.get("args", [])
        if "--tool" in args or not any(f.endswith(".bcf") for f in files):
            continue
        rel = os.path.relpath(dirpath, corpus)
        jobs.append({"id": rel, "dir": dirpath, "args": args})
    results = run_probe("config", jobs)
    outdir = os.path.join(ROOT, "crates/bibers-core/tests/data/bcf")
    pooldir = os.path.join(outdir, "pool")
    os.makedirs(pooldir, exist_ok=True)
    for f in os.listdir(pooldir):
        os.remove(os.path.join(pooldir, f))
    cases = {}
    for job, res in zip(jobs, results):
        assert job["id"] == res["id"]
        entry = {"args": job["args"], "exit": res.get("exit"), "log": res.get("log", []),
                 "opts": res.get("opts"), "rest": res.get("rest")}
        if "die" in res:
            entry["die"] = res["die"]
        state = res.get("state")
        if state is not None:
            st = {}
            for k, v in state.items():
                if k == "dm":
                    # one pooled object per DM sub-part
                    st[k] = {sk: pool(pooldir, sv) for sk, sv in v.items()}
                else:
                    st[k] = pool(pooldir, v)
            entry["state"] = st
        cases[job["id"]] = entry
    with open(os.path.join(outdir, "cases.json"), "w") as f:
        json.dump(cases, f, indent=0, sort_keys=True, ensure_ascii=False)
        f.write("\n")
    print(f"{len(cases)} cases, {len(os.listdir(pooldir))} pooled objects")


def pool(pooldir, v):
    s = json.dumps(v, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
    if len(s) < POOL_MIN:
        return v
    h = hashlib.sha256(s.encode()).hexdigest()[:20]
    p = os.path.join(pooldir, h + ".json")
    if not os.path.exists(p):
        with open(p, "w") as f:
            f.write(s + "\n")
    return {"$pool": h}


GETOPT_MATRIX = [
    [], ["x"], ["x.bcf"], ["--noconf", "x.bcf"], ["x.bcf", "--noconf"], ["-noconf", "x"],
    ["--help"], ["-h"], ["-?"], ["--version"], ["-v"], ["--vers"], ["--cache"], ["--tool-config"],
    ["--out", "x"], ["--output", "x"], ["--output-f=bbl", "x"], ["--output-fi=o.bbl", "x"],
    ["--output-fo", "bibtex", "x"], ["--output-format=dot", "x"], ["--output-format=foo", "x"],
    ["--output-format=bblxml", "x"], ["--output-format=biblatexml", "x"],
    ["--tool", "--output-format=bbl", "a.bib"], ["--tool", "a.bib"], ["--tool", "a.bltxml"],
    ["--tool", "--input-format=foo", "a"], ["--input-format=bibtex", "x"],
    ["--input-encoding=latin1", "x"], ["--input-enc", "latin1", "x"], ["-e", "UTF-8", "x"],
    ["--input-encodinge=x", "y"], ["-u", "-U", "x"], ["--bibencoding=latin1", "--bblencoding", "ascii", "x"],
    ["--bblsafechars", "--bblsafecharsset=full", "x"], ["--outfile=o.bbl", "--outformat=bbl", "x"],
    ["--tool-align", "--tool-fieldcase=title", "--tool-indent=4t", "--tool-resolve", "x"],
    ["--tool-indent=0", "x"], ["--output-indent=x", "x"], ["--output-indent", "3t", "x"],
    ["--output-fieldcase=Upper", "x"], ["--output-fieldcase=foo", "x"], ["--isbn10", "--isbn13", "x"],
    ["--dot-include=section,field", "x"], ["--output-format=dot", "--dot-include=section,FIELD", "--dot-include", "xref", "x"],
    ["--output-format=dot", "--dot-include=bogus", "x"], ["--output-format=dot", "--dot-include", "x"],
    ["-q", "x"], ["-q", "-q", "x"], ["--quiet", "--quiet", "--quiet", "x"], ["-qq", "x"],
    ["-w", "x"], ["-w", "100", "x"], ["--wraplines=50", "x"], ["--wraplines=abc", "x"], ["-w", "-q", "x"],
    ["-c", "level=3", "-c", "variable=shifted", "x"], ["--collate-options=level", "x"], ["-c", "x"],
    ["--collate_options", "a=b=c", "x"], ["-m", "3", "x"], ["--mincrossrefs=0", "x"],
    ["-O", "-", "x"], ["--output-file", "-", "x"], ["-l", "de_DE", "x"], ["-D", "-T", "-V", "-W", "x"],
    ["--debug", "--trace", "x"], ["--debug=1", "x"], ["--sortcase=true", "--sortupper", "false", "x"],
    ["--output-resolve", "x"], ["--output-resolve-sets", "x"], ["--output-macro-fields=a", "x"],
    ["--fastsort", "-f", "x"], ["--logfile=foo.blg", "x"], ["--nolog", "--onlylog", "x"],
    ["--", "--noconf"], ["--noconf", "--", "-x"], ["-", "x"], ["+noconf", "x"], ["+", "x"],
    ["--=x", "y"], ["---noconf", "x"], ["--NOCONF", "x"], ["--Noconf", "x"], ["-T", "x"],
    ["--t", "x"], ["--to", "x"], ["--too", "x"], ["--tool", "x"], ["--tool-n", "x"],
    ["--no", "x"], ["--noc", "x"], ["--nos", "x"], ["--nod", "x"], ["--val", "x"], ["--validate-d", "x"],
    ["--ssl-noverify-host", "x"], ["--ssl", "x"], ["--x", "x"], ["--xdata", "x"], ["--xdatam=a", "x"],
    ["--isbn-normalise", "x"], ["--isbn", "x"], ["--input-directory", "d", "--output-directory=o", "x"],
    ["--configfile", "c.conf", "x"], ["-g", "c.conf", "x"], ["--convert-control", "x"], ["--cache=1", "x"],
    ["--noconf=1", "x"], ["--logfile", "x"], ["--logfile"], ["--output-file="], ["--output-file=", "x"],
    ["--annotation-marker", "-an", "x"], ["--namesep", "--noconf", "x"], ["--xsvsep=\\s*\\|\\s*", "x"],
    ["--validate-control", "--validate-config", "--validate-bltxml", "--validate-bblxml", "x"],
    ["--no-default-datamodel", "--no-bltxml-schema", "--no-bblxml-schema", "x"],
    ["--noremove-tmp-dir", "--show-tmp-dir", "x"], ["--strip-comments", "--noskipduplicates", "x"],
    ["--output-xname", "--noxname", "--output-xnamesep=:", "x"], ["--others-string", "andere", "x"],
    ["--output-field-replace=a:b,c:d", "x"], ["--output-legacy-dates", "x"],
    ["a", "b", "c"], ["--tool", "a.bib", "b.bib"],
]


def getopt():
    results = run_probe("getopt", GETOPT_MATRIX)
    out = os.path.join(ROOT, "crates/bibers/tests/data/getopt.jsonl")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w") as f:
        for r in results:
            f.write(json.dumps(r, sort_keys=True, ensure_ascii=False) + "\n")
    print(f"{len(results)} argv vectors")


if __name__ == "__main__":
    {"config": config, "getopt": getopt}[sys.argv[1]]()

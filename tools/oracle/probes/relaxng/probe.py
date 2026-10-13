#!/usr/bin/env python3
"""Compare libxml2 and crates/relaxng on small documents.

    python3 probe.py SCHEMA.rng DOCS.txt [WORKDIR]

DOCS.txt has one document per line. Prints "libxml2 relaxng document" per
line, flagging disagreements with '!'. Needs the O3 perl (vendor/perl) and
`cargo build --release -p relaxng --example rngcheck`.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))))


def main():
    schema, docs = sys.argv[1], sys.argv[2]
    work = sys.argv[3] if len(sys.argv) > 3 else os.path.join(ROOT, "target", "rng-probe")
    os.makedirs(work, exist_ok=True)
    lines = [l.rstrip("\n") for l in open(docs, encoding="utf-8") if l.strip()]
    files = []
    for i, l in enumerate(lines):
        p = os.path.join(work, f"d{i:04d}.xml")
        with open(p, "w", encoding="utf-8") as fh:
            fh.write(l)
        files.append(p)
    inp = "\n".join(files) + "\n"
    lib = subprocess.run([os.path.join(ROOT, "vendor/perl/bin/perl"),
                          os.path.join(ROOT, "tools/oracle/probes/relaxng/verdicts.pl"), schema, "-"],
                         input=inp, capture_output=True, text=True).stdout.split("\n")
    rs = subprocess.run([os.path.join(ROOT, "target/release/examples/rngcheck"), schema, "-"],
                        input=inp, capture_output=True, text=True).stdout.split("\n")
    for l, a, b in zip(lines, lib, rs):
        va, vb = a.split("\t")[-1], b.split("\t")[-1]
        print(f"{'!' if va != vb else ' '} {va:8} {vb:8} {l}")


if __name__ == "__main__":
    main()

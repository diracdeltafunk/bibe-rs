#!/usr/bin/env python3
"""Compare bibers' --convert-control renderer with libxslt (xsltproc, the
same libxslt 1.1.35/libxml2 2.9.13 as biber 2.22) on .bcf files.

    python3 convert_diff.py WORKDIR FILE.bcf...

Needs `cargo build --release -p bibers-core --example bcfhtml`. Prints the
files whose output differs and a count.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))))
XSL = os.path.join(ROOT, "vendor/biber/data/bcf.xsl")


def main():
    work = sys.argv[1]
    files = sys.argv[2:]
    if files == ["-"]:
        files = [l.strip() for l in sys.stdin if l.strip()]
    os.makedirs(work, exist_ok=True)
    args = []
    for i, f in enumerate(files):
        args += [f, os.path.join(work, f"{i:05d}.rs.html")]
    subprocess.run([os.path.join(ROOT, "target/release/examples/bcfhtml")] + args, check=True)
    bad = 0
    for i, f in enumerate(files):
        mp = os.path.join(work, f"{i:05d}.rs.html")
        if not os.path.exists(mp):
            print("SKIP (not well-formed)", f)
            continue
        ref = subprocess.run(["xsltproc", XSL, f], capture_output=True).stdout
        mine = open(mp, "rb").read()
        if ref != mine:
            bad += 1
            open(os.path.join(work, f"{i:05d}.ref.html"), "wb").write(ref)
            print("DIFF", f, os.path.join(work, f"{i:05d}"))
    print(f"{len(files)} files, {bad} differ")


if __name__ == "__main__":
    main()

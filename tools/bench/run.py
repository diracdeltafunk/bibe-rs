#!/usr/bin/env python3
"""Benchmark bibers on the workloads of tools/bench/gen_workloads.py.

    tools/bench/run.py [--golden] [--runs N] [--bin PATH] [--threads N] [--markdown] [WORKLOAD...]
    tools/bench/run.py --check-threads [--threads N] [WORKLOAD...]

For each workload in target/bench/: runs bibers N times (default 5) in a
fresh copy of the inputs and reports the best and median wall time and the
peak RSS; checks the `.bbl` against biber's (`expected.bbl`, made once with
`--golden`, which also records biber's time in `biber.txt`). A differing
`.bbl` is reported as FAIL: an optimisation must never change output.
`--markdown` also prints the results as a Markdown table (the README's speed
table). `--threads N` passes `--threads N` to bibers (DC-068).

`--check-threads` checks that the thread count changes nothing: each workload
runs at `--threads 1` and twice at `--threads N` (default 8) with every
parallel loop forced on (`BIBERS_PAR_MIN_ITEMS=1`); the `.bbl`, the `.blg`
(without its `[ms]` column) and the terminal output must be identical.

Build first: `cargo build --release -p bibers`. Workloads are read from
$BIBERS_BENCH_DIR (default target/bench).
"""

import argparse
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# Workloads and biber's goldens; BIBERS_BENCH_DIR lets several worktrees share
# one set (the goldens of the big workloads take minutes to make).
BENCH = Path(os.environ.get("BIBERS_BENCH_DIR", ROOT / "target/bench"))


def timed(cmd, cwd, env=None):
    """(wall seconds, peak RSS bytes or None, exit code)."""
    if platform.system() == "Darwin":
        full = ["/usr/bin/time", "-l"] + cmd
    elif shutil.which("/usr/bin/time"):
        full = ["/usr/bin/time", "-v"] + cmd
    else:
        full = cmd
    t0 = time.perf_counter()
    p = subprocess.run(full, cwd=cwd, env=env, capture_output=True, text=True)
    wall = time.perf_counter() - t0
    rss = None
    m = re.search(r"(\d+)\s+maximum resident set size", p.stderr)
    if m:
        rss = int(m[1])
    m = re.search(r"Maximum resident set size \(kbytes\): (\d+)", p.stderr)
    if m:
        rss = int(m[1]) * 1024
    return wall, rss, p.returncode


def fresh(src):
    d = Path(tempfile.mkdtemp(prefix="bibers-bench-"))
    for f in ("doc.bcf", "doc.bib"):
        shutil.copy(src / f, d / f)
    return d


def golden(name, src):
    env = dict(os.environ, PAR_GLOBAL_TMPDIR=str(ROOT / "target/par-cache"))
    d = fresh(src)
    wall, rss, code = timed(["biber", "-q", "--noconf", "doc.bcf"], d, env)
    if code not in (0, 2):
        sys.exit(f"{name}: biber exited {code}")
    shutil.copy(d / "doc.bbl", src / "expected.bbl")
    (src / "biber.txt").write_text(f"{wall:.3f} {rss or 0}\n")
    shutil.rmtree(d)
    print(f"{name}: biber {wall:.2f} s, {(rss or 0) / 2**20:.0f} MB")


def strip_ms(blg):
    """A `.blg` without the elapsed-time column and the `===` date line."""
    lines = []
    for line in blg.splitlines():
        if line.startswith("[") and "] " in line:
            line = line.split("] ", 1)[1]
        if " - === " in line:
            continue
        lines.append(line)
    return "\n".join(lines)


def check_threads(names, binary, threads):
    """--check-threads: the outputs at 1 and `threads` threads are identical."""
    failed = False
    for name in names:
        src = BENCH / name
        outs = []
        for n, force in ((1, False), (threads, True), (threads, True)):
            d = fresh(src)
            env = dict(os.environ)
            env.pop("BIBERS_THREADS", None)
            if force:
                env["BIBERS_PAR_MIN_ITEMS"] = "1"
            p = subprocess.run(
                [binary, "--noconf", f"--threads={n}", "doc.bcf"],
                cwd=d, env=env, capture_output=True,
            )
            blg = (d / "doc.blg").read_text(errors="replace")
            outs.append(
                (p.returncode, (d / "doc.bbl").read_bytes(), strip_ms(blg),
                 strip_ms(p.stdout.decode(errors="replace")), p.stderr)
            )
            shutil.rmtree(d)
        what = ["exit code", ".bbl", ".blg", "stdout", "stderr"]
        diffs = sorted({w for o in outs[1:] for w, a, b in zip(what, outs[0], o) if a != b})
        status = "ok" if not diffs else "FAIL (" + ", ".join(diffs) + " differ)"
        failed |= bool(diffs)
        print(f"{name:<18} 1 vs {threads} threads: {status}")
    sys.exit(1 if failed else 0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("workloads", nargs="*")
    ap.add_argument("--golden", action="store_true")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--bin", default=str(ROOT / "target/release/bibers"))
    ap.add_argument("--markdown", action="store_true", help="also print a Markdown table")
    ap.add_argument("--threads", type=int, help="bibers --threads (DC-068)")
    ap.add_argument(
        "--check-threads", action="store_true",
        help="check that outputs at 1 and N threads are identical",
    )
    args = ap.parse_args()
    names = args.workloads or sorted(p.name for p in BENCH.iterdir() if (p / "doc.bcf").exists())
    if not names:
        sys.exit("no workloads: run tools/bench/gen_workloads.py first")
    if args.check_threads:
        check_threads(names, args.bin, args.threads or 8)
    extra = [f"--threads={args.threads}"] if args.threads is not None else []
    print(f"{'workload':<18} {'best':>8} {'median':>8} {'RSS':>7}  {'biber':>8} {'speedup':>8}  output")
    failed = False
    rows = []
    for name in names:
        src = BENCH / name
        if args.golden:
            golden(name, src)
            continue
        walls, rss_max, status = [], 0, "ok"
        for _ in range(args.runs):
            d = fresh(src)
            wall, rss, code = timed([args.bin, "-q", "--noconf", *extra, "doc.bcf"], d)
            walls.append(wall)
            rss_max = max(rss_max, rss or 0)
            exp = src / "expected.bbl"
            if code not in (0, 2):
                status = f"FAIL (exit {code})"
            elif not exp.exists():
                status = "no golden"
            elif (d / "doc.bbl").read_bytes() != exp.read_bytes():
                status = "FAIL (differs)"
            shutil.rmtree(d)
        failed |= status.startswith("FAIL")
        bt = src / "biber.txt"
        biber = float(bt.read_text().split()[0]) if bt.exists() else None
        best = min(walls)
        print(
            f"{name:<18} {best * 1000:7.1f}ms {statistics.median(walls) * 1000:7.1f}ms"
            f" {rss_max / 2**20:5.0f}MB  "
            + (f"{biber:7.2f}s {biber / best:7.0f}×" if biber else f"{'':>8} {'':>8}")
            + f"  {status}"
        )
        rows.append((name, entries(src), biber, best, rss_max, status))
    if args.markdown and rows:
        print()
        print(markdown(rows))
    sys.exit(1 if failed else 0)


def entries(src):
    """Entries in biber's `.bbl` (the cited entries), if there is one."""
    exp = src / "expected.bbl"
    return exp.read_bytes().count(b"\\entry{") if exp.exists() else None


def secs(t):
    return f"{t * 1000:.0f} ms" if t < 1 else f"{t:.2f} s" if t < 10 else f"{t:.1f} s"


def markdown(rows):
    """The results as a Markdown table, with the machine and date below it."""
    out = [
        "| Workload | Entries | biber 2.22 | bibers | Speedup | bibers peak RSS |",
        "|---|--:|--:|--:|--:|--:|",
    ]
    for name, n, biber, best, rss, status in rows:
        out.append(
            f"| `{name}` | {n if n is not None else '—'} | {secs(biber) if biber else '—'}"
            f" | {secs(best)}{'' if status == 'ok' else f' ({status})'}"
            f" | {f'{biber / best:.0f}×' if biber else '—'} | {rss / 2**20:.0f} MB |"
        )
    out.append("")
    out.append(
        f"*{platform.system()} {platform.machine()}, {time.strftime('%Y-%m-%d')}; bibers: best of"
        " the runs; biber: one run (`tools/bench/run.py --markdown`).*"
    )
    return "\n".join(out)


if __name__ == "__main__":
    main()

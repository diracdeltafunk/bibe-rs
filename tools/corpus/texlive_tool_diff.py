#!/usr/bin/env python3
"""Differential run of bibers vs biber (O1) in tool mode (`--tool`) over .bib
files, with several `--output-*` option combinations (sibling of
texlive_diff.py).

Outputs (`data_bibertool.bib`), exit codes and WARN/ERROR messages are
compared exactly (messages with btparse temp-file names mapped, DC-042).
Use modest parallelism: many concurrent copies of the PAR-packed biber
occasionally fail spuriously (exit 13/102); such runs are retried.

Usage (from a scratch directory):
  find $(kpsewhich -var-value TEXMFDIST) -name '*.bib' -size -2M > list.txt
  python3 /path/to/tools/corpus/texlive_tool_diff.py
Environment: LIST (file list), OUT (output dir), COMBOS (comma list of combo
indices), ORACLE (default `biber`), JOBS (default 6).
Results: OUT/out.txt / results.json; differing runs are kept under OUT/work/.
"""
import subprocess, shutil, pathlib, re, os, json, concurrent.futures as cf
from collections import Counter

REPO = pathlib.Path(__file__).resolve().parents[2]
ORACLE = os.environ.get('ORACLE', 'biber')
OUT = pathlib.Path(os.environ.get('OUT', '.')).resolve()
OUT.mkdir(parents=True, exist_ok=True)
BIBERS = str(REPO / 'target/release/bibers')
FILES = [l.strip() for l in open(os.environ.get('LIST', 'list.txt')) if l.strip()]
JOBS = int(os.environ.get('JOBS', '6'))

COMBOS = [
    [],
    ['--output-align', '--output-indent=4', '--output-fieldcase=lower', '--output-resolve'],
    ['--output-xname', '--output-legacy-dates', '--output-field-order=title,author,dates,names',
     '--output-listsep=AND', '--output-namesep=AND', '--output-all-macrodefs', '--output-xnamesep=:'],
    ['--output-safechars', '--output-indent=2t', '--wraplines=60', '--strip-comments',
     '--output-fieldcase=title', '--output-align'],
    ['--output-encoding=latin1', '--output-no-macrodefs', '--output-annotation-marker=+ann',
     '--output-xdatamarker=xd', '--output-field-replace=journaltitle:journal,location:address',
     '--output-resolve-crossrefs'],
]
sel = os.environ.get('COMBOS')
IDX = [int(x) for x in sel.split(',')] if sel else list(range(len(COMBOS)))


def msgs(blg):
    out = []
    for l in blg.splitlines():
        m = re.match(r'^\[\d+\]\s+\S+>\s+(WARN|ERROR)\s+-\s+(.*)$', l)
        if m:
            t = re.sub(r'\S*/[0-9a-f]{32}_\d+\.utf8', 'data.bib', m.group(2))
            t = re.sub(r'\bat \S+\.p[ml] line \d+', 'at <PERLSRC>', t)
            out.append(m.group(1) + ' - ' + t)
    return sorted(out)


def run_one(d, cmd, args):
    for _ in range(3):
        for x in ('data_bibertool.bib', 'data.bib.blg'):
            (d / x).unlink(missing_ok=True)
        try:
            r = subprocess.run(cmd + ['--noconf', '--tool'] + args + ['data.bib'], cwd=d,
                               capture_output=True, timeout=600,
                               env={'LANG': 'en_US.UTF-8', 'LC_ALL': 'en_US.UTF-8',
                                    'PATH': '/Library/TeX/texbin:/usr/bin:/bin'})
            code = r.returncode
            err = r.stderr.decode('utf-8', 'replace')
        except subprocess.TimeoutExpired:
            code, err = 'timeout', ''
        if code not in (13, 102):
            break
    out = (d / 'data_bibertool.bib').read_bytes() if (d / 'data_bibertool.bib').exists() else None
    blg = (d / 'data.bib.blg').read_text('utf-8', 'replace') if (d / 'data.bib.blg').exists() else ''
    return code, out, msgs(blg), 'panicked' in err


def run(i, f, c):
    d = OUT / 'work' / f'{i}-{c}'
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    shutil.copy(f, d / 'data.bib')
    res = {}
    for who, cmd in (('biber', [ORACLE]), ('bibers', [BIBERS])):
        res[who] = run_one(d, cmd, COMBOS[c])
        for x in ('data_bibertool.bib', 'data.bib.blg'):
            if (d / x).exists():
                (d / x).rename(d / (who + '-' + x))
    b, r = res['biber'], res['bibers']
    crash_b = b[0] not in (0, 2)
    if r[3]:
        status = 'PANIC'
    elif crash_b:
        status = 'BIBER-CRASH'
    elif b[0] == r[0] and b[1] == r[1] and b[2] == r[2]:
        status = 'SAME'
    elif b[1] == r[1] and b[0] == r[0]:
        status = 'OUT-SAME-MSG-DIFF'
    else:
        status = 'DIFF'
    if status == 'SAME':
        shutil.rmtree(d)
    return f, c, status, b[0], r[0]


jobs = [(i, f, c) for i, f in enumerate(FILES) for c in IDX]
with cf.ThreadPoolExecutor(JOBS) as ex:
    results = list(ex.map(lambda a: run(*a), jobs))
summary = Counter(s for _, _, s, _, _ in results)
print(summary)
json.dump(results, open(OUT / 'results.json', 'w'), indent=0)
with open(OUT / 'out.txt', 'w') as o:
    o.write(f'{summary}\n')
    for f, c, s, cb, cr in results:
        if s != 'SAME':
            line = f'{s} combo={c} {cb} {cr} {f}'
            print(line)
            o.write(line + '\n')

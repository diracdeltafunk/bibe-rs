#!/usr/bin/env python3
"""Translation check: replay every ported case's configuration in a fresh
in-process O3 biber (replay.pl: same Biber->new / setoption / setblxoption /
set_sortingtemplatename calls as the .t file, but no earlier prepare()) and
compare the .bbl with the CLI golden. Identical output means the CLI case
(flags + .bcf edits) is an exact translation of the test's configuration.
Writes tools/port-tests/out/replay.json."""
import json, os, re, subprocess, sys
from concurrent.futures import ThreadPoolExecutor
sys.path.insert(0, os.path.dirname(__file__))
import check

HERE = os.path.dirname(os.path.abspath(__file__))
cases = json.load(open(os.path.join(check.OUT, 'cases.json')))
names = sys.argv[1:] or sorted(k for k, v in cases.items() if not v.get('output_file'))


def run(name):
    c = cases[name]
    c['name'] = name
    flags = ['--sortdebug'] if '--sortdebug' in c['args'] else []
    os.makedirs(os.path.join(check.OUT, 'replay'), exist_ok=True)
    of = os.path.join(check.OUT, 'replay', name + '.bbl')
    if os.path.exists(of):
        os.remove(of)
    subprocess.run([os.path.join(HERE, 'replay.sh'), name, of] + flags, capture_output=True)
    enc = check.out_encoding(c)
    rep = open(of, 'rb').read().decode(enc, 'replace') if os.path.exists(of) else ''
    g = os.path.join(check.CORPUS, name, 'expected', c['bcf'][:-4] + '.bbl')
    gold = open(g, 'rb').read().decode(enc, 'replace') if os.path.exists(g) else ''
    ren = c.get('renames', {})
    for old, new in ren.items():
        rep = rep.replace('\\datalist[entry]{%s}' % old, '\\datalist[entry]{%s}' % new)
    a, b = check.norm_bbl(rep), check.norm_bbl(gold)
    # $MAPUNIQ keys are random (basic-misc.t, biber-test.conf)
    a = re.sub(r'loopkey:(\w+):\w+', r'loopkey:\1:<MAPUNIQ>', a)
    b = re.sub(r'loopkey:(\w+):\w+', r'loopkey:\1:<MAPUNIQ>', b)
    if len(a.splitlines()) <= 20 < len(b.splitlines()):
        return name, 'replay-crash', 'in-process output() died (no list with the global sorting template)'
    if a == b:
        return name, 'identical', ''
    # list-order differences only?
    if sorted(a.splitlines()) == sorted(b.splitlines()):
        return name, 'reordered', ''
    return name, 'differs', check.diff_hint(a, b)


with ThreadPoolExecutor(8) as ex:
    res = dict(ex.map(lambda n: (lambda r: (r[0], r[1:]))(run(n)), names))
out = os.path.join(check.OUT, 'replay.json')
old = json.load(open(out)) if os.path.exists(out) else {}
old.update({k: {'status': v[0], 'hint': v[1]} for k, v in res.items()})
json.dump(old, open(out, 'w'), indent=1, sort_keys=True)
from collections import Counter
print(Counter(v[0] for v in res.values()))
for k, v in sorted(res.items()):
    if v[0] != 'identical':
        print(k, v[0], v[1][:200])

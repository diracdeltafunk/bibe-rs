#!/usr/bin/env python3
"""Show the diff between the in-process .bbl (BiberTrace dump) and the golden of a ported case.
Usage: dumpdiff.py <case> [prep]"""
import difflib, json, os, re, sys
sys.path.insert(0, os.path.dirname(__file__))
import check, traceconf as tc
cases = json.load(open(os.path.join(check.OUT, 'cases.json')))
name = sys.argv[1]
c = cases[name]; c['name'] = name
p = int(sys.argv[2]) if len(sys.argv) > 2 else c['preps'][0]
d = open(os.path.join(tc.TRACE, c['tfile'], 'bbl', '%d.bbl' % p), 'rb').read().decode(check.out_encoding(c), 'replace')
g = open(os.path.join(check.CORPUS, name, 'expected', c['bcf'][:-4] + '.bbl'), 'rb').read().decode(check.out_encoding(c), 'replace')
if '--sortdebug' not in c['args']:
    d = check.strip_sortdebug(d)
d, g = check.norm_bbl(d), check.norm_bbl(g)
sys.stdout.writelines(difflib.unified_diff(d.splitlines(True), g.splitlines(True), 'inprocess', 'golden', n=2))

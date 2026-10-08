#!/usr/bin/env python3
"""Differential run of bibers vs biber (O1) over every .bib file in TeX Live.

Each file is processed with the same .bcf (tests/corpus/styles/authoryear:
authoryear, \nocite{*}). Outputs are compared after the corpus .bbl
normalisation; messages after mapping biber's btparse temp-file names to the
data file name (DC-042). Use a modest parallelism: many concurrent copies of
the PAR-packed biber occasionally fail spuriously (exit 13/102).

Usage (from a scratch directory):
  find $(kpsewhich -var-value TEXMFDIST) -name '*.bib' -size -2M > list.txt
  python3 /path/to/tools/corpus/texlive_diff.py
Results: out.txt / results.json; differing cases are kept under work/.
"""
import subprocess, shutil, pathlib, re, sys, concurrent.futures as cf, json
here=pathlib.Path('.').resolve()
bcf=open('/Users/ben/bibe-rs/tests/corpus/styles/authoryear/authoryear.bcf',encoding='utf-8').read()
bibers='/Users/ben/bibe-rs/target/release/bibers'
files=[l.strip() for l in open('list.txt') if l.strip()]
def norm(s):
    return re.sub(r'(\{\{|\\name\{[^}]*\}\{\d+\}\{)([^}]*)(\}\{%)', lambda m: m.group(1)+','.join(sorted(m.group(2).split(',')))+m.group(3), s)
def msgs(blg):
    out=[]
    for l in blg.splitlines():
        m=re.match(r'^\[\d+\]\s+\S+>\s+(WARN|ERROR)\s+-\s+(.*)$',l)
        if m:
            t=re.sub(r'\S*/[0-9a-f]{32}_\d+\.utf8','data.bib',m.group(2))
            t=re.sub(r'\bat \S+\.p[ml] line \d+','at <PERLSRC>',t)
            out.append(m.group(1)+' - '+t)
    return sorted(out)
def run(i,f):
    d=here/'work'/str(i); shutil.rmtree(d,ignore_errors=True); d.mkdir(parents=True)
    shutil.copy(f,d/'data.bib')
    (d/'t.bcf').write_text(bcf.replace('>biblatex-examples.bib<','>data.bib<'),encoding='utf-8')
    res={}
    for who,cmd in (('biber',['biber']),('bibers',[bibers])):
        try:
            r=subprocess.run(cmd+['--noconf','t.bcf'],cwd=d,capture_output=True,timeout=600,env={'LANG':'en_US.UTF-8','LC_ALL':'en_US.UTF-8','PATH':'/Library/TeX/texbin:/usr/bin:/bin'})
            code=r.returncode; err=r.stderr.decode('utf-8','replace')
        except subprocess.TimeoutExpired:
            code='timeout'; err=''
        bbl=(d/'t.bbl').read_text('utf-8','replace') if (d/'t.bbl').exists() else None
        blg=(d/'t.blg').read_text('utf-8','replace') if (d/'t.blg').exists() else ''
        res[who]=(code,bbl,msgs(blg),'panicked' in err)
        for x in ('t.bbl','t.blg'):
            if (d/x).exists(): (d/x).rename(d/(who+'-'+x))
    b,r=res['biber'],res['bibers']
    crash_b = b[0] not in (0,2)
    status=('PANIC' if r[3] else
            'SAME' if (b[0]==r[0] or crash_b) and b[1] is not None and r[1] is not None and norm(b[1])==norm(r[1]) and (b[2]==r[2]) else
            'BBL-SAME-MSG-DIFF' if b[1] is not None and r[1] is not None and norm(b[1])==norm(r[1]) else
            'BIBER-CRASH' if crash_b else
            'DIFF')
    if status=='SAME': shutil.rmtree(d)
    return f,status,b[0],r[0]
with cf.ThreadPoolExecutor(6) as ex:
    results=list(ex.map(lambda a: run(*a), enumerate(files)))
from collections import Counter
print(Counter(s for _,s,_,_ in results))
json.dump(results, open('results.json','w'), indent=0)
for f,s,cb,cr in results:
    if s!='SAME': print(s, cb, cr, f)

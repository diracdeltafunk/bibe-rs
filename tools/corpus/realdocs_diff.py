#!/usr/bin/env python3
# Differential run of bibers vs biber (O3) over every biblatex document in
# TeX Live docs. Usage: collect candidates.txt (see M3 notes in docs/PLAN.md),
# then run from a scratch dir. realdocs_mkcases.py turns one passing document
# per biblatex style into tests/corpus/realdocs/ cases.
import subprocess, shutil, pathlib, re, sys, os, json, concurrent.futures as cf
REPO=pathlib.Path('/Users/ben/bibe-rs'); O3=str(REPO/'tools/oracle/biber-src'); BIBERS=str(REPO/'target/release/bibers')
here=pathlib.Path('.').resolve()
docs=[l.strip() for l in open('candidates.txt') if l.strip()]
ENV={'LANG':'en_US.UTF-8','LC_ALL':'en_US.UTF-8','PATH':'/Library/TeX/texbin:/usr/bin:/bin','HOME':os.environ['HOME']}
def norm(s): return re.sub(r'(\{\{|\\name\{[^}]*\}\{\d+\}\{)([^}]*)(\}\{%)', lambda m: m.group(1)+','.join(sorted(m.group(2).split(',')))+m.group(3), s)
def msgs(p):
    if not p.exists(): return None
    out=[]
    for l in p.read_text('utf-8','replace').splitlines():
        m=re.match(r'^\[\d+\]\s+\S+>\s+(WARN|ERROR)\s+-\s+(.*)$',l)
        if m:
            t=re.sub(r"'?\S*/[0-9a-f]{32}_\d+\.utf8'?","<BIB>",m.group(2))
            t=re.sub(r"BibTeX subsystem: [^,]+\.bib,","BibTeX subsystem: <BIB>,",t)
            t=re.sub(r"cannot be read in encoding '([^']*)': .*", r"cannot be read in encoding '\1': <DC-046>", t)
            t=re.sub(r'\bat \S+\.p[ml] line \d+','at <PERLSRC>',t)
            out.append(m.group(1)+' - '+t)
    return sorted(out)
def one(i, tex):
    tex=pathlib.Path(tex); src=tex.parent
    d=here/'work'/str(i); shutil.rmtree(d,ignore_errors=True)
    try: shutil.copytree(src,d,symlinks=True,ignore=shutil.ignore_patterns('*.pdf','*.zip','*.tar*'))
    except Exception as e: return (str(tex),'COPYFAIL',str(e))
    body=tex.read_text('utf-8','replace')
    if re.search(r'backend\s*=\s*bibtex',body): return (str(tex),'BIBTEX-BACKEND','')
    eng='xelatex' if re.search(r'\\usepackage(\[[^]]*\])?\{(fontspec|polyglossia|xeCJK|unicode-math)\}',body) else 'pdflatex'
    if re.search(r'\\usepackage(\[[^]]*\])?\{luacode|lua',body) and 'luatex' in body.lower(): eng='lualatex'
    stem=tex.stem
    try:
        subprocess.run([eng,'-interaction=nonstopmode','-draftmode',tex.name],cwd=d,env=ENV,capture_output=True,timeout=240)
    except subprocess.TimeoutExpired:
        return (str(tex),'LATEX-TIMEOUT',eng)
    bcf=d/(stem+'.bcf')
    if not bcf.exists(): return (str(tex),'NO-BCF',eng)
    res={}
    for who,cmd in (('o3',O3),('bibers',BIBERS)):
        try:
            r=subprocess.run([cmd,'--noconf',stem],cwd=d,env=ENV,capture_output=True,timeout=600)
            code=r.returncode; panic=b'panicked' in r.stderr
        except subprocess.TimeoutExpired: code='timeout'; panic=False
        bbl=d/(stem+'.bbl'); blg=d/(stem+'.blg')
        res[who]=(code, norm(bbl.read_text('utf-8','replace')) if bbl.exists() else None, msgs(blg), panic)
        for f in (bbl,blg):
            if f.exists(): f.rename(d/(who+'-'+f.name))
    a,b=res['o3'],res['bibers']
    if b[3]: st='PANIC'
    elif a[1]==b[1] and a[2]==b[2] and (a[0]==b[0]): st='SAME'
    elif a[1]==b[1] and a[1] is not None: st='BBL-SAME-MSG-DIFF'
    else: st='DIFF'
    if st=='SAME': shutil.rmtree(d)
    return (str(tex),st,f"{eng} o3={a[0]} bibers={b[0]}")
with cf.ThreadPoolExecutor(4) as ex:
    results=list(ex.map(lambda a: one(*a), enumerate(docs)))
from collections import Counter
print(Counter(s for _,s,_ in results))
json.dump(results,open('results.json','w'),indent=0)
for i,(t,s,info) in enumerate(results):
    if s not in ('SAME',): print(i,s,info,t)

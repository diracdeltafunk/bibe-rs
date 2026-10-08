import json,re,pathlib,shutil,subprocess,os,collections,tempfile
REPO=pathlib.Path('/Users/ben/bibe-rs'); OUT=REPO/'tests/corpus/realdocs'
ENV={'LANG':'en_US.UTF-8','LC_ALL':'en_US.UTF-8','PATH':'/Library/TeX/texbin:/usr/bin:/bin','HOME':os.environ['HOME']}
r=json.load(open('results.json'))
reps=collections.OrderedDict()
for i,(t,s,info) in enumerate(r):
    if s!='SAME': continue
    body=pathlib.Path(t).read_text('utf-8','replace')
    m=re.search(r'\\usepackage\[([^]]*)\]\{biblatex\}',body); style='default'
    if m:
        sm=re.search(r'(?:^|,)\s*(?:bib|cite)?style\s*=\s*([\w-]+)',m.group(1))
        if sm: style=sm.group(1)
    if style in ('default','standard'): continue
    reps.setdefault(style,(t,info.split()[0]))
made=[]
for style,(t,eng) in reps.items():
    tex=pathlib.Path(t)
    with tempfile.TemporaryDirectory() as d:
        d=pathlib.Path(d); shutil.copytree(tex.parent,d/'src',symlinks=True,ignore=shutil.ignore_patterns('*.pdf','*.zip','*.tar*'))
        w=d/'src'
        subprocess.run([eng,'-interaction=nonstopmode','-draftmode',tex.name],cwd=w,env=ENV,capture_output=True,timeout=240)
        bcf=w/(tex.stem+'.bcf')
        if not bcf.exists(): print("no bcf",style); continue
        text=bcf.read_text('utf-8')
        files=[]; ok=True
        def repl(m):
            global ok
            name=m.group(2)
            if '://' in name: return m.group(0)
            p=(w/name)
            if not p.exists():
                found=subprocess.run(['kpsewhich',name],capture_output=True,text=True,env=ENV).stdout.strip()
                p=pathlib.Path(found) if found else None
            if not p or not p.exists(): ok=False; return m.group(0)
            files.append(p); return m.group(1)+pathlib.Path(name).name+m.group(3)
        text=re.sub(r'(<bcf:datasource[^>]*>)([^<]+)(</bcf:datasource>)',repl,text)
        size=len(text.encode())+sum(f.stat().st_size for f in files)
        if not ok or size>2_000_000 or len({f.name for f in files})!=len(files): print("skip",style,ok,size); continue
        name=re.sub(r'[^a-z0-9-]+','-',style.lower()).strip('-')
        c=OUT/name; shutil.rmtree(c,ignore_errors=True); c.mkdir(parents=True)
        (c/'doc.bcf').write_text(text,encoding='utf-8')
        for f in files: shutil.copy(f,c/f.name)
        rel=str(tex).split('texmf-dist/doc/')[1]
        (c/'case.toml').write_text(f'''description = "TeX Live documentation example using biblatex style {style}"
source = "texmf-dist/doc/{rel} (TeX Live 2026), compiled with {eng}"
args = ["--noconf", "doc.bcf"]
outputs = ["doc.bbl"]
tags = ["suite:realdocs", "style:{style}"]
''',encoding='utf-8')
        made.append(name)
print(len(made),"cases:",made)

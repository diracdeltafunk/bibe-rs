#!/usr/bin/env python3
"""Generate tests/corpus/ported/<tfile>-<n>/ corpus cases from BiberTrace logs.

Each distinct configuration (see traceconf.py) that has at least one assertion
attached becomes a CLI-runnable case:
  * the test's .bcf, with setblxoption() calls applied as edits of the
    <bcf:options component="biblatex"> blocks / template elements,
    set_sortingtemplatename() applied as a change of the global
    sortingtemplatename option (and the datalist), set_allkeys() as \\nocite{*};
  * setoption() calls as CLI flags, or a generated config file for
    structured options (nosort, sourcemap, ...);
  * --sortdebug when an assertion inspects sort keys.

Per-test-file overrides live in OVERRIDES below. Writes
tools/port-tests/out/cases.json (case -> tfile, preps, config) used by check.py.

Usage: gen.py [tfile ...]
"""
import html
import json
import os
import re
import shutil
import sys
from collections import OrderedDict

sys.path.insert(0, os.path.dirname(__file__))
import traceconf as tc

ROOT = tc.ROOT
CORPUS = os.path.join(ROOT, 'tests', 'corpus', 'ported')
OUT = os.path.join(ROOT, 'tools', 'port-tests', 'out')

# setoption name -> (cli flag, kind)
FLAGS = {
    'sortlocale': ('--sortlocale', 'val'),
    'sortcase': ('--sortcase', 'tf'),
    'sortupper': ('--sortupper', 'tf'),
    'nodieonerror': ('--nodieonerror', 'flag'),
    'validate_datamodel': ('--validate-datamodel', 'flag'),
    'output_encoding': ('--output-encoding', 'val'),
    'output_safechars': ('--output-safechars', 'flag'),
    'isbn13': ('--isbn13', 'flag'),
    'isbn_normalise': ('--isbn-normalise', 'flag'),
    'namesep': ('--namesep', 'val'),
    'others_string': ('--others-string', 'val'),
    'xsvsep': ('--xsvsep', 'val'),
    'annotation_marker': ('--annotation-marker', 'val'),
    'nostdmacros': ('--nostdmacros', 'flag'),
    'mincrossrefs': ('--mincrossrefs', 'val'),
    'output_format': ('--output-format', 'val'),
    'input_format': ('--input-format', 'val'),
    'output_align': ('--output-align', 'flag'),
    'output_resolve_xdata': ('--output-resolve-xdata', 'flag'),
    'output_resolve_crossrefs': ('--output-resolve-crossrefs', 'flag'),
    'output_xdatasep': ('--output-xdatasep', 'val'),
    'output_xname': ('--output-xname', 'flag'),
    'output_xnamesep': ('--output-xnamesep', 'val'),
    'output_legacy_dates': ('--output-legacy-dates', 'flag'),
    'output_all_macrodefs': ('--output-all-macrodefs', 'flag'),
    'no_bltxml_schema': ('--no-bltxml-schema', 'flag'),
}
# options that go in a generated config file
CONF_OPTS = {'sourcemap'}
# options written into the .bcf (parse_ctrlfile sets them unconditionally, so a
# config file could not override a .bcf value the way the test's setoption() does)
BCF_OPTS = {'nosort': ('nosorts', 'nosort'), 'noinit': ('noinits', 'noinit'),
            'nolabel': ('nolabels', 'nolabel'),
            'nolabelwidthcount': ('nolabelwidthcounts', 'nolabelwidthcount')}


def set_bcf_regex_option(bcf, opt, val):
    outer, inner = BCF_OPTS[opt]
    xml = '  <bcf:%s>\n' % outer
    for o in val:
        v = o['value']
        if isinstance(v, dict):
            v = v.get('__regex', '')
        attrs = (' field="%s"' % esc(o['name'])) if 'name' in o else ''
        xml += '    <bcf:%s%s value="%s"/>\n' % (inner, attrs, esc(v))
    xml += '  </bcf:%s>\n' % outer
    return replace_elements(bcf, outer, '', xml)
# options that do not affect a CLI run at all
IGNORE_OPTS = {'bcf', 'dsn', 'tool', 'output_directory'}


def esc(s):
    return html.escape(str(s), quote=True)


def is_true(v):
    return v not in (None, '', '0', 0, False, 'false')


# ---------------------------------------------------------------- bcf edits

def options_block(bcf, typ):
    """Return (start, end) span of the inner part of the biblatex options block of type typ."""
    m = re.search(r'<bcf:options component="biblatex" type="%s">\n(.*?)  </bcf:options>\n' % re.escape(typ),
                  bcf, re.S)
    if m:
        return bcf, m.start(1), m.end(1)
    # create one after the global block
    g = re.search(r'<bcf:options component="biblatex" type="global">\n.*?  </bcf:options>\n', bcf, re.S)
    ins = '  <bcf:options component="biblatex" type="%s">\n  </bcf:options>\n' % typ
    bcf = bcf[:g.end()] + ins + bcf[g.end():]
    return options_block(bcf, typ)


def option_xml(opt, val):
    if isinstance(val, list):
        lines = ['    <bcf:option type="multivalued">', '      <bcf:key>%s</bcf:key>' % opt]
        for i, v in enumerate(val, 1):
            if isinstance(v, dict):
                attrs = ''.join(' %s="%s"' % (k, esc(x)) for k, x in sorted(v.items())
                                if k != 'content' and x is not None)
                lines.append('      <bcf:value order="%d"%s>%s</bcf:value>' % (i, attrs, esc(v.get('content', ''))))
            else:
                lines.append('      <bcf:value order="%d">%s</bcf:value>' % (i, esc(v)))
        lines.append('    </bcf:option>')
        return '\n'.join(lines) + '\n'
    return ('    <bcf:option type="singlevalued">\n      <bcf:key>%s</bcf:key>\n'
            '      <bcf:value>%s</bcf:value>\n    </bcf:option>\n') % (opt, esc(val))


def set_blx_option(bcf, opt, val, typ='global'):
    bcf, s, e = options_block(bcf, typ)
    inner = bcf[s:e]
    pat = re.compile(r'    <bcf:option type="\w+">\n      <bcf:key>%s</bcf:key>\n(?:(?!</bcf:option>).)*</bcf:option>[^\n]*\n' % re.escape(opt), re.S)
    new = option_xml(opt, val)
    if pat.search(inner):
        inner = pat.sub(lambda m: new, inner, count=1)
    else:
        inner = inner + new
    return bcf[:s] + inner + bcf[e:]


def xml_from_simple(tag, d, indent):
    """Inverse of XML::LibXML::Simple for template structures."""
    attrs = []
    kids = []
    content = None
    for k in sorted(d):
        v = d[k]
        if k == 'content':
            content = v
        elif isinstance(v, list):
            kids.append((k, v))
        elif isinstance(v, dict):
            kids.append((k, [v]))
        elif v is not None:
            attrs.append(' %s="%s"' % (k, esc(v)))
    pad = ' ' * indent
    if not kids:
        return '%s<bcf:%s%s>%s</bcf:%s>\n' % (pad, tag, ''.join(attrs), esc(content or ''), tag)
    out = '%s<bcf:%s%s>\n' % (pad, tag, ''.join(attrs))
    # children: keep order by 'order' attribute when present
    for k, lst in kids:
        if all(isinstance(x, dict) and 'order' in x for x in lst):
            lst = sorted(lst, key=lambda x: int(x['order']))
        for x in lst:
            if isinstance(x, dict):
                out += xml_from_simple(k, x, indent + 2)
            else:
                out += '%s  <bcf:%s>%s</bcf:%s>\n' % (pad, k, esc(x), k)
    out += '%s</bcf:%s>\n' % (pad, tag)
    return out


def replace_elements(bcf, tag, attr_re, new_xml, anchor_tag=None):
    """Remove all <bcf:tag ...attr_re...>...</bcf:tag> and insert new_xml where the first was."""
    pat = re.compile(r'  <bcf:%s\b[^>]*%s[^>]*>.*?</bcf:%s>\n' % (tag, attr_re, tag), re.S)
    m = pat.search(bcf)
    if m:
        pos = m.start()
        bcf = pat.sub('', bcf)
        return bcf[:pos] + new_xml + bcf[pos:]
    # insert before the first sorting template or end of controlfile
    m = re.search(r'  <bcf:(%s)\b' % (anchor_tag or 'sortingtemplate'), bcf)
    pos = m.start() if m else bcf.rindex('</bcf:controlfile>')
    return bcf[:pos] + new_xml + bcf[pos:]


def set_labelalphatemplate(bcf, t, typ):
    d = dict(t)
    d['type'] = typ
    xml = xml_from_simple('labelalphatemplate', d, 2)
    return replace_elements(bcf, 'labelalphatemplate', r'type="%s"' % re.escape(typ), xml)


def set_labelalphanametemplate(bcf, t):
    xml = ''
    for name, parts in t.items():
        xml += '  <bcf:labelalphanametemplate name="%s">\n' % esc(name)
        for i, np in enumerate(parts, 1):
            attrs = ''.join(' %s="%s"' % (k, esc(np[k])) for k in
                            ('use', 'pre', 'substring_width', 'substring_side', 'substring_compound')
                            if np.get(k) is not None)
            xml += '    <bcf:namepart order="%d"%s>%s</bcf:namepart>\n' % (i, attrs, esc(np['namepart']))
        xml += '  </bcf:labelalphanametemplate>\n'
    return replace_elements(bcf, 'labelalphanametemplate', '', xml)


def set_sortingnamekeytemplate(bcf, t):
    xml = ''
    for name, spec in t.items():
        vis = spec.get('visibility')
        xml += '  <bcf:sortingnamekeytemplate name="%s"%s>\n' % (esc(name), ' visibility="%s"' % vis if vis else '')
        for i, kp in enumerate(spec['template'], 1):
            xml += '    <bcf:keypart order="%d">\n' % i
            for j, p in enumerate(kp, 1):
                attrs = ''.join(' %s="%s"' % (k, esc(p[k])) for k in ('use', 'inits') if p.get(k) is not None)
                xml += '      <bcf:part type="%s" order="%d"%s>%s</bcf:part>\n' % (p['type'], j, attrs, esc(p['value']))
            xml += '    </bcf:keypart>\n'
        xml += '  </bcf:sortingnamekeytemplate>\n'
    return replace_elements(bcf, 'sortingnamekeytemplate', '', xml)


def set_extradatespec(bcf, t):
    xml = '  <bcf:extradatespec>\n'
    for scope in t:
        xml += '    <bcf:scope>\n'
        for i, f in enumerate(scope, 1):
            xml += '      <bcf:field order="%d">%s</bcf:field>\n' % (i, esc(f))
        xml += '    </bcf:scope>\n'
    xml += '  </bcf:extradatespec>\n'
    return replace_elements(bcf, 'extradatespec', '', xml)


def set_global_sorting(bcf, listname, tname):
    """Make list `listname` use sorting template tname.

    An explicit <bcf:datalist> that is not the section's global-sort list just gets
    its sortingtemplatename attribute changed (its name is kept, as in-process).
    The global-sort list (implicit, or explicit with the global template) is
    switched by changing the global sortingtemplatename option, which renames it."""
    gss = re.search(r'<bcf:key>sortingtemplatename</bcf:key>\s*<bcf:value>([^<]*)</bcf:value>', bcf).group(1)
    pat = re.compile(r'<bcf:datalist\b[^>]*?/>|<bcf:datalist\b.*?</bcf:datalist>', re.S)
    for m in pat.finditer(bcf):
        s = m.group(0)
        if 'name="%s"' % listname in s:
            st = re.search(r'sortingtemplatename="([^"]*)"', s).group(1)
            if st != gss:
                s2 = re.sub(r'sortingtemplatename="[^"]*"', 'sortingtemplatename="%s"' % tname, s)
                return bcf[:m.start()] + s2 + bcf[m.end():], listname
    newname = '/'.join([tname] + listname.split('/')[1:])
    bcf = set_blx_option(bcf, 'sortingtemplatename', tname)
    pat = re.compile(r'<bcf:datalist\b[^>]*?/>|<bcf:datalist\b.*?</bcf:datalist>', re.S)

    def fix(m):
        s = m.group(0)
        if 'name="%s"' % listname not in s:
            return s
        s = s.replace('name="%s"' % listname, 'name="%s"' % newname)
        s = re.sub(r'sortingtemplatename="[^"]*"', 'sortingtemplatename="%s"' % tname, s)
        return s
    return pat.sub(fix, bcf), newname


def set_allkeys(bcf, secnum='0', keep=True):
    m = re.search(r'(<bcf:section number="%s">\n)(.*?)(  </bcf:section>)' % secnum, bcf, re.S)
    if not keep:
        return bcf[:m.start(2)] + '    <bcf:citekey order="1" intorder="1">*</bcf:citekey>\n' + bcf[m.end(2):]
    # The cited keys are kept: the tests reuse the output object, whose \\missing
    # entries from the first prepare() are still asserted afterwards.
    n = len(re.findall(r'<bcf:citekey\b', m.group(2))) + 1
    inner = m.group(2) + '    <bcf:citekey order="%d" intorder="1">*</bcf:citekey>\n' % n
    return bcf[:m.start(2)] + inner + bcf[m.end(2):]


def datasources(bcf):
    return re.findall(r'<bcf:datasource type="file" datatype="\w+"[^>]*>([^<]+)</bcf:datasource>', bcf)


# ---------------------------------------------------------------- config file

def conf_xml(opts):
    out = '<?xml version="1.0" encoding="UTF-8"?>\n<config>\n'
    for k, v in opts.items():
        if k in ('nosort', 'noinit', 'nolabel', 'nolabelwidthcount'):
            tag = {'noinit': 'noinits', 'nolabel': 'nolabels'}.get(k, k)
            out += '  <%s>\n' % tag
            for o in v:
                attrs = ''.join(' %s="%s"' % (a, esc(o[a])) for a in ('name', 'value') if a in o)
                out += '    <option%s/>\n' % attrs
            out += '  </%s>\n' % tag
        elif k == 'sourcemap':
            out += '  <sourcemap>\n'
            for maps in v:
                m = dict(maps)
                m.pop('level', None)
                out += xml_from_simple('maps', m, 4).replace('bcf:', '')
            out += '  </sourcemap>\n'
        else:
            out += '  <%s>%s</%s>\n' % (k, esc(v), k)
    out += '</config>\n'
    return out


# ---------------------------------------------------------------- generation

SORTKEY_SUBS = {'get_sortdata_for_key', 'get_sortdataschema'}


def build_case(tfile, n, cfg, asserts, notes, preps):
    name = '%s-%d' % (tfile, n)
    bcfname = cfg.bcf
    with open(os.path.join(tc.TDATA, bcfname), encoding='utf-8') as f:
        bcf = f.read()
    files = OrderedDict()
    args = []
    conf = OrderedDict()
    desc = []

    # biblatex options
    for (opt, scope, sv), val in cfg.blx_final().items():
        typ = 'global'
        if scope == 'ENTRYTYPE':
            typ = sv
        elif scope is not None:
            notes.append('unsupported blx scope %s for %s' % (scope, opt))
            continue
        if opt == 'labelalphatemplate':
            bcf = set_labelalphatemplate(bcf, val, typ)
        elif opt == 'labelalphanametemplate':
            bcf = set_labelalphanametemplate(bcf, val)
        elif opt == 'sortingnamekeytemplate':
            bcf = set_sortingnamekeytemplate(bcf, val)
        elif opt == 'extradatespec':
            bcf = set_extradatespec(bcf, val)
        elif isinstance(val, dict):
            notes.append('unsupported structured blx option %s' % opt)
            continue
        else:
            bcf = set_blx_option(bcf, opt, val, typ)
        desc.append('%s=%s%s' % (opt, json.dumps(val)[:40], '' if typ == 'global' else '@' + typ))

    renames = {}
    for lname, tname in cfg.tmplname.items():
        bcf, newname = set_global_sorting(bcf, lname, tname)
        renames[lname] = newname
        desc.append('sorting=%s' % tname)
    for lname, t in cfg.tmpl.items():
        notes.append('set_sortingtemplate() on %s ignored: a no-op in biber 2.22 (generate_sortdataschema relinks the template by name at prepare time)' % lname)

    for (s, a) in cfg.mods:
        sh = tc.short(s)
        if sh == 'set_allkeys' and is_true(a[0]):
            # reset_keyorder() => the test wants pure \nocite{*} (bib order)
            bcf = set_allkeys(bcf, keep=False)
            desc.append('nocite{*}')
        elif sh in ('del_citekeys', 'reset_keyorder', 'init_sets'):
            pass
        else:
            notes.append('unhandled mutator %s(%s)' % (sh, json.dumps(a)[:60]))

    # biber options
    bcf_biber_opts = set(re.findall(r'<bcf:key>(\w+)</bcf:key>',
                                    re.search(r'<bcf:options component="biber".*?</bcf:options>', bcf, re.S).group(0)))
    opts = OrderedDict()
    for k, v in cfg.opts_pre.items():
        if k not in bcf_biber_opts:
            opts[k] = v
    opts.update(cfg.opts)
    userconf = cfg.new_args.get('configfile')
    for k, v in cfg.new_args.items():
        if k not in ('noconf', 'configfile', 'tool', 'configtool'):
            opts.setdefault(k, v)
    for k, v in opts.items():
        if k in IGNORE_OPTS:
            continue
        if k in BCF_OPTS:
            bcf = set_bcf_regex_option(bcf, k, v)
            desc.append('%s (in .bcf)' % k)
            continue
        if k in CONF_OPTS:
            if v is not None:
                conf[k] = v
            continue
        if k not in FLAGS:
            notes.append('unmapped biber option %s=%s' % (k, json.dumps(v)[:60]))
            continue
        flag, kind = FLAGS[k]
        if kind == 'val':
            if v is None:
                continue
            args.append('%s=%s' % (flag, v))
        elif kind == 'tf':
            args.append('%s=%s' % (flag, 'true' if is_true(v) else 'false'))
        elif kind == 'flag':
            if is_true(v):
                args.append(flag)
        desc.append('%s=%s' % (k, v if not isinstance(v, (list, dict)) else '...'))

    sortdebug = any(c['sub'].split('::')[-1] in SORTKEY_SUBS or
                    (c['sub'].endswith('::get_entryfield') and c['args'][-1] == 'sortlabelalpha')
                    for a in asserts for c in a['calls'])
    if sortdebug:
        args.append('--sortdebug')

    if userconf:
        if conf:
            notes.append('both a test config file and generated config options')
        args.insert(0, '--configfile=%s' % userconf)
        with open(os.path.join(tc.TDATA, userconf), encoding='utf-8') as f:
            files[userconf] = f.read()
        desc.append('configfile=' + userconf)
    elif conf:
        args.insert(0, '--configfile=ported.conf')
        files['ported.conf'] = conf_xml(conf)
        desc.append('conf:' + ','.join(conf))
    else:
        args.insert(0, '--noconf')
    args.append(bcfname)
    if tfile in OVERRIDES:
        bcf, args = OVERRIDES[tfile](cfg, preps, bcf, args, desc)
    files[bcfname] = bcf
    notes_renames.update(renames)
    return name, files, args, desc


notes_renames = {}


def toml_str(s):
    return json.dumps(s, ensure_ascii=False)


def write_case(name, tfile, files, args, desc, preps, cfg, extra_tags=()):
    d = os.path.join(CORPUS, name)
    os.makedirs(d, exist_ok=True)
    # remove stale inputs (keep expected/)
    for f in os.listdir(d):
        if f not in ('expected',) and f not in files:
            p = os.path.join(d, f)
            if os.path.isfile(p):
                os.remove(p)
    for fn, content in files.items():
        with open(os.path.join(d, fn), 'w', encoding='utf-8') as f:
            f.write(content)
    for ds in datasources(files[cfg.bcf]):
        shutil.copyfile(os.path.join(tc.TDATA, ds), os.path.join(d, ds))
    stem = cfg.bcf[:-4]
    lines = lines_of(tfile, preps)
    toml = []
    toml.append('description = %s' % toml_str('biber t/%s.t, configuration %s (prepare() at line %s)%s' % (
        tfile, name.rsplit('-', 1)[1], lines, ': ' + '; '.join(desc) if desc else '')))
    toml.append('source = %s' % toml_str('vendor/biber/t/%s.t + t/tdata/%s (traced by tools/port-tests)' % (tfile, cfg.bcf)))
    toml.append('args = [%s]' % ', '.join(toml_str(a) for a in args))
    toml.append('outputs = [%s]' % toml_str(stem + '.bbl'))
    toml.append('tags = [%s]' % ', '.join(toml_str(t) for t in ['suite:ported', 't:' + tfile] + list(extra_tags)))
    if cfg.new_args.get('configfile') == 'biber-test.conf' and any('nocite' in x for x in desc):
        toml.insert(0, '# NONDETERMINISTIC: biber-test.conf creates entries keyed with $MAPUNIQ (random, BB-113);\n'
                       '# the loopkey:<x>:<random> keys differ between runs. Needs a harness normaliser.')
    with open(os.path.join(d, 'case.toml'), 'w', encoding='utf-8') as f:
        f.write('\n'.join(toml) + '\n')


def lines_of(tfile, preps):
    return ','.join(str(p['line']) for p in preps)


# ------------------------------------------------------------ manual overrides
# State the .t files set without going through a traced Biber sub.

def _src_lines(tfile):
    with open(os.path.join(tc.TDIR, tfile + '.t'), encoding='utf-8') as f:
        return f.read().split('\n')


def ov_crossrefs(cfg, preps, bcf, args, desc):
    # crossrefs.t:43  $DATAFIELD_SETS{'nobtitle'} = ['booktitle'];
    ins = ('  <bcf:datafieldset name="nobtitle">\n    <bcf:member field="booktitle"/>\n'
           '  </bcf:datafieldset>\n')
    i = bcf.index('  <bcf:datafieldset ')
    desc.append('datafieldset nobtitle (crossrefs.t:43)')
    return bcf[:i] + ins + bcf[i:], args


def ov_encoding(cfg, preps, bcf, args, desc):
    # change_ds_encoding($bib, $enc) sets the datasource encoding (= --input-encoding);
    # Biber::LaTeX::Recode->init_sets('full','full') = --decodecharsset/--output-safecharsset
    src = _src_lines('encoding')
    pl = preps[0]['line']
    enc = None
    full = False
    for ln in range(pl - 1, 0, -1):
        l = src[ln - 1]
        if 'parse_ctrlfile' in l:
            break
        m = re.search(r"change_ds_encoding\('[^']+', '([^']+)'\)", l)
        if m and enc is None:
            enc = m.group(1)
    full = any(tc.short(x) == 'init_sets' and a == ['full', 'full'] for x, a in cfg.mods)
    if enc:
        args.insert(-1, '--input-encoding=%s' % enc)
        desc.append('input_encoding=%s' % enc)
    if full:
        args[-1:-1] = ['--decodecharsset=full', '--output-safecharsset=full']
        desc.append('recode sets full/full')
    return bcf, args


OVERRIDES = {'crossrefs': ov_crossrefs, 'encoding': ov_encoding}


def tool_dsn(tfile, cfg, preps):
    src = _src_lines(tfile)
    for ln in range(preps[0]['line'] - 1, 0, -1):
        m = re.search(r"\$ARGV\[0\]\s*=\s*'([^']+)'", src[ln - 1])
        if m:
            return m.group(1)
    return cfg.opts.get('dsn') or cfg.opts_pre.get('dsn')


def build_output_case(tfile, n, cfg, asserts, notes, preps):
    """Tool mode / bibtex / biblatexml output (deferred to milestone M4)."""
    name = '%s-%d' % (tfile, n)
    files = OrderedDict()
    copies = []
    args = []
    desc = []
    conffile = cfg.new_args.get('configfile')
    if cfg.tool:
        args.append('--tool')
    if conffile:
        args.append('--configfile=%s' % conffile)
        copies.append(conffile)
        desc.append('configfile=' + conffile)
    else:
        args.append('--noconf')
    opts = OrderedDict(cfg.opts_pre)
    opts.update(cfg.opts)
    for k, v in opts.items():
        if k in IGNORE_OPTS or v is None:
            continue
        if k not in FLAGS:
            notes.append('unmapped biber option %s=%s' % (k, json.dumps(v)[:60]))
            continue
        flag, kind = FLAGS[k]
        if kind == 'val':
            args.append('%s=%s' % (flag, v))
        elif kind == 'tf':
            args.append('%s=%s' % (flag, 'true' if is_true(v) else 'false'))
        elif is_true(v):
            args.append(flag)
        desc.append('%s=%s' % (k, v))
    fmt = opts.get('output_format', 'bibtex')
    if cfg.tool:
        dsn = tool_dsn(tfile, cfg, preps)
        args.append(dsn)
        copies.append(dsn)
        stem = dsn.rsplit('.', 1)[0]
        out = stem + ('_bibertool.bib' if fmt == 'bibtex' else '_bibertool.bltxml')
        log = dsn + '.blg'
    else:
        if 'tool' in opts:
            notes.append('setoption(tool) without tool mode is not reproducible from the CLI; ignored')
        with open(os.path.join(tc.TDATA, cfg.bcf), encoding='utf-8') as f:
            bcf = f.read()
        files[cfg.bcf] = bcf
        copies.extend(datasources(bcf))
        args.append(cfg.bcf)
        out = cfg.bcf[:-4] + '_biber.bib'
        log = cfg.bcf[:-4] + '.blg'
    return name, files, copies, args, desc, out, log


def write_output_case(name, tfile, files, copies, args, desc, preps, out, log, description=None,
                      tags=('deferred:m4',)):
    d = os.path.join(CORPUS, name)
    os.makedirs(d, exist_ok=True)
    for f in os.listdir(d):
        p = os.path.join(d, f)
        if os.path.isfile(p):
            os.remove(p)
    for fn, content in files.items():
        with open(os.path.join(d, fn), 'w', encoding='utf-8') as f:
            f.write(content)
    for fn in copies:
        shutil.copyfile(os.path.join(tc.TDATA, fn), os.path.join(d, fn))
    toml = ['description = %s' % toml_str(description or 'biber t/%s.t, configuration %s (prepare at line %s)%s' % (
                tfile, name.rsplit('-', 1)[1], lines_of(tfile, preps), ': ' + '; '.join(desc) if desc else '')),
            'source = %s' % toml_str('vendor/biber/t/%s.t + t/tdata (traced by tools/port-tests)' % tfile),
            'args = [%s]' % ', '.join(toml_str(a) for a in args),
            'outputs = [%s]' % toml_str(out),
            'log = %s' % toml_str(log),
            'tags = [%s]' % ', '.join(toml_str(t) for t in ['suite:ported', 't:' + tfile] + list(tags))]
    with open(os.path.join(d, 'case.toml'), 'w', encoding='utf-8') as f:
        f.write('\n'.join(toml) + '\n')


# tool-config.t only checks configuration internals (unit vectors), no output
NO_OUTPUT_CASE = {'tool-config'}

SKIP_TFILES = {'bcfvalidation', 'configfile', 'langtags', 'utils', 'remote-files',
               'full-bbl', 'full-bibtex', 'full-dot'}


# The BIBER_DEV_TESTS end-to-end tests run bin/biber as a subprocess (not traced).
SUBPROCESS_CASES = {
    'full-bbl': dict(args=['--noconf', 'full-bbl.bcf'], bcf='full-bbl.bcf', out='full-bbl.bbl',
                     tags=[], ref='full-bbl.bbl',
                     desc='t/full-bbl.t: bin/biber --noconf --nolog --output-file=<tmp> t/tdata/full-bbl.bcf '
                          '(--nolog/--output-file dropped so that the .blg is kept)'),
    'full-bibtex': dict(args=['--noconf', '--output-format=bibtex', '--output-align', 'full-bibtex.bcf'],
                        bcf='full-bibtex.bcf', out='full-bibtex_biber.bib', tags=['deferred:m4'],
                        ref='full-bibtex_biber.bib',
                        desc='t/full-bibtex.t: bin/biber --noconf --nolog --output-format=bibtex '
                             '--output-file=<tmp> --output-align t/tdata/full-bibtex.bcf'),
}


def gen_subprocess_cases(all_cases):
    for tfile, spec in SUBPROCESS_CASES.items():
        name = tfile + '-1'
        with open(os.path.join(tc.TDATA, spec['bcf']), encoding='utf-8') as f:
            bcf = f.read()
        copies = datasources(bcf)
        write_output_case(name, tfile, {spec['bcf']: bcf}, copies, spec['args'], [], [], spec['out'],
                          spec['bcf'][:-4] + '.blg', description=spec['desc'], tags=spec['tags'])
        all_cases[name] = {'tfile': tfile, 'preps': [], 'args': spec['args'], 'bcf': spec['bcf'],
                           'output_file': spec['out'], 'notes': [], 'config': {}, 'renames': {},
                           'reference': 'vendor/biber/t/tdata/' + spec['ref']}
        print('%-22s 1 cases (subprocess test)' % tfile)


def main(argv):
    all_cases = {}
    cases_json = os.path.join(OUT, 'cases.json')
    if os.path.exists(cases_json):
        with open(cases_json) as f:
            all_cases = json.load(f)
    todo = argv or tc.tfiles()
    if any(t in SUBPROCESS_CASES for t in todo):
        gen_subprocess_cases(all_cases)
    for tfile in todo:
        if tfile in SKIP_TFILES:
            continue
        preps, asserts = tc.analyse(tfile)
        groups = OrderedDict()
        for p in preps:
            groups.setdefault(p['config'].key(), []).append(p)
        # drop old cases for this tfile
        for k in [k for k, v in all_cases.items() if v['tfile'] == tfile]:
            del all_cases[k]
        n = 0
        for key, ps in groups.items():
            pn = {p['prep'] for p in ps}
            ass = [a for a in asserts if a['prep'] in pn]
            if not ass:
                continue
            cfg = ps[0]['config']
            if cfg.tool or (cfg.output and cfg.output != 'Biber::Output::bbl'):
                if tfile in NO_OUTPUT_CASE:
                    continue
                n += 1
                notes = []
                name, files, copies, args, desc, out, log = build_output_case(tfile, n, cfg, ass, notes, ps)
                write_output_case(name, tfile, files, copies, args, desc, ps, out, log)
                all_cases[name] = {'tfile': tfile, 'preps': sorted(pn), 'args': args, 'bcf': cfg.bcf,
                                   'output_file': out, 'notes': notes, 'config': cfg.to_json(),
                                   'renames': {}, 'deferred': 'm4'}
                if notes:
                    print('%s: %s' % (name, '; '.join(notes)))
                continue
            n += 1
            notes = []
            notes_renames.clear()
            name, files, args, desc = build_case(tfile, n, cfg, ass, notes, ps)
            write_case(name, tfile, files, args, desc, ps, cfg)
            all_cases[name] = {'tfile': tfile, 'preps': sorted(pn), 'args': args,
                               'bcf': cfg.bcf, 'notes': notes, 'config': cfg.to_json(),
                               'renames': dict(notes_renames)}
            if notes:
                print('%s: %s' % (name, '; '.join(notes)))
        print('%-22s %d cases' % (tfile, n))
    os.makedirs(OUT, exist_ok=True)
    with open(cases_json, 'w') as f:
        json.dump(all_cases, f, indent=1, sort_keys=True)


if __name__ == '__main__':
    main(sys.argv[1:])

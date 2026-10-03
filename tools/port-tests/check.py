#!/usr/bin/env python3
"""Fidelity check: do the tests/corpus/ported goldens reproduce what biber's
t/*.t assert?

For every assertion recorded by BiberTrace, find the accessor it checks (the
"subject" call), classify it, and for .bbl-visible kinds verify the expected
value from the .t file against the golden output of the corpus case generated
for that configuration (gen.py):

  bbl_entry   get_output_entry(key, list)    expected entry text occurs in the golden .bbl section
  sort_order  list->get_keys                 expected key order == entry order of the list in the .bbl
  sortkey     get_sortdata_for_key(key)->[0] expected string == the --sortdebug line for key
  list_field  get_entryfield / get_extra*data_for_key / get_labelalphadata_for_key
              -> \\field{..}/\\strng{..}/\\true{..} in the entry
  un / ul     get_unsummary / get_uniquelist  -> un=/ul= options in the \\name blocks
  entry_field $entry->get_field(f)            -> \\field{f}{v} etc. (if it isn't there the
                                                assertion is classed as internal)
  labelname   get_labelname_info             -> \\field{labelnamesource}
  exists      bibentry/has_citekey/entry      -> \\entry{key} present
  alias       get_citekey_alias              -> \\keyalias{alias}{key}
  warnings    get_field('warnings')          -> WARN lines in expected/messages.txt

Additionally, for every case, the complete in-process .bbl captured after the
corresponding prepare() is compared with the golden (informational: tests
mutate state between prepare() calls, so non-asserted entries may differ).

Writes tools/port-tests/out/fidelity.json and prints a per-file summary.
"""
import json
import os
import re
import sys
import unicodedata
from collections import OrderedDict, Counter

sys.path.insert(0, os.path.dirname(__file__))
import traceconf as tc

ROOT = tc.ROOT
CORPUS = os.path.join(ROOT, 'tests', 'corpus', 'ported')
OUT = os.path.join(ROOT, 'tools', 'port-tests', 'out')

NAV = {'new', 'parse_ctrlfile', 'set_output_obj', 'setoption', 'setblxoption', 'prepare',
       'prepare_tool', 'sections', 'get_section', 'bibentries', 'datalists', 'get_list',
       'get_lists_by_attrs', 'get_output_obj', 'get_id', 'nth_name', 'is_undef',
       'is_def_and_null', 'is_null', 'get_citekeys', 'del_field', 'set_extraalphadata_for_key',
       'set_sortingtemplatename', 'del_entry', 'del_entries', 'del_everykeys', 'del_citekeys',
       'init_cache', 'set_allkeys', 'reset_keyorder', 'tool_mode_setup', 'get_dm',
       'set_output_target_file', 'set_output_target', 'number', 'getoption', 'get_labelname_info'}

LIST_FIELD = {
    'get_extradatedata_for_key': 'extradate',
    'get_extraalphadata_for_key': 'extraalpha',
    'get_extratitledata_for_key': 'extratitle',
    'get_extratitleyeardata_for_key': 'extratitleyear',
    'get_labelalphadata_for_key': 'labelalpha',
    'get_extranamedata_for_key': 'extraname',
}


def nfc(s):
    return unicodedata.normalize('NFC', s) if isinstance(s, str) else s


def norm_bbl(text):
    def sort_opts(m):
        return m.group(1) + ','.join(sorted(m.group(2).split(','))) + m.group(3)
    text = re.sub(r'(\\name\{[^}]*\}\{\d+\}\{)([^}]*)(\}\{%)', sort_opts, text)
    text = re.sub(r'(?m)^(\s*\{\{)([^}]*)(\}\{%)$', sort_opts, text)
    return nfc(text)


def strip_sortdebug(text):
    return re.sub(r"(?m)^    % sorting key for '.*':\n    % .*\n", '', text)


class Bbl:
    def __init__(self, text):
        self.text = norm_bbl(text)
        self.sections = OrderedDict()
        for m in re.finditer(r'\\refsection\{(\d+)\}\n(.*?)\\endrefsection\n', self.text, re.S):
            sec = {'text': m.group(2), 'lists': OrderedDict(), 'aliases': {}, 'missing': set(),
                   'entries': {}}
            for lm in re.finditer(r'  \\datalist\[(\w+)\]\{(.*?)\}\n(.*?)  \\enddatalist\n', m.group(2), re.S):
                ents = []
                for em in re.finditer(r"(?:    % sorting key for '([^\n]*?)':\n    % ([^\n]*)\n)?(    \\entry\{([^\n]*?)\}.*?    \\endentry\n)",
                                      lm.group(3), re.S):
                    ents.append({'key': em.group(4), 'sortkey': em.group(2), 'text': em.group(3)})
                    sec['entries'].setdefault(em.group(4), em.group(3))
                if lm.group(2) in sec['lists']:   # several lists may share a name (datalists.t)
                    sec['lists'][lm.group(2)]['dups'].append(ents)
                else:
                    sec['lists'][lm.group(2)] = {'type': lm.group(1), 'entries': ents, 'dups': []}
            for am in re.finditer(r'\\keyalias\{(.*?)\}\{(.*?)\}', m.group(2)):
                sec['aliases'][am.group(1)] = am.group(2)
            for mm in re.finditer(r'\\missing\{(.*?)\}', m.group(2)):
                sec['missing'].add(mm.group(1))
            self.sections[m.group(1)] = sec

    def section(self, n):
        return self.sections.get(str(n if n is not None else 0))


PYENC = {'utf8': 'utf-8', 'utf-8': 'utf-8', 'latin1': 'latin-1', 'latin3': 'iso8859-3',
         'latin9': 'iso8859-15', 'cp1252': 'cp1252', 'applemacce': 'mac-latin2', 'ascii': 'ascii'}


def out_encoding(c):
    enc = None
    for a in c['args']:
        if a.startswith('--output-encoding='):
            enc = a.split('=', 1)[1]
    if enc is None and not c.get('bcf'):
        enc = 'UTF-8'
    if enc is None:
        bcf = open(os.path.join(CORPUS, c['name'], c['bcf']), encoding='utf-8').read()
        m = re.search(r'<bcf:key>output_encoding</bcf:key>\s*<bcf:value>([^<]*)</bcf:value>', bcf)
        enc = m.group(1) if m else 'UTF-8'
    return PYENC.get(enc.lower(), enc)


def debyte(e):
    """Test strings passed through encode_utf8() arrive as latin-1 chars; undo that."""
    if isinstance(e, str):
        try:
            return e.encode('latin-1').decode('utf-8')
        except (UnicodeEncodeError, UnicodeDecodeError):
            return e
    if isinstance(e, list):
        return [debyte(x) for x in e]
    return e


def unwrap(ret):
    if isinstance(ret, list) and len(ret) == 1:
        return ret[0]
    return ret


def subject_of(a):
    calls = a['calls']
    wrapper = None
    subj = None
    for c in reversed(calls):
        sh = tc.short(c['sub'])
        if sh in ('is_undef', 'is_def_and_null', 'is_null') and wrapper is None and subj is None:
            wrapper = sh
            continue
        if sh in NAV and not (sh == 'get_labelname_info' and calls[-1] is c):
            continue
        subj = c
        break
    if subj is None:
        # entry existence / labelname info
        for c in reversed(calls):
            sh = tc.short(c['sub'])
            if sh in ('entry', 'bibentry', 'get_labelname_info', 'getoption'):
                subj = c
                break
    return subj, wrapper


def expected_of(a, wrapper, subj):
    fn = a['fn']
    args = a['args']
    if fn in ('eq_or_diff', 'is', 'is_deeply', 'eq_or_diff_text', 'eq_or_diff_data', 'isnt'):
        return True, debyte(args[1])
    if fn == 'ok':
        if wrapper == 'is_undef':
            return True, None
        if wrapper in ('is_def_and_null', 'is_null'):
            return True, ''
        if subj is not None:
            return True, unwrap(subj.get('ret'))
    return False, None


def name_maps(calls):
    """nlid -> (key, field); nid -> index (1-based)"""
    nl = {}
    nm = {}
    for c in calls:
        sh = tc.short(c['sub'])
        r = unwrap(c.get('ret'))
        if sh == 'get_field' and isinstance(r, dict) and r.get('__obj') == 'Biber::Entry::Names':
            nl[r['id']] = (c['args'][0].get('key'), c['args'][1])
            for i, n in enumerate(r.get('names', []), 1):
                if isinstance(n, dict) and isinstance(n.get('value'), dict):
                    nm[n['value'].get('id')] = (r['id'], i)
    return nl, nm


def entry_block(sec, key, listname=None):
    if listname and listname in sec['lists']:
        for e in sec['lists'][listname]['entries']:
            if e['key'] == key:
                return e['text']
    return sec['entries'].get(key)


def name_block(etext, field):
    m = re.search(r'\\name\{%s\}\{(\d+)\}\{([^}]*)\}\{%%\n(.*?)\n      \}\n' % re.escape(field), etext, re.S)
    if not m:
        return None
    names = re.findall(r'^\s*\{\{([^}]*)\}\{%$', m.group(3), re.M)
    return {'count': int(m.group(1)), 'opts': m.group(2), 'names': names}


def opt_val(opts, k):
    for o in opts.split(','):
        if '=' in o:
            a, b = o.split('=', 1)
            if a == k:
                return b
    return None


def field_lines(etext, field):
    f = re.escape(field)
    res = []
    for m in re.finditer(r'^      \\(field|strng|true|false|range|list|verb)\{%s\}(.*)$' % f, etext, re.M):
        res.append((m.group(1), m.group(2)))
    return res


def check_rendered(et, field, exp):
    """Fields whose internal value is rendered differently in the .bbl."""
    if field == 'entrytype' and isinstance(exp, str):
        ok = re.match(r'    \\entry\{[^}]*\}\{%s\}' % re.escape(exp), et) is not None
        return ok, '\\entry{..}{%s}' % exp
    if isinstance(exp, str):
        m = re.search(r'^      \\verb\{%s\}\n      \\verb (.*)\n      \\endverb$' % re.escape(field), et, re.M)
        if m and nfc(m.group(1)) == nfc(exp):
            return True, '\\verb{%s}' % field
        return False, ''
    if isinstance(exp, list) and exp and all(isinstance(x, list) and len(x) == 2 for x in exp):
        parts = []
        for a, b in exp:
            parts.append(a + ('' if b is None else '\\bibrangedash' if b == '' else '\\bibrangedash ' + b))
        want = '{%s}' % '\\bibrangessep '.join(parts)
        lines = field_lines(et, field)
        ok = any(k == 'field' and nfc(v) == nfc(want) for k, v in lines)
        return ok, '\\field{%s}%s' % (field, want)
    if isinstance(exp, list) and all(isinstance(x, str) for x in exp):
        m = re.search(r'^      \\list\{%s\}\{(\d+)\}\{%%\n(.*?)\n      \}$' % re.escape(field), et, re.M | re.S)
        if m:
            items = re.findall(r'^        \{(.*)\}%$', m.group(2), re.M)
            return [nfc(x) for x in items] == [nfc(x) for x in exp], '\\list{%s}' % field
    return False, ''


def check_field_value(etext, field, exp):
    """Is \\field{field}{exp} (or equivalent) in the entry?"""
    lines = field_lines(etext, field)
    if exp is None or exp == '' or exp == '0':
        if exp is None and not lines:
            return True, 'absent'
        if exp in ('0', '') and (not lines or any(k == 'false' for k, _ in lines)):
            return True, 'absent/false'
        if exp == '' and any(v == '{}' for _, v in lines):
            return True, 'empty'
    if isinstance(exp, (str, int, float)):
        e = nfc(str(exp))
        for k, v in lines:
            if k in ('field', 'strng') and v == '{%s}' % e:
                return True, '\\%s{%s}' % (k, field)
            if k == 'true' and e == '1':
                return True, '\\true{%s}' % field
        return False, 'lines=%s' % lines
    return False, 'unsupported expected %r' % (exp,)


def _static_full(case, ref):
    def f(cases, golden):
        g = open(os.path.join(CORPUS, case, 'expected', cases[case]['output_file']), 'rb').read()
        r = open(os.path.join(ROOT, ref), 'rb').read()
        return g == r, 'golden %s reference %s' % ('==' if g == r else '!=', ref)
    return f


def _static_exit(case):
    def f(cases, golden):
        m = open(os.path.join(CORPUS, case, 'expected', 'meta.toml')).read()
        return 'exit_code = 0' in m, 'meta.toml exit_code'
    return f


def _static_msg(case, msg):
    def f(cases, golden):
        msgs = golden(case)[2]
        return msg in msgs, 'messages.txt'
    return f


# Subprocess tests (BIBER_DEV_TESTS) and network tests: not traced; checked here.
STATIC = {
    'full-bbl': [
        (37, 'Full test has zero exit status', 'exit', _static_exit('full-bbl-1')),
        (38, 'Testing lossort case and sortinit for macros', 'bbl_full', _static_full('full-bbl-1', 'vendor/biber/t/tdata/full-bbl.bbl')),
        (39, 'Testing duplicate/case key warnings - 1', 'messages',
         _static_msg('full-bbl-1', "WARN - Duplicate entry key: 'F1' in file 'full-bbl.bib', skipping ...")),
        (40, 'Testing duplicate/case key warnings - 2', 'messages',
         _static_msg('full-bbl-1', "WARN - Possible typo (case mismatch) between datasource keys: 'f1' and 'F1' in file 'full-bbl.bib'")),
        (41, 'Testing duplicate/case key warnings - 3', 'messages',
         _static_msg('full-bbl-1', "WARN - Possible typo (case mismatch) between citation and datasource keys: 'C1' and 'c1' in file 'full-bbl.bib'")),
    ],
    'full-bibtex': [
        (35, 'Full test has zero exit status', 'exit', _static_exit('full-bibtex-1')),
        (38, 'Testing non-toolmode bibtex output', 'tool_full', _static_full('full-bibtex-1', 'vendor/biber/t/tdata/full-bibtex_biber.bib')),
    ],
    'full-dot': [
        (35, 'Full test has zero exit status', 'dot', None),
        (36, 'Testing dot output', 'dot', None),
    ],
    'remote-files': [
        (None, 'remote-files.t (1 test)', 'remote', None),
    ],
}
STATIC_REASON = {'dot': '.dot output is dropped (docs/compat.md, PLAN §10 #2)',
                 'remote': 'needs network (remote datasources are deferred to M4)'}


def main(argv):
    with open(os.path.join(OUT, 'cases.json')) as f:
        cases = json.load(f)
    by_prep = {}
    for name, c in cases.items():
        c['name'] = name
        for p in c['preps']:
            by_prep[(c['tfile'], p)] = name
    tfiles = argv or tc.tfiles()
    results = OrderedDict()
    summary = OrderedDict()
    bbl_cache = {}
    dumpcmp = OrderedDict()

    def golden(case):
        if case not in bbl_cache:
            c = cases[case]
            d = os.path.join(CORPUS, case, 'expected')
            p = os.path.join(d, c['output_file'] if c.get('output_file') else c['bcf'][:-4] + '.bbl')
            txt = open(p, 'rb').read().decode(out_encoding(c), errors='replace') if os.path.exists(p) else ''
            msgs = open(os.path.join(d, 'messages.txt'), encoding='utf-8').read() if os.path.exists(os.path.join(d, 'messages.txt')) else ''
            bbl_cache[case] = (Bbl(txt), txt, msgs)
        return bbl_cache[case]

    for tfile in tfiles:
        preps, asserts = tc.analyse(tfile)
        res = []
        cnt = Counter()
        for a in asserts:
            ntests = len(a['tests'])
            subj, wrapper = subject_of(a)
            sh = tc.short(subj['sub']) if subj else None
            has_exp, exp = expected_of(a, wrapper, subj)
            r = {'line': a['line'], 'tests': a['tests'], 'name': a['args'][-1] if a['args'] else None,
                 'fn': a['fn'], 'subject': subj['sub'] if subj else None}
            case = by_prep.get((tfile, a['prep'])) if a['prep'] else None
            r['case'] = case
            kind, ok, why = classify_and_check(tfile, a, subj, sh, has_exp, exp, case, golden, cases)
            r.update(kind=kind, status=ok, why=why)
            res.append(r)
            cnt[(kind, ok)] += ntests
        for i, (line, name, kind, fn) in enumerate(STATIC.get(tfile, [])):
            if fn is None:
                st, why = 'unported', STATIC_REASON[kind]
            else:
                ok, why = fn(cases, golden)
                st = 'verified' if ok else 'FAIL'
            res.append({'line': line, 'tests': [i + 1], 'name': name, 'fn': 'static', 'subject': None,
                        'case': tfile + '-1' if fn else None, 'kind': kind, 'status': st, 'why': why})
            cnt[(kind, st)] += 1
        results[tfile] = res
        summary[tfile] = cnt
        # in-process vs golden comparison
        for name, c in cases.items():
            if c['tfile'] != tfile:
                continue
            g = golden(name)[1]
            same = []
            for p in c['preps']:
                dp = os.path.join(tc.TRACE, tfile, 'bbl', '%d.bbl' % p)
                if not os.path.exists(dp):
                    same.append(None)
                    continue
                d = open(dp, encoding='utf-8', errors='replace').read()
                gd = g
                if '--sortdebug' not in c['args']:
                    d = strip_sortdebug(d)
                d = re.sub(r'\\datalist\[(\w+)\]\{[^}]*\}', r'\\datalist[\1]{}', norm_bbl(d))
                gd = re.sub(r'\\datalist\[(\w+)\]\{[^}]*\}', r'\\datalist[\1]{}', norm_bbl(gd))
                same.append(d == gd)
            dumpcmp[name] = same

    with open(os.path.join(OUT, 'fidelity.json'), 'w') as f:
        json.dump({'assertions': results, 'inprocess_bbl_identical': dumpcmp}, f, indent=1)

    tot = Counter()
    print('%-22s %6s %6s %6s %6s %s' % ('tfile', 'tests', 'verif', 'FAIL', 'other', 'kinds'))
    for t, cnt in summary.items():
        n = sum(cnt.values())
        v = sum(x for (k, s), x in cnt.items() if s == 'verified')
        fl = sum(x for (k, s), x in cnt.items() if s == 'FAIL')
        kinds = Counter()
        for (k, s), x in cnt.items():
            kinds[k + ':' + s] += x
        print('%-22s %6d %6d %6d %6d %s' % (t, n, v, fl, n - v - fl, dict(kinds)))
        tot.update(cnt)
    n = sum(tot.values())
    v = sum(x for (k, s), x in tot.items() if s == 'verified')
    fl = sum(x for (k, s), x in tot.items() if s == 'FAIL')
    print('%-22s %6d %6d %6d %6d' % ('TOTAL', n, v, fl, n - v - fl))
    ident = [x for v in dumpcmp.values() for x in v if x is not None]
    print('in-process .bbl identical to golden: %d/%d prepares' % (sum(ident), len(ident)))
    if '-v' in os.environ.get('CHECK_FLAGS', ''):
        for t, res in results.items():
            for r in res:
                if r['status'] == 'FAIL':
                    print('FAIL %s:%s %s [%s] %s' % (t, r['line'], r['case'], r['kind'], r['why'][:300]))


def _keys_check(drop_loop):
    def f(a, bbl, raw, msgs):
        sec = bbl.section(0)
        keys = sorted(k.lower() for k in sec['entries'] if not (drop_loop and k.startswith('loopkey')))
        exp = a['args'][1]
        return 'citekeys', keys == sorted(exp), 'golden keys %s' % keys
    return f


def _msgs_check(lines_fn):
    def f(a, bbl, raw, msgs):
        want = lines_fn(a)
        missing = [w for w in want if w not in msgs]
        return 'messages', not missing, 'missing in messages.txt: %s' % missing
    return f


def _loopkeys(a, bbl, raw, msgs):
    n = len([k for k in bbl.section(0)['entries'] if k.startswith('loopkey:')])
    return 'citekeys', n == 3, '%d loopkey entries' % n


XDATA_ERR = {468: "ERROR - Circular XDATA inheritance between 'lxd1:loop'<->'lxd2:loop'",
             469: "ERROR - Circular XDATA inheritance between 'lxd4:loop'<->'lxd4:loop'",
             470: "ERROR - Circular XDATA inheritance between 'loop'<->'loop:3'"}

# Assertions whose subject is not a traced Biber call, checked by hand-written rules.
MANUAL = {
    ('basic-misc', 121): _keys_check(False),
    ('basic-misc', 144): _keys_check(True),
    ('set-dynamic', 177): _keys_check(False),
    ('basic-misc', 921): _loopkeys,
    ('basic-misc', 922): lambda a, bbl, raw, msgs: (
        'entry_field', bool(re.search(r'\\entry\{loopkey:[^}]*\}.*?\\field\{note\}\{NOTEreplaced\}.*?\\endentry',
                                     raw, re.S)), 'loopkey:* entries have note=NOTEreplaced'),
    ('crossrefs', 916): _msgs_check(lambda a: a['args'][1].split('\n')),
    ('xdata', 468): _msgs_check(lambda a: [XDATA_ERR[468]]),
    ('xdata', 469): _msgs_check(lambda a: [XDATA_ERR[469]]),
    ('xdata', 470): _msgs_check(lambda a: [XDATA_ERR[470]]),
}

# Assertions that cannot hold for a fresh CLI run (they check state left over
# from an earlier prepare() of the same test).
ARTIFACTS = {
    ('basic-misc', 604): 'checks \\missing entries left in the reused output object by the first prepare(); '
                         'the configuration is \\nocite{*}, which reports no missing keys (covered by basic-misc-1)',
    ('basic-misc', 605): 'as basic-misc:604',
}


def classify_and_check(tfile, a, subj, sh, has_exp, exp, case, golden, cases):
    """Return (kind, status, why); status in verified/FAIL/internal/unported/artifact."""
    if (tfile, a['line']) in ARTIFACTS:
        return 'artifact', 'artifact', ARTIFACTS[(tfile, a['line'])]
    if (tfile, a['line']) in MANUAL and case:
        bbl, raw, msgs = golden(case)
        kind, ok, why = MANUAL[(tfile, a['line'])](a, bbl, raw, msgs)
        return kind, 'verified' if ok else 'FAIL', why
    if sh is None:
        nm = a['args'][-1] if a['args'] else ''
        if isinstance(nm, str) and nm.startswith('Validation of '):
            return 'rng_validation', 'internal', 'RNG schema validation (--validate-control/--validate-config, M4)'
        return 'other', 'unported', 'no Biber call in assertion'
    OUTPUT_ACCESSORS = ('get_output_entry', 'get_keys', 'output', 'get_output_macros', 'get_output_comments',
                        'get_sortdata_for_key')
    if sh in ('prepare_tool',) or (case is None and a['prep'] is not None):
        if sh in OUTPUT_ACCESSORS:
            return 'nocase', 'unported', 'no corpus case for this configuration'
        return 'internal', 'internal', sh
    args = subj['args']
    if case is None:
        return 'internal', 'internal', sh
    bbl, raw, msgs = golden(case)
    if not raw:
        return 'nocase', 'unported', 'golden has no .bbl'
    if cases[case].get('output_file'):
        return check_output_file(a, subj, sh, has_exp, exp, raw)
    lst = None
    if args and isinstance(args[0], dict) and args[0].get('__obj') == 'Biber::DataList':
        lst = args[0]
    renamed = cases[case].get('renames', {})

    def listname(l):
        n = l.get('name')
        return renamed.get(n, n)

    secnum = lst.get('section') if lst else 0

    if sh == 'get_output_entry':
        key = args[1]
        lobj = args[2] if len(args) > 2 and isinstance(args[2], dict) else None
        sn = args[3] if len(args) > 3 else (lobj.get('section') if lobj else 0)
        if sn is None:
            sn = 0
        sec = bbl.section(sn)
        if not has_exp:
            return 'bbl_entry', 'unported', 'no expected value'
        if exp is None:
            ok = sec is None or (key not in sec['entries'] and key not in sec['missing'] and key not in sec['aliases'])
            return 'bbl_entry', 'verified' if ok else 'FAIL', 'expect absent'
        e = norm_bbl(exp)
        if '<BDS>' in e:
            return 'bbl_entry_raw', 'internal', 'uninstantiated entry (<BDS> placeholders)'
        if sec is None:
            return 'bbl_entry', 'FAIL', 'no section %s in golden' % sn
        if lobj:
            ln = listname(lobj)
            ents = sec['lists'].get(ln, {}).get('entries', [])
            if any(x['text'] == e for x in ents):
                return 'bbl_entry', 'verified', 'in list %s' % ln
        if e in sec['text']:
            return 'bbl_entry', 'verified', 'in section text'
        g = entry_block(sec, key, listname(lobj) if lobj else None) or ''
        return 'bbl_entry', 'FAIL', diff_hint(e, g)

    if sh == 'output' and args and isinstance(args[0], dict) and args[0].get('__obj') == 'Biber::Output::bbl':
        if not isinstance(a['args'][1], str):
            return 'bbl_full', 'unported', 'no expected'
        enc = out_encoding(cases[case])
        try:
            e = a['args'][1].encode('latin-1').decode(enc)
        except (UnicodeEncodeError, UnicodeDecodeError):
            e = a['args'][1]
        ok = norm_bbl(e) == norm_bbl(raw)
        if a['fn'] == 'isnt':
            ok = not ok
        return 'bbl_full', 'verified' if ok else 'FAIL', diff_hint(norm_bbl(e), norm_bbl(raw))

    if sh == 'get_keys' and lst is not None:
        ln = listname(lst)
        sec = bbl.section(secnum)
        L = sec['lists'].get(ln, {}) if sec else {}
        got = [x['key'] for x in L.get('entries', [])]
        if not has_exp:
            return 'sort_order', 'unported', 'no expected'
        ok = got == exp or any([x['key'] for x in d] == exp for d in L.get('dups', []))
        return 'sort_order', 'verified' if ok else 'FAIL', 'list %s: golden %s' % (ln, got)

    if sh == 'get_sortdata_for_key' and lst is not None:
        ln = listname(lst)
        key = args[1]
        sec = bbl.section(secnum)
        if '--sortdebug' not in cases[case]['args']:
            return 'sortkey', 'FAIL', 'case lacks --sortdebug'
        sk = None
        for x in (sec['lists'].get(ln, {}).get('entries', []) if sec else []):
            if x['key'] == key:
                sk = x['sortkey']
        e = exp if isinstance(exp, str) else unwrap(subj['ret'])
        if isinstance(e, list):
            e = e[0]
        ok = sk is not None and nfc(sk) == nfc(e)
        return 'sortkey', 'verified' if ok else 'FAIL', 'golden %r' % sk

    if sh in ('get_unsummary', 'get_uniquelist', 'get_entryfield') or sh in LIST_FIELD:
        nl, nm = name_maps(a['calls'])
        sec = bbl.section(secnum)
        ln = listname(lst) if lst else None
        if sh in ('get_unsummary', 'get_uniquelist'):
            nlid = args[1]
            if nlid not in nl:
                return 'list_field', 'unported', 'cannot resolve namelist id'
            key, field = nl[nlid]
            et = entry_block(sec, key, ln) if sec else None
            if et is None:
                return 'list_field', 'FAIL', 'entry %s not in golden' % key
            nb = name_block(et, field)
            if nb is None:
                return 'list_field', 'FAIL', 'no \\name{%s} in %s' % (field, key)
            if sh == 'get_uniquelist':
                v = opt_val(nb['opts'], 'ul')
                ok = (v == (str(exp) if exp is not None else None))
                return 'ul', 'verified' if ok else 'FAIL', 'golden ul=%s' % v
            nid = args[2]
            if nid not in nm:
                return 'un', 'unported', 'cannot resolve name id'
            idx = nm[nid][1]
            if idx > len(nb['names']):
                return 'un', 'FAIL', 'name %d missing' % idx
            v = opt_val(nb['names'][idx - 1], 'un')
            ok = (v == (str(exp) if exp is not None else None))
            return 'un', 'verified' if ok else 'FAIL', 'golden un=%s (%s)' % (v, nb['names'][idx - 1])
        key = args[1]
        field = args[2] if sh == 'get_entryfield' else LIST_FIELD[sh]
        et = entry_block(sec, key, ln) if sec else None
        if et is None and has_exp and exp is None:
            return 'list_field', 'verified', 'vacuous: no entry %s (expected undef)' % key
        if et is None:
            return 'list_field', 'FAIL', 'entry %s not in golden' % key
        if not has_exp:
            return 'list_field', 'unported', 'no expected'
        if field == 'sortlabelalpha' and isinstance(exp, str):
            sk = None
            for x in (sec['lists'].get(ln, {}).get('entries', []) if sec else []):
                if x['key'] == key:
                    sk = x['sortkey']
            if sk is None:
                return 'list_field', 'internal', 'sortlabelalpha: no --sortdebug key'
            ok = (',%s,' % nfc(exp)) in (',%s,' % nfc(sk))
            return 'sortlabel', 'verified' if ok else 'FAIL', 'sortdebug key %r' % sk
        ok, why = check_field_value(et, field, exp)
        if not ok and field in ('sortlabelalpha', 'labelprefix'):
            return 'list_field', 'internal', 'field %s not output in .bbl' % field
        return 'list_field', 'verified' if ok else 'FAIL', '%s.%s: %s' % (key, field, why)

    if sh == 'get_field' and args and isinstance(args[0], dict) and args[0].get('__obj') == 'Biber::Entry':
        key = args[0].get('key')
        field = args[1]
        sec = bbl.section(0)
        if field == 'warnings' and isinstance(exp, list):
            ok = all(('WARN - ' + w) in msgs for w in exp)
            return 'warnings', 'verified' if ok else 'internal', 'messages.txt'
        if not has_exp:
            return 'entry_field', 'internal', 'no expected'
        et = entry_block(sec, key) if sec else None
        if et is None:   # the entry object does not know its section; try the others
            for sn, sx in bbl.sections.items():
                et = et or entry_block(sx, key)
        if et is None:
            return 'entry_field', 'internal', 'entry not in output'
        if isinstance(exp, (str, type(None))):
            ok, why = check_field_value(et, field, exp)
            if ok:
                return 'entry_field', 'verified', why
        ok, why = check_rendered(et, field, exp)
        if ok:
            return 'entry_field', 'verified', why
        return 'entry_field', 'internal', 'value not directly visible in .bbl'

    if sh == 'get_labelname_info':
        key = args[0].get('key')
        sec = bbl.section(0)
        et = entry_block(sec, key) if sec else None
        if et and has_exp:
            ok, why = check_field_value(et, 'labelnamesource', exp)
            if not ok and not field_lines(et, 'labelnamesource'):
                return 'labelname', 'internal', 'no labelnamesource in output'
            return 'labelname', 'verified' if ok else 'FAIL', why
        return 'labelname', 'internal', ''

    if sh in ('has_citekey', 'bibentry', 'entry') and has_exp:
        key = args[1]
        sn = args[0].get('number', 0) if isinstance(args[0], dict) else 0
        sec = bbl.section(sn or 0)
        present = bool(sec and (key in sec['entries']))
        if sh in ('bibentry', 'entry') and not present:
            return 'exists', 'internal', 'entry object exists internally but is not output'
        want = exp not in (None, '', '0', 0) if sh == 'has_citekey' else exp is not None
        if not isinstance(exp, (str, type(None))) and sh != 'has_citekey':
            want = True
        if a['fn'] == 'ok' and sh != 'has_citekey':
            want = True if exp is not None else False
        return 'exists', 'verified' if present == want else 'FAIL', 'present=%s want=%s' % (present, want)

    if sh == 'get_citekey_alias':
        sec = bbl.section(args[0].get('number', 0))
        got = sec['aliases'].get(args[1]) if sec else None
        ok = got == exp
        return 'alias', 'verified' if ok else 'FAIL', 'golden %s' % got

    return 'internal', 'internal', sh


def check_output_file(a, subj, sh, has_exp, exp, raw):
    """Tool mode / bibtex / biblatexml output (deferred:m4 cases)."""
    args = subj['args']
    text = nfc(raw)
    if sh == 'get_output_entry':
        key = args[1]
        if has_exp and exp is None:
            ok = not re.search(r'^@\w+\{%s,' % re.escape(nfc(key)), text, re.M | re.I)
            return 'tool_entry', 'verified' if ok else 'FAIL', 'expect absent'
        if not isinstance(exp, str):
            return 'tool_entry', 'unported', 'no expected'
        ok = nfc(exp) in text
        return 'tool_entry', 'verified' if ok else 'FAIL', '' if ok else 'entry text not found in output'
    if sh in ('get_output_macros', 'get_output_comments') and isinstance(exp, list):
        missing = [m for m in exp if nfc(m) not in text]
        return 'tool_macros', 'verified' if not missing else 'FAIL', 'missing %s' % missing
    if sh == 'get_keys' and isinstance(exp, list):
        keys = re.findall(r'^@\w+\{([^,\s]+),', text, re.M)
        if not keys:
            return 'tool_order', 'internal', 'biblatexml tool output is written in citekey order, not list order (BB-114)'
        ok = [nfc(k) for k in keys] == [nfc(k) for k in exp]
        return 'tool_order', 'verified' if ok else 'FAIL', 'golden %s' % keys
    if sh == 'output' and isinstance(a['args'][1], str):
        e = nfc(debyte(a['args'][1]))
        ok = e == text
        return 'tool_full', 'verified' if ok else 'FAIL', '' if ok else diff_hint(e, text)
    return 'internal', 'internal', sh


def diff_hint(e, g):
    el = e.splitlines()
    gl = g.splitlines()
    for i, (x, y) in enumerate(zip(el, gl)):
        if x != y:
            return 'line %d: expected %r golden %r' % (i + 1, x, y)
    return 'length differs: expected %d lines golden %d lines' % (len(el), len(gl))


if __name__ == '__main__':
    main([a for a in sys.argv[1:] if not a.startswith('-')])

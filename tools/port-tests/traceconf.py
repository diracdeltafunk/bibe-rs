"""Reconstruct the configurations biber's t/*.t run, from BiberTrace event logs.

A *configuration* is everything that determines the output of one prepare()
call: Biber->new arguments, the control file, setoption/setblxoption calls,
datalist template changes, output class.  Assertions are attached to the most
recent prepare() of the current Biber object.
"""
import json
import os
from collections import OrderedDict

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
TRACE = os.path.join(ROOT, 'tools', 'port-tests', 'out', 'trace')
TDATA = os.path.join(ROOT, 'vendor', 'biber', 't', 'tdata')
TDIR = os.path.join(ROOT, 'vendor', 'biber', 't')

# Calls which only *read* state (do not affect configuration).
MUTATORS_RESET = {
    # test-side state resets between prepare() calls; a fresh CLI run is clean
    'Biber::Entry::del_field', 'Biber::DataList::set_extraalphadata_for_key',
    'Biber::Entries::del_entry', 'Biber::Entries::del_entries',
    'Biber::Section::del_everykeys', 'Biber::Input::file::bibtex::init_cache',

}


def load_events(tfile):
    path = os.path.join(TRACE, tfile, 'events.jsonl')
    with open(path, encoding='utf-8') as f:
        return [json.loads(l) for l in f]


def short(sub):
    return sub.split('::')[-1]


class Config:
    def __init__(self):
        self.new_args = {}
        self.bcf = None
        self.opts_pre = OrderedDict()   # setoption before parse_ctrlfile
        self.opts = OrderedDict()       # setoption after parse_ctrlfile
        self.blx = []                   # (opt, val, scope, scopeval)
        self.tmplname = OrderedDict()   # list name -> sortingtemplatename
        self.tmpl = OrderedDict()       # list name -> sortingtemplate struct
        self.output = None
        self.tool = False
        self.mods = []                  # other mutators (section key manipulations, ...)
        self.resets = []                # state resets (ignored for CLI)

    def copy(self):
        c = Config()
        c.new_args = dict(self.new_args)
        c.bcf = self.bcf
        c.opts_pre = OrderedDict(self.opts_pre)
        c.opts = OrderedDict(self.opts)
        c.blx = list(self.blx)
        c.tmplname = OrderedDict(self.tmplname)
        c.tmpl = OrderedDict(self.tmpl)
        c.output = self.output
        c.tool = self.tool
        c.mods = list(self.mods)
        c.resets = list(self.resets)
        return c

    def key(self):
        # blx: last write wins per (opt, scope, scopeval)
        blx = OrderedDict()
        for (o, v, s, sv) in self.blx:
            blx[json.dumps([o, s, sv])] = v
        return json.dumps([self.new_args, self.bcf, self.opts_pre, self.opts, blx,
                           self.tmplname, self.tmpl, self.output, self.tool, self.mods],
                          sort_keys=True)

    def blx_final(self):
        blx = OrderedDict()
        for (o, v, s, sv) in self.blx:
            blx[(o, s, sv)] = v
        return blx

    def to_json(self):
        return {
            'new_args': self.new_args, 'bcf': self.bcf,
            'opts_pre': self.opts_pre, 'opts': self.opts,
            'blx': [[o, v, s, sv] for (o, s, sv), v in self.blx_final().items()],
            'tmplname': self.tmplname, 'tmpl': self.tmpl,
            'output': self.output, 'tool': self.tool, 'mods': self.mods,
        }


def analyse(tfile):
    """Return (preps, asserts).

    preps: list of dicts {prep: N, config: Config, line: L}
    asserts: list of dicts {assert event..., prep: N or None, calls: [call events]}
    """
    events = load_events(tfile)
    cur = None
    preps = []
    asserts = []
    calls = []
    cur_prep = None
    for e in events:
        if e['ev'] == 'call':
            s = e['sub']
            a = e['args']
            if s == 'Biber::new':
                cur = Config()
                kv = a[1:]
                cur.new_args = {kv[i]: kv[i + 1] for i in range(0, len(kv) - 1, 2)}
                if cur.new_args.get('tool'):
                    cur.tool = True
                cur_prep = None
            elif cur is None:
                pass
            elif s == 'Biber::parse_ctrlfile':
                cur.bcf = a[1]
            elif s == 'Biber::tool_mode_setup':
                cur.tool = True
            elif s == 'Biber::Config::setoption':
                if cur.bcf is None and not cur.tool:
                    cur.opts_pre[a[1]] = a[2]
                else:
                    cur.opts[a[1]] = a[2]
            elif s == 'Biber::Config::setblxoption':
                a = a + [None] * (6 - len(a))
                cur.blx.append((a[2], a[3], a[4], a[5]))
            elif s == 'Biber::set_output_obj':
                cur.output = a[1]['__obj']
            elif s == 'Biber::DataList::set_sortingtemplatename':
                cur.tmplname[a[0].get('name')] = a[1]
            elif s == 'Biber::DataList::set_sortingtemplate':
                cur.tmpl[a[0].get('name')] = a[1]
            elif s in ('Biber::prepare', 'Biber::prepare_tool'):
                preps.append({'prep': e['prep'], 'config': cur.copy(), 'line': e['line'],
                              'sub': s})
                cur.resets = []
                cur_prep = e['prep']
            elif s in MUTATORS_RESET:
                cur.resets.append(short(s))
            elif short(s).startswith(('set_', 'del_', 'reset_', 'add_', 'init')) and \
                    not short(s).startswith('set_output'):
                cur.mods.append([s, a[1:]])
            calls.append(e)
        elif e['ev'] == 'assert':
            ae = dict(e)
            ae['prep'] = cur_prep
            ae['calls'] = calls
            asserts.append(ae)
            calls = []
    return preps, asserts


def tfiles():
    return sorted(d for d in os.listdir(TRACE) if os.path.isdir(os.path.join(TRACE, d)))


if __name__ == '__main__':
    import sys
    for t in (sys.argv[1:] or tfiles()):
        preps, asserts = analyse(t)
        keys = OrderedDict()
        for p in preps:
            keys.setdefault(p['config'].key(), []).append(p['prep'])
        print(f"{t}: {len(preps)} prepares, {len(keys)} distinct configs, {len(asserts)} asserts")
        for k, ps in keys.items():
            n = sum(1 for a in asserts if a['prep'] in ps)
            print('   preps', ps, 'asserts', n)

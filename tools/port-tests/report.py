#!/usr/bin/env python3
"""Write tests/ported/README.md from out/fidelity.json, out/cases.json, out/replay.json."""
import json
import os
import sys
from collections import Counter, OrderedDict

sys.path.insert(0, os.path.dirname(__file__))
import check

ROOT = check.ROOT
OUT = check.OUT

NOT_PORTED_REASON = {
    'full-dot': '.dot output is dropped (PLAN §10 #2)',
    'remote-files': 'needs network; remote datasources are deferred to M4',
    'basic-misc': 'lines 604-605 assert `\\missing` entries left in the reused output object by an earlier prepare(); '
                  'not reproducible by a fresh run (the same expectation is covered by basic-misc-1)',
}

NOTES = OrderedDict([
    ('bcfvalidation', 'RNG validation of every t/tdata .bcf: unit vectors (M4, `--validate-control`).'),
    ('configfile', 'biber.conf parsing (`getoption` after reading biber-test.conf) and its RNG validation.'),
    ('utils', 'pure functions: latex_decode/encode, rangelen, parse_range, normalise_string*, map_boolean, ...'),
    ('langtags', 'BCP47 parser.'),
    ('names', 'parsename/name_to_bibtex vectors + .bbl names; config 2 is output_encoding=latin1.'),
    ('names_x', 'extended name format parser (parsename_x).'),
    ('tool-config', 'tool-mode config/datamodel queries: unit vectors only (no output assertion).'),
    ('tool', 'tool mode, bibtex output (deferred:m4).'),
    ('tool-bltxml', 'tool mode, biblatexml output (deferred:m4). The sort-order assertion is not visible: '
                    'the biblatexml writer ignores the tool sort (BB-114).'),
    ('tool-bltxml-inout', 'biblatexml in and out (deferred:m4).'),
    ('bibtex-output', 'non-tool `--output-format=bibtex` (deferred:m4). The test also sets the `tool` option, '
                      'which cannot be expressed on the CLI; all assertions hold without it.'),
    ('full-bibtex', 'subprocess test; golden == t/tdata/full-bibtex_biber.bib (deferred:m4).'),
    ('full-bbl', 'subprocess test; golden == t/tdata/full-bbl.bbl byte for byte. `--nolog` dropped to keep the .blg.'),
    ('basic-misc', 'configs 2-4 are \\nocite{*} with biber-test.conf, whose `$MAPUNIQ` keys are random (BB-113): '
                   'the goldens are nondeterministic until the harness can normalise them.'),
    ('crossrefs', '`$DATAFIELD_SETS{nobtitle}` (set directly by the test) is written into the .bcf as a datafieldset.'),
    ('encoding', '`change_ds_encoding()` maps to `--input-encoding`, `Recode->init_sets(full,full)` to '
                 '`--decodecharsset=full --output-safecharsset=full`.'),
    ('sort-case', '`set_sortingtemplate()` is a no-op in 2.22 (prepare relinks the template by name).'),
    ('sort-uc', 'as sort-case; the shorthand list is switched to the nonexistent template `global`, which '
                'falls back to the global sorting template.'),
    ('labelalpha', '`sortlabelalpha` is checked through the --sortdebug key (the template sorts on labelalpha). '
                   'Lines 63/106/141 ask for key `l1` (typo for `L1`) and pass vacuously.'),
    ('datalists', 'the .bcf declares two datalists named `lname`; both are written to the .bbl under the same name.'),
])


def main():
    fid = json.load(open(os.path.join(OUT, 'fidelity.json')))['assertions']
    cases = json.load(open(os.path.join(OUT, 'cases.json')))
    replay = json.load(open(os.path.join(OUT, 'replay.json'))) if os.path.exists(os.path.join(OUT, 'replay.json')) else {}
    ncases = Counter(v['tfile'] for v in cases.values())
    ndefer = Counter(v['tfile'] for v in cases.values() if v.get('deferred') or v.get('output_file', '').endswith(('.bib', '.bltxml')))
    vec_files = set(f[:-5] for f in os.listdir(os.path.join(ROOT, 'tests', 'ported')) if f.endswith('.json'))
    rows = []
    tot = Counter()
    for t, rs in fid.items():
        k = Counter()
        for r in rs:
            k[r['status']] += len(r['tests'])
        n = sum(k.values())
        np_ = k['unported'] + k['artifact'] + k['FAIL']
        row = (t, n, ncases[t], k['verified'], k['internal'], np_)
        rows.append(row)
        tot.update({'n': n, 'cases': ncases[t], 'v': k['verified'], 'i': k['internal'], 'np': np_,
                    'fail': k['FAIL']})
    rep = Counter(v['status'] for v in replay.values())
    kinds = Counter()
    for t, rs in fid.items():
        for r in rs:
            if r['status'] == 'verified':
                kinds[r['kind']] += len(r['tests'])
    targets = Counter()
    for t in vec_files:
        d = json.load(open(os.path.join(ROOT, 'tests', 'ported', t + '.json')))
        for v in d['vectors']:
            targets[v['target']] += len(v['tests'])

    L = []
    w = L.append
    w('# Ported biber tests (`t/*.t`)')
    w('')
    w('biber 2.22 ships 51 test files with **%d** TAP tests. They drive biber in-process (`Biber->new`, '
      '`setoption`/`setblxoption`, `parse_ctrlfile`, `prepare`) and then inspect objects or compare '
      '`.bbl` entry strings. This directory and `tests/corpus/ported/` turn them into black-box checks '
      'for bibers (docs/PLAN.md §6.2 item 2).' % tot['n'])
    w('')
    w('| | TAP tests |')
    w('|---|---:|')
    w('| covered by corpus cases (verified against the goldens by the fidelity check) | **%d** |' % tot['v'])
    w('| extracted as unit-test vectors (`tests/ported/<tfile>.json`) | **%d** |' % tot['i'])
    w('| not ported | **%d** |' % tot['np'])
    w('| total | %d |' % tot['n'])
    w('')
    w('%d corpus cases (`tests/corpus/ported/<tfile>-<n>`, tags `suite:ported`, `t:<tfile>`; %d of them '
      'are tool/bibtex/biblatexml output and also carry `deferred:m4`). Fidelity-check failures: **%d**.'
      % (tot['cases'], sum(ndefer.values()), tot['fail']))
    w('')
    w('## How it works')
    w('')
    w('Everything is regenerated by the scripts in `tools/port-tests/` (outputs in `tools/port-tests/out/`, gitignored):')
    w('')
    w('1. **Trace** (`trace.sh` + `BiberTrace.pm`): runs each `t/*.t` under oracle O3 with an instrumentation '
      'module that wraps every `Biber::*` sub and the Test::More/Test::Differences assertions. Calls made by '
      'the `.t` itself (not biber-internal ones) are logged with arguments and return values, every assertion '
      'with got/expected and its TAP numbers, and after each `prepare()` the complete in-process `.bbl` is '
      'dumped. All 1275 tests still pass under the instrumentation.')
    w('2. **Configurations** (`traceconf.py`): replays the log as a state machine. Each `prepare()` snapshots '
      'the configuration: `Biber->new` arguments, `.bcf`, `setoption`/`setblxoption` calls, datalist '
      'template changes, key-set changes. Assertions belong to the latest `prepare()`.')
    w('3. **Cases** (`gen.py`): one corpus case per distinct configuration that has assertions. '
      '`setoption` becomes a CLI flag (`--configfile` for structured options); `setblxoption` and '
      '`nosort`/`noinit`/`nolabel`/`nolabelwidthcount` become edits of a copy of the `.bcf` '
      '(`<bcf:options component="biblatex">` blocks, template elements); `set_sortingtemplatename` changes '
      'the datalist (or the global `sortingtemplatename`, which renames the list); `set_allkeys` becomes '
      '`\\nocite{*}`. `--sortdebug` is added when sort keys are asserted. State the tests set outside traced '
      'subs is handled by small per-file overrides (`OVERRIDES`, `SUBPROCESS_CASES`). Goldens: '
      '`cargo xtask golden ported/` (O1; O3 reproduces them, `golden --check --oracle o3`).')
    w('4. **Fidelity check** (`check.py`): for every assertion it finds the accessor the test checked and '
      'verifies the expected value from the `.t` against the golden of the matching case: entry text '
      '(`get_output_entry`) in the `.bbl` section/list; key order (`get_keys`) against the datalist; '
      '`get_sortdata_for_key` and `sortlabelalpha` against the `--sortdebug` lines; '
      '`get_unsummary`/`get_uniquelist` against `un=`/`ul=` in the `\\name` blocks; '
      '`get_entryfield`/`get_extra*data`/`get_labelalphadata` and `$entry->get_field` against '
      '`\\field`/`\\strng`/`\\true`/`\\list`/`\\verb`/ranges; datamodel warnings and error texts against '
      '`expected/messages.txt`; full `$outvar` outputs (encoding.t, tool modes) against the whole file. '
      'Results: `out/fidelity.json`.')
    w('5. **Translation check** (`replaycheck.py` + `replay.pl`): replays each case\'s configuration in a '
      '*fresh* in-process O3 biber (same calls as the `.t`, without the earlier `prepare()`s) and compares '
      'the whole `.bbl` with the CLI golden: %d identical, %d differ, %d replay crashes (of %d `.bbl` cases).'
      % (rep['identical'], rep['differs'], rep['replay-crash'], sum(rep.values())))
    w('   The one difference (sort-order-16) is a stale citecount from the keys the test deleted; the crashes '
      '(sort-uc-3..5) are in-process `output()` dying when no list uses the global sorting template, which a '
      'CLI run cannot reach.')
    w('6. **Unit vectors** (`vectors.py`): assertions about internals that no output shows.')
    w('7. **This README** (`report.py`).')
    w('')
    w('Regenerate: `tools/port-tests/trace.sh && cd tools/port-tests && python3 gen.py && '
      '(cd ../.. && cargo xtask golden ported/) && python3 check.py && python3 replaycheck.py && '
      'python3 vectors.py && python3 report.py`. `dumpdiff.py <case>` diffs a case against the in-process dump.')
    w('')
    w('Verified assertions by kind: ' + ', '.join('%s %d' % kv for kv in kinds.most_common()) + '.')
    w('')
    w('## Unit-test vectors')
    w('')
    w('`tests/ported/<tfile>.json` (format described in `tools/port-tests/vectors.py`). Each vector names the '
      'Perl sub, its arguments, the expected value from the `.t`, the raw in-process return value, the `.t` '
      'source line (which shows any test-side transformation such as `NFC()` or `->[0]`), the intended Rust '
      'crate/module and, for configuration-dependent internals, the corpus case that reproduces the '
      'configuration. Machine paths are written as `<BIBER_SRC>`/`<ROOT>`.')
    w('')
    w('| target crate/module | vectors (TAP tests) |')
    w('|---|---:|')
    for k, v in targets.most_common():
        w('| %s | %d |' % (k, v))
    w('')
    w('## Per test file')
    w('')
    w('| t/*.t | tests | cases | corpus-covered | unit vectors | not ported | notes |')
    w('|---|---:|---:|---:|---:|---:|---|')
    for (t, n, nc, v, i, np_) in rows:
        note = NOTES.get(t, '')
        if np_ and t in NOT_PORTED_REASON:
            note = (note + ' Not ported: ' + NOT_PORTED_REASON[t]).strip()
        w('| %s | %d | %d | %d | %d | %d | %s |' % (t, n, nc, v, i, np_, note))
    w('| **total** | **%d** | **%d** | **%d** | **%d** | **%d** | |' % (tot['n'], tot['cases'], tot['v'], tot['i'], tot['np']))
    w('')
    w('## Findings')
    w('')
    w('- **BB-114** (new, docs/biber-bugs.md): in tool mode the biblatexml writer outputs entries in citekey '
      'order and ignores the `tool` sorting template that the bibtex writer applies (tool-bltxml.t asserts '
      'the sorted list, which never reaches the file).')
    w('- `$MAPUNIQ` (BB-113) makes basic-misc-2..4 nondeterministic; the oracle runs differ only in the '
      '`loopkey:<x>:<random>` keys.')
    w('- Test-suite quirks (not biber bugs): `set_sortingtemplate()` in sort-case.t/sort-uc.t has no effect; '
      'labelalpha.t asks for key `l1` instead of `L1` three times (vacuous); basic-misc.t:604-605 assert '
      'state left over from an earlier `prepare()`; crossrefs.t and annotations.t set package globals '
      'directly; bibtex-output.t sets the `tool` option outside tool mode; the in-process tests reuse the '
      'output object, so its entries accumulate across `prepare()` calls.')
    w('')
    with open(os.path.join(ROOT, 'tests', 'ported', 'README.md'), 'w', encoding='utf-8') as f:
        f.write('\n'.join(L))


if __name__ == '__main__':
    main()

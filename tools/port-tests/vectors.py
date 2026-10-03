#!/usr/bin/env python3
"""Extract table-driven unit-test vectors for assertions about biber internals
that are not visible in any output (status "internal" in out/fidelity.json),
into tests/ported/<tfile>.json.

Each vector:
  test, line, tests (TAP numbers), name   -- where it comes from
  target   future crate/module that should consume it
  area     short name of the function/behaviour
  call     the Perl sub whose result is checked, with its arguments
  chain    the Biber calls the .t made to get there (navigation omitted)
  expected the expected value from the .t (null = undef); "assert" says how it
           is compared (eq_or_diff/is: string equality, is_deeply: structure,
           ok+is_undef: undefined, ok: true)
  ret      the value the sub returned in-process (O3), before any test-side
           transformation (NFC(), ->[0], ->{field}{year}, ...); `source` is the
           .t line, so the transformation can be read off it
  case     for configuration-dependent internals: the ported corpus case whose
           inputs reproduce the configuration, plus `entry`/`field` when known

Objects are serialised by BiberTrace: {"__obj": class, ...}; Perl regexes as
{"__regex": "(?^:...)"}; byte strings (encode_utf8) arrive as latin-1 code points.
"""
import json
import os
import sys
from collections import OrderedDict, Counter

sys.path.insert(0, os.path.dirname(__file__))
import traceconf as tc
import check

ROOT = tc.ROOT
DEST = os.path.join(ROOT, 'tests', 'ported')

TARGET = OrderedDict([
    ('Biber::LaTeX::Recode::latex_decode', ('latex-recode', 'latex_decode')),
    ('Biber::LaTeX::Recode::latex_encode', ('latex-recode', 'latex_encode')),
    ('Biber::Utils::rangelen', ('bibers-core::input::handlers (ranges)', 'rangelen')),
    ('Biber::Utils::parse_range', ('bibers-core::input::handlers (ranges)', 'parse_range')),
    ('Biber::Utils::normalise_string', ('bibers-core::process (normalise)', 'normalise_string')),
    ('Biber::Utils::normalise_string_hash', ('bibers-core::process::hashes', 'normalise_string_hash')),
    ('Biber::Utils::normalise_string_underscore', ('bibers-core::process (normalise)', 'normalise_string_underscore')),
    ('Biber::Utils::strip_noinit', ('bibers-core::process (initials)', 'strip_noinit')),
    ('Biber::Utils::map_boolean', ('bibers-core::config::blx_options', 'map_boolean')),
    ('Biber::Utils::split_xsv', ('bibers-core::input::handlers (xsv lists)', 'split_xsv')),
    ('Biber::Utils::remove_outer', ('btparse / bibers-core::input (brace stripping)', 'remove_outer')),
    ('Biber::Utils::reduce_array', ('perl-compat', 'reduce_array')),
    ('Biber::Utils::locate_data_file', ('bibers (CLI file lookup, kpsewhich)', 'locate_data_file')),
    ('Biber::Input::file::bibtex::parsename', ('btparse::split_name + bibers-core::input::handlers::names', 'parsename')),
    ('Biber::Input::file::bibtex::parsename_x', ('bibers-core::input::handlers::names (extended name format)', 'parsename_x')),
    ('Biber::Entry::Name::name_to_bibtex', ('bibers-core::output::bibtex (M4)', 'name_to_bibtex')),
    ('Biber::Entry::Name::name_to_xname', ('bibers-core::output::bibtex (M4)', 'name_to_xname')),
    ('Biber::Entry::Name::get_namepart_initial', ('bibers-core::input::handlers::names (initials)', 'namepart_initial')),
    ('Biber::Entry::Name::get_namepart', ('bibers-core::input::handlers::names', 'namepart')),
    ('Biber::Entry::Names::count', ('bibers-core::model', 'names_count')),
    ('Biber::LangTag::dump', ('bibers-core::langtag (BCP47)', 'langtag_parse')),
    ('Biber::LangTags::parse', ('bibers-core::langtag (BCP47)', 'langtag_parse')),
    ('Biber::Config::getoption', ('bibers-core::config (options, conf_file, precedence)', 'getoption')),
    ('Biber::Config::getblxoption', ('bibers-core::config::blx_options', 'getblxoption')),
    ('Biber::Entry::get_field', ('bibers-core::input::handlers / resolve (entry field values)', 'entry_field')),
    ('Biber::Entry::get_labeldate_info', ('bibers-core::process::labels', 'labeldate_info')),
    ('Biber::Entry::get_labelname_info', ('bibers-core::process::labels', 'labelname_info')),
    ('Biber::Entry::has_keyword', ('bibers-core::model', 'has_keyword')),
    ('Biber::DataList::get_visible_cite', ('bibers-core::process::visibility', 'visible_cite')),
    ('Biber::DataList::get_visible_bib', ('bibers-core::process::visibility', 'visible_bib')),
    ('Biber::DataList::get_visible_alpha', ('bibers-core::process::visibility', 'visible_alpha')),
    ('Biber::DataList::_get_uniquename', ('bibers-core::process::uniqueness', 'uniquename_counts')),
    ('Biber::DataList::get_namestring', ('bibers-core::process::uniqueness', 'namestring')),
    ('Biber::DataList::get_namestrings', ('bibers-core::process::uniqueness', 'namestrings')),
    ('Biber::DataList::get_namedisschema', ('bibers-core::process::uniqueness', 'namedisschema')),
    ('Biber::DataList::get_sortdataschema', ('bibers-core::sort', 'sortdataschema')),
    ('Biber::DataList::get_sortingtemplate', ('bibers-core::bcf::templates', 'sortingtemplate')),
    ('Biber::DataList::get_entryfield', ('bibers-core::process (per-list entry fields)', 'list_entryfield')),
    ('Biber::Internals::_liststring', ('bibers-core::process::hashes (list strings)', 'liststring')),
    ('Biber::Section::bibentry', ('bibers-core::resolve (entries pulled in by crossref)', 'section_entry_exists')),
    ('Biber::Entries::entry', ('bibers-core::input::sourcemap', 'entry_exists')),
    ('Biber::Section::get_orig_order_citekeys', ('bibers-core::model (sections)', 'orig_order_citekeys')),
    ('Biber::Output::base::get_output_section', ('bibers-core::output::bbl', 'output_section')),
    ('Biber::Output::base::get_output_head', ('bibers-core::output::bbl', 'bbl_header')),
    ('Biber::get_preamble', ('bibers-core::output::bbl', 'preamble')),
    ('Biber::Output::base::get_output_entry', ('bibers-core::output::bbl', 'raw_entry')),
    ('Biber::DataModel::get_fields_of_type', ('bibers-core::datamodel', 'fields_of_type')),
    ('Biber::DataModel::is_field_for_entrytype', ('bibers-core::datamodel', 'is_field_for_entrytype')),
    ('Biber::DataList::get_keys', ('bibers-core::sort (tool-mode list order)', 'list_keys')),
])

SKIP_CHAIN = {'new', 'parse_ctrlfile', 'set_output_obj', 'setoption', 'setblxoption', 'prepare',
              'sections', 'get_section', 'bibentries', 'datalists', 'get_list', 'get_output_obj',
              'langtags', 'get_id', 'del_field', 'set_extraalphadata_for_key', 'get_citekeys'}


def main():
    fid = json.load(open(os.path.join(check.OUT, 'fidelity.json')))['assertions']
    total = Counter()
    for tfile, results in fid.items():
        preps, asserts = tc.analyse(tfile)
        prepcfg = {p['prep']: p['config'] for p in preps}
        src = open(os.path.join(tc.TDIR, tfile + '.t'), encoding='utf-8').read().split('\n')
        vecs = []
        for a, r in zip(asserts, results):
            assert a['line'] == r['line']
            if r['status'] != 'internal':
                continue
            subj, wrapper = check.subject_of(a)
            has_exp, exp = check.expected_of(a, wrapper, subj)
            sub = subj['sub'] if subj else None
            target, area = TARGET.get(sub, ('?', sub.split('::')[-1] if sub else '?'))
            if r['kind'] == 'rng_validation':
                f = r['name'][len('Validation of '):]
                schema = 'bcf.rnc' if f.endswith('.bcf') else 'config.rnc'
                target, area = 'bibers-core::bcf / config (RNG schema validation, M4)', 'validate_' + schema[:-4]
                subj = {'sub': 'XML::LibXML::RelaxNG::validate', 'args': ['data/schemata/' + schema,
                                                                          'vendor/biber/t/' + f if '/' in f else 'vendor/biber/t/tdata/' + f],
                        'ret': ''}
                sub = subj['sub']
            chain = [{'sub': c['sub'], 'args': c['args']} for c in a['calls']
                     if tc.short(c['sub']) not in SKIP_CHAIN and c is not subj]
            v = OrderedDict()
            v['test'] = tfile + '.t'
            v['line'] = a['line']
            v['tests'] = a['tests']
            v['name'] = a['args'][-1] if a['args'] and isinstance(a['args'][-1], str) else None
            v['target'] = target
            v['area'] = area
            v['source'] = src[a['line'] - 1].strip()
            v['call'] = {'sub': sub, 'args': subj['args'] if subj else None}
            if chain:
                v['chain'] = chain
            v['assert'] = a['fn'] + ('+' + wrapper if wrapper else '')
            v['expected'] = exp if has_exp else None
            v['ret'] = check.unwrap(subj.get('ret')) if subj else None
            if r['kind'] == 'rng_validation':
                v['expected'] = ''
                v['assert'] = 'is: validation error message is empty (file is valid)'
            if r.get('case'):
                v['case'] = 'tests/corpus/ported/' + r['case']
                if subj and subj['args'] and isinstance(subj['args'][0], dict) and \
                        subj['args'][0].get('__obj') == 'Biber::Entry':
                    v['entry'] = subj['args'][0].get('key')
                    if len(subj['args']) > 1 and isinstance(subj['args'][1], str):
                        v['field'] = subj['args'][1]
            elif a.get('prep') in prepcfg:
                v['config'] = prepcfg[a['prep']].to_json()
            if r['why'] and r['why'] not in (sub, tc.short(sub or '')):
                v['note'] = r['why']
            vecs.append(v)
            total[target] += len(a['tests'])
        path = os.path.join(DEST, tfile + '.json')
        if vecs:
            txt = json.dumps({'source': 'vendor/biber/t/%s.t' % tfile,
                           'generated_by': 'tools/port-tests/vectors.py (from a BiberTrace run under oracle O3)',
                           'vectors': vecs}, indent=1, ensure_ascii=False)
            # machine-specific paths
            txt = txt.replace(os.path.join(ROOT, 'vendor', 'perl-build', 'biber-src'), '<BIBER_SRC>')
            txt = txt.replace(ROOT, '<ROOT>')
            with open(path, 'w', encoding='utf-8') as f:
                f.write(txt + '\n')
        elif os.path.exists(path):
            os.remove(path)
    for k, n in total.most_common():
        print('%4d  %s' % (n, k))
    print('%4d  TOTAL' % sum(total.values()))


if __name__ == '__main__':
    main()

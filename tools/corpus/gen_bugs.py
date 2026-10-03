#!/usr/bin/env python3
"""Generate tests/corpus/bugs/: one case per biber 2.22 bug with an observable
effect (docs/biber-bugs.md, status V or C).

Every case is defined below, inline: the LaTeX document (its .bcf is made with
one `pdflatex -draftmode` run), the data/config files, and biber's argv. The
reproducers were migrated from scratch/{input,config,semantics,sorting,output,
verify}/ (see the research reports in docs/research/).

The goldens record biber's ACTUAL (buggy) behaviour. Cases for bugs that
bibers always fixes carry the DC-NNN id from docs/compat.md §1 in `allow`.

Bugs without a case (and why) are listed in NO_CASE at the end of this file;
`--list-skipped` prints them.

Usage: python3 tools/corpus/gen_bugs.py [--list-skipped] [NAME-REGEX]
Then:  cargo xtask golden bugs/
"""

import concurrent.futures as cf
import re
import shutil
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

from corpuslib import CORPUS, latex_error, log, run_latex, write_case

SUITE = CORPUS / "bugs"


def doc(opts: str = "style=authoryear", body: str = r"\nocite{*}", preamble: str = "",
        resources: tuple[str, ...] = ("test.bib",), bib: bool = True, extra_pkgs: str = "") -> str:
    res = "".join(
        (r if r.startswith("\\") else f"\\addbibresource{{{r}}}") + "\n" for r in resources)
    return (
        "\\documentclass{article}\n"
        "\\usepackage[T1]{fontenc}\n"
        f"{extra_pkgs}"
        f"\\usepackage[backend=biber{',' if opts else ''}{opts}]{{biblatex}}\n"
        f"{preamble}"
        f"{res}"
        "\\begin{document}\n"
        f"{body}\n"
        f"{'\\printbibliography' if bib else ''}\n"
        "\\end{document}\n"
    )


@dataclass
class Case:
    name: str
    bb: str
    description: str
    source: str
    files: dict[str, str]                        # name -> content (data, configs)
    tex: str | None = None                       # LaTeX document; None = no .bcf
    stem: str = "doc"
    args: list[str] | None = None                # default: --noconf <stem>.bcf
    outputs: list[str] | None = None             # default: <stem>.bbl
    log: str | None = None
    allow: list[str] = field(default_factory=list)
    tags: list[str] = field(default_factory=list)
    compare: str | None = None
    bcf_edit: object = None                      # function(str) -> str
    allow_junk: tuple[str, ...] = ()


# --------------------------------------------------------------------------
# .bcf edits for bugs that need a hand-edited control file

def drop_driver_maps(bcf: str) -> str:
    """Remove the driver/style-level <bcf:maps> (biblatex always writes some),
    leaving only the document's user-level \\DeclareSourcemap maps."""
    out, n = re.subn(r'\s*<bcf:maps datatype="bibtex" level="(?:driver|style)">.*?</bcf:maps>',
                     "", bcf, flags=re.S)
    assert n >= 1, "no driver maps found"
    return out


def duplicate_first_datalist(bcf: str) -> str:
    m = re.search(r"\s*<bcf:datalist .*?</bcf:datalist>", bcf, flags=re.S)
    assert m, "no datalist"
    return bcf[:m.end()] + m.group(0) + bcf[m.end():]


# --------------------------------------------------------------------------
# Shared fixtures

SM_CONF_HEAD = '<?xml version="1.0" encoding="UTF-8"?>\n<config>\n  <sourcemap>\n    <maps datatype="bibtex">\n'
SM_CONF_TAIL = "    </maps>\n  </sourcemap>\n</config>\n"

DM_BIB = r"""@book{nb, title={No author}, year={1999}, isbn={0-201-13447-1}, pages={10-20}, gender={xx}, edition={0}}
@article{a2, author={A. B}, title={T}, journaltitle={J}, date={2001}, year={2001}, foo={bar}}
@weirdtype{w1, author={A. B}, title={T}, year={2001}}
@article{a3, author={A. B}, title={T}, journaltitle={J}, year={2001}, issn={1234-5678}}
"""

ANN_BIB = r"""@misc{a1, author = {John Doe and Jane Roe}, author+an = {1:family=highlight;2=corresponding; =whole}, title = {T}, title+an:alt = {="literal; x"}, keywords = {xdata=xd-keywords}}
@xdata{xd, keywords = {k1, k2}}
@misc{a2, title={T2}, author+an = {family=noitem}, author = {A B}, pages+an={=nofield}}
"""

BLTXML = """<?xml version="1.0" encoding="UTF-8"?>
<bltx:entries xmlns:bltx="http://biblatex-biber.sourceforge.net/biblatexml">
  <bltx:entry id="b1" entrytype="misc">
    <bltx:title>Title</bltx:title>
    <bltx:eprint type="arxiv" class="cs.DL">1234.5678</bltx:eprint>
    <bltx:url>http://x.org/é ü</bltx:url>
  </bltx:entry>
</bltx:entries>
"""

DATES_BIB = r"""@misc{d01, title={1}, date = {2000-01-05/2001-02}}
@misc{d02, title={2}, date = {199X}}
@misc{d03, title={3}, date = {199X-05}}
@misc{d04, title={4}, date = {2000-02-XX}}
@misc{d05, title={5}, date = {2000?}}
@misc{d16, title={16}, date = {XXXX}}
@misc{d19, title={19}, date = {2000/1999X}}
"""

CASES: list[Case] = [
    # ---------------------------------------------------------------- crashes
    Case("bb-001-uniquelist-final-crash", "BB-001",
         "uniquelist crashes when a labelname family name is literally 'final'",
         "scratch/semantics/t1 (docs/research/03 B1)",
         {"t.bib": "@book{a, author={final}, title={T1}, year={2000}}\n"
                   "@book{b, author={final and Smith, John}, title={T2}, year={2001}}\n"},
         tex=doc("style=authoryear,uniquelist=true,uniquename=true,maxcitenames=1",
                 r"\cite{a,b}", resources=("t.bib",)),
         stem="t", tags=["perl-die", "o3-differs"], allow=["DC-001"]),
    Case("bb-002-multiple-string-macros", "BB-002",
         "several macros in one @string{} -> fatal syntax error in pass 2",
         "scratch/input/runs/multistring (docs/research/01 B1)",
         {"test.bib": '@string{aa = "Alpha", bb = "Beta"}\n'
                      '@article{k, author={X Y}, title = aa # " " # bb, note = jan # " and " # dec}\n'},
         tex=doc(), allow=["DC-002"]),
    Case("bb-003-char-brace", "BB-003",
         r"\char\"7B decodes to a bare brace: fatal syntax error, next entry swallowed",
         "scratch/input/runs/charbrace (docs/research/01 B2)",
         {"test.bib": '@misc{cb1, title = {a \\char"7B b}}\n@misc{cb2, title = {ok}}\n'},
         tex=doc(), allow=["DC-003"]),
    Case("bb-004-presort-regex", "BB-004",
         "presort value interpolated into a regex: 'a(b' is fatal",
         "scratch/sorting/t7 (docs/research/04 B2)",
         {"t.bib": "@book{p1, author={Bee, A}, title={T}, presort={a(b}}\n"
                   "@book{p2, author={Cee, A}, title={U}}\n"},
         tex=doc("sorting=nyt", r"\nocite{*}", resources=("t.bib",)), stem="t",
         tags=["perl-die", "o3-differs"], allow=["DC-004"]),
    Case("bb-005-conf-noinit-crash", "BB-005",
         "biber.conf <noinit> in the documented form crashes ('Not a HASH reference')",
         "scratch/config/c_noinit.conf (docs/research/02 B1)",
         {"test.bib": "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}}\n",
          "c_noinit.conf": '<?xml version="1.0" encoding="UTF-8"?>\n<config>\n'
                           '<noinit><option value="ZZNOINIT"/></noinit>\n</config>\n'},
         tex=doc(), args=["--configfile=c_noinit.conf", "doc.bcf"], allow=["DC-005"],
         tags=["feature:configfile", "perl-die", "o3-differs"]),
    Case("bb-005-conf-nolabel-crash", "BB-005",
         "biber.conf <nolabel> in the documented form crashes ('Not a HASH reference')",
         "scratch/config/c_nolabel.conf (docs/research/02 B1)",
         {"test.bib": "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}}\n",
          "c_nolabel.conf": '<?xml version="1.0" encoding="UTF-8"?>\n<config>\n'
                            '<nolabel><option value="ZZNOLABEL"/></nolabel>\n</config>\n'},
         tex=doc("style=alphabetic"), args=["--configfile=c_nolabel.conf", "doc.bcf"],
         allow=["DC-005"], tags=["feature:configfile", "perl-die", "o3-differs"]),
    Case("bb-006-conf-sortinclusion-crash", "BB-006",
         "a single <sortinclusion> in biber.conf crashes ('Not an ARRAY reference')",
         "scratch/config/c_sortincl.conf (docs/research/02 B2)",
         {"test.bib": "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}}\n",
          "c_sortincl.conf": '<?xml version="1.0" encoding="UTF-8"?>\n<config>\n'
                             '<sortinclusion type="book"><inclusion>title</inclusion></sortinclusion>\n'
                             '</config>\n'},
         tex=doc(), args=["--configfile=c_sortincl.conf", "doc.bcf"], allow=["DC-005"],
         tags=["feature:configfile", "perl-die", "o3-differs"]),
    Case("bb-007-section-without-datasource", "BB-007",
         "citations in a section without data sources while another section has some -> crash",
         "scratch/config/t4 (docs/research/02 B11)",
         {"refs.bib": "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}}\n"
                      "@article{lamport, author={Leslie Lamport}, title={LaTeX}, journaltitle={J}, date={1994}}\n"},
         tex=doc("", "\\cite{knuth}\n\\begin{refsection}[refs.bib]\n\\cite{lamport}\n"
                     "\\printbibliography\n\\end{refsection}", resources=()),
         allow=["DC-006"]),
    Case("bb-010-collate-ignorechar-crash", "BB-010",
         "-c ignoreChar=... dies: options are applied with change(), not new()",
         "docs/research/04 B8",
         {"test.bib": "@book{a, author={Zed, A}, title={T}}\n@book{b, author={Abe, B}, title={U}}\n"},
         tex=doc("sorting=nyt"), args=["--noconf", "--collate-options=ignoreChar=-", "doc.bcf"],
         tags=["perl-die", "o3-differs"], allow=["DC-008"]),
    Case("bb-011-cjk-sortlocale", "BB-011",
         "the PAR binary dies for sortlocale ja (Unicode::Collate::CJK packed under a bogus path); "
         "O3 (source build) works",
         "scratch/sorting/t7 (docs/research/04 B7)",
         {"t.bib": "@book{j1, author={山田, 太郎}, title={本}}\n@book{j2, author={佐藤, 花子}, title={木}}\n"
                   "@book{p1, author={Bee, A}, title={T}}\n"},
         tex=doc("sorting=nyt,sortlocale=ja_JP", r"\nocite{*}", resources=("t.bib",)), stem="t",
         allow=["DC-009"], tags=["o3-differs"]),
    # ------------------------------------------------------------- data loss
    Case("bb-014-bbl-truncated-on-error", "BB-014",
         "the .bbl is truncated before processing: an error (missing .bib) destroys the previous .bbl",
         "scratch/output/bugs/trunc.bcf (docs/research/05 B16)",
         {"doc.bbl": "% A previous, good .bbl that biber should not destroy.\n\\endinput\n"},
         tex=doc(resources=("does-not-exist.bib",)), allow=["DC-012"], allow_junk=("doc.bbl",)),
    # -------------------------------------------------------- silent output
    Case("bb-015-bbl-comma-cleanup", "BB-015",
         r"',}}' in field data is rewritten to '}}': title={Foo {bar,}} loses its comma",
         "scratch/output/bugs/t1.bib (docs/research/05 B1)",
         {"test.bib": "@article{comma, author={Roe, Rich}, title={Foo {bar,}}, journaltitle={J}, date={2001}}\n"},
         tex=doc()),
    Case("bb-016-rangelen-zero-digit", "BB-016",
         r"rangelen treats a 0 digit as missing: pages={105-10} -> \range{pages}{11}",
         "scratch/output/bugs/t1.bib (docs/research/05 B3)",
         {"test.bib": "@article{range1, author={Doe, John}, title={Range}, journaltitle={J}, date={2000}, pages={105-10}}\n"
                      "@article{range2, author={Doe, Jane}, title={Range2}, journaltitle={J}, date={2000}, pages={135-38}}\n"},
         tex=doc()),
    Case("bb-017-sortkey-eval", "BB-017",
         "sort keys built by string-eval: a trailing backslash gives an undef key that sorts first",
         "scratch/verify/w (docs/research/04 B1)",
         {"w.bib": "@book{a, author={Smith, A}, title={Alpha}, year=2000}\n"
                   "@book{b, author={Smith, A}, title={The Foo\\\\Bar}, year=2000}\n"
                   "@book{c, author={Smith, A}, title={Middle}, year=2000}\n"},
         tex=doc("style=authoryear,sorting=nyt", resources=("w.bib",)), stem="w"),
    Case("bb-018-filter-readds-sets", "BB-018",
         "a filtered list re-adds every set and set member if any set passes the filter",
         "scratch/semantics/filt (docs/research/02 B14, 03 B2)",
         {"t.bib": "@set{S1, entryset={a,b}, shorthand={SH1}}\n@set{S2, entryset={c,d}}\n"
                   "@book{a, author={Alpha, A.}, title={TA}, year={2000}}\n"
                   "@book{b, author={Beta, B.}, title={TB}, year={2001}}\n"
                   "@book{c, author={Gamma, C.}, title={TC}, year={2002}}\n"
                   "@book{d, author={Delta, D.}, title={TD}, year={2003}}\n"
                   "@book{e, author={Eps, E.}, title={TE}, year={2003}}\n"},
         tex=doc("style=authoryear", "\\cite{S1,S2,e}\n\\printbiblist{shorthand}",
                 resources=("t.bib",)), stem="t"),
    Case("bb-019-per-name-option-leak-labelalpha", "BB-019",
         "per-name useprefix leaks to following names (labelalpha depends on name order)",
         "scratch/semantics/leak (docs/research/03 B5)",
         {"t.bib": "@book{a, author={family=Smith, given=John, prefix=von, useprefix=true and family=Jones, given=Bob, prefix=van}, title={T}, year={2000}}\n"
                   "@book{b, author={family=Jones, given=Bob, prefix=van and family=Smith, given=John, prefix=von, useprefix=true}, title={T}, year={2000}}\n"},
         tex=doc("style=alphabetic,maxalphanames=3", r"\cite{a,b}", resources=("t.bib",)), stem="t"),
    Case("bb-019-per-name-option-leak-sorting", "BB-019",
         "per-name useprefix leaks to following names in the sort key",
         "scratch/sorting/t8 (docs/research/04 B9)",
         {"t.bib": "@book{n1, author={family=Alpha, given=Al, prefix=von, useprefix=true and family=Zulu, given=Zed, prefix=van}, title={T}}\n"
                   "@book{n2, author={family=Zulu, given=Zed, prefix=van and family=Alpha, given=Al, prefix=von, useprefix=true}, title={T}}\n"},
         tex=doc("sorting=nyt", resources=("t.bib",)), stem="t",
         args=["--noconf", "--sortdebug", "t.bcf"]),
    Case("bb-020-uniquename-false-skips-entry", "BB-020",
         "a per-name uniquename=false drops the whole entry from uniquename processing",
         "scratch/semantics/unleak (docs/research/03 B5b)",
         {"t.bib": "@book{a, author={Smith, John and Doe, Jane}, title={T1}, year={2000}}\n"
                   "@book{b, author={Smith, Bob and family=Doe, given=Jim, uniquename=false}, title={T2}, year={2001}}\n"},
         tex=doc("style=authoryear,uniquename=init,maxcitenames=3", r"\cite{a,b}",
                 resources=("t.bib",)), stem="t"),
    Case("bb-021-entry-scoped-options-ignored", "BB-021",
         "per-entry labelalpha=false / labeldateparts=false are ignored",
         "scratch/semantics/peropt (docs/research/03 B6)",
         {"t.bib": "@book{a, author={Smith, John}, title={T1}, year={2000}, options={labelalpha=false}}\n"
                   "@book{b, author={Smith, John}, title={T2}, year={2000}}\n"
                   "@book{c, author={Jones, Bob}, title={T3}, year={2001}, options={labeldateparts=false}}\n"
                   "@book{d, author={Jones, Bob}, title={T4}, year={2001}}\n"},
         tex=doc("style=alphabetic,labeldateparts=true", r"\cite{a,b,c,d}", resources=("t.bib",)),
         stem="t"),
    Case("bb-022-crossref-date-meta", "BB-022",
         "a child with its own date inherits the parent's datecirca",
         "scratch/semantics/cr1 (docs/research/03 B3)",
         {"t.bib": "@mvbook{P, title={Parent}, date={1999~}, origdate={1990-05}, editor={Ed, Ed}}\n"
                   "@book{C, crossref={P}, title={Child}, date={2000}, author={Au, Au}}\n"},
         tex=doc("style=authoryear", r"\cite{C}", resources=("t.bib",)), stem="t"),
    Case("bb-023-crossref-datepart-mix", "BB-023",
         "child origdate=2005 gets the parent's origmonth (only the main datesplit is checked)",
         "scratch/semantics/cr1 (docs/research/03 B4)",
         {"t.bib": "@mvbook{P, title={Parent}, date={1999~}, origdate={1990-05}, editor={Ed, Ed}}\n"
                   "@book{D, crossref={P}, title={Child2}, origdate={2005}, author={Bu, Bu}}\n"},
         tex=doc("style=authoryear", r"\cite{D}", resources=("t.bib",)), stem="t"),
    Case("bb-024-xdata-name-index", "BB-024",
         "granular XDATA names get 0-based indices, shifting uniquename visibility",
         "scratch/semantics/xdidx vs xdidx2 (docs/research/03 B8)",
         {"t.bib": "@xdata{X, author={Smith, John and Brown, Alice and Jones, Jim}}\n"
                   "@book{A, author={xdata=X-author}, title={TA}, year={2000}}\n"
                   "@book{B, author={Brown, Bill and Doe, Dan and Roe, Ray}, title={TB}, year={2001}}\n"},
         tex=doc("style=authoryear,uniquename=init,maxcitenames=1", r"\cite{A,B}",
                 resources=("t.bib",)), stem="t"),
    Case("bb-025-xdata-rerefs", "BB-025",
         "an XDATA entry's own granular refs are re-resolved per referrer: names duplicated",
         "scratch/semantics/xdtwice (docs/research/03 B9)",
         {"t.bib": "@xdata{Y, author={Alpha, Ann and Beta, Bob}}\n"
                   "@xdata{X, author={xdata=Y-author and Doe, Jim}, publisher={Pub}}\n"
                   "@book{A1, xdata={X}, title={T1}, year={2000}}\n"
                   "@book{A2, xdata={X}, title={T2}, year={2001}}\n"
                   "@book{A3, xdata={X}, title={T3}, year={2002}}\n"},
         tex=doc("style=authoryear", r"\cite{A1,A2,A3}", resources=("t.bib",)), stem="t"),
    Case("bb-026-shared-namelists", "BB-026",
         "inherited name lists share per-list state with the parent: identical children differ",
         "scratch/semantics/sharednl (docs/research/03 B10)",
         {"t.bib": "@collection{P, editor={Alpha, Ann and Beta, Bob and Gamma, Gil}, title={Coll}, year={2000}}\n"
                   "@incollection{C, crossref={P}, title={Chap}, author={Zed, Zoe}, options={maxcitenames=1}}\n"
                   "@collection{Q, editor={Alpha, Ann and Beta, Bob and Gamma, Gil}, title={Coll2}, year={2000}}\n"
                   "@incollection{D, title={Chap2}, author={Zed, Zoe}, options={maxcitenames=1}, editor={Alpha, Ann and Beta, Bob and Gamma, Gil}}\n"},
         tex=doc("style=authoryear,maxcitenames=3", r"\cite{C,P,D,Q}", resources=("t.bib",)),
         stem="t"),
    Case("bb-027-workuniq-skipbib-token", "BB-027",
         "work uniqueness skips only the literal token 'skipbib' (not skipbib=true, dataonly)",
         "scratch/semantics/st_skipbib, st_skipbib_true, st_dataonly (docs/research/03 B11)",
         {"t.bib": "@book{a, author={Smith, John}, title={T1}, year={2000}}\n"
                   "@book{b, author={Smith, John}, title={T2}, year={2001}, options={skipbib}}\n"
                   "@book{c, author={Jones, Bob}, title={T3}, year={2000}}\n"
                   "@book{d, author={Jones, Bob}, title={T4}, year={2001}, options={skipbib=true}}\n"
                   "@book{e, author={Brown, Al}, title={T5}, year={2000}}\n"
                   "@book{f, author={Brown, Al}, title={T6}, year={2001}, options={dataonly}}\n"},
         tex=doc("style=authoryear,singletitle=true", r"\cite{a,b,c,d,e,f}", resources=("t.bib",)),
         stem="t"),
    Case("bb-028-datefield-helper-mutation", "BB-028",
         "custom date field ending in 'date' (updatedate): only the first entry gets circa flags",
         "scratch/output/bugs/t4 (docs/research/05 B4)",
         {"test.bib": "@misc{u1, author={Aa, A}, title={One}, date={2000}, updatedate={2010~}, urldate={2011~}}\n"
                      "@misc{u2, author={Bb, B}, title={Two}, date={2001}, updatedate={2012~}, urldate={2013~}}\n"
                      "@misc{u3, author={Cc, C}, title={Three}, date={2002}, updatedate={2014~}, urldate={2015~}}\n",
          "bb028.dbx": "\\DeclareDatamodelFields[type=field,datatype=date,skipout]{updatedate}\n"
                       "\\DeclareDatamodelEntryfields{updatedate}\n"},
         tex=doc("style=authoryear,datamodel=bb028")),
    Case("bb-029-textdollar-decoding", "BB-029",
         r"\textdollar/\textpercent/\textampersand decode to raw $ % &; '\textdollar{}5' -> '${}5'",
         "scratch/input/runs/dec1, dec2 (docs/research/01 B4)",
         {"test.bib": "@misc{p1, author = {A B}, title = {50\\textpercent{} off \\textampersand\\ more \\textdollar 5}}\n"
                      "@misc{d1, title = {\\textdollar{}5 and \\textdollar 6 and \\ss{}x and \\pounds{}7 and \\dag{}x}}\n"},
         tex=doc()),
    Case("bb-030-smap-entrytarget-source", "BB-030",
         "map_field_source -> map_field_target with map_entrytarget reads the current entry",
         "scratch/input/smap/sm3.conf (docs/research/01 B5)",
         {"test.bib": "@misc{s1, title = {ORIGTITLE}}\n",
          "sm3.conf": SM_CONF_HEAD +
          '      <map>\n'
          '        <map_step map_entry_new="newk" map_entry_newtype="misc" map_entry_nocite="1"/>\n'
          '        <map_step map_entrytarget="newk" map_field_set="title" map_field_value="NEWTITLE"/>\n'
          '        <map_step map_entrytarget="newk" map_field_source="title" map_field_target="note"/>\n'
          '      </map>\n' + SM_CONF_TAIL},
         tex=doc(), args=["--configfile=sm3.conf", "doc.bcf"], tags=["feature:sourcemap"]),
    Case("bb-031-starnocited", "BB-031",
         "map_entrykey_starnocited applies to every entry when there is no \\nocite{*}",
         "scratch/input/runs/star (docs/research/01 B6)",
         {"test.bib": "@misc{a, title={T}}\n"},
         tex=doc("", r"\cite{a}",
                 preamble="\\DeclareSourcemap{\\maps[datatype=bibtex]{\n"
                          "  \\map{\\step[starnocited=true, fieldset=usera, fieldvalue={STAR}]}\n}}\n"),
         tags=["feature:sourcemap"]),
    Case("bb-032-smap-captures", "BB-032",
         "sourcemap captures: $0 is the last capture, $1 after notmatch is '1', named captures",
         "scratch/input/smap/sm2.conf (docs/research/01 B9)",
         {"test.bib": "@misc{s1, title={A big one}, note={Hello}, series={Quux}, location={London}, "
                      "edition={ed}, publisher={abc}}\n",
          "sm2.conf": SM_CONF_HEAD +
          '      <map><map_step map_field_source="title" map_match="(?&lt;rest&gt;big)" map_replace="[$+{rest}]"/></map>\n'
          '      <map><map_step map_field_source="note" map_match="(\\w)(\\w+)" map_replace="\\L$1$2\\E!"/></map>\n'
          '      <map><map_step map_field_source="series" map_match="(?&lt;x&gt;Q)" map_replace="[$1]"/></map>\n'
          '      <map>\n'
          '        <map_step map_field_source="location" map_match="(o)" />\n'
          '        <map_step map_field_set="addendum" map_field_value="[$1|$2|$3|$0]"/>\n'
          '      </map>\n'
          '      <map>\n'
          '        <map_step map_field_source="edition" map_notmatch="zzz" />\n'
          '        <map_step map_field_set="usera" map_field_value="[$1]"/>\n'
          '      </map>\n'
          '      <map>\n'
          '        <map_step map_field_source="publisher" map_match="b" map_final="1"/>\n'
          '        <map_step map_field_set="userb" map_field_value="publisher-had-b"/>\n'
          '      </map>\n' + SM_CONF_TAIL},
         tex=doc(), args=["--configfile=sm2.conf", "doc.bcf"], tags=["feature:sourcemap"]),
    Case("bb-033-smap-replace-perl-eval", "BB-033",
         "sourcemap replace strings are evaluated as Perl; any error silently yields (?^u:...)",
         "scratch/input/smap/sm.conf (docs/research/01 B8)",
         {"test.bib": "@misc{s1, title={Big Title here}, note={x12y}, publisher={ab}, location={london}, "
                      "edition={11}, howpublished={\u263a}, organization={xABCx}, type={t}}\n",
          "sm.conf": SM_CONF_HEAD +
          '      <map><map_step map_field_source="title" map_match="(\\p{Lu})(\\w+)\\s+(?&lt;rest&gt;.*)" map_replace="[$3]\\L$1$2\\E+$+{rest}"/></map>\n'
          '      <map><map_step map_field_source="note" map_match="(?&lt;=x)(\\d+)(?=y)" map_replace="&lt;$1&gt;"/></map>\n'
          '      <map><map_step map_field_source="publisher" map_match="(a)(b)?" map_replace="$1&quot;"/></map>\n'
          '      <map><map_step map_field_source="location" map_match="(\\w+)" map_replace="@{[ uc $1 ]}"/></map>\n'
          '      <map><map_step map_field_source="edition" map_match="(\\d)\\1" map_replace="dbl"/></map>\n'
          '      <map><map_step map_field_source="howpublished" map_match="\\x{263A}|\\N{U+263B}" map_replace="smile"/></map>\n'
          '      <map><map_step map_field_source="organization" map_match="(?i)ABC" map_replace="${\\ scalar reverse \'olleh\'}"/></map>\n'
          '      <map><map_step map_field_source="type" map_match="t" map_replace="\\$1 \\u\\LfOO $&amp;"/></map>\n'
          + SM_CONF_TAIL},
         tex=doc(), args=["--configfile=sm.conf", "doc.bcf"], allow=["DC-023"],
         tags=["feature:sourcemap"]),
    Case("bb-034-mapuniq-outside-foreach", "BB-034",
         "$MAPUNIQ/$MAPLOOP stay literal outside map_foreach",
         "scratch/input/smap/sm.conf, sm3.conf (docs/research/01 B7)",
         {"test.bib": "@misc{s1, title={T}, series={Qx}}\n",
          "sm.conf": SM_CONF_HEAD +
          '      <map>\n'
          '        <map_step map_field_source="series" map_match="^(.)(.)"/>\n'
          '        <map_step map_field_set="addendum" map_field_value="$2-$1-$MAPUNIQ"/>\n'
          '      </map>\n'
          '      <map><map_step map_field_set="usera" map_field_value="u-$MAPLOOP"/></map>\n'
          + SM_CONF_TAIL},
         tex=doc(), args=["--configfile=sm.conf", "doc.bcf"], tags=["feature:sourcemap"]),
    Case("bb-035-xsv-granular-xdata", "BB-035",
         "granular XDATA refs in xsv fields (keywords={xdata=xd-keywords}) are output literally",
         "scratch/input/runs/ann (docs/research/01 B10)",
         {"test.bib": ANN_BIB}, tex=doc()),
    Case("bb-036-annotation-literal-split", "BB-036",
         "annotations split on ';' before literal handling: =\"literal; x\" loses its literal flag",
         "scratch/input/runs/ann (docs/research/01 B11)",
         {"test.bib": ANN_BIB}, tex=doc()),
    Case("bb-037-unspecified-dates", "BB-037",
         "unsupported ISO 8601-2 unspecified dates (199X-05, XXXX) silently become empty open years",
         "scratch/input/runs/dates (docs/research/01 B12)",
         {"test.bib": DATES_BIB}, tex=doc()),
    Case("bb-038-xname-braces-lost", "BB-038",
         "extended name format family={Barnes Noble} loses its protecting braces",
         "scratch/input/runs/xnames (docs/research/01 B13)",
         {"test.bib": "@misc{x1, title={1}, author = {given=Ann, family={Barnes Noble} and family=Smith, given=John, prefix=van der, useprefix=true}}\n"
                      "@misc{x2, title={2}, author = {{Barnes Noble}, Ann}}\n"},
         tex=doc()),
    Case("bb-039-range-parser", "BB-039",
         "range parser: '-5' and '-' malformed, {12-15} split inside braces, '12~' keeps a space",
         "scratch/input/runs/ranges (docs/research/01 B3)",
         {"test.bib": "@misc{r1, title={a}, pages = {-5}}\n@misc{r2, title={b}, pages = {5-}}\n"
                      "@misc{r3, title={c}, pages = {-}}\n@misc{r4, title={d}, pages = {12~--~15, 17; 20\u201322}}\n"
                      "@misc{r5, title={e}, pages = {M-1--M-4}}\n@misc{r6, title={f}, pages = {{12-15}}}\n"
                      "@misc{r7, title={g}, pages = {xii--xv}}\n@misc{r8, title={h}, pages = {12~}}\n"
                      "@misc{r9, title={i}, pages = {--}}\n"},
         tex=doc()),
    Case("bb-040-preamble-dropped", "BB-040",
         "datasource reading stops once all cited keys are found: later @preamble dropped",
         "scratch/input/runs/pre (docs/research/01 B18)",
         {"test.bib": '@preamble{"\\\\def\\\\one{1}"}\n@article{a, author={A B}, title={T}}\n',
          "test2.bib": '@preamble{"\\\\def\\\\two{2}"}\n@article{b, author={C D}, title={T2}}\n'},
         tex=doc("style=authoryear", r"\cite{a}", resources=("test.bib", "test2.bib"))),
    Case("bb-041-name-commas", "BB-041",
         "comma sanity checks skipped for names starting with '{' and ending with '}'",
         "scratch/input/runs/commas (docs/research/01 B19)",
         {"test.bib": "@misc{m1, title={1}, author = {{Doe}, J, K, {L}}}\n"
                      "@misc{m2, title={2}, author = {{Doe},, {L}}}\n"},
         tex=doc()),
    Case("bb-042-sort-column-type", "BB-042",
         "labelyear sort column typed as string: -50, 1000, 2000, 80, 999",
         "scratch/sorting/t9 (docs/research/04 B10)",
         {"t.bib": "@book{y1, title={T}, date={0999}}\n@book{y2, title={T}, date={1000}}\n"
                   "@book{y3, title={T}, date={-0050}}\n@book{y4, title={T}, date={2000}}\n"
                   "@book{y5, title={T}, date={0080}}\n"},
         tex=doc("sorting=ly,style=authoryear", "\\nocite{*}\n\\printbibliography\n"
                 "\\newrefcontext[sorting=yr]",
                 preamble="\\DeclareSortingTemplate{ly}{\\sort{\\field{labelyear}}}\n"
                          "\\DeclareSortingTemplate{yr}{\\sort{\\field{year}}}\n",
                 resources=("t.bib",)), stem="t"),
    Case("bb-043-sortlocale-vs-set-locale", "BB-043",
         "--sortlocale does not override a per-sort-set locale (contrary to the manual)",
         "scratch/sorting/t3 (docs/research/04 B4)",
         {"t.bib": "@book{s1, title={Apple}}\n@book{s2, title={\u00c4pple}}\n@book{s3, title={Zebra}}\n"},
         tex=doc("sorting=tl", preamble="\\DeclareSortingTemplate{tl}{\n"
                                        "  \\sort[locale=swedish]{\\field{title}}\n}\n",
                 resources=("t.bib",)), stem="t",
         args=["--noconf", "--sortlocale=en_US", "t.bcf"]),
    Case("bb-044-sortinithash-locale", "BB-044",
         "sortinithash ignores --sortlocale: Ä sorts after Z (sv) but gets A's hash",
         "scratch/sorting/t4 (docs/research/04 B6)",
         {"t.bib": "@book{s1, title={Apple}}\n@book{s2, title={\u00c4pple}}\n@book{s3, title={Zebra}}\n"},
         tex=doc("sorting=nyt", resources=("t.bib",)), stem="t",
         args=["--noconf", "--sortlocale=sv_SE", "t.bcf"]),
    Case("bb-045-collate-options-doc-syntax", "BB-045",
         "documented --collate-options \"variable => 'shifted'\" syntax breaks per-locale columns: unsorted",
         "scratch/sorting/t3 (docs/research/04 B3)",
         {"t.bib": "@book{u1, title={apple}}\n@book{u4, title={\u00e4pple}}\n@book{u6, title={zebra}}\n"
                   "@book{u7, title={Zebra}}\n@book{u10, title={apple pie}}\n"},
         tex=doc("sorting=tl", preamble="\\DeclareSortingTemplate{tl}{\n"
                                        "  \\sort[locale=swedish,direction=descending]{\\field{title}}\n}\n",
                 resources=("t.bib",)), stem="t",
         args=["--noconf", "--collate-options=variable => 'shifted'", "t.bcf"]),
    Case("bb-046-safechars-verbatim", "BB-046",
         "--output-safechars is applied to verbatim fields (urlraw) and keys",
         "scratch/output/bugs/t1.bib (docs/research/05 B15)",
         {"test.bib": "@online{url1, author={Ux, Ann}, title={Url}, url={http://example.org/caf\u00e9?a=1}, date={2003}}\n"},
         tex=doc(), args=["--noconf", "--output-safechars", "doc.bcf"],
         tags=["deferred:DC-034"]),
    Case("bb-049-empty-section-no-refsection", "BB-049",
         "a refsection with no found entries emits no \\refsection block, losing its \\missing lines",
         "scratch/output/bugs/miss.tex (docs/research/05 B22)",
         {"alias.bib": "@book{x, author={A, B}, title={T}}\n"},
         tex=doc("", r"\nocite{nonexistent}", resources=("alias.bib",), bib=False)),
    Case("bb-050-bltxml-eprint", "BB-050",
         "biblatexml eprint type/class attributes are ignored (misplaced parenthesis)",
         "scratch/input/runs/bltx (docs/research/01 B16)",
         {"test.bltxml": BLTXML},
         tex=doc(resources=("\\addbibresource[datatype=biblatexml]{test.bltxml}",)),
         tags=["feature:biblatexml", "deferred:DC-031"]),
    Case("bb-051-bltxml-uri-escaping", "BB-051",
         "biblatexml URIs are percent-escaped at input from NFD (e%CC%81), urlraw escaped too",
         "scratch/input/runs/bltx vs runs/url (docs/research/01 B17)",
         {"test.bltxml": BLTXML,
          "test.bib": "@misc{u1, title={T}, url = {http://x.org/\u00e9 \u00fc}, urls = {http://a.org/\u00e9 and http://b.org/\u00fc}}\n"},
         tex=doc(resources=("\\addbibresource[datatype=biblatexml]{test.bltxml}", "test.bib")),
         tags=["feature:biblatexml", "deferred:DC-031"]),
    Case("bb-052-conf-sourcemaps-dropped", "BB-052",
         "biber.conf sourcemaps are dropped when the .bcf has no driver/style maps (hand-edited .bcf)",
         "scratch/config/c_map.conf (docs/research/02 B13)",
         {"test.bib": "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}}\n",
          "c_map.conf": SM_CONF_HEAD +
          '      <map><map_step map_field_set="note" map_field_value="FROMCONF"/></map>\n'
          + SM_CONF_TAIL},
         tex=doc(preamble="\\DeclareSourcemap{\\maps[datatype=bibtex]{\n"
                          "  \\map{\\step[fieldset=addendum, fieldvalue={FROMDOC}]}\n}}\n"),
         args=["--configfile=c_map.conf", "doc.bcf"], bcf_edit=drop_driver_maps,
         tags=["feature:configfile", "feature:sourcemap"]),
    Case("bb-054-duplicate-datalists", "BB-054",
         "duplicate <bcf:datalist>s are not de-duplicated (hand-edited .bcf)",
         "scratch/config/t5dup.bcf (docs/research/02 B12)",
         {"refs.bib": "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}}\n"
                      "@article{lamport, author={Leslie Lamport}, title={LaTeX}, journaltitle={J}, date={1994}}\n"},
         tex=doc("", r"\cite{knuth,lamport}", resources=("refs.bib",)),
         bcf_edit=duplicate_first_datalist),
    Case("bb-055-zero-is-empty-sorting", "BB-055",
         "year={0}, date={0000} and volume={0} are treated as missing for sorting",
         "scratch/sorting/t6 (docs/research/04 B5)",
         {"t.bib": "@book{d1, author={A, A}, title={T}, date={-0050}}\n"
                   "@book{d5, author={A, A}, title={T}, year={2000}}\n"
                   "@book{d11, author={A, A}, title={T}}\n"
                   "@book{d12, author={A, A}, title={T}, year={0}}\n"
                   "@book{d13, author={A, A}, title={T}, date={0000}}\n"
                   "@book{d16, author={A, A}, title={T}, volume={3}, year={2000}}\n"
                   "@book{d17, author={A, A}, title={T}, volume={0}, year={2000}}\n"},
         tex=doc("sorting=ymdnt", preamble="\\DeclareSortingTemplate{ymdnt}{\n"
                 "  \\sort{\\field{presort}}\n  \\sort{\\field{sortyear}\\field{year}}\n"
                 "  \\sort{\\field{month}}\n  \\sort{\\field{day}}\n  \\sort{\\field{volume}}\n"
                 "  \\sort{\\citeorder}\n}\n", resources=("t.bib",)), stem="t"),
    # -------------------------------------------------------- nondeterminism
    Case("bb-060-per-name-options-order", "BB-060",
         "per-name/per-namelist options are emitted in hash order (normalised by the harness)",
         "scratch/output/bugs/t1.bib, t2.bib (docs/research/05 B2)",
         {"test.bib": "@article{pno, author={given=John, family=Smith, prefix=van, useprefix=true, giveninits=true, terseinits=false, sortingnamekeytemplatename=global, familyinits=false}, title={Opts}, journaltitle={J}, date={2002}}\n"
                      "@book{nlo, author={useprefix=true and giveninits=true and terseinits=true and van Doe, John}, title={NL}, date={2002}}\n"},
         tex=doc(), allow=["DC-013"]),
    Case("bb-061-mincrossrefs-promotion-order", "BB-061",
         "mincrossrefs-promoted parents are added in hash order (visible with sorting=none)",
         "scratch/semantics/nd1 (docs/research/03 B7)",
         {"t.bib": "".join(
             f"@mvbook{{P{i}, title={{Parent {i}}}, year={{1990}}}}\n"
             f"@book{{c{i}a, crossref={{P{i}}}, title={{A{i}}}, author={{X, Y}}}}\n"
             f"@book{{c{i}b, crossref={{P{i}}}, title={{B{i}}}, author={{X, Y}}}}\n" for i in range(1, 7))},
         tex=doc("sorting=none", "\\cite{" + ",".join(f"c{i}{x}" for i in range(1, 7) for x in "ab") + "}",
                 resources=("t.bib",)), stem="t", allow=["DC-014"], compare="unordered-lines"),
    Case("bb-062-dynamic-sets-nocite-order", "BB-062",
         "dynamic set keys are added in hash order under \\nocite{*} (sorting=none)",
         "scratch/semantics/nd2 (docs/research/03 B7b)",
         {"t.bib": "".join(
             f"@book{{m{i}a, author={{A{i}, B}}, title={{T{i}}}, year={{2000}}}}\n"
             f"@book{{m{i}b, author={{C{i}, D}}, title={{U{i}}}, year={{2000}}}}\n" for i in range(1, 7))},
         tex=doc("sorting=none", preamble="".join(
             f"\\defbibentryset{{S{i}}}{{m{i}a,m{i}b}}" for i in range(1, 7)) + "\n",
             resources=("t.bib",)), stem="t", allow=["DC-015"], compare="unordered-lines"),
    Case("bb-065-warning-order", "BB-065",
         "warning order varies between runs with -V (the harness compares messages as a sorted multiset)",
         "scratch/config/t3 + dm.bib (docs/research/02 §6)",
         {"dm.bib": DM_BIB}, tex=doc("style=alphabetic", resources=("dm.bib",)),
         args=["--noconf", "--validate-datamodel", "doc.bcf"], allow=["DC-016"]),
    # ------------------------------------------------------ CLI, config, logs
    Case("bb-066-missing-configfile-ignored", "BB-066",
         "--configfile pointing to a non-existent file is silently ignored",
         "docs/research/02 B3",
         {"test.bib": "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}}\n"},
         tex=doc(), args=["--configfile=does-not-exist.conf", "doc.bcf"],
         tags=["feature:configfile"]),
    Case("bb-067-dieondatamodel-no-early-exit", "BB-067",
         "--dieondatamodel never exits early: all violations logged, .bbl written, exit 2",
         "scratch/config/t3 + dm.bib (docs/research/02 B4)",
         {"dm.bib": DM_BIB}, tex=doc("style=alphabetic", resources=("dm.bib",)),
         args=["--noconf", "--validate-datamodel", "--dieondatamodel", "doc.bcf"]),
    Case("bb-069-stdout-dash-file", "BB-069",
         "-O - (stdout) still creates an empty file literally named '-'",
         "scratch/config (docs/research/02 B8)",
         {"test.bib": "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}}\n"},
         tex=doc(), args=["--noconf", "-O", "-", "doc.bcf"], outputs=["-", "doc.bbl"],
         log="doc.blg"),
    Case("bb-070-no-datasources", "BB-070",
         "'No data sources defined!' is never shown; exit 0 with an empty .bbl",
         "scratch/config/t2 (docs/research/02 B9)",
         {}, tex=doc("", r"\cite{knuth}", resources=())),
    Case("bb-071-missing-control-file", "BB-071",
         "missing .bcf: the latexmk-friendly 'Cannot find control file' message is unreachable",
         "docs/research/02 B10",
         {}, tex=None, args=["--noconf", "missing.bcf"], outputs=["missing.bbl"]),
    Case("bb-072-input-encoding-short-option", "BB-072",
         "the documented -e short option for --input-encoding does not exist",
         "docs/research/02 B27",
         {"test.bib": "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}}\n"},
         tex=doc(), args=["--noconf", "-e", "UTF-8", "doc.bcf"]),
    Case("bb-073-fastsort-warns", "BB-073",
         "--fastsort (documented no-op) emits a WARN per sorted list",
         "docs/research/02 B29",
         {"refs.bib": "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}}\n"},
         tex=doc("", "\\cite{knuth}\n\\printbibliography\n\\newrefcontext[sorting=ynt]",
                 resources=("refs.bib",)),
         args=["--noconf", "--fastsort", "doc.bcf"]),
    Case("bb-075-debug-btparse-not-fatal", "BB-075",
         "with --debug, btparse syntax errors are neither counted nor fatal",
         "scratch/input/runs/dbg (docs/research/01 B15)",
         {"test.bib": "@article{good1, title = {x}}\n@article{bad1, title = {x} author = {y}}\n"},
         tex=doc(), args=["--noconf", "--debug", "doc.bcf"], allow=["DC-044"]),
    Case("bb-076-btparse-diagnostic-location", "BB-076",
         "btparse errors name a temp file and pass-2 line numbers (reports line 13 for an error in line 2: pass-2 buffer)",
         "docs/research/01 B20",
         {"test.bib": "@article{good1, title = {x}, author = {A B}, year = 2000}\n"
                      '@misc{cb1, title = {a \\char"7B b}}\n@misc{cb2, title = {ok}}\n'},
         tex=doc(), allow=["DC-042"]),
    Case("bb-077-ascii-encoding-dead-code", "BB-077",
         "bibencoding=ascii with non-ASCII bytes fails in the strict slurp (leniency is dead code)",
         "scratch/input/runs/enc (docs/research/01 B14)",
         {"test.bib": "@misc{e1, title={Caf\u00e9}}\n"},
         tex=doc(resources=("\\addbibresource[bibencoding=ascii]{test.bib}",))),
    Case("bb-078-month-not-integer-warning", "BB-078",
         "misleading 'not an integer' warning for month={February}, then converted to 2",
         "scratch/input/runs/dates d18 (docs/research/01 B22)",
         {"test.bib": "@misc{d18, title={18}, year = {2001}, month = {February}, date={2000-03}}\n"
                      "@misc{d21, title={21}, year = {2001}, month = {February}}\n"},
         tex=doc()),
    Case("bb-080-isbn-checksum", "BB-080",
         "isbn data constraint passes invalid checksums; the -V hint gives no details",
         "scratch/config/refs.bib (docs/research/02 B21)",
         {"refs.bib": "@book{bad, author={A. Author}, title={Bad}, year={1990}, isbn={0-201-13447-1}, issn={1234-5678}}\n"
                      "@book{knuth, author={Donald E. Knuth}, title={The TeXbook}, year={1984}, isbn={0-201-13447-0}}\n"},
         tex=doc(resources=("refs.bib",)), args=["--noconf", "--validate-datamodel", "doc.bcf"]),
]

# Bugs in docs/biber-bugs.md (status V or C) that have no case here.
NO_CASE = {
    "BB-008": "--output-fieldcase only matters for bibtex output (tool mode / --output-format=bibtex, DC-030)",
    "BB-009": "bblxml output (dropped, DC-020)",
    "BB-012": "tool mode (DC-030)",
    "BB-013": "tool mode (DC-030)",
    "BB-047": "tool/bibtex output (DC-030)",
    "BB-048": "only affects tool/bibtex output; the .bbl writer substitutes \\0 (checked: no spurious warning in .bbl mode)",
    "BB-053": "visible only in tool mode (DC-030)",
    "BB-056": "tool/bibtex output (DC-030)",
    "BB-063": "bblxml output (DC-020)",
    "BB-068": "--output-format=bibtex (DC-030)",
    "BB-074": "--output-format=biblatexml/bibjson (DC-031/DC-022)",
    "BB-079": "only the log level of an INFO line differs (not compared); the overriding itself is covered by styles/authoryear--sortlocale-da_DK",
    "BB-109": "performance only (see tools/corpus/gen_scale.py)",
    "BB-110": "performance only (see tools/corpus/gen_scale.py)",
    "BB-111": "performance only",
    "BB-112": "the fatal error names a random cycle and the number of preceding warnings varies; "
              "neither `unordered-lines` nor message sorting makes it stable (repro: biber --noconf "
              "xdata.bcf in a copy of t/tdata, or tdata's xdata.bcf); tdata/xdata-noloop covers the rest",
    "BB-113": "$MAPUNIQ is time-based, so the .bbl differs on every run (repro: biblatex example "
              "95-customlists.tex); nothing to compare against",
}


def build(c: Case) -> str:
    files: dict[str, str | bytes] = dict(c.files)
    if c.tex is not None:
        files[f"{c.stem}.tex"] = c.tex
        with tempfile.TemporaryDirectory(prefix="bibers-bug-") as td:
            work = Path(td)
            for name, content in files.items():
                if name.endswith((".tex", ".dbx")):
                    (work / name).write_text(content, encoding="utf-8")
            run_latex(work, f"{c.stem}.tex")
            bcf = work / f"{c.stem}.bcf"
            if not bcf.exists():
                return f"FAILED: no .bcf: {latex_error(work, c.stem)}"
            text = bcf.read_text(encoding="utf-8")
            if c.bcf_edit:
                text = c.bcf_edit(text)
            files[f"{c.stem}.bcf"] = text.encode("utf-8")
    spec = dict(
        description=f"{c.bb}: {c.description}",
        source=f"{c.source}; generated by tools/corpus/gen_bugs.py",
        args=c.args or ["--noconf", f"{c.stem}.bcf"],
        outputs=c.outputs or [f"{c.stem}.bbl"],
        log=c.log,
        compare=c.compare,
        tags=["suite:bugs", f"bug:{c.bb}", *c.tags],
        allow=c.allow,
    )
    write_case(SUITE / c.name, spec, files, allow_junk=c.allow_junk)
    return "ok"


def main() -> None:
    args = sys.argv[1:]
    if "--list-skipped" in args:
        for k, v in NO_CASE.items():
            print(f"{k}: {v}")
        return
    pat = re.compile(args[0] if args else ".")
    names = [c.name for c in CASES]
    assert len(names) == len(set(names)), "duplicate case names"
    todo = [c for c in CASES if pat.search(c.name)]
    if not args and SUITE.exists():
        for d in SUITE.iterdir():
            if d.is_dir() and d.name not in names:
                log(f"removing stale case {d.name}")
                shutil.rmtree(d)
    with cf.ThreadPoolExecutor(max_workers=8) as ex:
        for c, res in zip(todo, ex.map(build, todo)):
            log(f"{c.name:45} {res}")


if __name__ == "__main__":
    main()

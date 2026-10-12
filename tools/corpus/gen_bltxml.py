#!/usr/bin/env python3
"""Generate tests/corpus/bltxml/: biblatexml data sources (DC-031), one case per
group of constructs of Biber::Input::file::biblatexml: every field kind, names
(name parts, initials, per-name and per-namelist attributes), every date form,
ranges, lists, xsv fields, verbatim/URI fields, related entries, annotations,
XDATA (whole and granular), aliases (`ids`), cited-key and allkeys modes,
source maps (XPath field addressing), malformed and invalid input.

Each case's .bcf comes from one `pdflatex -draftmode` run of the inline
document. Cases where biber dies (exit 25/255) are tagged `perl-die` and list
DC-031 in `allow`: bibers' reviewed behaviour is in `expected-bibers/`
(`cargo xtask bless <case>`).

Usage: python3 tools/corpus/gen_bltxml.py [NAME-REGEX]
Then:  cargo xtask golden bltxml/   (O1; set PAR_GLOBAL_TEMP to a fresh directory
       if the TeX Live binary's PAR cache in $TMPDIR has been partly cleaned:
       every date then fails with "Can't call method year")
"""

import concurrent.futures as cf
import re
import shutil
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

from corpuslib import CORPUS, latex_error, log, run_latex, write_case

SUITE = CORPUS / "bltxml"
NS = "http://biblatex-biber.sourceforge.net/biblatexml"


def bltxml(*entries: str, head: str = "") -> str:
    return (f'<?xml version="1.0" encoding="UTF-8"?>\n{head}'
            f'<bltx:entries xmlns:bltx="{NS}">\n' + "".join(entries) + "</bltx:entries>\n")


def entry(key: str, etype: str, *fields: str) -> str:
    return f'  <bltx:entry id="{key}" entrytype="{etype}">\n' + "".join(
        f"    {f}\n" for f in fields) + "  </bltx:entry>\n"


def doc(opts: str = "style=authoryear", body: str = r"\nocite{*}", preamble: str = "",
        resources: tuple[str, ...] = ("test.bltxml",)) -> str:
    res = "".join(
        (r if r.startswith("\\") else f"\\addbibresource[datatype=biblatexml]{{{r}}}") + "\n"
        for r in resources)
    return (
        "\\documentclass{article}\n"
        "\\usepackage[T1]{fontenc}\n"
        f"\\usepackage[backend=biber,{opts}]{{biblatex}}\n"
        f"{preamble}{res}"
        "\\begin{document}\n"
        f"{body}\n"
        "\\printbibliography\n"
        "\\end{document}\n"
    )


def smap(*maps: str) -> str:
    return "\\DeclareSourcemap{\\maps[datatype=biblatexml]{\n" + "".join(
        f"  {m}\n" for m in maps) + "}}\n"


@dataclass
class Case:
    name: str
    description: str
    files: dict[str, str]
    tex: str
    args: list[str] | None = None
    allow: list[str] = field(default_factory=list)
    tags: list[str] = field(default_factory=list)


def name(*parts: str, attrs: str = "") -> str:
    return f"<bltx:name{attrs}>" + "".join(parts) + "</bltx:name>"


def np(t: str, v: str, attrs: str = "") -> str:
    return f'<bltx:namepart type="{t}"{attrs}>{v}</bltx:namepart>'


def lst(*items: str) -> str:
    return "<bltx:list>" + "".join(
        i if i.startswith("<bltx:item") else f"<bltx:item>{i}</bltx:item>" for i in items) + "</bltx:list>"


# ------------------------------------------------------------------ fixtures

FIELDS = bltxml(
    entry("f1", "article",
          "<bltx:ids><bltx:key>f1alias</bltx:key><bltx:key>f1alias2</bltx:key></bltx:ids>",
          "<bltx:options>useprefix=true, skipbiblist=true</bltx:options>",
          '<bltx:names type="author">' + name(np("family", "Doe"), np("given", "John")) + "</bltx:names>",
          "<bltx:title>A &amp; B &lt;c&gt; title with é and &#233;</bltx:title>",
          "<bltx:title>Second title is ignored</bltx:title>",
          "<bltx:subtitle><![CDATA[Sub & <title>]]></bltx:subtitle>",
          "<bltx:journaltitle>Journal of Things</bltx:journaltitle>",
          "<bltx:volume>10</bltx:volume>",
          "<bltx:number>3</bltx:number>",
          "<bltx:pages>" + lst("<bltx:item><bltx:start>1</bltx:start><bltx:end>10</bltx:end></bltx:item>",
                               "<bltx:item><bltx:start>15</bltx:start><bltx:end></bltx:end></bltx:item>",
                               "<bltx:item><bltx:start>20</bltx:start></bltx:item>") + "</bltx:pages>",
          "<bltx:keywords>kw1, kw2</bltx:keywords>",
          "<bltx:language>" + lst("english", "german") + "</bltx:language>",
          "<bltx:publisher>Plain Publisher</bltx:publisher>",
          "<bltx:location>  Spaced  location  </bltx:location>",
          "<bltx:doi>10.1000/xyz_1</bltx:doi>",
          "<bltx:url>http://example.org/a b?x=é</bltx:url>",
          "<bltx:eprint type=\"arxiv\" class=\"cs.DL\">1234.5678</bltx:eprint>",
          "<bltx:isbn>0-201-13447-0</bltx:isbn>",
          "<bltx:langid>english</bltx:langid>",
          "<bltx:abstract>Abstract text</bltx:abstract>",
          "<bltx:note>A note<!-- with a comment --> continues</bltx:note>",
          "<bltx:annotation>generic annotation</bltx:annotation>",
          '<bltx:annotation field="title">t-ann</bltx:annotation>',
          '<bltx:annotation field="language" item="2" name="alt" literal="1">lang2</bltx:annotation>',
          '<bltx:annotation field="author" item="1" part="family">fam</bltx:annotation>',
          '<bltx:annotation field="title" name="other" literal="0">other</bltx:annotation>',
          '<bltx:related>' + lst('<bltx:item type="multivolume" ids="f2,f3" string="str" options="dataonly"/>') + '</bltx:related>',
          "<bltx:date>2001-02-03</bltx:date>",
          "<bltx:sortkey>x</bltx:sortkey>",
          "<bltx:usera>verbatim? no, literal</bltx:usera>",
          "<bltx:foo>unknown field</bltx:foo>",
          '<x:title xmlns:x="http://example.org/other">other namespace</x:title>',
          "<bltx:pagetotal>123</bltx:pagetotal>"),
    entry("f2", "inbook",
          '<bltx:names type="author">' + name(np("family", "Roe"), np("given", "Jane")) + "</bltx:names>",
          "<bltx:title>Chapter</bltx:title>",
          "<bltx:crossref>f4</bltx:crossref>",
          "<bltx:pages>" + lst("<bltx:item><bltx:start>5</bltx:start><bltx:end>9</bltx:end></bltx:item>") + "</bltx:pages>"),
    entry("f3", "book",
          '<bltx:names type="author">' + name(np("family", "Poe"), np("given", "Edgar")) + "</bltx:names>",
          "<bltx:title>Book</bltx:title>",
          "<bltx:xref>f4</bltx:xref>",
          "<bltx:date>1999</bltx:date>",
          "<bltx:edition>2</bltx:edition>",
          '<bltx:related>' + lst('<bltx:item type="translationof" ids="f1"/>') + '</bltx:related>'),
    entry("f4", "collection",
          '<bltx:names type="editor">' + name(np("family", "Ed"), np("given", "Itor")) + "</bltx:names>",
          "<bltx:title>Collection</bltx:title>",
          "<bltx:booktitle>Collection Booktitle</bltx:booktitle>",
          "<bltx:publisher>" + lst("Pub A", "Pub B") + "</bltx:publisher>",
          "<bltx:location>" + lst("Berlin", "Paris") + "</bltx:location>",
          "<bltx:date>1998</bltx:date>"),
    entry("s1", "set",
          "<bltx:entryset>" + lst("f2", "f3") + "</bltx:entryset>",
          "<bltx:annotation>A set</bltx:annotation>"),
)

NAMES = bltxml(
    entry("n1", "book",
          '<bltx:names type="author" useprefix="true" morenames="1">'
          + name(np("family", "Beethoven"), np("prefix", "van"),
                 '<bltx:namepart type="given"><bltx:namepart initial="L">Ludwig</bltx:namepart>'
                 '<bltx:namepart>Maria</bltx:namepart></bltx:namepart>',
                 np("suffix", "Jr."), attrs=' gender="sm" id="hid1" useprefix="false"')
          + name(np("family", "Ångström"), np("given", "Anders", ' initial="X"'), attrs=' initial="Q"')
          + name(np("family", "{Barnes and Noble}"))
          + name(np("given", "Jean-Paul"), np("family", "Sartre"))
          + name('<bltx:namepart type="given"><bltx:namepart>Jo</bltx:namepart><bltx:namepart>B</bltx:namepart>'
                 '<bltx:namepart>C</bltx:namepart></bltx:namepart>', np("family", "Three"))
          + name('<bltx:namepart type="given"><bltx:namepart>Anne</bltx:namepart>'
                 '<bltx:namepart>Bea</bltx:namepart><bltx:namepart>Cee</bltx:namepart></bltx:namepart>', np("family", "Four"))
          + "</bltx:names>",
          '<bltx:names type="editor">' + name(np("family", "Ed"), np("given", "Only")) + "</bltx:names>",
          '<bltx:names type="translator" morenames="true">' + name(np("family", "Trans"), np("given", "Lator")) + "</bltx:names>",
          '<bltx:names type="afterword" morenames="0">' + name(np("family", "After"), np("given", "Word"), attrs=' gender="pf"') + "</bltx:names>",
          "<bltx:title>Names</bltx:title>",
          "<bltx:date>2000</bltx:date>"),
    entry("n2", "book",
          '<bltx:names type="author">' + name(np("family", "Doe"), np("given", "John")) + name(np("family", "Doe"), np("given", "Jane")) + "</bltx:names>",
          "<bltx:title>Uniquename</bltx:title>",
          "<bltx:date>2000</bltx:date>"),
    entry("n3", "book",
          '<bltx:names type="author">' + name(np("family", "Doe"), np("given", "Jim"), attrs=' uniquename="false"') + "</bltx:names>",
          '<bltx:names type="editor"></bltx:names>',
          "<bltx:title>Per-name option</bltx:title>",
          "<bltx:date>2000</bltx:date>"),
)

DATES = bltxml(*[
    entry(f"d{i:02}", "misc", f"<bltx:title>Date {i:02}</bltx:title>", *d)
    for i, d in enumerate([
        ["<bltx:date>2001</bltx:date>"],
        ["<bltx:date>2001-02</bltx:date>"],
        ["<bltx:date>2001-02-03</bltx:date>"],
        ["<bltx:date>2001-02-03T10:20:30</bltx:date>"],
        ["<bltx:date>2001-02-03T10:20:30Z</bltx:date>"],
        ["<bltx:date>2001-02-03T10:20:30+05:30</bltx:date>"],
        ["<bltx:date>-0044-03-15</bltx:date>"],
        ["<bltx:date>0000</bltx:date>"],
        ["<bltx:date>2001~</bltx:date>"],
        ["<bltx:date>2001?</bltx:date>"],
        ["<bltx:date>2001%</bltx:date>"],
        ["<bltx:date>199X</bltx:date>"],
        ["<bltx:date>19XX</bltx:date>"],
        ["<bltx:date>1999-XX</bltx:date>"],
        ["<bltx:date>1999-02-XX</bltx:date>"],
        ["<bltx:date>2000-02-XX</bltx:date>"],
        ["<bltx:date>1999-XX-XX</bltx:date>"],
        ["<bltx:date>2001-21</bltx:date>"],
        ["<bltx:date>2001-33</bltx:date>"],
        ["<bltx:date><bltx:start>2001</bltx:start><bltx:end>2003</bltx:end></bltx:date>"],
        ["<bltx:date><bltx:start>2001-02-03</bltx:start><bltx:end></bltx:end></bltx:date>"],
        ["<bltx:date><bltx:start>2001</bltx:start><bltx:end>..</bltx:end></bltx:date>"],
        ["<bltx:date><bltx:start>2001~</bltx:start><bltx:end>2003?</bltx:end></bltx:date>"],
        ["<bltx:date><bltx:start>2001-22</bltx:start><bltx:end>2001-23</bltx:end></bltx:date>"],
        ["<bltx:date><bltx:start>2001-02-03T10:00:00Z</bltx:start><bltx:end>2001-02-04T11:00:00+01:00</bltx:end></bltx:date>"],
        ["<bltx:date><bltx:start>1990</bltx:start><bltx:end>abc</bltx:end></bltx:date>"],
        ["<bltx:date><bltx:start>199X</bltx:start><bltx:end>2005</bltx:end></bltx:date>"],
        ["<bltx:date>2001/2003</bltx:date>"],
        ["<bltx:date>2001</bltx:date>", '<bltx:date type="orig">1850</bltx:date>',
         '<bltx:date type="event"><bltx:start>2000-05-01</bltx:start><bltx:end>2000-05-03</bltx:end></bltx:date>',
         '<bltx:date type="url">2020-01-02</bltx:date>'],
        ['<bltx:date type="orig">1850</bltx:date>'],
    ])
])

JULIAN = bltxml(*[
    entry(f"j{i}", "misc", f"<bltx:title>J {i}</bltx:title>", d)
    for i, d in enumerate([
        "<bltx:date>1500-01-01</bltx:date>",
        "<bltx:date>1500</bltx:date>",
        "<bltx:date><bltx:start>1400-02-10</bltx:start><bltx:end>1700-02-10</bltx:end></bltx:date>",
    ])
])

XDATA = bltxml(
    entry("x1", "xdata", "<bltx:publisher>" + lst("XPub") + "</bltx:publisher>",
          "<bltx:location>" + lst("XLoc1", "XLoc2") + "</bltx:location>",
          "<bltx:note>XNote</bltx:note>"),
    entry("x2", "xdata",
          '<bltx:names type="author">' + name(np("family", "Xa"), np("given", "Al")) + name(np("family", "Xb"), np("given", "Bo")) + "</bltx:names>",
          "<bltx:title>XTitle</bltx:title>",
          "<bltx:pages>" + lst("<bltx:item><bltx:start>3</bltx:start><bltx:end>4</bltx:end></bltx:item>") + "</bltx:pages>",
          "<bltx:url>http://example.org/x</bltx:url>",
          "<bltx:xdata>x1</bltx:xdata>"),
    entry("e1", "book", "<bltx:xdata>" + lst("x1", "x2") + "</bltx:xdata>", "<bltx:date>2000</bltx:date>"),
    entry("e2", "book",
          '<bltx:names type="author">' + name(attrs=' xdata="x2-author-2"') + name(np("family", "Own"), np("given", "Name")) + "</bltx:names>",
          '<bltx:names type="editor">' + name(attrs=' xdata="x2-author"') + "</bltx:names>",
          '<bltx:title xdata="x2-title"/>',
          '<bltx:note xdata="x1-note"/>',
          "<bltx:location>" + lst('<bltx:item xdata="x1-location-2"/>', "Own") + "</bltx:location>",
          '<bltx:pages xdata="x2-pages"/>',
          '<bltx:url xdata="x2-url"/>',
          '<bltx:publisher xdata="x1-publisher"/>',
          "<bltx:date>2001</bltx:date>"),
    entry("e3", "book", "<bltx:xdata>missingx</bltx:xdata>", "<bltx:title>Missing</bltx:title>",
          '<bltx:subtitle xdata="nofield"/>'),
)

ALLKEYS_WARN = bltxml(
    entry("k1", "misc", "<bltx:ids><bltx:key>ka</bltx:key><bltx:key>k2</bltx:key></bltx:ids>", "<bltx:title>K1</bltx:title>"),
    entry("k2", "misc", "<bltx:ids><bltx:key>ka</bltx:key><bltx:key>kb</bltx:key></bltx:ids>", "<bltx:title>K2</bltx:title>"),
    entry("k3", "misc", "<bltx:ids><bltx:key>kb</bltx:key><bltx:key>k1</bltx:key></bltx:ids>", "<bltx:title>K3</bltx:title>"),
    entry("k1", "misc", "<bltx:title>K1 duplicate</bltx:title>"),
    entry("K3", "misc", "<bltx:title>K3 case variant</bltx:title>"),
    '  <bltx:entry entrytype="misc"><bltx:title>No id</bltx:title></bltx:entry>\n',
    entry("ka", "misc", "<bltx:title>Real key that is also an alias</bltx:title>"),
)

CITED = bltxml(
    entry("c1", "inbook", "<bltx:title>C1</bltx:title>", "<bltx:crossref>c2</bltx:crossref>",
          "<bltx:ids><bltx:key>c1alias</bltx:key></bltx:ids>"),
    entry("c2", "book", "<bltx:title>C2</bltx:title>", "<bltx:booktitle>C2 book</bltx:booktitle>",
          "<bltx:date>1999</bltx:date>"),
    entry("c3", "misc", "<bltx:title>C3 first</bltx:title>", "<bltx:xdata>c4</bltx:xdata>"),
    entry("c3", "misc", "<bltx:title>C3 second</bltx:title>"),
    entry("c4", "xdata", "<bltx:note>From c4</bltx:note>"),
    entry("c5", "misc", "<bltx:title>C5 not cited</bltx:title>"),
    entry("c6", "misc", "<bltx:title>C6</bltx:title>",
          '<bltx:related>' + lst('<bltx:item type="reprint" ids="c5"/>') + '</bltx:related>'),
)

MAPPED = bltxml(
    entry("m1", "book",
          '<bltx:names type="author">' + name(np("family", "Smith"), np("given", "Ann")) + name(np("family", "Jones"), np("given", "Bob")) + "</bltx:names>",
          '<bltx:names type="editor">' + name(np("family", "Edit"), np("given", "Or")) + "</bltx:names>",
          '<bltx:names type="afterword">' + name(np("family", "After"), np("given", "Word")) + "</bltx:names>",
          "<bltx:title>The Original Title</bltx:title>",
          "<bltx:usera>alpha</bltx:usera>",
          "<bltx:userb>beta</bltx:userb>",
          "<bltx:userc>gamma</bltx:userc>",
          "<bltx:userd>delta</bltx:userd>",
          "<bltx:usere>x, y</bltx:usere>",
          "<bltx:note>Note 123-456</bltx:note>",
          "<bltx:series>Series</bltx:series>",
          "<bltx:date>2001</bltx:date>"),
    entry("m2", "article",
          '<bltx:names type="author">' + name(np("family", "Doe"), np("given", "Jo")) + "</bltx:names>",
          "<bltx:title>Article Title</bltx:title>",
          "<bltx:journaltitle>J</bltx:journaltitle>",
          "<bltx:note>keep</bltx:note>",
          "<bltx:date>2002</bltx:date>"),
    entry("m3", "misc", "<bltx:title>Dropped entry</bltx:title>", "<bltx:note>DROP</bltx:note>"),
    entry("m4", "report", "<bltx:title>Report</bltx:title>", "<bltx:type>techreport</bltx:type>", "<bltx:date>2003</bltx:date>"),
)

MAPS = smap(
    r"\map{\step[typesource=report, typetarget=misc, final]\step[fieldset=addendum, origentrytype]}",
    r"\map[overwrite]{\step[fieldsource=title, match=\regexp{Original}, replace={Mapped}]}",
    r"\map{\step[fieldsource=note, match=\regexp{(\d+)-(\d+)}, final]\step[fieldset=userf, fieldvalue={$2:$1}]}",
    r"\map[overwrite]{\step[fieldsource=usera, fieldtarget=userb]}",
    r"\map{\step[fieldsource=afterword, fieldtarget=annotator]}",
    r"\map[overwrite]{\step[fieldsource=editor, fieldtarget=translator]}",
    r"\map[overwrite]{\step[fieldsource=title, match=\regexp{Original}, final]"
    r"\step[fieldset={./bltx:names[@type='author']/bltx:name[2]/@gender}, fieldvalue={sm}]"
    r"\step[fieldset={./bltx:names[@type='author']/bltx:name[1]/bltx:namepart[@type='given']}, fieldvalue={Anna}]}",
    r"\map{\step[fieldsource=userc, final]\step[fieldset=userc, null]}",
    r"\map[overwrite]{\step[fieldset=series, fieldvalue={ appended}, append]}",
    r"\map[overwrite]{\step[fieldset=userd, fieldvalue={ strict}, appendstrict]\step[fieldset=edition, fieldvalue={ none}, appendstrict]}",
    r"\map{\step[fieldsource=journaltitle]\step[fieldset=usera, origfield]\step[fieldset=userb, origfieldval]}",
    r"\map[foreach=usere]{\step[fieldset=lista, fieldvalue={$MAPLOOP}, append]}",
    r"\map{\step[fieldsource=note, notmatch=\regexp{keep}, final]\step[fieldset=userc, fieldvalue={notkeep}]}",
    r"\map{\step[fieldsource=title, matchi=\regexp{article}, final]\step[fieldset=usere, fieldvalue={matchi}]}",
    r"\map{\pernottype{book}\step[fieldset=pubstate, fieldvalue={pernottype}]}",
    r"\map{\perdatasource{test.bltxml}\step[fieldset=howpublished, fieldvalue={perds}]}",
    r"\map{\perdatasource{other.bltxml}\step[fieldset=howpublished, fieldvalue={otherds}]}",
    r"\map[refsection=1]{\step[fieldset=version, fieldvalue={rs1}]}",
    r"\map{\step[citedornocited, final]\step[fieldset=chapter, fieldvalue={cited}]}",
    r"\map{\step[fieldset=userb, fieldvalue={not overwritten}]}",
)

MAPS_TARGET_NEW = smap(
    r"\map{\step[fieldsource=title, fieldtarget=subtitle]}",
    r"\map{\step[fieldsource=userd, fieldtarget={./@usera}]}",
)

MAPS_NEW_ENTRY = smap(
    r"\map{\step[fieldsource=title, match=\regexp{Original}, final]"
    r"\step[entrynew=newkey, entrynewtype=book]"
    r"\step[entrytarget=newkey, fieldset=title, fieldvalue={New}]"
    r"\step[entrytarget=nokey, fieldset=title, fieldvalue={New}]"
    r"\step[entrytarget=newkey, fieldset={./@entrytype}, fieldvalue={misc}]}",
    r"\map[foreach={./bltx:usere}]{\step[entrynew={loop:$MAPLOOP}, entrynewtype=misc]}",
)

MAPS_CRASH_NOTFIELD = smap(r"\map{\step[notfield=pubstate, fieldset=pubstate, fieldvalue={notfield}]}")
MAPS_CRASH_PERTYPE = smap(r"\map{\pertype{book}\step[fieldset=pubstate, fieldvalue={pertype}]}")
MAPS_CRASH_MATCHES = smap(r"\map[overwrite]{\step[fieldsource=userb, matches={beta,gamma}, replace={B,C}]}")
MAPS_CRASH_ENTRYNULL = smap(r"\map{\step[fieldsource=note, match=\regexp{DROP}, final]\step[entrynull]}")
MAPS_CRASH_NULL = smap(r"\map{\step[fieldset=userc, null]}")
MAPS_BAD_NAME = smap(r"\map[overwrite]{\step[fieldset={./bltx:names[@type='author']/bltx:name[2]/@gender}, fieldvalue={sm}]}")
MAPS_BAD_XPATH = smap(r"\map{\step[fieldsource={./bltx:title[}, final]\step[fieldset=userf, fieldvalue={x}]}")

INVALID_DATES = bltxml(
    entry("i1", "misc", "<bltx:title>I1</bltx:title>", "<bltx:date>abc</bltx:date>"),
    entry("i2", "misc", "<bltx:title>I2</bltx:title>", "<bltx:date>/2001</bltx:date>"),
    entry("i3", "misc", "<bltx:title>I3</bltx:title>", "<bltx:date><bltx:start>xyz</bltx:start><bltx:end>2001</bltx:end></bltx:date>"),
    entry("i4", "misc", "<bltx:title>I4</bltx:title>", "<bltx:date>-380</bltx:date>"),
    entry("i5", "misc", "<bltx:title>I5</bltx:title>", "<bltx:date> 2001 </bltx:date>"),
)


CASES: list[Case] = [
    Case("fields-allkeys", "every field kind (literal, list, xsv, range, verbatim, URI, entry keys, related, annotations, options, ids) with \\nocite{*}",
         {"test.bltxml": FIELDS}, doc()),
    Case("fields-cited", "the fields fixture with cited keys only: crossref/xref/related/set dependencies fetched from the biblatexml file, an alias citation",
         {"test.bltxml": FIELDS}, doc(body=r"\cite{f1,f2,s1,f1alias}")),
    Case("fields-numeric-fixquirks", "the fields fixture with the biblatexml quirks fixed (eprint attributes, URI escaping, relatedoptions)",
         {"test.bltxml": FIELDS}, doc("style=numeric"),
         args=["--noconf", "--fix-quirks=bltxml-eprint-attrs,bltxml-uri-escape,bltxml-relatedoptions", "doc.bcf"],
         allow=["DC-050"], tags=["quirk-fixed"]),
    Case("names", "names: name parts with sub-parts and initials, prefix/suffix, gender, id (hashid), per-name and per-namelist options, morenames",
         {"test.bltxml": NAMES}, doc("style=authoryear,uniquename=full,uniquelist=true,maxnames=3")),
    Case("names-alphabetic", "names under alphabetic labels (name part lengths, initials)",
         {"test.bltxml": NAMES}, doc("style=alphabetic,useprefix=true")),
    Case("dates", "every date form: simple, range (open, '..', empty end), approximate/uncertain, unspecified X forms, BCE, times and time zones, year divisions, date types",
         {"test.bltxml": DATES}, doc("style=authoryear,sorting=none")),
    Case("dates-julian", "dates converted to the Julian calendar (julian, gregorianstart)",
         {"test.bltxml": JULIAN}, doc("style=authoryear,julian=true,gregorianstart=1582-10-15")),
    Case("dates-invalid", "invalid simple dates and range starts: biber dies calling ->year on undef; bibers warns and ignores the date",
         {"test.bltxml": INVALID_DATES}, doc(), allow=["DC-031"], tags=["perl-die"]),
    Case("xdata", "whole and granular XDATA references from literal, list, name, range and URI fields; missing XDATA entries and fields",
         {"test.bltxml": XDATA}, doc()),
    Case("allkeys-warnings", "allkeys: entries without id, duplicate keys, case variants, conflicting aliases",
         {"test.bltxml": ALLKEYS_WARN}, doc()),
    Case("cited-keys", "cited keys: crossref and XDATA dependencies, duplicate entries in the file, a missing key, an uncited related entry",
         {"test.bltxml": CITED}, doc(body=r"\cite{c1,c3,c6,missing}")),
    Case("sections", "two refsections reading the same biblatexml file (module-level key order state)",
         {"test.bltxml": CITED}, doc(body="\\begin{refsection}\\nocite{*}\\printbibliography\\end{refsection}\n"
                                           "\\begin{refsection}\\cite{c2}\\printbibliography\\end{refsection}")),
    Case("smap", "biblatexml source maps: XPath and field-name targets, type changes, match/replace with captures, names to names, attribute set, null, append, origfield/origfieldval/origentrytype, foreach, notmatch, matchi (case-sensitive in biber), per_nottype, per_datasource, refsection, cited conditions, final",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS, body=r"\nocite{*}\cite{m2}")),
    Case("smap-fixquirks", "the smap case with bltxml-smap-matchi fixed",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS, body=r"\nocite{*}\cite{m2}"),
         args=["--noconf", "--fix-quirks=bltxml-smap-matchi", "doc.bcf"], allow=["DC-050"], tags=["quirk-fixed"]),
    Case("smap-target-new", "mapping a field to a field that does not exist yet stores the source node's serialised XML (and to an attribute)",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS_TARGET_NEW)),
    Case("smap-new-entry", "map_entry_new, map_entrytarget (an XPath error makes 'exists' true, so the new entry gets no fields), foreach over an XPath",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS_NEW_ENTRY)),
    Case("smap-crash-notfield", "map_notfield without map_field_source: biber dies (empty XPath); bibers applies the step",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS_CRASH_NOTFIELD), allow=["DC-031"], tags=["perl-die"]),
    Case("smap-crash-pertype", "per_type in a biblatexml map: biber dies ($entry->type); bibers compares the entry type",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS_CRASH_PERTYPE), allow=["DC-031"], tags=["perl-die"]),
    Case("smap-crash-matches", "map_matches in a biblatexml map: biber dies ($etarget->set); bibers sets the field",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS_CRASH_MATCHES), allow=["DC-031"], tags=["perl-die"]),
    Case("smap-bad-xpath", "an invalid XPath in a biblatexml map: biber dies; bibers reports an ERROR",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS_BAD_XPATH), allow=["DC-031"], tags=["perl-die"]),
    Case("smap-crash-entrynull", "map_entry_null with \\nocite{*}: biber keeps the key and dies in get_dependents; bibers drops the entry",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS_CRASH_ENTRYNULL), allow=["DC-031"], tags=["perl-die"]),
    Case("smap-crash-entrynull-cited", "map_entry_null on a cited key: biber dies; bibers drops the entry with a warning",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS_CRASH_ENTRYNULL, body=r"\cite{m1,m3}"), allow=["DC-031"], tags=["perl-die"]),
    Case("smap-crash-null", "map_null on a field an entry does not have: biber dies (unbindNode on undef); bibers does nothing",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS_CRASH_NULL), allow=["DC-031"], tags=["perl-die"]),
    Case("smap-bad-name", "a map creating an element from an XPath step that selects nothing (bltx:name[2]): XML::LibXML dies ('bad name'); bibers reports an ERROR",
         {"test.bltxml": MAPPED}, doc(preamble=MAPS_BAD_NAME), allow=["DC-031"], tags=["perl-die"]),
    Case("malformed", "malformed XML: biber dies with libxml2's parser error; bibers reports an ERROR (exit 2)",
         {"test.bltxml": bltxml(entry("a", "misc", "<bltx:title>T</bltx:titl>"))}, doc(),
         allow=["DC-031"], tags=["perl-die"]),
    Case("empty-file", "an empty biblatexml file is ignored with a warning",
         {"test.bltxml": ""}, doc()),
    Case("datepart-field", "a data-model field with no biblatexml handler (year): biber dies; bibers warns and ignores it",
         {"test.bltxml": bltxml(entry("a", "misc", "<bltx:title>T</bltx:title>", "<bltx:year>2000</bltx:year>"))}, doc(),
         allow=["DC-031"], tags=["perl-die"]),
    Case("default-namespace", "a biblatexml file using the default namespace instead of the bltx prefix: biber dies; bibers reports an ERROR",
         {"test.bltxml": f'<?xml version="1.0"?>\n<entries xmlns="{NS}"><entry id="a" entrytype="misc"><title>T</title></entry></entries>\n'},
         doc(), allow=["DC-031"], tags=["perl-die"]),
    Case("other-prefix", "a biblatexml file using another prefix for the namespace: biber dies on the XPath for aliases (undefined prefix); bibers reports an ERROR",
         {"test.bltxml": f'<?xml version="1.0"?>\n<b:entries xmlns:b="{NS}"><b:entry id="a" entrytype="misc"><b:title>T</b:title></b:entry></b:entries>\n'},
         doc(), allow=["DC-031"], tags=["perl-die"]),
    Case("other-prefix-cited", "another namespace prefix with cited keys: the entry is found but its fields are not",
         {"test.bltxml": f'<?xml version="1.0"?>\n<b:entries xmlns:b="{NS}"><b:entry id="a" entrytype="misc"><b:title>T</b:title></b:entry></b:entries>\n'},
         doc(body=r"\cite{a}")),
    Case("no-schema", "--no-bltxml-schema (no .rng written)",
         {"test.bltxml": FIELDS}, doc(), args=["--noconf", "--no-bltxml-schema", "doc.bcf"]),
    Case("mixed-sources", "a biblatexml and a bibtex data source in one section",
         {"test.bltxml": CITED, "test.bib": "@book{b1, author={Bib Author}, title={From bib}, date={2005}, crossref={c2}}\n"},
         doc(resources=("test.bltxml", "test.bib")).replace(
             "\\addbibresource[datatype=biblatexml]{test.bib}", "\\addbibresource{test.bib}"),
         ),
]


def build(c: Case) -> str:
    files: dict[str, str | bytes] = dict(c.files)
    files["doc.tex"] = c.tex
    with tempfile.TemporaryDirectory(prefix="bibers-bltxml-") as td:
        work = Path(td)
        (work / "doc.tex").write_text(c.tex, encoding="utf-8")
        run_latex(work, "doc.tex")
        bcf = work / "doc.bcf"
        if not bcf.exists():
            return f"FAILED: no .bcf: {latex_error(work, 'doc')}"
        files["doc.bcf"] = bcf.read_bytes()
    spec = dict(
        description=c.description,
        source="hand-written; generated by tools/corpus/gen_bltxml.py",
        args=c.args or ["--noconf", "doc.bcf"],
        outputs=["doc.bbl"],
        tags=["suite:bltxml", "feature:biblatexml", *c.tags],
        allow=c.allow,
    )
    write_case(SUITE / c.name, spec, files)
    return "ok"


def main() -> None:
    pat = re.compile(sys.argv[1] if len(sys.argv) > 1 else ".")
    names = [c.name for c in CASES]
    assert len(names) == len(set(names)), "duplicate case names"
    todo = [c for c in CASES if pat.search(c.name)]
    if len(sys.argv) == 1 and SUITE.exists():
        for d in SUITE.iterdir():
            if d.is_dir() and d.name not in names:
                log(f"removing stale case {d.name}")
                shutil.rmtree(d)
    with cf.ThreadPoolExecutor(max_workers=8) as ex:
        for c, res in zip(todo, ex.map(build, todo)):
            log(f"{c.name:40} {res}")


if __name__ == "__main__":
    main()

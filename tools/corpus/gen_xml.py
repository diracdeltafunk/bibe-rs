#!/usr/bin/env python3
"""Generate tests/corpus/xml/: RELAX NG validation (--validate-control,
--validate-config; DC-033) and --convert-control cases.

Inputs come from biber's t/tdata (as in gen_tdata.py) and a few corpus
.bcf files, some edited to be invalid. Ports t/configfile.t's config
validation; t/bcfvalidation.t is ported as crates/bibers/tests/xml.rs.

Usage: python3 tools/corpus/gen_xml.py [NAME-REGEX]
Then:  cargo xtask golden xml/
"""

import re
import sys

from corpuslib import CORPUS, TDATA, ensure_shared_examples_bib, log, write_case

SUITE = CORPUS / "xml"


def general(extra_args, name, description, bcf=None, conf=None, outputs=None, tags=()):
    files = {"general.bcf": bcf if bcf is not None else TDATA / "general.bcf",
             "examples.bib": TDATA / "examples.bib"}
    if conf is not None:
        files["biber-test.conf"] = conf
    args = ([] if conf is not None else ["--noconf"]) + extra_args + ["general.bcf"]
    return name, {
        "description": description,
        "source": "vendor/biber/t/tdata/general.bcf" + (" + biber-test.conf" if conf is not None else ""),
        "args": args,
        "outputs": outputs or ["general.bbl"],
        "tags": ["suite:xml", *tags],
    }, files


def tdata_case(stem, bib, extra_args, name, description, outputs_extra=(), tags=()):
    return name, {
        "description": description,
        "source": f"vendor/biber/t/tdata/{stem}.bcf",
        "args": ["--noconf", *extra_args, f"{stem}.bcf"],
        "outputs": [f"{stem}.bbl", *outputs_extra],
        "tags": ["suite:xml", *tags],
    }, {f"{stem}.bcf": TDATA / f"{stem}.bcf", **{b: TDATA / b for b in ([bib] if isinstance(bib, str) else bib)}}


def edited_bcf(fn):
    s = (TDATA / "general.bcf").read_text(encoding="utf-8")
    t = fn(s)
    assert t != s, "edit did not apply; general.bcf changed upstream?"
    return t


def edited_conf(fn):
    s = (TDATA / "biber-test.conf").read_text(encoding="utf-8")
    t = fn(s)
    assert t != s, "edit did not apply; biber-test.conf changed upstream?"
    return t


def cases():
    vc = ["feature:validate-control"]
    vf = ["feature:validate-config"]
    cc = ["feature:convert-control"]
    yield general(["--validate-control"], "validate-control-general",
                  "--validate-control on a valid .bcf (t/bcfvalidation.t)", tags=vc)
    yield tdata_case("names", "names.bib", ["--validate-control"], "validate-control-names",
                     "--validate-control on a valid .bcf (t/bcfvalidation.t)", tags=vc)
    bad_attr = edited_bcf(lambda s: s.replace('<bcf:section number="0">', '<bcf:section number="0" foo="x">', 1))
    yield general(["--validate-control"], "validate-control-invalid-attribute",
                  "--validate-control: an unknown attribute makes the .bcf invalid (fatal)", bcf=bad_attr, tags=vc)
    yield general(["--validate-control", "--nodieonerror"], "validate-control-invalid-nodieonerror",
                  "--validate-control --nodieonerror: an invalid .bcf is reported and processing continues",
                  bcf=bad_attr, tags=vc)
    yield general(["--validate-control"], "validate-control-invalid-value",
                  "--validate-control: an empty bltxversion (minLength 1) is invalid",
                  bcf=edited_bcf(lambda s: re.sub(r'bltxversion="[^"]*"', 'bltxversion=""', s, count=1)), tags=vc)
    yield general(["--validate-control"], "validate-control-missing-element",
                  "--validate-control: a required element (the biber options) is missing",
                  bcf=edited_bcf(lambda s: re.sub(r'\s*<bcf:options component="biber" type="global">.*?</bcf:options>',
                                                  "", s, count=1, flags=re.S)), tags=vc)
    yield general(["--validate-control"], "validate-control-misplaced-element",
                  "--validate-control: elements out of schema order",
                  bcf=edited_bcf(lambda s: s.replace(
                      '<bcf:options component="biber" type="global">',
                      '<bcf:datafieldset name="xx"><bcf:member field="title"/></bcf:datafieldset>\n  '
                      '<bcf:options component="biber" type="global">', 1)),
                  tags=vc)
    ensure_shared_examples_bib()
    chicago = CORPUS / "styles" / "chicago-notes" / "chicago-notes.bcf"
    yield "validate-control-chicago-notes", {
        "description": "--validate-control on chicago-notes: biblatex writes map_entry_nocite, which bcf.rng lacks (fatal)",
        "source": "tests/corpus/styles/chicago-notes (gen_styles.py)",
        "args": ["--noconf", "--validate-control", "chicago-notes.bcf"],
        "outputs": ["chicago-notes.bbl"],
        "shared": ["biblatex-examples.bib"],
        "tags": ["suite:xml", *vc],
    }, {"chicago-notes.bcf": chicago}
    conf = TDATA / "biber-test.conf"
    yield general(["--configfile=biber-test.conf", "--mincrossrefs=7", "--validate-config"], "validate-config-general",
                  "--validate-config on t/tdata/biber-test.conf (t/configfile.t)", conf=conf, tags=vf)
    yield general(["--configfile=biber-test.conf", "--validate-config"], "validate-config-invalid-element",
                  "--validate-config: an unknown element in the config file (fatal)",
                  conf=edited_conf(lambda s: s.replace("<config>", "<config>\n  <bogus>1</bogus>", 1)), tags=vf)
    yield general(["--configfile=biber-test.conf", "--validate-config", "--nodieonerror"],
                  "validate-config-invalid-nodieonerror",
                  "--validate-config --nodieonerror: an invalid config file is reported and processing continues",
                  conf=edited_conf(lambda s: s.replace("<config>", "<config>\n  <bogus>1</bogus>", 1)), tags=vf)
    yield general(["--configfile=biber-test.conf", "--validate-config"], "validate-config-invalid-value",
                  "--validate-config: a non-integer mincrossrefs",
                  conf=edited_conf(lambda s: re.sub(r"<mincrossrefs>\s*\d+\s*</mincrossrefs>",
                                                    "<mincrossrefs>many</mincrossrefs>", s, count=1)), tags=vf)
    yield general(["--validate-config"], "validate-config-noconf",
                  "--validate-config with --noconf: there is no config file to validate", tags=vf)
    yield general(["--configfile=biber-test.conf", "--validate-config", "--validate-control"], "validate-both",
                  "--validate-config and --validate-control together", conf=conf, tags=vf + vc)
    yield general(["--convert-control"], "convert-control-general",
                  "--convert-control: general.bcf as HTML", outputs=["general.bbl", "general.bcf.html"], tags=cc)
    yield general(["--convert-control", "--validate-control"], "convert-control-validate",
                  "--convert-control with --validate-control", outputs=["general.bbl", "general.bcf.html"], tags=cc + vc)
    for stem, bib in [("maps", "maps.bib"), ("labelalpha", "labelalpha.bib"), ("sort-complex", "sort.bib"),
                      ("full-bbl", "full-bbl.bib"), ("names", "names.bib"), ("sections", [f"sections{i}.bib" for i in range(1, 5)]),
                      ("set-dynamic", "sets.bib"), ("translit", "translit.bib")]:
        yield tdata_case(stem, bib, ["--convert-control"], f"convert-control-{stem}",
                         f"--convert-control: {stem}.bcf as HTML", outputs_extra=[f"{stem}.bcf.html"], tags=cc)
    yield "convert-control-chicago-notes", {
        "description": "--convert-control on a large real .bcf (contributed style chicago-notes: sourcemaps, inheritance)",
        "source": "tests/corpus/styles/chicago-notes (gen_styles.py)",
        "args": ["--noconf", "--convert-control", "chicago-notes.bcf"],
        "outputs": ["chicago-notes.bbl", "chicago-notes.bcf.html"],
        "shared": ["biblatex-examples.bib"],
        "tags": ["suite:xml", *cc],
    }, {"chicago-notes.bcf": chicago}


def main():
    pat = re.compile(sys.argv[1]) if len(sys.argv) > 1 else None
    for name, spec, files in cases():
        if pat and not pat.search(name):
            continue
        for f in files.values():
            if hasattr(f, "exists") and not f.exists():
                raise SystemExit(f"missing input {f}")
        write_case(SUITE / name, spec, files)
        log(f"xml/{name}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Mutate .bcf files to exercise every branch of bcf.xsl (for
convert_diff.py): random values for the attributes the stylesheet reads,
random sort keys (ties, case, non-ASCII, missing), extra elements of the
kinds the stylesheet renders, odd text (markup characters, entities,
comments splitting text).

    python3 mutate_convert.py OUTDIR SEED COUNT FILE...
"""
import os
import random
import sys
from xml.dom import minidom

NS = "https://sourceforge.net/projects/biblatex"
VALUES = ["", "0", "1", "2", "10", "-1", "1.5", " 3 ", "x", "true", "false", "all", "left", "right",
          "descending", "ascending", "v", "vf", "l", "A", "a", "B", "b", "é", "É", "z", "Z",
          "<&>\"'", "abc def", "1e2", ".5", "3.", "+2", "none", "one", "integer", "datespec",
          "conditional", "data", "mandatory", "set", "singlevalued", "multivalued", "biber", "global"]
ATTRS = ["order", "final", "sort_direction", "sortupper", "sortcase", "pad_side", "pad_char",
         "pad_width", "substring_side", "substring_width", "ifnames", "namessep", "names",
         "noalphaothers", "substring_width_max", "substring_fixed_threshold", "substring_compound",
         "map_overwrite", "refsection", "map_foreach", "map_final", "map_type_source",
         "map_type_target", "map_entry_new", "map_entry_newtype", "map_field_source",
         "map_field_target", "map_match", "map_matchi", "map_notmatch", "map_notmatchi",
         "map_matches", "map_matchesi", "map_replace", "map_entrykey_cited",
         "map_entrykey_nocited", "map_entrykey_citedornocited", "map_entrykey_allnocited",
         "map_entrykey_starnocited", "map_entry_null", "map_field_set", "map_null",
         "map_field_value", "map_origentrytype", "map_origfield", "map_origfieldval",
         "map_entrytarget", "type", "datatype", "fieldtype", "field", "backendout", "backendin",
         "inherit_all", "override_target", "suppress", "target", "source", "skip", "use", "inits",
         "base", "disambiguation", "hashscope", "visibility", "nullok", "skip_output", "format",
         "quant", "rangemin", "rangemax", "coerce", "count", "members", "number", "section",
         "entrytype", "from", "to", "name", "level", "pre", "labelprefix", "sortscheme",
         "uniquenametemplatename", "labelalphanametemplatename", "component", "version",
         "bltxversion"]
ELEMS = ["transliteration", "translit", "per_type", "per_datasource", "map_step", "constraint",
         "antecedent", "consequent", "fieldxor", "fieldor", "field", "sortitem", "sort", "part",
         "keypart", "labelpart", "labelelement", "namepart", "type_pair", "inherit", "defaults",
         "filter", "citekey", "citekeycount", "datasource", "value", "option", "member",
         "exclusion", "inclusion", "constant", "entrytype", "scope", "presort", "section", "bibdata"]


def elements(n):
    out = []
    for c in n.childNodes:
        if c.nodeType == c.ELEMENT_NODE:
            out.append(c)
            out.extend(elements(c))
    return out


def mutate(doc, rnd):
    els = elements(doc.documentElement)
    e = rnd.choice(els)
    op = rnd.randrange(6)
    if op <= 1:
        e.setAttribute(rnd.choice(ATTRS), rnd.choice(VALUES))
    elif op == 2:
        for c in list(e.childNodes):
            if c.nodeType == c.TEXT_NODE:
                e.removeChild(c)
        if not any(c.nodeType == c.ELEMENT_NODE for c in e.childNodes):
            e.appendChild(doc.createTextNode(rnd.choice(VALUES)))
            if rnd.random() < 0.2:
                e.appendChild(doc.createComment("c"))
                e.appendChild(doc.createTextNode(rnd.choice(VALUES)))
    elif op == 3:
        name = rnd.choice(ELEMS)
        new = doc.createElementNS(NS, "bcf:" + name)
        for _ in range(rnd.randint(0, 4)):
            new.setAttribute(rnd.choice(ATTRS), rnd.choice(VALUES))
        if rnd.random() < 0.7:
            new.appendChild(doc.createTextNode(rnd.choice(VALUES)))
        e.appendChild(new)
    elif op == 4:
        # Duplicate a sibling with a different order key.
        c = e.cloneNode(True)
        c.setAttribute("order", rnd.choice(VALUES))
        e.parentNode.insertBefore(c, e)
    else:
        if e.hasAttribute("order"):
            e.removeAttribute("order")


def main():
    outdir, seed, count = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    files = sys.argv[4:]
    rnd = random.Random(seed)
    os.makedirs(outdir, exist_ok=True)
    texts = {f: open(f, "rb").read() for f in files}
    for i in range(count):
        f = rnd.choice(files)
        doc = minidom.parseString(texts[f])
        for _ in range(rnd.randint(1, 12)):
            mutate(doc, rnd)
        name = os.path.join(outdir, f"{seed:03d}-{i:05d}.bcf")
        with open(name, "wb") as fh:
            fh.write(doc.toxml(encoding="UTF-8"))
        print(name)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Generate mutated XML documents for differential validation tests of
crates/relaxng against libxml2 (verdicts.pl).

    python3 mutate.py OUTDIR SEED COUNT FILE...

Each output is one input file with 1-3 random mutations: drop, duplicate,
rename or move an element; drop, add or rename an attribute; change an
attribute value or a text content to a "bad" value; add stray text; swap
siblings. Writes OUTDIR/NNNNN.xml and prints their names.
"""
import os
import random
import sys
from xml.dom import minidom

VALUES = ["", " ", "x", "0", "1", "2", "-1", "true", "false", " true ", "1.5", "abc def",
          "  12  ", "+3", "01", "yes", "\t", "a\nb", "99999999999999999999", "use", "useprefix",
          "global", "biber", "none", "nty", "é", "1t", "3", "inf"]


def elements(node):
    out = []
    for c in node.childNodes:
        if c.nodeType == c.ELEMENT_NODE:
            out.append(c)
            out.extend(elements(c))
    return out


def mutate(doc, rnd):
    root = doc.documentElement
    els = elements(root)
    if not els:
        return
    e = rnd.choice(els)
    op = rnd.randrange(12)
    parent = e.parentNode
    if op == 0:
        parent.removeChild(e)
    elif op == 1:
        parent.insertBefore(e.cloneNode(True), e)
    elif op == 2:
        other = rnd.choice(els)
        new = doc.createElementNS(e.namespaceURI, (e.prefix + ":" if e.prefix else "") + other.localName)
        for a in list(e.attributes.values()):
            new.setAttribute(a.name, a.value)
        for c in list(e.childNodes):
            new.appendChild(c)
        parent.replaceChild(new, e)
    elif op == 3 and e.attributes.length:
        a = rnd.choice(list(e.attributes.values()))
        e.removeAttribute(a.name)
    elif op == 4:
        names = sorted({a.name for x in els for a in x.attributes.values()} | {"foo"})
        e.setAttribute(rnd.choice(names), rnd.choice(VALUES))
    elif op == 5 and e.attributes.length:
        a = rnd.choice(list(e.attributes.values()))
        e.setAttribute(a.name, rnd.choice(VALUES))
    elif op == 6:
        for c in list(e.childNodes):
            if c.nodeType == c.TEXT_NODE:
                e.removeChild(c)
        if not any(c.nodeType == c.ELEMENT_NODE for c in e.childNodes) or rnd.random() < 0.3:
            e.appendChild(doc.createTextNode(rnd.choice(VALUES)))
    elif op == 7:
        e.insertBefore(doc.createTextNode(rnd.choice(["x", " ", "\n  ", "1"])), e.firstChild)
    elif op == 8:
        sib = e.nextSibling
        while sib is not None and sib.nodeType != sib.ELEMENT_NODE:
            sib = sib.nextSibling
        if sib is not None:
            parent.removeChild(sib)
            parent.insertBefore(sib, e)
    elif op == 9:
        target = rnd.choice(els)
        if target is not e and not is_ancestor(e, target):
            parent.removeChild(e)
            target.appendChild(e)
    elif op == 10 and e.attributes.length:
        a = rnd.choice(list(e.attributes.values()))
        v = a.value
        e.removeAttribute(a.name)
        e.setAttribute(a.name + "x", v)
    else:
        # Empty an element completely.
        for c in list(e.childNodes):
            e.removeChild(c)


def is_ancestor(a, b):
    while b is not None:
        if b is a:
            return True
        b = b.parentNode
    return False


def main():
    outdir, seed, count = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    files = sys.argv[4:]
    rnd = random.Random(seed)
    os.makedirs(outdir, exist_ok=True)
    texts = {f: open(f, "rb").read() for f in files}
    for i in range(count):
        f = rnd.choice(files)
        try:
            doc = minidom.parseString(texts[f])
        except Exception:
            continue
        for _ in range(rnd.randint(1, 3)):
            mutate(doc, rnd)
        name = os.path.join(outdir, f"{seed:03d}-{i:05d}.xml")
        with open(name, "wb") as fh:
            fh.write(doc.toxml(encoding="UTF-8"))
        print(name)


if __name__ == "__main__":
    main()

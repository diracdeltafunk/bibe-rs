#!/usr/bin/env python3
"""Split a biblatexml file into one document per <bltx:entry>, and also
write variants of each entry with one child element removed.

    python3 split_entries.py OUTDIR FILE...
"""
import os
import sys
from xml.dom import minidom


def main():
    outdir = sys.argv[1]
    os.makedirs(outdir, exist_ok=True)
    n = 0
    for f in sys.argv[2:]:
        doc = minidom.parse(f)
        root = doc.documentElement
        entries = [c for c in root.childNodes if c.nodeType == c.ELEMENT_NODE]
        for e in entries:
            variants = [None] + [i for i, c in enumerate(e.childNodes) if c.nodeType == c.ELEMENT_NODE]
            for drop in variants:
                d2 = minidom.parseString(doc.toxml(encoding="UTF-8"))
                r2 = d2.documentElement
                for c in list(r2.childNodes):
                    r2.removeChild(c)
                e2 = d2.importNode(e, True)
                if drop is not None:
                    e2.removeChild(e2.childNodes[drop])
                r2.appendChild(e2)
                name = os.path.join(outdir, f"e{n:05d}.xml")
                n += 1
                with open(name, "wb") as fh:
                    fh.write(d2.toxml(encoding="UTF-8"))
                print(name)


if __name__ == "__main__":
    main()

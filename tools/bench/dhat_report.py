#!/usr/bin/env python3
"""Summarise a dhat heap profile of bibers (where the peak heap goes).

    cargo build --profile profiling -p bibers --features dhat-heap
    (cd workdir && .../target/profiling/bibers -q --noconf doc.bcf)   # writes dhat-heap.json
    tools/bench/dhat_report.py dhat-heap.json [--by site|phase|area] [--depth N]
                               [--within REGEX] [--top N] [--key gb|eb|tb]

`--key`: bytes live at the global peak (`gb`, the default), at exit (`eb`)
or allocated in total (`tb`). `--by site` (default) groups by the first
`--depth` frames that are not allocator or standard library code; `--by
phase` by the pipeline phase (or writer) that made the allocation; `--by
area` by a rough data area (entries, names, list state, …). `--within`
keeps only allocations whose stack mentions REGEX.
"""

import argparse
import json
import re
from collections import defaultdict

SKIP = re.compile(
    r"^(alloc::|core::|std::|<alloc::|<core::|<std::|hashbrown|<hashbrown|serde_json::|"
    r"<serde_json::|dhat|<dhat|__rust|rust_|<T as |<\[|<str as|<&|<rustc_hash|"
    r"compact_str::|<compact_str::|thin_vec::|<thin_vec::|<u8 as)"
)

# The frame just below one of these is the phase.
DRIVERS = re.compile(
    r"(pipeline::prepare_section|lists::process_lists|bbl::render_text|pipeline::prepare)$"
)

AREAS = [
    ("output text", r"output::bbl::"),
    ("parsed data files (cache)", r"btparse::|cache_data"),
    ("list state: names (namelistdata)", r"set_namedis|DisPart|uniqueness::|generate_contextdata|process_visible_names|process_pername_hashes"),
    ("list state: entries' fields", r"process_entries_post|process_fullhash|process_namehash|set_entryfield|labelalpha|process_extra|ls::"),
    ("list state: sort data", r"generate_sortinfo|sort::"),
    ("list state: other", r"liststate|pv::"),
    ("names", r"NameArena|names::|name_handler|NamePart|parsename|gen_initials"),
    ("entries: fields", r"SymMap<core::option::Option<bibers_core::model::Value|create_entry|run_handler|literal|Value as core::clone"),
    ("entries: slots, options", r"SymMap<serde_json|BlxOptions"),
    ("section key records", r"add_citekeys|add_everykey|keytods|EntryMap"),
    ("sets, clones, xdata", r"resolve::"),
]


def frame_name(f):
    m = re.match(r"(?:0x[0-9a-f]+: |\[[^\]]*\] )?(.*?)( \(.*\))?$", f)
    name = m.group(1) if m else f
    return re.sub(r"::h[0-9a-f]{16}$", "", name), (m.group(2) or "") if m else ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("--by", choices=["site", "phase", "area"], default="site")
    ap.add_argument("--key", default="gb")
    ap.add_argument("--depth", type=int, default=1)
    ap.add_argument("--within")
    ap.add_argument("--top", type=int, default=30)
    a = ap.parse_args()
    d = json.load(open(a.file))
    names = [frame_name(f) for f in d["ftbl"]]
    agg = defaultdict(int)
    total = 0
    for pp in d["pps"]:
        v = pp.get(a.key, 0)
        if not v:
            continue
        fs = [names[i] for i in pp["fs"]]
        if a.within and not any(re.search(a.within, n) for n, _ in fs):
            continue
        total += v
        if a.by == "site":
            frames = [n + (loc if a.depth == 1 else "") for n, loc in fs if not SKIP.search(n)]
            k = " <- ".join(frames[: a.depth]) or "?"
        elif a.by == "phase":
            k = "?"
            for i, (n, _) in enumerate(fs):
                if DRIVERS.search(n):
                    k = fs[i - 1][0] if i > 0 else n
                    break
        else:
            stack = "\n".join(n for n, _ in fs)
            k = next((name for name, rx in AREAS if re.search(rx, stack)), "other")
        agg[k] += v
    print(f"total ({a.key}): {total / 2**20:.1f} MB")
    for k, v in sorted(agg.items(), key=lambda x: -x[1])[: a.top]:
        print(f"{v / 2**20:8.1f} MB  {k}")


if __name__ == "__main__":
    main()

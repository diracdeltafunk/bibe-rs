#!/usr/bin/env python3
"""Summarise a samply profile as text: self and inclusive time per function.

Record with symbols (build with `cargo build --profile profiling`):

    samply record --save-only --unstable-presymbolicate -r 4000 \
        -o prof.json.gz -- target/profiling/bibers -q doc.bcf
    tools/bench/profile_report.py prof.json.gz [--top 40] [--filter REGEX]

Samples are attributed to real (non-inlined) functions: the sidecar's inline
information is per symbol, not per address. `--filter` restricts the inclusive table to matching names; `--within` counts
only the samples under a matching function (a callee breakdown).
"""

import argparse
import collections
import gzip
import json
import re


def load(path):
    opener = gzip.open if path.endswith(".gz") else open
    with opener(path, "rt") as f:
        prof = json.load(f)
    syms_path = re.sub(r"\.gz$", "", path)
    syms_path = re.sub(r"\.json$", "", syms_path) + ".json.syms.json"
    with open(syms_path) as f:
        syms = json.load(f)
    return prof, syms


def symbolicator(prof, syms):
    strings = syms["string_table"]
    by_lib = {}
    for lib in syms["data"]:
        addr_to_sym = dict(lib["known_addresses"])
        by_lib[lib["code_id"].upper()] = (addr_to_sym, lib["symbol_table"])
    libs = prof["libs"]

    def frames(lib_index, address):
        """Function names for one native frame, outermost first."""
        lib = libs[lib_index]
        entry = by_lib.get(lib.get("codeId", "").upper())
        if entry is None:
            return [f"{lib['name']}+{address:#x}"]
        addr_to_sym, table = entry
        i = addr_to_sym.get(address)
        if i is None:
            return [f"{lib['name']}+{address:#x}"]
        sym = table[i]
        # The sidecar keeps one inline chain per *symbol*, not per address,
        # so inline expansion would misattribute: use the real function.
        return [strings[sym["symbol"]]]

    return frames


def short(name):
    name = re.sub(r"::h[0-9a-f]{16}$", "", name)
    return name if len(name) <= 140 else name[:137] + "..."


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("profile")
    ap.add_argument("--top", type=int, default=40)
    ap.add_argument("--filter")
    ap.add_argument("--callers", help="regex: show who calls matching functions")
    ap.add_argument(
        "--within",
        help="regex: only count samples whose stack contains a matching function "
        "(percentages stay relative to all samples)",
    )
    args = ap.parse_args()
    prof, syms = load(args.profile)
    frames = symbolicator(prof, syms)
    self_t = collections.Counter()
    callers = collections.Counter()
    crx = re.compile(args.callers) if args.callers else None
    wrx = re.compile(args.within) if args.within else None
    incl_t = collections.Counter()
    total = 0
    for th in prof["threads"]:
        ft, st, rt = th["frameTable"], th["stackTable"], th["resourceTable"]
        ft_func = ft["func"]
        func_res = th["funcTable"]["resource"]
        cache = {}

        def stack_names(s):
            if s in cache:
                return cache[s]
            chain = []
            while s is not None:
                f = st["frame"][s]
                res = func_res[ft_func[f]]
                lib = rt["lib"][res] if res is not None and res >= 0 else None
                addr = ft["address"][f]
                if lib is None or addr is None or addr < 0:
                    chain.append(["?"])
                else:
                    chain.append(frames(lib, addr))
                s = st["prefix"][s]
            names = [n for fr in reversed(chain) for n in fr]
            cache[s] = names
            return names

        samples = th["samples"]
        weights = samples.get("weight") or [1] * samples["length"]
        for s, w in zip(samples["stack"], weights):
            if s is None:
                continue
            w = w or 1
            names = [short(n) for n in stack_names(s)]
            total += w
            if wrx and not any(wrx.search(n) for n in names):
                continue
            if names:
                self_t[names[-1]] += w
            for n in set(names):
                incl_t[n] += w
            if crx:
                # nearest caller outside the matched function (and std)
                for i in range(len(names) - 1, -1, -1):
                    if crx.search(names[i]):
                        for j in range(i - 1, -1, -1):
                            if not crx.search(names[j]) and re.search(
                                r"bibers|btparse|ucollate|perl_compat|latex_recode|relaxng", names[j]
                            ):
                                callers[names[j]] += w
                                break
                        break
    print(f"{total} samples")
    if crx:
        print(f"\n== callers of /{args.callers}/ ==")
        for n, c in callers.most_common(args.top):
            print(f"{100 * c / total:6.2f}%  {n}")
        return
    print("\n== self ==")
    for n, c in self_t.most_common(args.top):
        print(f"{100 * c / total:6.2f}%  {n}")
    print("\n== inclusive ==")
    rx = re.compile(args.filter) if args.filter else None
    shown = 0
    for n, c in incl_t.most_common():
        if rx and not rx.search(n):
            continue
        print(f"{100 * c / total:6.2f}%  {n}")
        shown += 1
        if shown >= args.top:
            break


if __name__ == "__main__":
    main()

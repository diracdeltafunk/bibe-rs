#!/usr/bin/env bash
# Differential test of crates/relaxng (with libxml2 interleave emulation)
# against libxml2 (O3's XML::LibXML) on mutated documents.
#
#   tools/oracle/probes/relaxng/differential.sh OUTDIR SEED COUNT SCHEMA.rng FILE...
#
# Prints the number of mutants, the libxml2 verdict counts, and every
# disagreement.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
out=$1 seed=$2 count=$3 schema=$4
shift 4
rm -rf "$out"
mkdir -p "$out"
python3 "$ROOT/tools/oracle/probes/relaxng/mutate.py" "$out/docs" "$seed" "$count" "$@" > "$out/list.txt"
"$ROOT/vendor/perl/bin/perl" "$ROOT/tools/oracle/probes/relaxng/verdicts.pl" "$schema" - < "$out/list.txt" > "$out/libxml2.txt"
RNG_LIBXML2=1 "$ROOT/target/release/examples/rngcheck" "$schema" - < "$out/list.txt" > "$out/relaxng.txt" 2> /dev/null
echo "mutants: $(wc -l < "$out/list.txt")"
cut -f2 "$out/libxml2.txt" | sort | uniq -c
if diff "$out/libxml2.txt" "$out/relaxng.txt" > "$out/diff.txt"; then
  echo "disagreements: 0"
else
  echo "disagreements: $(grep -c '^<' "$out/diff.txt")"
  cat "$out/diff.txt"
fi

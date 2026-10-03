#!/bin/sh
# Time an implementation over the benchmark inputs written by
#   python3 tools/corpus/gen_scale.py bench
# Usage: tools/corpus/bench.sh [DIR] (default target/bench-corpus)
#   BIBER=/path/to/bibers tools/corpus/bench.sh     (default: biber on PATH)
# Each input runs in a scratch copy, so the bench dirs stay clean.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
dir=${1:-$root/target/bench-corpus}
biber=${BIBER:-biber}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for c in "$dir"/*/; do
  name=$(basename "$c")
  rm -rf "$tmp/w" && cp -R "$c" "$tmp/w"
  start=$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
  (cd "$tmp/w" && "$biber" --noconf doc.bcf >/dev/null 2>&1) && rc=0 || rc=$?
  end=$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
  printf '%-20s exit %-3s %8.2fs\n' "$name" "$rc" "$(echo "$end - $start" | bc)"
done

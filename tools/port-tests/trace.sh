#!/usr/bin/env bash
# Run biber's t/*.t under oracle O3 with BiberTrace instrumentation.
# Output: tools/port-tests/out/trace/<tfile>/{events.jsonl,bbl/N.bbl,tap.txt}
# Usage: tools/port-tests/trace.sh [name ...]   (names without .t; default: all)
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PERL="$ROOT/vendor/perl/bin/perl"
SRC="$ROOT/vendor/perl-build/biber-src"
OUTROOT="$ROOT/tools/port-tests/out/trace"
unset PERL_LOCAL_LIB_ROOT PERL5OPT
export LANG=C LC_ALL=C
export PERL5LIB="$ROOT/tools/port-tests:$SRC/blib/lib:$SRC/blib/arch"
export PATH="$ROOT/vendor/perl/bin:$PATH"
export BIBER_DEV_TESTS="${BIBER_DEV_TESTS-0}"
cd "$SRC"
names=("$@")
if [ ${#names[@]} -eq 0 ]; then
  for t in t/*.t; do names+=("$(basename "$t" .t)"); done
fi
for b in "${names[@]}"; do
  out="$OUTROOT/$b"
  rm -rf "$out"; mkdir -p "$out"
  BIBERTRACE_OUT="$out" "$PERL" -MBiberTrace "t/$b.t" > "$out/tap.txt" 2>&1
  rc=$?
  plan=$(grep -m1 -E '^1\.\.[0-9]+' "$out/tap.txt" | sed 's/^1\.\.//; s/ .*//')
  ok=$(grep -cE '^ok ' "$out/tap.txt")
  nok=$(grep -cE '^not ok ' "$out/tap.txt")
  printf '%-26s plan=%-4s ok=%-4s notok=%-3s exit=%s\n' "$b" "${plan:-?}" "$ok" "$nok" "$rc"
done

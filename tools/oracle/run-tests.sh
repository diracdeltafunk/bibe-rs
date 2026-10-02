#!/usr/bin/env bash
# Run biber 2.22's own test suite (t/*.t) against oracle O3, one file at a
# time, from the built source copy (vendor/perl-build/biber-src), with LANG=C
# as biber's Build.PL ACTION_test does.  Prints per-file ok/not-ok counts and
# keeps full TAP output in vendor/perl-build/test-results/<file>.tap.
# Usage: tools/oracle/run-tests.sh [t/file.t ...]
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PERL="$ROOT/vendor/perl/bin/perl"
SRC="$ROOT/vendor/perl-build/biber-src"
OUT="$ROOT/vendor/perl-build/test-results"
unset PERL_LOCAL_LIB_ROOT PERL5OPT
export LANG=C LC_ALL=C
# Absolute @INC: the tests chdir into t/tdata, and the full-*.t / remote-files.t
# tests run `perl bin/biber` with the first perl on PATH.
export PERL5LIB="$SRC/blib/lib:$SRC/blib/arch"
export PATH="$ROOT/vendor/perl/bin:$PATH"
# Also run the end-to-end tests (full-bbl, full-bibtex, full-dot, remote-files;
# remote-files needs network access).  Set BIBER_DEV_TESTS=0 to skip them.
export BIBER_DEV_TESTS="${BIBER_DEV_TESTS-1}"
mkdir -p "$OUT"
cd "$SRC"
files=("$@"); [ ${#files[@]} -eq 0 ] && files=(t/*.t)
printf '%-28s %5s %5s %5s %6s %s\n' file plan ok notok exit secs
for t in "${files[@]}"; do
  b="$(basename "$t" .t)"
  s=$(date +%s)
  "$PERL" "$t" > "$OUT/$b.tap" 2>&1
  rc=$?
  e=$(( $(date +%s) - s ))
  plan=$(grep -m1 -E '^1\.\.[0-9]+' "$OUT/$b.tap" | sed 's/^1\.\.//; s/ .*//')
  ok=$(grep -cE '^ok ' "$OUT/$b.tap")
  nok=$(grep -cE '^not ok ' "$OUT/$b.tap")
  printf '%-28s %5s %5s %5s %6s %s\n' "$b" "${plan:-?}" "$ok" "$nok" "$rc" "$e"
done

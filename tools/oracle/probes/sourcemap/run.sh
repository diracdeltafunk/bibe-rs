#!/usr/bin/env bash
# Run the source-built biber (O3) recording every bibtex create_entry call:
# the entry before and after source mapping, entries created by maps, the
# section changes, and the warnings/debug messages of the map engine.
# Usage: tools/oracle/probes/sourcemap/run.sh <out.jsonl> <biber args...>
# Run it in the directory holding the case's inputs (like the corpus harness).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
PERL="$ROOT/vendor/perl/bin/perl"
SRC="$ROOT/vendor/perl-build/biber-src"
OUT="$1"; shift
rm -f "$OUT"
unset PERL5LIB PERL_LOCAL_LIB_ROOT PERL5OPT
export BIBERS_SMAP_OUT="$OUT"
exec "$PERL" -I"$SRC/blib/lib" -I"$ROOT/tools/oracle/probes/sourcemap" -MSourcemapDump "$SRC/blib/script/biber" "$@"

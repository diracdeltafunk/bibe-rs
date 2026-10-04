#!/usr/bin/env bash
# Run the source-built biber (O3) with per-phase state dumps.
# Usage: tools/oracle/probes/pipeline/run.sh <dump-dir> <biber args...>
# Run it in the directory holding the case's inputs (like the corpus harness).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
PERL="$ROOT/vendor/perl/bin/perl"
SRC="$ROOT/vendor/perl-build/biber-src"
DUMP="$1"; shift
mkdir -p "$DUMP"
rm -f "$DUMP"/*.json
unset PERL5LIB PERL_LOCAL_LIB_ROOT PERL5OPT
export BIBERS_DUMP_DIR="$(cd "$DUMP" && pwd)"
exec "$PERL" -I"$SRC/blib/lib" -I"$ROOT/tools/oracle/probes/pipeline" -MBiberPhaseDump "$SRC/blib/script/biber" "$@"

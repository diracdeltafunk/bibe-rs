#!/usr/bin/env bash
# Run replay.pl under oracle O3. Usage: tools/port-tests/replay.sh <case> <out.bbl> [--sortdebug]
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="$ROOT/vendor/perl-build/biber-src"
unset PERL_LOCAL_LIB_ROOT PERL5OPT
export LANG=C LC_ALL=C PORT_ROOT="$ROOT"
export PERL5LIB="$SRC/blib/lib:$SRC/blib/arch"
exec "$ROOT/vendor/perl/bin/perl" "$ROOT/tools/port-tests/replay.pl" "$@"

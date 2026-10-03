#!/bin/sh
# Run probe.pl under oracle O3 (see probe.pl for the modes).
ROOT=${BIBE_RS_ROOT:-/Users/ben/bibe-rs}
unset PERL5LIB PERL5OPT
exec "$ROOT/vendor/perl/bin/perl" -I"$ROOT/vendor/perl-build/biber-src/blib/lib" \
  "$(dirname "$0")/probe.pl" "$@"

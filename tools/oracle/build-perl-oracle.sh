#!/usr/bin/env bash
# Build oracle O3: a source-built biber 2.22 on a pinned perl 5.42.0, with CPAN
# dependencies pinned to the versions bundled in the TeX Live 2026 biber 2.22
# PAR binary.  See tools/oracle/README.md.
#
# Everything is installed under vendor/perl (prefix) and built under
# vendor/perl-build.  Nothing is installed system-wide; ~/.cpan, ~/perl5 and
# vendor/biber are never touched.
#
# Idempotent: each phase is skipped when its result is already present.
# Usage:  tools/oracle/build-perl-oracle.sh [phase...]
#   phases: perl cpanm deps biber lock   (default: all, in that order)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="$ROOT/vendor/perl"
BUILD="$ROOT/vendor/perl-build"
ORACLE="$ROOT/tools/oracle"
BIBER_GIT="$ROOT/vendor/biber"
BIBER_SRC="$BUILD/biber-src"
PERL_VERSION=5.42.0
PERL_SHA256=e093ef184d7f9a1b9797e2465296f55510adb6dab8842b0c3ed53329663096dc
CPANM_VERSION=1.7048
JOBS="$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)"
PERL="$PREFIX/bin/perl"

# Hermetic environment: never pick up the user's local::lib / cpan config.
unset PERL5LIB PERL_LOCAL_LIB_ROOT PERL_MB_OPT PERL_MM_OPT PERL5OPT PERL_CPANM_OPT || true
export PERL_CPANM_HOME="$BUILD/cpanm-home"
export PERL_MM_USE_DEFAULT=1
export LANG=C LC_ALL=C
export PATH="$PREFIX/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

mkdir -p "$BUILD" "$PREFIX"
log() { printf '==> %s\n' "$*" >&2; }

fetch() { # url dest
  [ -s "$2" ] || { log "fetch $1"; curl -fsSL --retry 3 -o "$2.part" "$1" && mv "$2.part" "$2"; }
}

# ---------------------------------------------------------------- perl
phase_perl() {
  if [ -x "$PERL" ] && "$PERL" -e 'exit($^V eq "v'$PERL_VERSION'" ? 0 : 1)'; then
    log "perl $PERL_VERSION already installed"; return
  fi
  local tgz="$BUILD/perl-$PERL_VERSION.tar.gz"
  fetch "https://www.cpan.org/src/5.0/perl-$PERL_VERSION.tar.gz" "$tgz"
  echo "$PERL_SHA256  $tgz" | shasum -a 256 -c -
  rm -rf "$BUILD/perl-$PERL_VERSION"
  tar -C "$BUILD" -xzf "$tgz"
  cd "$BUILD/perl-$PERL_VERSION"
  # Same Configure flags as the PAR binary's perl (Homebrew perl 5.42.0):
  # -Dusethreads -Duseshrplib -Duselargefiles, installstyle=lib/perl5.
  # Paths differ (prefix = vendor/perl); man pages are not installed.
  ./Configure -des \
    -Dprefix="$PREFIX" \
    -Dinstallstyle=lib/perl5 \
    -Duseshrplib -Duselargefiles -Dusethreads \
    -Dman1dir=none -Dman3dir=none \
    > "$BUILD/perl-configure.log" 2>&1
  make -j"$JOBS" > "$BUILD/perl-make.log" 2>&1
  make install > "$BUILD/perl-install.log" 2>&1
  cd "$ROOT"
  "$PERL" -v | sed -n 2p
}

# ---------------------------------------------------------------- cpanm
phase_cpanm() {
  if [ -x "$PREFIX/bin/cpanm" ]; then log "cpanm present"; return; fi
  local tgz="$BUILD/App-cpanminus-$CPANM_VERSION.tar.gz"
  fetch "https://cpan.metacpan.org/authors/id/M/MI/MIYAGAWA/App-cpanminus-$CPANM_VERSION.tar.gz" "$tgz"
  rm -rf "$BUILD/App-cpanminus-$CPANM_VERSION"
  tar -C "$BUILD" -xzf "$tgz"
  ( cd "$BUILD/App-cpanminus-$CPANM_VERSION" && "$PERL" Makefile.PL && make && make install ) \
    > "$BUILD/cpanm-install.log" 2>&1
}

# ---------------------------------------------------------------- deps
# tools/oracle/deps.pinned lists one CPAN distribution path per line
# (AUTHOR/Dist-Name-VERSION.tar.gz), in install order.  Each is installed with
# --notest (biber's own test suite is the acceptance test) and without
# pulling newer versions of already-pinned deps (cpanm resolves prerequisites
# against what is already installed, so order matters: leaves first).
dist_installed() { # Dist-Name-VERSION -> true if recorded as installed
  grep -qxF "$1" "$PREFIX/.oracle-installed-dists" 2>/dev/null
}

# OpenSSL 3 for Net::SSLeay (https remote datasources only).  The PAR links
# Net::SSLeay against macOS's /usr/lib/libssl (LibreSSL), whose headers are not
# in the SDK, so we need some OpenSSL 3 install: $OPENSSL_PREFIX, else an
# existing Homebrew openssl@3 keg (used read-only; nothing is installed).
# OpenSSL 4 does not work with Net::SSLeay 1.94.  Without one, the [ssl]
# dists are skipped and biber-src cannot fetch https:// datasources.
find_openssl() {
  local p
  for p in "${OPENSSL_PREFIX:-}" /opt/homebrew/opt/openssl@3 /usr/local/opt/openssl@3; do
    [ -n "$p" ] && [ -f "$p/include/openssl/ssl.h" ] && { echo "$p"; return 0; }
  done
  return 1
}

cpanm_() {
  "$PERL" "$PREFIX/bin/cpanm" --notest --no-man-pages --no-lwp "$@"
}

phase_deps() {
  local raw line dist url ssl openssl=""
  openssl="$(find_openssl || true)"
  touch "$PREFIX/.oracle-installed-dists"
  while IFS= read -r raw || [ -n "$raw" ]; do
    ssl=0; case "$raw" in *"[ssl]"*) ssl=1 ;; esac
    line="${raw%%#*}"; line="$(echo "$line" | xargs)"; [ -z "$line" ] && continue
    dist="$(basename "$line")"; dist="${dist%.tar.gz}"; dist="${dist%.tgz}"; dist="${dist%.zip}"
    dist_installed "$dist" && continue
    if [ "$ssl" = 1 ] && [ -z "$openssl" ]; then
      log "SKIP $dist: no OpenSSL 3 found (set OPENSSL_PREFIX)"; continue
    fi
    url="$BUILD/dists/$(basename "$line")"
    mkdir -p "$BUILD/dists"
    fetch "https://cpan.metacpan.org/authors/id/$line" "$url"
    log "install $dist"
    if [ "$ssl" = 1 ]; then
      OPENSSL_PREFIX="$openssl" cpanm_ "$url" >> "$BUILD/deps-install.log" 2>&1
    else
      cpanm_ "$url" >> "$BUILD/deps-install.log" 2>&1
    fi || { log "FAILED $dist (see $BUILD/deps-install.log and $PERL_CPANM_HOME/latest-build/build.log)"; exit 1; }
    echo "$dist" >> "$PREFIX/.oracle-installed-dists"
  done < "$ORACLE/deps.pinned"
}

# ---------------------------------------------------------------- biber
# Copy the pinned biber tree (vendor/biber is never written to) and build it
# there so that data files land in blib/lib/Biber as an installed biber
# would have them.  The wrapper tools/oracle/biber-src runs from this copy.
phase_biber() {
  local rev; rev="$(git -C "$BIBER_GIT" rev-parse HEAD)"
  if [ -f "$BIBER_SRC/.oracle-rev" ] && [ "$(cat "$BIBER_SRC/.oracle-rev")" = "$rev" ] \
     && [ -f "$BIBER_SRC/blib/lib/Biber.pm" ]; then
    log "biber-src up to date ($rev)"; return
  fi
  rm -rf "$BIBER_SRC"
  mkdir -p "$BIBER_SRC"
  git -C "$BIBER_GIT" archive --format=tar HEAD | tar -C "$BIBER_SRC" -xf -
  ( cd "$BIBER_SRC" && "$PERL" Build.PL && ./Build ) > "$BUILD/biber-build.log" 2>&1
  echo "$rev" > "$BIBER_SRC/.oracle-rev"
}

# ---------------------------------------------------------------- lock
phase_lock() {
  # Every CPAN dist cpanm installed must be one we pinned (no silent upgrades
  # or extra dists pulled in as prerequisites).
  local meta extra
  meta="$("$PERL" -MConfig -e 'print $Config{installsitearch}')/.meta"
  extra="$(ls "$meta" | grep -vxF -f "$PREFIX/.oracle-installed-dists" | grep -v '^App-cpanminus-' || true)"
  if [ -n "$extra" ]; then log "WARNING: unpinned dists installed:"; echo "$extra" >&2; fi
  "$PERL" "$ORACLE/list-versions.pl" "$BIBER_SRC" > "$ORACLE/versions.lock"
  log "wrote $ORACLE/versions.lock"
}

phases=("$@"); [ ${#phases[@]} -eq 0 ] && phases=(perl cpanm deps biber lock)
for p in "${phases[@]}"; do "phase_$p"; done

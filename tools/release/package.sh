#!/usr/bin/env bash
# Package a bibers release binary with its licence and documentation.
#
#     tools/release/package.sh VERSION PLATFORM BINARY [OUTDIR]
#
# Writes OUTDIR/bibers-VERSION-PLATFORM.tar.gz (or .zip when BINARY ends in
# .exe) holding bibers[.exe], LICENSE, README.md and docs/compat.md in a
# top-level bibers-VERSION-PLATFORM/ directory, and prints the archive path.
# OUTDIR defaults to target/dist. Used by .github/workflows/release.yml.
set -euo pipefail

if [[ $# -lt 3 ]]; then
    echo "usage: $0 VERSION PLATFORM BINARY [OUTDIR]" >&2
    exit 2
fi
version=$1
platform=$2
binary=$3
root=$(cd "$(dirname "$0")/../.." && pwd)
outdir=${4:-$root/target/dist}

name="bibers-$version-$platform"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/$name/docs" "$outdir"

exe=bibers
[[ $binary == *.exe ]] && exe=bibers.exe
cp "$binary" "$stage/$name/$exe"
chmod 755 "$stage/$name/$exe"
cp "$root/LICENSE" "$root/README.md" "$stage/$name/"
cp "$root/docs/compat.md" "$stage/$name/docs/"

outdir=$(cd "$outdir" && pwd)
if [[ $exe == bibers.exe ]]; then
    archive="$outdir/$name.zip"
    rm -f "$archive"
    if command -v 7z >/dev/null; then
        (cd "$stage" && 7z a -tzip -bso0 "$archive" "$name")
    else
        (cd "$stage" && zip -qr "$archive" "$name")
    fi
else
    archive="$outdir/$name.tar.gz"
    # COPYFILE_DISABLE: no macOS resource-fork (._*) entries.
    COPYFILE_DISABLE=1 tar -C "$stage" -czf "$archive" "$name"
fi
echo "$archive"

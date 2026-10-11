# Noncharacter cases of tests/corpus/io/ (UTF-8 input is read with
# PerlIO::utf8_strict, which rejects noncharacters; LaTeX-decoded ones are
# turned into U+FFFD when biber rewrites its temp file).
set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
C="$ROOT/tests/corpus/io"
BCF="$ROOT/tests/corpus/ported/encoding-4/encoding6.bcf"
mk() {
  local name="$1" desc="$2" bib="$3"
  mkdir -p "$C/$name"
  cp "$BCF" "$C/$name/encoding6.bcf"
  printf '%b' "$bib" > "$C/$name/encoding6.bib"
  cat > "$C/$name/case.toml" <<EOF
description = "$desc"
source = "tools/corpus/gen_io_nonchar.sh"
args = ["--noconf", "encoding6.bcf"]
outputs = ["encoding6.bbl"]
tags = ["suite:io", "feature:encoding"]
EOF
}
mk utf8-noncharacter-decoded '\\char of noncharacters (U+FDD0, U+1FFFF): U+FFFD in the .bbl' \
  '@misc{a, title={x\\char"FDD0 y \\char"1FFFF z}}\n@misc{b, title={ok}}\n'
mk utf8-noncharacter-literal 'A literal noncharacter (U+FDD1) in a UTF-8 .bib: read error' \
  '@misc{a, title={x \xef\xb7\x91 z}}\n'
# Perl appends ' at FILE line N.' to the read error (docs/compat.md DC-046).
echo 'allow = ["DC-046"]' >> "$C/utf8-noncharacter-literal/case.toml"

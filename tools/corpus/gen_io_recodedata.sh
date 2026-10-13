set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
C="$ROOT/tests/corpus/io"
BCF="$ROOT/tests/corpus/ported/encoding-4/encoding6.bcf"
mkdir -p "$C"

custom_xml() {
cat <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<texmap>
  <maps type="letters" set="base,mine">
    <map><from>ss</from>                    <to hex="DF">ß</to></map>
    <map><from>myoe</from>                  <to hex="153">œ</to></map>
    <map><from preferred="1">oe</from>      <to hex="153">œ</to></map>
    <map><from>l</from>                     <to hex="142">ł</to></map>
    <map><from>o</from>                     <to hex="F8">ø</to></map>
  </maps>
  <maps type="diacritics" set="base">
    <map><from>"</from>                     <to hex="308">̈</to></map>
    <map><from>'</from>                     <to hex="301">́</to></map>
  </maps>
  <maps type="diacritics" set="mine">
    <map><from>`</from>                     <to hex="300">̀</to></map>
  </maps>
  <maps type="symbols" set="mine">
    <map><from>textuparrow</from>           <to hex="2191">↑</to></map>
    <map><from>textmyarrow</from>           <to hex="2191">↑</to></map>
  </maps>
  <maps type="punctuation" set="base,mine">
    <map><from preferred="1" raw="1">--</from> <to hex="2013">–</to></map>
  </maps>
  <decode_exclude>
    <char>o</char>
  </decode_exclude>
  <encode_exclude>
    <char>{</char>
  </encode_exclude>
</texmap>
EOF
}

bib() {
cat <<'EOF'
% This is a UTF-8 encoded file
@BOOK{test,
  AUTHOR = {Edward \"Encalcer and Jo \ss{}e and \o{}ystein \'Ab},
  YEAR = {1999},
  TITLE = {\textuparrow\ à \'e titlé \myoe\ \l{} ł – \textmyarrow\ \`a ø},
  PUBLISHER = {A press -- Œuvres \oe{} Straße}
}
EOF
}

mkcase() {
  local name="$1" desc="$2"; shift 2
  local d="$C/$name"
  mkdir -p "$d"
  cp "$BCF" "$d/encoding6.bcf"
  bib > "$d/encoding6.bib"
  local args='"--noconf"'
  for a in "$@"; do args="$args, \"$a\""; done
  cat > "$d/case.toml" <<EOF
description = "$desc"
source = "bibers M4 io package (DC-045): user recode data file; inputs from t/tdata/encoding6.bcf"
args = [$args, "encoding6.bcf"]
outputs = ["encoding6.bbl"]
tags = ["suite:io", "feature:recodedata"]
EOF
}

mkcase recodedata-custom-base "--recodedata with a small custom file, default sets (base/base), --output-safechars" \
  --recodedata=myrecode.xml --output-safechars
custom_xml > "$C/recodedata-custom-base/myrecode.xml"
mkcase recodedata-custom-mine "--recodedata with a custom set name for decoding and encoding" \
  --recodedata=myrecode.xml --decodecharsset=mine --output-safecharsset=mine --output-safechars
custom_xml > "$C/recodedata-custom-mine/myrecode.xml"
mkcase recodedata-custom-mixed "--recodedata: decode with 'mine', encode with 'base'" \
  --recodedata=myrecode.xml --decodecharsset=mine --output-safechars
custom_xml > "$C/recodedata-custom-mine/myrecode.xml"
custom_xml > "$C/recodedata-custom-mixed/myrecode.xml"
mkcase recodedata-custom-latin1 "--recodedata with latin1 output: forced recoding of unencodable entries uses the user data" \
  --recodedata=myrecode.xml --output-encoding=latin1
custom_xml > "$C/recodedata-custom-latin1/myrecode.xml"
mkcase recodedata-custom-noset "--recodedata with a set name the file does not define (no recoding at all)" \
  --recodedata=myrecode.xml --decodecharsset=full --output-safecharsset=full --output-safechars
custom_xml > "$C/recodedata-custom-noset/myrecode.xml"
# A modified copy of biber's own data: overrides one decode and one encode mapping.
mkcase recodedata-override-full "--recodedata with a modified copy of recode_data.xml (overridden maps), full sets" \
  --recodedata=myrecode.xml --decodecharsset=full --output-safecharsset=full --output-safechars
sed -e 's|<map><from>ss</from>                               <to hex="DF">ß</to></map>|<map><from>ss</from> <to hex="1E9E">ẞ</to></map><map><from preferred="1">textmyss</from> <to hex="DF">ß</to></map>|' \
    "$ROOT/crates/latex-recode/data/recode_data.xml" > "$C/recodedata-override-full/myrecode.xml"
grep -c 'textUP\|1E9E' "$C/recodedata-override-full/myrecode.xml"

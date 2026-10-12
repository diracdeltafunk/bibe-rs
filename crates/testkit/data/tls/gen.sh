# Regenerate the test TLS material used by bibers_testkit::servers (remote
# data source tests). Not secret: test-only keys, valid for 100 years.
set -eu
cd "$(dirname "$0")"
openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem -days 36500 \
  -subj "/CN=bibers test CA" -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign"
mk() {
  local name="$1" san="$2"
  openssl req -newkey rsa:2048 -nodes -keyout "$name.key" -out "$name.csr" -subj "/CN=$name"
  printf 'subjectAltName=%s\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' "$san" > "$name.ext"
  openssl x509 -req -in "$name.csr" -CA ca.pem -CAkey ca.key -CAcreateserial -out "$name.pem" \
    -days 36500 -extfile "$name.ext"
  rm "$name.csr" "$name.ext"
}
mk localhost "DNS:localhost,IP:127.0.0.1"
mk wronghost "DNS:wrong.invalid"
rm -f ca.srl

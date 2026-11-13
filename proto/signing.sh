#!/bin/sh
# The M0 code signing identity: a self-signed certificate named
# "Mado Self-Signed (M0)" in its own keychain under .mado-dev/signing/, never
# in the login keychain. It signs prototype builds only; the release identity
# (15 §3) belongs to the maintainer.
#
#   proto/signing.sh create         make the certificate and keychain once
#   proto/signing.sh sign <path>    sign an app bundle as 15 §3 describes
#   proto/signing.sh show           print the identity's SHA-1
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
dir="$root/.mado-dev/signing"
keychain="$dir/m0.keychain-db"
password_file="$dir/keychain-password"
name="Mado Self-Signed (M0)"
identifier="app.mado.proto"

die() {
  printf 'signing.sh: %s\n' "$1" >&2
  exit 1
}

unlock() {
  [ -f "$keychain" ] || die "no keychain; run: proto/signing.sh create"
  security unlock-keychain -p "$(cat "$password_file")" "$keychain"
}

sha1() {
  security find-certificate -c "$name" -Z "$keychain" | awk '/^SHA-1 hash:/ { print $3 }'
}

create() {
  [ -f "$keychain" ] && die "$keychain already exists"
  mkdir -p "$dir"
  chmod 700 "$dir"
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  # macOS's LibreSSL writes a PKCS#12 that `security import` reads.
  /usr/bin/openssl rand -hex 24 >"$password_file"
  chmod 600 "$password_file"
  cat >"$work/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $name
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
EOF
  /usr/bin/openssl req -x509 -newkey rsa:3072 -nodes -days 7300 \
    -config "$work/cert.cnf" -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
  p12_password=$(/usr/bin/openssl rand -hex 16)
  /usr/bin/openssl pkcs12 -export -name "$name" -inkey "$work/key.pem" -in "$work/cert.pem" \
    -passout "pass:$p12_password" -out "$work/identity.p12"
  security create-keychain -p "$(cat "$password_file")" "$keychain"
  security set-keychain-settings "$keychain"
  unlock
  security import "$work/identity.p12" -k "$keychain" -P "$p12_password" -T /usr/bin/codesign >/dev/null
  # Let codesign use the key without a prompt.
  security set-key-partition-list -S apple-tool:,apple: -s -k "$(cat "$password_file")" "$keychain" >/dev/null
  printf '%s  %s\n' "$(sha1)" "$name"
}

sign() {
  target=$1
  [ -e "$target" ] || die "no such path: $target"
  unlock
  hash=$(sha1)
  [ -n "$hash" ] || die "no certificate named $name in $keychain"
  codesign --force --options runtime --timestamp=none \
    --entitlements "$root/proto/macos/entitlements.plist" \
    --keychain "$keychain" --sign "$hash" \
    -r="designated => identifier \"$identifier\" and certificate leaf = H\"$hash\"" \
    "$target"
  codesign --verify --deep --strict "$target"
}

case "${1:-}" in
  create) create ;;
  sign) [ $# -eq 2 ] || die "usage: proto/signing.sh sign <path>"; sign "$2" ;;
  show) unlock; printf '%s  %s\n' "$(sha1)" "$name" ;;
  *) die "usage: proto/signing.sh create | sign <path> | show" ;;
esac

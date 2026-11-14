#!/bin/sh
# Builds, signs and packages Mado for one macOS target (spec 15 §2, §3, §6):
#
#   scripts/release/macos.sh <aarch64|x86_64> <version> <out-dir>
#
# Needs MACOS_CERT_P12 (base64) and MACOS_CERT_PASSWORD for the self-signed
# identity, and TAURI_SIGNING_PRIVATE_KEY (and _PASSWORD) for the updater
# signature. Writes to <out-dir>:
#   Mado_<version>_<arch>.dmg
#   Mado_<arch>.app.tar.gz and .sig    (the updater archive)
#   SHA256SUMS-<arch>
# The UI and page scripts must be built first (bun run build).
set -eu

arch=${1:?arch}
version=${2:?version}
out=${3:?out-dir}
root=$(cd "$(dirname "$0")/../.." && pwd)
target="$arch-apple-darwin"
sha1=$(tr -d '[:space:]' <"$root/app/macos/cert-sha1.txt")
identifier=app.mado.Mado
mkdir -p "$out"
out=$(cd "$out" && pwd)

work=$(mktemp -d)
keychain="$work/release.keychain-db"
searched=$(security list-keychains -d user | tr -d '"' | tr '\n' ' ')
cleanup() {
  # shellcheck disable=SC2086 # one path per word, as listed
  security list-keychains -d user -s $searched 2>/dev/null || true
  security delete-keychain "$keychain" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

# 1. Build the bundle, unsigned.
cd "$root/app"
bunx tauri build --target "$target" --bundles app --no-sign \
  --config "{\"version\":\"$version\"}"
app="$root/target/$target/release/bundle/macos/Mado.app"
[ -d "$app" ] || { echo "macos.sh: no bundle at $app" >&2; exit 1; }

# 2. Import the identity into a throwaway keychain and sign with the
# explicit designated requirement, so Keychain items stay readable across
# updates.
keychain_password=$(openssl rand -hex 16)
printf '%s' "$MACOS_CERT_P12" | base64 --decode >"$work/identity"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$work/identity" -f pkcs12 -k "$keychain" -P "$MACOS_CERT_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple: -s -k "$keychain_password" "$keychain" >/dev/null
# codesign finds the identity only in a keychain on the search list.
# shellcheck disable=SC2086 # one path per word, as listed
security list-keychains -d user -s "$keychain" $searched
found=$(security find-certificate -a -Z "$keychain" | awk '/^SHA-1 hash:/ { print tolower($3) }')
case " $found " in
  *" $sha1 "*) ;;
  *) echo "macos.sh: the identity is not the one in app/macos/cert-sha1.txt" >&2; exit 1 ;;
esac
codesign --force --options runtime --timestamp=none \
  --entitlements "$root/app/macos/entitlements.plist" \
  --keychain "$keychain" --sign "$sha1" \
  -r="designated => identifier \"$identifier\" and certificate leaf = H\"$sha1\"" \
  "$app"
codesign --verify --deep --strict "$app"
codesign -d -r- "$app" 2>&1 | grep -q "certificate leaf = H\"$sha1\""

# 3. The updater archive and its minisign signature.
tarball="$out/Mado_$arch.app.tar.gz"
tar -C "$(dirname "$app")" -czf "$tarball" Mado.app
# Bound to the version, so a manifest cannot pair another version number
# with this archive (requireSignedVersion in tauri.conf.json).
bunx tauri signer sign --app-version "$version" "$tarball" >/dev/null
[ -f "$tarball.sig" ] || { echo "macos.sh: no signature" >&2; exit 1; }

# 4. The disk image.
dmg="$out/Mado_${version}_$arch.dmg"
staging="$work/dmg"
mkdir -p "$staging"
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "Mado" -srcfolder "$staging" -fs HFS+ -format UDZO -ov "$dmg" >/dev/null

(cd "$out" && shasum -a 256 "$(basename "$dmg")" "$(basename "$tarball")" >"SHA256SUMS-$arch")
printf '%s\n' "$dmg" "$tarball"

#!/bin/sh
# Builds a signed MadoProto.app from the release binary.
#
#   proto/build-app.sh [name] [build-version] [adhoc]
#
# name defaults to MadoProto, so the app lands in proto/target/MadoProto.app.
# A second name and version make a second build, with its own binary, signed
# with the same identity (experiment 11); "adhoc" signs it ad hoc instead.
set -eu

cd "$(dirname "$0")"
name=${1:-MadoProto}
version=${2:-1}
signer=${3:-self-signed}
app="target/$name.app"

# The version goes into the binary too, so each build has its own CDHash.
MADO_PROTO_BUILD=$version cargo build --release --locked
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/mado-proto "$app/Contents/MacOS/mado-proto"
sed "s/BUILD_VERSION/$version/" macos/Info.plist >"$app/Contents/Info.plist"
if [ "$signer" = adhoc ]; then
  codesign --force --sign - "$app"
else
  ./signing.sh sign "$app"
fi
printf '%s\n' "$(pwd)/$app"

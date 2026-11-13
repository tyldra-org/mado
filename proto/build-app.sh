#!/bin/sh
# Builds a signed MadoProto.app from the release binary.
#
#   proto/build-app.sh [name] [build-version]
#
# name defaults to MadoProto, so the app lands in proto/target/MadoProto.app.
# A second name and version make a second build signed with the same
# identity (experiment 11).
set -eu

cd "$(dirname "$0")"
name=${1:-MadoProto}
version=${2:-1}
app="target/$name.app"

cargo build --release --locked
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/mado-proto "$app/Contents/MacOS/mado-proto"
sed "s/BUILD_VERSION/$version/" macos/Info.plist >"$app/Contents/Info.plist"
./signing.sh sign "$app"
printf '%s\n' "$(pwd)/$app"

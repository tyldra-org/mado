#!/bin/sh
# The M0 checks that need someone at the Mac: each shows a macOS or
# 1Password prompt, or needs an account. Run it yourself, in one sitting:
#
#   proto/supervised.sh
#
# Apps are started with open(1), so macOS asks on behalf of Mado Proto, not
# the terminal. Results land in proto/out/*-supervised.json.
set -eu

cd "$(dirname "$0")"
app=target/MadoProto.app
out=$(pwd)/out
mkdir -p "$out"
[ -d "$app" ] || ./build-app.sh >/dev/null

step() {
  printf '\n%s\n' "$1"
  printf 'Press Return to start. '
  read -r _
}

step "1/3 Speech recognition (row 15). macOS asks Mado Proto for Speech Recognition and then the Microphone. Click Allow for both."
open -n -W --env MADO_PROTO_SUPERVISED=1 "$app" --args --exp permissions --out "$out/permissions-supervised.json"

step "2/3 Password managers (row 20). First: in 1Password, turn on Settings > Developer > Integrate with 1Password CLI, and run 'bw login' in a terminal. Then approve 1Password's prompt when it appears."
open -n -W --env MADO_PROTO_SUPERVISED=1 "$app" --args --exp passwords --out "$out/passwords-supervised.json"

step "3/3 DRM (row 9). Mado Proto opens with an address bar. For each service, sign in, play one title, and note 'plays' or the error shown: Netflix, Disney+, Max, Prime Video, Hulu, Apple TV+, Apple Music, Spotify, YouTube Premium, Crunchyroll. Quit Mado Proto when done."
open -n -W "$app"

printf '\nDone. Send the two JSON files and your DRM notes back to the M0 report.\n'

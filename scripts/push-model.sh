#!/usr/bin/env bash
# Copy models/phonon-31m-en into the app's data directory on the phone,
# where the app looks for it (<data dir>/phonon-31m-en, Android's
# internal files directory). Needs the debuggable APK installed, for run-as.
#
#   scripts/push-model.sh [<model dir>]   # default: $VOICEXN_MODELS/phonon-31m-en
#
# For another app, from this checkout (plan 20261029_01):
#   APP_ID         the debuggable app to run-as (default dev.tmzt.voicexn)
#   VOICEXN_MODELS the folder holding phonon-31m-en and stt-1b-en_fr
#                  (default <this repository>/models)
#   STAGE          where files are staged on the phone (default
#                  /data/local/tmp/voicexn-tts; emptied after the copy)
#
#   APP_ID=com.tmzt.roundtwo ~/src/libvoicexn/scripts/push-model.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODELS="${VOICEXN_MODELS:-$ROOT/models}"
SRC="${1:-$MODELS/phonon-31m-en}"
APP_ID="${APP_ID:-dev.tmzt.voicexn}"
TMP="${STAGE:-/data/local/tmp/voicexn-tts}"

[[ -f "$SRC/model.q8.gguf" ]] || { echo "error: no model in $SRC; libvoicexn's scripts/fetch-model.sh fetches it" >&2; exit 1; }
adb shell rm -rf "$TMP"
adb shell mkdir -p "$TMP/phonon-31m-en"
# `/.`: the contents, also when the folder is a link to a shared one.
adb push "$SRC/." "$TMP/phonon-31m-en/" >/dev/null
adb shell run-as "$APP_ID" sh -c "'rm -rf files/phonon-31m-en && mkdir -p files && cp -r $TMP/phonon-31m-en files/'"
adb shell rm -rf "$TMP"
echo "==> $(adb shell run-as "$APP_ID" ls files/phonon-31m-en/voices | wc -l | tr -d ' ') voices in $APP_ID files/phonon-31m-en"

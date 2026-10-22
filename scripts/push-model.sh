#!/usr/bin/env bash
# Copy models/phonon-31m-en into the app's data directory on the phone,
# where the app looks for it (<data dir>/phonon-31m-en, Android's
# internal files directory). Needs the debuggable APK installed, for run-as.
#
#   scripts/push-model.sh [<model dir>]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-$ROOT/models/phonon-31m-en}"
APP_ID=dev.tmzt.voicexn
TMP=/data/local/tmp/voicexn

[[ -f "$SRC/model.q8.gguf" ]] || { echo "error: no model in $SRC; run scripts/fetch-model.sh" >&2; exit 1; }
adb shell rm -rf "$TMP"
adb shell mkdir -p "$TMP"
adb push "$SRC" "$TMP/" >/dev/null
adb shell run-as "$APP_ID" sh -c "'rm -rf files/phonon-31m-en && mkdir -p files && cp -r $TMP/phonon-31m-en files/'"
adb shell rm -rf "$TMP"
echo "==> $(adb shell run-as "$APP_ID" ls files/phonon-31m-en/voices | wc -l | tr -d ' ') voices in $APP_ID files/phonon-31m-en"

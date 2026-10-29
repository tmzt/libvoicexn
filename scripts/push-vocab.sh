#!/usr/bin/env bash
# Copy vocab.txt, the STT boost list (plan 20261022_04), into the app's data
# directory, where an `stt-moshi-xn-boost` build reads it at load.
#
#   scripts/push-vocab.sh [<vocab file>]   # default: <this repository>/vocab.txt
#
# For another app, from this checkout (plan 20261029_01):
#   APP_ID   the debuggable app to run-as (default dev.tmzt.voicexn)
#   STAGE    the staging file on the phone (default
#            /data/local/tmp/voicexn-vocab.txt; removed after the copy)
#
#   APP_ID=com.tmzt.roundtwo ~/src/libvoicexn/scripts/push-vocab.sh my-vocab.txt
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-$ROOT/vocab.txt}"
APP_ID="${APP_ID:-dev.tmzt.voicexn}"
TMP="${STAGE:-/data/local/tmp/voicexn-vocab.txt}"
adb push "$SRC" "$TMP" >/dev/null
adb shell run-as "$APP_ID" cp "$TMP" files/vocab.txt
adb shell rm "$TMP"
echo "==> $APP_ID files/vocab.txt: $(adb shell run-as "$APP_ID" grep -vc '^#' files/vocab.txt) phrases"

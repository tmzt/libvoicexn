#!/usr/bin/env bash
# Copy vocab.txt, the STT boost list (plan 20261022_04), into the app's data
# directory, where an `stt-moshi-xn-boost` build reads it at load.
#
#   scripts/push-vocab.sh [<vocab file>]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-$ROOT/vocab.txt}"
APP_ID=dev.tmzt.voicexn
adb push "$SRC" /data/local/tmp/voicexn-vocab.txt >/dev/null
adb shell run-as "$APP_ID" cp /data/local/tmp/voicexn-vocab.txt files/vocab.txt
adb shell rm /data/local/tmp/voicexn-vocab.txt
echo "==> $APP_ID files/vocab.txt: $(adb shell run-as "$APP_ID" grep -vc '^#' files/vocab.txt) phrases"

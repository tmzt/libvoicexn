#!/usr/bin/env bash
# Copy the STT model (what the app loads: model.q4k.gguf, which holds the LM
# and Mimi's encoder, config.json and tokenizer.model; about 1.02 GB) into
# the app's data directory, <data dir>/stt-1b-en_fr. Needs the debuggable
# APK installed, for run-as. A mimi.safetensors left there by an older push
# is deleted: nothing reads it since plan 20261022_06.
#
#   scripts/push-stt.sh [<model dir>]           # default: $VOICEXN_MODELS/stt-1b-en_fr
#   STT_QUANTS="q8_0 q4_0" scripts/push-stt.sh  # other quantizations
#
# For another app, from this checkout (plan 20261029_01):
#   APP_ID         the debuggable app to run-as (default dev.tmzt.voicexn)
#   VOICEXN_MODELS the folder holding phonon-31m-en and stt-1b-en_fr
#                  (default <this repository>/models)
#   STAGE          where files are staged on the phone (default
#                  /data/local/tmp/voicexn-stt); kept, and shared by every app
#
#   APP_ID=com.tmzt.roundtwo ~/src/libvoicexn/scripts/push-stt.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODELS="${VOICEXN_MODELS:-$ROOT/models}"
SRC="${1:-$MODELS/stt-1b-en_fr}"
APP_ID="${APP_ID:-dev.tmzt.voicexn}"
TMP="${STAGE:-/data/local/tmp/voicexn-stt}"

[[ -f "$SRC/config.json" ]] || { echo "error: no model in $SRC; libvoicexn's scripts/fetch-stt.sh fetches it" >&2; exit 1; }
# A file already staged at its full size is not pushed again, and a push
# is retried: wireless adb drops large transfers now and then.
adb shell mkdir -p "$TMP"
GGUFS=()
for quant in ${STT_QUANTS:-q4k}; do
    GGUFS+=("model.${quant/q8_0/q8}.gguf")
done
FILES=("${GGUFS[@]}" config.json tokenizer.model)
for f in "${FILES[@]}"; do
    # -L: the file a link points at, when the models folder is shared.
    size=$(stat -L -f %z "$SRC/$f" 2>/dev/null || stat -L -c %s "$SRC/$f")
    for attempt in 1 2 3; do
        [[ "$(adb shell stat -c %s "$TMP/$f" 2>/dev/null | tr -d '\r')" == "$size" ]] && break
        echo "==> push $f (attempt $attempt)"
        adb push "$SRC/$f" "$TMP/$f" >/dev/null || { sleep 2; adb reconnect >/dev/null 2>&1 || true; sleep 3; }
    done
    [[ "$(adb shell stat -c %s "$TMP/$f" | tr -d '\r')" == "$size" ]] || { echo "error: $f did not arrive" >&2; exit 1; }
done
# The staged files are kept, so another quantization only adds its GGUF.
adb shell rm -f "$TMP/mimi.safetensors"
COPY=""
for f in "${FILES[@]}"; do COPY+=" $TMP/$f"; done
adb shell run-as "$APP_ID" sh -c "'mkdir -p files/stt-1b-en_fr && rm -f files/stt-1b-en_fr/mimi.safetensors && cp$COPY files/stt-1b-en_fr/'"
echo "==> $APP_ID files/stt-1b-en_fr: $(adb shell run-as "$APP_ID" du -sh files/stt-1b-en_fr | cut -f1)"

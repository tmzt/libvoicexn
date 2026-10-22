#!/usr/bin/env bash
# Copy the STT model (what the app loads: model.q4k.gguf, mimi.safetensors,
# config.json, tokenizer.model; about 1.4 GB) into the app's data directory,
# <data dir>/stt-1b-en_fr. Needs the debuggable APK installed, for run-as.
#
#   scripts/push-stt.sh [<model dir>]
#   STT_QUANTS="q8_0 q4_0" scripts/push-stt.sh    # other quantizations
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-$ROOT/models/stt-1b-en_fr}"
APP_ID=dev.tmzt.voicexn
TMP=/data/local/tmp/voicexn-stt

[[ -f "$SRC/config.json" ]] || { echo "error: no model in $SRC; run scripts/fetch-stt.sh" >&2; exit 1; }
# A file already staged at its full size is not pushed again, and a push
# is retried: wireless adb drops large transfers now and then.
adb shell mkdir -p "$TMP"
GGUFS=()
for quant in ${STT_QUANTS:-q4k}; do
    GGUFS+=("model.${quant/q8_0/q8}.gguf")
done
for f in "${GGUFS[@]}" mimi.safetensors config.json tokenizer.model; do
    size=$(stat -f %z "$SRC/$f" 2>/dev/null || stat -c %s "$SRC/$f")
    for attempt in 1 2 3; do
        [[ "$(adb shell stat -c %s "$TMP/$f" 2>/dev/null | tr -d '\r')" == "$size" ]] && break
        echo "==> push $f (attempt $attempt)"
        adb push "$SRC/$f" "$TMP/$f" >/dev/null || { sleep 2; adb reconnect >/dev/null 2>&1 || true; sleep 3; }
    done
    [[ "$(adb shell stat -c %s "$TMP/$f" | tr -d '\r')" == "$size" ]] || { echo "error: $f did not arrive" >&2; exit 1; }
done
# The staged files are kept, so another quantization only adds its GGUF.
adb shell run-as "$APP_ID" sh -c "'mkdir -p files/stt-1b-en_fr && cp $TMP/* files/stt-1b-en_fr/'"
echo "==> $APP_ID files/stt-1b-en_fr: $(adb shell run-as "$APP_ID" du -sh files/stt-1b-en_fr | cut -f1)"

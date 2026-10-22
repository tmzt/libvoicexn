#!/usr/bin/env bash
# Download Kyutai's stt-1b-en_fr (CC-BY-4.0) into the shared model store,
# $COMMON_MODELS/stt-1b-en_fr (default ~/src/common-models), quantize it
# there, and link models/stt-1b-en_fr to it:
#   model.safetensors  mimi.safetensors   the checkpoint as published
#   model.q8.gguf      the LM (q8_0 linears) and Mimi's encoder, one file
#   config.json  tokenizer.model
#
#   scripts/fetch-stt.sh
#
# The checkpoint and xn-moshi (whose `quantize` example does the
# quantizing) are pinned. Files already present are kept.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="${COMMON_MODELS:-$HOME/src/common-models}/stt-1b-en_fr"
REPO=https://huggingface.co/kyutai/stt-1b-en_fr-candle/resolve/095e38f6242006a93c2541149b181988397f5c7c
XN_MOSHI_REV=b5366b7ecca93beafb6d1eee396677fb28088e8c

fetch() { # <url> <file>
    [[ -s "$2" ]] && return
    curl -fsSL --retry 3 -o "$2.part" "$1"
    mv "$2.part" "$2"
}

mkdir -p "$DIR"
fetch "$REPO/config.json" "$DIR/config.json"
fetch "$REPO/tokenizer_en_fr_audio_8000.model" "$DIR/tokenizer.model"
fetch "$REPO/mimi-pytorch-e351c8d8@125.safetensors" "$DIR/mimi.safetensors"
fetch "$REPO/model.safetensors" "$DIR/model.safetensors"

if [[ ! -s "$DIR/model.q8.gguf" ]]; then
    XN_MOSHI="$ROOT/target/xn-moshi"
    if [[ ! -d "$XN_MOSHI/.git" ]]; then
        git clone -q https://github.com/gradium-ai/xn-moshi "$XN_MOSHI"
    fi
    if [[ "$(git -C "$XN_MOSHI" rev-parse HEAD)" != "$XN_MOSHI_REV" ]]; then
        git -C "$XN_MOSHI" fetch -q origin "$XN_MOSHI_REV"
        git -C "$XN_MOSHI" checkout -q "$XN_MOSHI_REV"
    fi
    # It takes the folder: model.safetensors and mimi.safetensors.
    cargo run --release --manifest-path "$XN_MOSHI/Cargo.toml" --example quantize -- \
        asr "$DIR" "$DIR/model.q8.gguf.part" --quant q8_0
    mv "$DIR/model.q8.gguf.part" "$DIR/model.q8.gguf"
fi
mkdir -p "$ROOT/models"
ln -sfn "$DIR" "$ROOT/models/stt-1b-en_fr"
echo "==> $DIR: $(du -sh "$DIR/model.q8.gguf" | cut -f1) model.q8.gguf"

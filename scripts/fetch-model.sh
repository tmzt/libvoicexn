#!/usr/bin/env bash
# Download Phonon 31m and the web demo's voices into models/phonon-31m-en/.
#
#   scripts/fetch-model.sh [<dir>]
#
# Both repositories are pinned. The weights, tokenizer and voices are
# Apache-2.0 (huggingface.co/gradium/phonon-31m-en). Files already present
# are kept, so a rerun only fetches what is missing.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="${1:-$ROOT/models/phonon-31m-en}"
MODEL=https://huggingface.co/gradium/phonon-31m-en/resolve/92741c5b07d085ff840e26fea8ce43110b87852f
VOICES=https://huggingface.co/gradium/phonon-voices/resolve/da6c9ec0d47be8df5c87c730a3aa97187f3229c3

fetch() { # <url> <file>
    [[ -s "$2" ]] && return
    curl -fsSL --retry 3 -o "$2.part" "$1"
    mv "$2.part" "$2"
}

mkdir -p "$DIR/voices"
for f in config.json tokenizer.json model.q8.gguf default-voice.safetensors; do
    fetch "$MODEL/$f" "$DIR/$f"
done
while IFS=$'\t' read -r name _ file; do
    [[ -z "$name" || "$name" == \#* ]] && continue
    fetch "$VOICES/$file" "$DIR/voices/$name.safetensors"
done < "$ROOT/scripts/voices.tsv"
echo "==> $DIR: $(ls "$DIR/voices" | wc -l | tr -d ' ') voices, $(du -sh "$DIR" | cut -f1)"

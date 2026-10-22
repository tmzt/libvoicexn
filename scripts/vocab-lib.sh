#!/usr/bin/env bash
# Write vocab.txt, the STT boost list (plan 20261022_04): the lib* folders
# in ~/src, one per line.
#
#   scripts/vocab-lib.sh [<src dir>]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-$HOME/src}"
{
    echo "# The lib* folders in ~/src, from scripts/vocab-lib.sh."
    for d in "$SRC"/lib*/; do basename "$d"; done
} > "$ROOT/vocab.txt"
echo "==> vocab.txt: $(grep -vc '^#' "$ROOT/vocab.txt") phrases"

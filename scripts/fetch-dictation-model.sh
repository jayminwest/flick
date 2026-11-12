#!/usr/bin/env bash
# Download the default dictation model (docs/dictation.md) to the path Flick expects, and
# check its SHA-256. Flick itself never downloads models; this script is the one place the
# model's URL and hash are pinned. Safe to re-run: a good file is kept, a bad one replaced.
set -euo pipefail

NAME="ggml-large-v3-turbo-q5_0.bin"
URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$NAME"
SHA256="394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
DIR="${FLICK_MODELS_DIR:-$HOME/Library/Application Support/Flick/models}"

dest="$DIR/$NAME"
sum() { shasum -a 256 "$1" | cut -d' ' -f1; }

if [ -f "$dest" ] && [ "$(sum "$dest")" = "$SHA256" ]; then
  echo "ok: $dest"
  exit 0
fi

mkdir -p "$DIR"
part="$dest.part"
trap 'rm -f "$part"' EXIT
echo "downloading $NAME (about 574 MB)"
curl -L --fail --progress-bar -o "$part" "$URL"

got="$(sum "$part")"
if [ "$got" != "$SHA256" ]; then
  echo "error: SHA-256 mismatch for $NAME" >&2
  echo "  expected $SHA256" >&2
  echo "  got      $got" >&2
  exit 1
fi
mv "$part" "$dest"
echo "ok: $dest"

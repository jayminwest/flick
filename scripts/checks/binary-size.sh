#!/usr/bin/env bash
# Gate check:binary-size: the release binary stays under the ceiling in
# scripts/binary-size-budget.json.
set -euo pipefail
cd "$(dirname "$0")/../.."

BUDGET=scripts/binary-size-budget.json
max=$(jq -r '.max_bytes' "$BUDGET")
cargo build --release --locked
target=$(cargo metadata --format-version 1 --no-deps --locked | jq -r '.target_directory')
size=$(stat -f %z "$target/release/flick")

if [ "$size" -gt "$max" ]; then
  echo "error: release binary is $size bytes, over the $max-byte budget in $BUDGET"
  exit 1
fi
echo "check:binary-size: $size of $max bytes"

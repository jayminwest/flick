#!/usr/bin/env bash
# One-time developer setup: use the tracked hooks in scripts/hooks, and list any
# tool that scripts/check-all.sh needs but cannot find.
set -euo pipefail
cd "$(dirname "$0")/.."

git config core.hooksPath scripts/hooks
echo "core.hooksPath = scripts/hooks (pre-commit runs scripts/check-all.sh)"

missing=0
need() { # command, install hint
  command -v "$1" >/dev/null 2>&1 && return 0
  echo "missing: $1  ($2)"
  missing=1
}
need cargo "https://rustup.rs"
need jq "brew install jq"
need perl "ships with macOS"
need cargo-machete "cargo install --locked cargo-machete"
need cargo-deny "cargo install --locked cargo-deny"
need cargo-llvm-cov "cargo install --locked cargo-llvm-cov"
if command -v rustup >/dev/null 2>&1 && ! rustup component list --installed | grep -q '^llvm-tools'; then
  echo "missing: llvm-tools  (rustup component add llvm-tools-preview)"
  missing=1
fi
[ $missing = 0 ] && echo "all check:all tools found"
exit 0

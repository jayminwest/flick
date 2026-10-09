#!/usr/bin/env bash
# Gate lint: rustfmt (rustfmt.toml) and clippy (Cargo.toml [lints], clippy.toml).
# Warnings fail. Fix existing violations or mark them #[expect(lint, reason = "...")].
set -euo pipefail
cd "$(dirname "$0")/../.."

cargo fmt --check
cargo clippy --all-targets --locked --message-format=short -- -D warnings

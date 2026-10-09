#!/usr/bin/env bash
# Gate check:deps: no unused dependencies (cargo machete; false positives go in
# Cargo.toml [package.metadata.cargo-machete] with a why), and no vulnerable,
# yanked, disallowed-license or unknown-source crates (cargo deny, deny.toml).
set -euo pipefail
cd "$(dirname "$0")/../.."

cargo machete
cargo deny --locked check

#!/usr/bin/env bash
# Gate check:agents: AGENTS.md must not point at things that do not exist (port of
# the os-eco validate-agents-md.ts). Checks:
#   - every backticked token that looks like a repo path exists
#   - every backticked command whose first word is a repo path (`scripts/x.sh --flag`)
#     names an executable file
#   - every `scripts/...` invocation in a fenced bash/sh/shell block is executable
set -euo pipefail
cd "$(dirname "$0")/../.."

DOC=AGENTS.md
# Paths AGENTS.md may name although they are not in the tree. Each needs a why.
KNOWN_MISSING=(
  .mulch/archive # created by `ml prune` on first archive; the mulch section describes it
)

[ -f "$DOC" ] || { echo "error: $DOC not found"; exit 1; }

known_missing() {
  local k
  for k in "${KNOWN_MISSING[@]}"; do [ "$1" = "$k" ] && return 0; done
  return 1
}

fail=0
checked=0
check_path() { # token, kind
  checked=$((checked + 1))
  if [ "$2" = command ]; then
    [ -x "$1" ] && return 0
    echo "error: $DOC: command \`$1\` is not an executable file"
  else
    [ -e "$1" ] && return 0
    known_missing "$1" && return 0
    echo "error: $DOC: path \`$1\` does not exist"
  fi
  fail=1
}

# Inline code spans, one per line.
spans=$(grep -oE '`[^`]+`' "$DOC" | sed -E 's/^`//; s/`$//' || true)
while IFS= read -r span; do
  token=$(printf '%s' "$span" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')
  case "$token" in
    '' | http://* | https://* | @* | *... | *'<'* | *'>'* | *'*'*) continue ;;
  esac
  if [[ "$token" =~ [[:space:]] ]]; then
    first=${token%%[[:space:]]*}
    [[ "$first" == */* && "$first" =~ ^[.A-Za-z0-9_][A-Za-z0-9_./-]*$ ]] || continue
    [[ "$first" == http* || "$first" == *'<'* ]] && continue
    check_path "$first" command
    continue
  fi
  [[ "$token" =~ ^\.[A-Za-z0-9]+$ ]] && continue
  [[ "$token" =~ ^[.A-Za-z0-9_][A-Za-z0-9_./-]*$ ]] || continue
  [[ "$token" == */* || "$token" =~ \.(md|json|ya?ml|toml|rs|sh|lock)$ ]] || continue
  check_path "${token%/}" path
done <<<"$spans"

# scripts/... invocations inside fenced shell blocks.
fenced=$(awk '/^```(bash|sh|shell)[[:space:]]*$/ { on = 1; next } /^```/ { on = 0 } on' "$DOC" |
  sed 's/#.*//' | grep -oE '(^|[[:space:]])(\./)?scripts/[A-Za-z0-9_./-]+' | sed -E 's/^[[:space:]]+//; s|^\./||' || true)
while IFS= read -r cmd; do
  [ -n "$cmd" ] && check_path "$cmd" command
done <<<"$fenced"

[ $fail = 0 ] && echo "check:agents: $checked references ok"
exit $fail

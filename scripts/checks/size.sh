#!/usr/bin/env bash
# Gate check:size: file-size ratchet for src/**/*.rs and tests/**/*.rs.
# Limit and per-file budgets live in scripts/file-size-budgets.json.
set -euo pipefail
cd "$(dirname "$0")/../.."

BUDGETS=scripts/file-size-budgets.json
limit=$(jq -r '.limit' "$BUDGETS")
budgets=$(jq -r '.budgets | to_entries[] | "\(.key) \(.value)"' "$BUDGETS")
budget_for() { echo "$budgets" | awk -v f="$1" '$1 == f { print $2 }'; }

fail=0
files=$(find src tests -name '*.rs' -type f 2>/dev/null | sort || true)
for f in $files; do
  lines=$(wc -l <"$f" | tr -d ' ')
  [ "$lines" -gt "$limit" ] || continue
  budget=$(budget_for "$f")
  if [ -z "$budget" ]; then
    echo "error: $f has $lines lines, over the $limit-line limit; split it (no budget entry)"
    fail=1
  elif [ "$lines" -gt "$budget" ]; then
    echo "error: $f has $lines lines, over its budget of $budget; split it"
    fail=1
  elif [ "$lines" -lt "$budget" ]; then
    echo "note: $f shrank to $lines lines; lower its budget in $BUDGETS"
  fi
done

while read -r f budget; do
  [ -n "$f" ] || continue
  if [ ! -f "$f" ] || [ "$(wc -l <"$f" | tr -d ' ')" -le "$limit" ]; then
    echo "error: stale budget entry $f in $BUDGETS (file gone or within the limit); delete it"
    fail=1
  fi
done <<<"$budgets"

[ $fail = 0 ] && echo "check:size: $(echo "$files" | wc -w | tr -d ' ') files within budget"
exit $fail

#!/usr/bin/env bash
# Gate check:coverage: runs the test suite under cargo llvm-cov, then holds each
# scope in scripts/coverage-budgets.json at or above its line-coverage floor.
set -euo pipefail
cd "$(dirname "$0")/../.."

BUDGETS=scripts/coverage-budgets.json
report=$(mktemp)
trap 'rm -f "$report"' EXIT

cargo llvm-cov --locked --json --summary-only --output-path "$report"

# One line per scope: status scope covered count percent floor.
results=$(jq -r --arg root "$PWD/" --slurpfile b "$BUDGETS" '
  [.data[0].files[] | {f: (.filename | ltrimstr($root)), l: .summary.lines}] as $files
  | $b[0].floors | to_entries[]
  | .key as $scope | .value as $floor
  | [$files[] | select(if ($scope | endswith("/")) then (.f | startswith($scope)) else .f == $scope end)]
  | if length == 0 then "stale \($scope) 0 0 0 \($floor)"
    else (map(.l.covered) | add) as $c | (map(.l.count) | add) as $n
      | (if $n == 0 then 100 else ($c * 100 / $n) end) as $p
      | "\(if $p < $floor then "low" else "ok" end) \($scope) \($c) \($n) \($p * 100 | floor / 100) \($floor)"
    end' "$report")

fail=0
while read -r status scope covered count pct floor; do
  case "$status" in
    stale) echo "error: coverage scope $scope in $BUDGETS matches no file; delete or fix it"; fail=1 ;;
    low) echo "error: $scope line coverage $pct% ($covered/$count) is below its $floor% floor budget"; fail=1 ;;
    ok) echo "$scope: $pct% (floor $floor%)" ;;
  esac
done <<<"$results"
exit $fail

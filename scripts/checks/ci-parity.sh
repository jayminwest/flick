#!/usr/bin/env bash
# Gate check:ci-parity: CI runs exactly scripts/check-all.sh, so local and CI gates
# match by construction. This asserts it:
#   - every manifest gate has an executable script
#   - every `run:` step in .github/workflows/ci*.yml is a one-line `scripts/check-all.sh`
#     (setup goes in `uses:` steps), and at least one exists
#   - the pre-commit hook runs scripts/check-all.sh
set -euo pipefail
cd "$(dirname "$0")/../.."

fail=0
gates=0
for g in $(scripts/check-all.sh --list); do
  gates=$((gates + 1))
  s="scripts/checks/${g#check:}.sh"
  [ -x "$s" ] || { echo "error: gate $g has no executable $s"; fail=1; }
done

runs=0
for wf in .github/workflows/ci*.yml; do
  [ -f "$wf" ] || continue
  while IFS= read -r line; do
    n=${line%%:*}
    cmd=$(echo "${line#*:}" | sed -E 's/^[[:space:]]*(-[[:space:]]*)?run:[[:space:]]*//; s/[[:space:]]+#.*$//; s/[[:space:]]+$//')
    if [[ "$cmd" =~ ^scripts/check-all\.sh([[:space:]]+--bail)?$ ]]; then
      runs=$((runs + 1))
    else
      echo "error: $wf:$n: CI step runs '$cmd'; CI may only run scripts/check-all.sh (put the check in a gate)"
      fail=1
    fi
  done < <(grep -nE '^[[:space:]]*(-[[:space:]]*)?run:' "$wf" || true)
done
[ $runs -gt 0 ] || { echo "error: no .github/workflows/ci*.yml step runs scripts/check-all.sh"; fail=1; }

grep -q 'scripts/check-all.sh' scripts/hooks/pre-commit ||
  { echo "error: scripts/hooks/pre-commit does not run scripts/check-all.sh"; fail=1; }

[ $fail = 0 ] && echo "check:ci-parity: $gates gates, $runs CI step(s) run scripts/check-all.sh"
exit $fail

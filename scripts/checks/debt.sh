#!/usr/bin/env bash
# Gate check:debt: every TODO/FIXME/HACK/XXX in src/, tests/ and scripts/ cites a seed
# (flick-xxxx), an issue (#N) or a URL on the same line, or is allowlisted by
# path:line in scripts/debt-allowlist.json. Stale allowlist entries fail.
set -euo pipefail
cd "$(dirname "$0")/../.."

ALLOW=scripts/debt-allowlist.json
MARKER='\b(TODO|FIXME|HACK|XXX)\b'
CITE='flick-[0-9a-z]{4}|#[0-9]+|https?://'

bad=$(jq -r '.allow[] | select((.at // "") == "" or (.why // "") == "") | .at // "?"' "$ALLOW")
allowed=$(jq -r '.allow[].at' "$ALLOW")

fail=0
if [ -n "$bad" ]; then
  echo "error: $ALLOW entries need both at and why: $bad"
  fail=1
fi

# This script names the markers itself, so it is not scanned.
hits=$(find src tests scripts -type f \( -name '*.rs' -o -name '*.sh' -o -name '*.toml' \) \
  ! -path scripts/checks/debt.sh 2>/dev/null | sort |
  xargs grep -nE "$MARKER" /dev/null 2>/dev/null | grep -vE "$CITE" || true)
used=""
while IFS= read -r hit; do
  [ -n "$hit" ] || continue
  at=$(echo "$hit" | cut -d: -f1,2)
  if echo "$allowed" | grep -qxF "$at"; then
    used="$used $at"
  else
    echo "error: $hit  [debt marker must cite flick-xxxx, #N or a URL]"
    fail=1
  fi
done <<<"$hits"

for at in $allowed; do
  case " $used " in
    *" $at "*) ;;
    *) echo "error: stale entry $at in $ALLOW (no uncited marker there); delete it"; fail=1 ;;
  esac
done

[ $fail = 0 ] && echo "check:debt: no uncited debt markers"
exit $fail

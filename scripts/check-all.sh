#!/usr/bin/env bash
# check:all: run every quality gate in a fixed order (Rust port of the os-eco
# check-all.ts contract). CI and scripts/hooks/pre-commit run only this script.
#
#   scripts/check-all.sh           one ✓/✗ line per gate, failure lines on error
#   scripts/check-all.sh --bail    stop at the first failing gate
#   scripts/check-all.sh --list    print the gate manifest, one per line
#   CHECK_ALL_VERBOSE=1 scripts/check-all.sh   stream every gate's full output
#
# Gate `lint` runs scripts/checks/lint.sh; gate `check:<x>` runs scripts/checks/<x>.sh.
# Written for macOS /bin/bash 3.2: no associative arrays, no mapfile.
set -euo pipefail
cd "$(dirname "$0")/.."

# Cheap static gates first, tests + coverage second to last, CI parity last.
GATES="lint check:layers check:agents check:deps check:size check:debt check:binary-size
check:coverage check:ci-parity"

gate_script() { echo "scripts/checks/${1#check:}.sh"; }
now() { perl -MTime::HiRes=time -e 'printf "%.3f\n", time'; }

# The failure-relevant lines of a gate's output, else its last 25 lines.
signatures() {
  local m
  m=$(grep -v -E '\.\.\. ok$' "$1" | grep -E -i 'error|warning|FAILED|panicked|budget' | head -50 || true)
  if [ -n "$m" ]; then echo "$m"; else grep -v '^[[:space:]]*$' "$1" | tail -25 || true; fi
}

bail=0
for arg in "$@"; do
  case "$arg" in
    --bail) bail=1 ;;
    --list) for g in $GATES; do echo "$g"; done; exit 0 ;;
    *) echo "usage: scripts/check-all.sh [--bail | --list]" >&2; exit 2 ;;
  esac
done
verbose="${CHECK_ALL_VERBOSE:-0}"

width=0
for g in $GATES; do [ ${#g} -gt $width ] && width=${#g}; done

logs=$(mktemp -d)
trap 'rm -rf "$logs"' EXIT
start=$(now)
total=0
failed=""
for g in $GATES; do
  total=$((total + 1))
  script=$(gate_script "$g")
  log="$logs/$total.log"
  t0=$(now)
  [ "$verbose" = 1 ] && printf '\n── %s ──\n' "$g"
  status=0
  if [ ! -x "$script" ]; then
    echo "error: gate $g has no executable $script" >"$log"
    status=1
  elif [ "$verbose" = 1 ]; then
    "$script" || status=$?
  else
    "$script" >"$log" 2>&1 || status=$?
  fi
  secs=$(perl -e 'printf "%.1f", $ARGV[1] - $ARGV[0]' "$t0" "$(now)")
  if [ $status -eq 0 ]; then mark="✓"; else mark="✗"; failed="$failed $g@$total"; fi
  printf '%s %-*s (%ss)\n' "$mark" "$width" "$g" "$secs"
  [ $status -ne 0 ] && [ $bail = 1 ] && break
done
secs=$(perl -e 'printf "%.1f", $ARGV[1] - $ARGV[0]' "$start" "$(now)")

if [ -z "$failed" ]; then
  printf '\n%d/%d gates passed (%ss)\n' "$total" "$total" "$secs"
  exit 0
fi

n=0
names=""
for f in $failed; do n=$((n + 1)); names="$names${names:+, }${f%%@*}"; done
printf '\n%d gate(s) failed: %s\n\n' "$n" "$names" >&2
for f in $failed; do
  g=${f%%@*}
  echo "── $g ──" >&2
  if [ "$verbose" != 1 ]; then signatures "$logs/${f##*@}.log" | sed 's/^/  /' >&2; fi
  echo "  ↳ re-run: $(gate_script "$g")  (or CHECK_ALL_VERBOSE=1 scripts/check-all.sh)" >&2
done
exit 1

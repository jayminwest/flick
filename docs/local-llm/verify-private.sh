#!/usr/bin/env bash
# verify-private.sh: prove a private chat left no trace on the laptop or the Mac Pro
# (flick-bcd7). Guide: docs/local-llm.md, "Verify private mode".
#
# Usage, on the laptop:
#   docs/local-llm/verify-private.sh [--host mac-pro] [--no-remote] [--reuse] [--since-mins N]
# On the Mac Pro, for a complete scan of root-only logs:
#   sudo bash verify-private.sh --here mac-pro [--since-mins N]
#
# The script makes a random canary and you send it through Flick's private chat by hand.
# Private chat has no CLI on purpose. The script then greps each location for the canary and
# prints PASS, FAIL, SKIP (location absent) or INCOMPLETE (some files unreadable) per location.
# Exit: 0 all pass, 1 any FAIL, 2 no FAIL but something INCOMPLETE, 64 bad usage.
#
# The canary never touches disk or argv: grep and sqlite3 get it on a pipe or /dev/fd, ssh
# gets it on stdin. Avoid heredocs and here-strings in this file: bash writes those to temp files.
#
# Test overrides (point the scan at fake paths; never needed for a real run):
#   FLICK_VERIFY_LAPTOP_DIRS / FLICK_VERIFY_MACPRO_DIRS  colon-separated paths that replace
#       the default file locations (each path is its own row)
#   FLICK_VERIFY_DB        flick.db for the clip-history check ("" skips it)
#   FLICK_VERIFY_FEEDBACK  feedback file ("" skips it)
#   FLICK_VERIFY_NO_LOG=1  skip the unified log (`log show`) checks

CANARY=""
FAILS=0
INCOMPLETES=0
PASSES=0

say() { printf '%s\n' "$*"; }
row() { # status, label, detail
  printf '%-10s %s%s\n' "$1" "$2" "${3:+  ($3)}"
  case $1 in
    PASS) PASSES=$((PASSES + 1)) ;;
    FAIL) FAILS=$((FAILS + 1)) ;;
    INCOMPLETE) INCOMPLETES=$((INCOMPLETES + 1)) ;;
  esac
}

# The canary as a grep pattern file, on a pipe.
pattern() { printf '%s\n' "$CANARY"; }

valid_canary() { [[ $1 =~ ^flick-canary-[0-9a-f]{16}$ ]]; }

new_canary() {
  CANARY="flick-canary-$(od -An -tx1 -N8 /dev/urandom | tr -d ' \n')"
  valid_canary "$CANARY" || { say "error: could not make a canary"; exit 1; }
}

# Prompt for a canary without echoing it (stdin may be a pipe in tests).
read_canary() {
  local c=""
  if [ -t 0 ]; then
    printf 'Canary (input hidden): ' >&2
    IFS= read -rs c
    printf '\n' >&2
  else
    IFS= read -r c
  fi
  valid_canary "$c" || { say "error: not a canary from this script: expected flick-canary-<16 hex>"; exit 64; }
  CANARY=$c
}

# Grep one or more paths for the canary. Missing paths are skipped; unreadable files make the
# row INCOMPLETE unless there is also a hit.
scan() { # label, path...
  local label=$1 p out line hits="" nhits=0 denied=0 present=0 z
  shift
  for p in "$@"; do
    [ -n "$p" ] && [ -e "$p" ] || continue
    present=1
    out=$(grep -rlF -D skip -f <(pattern) -- "$p" 2>&1 </dev/null)
    while IFS= read -r line; do
      [ -n "$line" ] || continue
      case $line in
        grep:*) denied=$((denied + 1)) ;;
        *) nhits=$((nhits + 1)); [ $nhits -le 5 ] && hits="$hits${hits:+, }$line" ;;
      esac
    done < <(printf '%s\n' "$out")
    # grep does not look inside rotated, compressed logs.
    if [ -d "$p" ]; then
      while IFS= read -r z; do
        [ -n "$z" ] || continue
        case $z in
          *.gz) gzip -dc -- "$z" 2>/dev/null ;;
          *.bz2) bzip2 -dc -- "$z" 2>/dev/null ;;
        esac | grep -qF -f <(pattern) && { nhits=$((nhits + 1)); [ $nhits -le 5 ] && hits="$hits${hits:+, }$z"; }
      done < <(find "$p" -type f \( -name '*.gz' -o -name '*.bz2' \) 2>/dev/null)
    fi
  done
  if [ $present = 0 ]; then
    row SKIP "$label" "not present"
  elif [ $nhits -gt 0 ]; then
    row FAIL "$label" "$nhits file(s): $hits"
  elif [ $denied -gt 0 ]; then
    row INCOMPLETE "$label" "no hit, but $denied path(s) unreadable; give the terminal Full Disk Access or use sudo"
  else
    row PASS "$label"
  fi
}

# Clip history lives in flick.db's clips table; grep of the file covers it too, this names it.
scan_clips() { # db
  local db=$1 n
  if [ -z "$db" ] || [ ! -f "$db" ]; then row SKIP "clip history (clips table)" "no flick.db"; return; fi
  if ! command -v sqlite3 >/dev/null; then row INCOMPLETE "clip history (clips table)" "sqlite3 not found"; return; fi
  # The canary is [a-z0-9-] only (valid_canary), so it is safe inside the SQL string.
  n=$(printf "SELECT count(*) FROM clips WHERE instr(text, '%s') > 0;\n" "$CANARY" | sqlite3 -readonly "$db" 2>/dev/null)
  case $n in
    0) row PASS "clip history (clips table)" ;;
    '' | *[!0-9]*) row INCOMPLETE "clip history (clips table)" "could not query $db" ;;
    *) row FAIL "clip history (clips table)" "$n clip(s)" ;;
  esac
}

scan_log() { # label, predicate, minutes
  local label=$1 out n st
  if [ "${FLICK_VERIFY_NO_LOG:-}" = 1 ]; then row SKIP "$label" "FLICK_VERIFY_NO_LOG=1"; return; fi
  if ! command -v log >/dev/null; then row INCOMPLETE "$label" "no log command"; return; fi
  # Prints "<hits> <log show exit>"; PIPESTATUS does not survive the command substitution.
  out=$(log show --style syslog --info --debug --last "${3}m" --predicate "$2" 2>/dev/null </dev/null |
    grep -cF -f <(pattern); printf ' %s' "${PIPESTATUS[0]}")
  n=${out%%[!0-9]*}
  st=${out##*[!0-9]}
  if [ "$st" != 0 ]; then
    row INCOMPLETE "$label" "log show failed (exit $st)"
  elif [ "${n:-0}" -gt 0 ]; then
    row FAIL "$label" "$n line(s) in the last ${3}m"
  else
    row PASS "$label" "last ${3}m"
  fi
}

# Run one row per path of a colon-separated override list.
scan_list() { # list
  local p rest=$1
  while [ -n "$rest" ]; do
    p=${rest%%:*}
    [ "$p" = "$rest" ] && rest="" || rest=${rest#*:}
    [ -n "$p" ] && scan "$p" "$p"
  done
}

laptop_checks() { # minutes
  local support="$HOME/Library/Application Support/Flick" feedback db
  say "== laptop =="
  if [ -n "${FLICK_VERIFY_LAPTOP_DIRS+x}" ]; then
    scan_list "$FLICK_VERIFY_LAPTOP_DIRS"
  else
    scan "Flick App Support (flick.db, -wal, -shm)" "$support"
    scan "Flick saved window state" "$HOME/Library/Saved Application State/com.jayminwest.flick.savedState"
    scan "~/Library/Logs" "$HOME/Library/Logs"
    scan "~/Library/Caches" "$HOME/Library/Caches"
    scan "temp dirs (\$TMPDIR, /tmp)" "${TMPDIR:-}" /private/tmp
  fi
  db=${FLICK_VERIFY_DB-"$support/flick.db"}
  scan_clips "$db"
  if [ -n "${FLICK_VERIFY_FEEDBACK+x}" ]; then
    feedback=$FLICK_VERIFY_FEEDBACK
  else
    feedback=$(flick feedback path 2>/dev/null </dev/null || true)
  fi
  if [ -n "$feedback" ]; then
    scan "feedback file" "$feedback"
  else
    row SKIP "feedback file" "no path; run \`flick feedback path\` and grep it by hand"
  fi
  scan_log "unified log: flick, curl, tailscale" \
    'process BEGINSWITH[c] "flick" OR process == "curl" OR process CONTAINS[c] "tailscale"' "$1"
}

macpro_checks() { # minutes
  local home=$HOME
  # Under sudo, scan the invoking user's home, not root's.
  if [ -n "${SUDO_USER:-}" ]; then
    home=$(dscl . -read "/Users/$SUDO_USER" NFSHomeDirectory 2>/dev/null | awk '{print $2}')
    [ -n "$home" ] || home=$HOME
  fi
  say "== mac-pro =="
  if [ -n "${FLICK_VERIFY_MACPRO_DIRS+x}" ]; then
    scan_list "$FLICK_VERIFY_MACPRO_DIRS"
  else
    scan "~/.mlx-serve" "$home/.mlx-serve"
    scan "/tmp" /private/tmp
    scan "/var/log" /private/var/log
    scan "/Library/Logs (incl. DiagnosticReports)" /Library/Logs
    scan "~/Library/Logs" "$home/Library/Logs"
  fi
  scan_log "unified log: mlx-serve, tailscaled" \
    'process CONTAINS[c] "mlx" OR process == "tailscaled"' "$1"
}

remote_checks() { # host, minutes
  local self=${BASH_SOURCE[0]} st
  say "Scanning $1 over ssh (you may be asked for a password)..."
  # The canary goes in the script text on ssh's stdin, never in the remote command line.
  { printf 'FLICK_CANARY_IN=%s\n' "$CANARY"; cat "$self"; } |
    ssh -T "$1" bash -s -- --here mac-pro --since-mins "$2" | sed 's/^/  /'
  st=${PIPESTATUS[1]}
  case $st in
    0) ;;
    1) FAILS=$((FAILS + 1)) ;;
    2) INCOMPLETES=$((INCOMPLETES + 1)) ;;
    *) row INCOMPLETE "mac-pro over ssh" "ssh exited $st; run the script there with --here mac-pro" ;;
  esac
}

summary() { # scope
  say ""
  if [ $FAILS -gt 0 ]; then
    say "RESULT $1: FAIL ($FAILS check(s) found the canary). Private mode leaked; do not use it until fixed."
    exit 1
  elif [ $INCOMPLETES -gt 0 ]; then
    say "RESULT $1: INCOMPLETE (no hit, but $INCOMPLETES check(s) could not read everything)."
    exit 2
  fi
  say "RESULT $1: PASS (no trace of the canary)."
  exit 0
}

usage() {
  sed -n '2,13p' "${BASH_SOURCE[0]}" 2>/dev/null | sed 's/^# \{0,1\}//'
  exit 64
}

main() {
  local host=mac-pro remote=1 reuse=0 here="" mins="" start
  while [ $# -gt 0 ]; do
    case $1 in
      --host) host=${2:?--host needs a value}; shift 2 ;;
      --no-remote) remote=0; shift ;;
      --reuse) reuse=1; shift ;;
      --here) here=${2:?--here needs laptop or mac-pro}; shift 2 ;;
      --since-mins) mins=${2:?--since-mins needs a number}; shift 2 ;;
      -h | --help) usage ;;
      *) say "error: unknown argument $1"; usage ;;
    esac
  done
  [ -z "$mins" ] || [[ $mins =~ ^[0-9]+$ ]] || { say "error: --since-mins takes whole minutes"; exit 64; }

  if [ "$here" = mac-pro ]; then
    if [ -n "${FLICK_CANARY_IN:-}" ]; then
      valid_canary "$FLICK_CANARY_IN" || { say "error: bad canary on stdin"; exit 64; }
      CANARY=$FLICK_CANARY_IN
    else
      read_canary
    fi
    macpro_checks "${mins:-60}"
    summary mac-pro
  elif [ -n "$here" ] && [ "$here" != laptop ]; then
    say "error: --here takes laptop or mac-pro"
    exit 64
  fi

  start=$(date +%s)
  if [ $reuse = 1 ]; then
    read_canary
  else
    new_canary
    say "Canary: $CANARY"
    say ""
    say "1. Open Flick's private chat (the llm:private item or private_hotkey)."
    say "2. Send:  Repeat this word back exactly: $CANARY"
    say "3. Wait for the reply. Select it and copy with Cmd-C (tests concealed copy)."
    say "4. Close the private window."
    say "5. Press Return here. Do not paste the canary anywhere else."
    IFS= read -r _ 2>/dev/null </dev/tty || IFS= read -r _
  fi
  # Unified log window: the whole exchange plus a margin; an hour for --reuse.
  if [ -z "$mins" ]; then
    if [ $reuse = 1 ]; then mins=60; else mins=$((($(date +%s) - start) / 60 + 15)); fi
  fi

  laptop_checks "$mins"
  [ $remote = 0 ] || remote_checks "$host" "$mins"
  summary overall
}

main "$@"

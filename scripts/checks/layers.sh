#!/usr/bin/env bash
# Gate check:layers: grep-based layer boundaries. Rules and allow entries live in
# scripts/layer-rules.toml (format documented there).
set -euo pipefail
cd "$(dirname "$0")/../.."

RULES=scripts/layer-rules.toml
US=$'\x1f' # field separator: unlike tab, it keeps empty fields when read splits

# Emit one record per table: kind US name US why US paths US except US each US
# pattern US ignore US enabled (rules) or kind US rule US path US why (allows).
parse() {
  awk -v us="$US" '
    function flush() {
      if (kind == "rule")
        print "rule" us v["name"] us v["why"] us v["paths"] us v["except"] us v["each"] us \
          v["pattern"] us v["ignore"] us v["enabled"]
      else if (kind == "allow")
        print "allow" us v["rule"] us v["path"] us v["why"]
      split("", v)
    }
    /^[[:space:]]*(#|$)/ { next }
    /^\[\[rule\]\]/  { flush(); kind = "rule"; next }
    /^\[\[allow\]\]/ { flush(); kind = "allow"; next }
    /^[[:space:]]*\[/ { print "error: " FILENAME ":" NR ": unsupported table" > "/dev/stderr"; bad = 1; next }
    {
      eq = index($0, "=")
      if (eq == 0) { print "error: " FILENAME ":" NR ": expected key = value" > "/dev/stderr"; bad = 1; next }
      key = $0; sub(/=.*/, "", key); gsub(/[[:space:]]/, "", key)
      rest = substr($0, eq + 1); sub(/^[[:space:]]+/, "", rest)
      q = substr(rest, 1, 1)
      if (q == "\x27") {
        val = substr(rest, 2); sub(/\x27.*/, "", val)
      } else if (q == "\"") {
        val = ""; i = 2
        while (i <= length(rest) && substr(rest, i, 1) != "\"") {
          c = substr(rest, i, 1)
          if (c == "\\") { i++; c = substr(rest, i, 1) }
          val = val c; i++
        }
      } else {
        val = rest; sub(/[[:space:]]*(#.*)?$/, "", val)
      }
      v[key] = val
    }
    END { flush(); exit bad }
  ' "$RULES"
}

records=$(parse)
allows=$(mktemp)
used=$(mktemp)
trap 'rm -f "$allows" "$used"' EXIT
fail=0

while IFS="$US" read -r kind rule path why _; do
  [ "$kind" = allow ] || continue
  if [ -z "$rule" ] || [ -z "$path" ] || [ -z "$why" ]; then
    echo "error: $RULES: [[allow]] entry needs rule, path and why (rule='$rule' path='$path')"
    fail=1
  fi
  echo "$rule$US$path" >>"$allows"
done <<<"$records"

# Print violations of one rule in one directory as `path:line: text`.
scan() { # dir except pattern ignore
  [ -d "$1" ] || return 0
  find "$1" -name '*.rs' -type f | sort | while read -r file; do
    if [ -n "$2" ]; then case "$file/" in "$2"/*) continue ;; esac; fi
    grep -nE -- "$3" "$file" | grep -vE '^[0-9]+:[[:space:]]*//' |
      { if [ -n "$4" ]; then grep -vE -- "$4"; else cat; fi; } | sed "s|^|$file:|" || true
  done
}

rules=0
while IFS="$US" read -r kind name why paths except each pattern ignore enabled; do
  [ "$kind" = rule ] || continue
  rules=$((rules + 1))
  if [ -z "$name" ] || [ -z "$why" ] || [ -z "$paths" ] || [ -z "$pattern" ]; then
    echo "error: $RULES: [[rule]] '$name' needs name, why, paths and pattern"
    fail=1
    continue
  fi
  [ "$enabled" = true ] || continue
  if [ "$each" = true ]; then
    dirs=$(find "$paths" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort || true)
  else
    dirs=$paths
  fi
  for dir in $dirs; do
    self=$(basename "$dir")
    hits=$(scan "$dir" "$except" "$pattern" "${ignore//\{self\}/$self}")
    [ -n "$hits" ] || continue
    while read -r hit; do
      file=${hit%%:*}
      if grep -qxF "$name$US$file" "$allows"; then
        echo "$name$US$file" >>"$used"
      else
        echo "error: $hit  [layer rule $name: $why]"
        fail=1
      fi
    done <<<"$hits"
  done
done <<<"$records"

# Allow entries must still match something, so they cannot outlive the code they excuse.
while IFS="$US" read -r rule path; do
  [ -n "$rule" ] || continue
  enabled=$(echo "$records" | awk -F "$US" -v r="$rule" '$1 == "rule" && $2 == r { print $9 }')
  [ "$enabled" = true ] || continue
  if ! grep -qxF "$rule$US$path" "$used"; then
    echo "error: stale [[allow]] rule=$rule path=$path matches no violation; delete it"
    fail=1
  fi
done <"$allows"

[ $fail = 0 ] && echo "check:layers: $rules rules ok"
exit $fail

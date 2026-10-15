#!/bin/sh
# Test fixture for the ext transport (src/modules/ext/transport.rs): a Flick extension
# whose first argument picks how it behaves.
#   echo            reply with one item: key = the request's op, title = "$FLICK_EXT
#                   $FLICK_VERSION", subtitle = $FLICK_SOCKET; status "<op> done"
#   deaf            reply without reading stdin
#   sleep N FILE    write "<pid> <sleep pid>" to FILE, then sleep N seconds
#   fail            two stderr lines, exit 3
#   exit            exit 3, no output
#   signal          kill itself with SIGKILL
#   error           reply {"error": "no such view"}
#   garbage         reply text that is not JSON
#   huge            reply over 1 MiB
#   bad N           reply N items without a title (each one a warning)
mode=$1
case $mode in
deaf) ;;
*) request=$(cat) ;;
esac
op=$(printf '%s' "$request" | sed -n 's/.*"op":"\([a-z_]*\)".*/\1/p')
case $mode in
echo)
  printf '{"v":1,"status":"%s done","items":[{"key":"%s","title":"%s %s","subtitle":"%s"}]}\n' \
    "$op" "$op" "$FLICK_EXT" "$FLICK_VERSION" "$FLICK_SOCKET"
  ;;
deaf) echo '{"items":[]}' ;;
sleep)
  sleep "$2" &
  echo "$$ $!" >"$3"
  wait
  ;;
fail)
  echo "warming up" >&2
  echo "bad token" >&2
  exit 3
  ;;
exit) exit 3 ;;
signal) kill -9 $$ ;;
error) echo '{"error":"no such view"}' ;;
garbage) echo 'not json' ;;
huge) head -c 1100000 /dev/zero | tr '\0' ' ' ;;
bad)
  printf '{"items":['
  i=0
  while [ "$i" -lt "$2" ]; do
    [ "$i" -gt 0 ] && printf ','
    printf '{"key":"k%s"}' "$i"
    i=$((i + 1))
  done
  printf ']}\n'
  ;;
*)
  echo "unknown mode $mode" >&2
  exit 2
  ;;
esac

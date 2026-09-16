#!/bin/sh
# Lightweight supervision; TUN policy belongs to cloud YAML, not this script.
set -u
root=/jffs/camofy
pidfile="$root/agent-supervisor.pid"
if [ -s "$pidfile" ]; then
  old=$(cat "$pidfile")
  if kill -0 "$old" 2>/dev/null && tr '\000' ' ' < "/proc/$old/cmdline" | grep -q '/jffs/camofy/start-agent.sh'; then exit 0; fi
fi
echo $$ > "$pidfile"
child=''
cleanup() {
  if [ -n "$child" ]; then kill -TERM "$child" 2>/dev/null || true; wait "$child" 2>/dev/null || true; fi
  rm -f "$root/agent.pid" "$pidfile"
}
trap cleanup 0
trap 'cleanup; trap - 0; exit 0' 2 15
while :; do
  if [ -f "$root/log/agent.log" ] && [ "$(wc -c < "$root/log/agent.log")" -gt 131072 ]; then
    mv "$root/log/agent.log" "$root/log/agent.log.previous"
  fi
  "$root/camofy" "$root/agent.json" >> "$root/log/agent.log" 2>&1 &
  child=$!
  echo "$child" > "$root/agent.pid"
  wait "$child" || true
  child=''
  sleep 5
done

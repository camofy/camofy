#!/bin/sh
# Bounded upgrade of the already-migrated router. All builds occur on the workstation.
set -eu
umask 077
root=/jffs/camofy
backup=/jffs/camofy/rollback-device-control-20260916
staged=/tmp/camofy-agent-control-20260916
config=/tmp/camofy-device-settings-20260916.json
test -f "$staged" && test -f "$config"
test ! -e "$backup"
test "$(md5sum "$staged" | cut -d ' ' -f 1)" = "$1"
grep -q '"dns_redirect": false' "$config"
grep -A 3 '^tun:' "$root/config/running.yaml" | grep -q 'enable: false'
mkdir "$backup"
cp "$root/camofy" "$root/agent.json" "$root/start-agent.sh" "$backup/"
cp "$root/config/running.yaml" "$root/config/last-good.json" "$backup/"
ip route > "$backup/routes.before"
ip rule > "$backup/rules.before"
iptables-save | sed '/^#/d;s/\[[0-9]*:[0-9]*\]/[COUNTERS]/g' > "$backup/iptables.before"
stop() {
  stop_pids=''
  for file in agent-supervisor.pid agent.pid; do
    if [ -s "$root/$file" ]; then
      pid=$(cat "$root/$file")
      if [ -r "/proc/$pid/cmdline" ] && tr '\000' ' ' < "/proc/$pid/cmdline" | grep -q '/jffs/camofy/'; then stop_pids="$stop_pids $pid"; fi
    fi
  done
  # Capture both PIDs before supervisor cleanup removes the PID files.
  for pid in $stop_pids; do kill -TERM "$pid" 2>/dev/null || true; done
  n=0
  while pidof camofy >/dev/null 2>&1; do
    n=$((n+1)); test "$n" -lt 65 || return 1
    sleep 1
  done
  test -z "$(pidof mihomo 2>/dev/null || true)"
}
launch() { nohup /bin/sh "$root/start-agent.sh" </dev/null >>"$root/log/supervisor.log" 2>&1 & }
rollback() {
  trap - 0
  echo 'Upgrade failed; restoring previous Agent and binding.'
  stop || { echo 'Cannot safely stop Agent; inspect manually before restoration'; exit 1; }
  cp "$backup/camofy" "$root/camofy.restore"
  chmod 755 "$root/camofy.restore"
  mv "$root/camofy.restore" "$root/camofy"
  cp "$backup/agent.json" "$root/agent.json"
  cp "$backup/running.yaml" "$root/config/running.yaml"
  cp "$backup/last-good.json" "$root/config/last-good.json"
  launch
  exit 1
}
echo 'Backup ready; stopping only Camofy supervisor, Agent and its Mihomo.'
stop
trap rollback 0
cp "$staged" "$root/camofy.next"
chmod 755 "$root/camofy.next"
mv "$root/camofy.next" "$root/camofy"
cp "$config" "$root/agent.json.next"
chmod 600 "$root/agent.json.next"
mv "$root/agent.json.next" "$root/agent.json"
launch
n=0
while ! curl --noproxy '*' -fsS --max-time 2 http://192.168.50.1:3000/api/pair/status | grep -q '"bound":true'; do
  n=$((n+1)); test "$n" -lt 45 || exit 1
  sleep 1
done
grep -A 3 '^tun:' "$root/config/running.yaml" | grep -q 'enable: false'
ip route > "$backup/routes.after"
ip rule > "$backup/rules.after"
iptables-save | sed '/^#/d;s/\[[0-9]*:[0-9]*\]/[COUNTERS]/g' > "$backup/iptables.after"
cmp "$backup/routes.before" "$backup/routes.after"
cmp "$backup/rules.before" "$backup/rules.after"
cmp "$backup/iptables.before" "$backup/iptables.after"
trap - 0
echo 'Agent upgrade succeeded; binding retained; TUN off; routing/firewall unchanged.'

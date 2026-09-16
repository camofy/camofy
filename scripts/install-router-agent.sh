#!/bin/sh
# One-time migration of the inspected router. Requires cloud migration completed
# and a validated no-TUN subscription. No build, firewall edits, or router reboot.
set -eu
umask 077
stage=/tmp/camofy-agent-stage-20260916
root=/jffs/camofy
expected_agent=${1:?expected agent SHA256 required}
old_pid=${2:?verified legacy Camofy PID required}
case "$old_pid" in *[!0-9]*|'') exit 2;; esac
test "$root" = /jffs/camofy
test ! -e "$root/camofy.legacy-20260916"
test "$(md5sum "$root/camofy" | cut -d ' ' -f 1)" = 1cb083999d66b4a4bc0018f76ab573d0
test "$(md5sum /jffs/scripts/services-start | cut -d ' ' -f 1)" = bd5ae24c478bae5f7088fd75515f442a
test "$(readlink /proc/$old_pid/exe)" = "$root/camofy"
openssl dgst -sha256 "$stage/camofy-agent" | grep -qi "$expected_agent"
sh -n "$stage/router-supervise.sh"
sh -n "$stage/services-start.agent"
"$root/core/mihomo" -t -d "$root/config" -f "$stage/cloud-test.yaml"
ip route show | sort > "$stage/routes-before.txt"
ip rule show > "$stage/rules-before.txt"
iptables-save | grep -v '^#' | sed 's/\[[0-9][0-9]*:[0-9][0-9]*\]/[0:0]/g' > "$stage/firewall-before.txt"
cp /jffs/scripts/services-start "$root/services-start.legacy-20260916"
cp "$stage/camofy-agent" "$root/camofy.next"
chmod 755 "$root/camofy.next"
changed=0
supervisor_pid=''
finish() {
  status=$?
  trap - EXIT
  if [ "$status" != 0 ] && [ "$changed" = 1 ]; then
    echo 'Agent verification failed; restoring old Camofy.' >&2
    if [ -n "$supervisor_pid" ]; then kill -TERM "$supervisor_pid" 2>/dev/null || true; wait "$supervisor_pid" 2>/dev/null || true; fi
    if [ -s "$root/agent.pid" ]; then
      agent_pid=$(cat "$root/agent.pid")
      if [ -r "/proc/$agent_pid/cmdline" ] && tr '\000' ' ' < "/proc/$agent_pid/cmdline" | grep -q '/jffs/camofy/agent.json'; then
        kill -TERM "$agent_pid" 2>/dev/null || true
      fi
    fi
    sleep 3
    mv "$root/camofy" "$stage/camofy.failed"
    mv "$root/camofy.legacy-20260916" "$root/camofy"
    cp "$root/services-start.legacy-20260916" /jffs/scripts/services-start
    chmod 755 /jffs/scripts/services-start
    CAMOFY_HOST=0.0.0.0 CAMOFY_PORT=3000 "$root/camofy" >> "$root/log/boot.log" 2>&1 < /dev/null &
  fi
  exit "$status"
}
trap finish EXIT
kill -TERM "$old_pid"
for i in 1 2 3 4 5; do test ! -e "/proc/$old_pid/exe" && break; sleep 1; done
test ! -e "/proc/$old_pid/exe"
mv "$root/camofy" "$root/camofy.legacy-20260916"
mv "$root/camofy.next" "$root/camofy"
changed=1
cp "$stage/agent.json" "$root/agent.json"
cp "$stage/router-supervise.sh" "$root/start-agent.sh"
cp "$stage/services-start.agent" /jffs/scripts/services-start
chmod 600 "$root/agent.json"
chmod 755 "$root/start-agent.sh" /jffs/scripts/services-start
# The supervisor survives SSH disconnection and restarts only the Agent.
trap '' HUP
"$root/start-agent.sh" >> "$root/log/boot.log" 2>&1 < /dev/null &
supervisor_pid=$!
ready=0
i=0
while [ "$i" -lt 45 ]; do
  i=$((i + 1))
  if [ -s "$root/config/last-good.json" ] && [ -s "$root/config/running.yaml" ]; then
    secret=$(sed -n 's/^secret: //p' "$root/config/running.yaml" | tr -d '"')
    if curl --noproxy '*' -fsS --max-time 2 -H "Authorization: Bearer $secret" http://127.0.0.1:9091/version > "$stage/core-version.json"; then ready=1; break; fi
  fi
  sleep 2
done
test "$ready" = 1
ip route show | sort > "$stage/routes-after.txt"
ip rule show > "$stage/rules-after.txt"
iptables-save | grep -v '^#' | sed 's/\[[0-9][0-9]*:[0-9][0-9]*\]/[0:0]/g' > "$stage/firewall-after.txt"
cmp "$stage/routes-before.txt" "$stage/routes-after.txt"
cmp "$stage/rules-before.txt" "$stage/rules-after.txt"
cmp "$stage/firewall-before.txt" "$stage/firewall-after.txt"
echo 'Agent and Mihomo healthy; routes, policy rules and firewall unchanged.'

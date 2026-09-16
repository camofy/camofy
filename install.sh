#!/usr/bin/env sh
# Fresh Agent installation only. Never overwrite an existing router installation.
set -eu
umask 077
CLOUD=https://camofy.app
ROOT=''
LISTEN=''
START=1
PLAN=0
usage() {
  printf '%s\n' 'Camofy Agent installer (Linux amd64 / armv7)' \
    'curl -fsSL https://camofy.app/install.sh | sh' \
    'Options: --data-root /absolute/path --listen 127.0.0.1:3000' \
    '         --cloud https://your-cloud.example --no-start --plan --help' \
    'Fresh installs only. Existing installs are left untouched.' \
    'Mihomo stays stopped until you bind an identity and explicitly start it.'
}
die() { printf '%s\n' "$*" >&2; exit 1; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --data-root|--listen|--cloud)
      [ "$#" -ge 2 ] || die "Missing value for $1"
      case "$1" in --data-root) ROOT=$2;; --listen) LISTEN=$2;; --cloud) CLOUD=${2%/};; esac
      shift 2;;
    --no-start) START=0; shift;;
    --plan) PLAN=1; shift;;
    --help|-h) usage; exit 0;;
    *) die "Unknown option: $1";;
  esac
done
[ "$(uname -s)" = Linux ] || die 'Only Linux is supported.'
case "$(uname -m)" in x86_64|amd64) ARCH=amd64;; armv7*) ARCH=armv7;; *) die 'Unsupported architecture; no files changed.';; esac
if [ -z "$ROOT" ]; then
  if [ -d /jffs ] && [ -w /jffs ]; then ROOT=/jffs/camofy
  else ROOT="${HOME:?HOME is required}/.local/share/camofy"; fi
fi
case "$ROOT" in /*) ;; *) die 'Data root must be absolute.';; esac
case "$ROOT" in /|*/|*..*|*[!a-zA-Z0-9_./-]*) die 'Unsafe data root (use letters, digits, /, _, - and . only).';; esac
printf '%s\n' "$CLOUD" | grep -Eq '^https://[a-zA-Z0-9.-]+(:[0-9]+)?$' || die 'Cloud must be an HTTPS origin without a path.'
if [ -z "$LISTEN" ]; then
  lan=''
  if [ -d /jffs ] && command -v nvram >/dev/null 2>&1; then lan=$(nvram get lan_ipaddr 2>/dev/null || true); fi
  LISTEN="${lan:-127.0.0.1}:3000"
fi
printf '%s\n' "$LISTEN" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+:[0-9]+$' || die 'Listen address must be IPv4:port.'
if [ "$PLAN" = 1 ]; then
  printf 'Cloud: %s\nManifest: %s/downloads/manifest/%s\nData: %s\nLocal UI: http://%s/\n' "$CLOUD" "$CLOUD" "$ARCH" "$ROOT" "$LISTEN"
  exit 0
fi
[ ! -e "$ROOT" ] && [ ! -L "$ROOT" ] || die 'Existing installation/data found. Refusing to overwrite; migrate or upgrade explicitly.'
for tool in curl tar gzip sha256sum mktemp; do command -v "$tool" >/dev/null 2>&1 || die "Required tool not found: $tool"; done
BOOT=''
case "$ROOT" in /jffs/*)
  BOOT=/jffs/scripts/services-start
  if [ -f "$BOOT" ] && grep -q -i camofy "$BOOT"; then die 'Existing Camofy boot hook found; migration required.'; fi;;
esac
work=$(mktemp -d "${TMPDIR:-/tmp}/camofy-install.XXXXXXXX")
cleanup() {
  rm -f "$work/manifest" "$work/agent.archive" "$work/mihomo.archive" "$work/agent" "$work/mihomo" "$work/check"
  rmdir "$work" 2>/dev/null || true
}
trap cleanup 0
trap 'exit 1' 1 2 15
fetch() { curl --proto '=https' --proto-redir '=https' -fSL --connect-timeout 15 --max-time 300 "$1" -o "$2"; }
printf '%s\n' 'Resolving the current Agent and Mihomo releases through the cloud...'
fetch "$CLOUD/downloads/manifest/$ARCH" "$work/manifest" || die 'No compatible verified release is available. No installation files changed.'
agent_count=0
core_count=0
while read -r kind url hash extra; do
  [ -z "$extra" ] || die 'Invalid manifest.'
  case "$kind" in agent) agent_count=$((agent_count+1));; mihomo) core_count=$((core_count+1));; *) die 'Invalid manifest artifact.';; esac
  case "$url" in "$CLOUD"/github/*) ;; *) die 'Manifest download is outside this cloud.';; esac
  [ "${#hash}" = 64 ] || die 'Missing SHA256 digest.'
  case "$hash" in *[!a-fA-F0-9]*) die 'Invalid SHA256 digest.';; esac
  fetch "$url" "$work/$kind.archive" || die 'Download failed; installation not changed.'
  printf '%s  %s\n' "$hash" "$work/$kind.archive" > "$work/check"
  sha256sum -c "$work/check" || die 'SHA256 verification failed; installation not changed.'
done < "$work/manifest"
[ "$agent_count" = 1 ] && [ "$core_count" = 1 ] || die 'Manifest must contain exactly one Agent and one Mihomo.'
entries=$(tar -tzf "$work/agent.archive")
entry=''
for item in $entries; do
  case "$item" in ./) ;; camofy-agent|./camofy-agent) [ -z "$entry" ] || die 'Duplicate Agent archive entry.'; entry=$item;; *) die 'Unexpected Agent archive entry.';; esac
done
[ -n "$entry" ] || die 'Archive does not contain the new Camofy Agent.'
# Extract to stdout so archive paths/symlinks cannot write outside the staging directory.
tar -xOzf "$work/agent.archive" "$entry" > "$work/agent"
gzip -dc "$work/mihomo.archive" > "$work/mihomo"
[ -s "$work/agent" ] && [ -s "$work/mihomo" ] || die 'Empty executable.'
# An atomic mkdir ensures that concurrent installers cannot overwrite each other.
mkdir -p "$(dirname "$ROOT")"
mkdir "$ROOT" || die 'Cannot create an exclusive installation directory.'
mkdir "$ROOT/core" "$ROOT/config" "$ROOT/log"
mv "$work/agent" "$ROOT/camofy"
mv "$work/mihomo" "$ROOT/core/mihomo"
chmod 755 "$ROOT/camofy" "$ROOT/core/mihomo"
cat > "$ROOT/agent.json" <<EOF
{"cloud_url":"$CLOUD","mihomo":"$ROOT/core/mihomo","data_dir":"$ROOT/config","controller_port":9091,"dns_redirect":false,"web_listen":"$LISTEN"}
EOF
printf '%s\n' '{"stopped":true,"command_id":null,"command_error":null}' > "$ROOT/config/control.json"
cat > "$ROOT/start-agent.sh" <<EOF
#!/bin/sh
ROOT='$ROOT'
EOF
cat >> "$ROOT/start-agent.sh" <<'EOF'
set -u
if [ -s "$ROOT/agent-supervisor.pid" ]; then
  old=$(cat "$ROOT/agent-supervisor.pid")
  if kill -0 "$old" 2>/dev/null && tr '\000' ' ' < "/proc/$old/cmdline" | grep -Fq "$ROOT/start-agent.sh"; then exit 0; fi
fi
echo $$ > "$ROOT/agent-supervisor.pid"
child=''
cleanup() {
  if [ -n "$child" ]; then kill -TERM "$child" 2>/dev/null || true; wait "$child" 2>/dev/null || true; fi
  rm -f "$ROOT/agent.pid" "$ROOT/agent-supervisor.pid"
}
trap cleanup 0
trap 'exit 0' 2 15
while :; do
  if [ -f "$ROOT/log/agent.log" ] && [ "$(wc -c < "$ROOT/log/agent.log")" -gt 131072 ]; then mv "$ROOT/log/agent.log" "$ROOT/log/agent.log.previous"; fi
  "$ROOT/camofy" "$ROOT/agent.json" >> "$ROOT/log/agent.log" 2>&1 &
  child=$!
  echo "$child" > "$ROOT/agent.pid"
  wait "$child" || true
  child=''
  sleep 5
done
EOF
chmod 755 "$ROOT/start-agent.sh"
if [ -n "$BOOT" ]; then
  mkdir -p /jffs/scripts
  if [ ! -f "$BOOT" ]; then printf '%s\n' '#!/bin/sh' > "$BOOT"; fi
  printf '\n# Camofy Agent auto-start\nnohup /bin/sh "%s/start-agent.sh" </dev/null >>"%s/log/supervisor.log" 2>&1 &\n' "$ROOT" "$ROOT" >> "$BOOT"
  chmod 755 "$BOOT"
fi
if [ "$START" = 1 ]; then nohup /bin/sh "$ROOT/start-agent.sh" </dev/null >>"$ROOT/log/supervisor.log" 2>&1 & fi
printf 'Installed. Open http://%s/ to sign in and authorize the device.\n' "$LISTEN"
printf '%s\n' 'Mihomo is initially stopped. Start it explicitly after checking the assigned identity.'
if [ -z "$BOOT" ]; then printf 'To run after reboot, configure your service manager to invoke: %s/start-agent.sh\n' "$ROOT"; fi

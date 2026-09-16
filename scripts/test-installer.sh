#!/bin/sh
# Runs only in a disposable Linux container. Never targets a real router.
set -eu
suite=$(mktemp -d /tmp/camofy-installer-test.XXXXXXXX)
export FIXTURE="$suite/fixture"
mkdir -p "$FIXTURE/bin" "$FIXTURE/package"
printf 'new-agent-fixture\n' > "$FIXTURE/package/camofy-agent"
tar -czf "$FIXTURE/agent.archive" -C "$FIXTURE/package" .
printf 'mihomo-fixture\n' | gzip > "$FIXTURE/mihomo.archive"
agent_hash=$(sha256sum "$FIXTURE/agent.archive" | cut -d ' ' -f 1)
core_hash=$(sha256sum "$FIXTURE/mihomo.archive" | cut -d ' ' -f 1)
printf 'agent https://camofy.app/github/camofy/camofy/releases/download/v1/agent %s\nmihomo https://camofy.app/github/MetaCubeX/mihomo/releases/download/v1/core %s\n' "$agent_hash" "$core_hash" > "$FIXTURE/manifest"
cp "$FIXTURE/manifest" "$FIXTURE/manifest.good"
cat > "$FIXTURE/bin/curl" <<'EOF'
#!/bin/sh
set -eu
url=''
dest=''
while [ "$#" -gt 0 ]; do
  case "$1" in https://*) url=$1; shift;; -o) dest=$2; shift 2;; *) shift;; esac
done
case "$url" in
  */downloads/manifest/*) cp "$FIXTURE/manifest" "$dest";;
  */agent) cp "$FIXTURE/agent.archive" "$dest";;
  */core) cp "$FIXTURE/mihomo.archive" "$dest";;
  *) exit 90;;
esac
EOF
chmod +x "$FIXTURE/bin/curl"
export PATH="$FIXTURE/bin:$PATH"
install_script=${1:-/src/install.sh}
sh -n "$install_script"
sh "$install_script" --help
sh "$install_script" --plan --data-root "$suite/planned"
test ! -e "$suite/planned"
sh "$install_script" --no-start --data-root "$suite/new"
cmp "$suite/new/camofy" "$FIXTURE/package/camofy-agent"
grep -q '"stopped":true' "$suite/new/config/control.json"
grep -q '"dns_redirect":false' "$suite/new/agent.json"
grep -q 'https://camofy.app' "$suite/new/agent.json"
test ! -f "$suite/new/agent.pid"
sh -n "$suite/new/start-agent.sh"
if sh "$install_script" --no-start --data-root "$suite/new"; then exit 1; fi
cmp "$suite/new/camofy" "$FIXTURE/package/camofy-agent"
printf 'corrupt' >> "$FIXTURE/agent.archive"
if sh "$install_script" --no-start --data-root "$suite/corrupt"; then exit 1; fi
test ! -e "$suite/corrupt"
printf 'agent https://evil.example/file %s\n' "$agent_hash" > "$FIXTURE/manifest"
if sh "$install_script" --no-start --data-root "$suite/external"; then exit 1; fi
test ! -e "$suite/external"
if sh "$install_script" --plan --data-root /; then exit 1; fi
if sh "$install_script" --plan --cloud http://example.com; then exit 1; fi
if sh "$install_script" --data-root; then exit 1; fi
printf '#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo mips;; esac\n' > "$FIXTURE/bin/uname"
chmod +x "$FIXTURE/bin/uname"
if sh "$install_script" --plan; then exit 1; fi
echo 'Installer tests passed: fresh install, no-start, overwrite refusal, integrity and origin checks, architecture guard.'

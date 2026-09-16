#!/usr/bin/env sh
# The former router UI auto-installer is retired.
set -eu
printf '%s\n' 'Camofy now uses a cloud service and a headless agent.' \
  'Cloud: copy .env.example to .env, configure secrets, then docker compose up -d --build.' \
  'Agent: build camofy-agent, install Mihomo separately, and configure examples/agent.json.' \
  'No router files or services have been changed. See README.md and docs/cloud.md.'

# Router migration and no-TUN verification (2026-09-16)

## Current device-control upgrade (supersedes the URL/headless notes below)

Explicitly authorized by the user. Cloud image was built locally, pushed to
`registry.example.com/camofy-cloud:20260916-device-control` and pulled by ym via
`registry-mirror.example.com/camofy-cloud@sha256:071ca6b79e5414556503544e5c868fa5bf5180f526dbca6585bcd81d6ff1864b`.
Deployment evidence: `ym/camofy-deployments/20260916T022126Z` on ym.

Router UI: `http://192.168.50.1:3000/` (LAN IP only). It shows device identity,
runtime state and Mihomo start/stop/restart; it has no subscription URL editor.
First-time devices use cloud OAuth login/consent with the default server hidden
inside More options. Identity changes use authenticated device sync, not client
subscription URLs. The old router used a client token; it was moved to a dedicated
device credential on the same no-TUN identity. Existing third-party URLs were retained.

Installed Agent size: 2,775,264 bytes. SHA256:
`548957aca209e0db23cd5930ad70e18ef6da039aaceb5f4f77f91f39bcd7763f`.
Full pre-upgrade backup: local ignored
`output/router-device-control-20260916/camofy-pre-control-20260916.tar.gz`.
On-router previous Agent/settings/cache backup:
`/jffs/camofy/rollback-device-control-20260916/`.

Verification completed:
- Isolated PostgreSQL/Agent integration: identity reassignment with unchanged
  credential, tenant/CSRF isolation, OAuth one-time exchange, core command acknowledgements.
- Browser: unbound redirect, cloud login/consent/return, cloud device detail and
  system Profile views, local mobile layout, offline-cloud local restart.
- Actual router: cloud stop exited Mihomo; Agent restart preserved stopped intent;
  local Web start restored Mihomo; cloud restart produced a new process and report.
- `tun.enable=false`, DNS redirect disabled, original no-TUN identity retained;
  exact cloud-domain DIRECT rule is first. Routes, policy rules and normalized
  firewall rules match the pre-upgrade snapshot after tests.
- On-device proxy probe: invalid-port negative control passed, proxy HTTPS 204,
  distinct proxy/direct egress, JP proxy location. No curl false-positive assumption.
- Camofy/Sub2API HTTPS health passed; no Sub2API/Caddy restart in this deployment.

The first deployment gate compared live iptables packet counters and raised a
false failure. Normalizing counters proved rules unchanged. Its automatic rollback
could not stop the supervisor while the child was still running, so it did not
overwrite the new executable/settings. Both processes were subsequently stopped
and the new supervisor started cleanly for persistence verification. The helper
now captures both PIDs before sending TERM and normalizes counters. Backup recovery
is available; automatic rollback of this revised helper has not been fault-injected.

Recovery: stop both recorded, command-line-verified supervisor/Agent PIDs together,
wait for their child core to exit, restore the backed-up executable, agent.json,
running.yaml and last-good.json, then launch start-agent.sh. A persisted
`config/control.json` stop intent should be retained unless deliberately changed.
Do not reboot, flush firewall rules, or run the original one-time migration installer.

## Original migration record

Deployment was explicitly authorized to `ssh router` only after the previous
configuration had been copied to the cloud. No server/router build was performed.
Cloud images were built locally, pushed via `registry.example.com`, and ym pulls
an immutable digest via `registry-mirror.example.com`. No application source is in private-deployment.

## Cloud model

- Identity: ordered `profiles: [{profile_id, enabled}]`; UI name is “身份”, internal
  API resource kind remains `bundle`. Profile activation is never global.
- Multiple subscription profiles (independent proxy/refresh settings) and independent
  profiles can be mixed; there is no fixed profile-count quota.
- Rules concatenate; named nodes/groups upsert; ordinary maps deep-merge and other
  fields replace. The six legacy prepend/append directives retain their positioning
  semantics. See `docs/cloud.md` for null, duplicate and MATCH behavior.
- Identity creation issues a stable subscription URL. Agent accepts this URL directly,
  downloads the merged YAML, receives WebSocket changes and checks every 300 seconds.
  A device-specific subscription URL is optional for telemetry and delay commands.

## Migrated data

Target account: `operator@example.com` (no account password is stored in this repository).
Three old upstream subscription URLs, three independent profiles and old defaults
were imported. All three cached upstream files were also saved as independent
snapshot profiles so failed/changed upstreams cannot destroy the old state.
All six original profile contents and all three URLs were compared against the
router backup after import. Blank/comment-only files become empty mappings.

“路由器旧配置（迁移留存）” preserves the previous associations and saved selection/
schedule metadata. Its enabled upstream input is the frozen old active cache, not
a newly fetched replacement. Old remote profiles remain available but disabled in
this identity. Old subscriptions refresh every 24 hours; the prior 03:00 cron is
preserved as metadata, not implemented as a wall-clock cron schedule.

At migration time, the old active `naixi` cache was empty and its upstream failed
YAML validation. `wd` could not be fetched directly. Their old data and URLs remain
available; `kitty network` refreshed successfully. No success is claimed for failed
upstreams; their proxy/URL can be adjusted in the cloud.

“路由器联调（无 TUN）” combines the user-supplied new subscription and the ordinary
“测试运行参数 · 关闭 TUN” profile. The source refreshed successfully (32 nodes).
The test profile sets `tun.enable: false`, mixed port 17890, `allow-lan: false`,
loopback binding, DNS listen `127.0.0.1:15353`, and disables unused proxy listeners.
The Agent has `dns_redirect: false`. Neither Agent nor scripts special-case this
identity. It is the canonical identity subscription URL that the router consumes.

## Installed files / recovery

- `/jffs/camofy/camofy`: headless ARMv7-musl Agent, built locally with cargo-zigbuild.
- `/jffs/camofy/agent.json`: subscription URL, existing Mihomo path, data directory,
  loopback controller 9091, DNS redirect disabled; mode 0600.
- `/jffs/camofy/start-agent.sh`: small process supervisor; auto-start wired into the
  existing `/jffs/scripts/services-start`, retaining its other firmware hooks.
- `/jffs/camofy/core/mihomo`: pre-existing 1.19.17 binary, unchanged.
- `/jffs/camofy/config`: existing data retained, plus running YAML and last-good cache.
- `/jffs/camofy/camofy.legacy-20260916` and
  `/jffs/camofy/services-start.legacy-20260916`: original program/startup script.

Full original binary/config/core/startup backup is local in ignored
`output/router-migration-20260916-015147/`. That directory also holds private
migration manifests and verification artifacts; never commit it to the public repo.
Do not re-run the one-time installer against an already migrated router.

To recover manually, first stop the supervisor and Agent by their PID files,
checking `/proc/<pid>/cmdline` identifies these exact Camofy processes. Send TERM
to both; confirm their child Mihomo exits before replacing anything. Restore the
old binary and startup script from the two backups above, then launch old Camofy
with its original settings. The original app.json and profile files were not
modified. Never reboot the router or flush firewall tables as a rollback shortcut.

## Verification

- Windows and Linux engine/security tests passed; isolated PostgreSQL + real Agent
  integration passed: proxy fetch, scheduled/manual refresh, identity bindings,
  tenant isolation, stable URLs, revisions, WebSocket convergence, rollback/revocation.
- ARM binary executes on this ARMv7 Linux 4.1 router; real `mihomo -t` passed.
- Final API reports TUN off, `allow-lan=false`, proxy/controller/DNS on loopback only;
  legacy UI port 3000 is closed. Route tables, policy rules and normalized iptables
  rules matched the pre-install snapshot after installation, reload and restart.
- Cloud test-profile log level changed to error, appeared in router running YAML,
  then was restored to warning through the same URL and notification flow.
- Agent was terminated deliberately; supervisor restarted it, restored the valid
  configuration and healthy Mihomo, with network rules unchanged.
- Firmware curl 7.76.1 ignored explicit HTTP/SOCKS proxy settings (even port 1
  incorrectly succeeded). Its early HTTP 204 results are NOT proxy evidence.
  SSH forwarding is disabled and was not enabled. A temporary locally cross-built
  `examples/proxy-probe.rs` was run on-device: port-1 negative control failed as
  required, real HTTP-proxy HTTPS request returned 204, egress differed from direct,
  and the trace reported JP. The temporary probe is not a permanent router install.
- Observed Agent RSS ~3.5 MiB, Mihomo RSS ~57 MiB; this is a snapshot, not a load test.
- Deployed Agent SHA256: `1dbeebc8cbd1e14abf7660e1152fb79c510d6103a5225570cc1945f2b25e0877`.
- Final Camofy/Sub2API HTTPS health checks passed. The bounded Camofy deployment
  itself preserved all unrelated containers. A separate Sub2API image replacement
  was observed at 2026-09-15 18:13:42 UTC, after that deployment; it was not initiated
  by this task's commands. Its DB, Redis and Caddy retained their original starts.
- Existing Mihomo warned its old JFFS cache.db cannot be opened (`invalid argument`);
  this was also present during preflight and did not prevent config validation or
  network forwarding. Agent last-good persistence/restart passed independently.
- Initial installer used unavailable `seq`; it was replaced with POSIX shell loops.
  A rollback startup race was corrected to track the launched supervisor PID and
  explicitly stop any remaining Agent before restoration. Final installation passed.

TUN interception, reboot/power-loss recovery, all old upstreams and third-party app
imports are not claimed as verified by this no-TUN deployment.
